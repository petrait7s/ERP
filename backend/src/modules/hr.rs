//! HR: departments, employees, attendance, leave and payroll with provident fund and salary TDS.
use crate::{api::Api, auth::AuthUser, db::{self, created, data, pct, r2, Created, RawJson}, error::{AppError, AppResult}, modules::{accounting::{self, Acc, Post, JL}, approvals, finance}, resource::{self, opt, req, Resource, K}, state::AppState};
use axum::{extract::{Path, Query, State}, Json};
use chrono::{Datelike, NaiveDate, Weekday};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::json;
use sqlx::PgConnection;
use std::collections::HashMap;
use uuid::Uuid;

static DEPARTMENTS: Resource = Resource {
    path: "/departments", table: "departments", tag: "HR", label: "department", read: "hr.read", write: "hr.write",
    fields: &[req("code", K::Text(20)), req("name", K::Text(120)), opt("name_bn", K::Text(120)), opt("parent_id", K::Fk("departments")), opt("is_active", K::Bool)],
    search: &["code", "name"], filters: &["is_active", "parent_id"], order: "code",
    extra: ", (SELECT count(*) FROM employees e WHERE e.department_id = t.id AND e.status = 'active') AS headcount", auto_code: None,
    example: r#"{"code":"IMP","name":"Import & Logistics","name_bn":"আমদানি ও লজিস্টিকস"}"#,
};

static EMPLOYEES: Resource = Resource {
    path: "/employees", table: "employees", tag: "HR", label: "employee", read: "hr.read", write: "hr.write",
    fields: &[req("code", K::Text(20)), req("name", K::Text(160)), opt("name_bn", K::Text(160)), opt("designation", K::Text(100)), opt("branch_id", K::Fk("branches")), opt("department_id", K::Fk("departments")),
        opt("nid", K::Text(20)), opt("tin", K::Text(20)), opt("phone", K::Text(40)), opt("email", K::Email), opt("address", K::Text(400)),
        req("join_date", K::Date), opt("exit_date", K::Date), opt("employment_type", K::Enum(&["permanent", "probation", "contract", "intern"])), opt("status", K::Enum(&["active", "on_leave", "resigned", "terminated"])),
        opt("basic_salary", K::Dec), opt("house_rent", K::Dec), opt("medical", K::Dec), opt("conveyance", K::Dec), opt("other_allowance", K::Dec), opt("pf_rate", K::Dec), opt("monthly_tax", K::Dec),
        opt("bank_account_no", K::Text(40)), opt("payment_method", K::Enum(&["bank", "cash", "bkash"]))],
    search: &["code", "name", "name_bn", "designation", "phone"], filters: &["branch_id", "department_id", "status", "employment_type"], order: "code",
    extra: ", (SELECT name FROM departments d WHERE d.id = t.department_id) AS department_name, (SELECT name FROM branches b WHERE b.id = t.branch_id) AS branch_name, (t.basic_salary + t.house_rent + t.medical + t.conveyance + t.other_allowance) AS gross_salary",
    auto_code: Some(("code", "employee", "EMP")),
    example: r#"{"name":"Nusrat Jahan","name_bn":"নুসরাত জাহান","designation":"Import Officer","department_id":"<uuid>","join_date":"2025-07-01","basic_salary":32000,"house_rent":16000,"medical":4000,"conveyance":3000,"pf_rate":10,"monthly_tax":0,"payment_method":"bank","bank_account_no":"1011220009911"}"#,
};

// ---------------- attendance ----------------
#[derive(Deserialize)]
struct AttRow { employee_id: Uuid, status: Option<String>, check_in: Option<chrono::NaiveTime>, check_out: Option<chrono::NaiveTime>, notes: Option<String> }
#[derive(Deserialize)]
struct AttIn { work_date: NaiveDate, rows: Vec<AttRow> }

/// Record a day's attendance for many employees at once (re-sending a day overwrites it).
async fn mark_attendance(State(st): State<AppState>, user: AuthUser, Json(b): Json<AttIn>) -> AppResult<Json<serde_json::Value>> {
    if b.rows.is_empty() { return Err(AppError::field("rows", "Add at least one employee")); }
    let mut tx = st.db.begin().await?;
    for (i, r) in b.rows.iter().enumerate() {
        let status = r.status.clone().unwrap_or_else(|| "present".into());
        if !["present", "late", "absent", "leave", "holiday", "weekend", "half_day"].contains(&status.as_str()) { return Err(AppError::field(&format!("rows[{i}].status"), "Unknown status")); }
        if let (Some(a), Some(o)) = (r.check_in, r.check_out) { if o < a { return Err(AppError::field(&format!("rows[{i}].check_out"), "Check-out is before check-in")); } }
        db::ensure(&mut tx, "employees", user.company_id, r.employee_id, &format!("rows[{i}].employee_id")).await?;
        sqlx::query("INSERT INTO attendance (company_id, employee_id, work_date, check_in, check_out, status, notes) VALUES ($1,$2,$3,$4,$5,$6,$7) ON CONFLICT (employee_id, work_date) DO UPDATE SET check_in = EXCLUDED.check_in, check_out = EXCLUDED.check_out, status = EXCLUDED.status, notes = EXCLUDED.notes")
            .bind(user.company_id).bind(r.employee_id).bind(b.work_date).bind(r.check_in).bind(r.check_out).bind(&status).bind(&r.notes).execute(&mut *tx).await?;
    }
    db::audit(&mut tx, &user, "mark", "attendance", b.work_date, json!({"rows": b.rows.len()})).await?;
    tx.commit().await?;
    Ok(Json(json!({"data": {"work_date": b.work_date, "saved": b.rows.len()}})))
}

async fn list_attendance(State(st): State<AppState>, user: AuthUser, Query(q): Query<HashMap<String, String>>) -> AppResult<RawJson> {
    let mut qb = db::list_query();
    qb.push("a.id, a.employee_id, e.code, e.name AS employee_name, a.work_date, a.check_in, a.check_out, a.status, a.notes FROM attendance a JOIN employees e ON e.id = a.employee_id WHERE a.company_id = ");
    qb.push_bind(user.company_id);
    db::filter_eq(&mut qb, &q, "employee_id", "a.employee_id", "::uuid");
    db::filter_eq(&mut qb, &q, "status", "a.status", "");
    db::filter_dates(&mut qb, &q, "a.work_date");
    db::page_json(&st.db, qb, "a.work_date DESC, e.code", &q).await
}

async fn attendance_summary(State(st): State<AppState>, user: AuthUser, Query(q): Query<HashMap<String, String>>) -> AppResult<RawJson> {
    let (y, m) = year_month(&q)?;
    let start = NaiveDate::from_ymd_opt(y, m, 1).unwrap();
    let s: String = sqlx::query_scalar("SELECT coalesce(jsonb_agg(jsonb_build_object('employee_id', e.id, 'code', e.code, 'name', e.name, 'present', count(*) FILTER (WHERE a.status IN ('present','late')), 'late', count(*) FILTER (WHERE a.status = 'late'), 'absent', count(*) FILTER (WHERE a.status = 'absent'), 'leave', count(*) FILTER (WHERE a.status = 'leave'), 'half_day', count(*) FILTER (WHERE a.status = 'half_day')) ORDER BY e.code), '[]')::text FROM employees e LEFT JOIN attendance a ON a.employee_id = e.id AND a.work_date >= $2 AND a.work_date < ($2 + interval '1 month') WHERE e.company_id = $1 AND e.status = 'active' GROUP BY e.id")
        .bind(user.company_id).bind(start).fetch_one(&st.db).await.or_else(|_| -> AppResult<String> { Ok("[]".into()) })?;
    Ok(data(s))
}

fn year_month(q: &HashMap<String, String>) -> AppResult<(i32, u32)> {
    let y = q.get("year").and_then(|v| v.parse::<i32>().ok()).ok_or_else(|| AppError::BadRequest("year is required".into()))?;
    let m = q.get("month").and_then(|v| v.parse::<u32>().ok()).filter(|m| (1..=12).contains(m)).ok_or_else(|| AppError::BadRequest("month must be 1 to 12".into()))?;
    Ok((y, m))
}

// ---------------- leave ----------------
#[derive(Deserialize)]
struct LeaveIn { employee_id: Uuid, leave_type: String, from_date: NaiveDate, to_date: NaiveDate, days: Option<Decimal>, reason: Option<String> }

/// Working days between two dates, skipping Friday and Saturday (Bangladesh weekend is Friday; Saturday is commonly off too).
fn working_days(from: NaiveDate, to: NaiveDate) -> Decimal {
    let mut d = from; let mut n = 0;
    while d <= to { if d.weekday() != Weekday::Fri && d.weekday() != Weekday::Sat { n += 1; } d = d.succ_opt().unwrap(); }
    Decimal::from(n)
}

async fn create_leave(State(st): State<AppState>, user: AuthUser, Json(b): Json<LeaveIn>) -> AppResult<Created> {
    if !["casual", "sick", "earned", "maternity", "unpaid", "other"].contains(&b.leave_type.as_str()) { return Err(AppError::field("leave_type", "Unknown leave type")); }
    if b.to_date < b.from_date { return Err(AppError::field("to_date", "End date is before start date")); }
    let days = b.days.unwrap_or_else(|| working_days(b.from_date, b.to_date));
    if days <= Decimal::ZERO { return Err(AppError::field("days", "Leave must cover at least one working day")); }
    let mut tx = st.db.begin().await?;
    db::ensure(&mut tx, "employees", user.company_id, b.employee_id, "employee_id").await?;
    let overlap: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM leave_requests WHERE employee_id = $1 AND status IN ('pending','approved') AND from_date <= $3 AND to_date >= $2)").bind(b.employee_id).bind(b.from_date).bind(b.to_date).fetch_one(&mut *tx).await?;
    if overlap { return Err(AppError::rule("This employee already has leave in that period")); }
    let doc: (Uuid, String) = sqlx::query_as("INSERT INTO leave_requests (company_id, employee_id, leave_type, from_date, to_date, days, reason) VALUES ($1,$2,$3,$4,$5,$6,$7) RETURNING id, to_jsonb(leave_requests)::text").bind(user.company_id).bind(b.employee_id).bind(&b.leave_type).bind(b.from_date).bind(b.to_date).bind(days).bind(&b.reason).fetch_one(&mut *tx).await?;
    db::audit(&mut tx, &user, "create", "leave_requests", doc.0, json!({"days": days})).await?;
    tx.commit().await?;
    Ok(created(doc.1))
}

#[derive(Deserialize)]
struct Decide { approve: bool }
async fn decide_leave(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>, Json(b): Json<Decide>) -> AppResult<RawJson> {
    let mut tx = st.db.begin().await?;
    let (status, emp, from, to, kind): (String, Uuid, NaiveDate, NaiveDate, String) = sqlx::query_as("SELECT status, employee_id, from_date, to_date, leave_type FROM leave_requests WHERE id = $1 AND company_id = $2 FOR UPDATE").bind(id).bind(user.company_id).fetch_optional(&mut *tx).await?.ok_or_else(|| AppError::not_found("leave request"))?;
    if status != "pending" { return Err(AppError::rule(format!("This request is already {status}"))); }
    sqlx::query("UPDATE leave_requests SET status = $2, decided_by = $3, decided_at = now() WHERE id = $1").bind(id).bind(if b.approve { "approved" } else { "rejected" }).bind(user.user_id).execute(&mut *tx).await?;
    if b.approve {
        let mut d = from;
        while d <= to {
            if d.weekday() != Weekday::Fri && d.weekday() != Weekday::Sat {
                // Unpaid leave counts as absence in payroll; paid leave does not.
                sqlx::query("INSERT INTO attendance (company_id, employee_id, work_date, status, notes) VALUES ($1,$2,$3,$4,$5) ON CONFLICT (employee_id, work_date) DO UPDATE SET status = EXCLUDED.status, notes = EXCLUDED.notes")
                    .bind(user.company_id).bind(emp).bind(d).bind(if kind == "unpaid" { "absent" } else { "leave" }).bind(format!("{kind} leave")).execute(&mut *tx).await?;
            }
            d = d.succ_opt().unwrap();
        }
    }
    db::audit(&mut tx, &user, if b.approve { "approve" } else { "reject" }, "leave_requests", id, json!({})).await?;
    let doc: String = sqlx::query_scalar("SELECT to_jsonb(l)::text FROM leave_requests l WHERE id = $1").bind(id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(data(doc))
}

async fn list_leave(State(st): State<AppState>, user: AuthUser, Query(q): Query<HashMap<String, String>>) -> AppResult<RawJson> {
    let mut qb = db::list_query();
    qb.push("l.*, e.name AS employee_name, e.code AS employee_code FROM leave_requests l JOIN employees e ON e.id = l.employee_id WHERE l.company_id = ");
    qb.push_bind(user.company_id);
    db::filter_eq(&mut qb, &q, "status", "l.status", "");
    db::filter_eq(&mut qb, &q, "employee_id", "l.employee_id", "::uuid");
    db::filter_dates(&mut qb, &q, "l.from_date");
    db::page_json(&st.db, qb, "l.created_at DESC", &q).await
}

// ---------------- payroll ----------------
#[derive(Deserialize)]
struct RunIn { year: i32, month: u32, working_days: Option<i32> }

const RUN_DOC: &str = "SELECT (to_jsonb(r) || jsonb_build_object('employees', (SELECT count(*) FROM payslips s WHERE s.run_id = r.id),
  'payslips', (SELECT coalesce(jsonb_agg(to_jsonb(s) || jsonb_build_object('employee_code', e.code, 'employee_name', e.name, 'designation', e.designation, 'payment_method', e.payment_method, 'bank_account_no', e.bank_account_no) ORDER BY e.code), '[]') FROM payslips s JOIN employees e ON e.id = s.employee_id WHERE s.run_id = r.id),
  'approvals', (SELECT coalesce(jsonb_agg(jsonb_build_object('level', a.level, 'approver_role', a.approver_role, 'status', a.status, 'comment', a.comment) ORDER BY a.level), '[]') FROM approval_requests a WHERE a.entity_type = 'payroll_run' AND a.entity_id = r.id)))::text
  FROM payroll_runs r WHERE r.id = $1 AND r.company_id = $2";

/// Compute one payslip. Absence is deducted from gross pro rata; PF is a percentage of basic,
/// matched by the employer; salary tax comes from the employee's monthly TDS.
pub fn payslip(basic: Decimal, hr: Decimal, med: Decimal, conv: Decimal, other: Decimal, pf_rate: Decimal, tax: Decimal, working: i32, absent: Decimal) -> (Decimal, Decimal, Decimal, Decimal, Decimal) {
    let gross = basic + hr + med + conv + other;
    let absent_days = absent.min(Decimal::from(working));
    let ded = r2(gross * absent_days / Decimal::from(working.max(1)));
    let pf = pct(basic, pf_rate);
    let net = gross - ded - pf - tax;
    (gross, ded, pf, pf, net)
}

async fn create_run(State(st): State<AppState>, user: AuthUser, Json(b): Json<RunIn>) -> AppResult<Created> {
    if !(2000..=2100).contains(&b.year) || !(1..=12).contains(&b.month) { return Err(AppError::field("month", "Choose a valid year and month")); }
    let mut tx = st.db.begin().await?;
    let wd = match b.working_days { Some(d) if (1..=31).contains(&d) => d, Some(_) => return Err(AppError::field("working_days", "Must be 1 to 31")), None => db::setting_dec(&mut tx, user.company_id, "hr.working_days_per_month", 26.into()).await?.to_string().parse().unwrap_or(26) };
    let start = NaiveDate::from_ymd_opt(b.year, b.month, 1).unwrap();
    let no = db::next_no(&mut tx, user.company_id, "payroll", "PAYROLL").await?;
    let id: Uuid = sqlx::query_scalar("INSERT INTO payroll_runs (company_id, run_no, period_year, period_month, working_days, created_by) VALUES ($1,$2,$3,$4,$5,$6) RETURNING id").bind(user.company_id).bind(&no).bind(b.year).bind(b.month as i32).bind(wd).bind(user.user_id).fetch_one(&mut *tx).await
        .map_err(|e| match AppError::from(e) { AppError::Conflict(_) => AppError::Conflict(format!("Payroll for {}-{:02} already exists", b.year, b.month)), o => o })?;
    let emps: Vec<(Uuid, Decimal, Decimal, Decimal, Decimal, Decimal, Decimal, Decimal)> = sqlx::query_as("SELECT id, basic_salary, house_rent, medical, conveyance, other_allowance, pf_rate, monthly_tax FROM employees WHERE company_id = $1 AND status IN ('active','on_leave') AND join_date < ($2 + interval '1 month') AND (exit_date IS NULL OR exit_date >= $2) ORDER BY code").bind(user.company_id).bind(start).fetch_all(&mut *tx).await?;
    if emps.is_empty() { return Err(AppError::rule("There are no active employees to pay")); }
    let (mut tg, mut td, mut tn, mut tpf) = (Decimal::ZERO, Decimal::ZERO, Decimal::ZERO, Decimal::ZERO);
    for (emp, basic, hr, med, conv, other, pf_rate, tax) in emps {
        let (full, half): (i64, i64) = sqlx::query_as("SELECT count(*) FILTER (WHERE status = 'absent'), count(*) FILTER (WHERE status = 'half_day') FROM attendance WHERE employee_id = $1 AND work_date >= $2 AND work_date < ($2 + interval '1 month')").bind(emp).bind(start).fetch_one(&mut *tx).await?;
        let absent = Decimal::from(full) + Decimal::from(half) / Decimal::from(2);
        let (gross, ded, pf_e, pf_r, net) = payslip(basic, hr, med, conv, other, pf_rate, tax, wd, absent);
        if net < Decimal::ZERO { return Err(AppError::rule("An employee's deductions exceed their pay. Check absence, PF and tax settings.")); }
        sqlx::query("INSERT INTO payslips (run_id, employee_id, working_days, payable_days, absent_days, basic, house_rent, medical, conveyance, other_allowance, gross, absent_deduction, pf_employee, pf_employer, tax, net_pay) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16)")
            .bind(id).bind(emp).bind(wd).bind(Decimal::from(wd) - absent.min(Decimal::from(wd))).bind(absent).bind(basic).bind(hr).bind(med).bind(conv).bind(other).bind(gross).bind(ded).bind(pf_e).bind(pf_r).bind(tax).bind(net).execute(&mut *tx).await?;
        tg += gross; td += ded + pf_e + tax; tn += net; tpf += pf_r;
    }
    sqlx::query("UPDATE payroll_runs SET total_gross = $2, total_deductions = $3, total_net = $4, total_employer_pf = $5 WHERE id = $1").bind(id).bind(tg).bind(td).bind(tn).bind(tpf).execute(&mut *tx).await?;
    db::audit(&mut tx, &user, "create", "payroll_runs", id, json!({"run_no": no, "net": tn})).await?;
    let doc: String = sqlx::query_scalar(RUN_DOC).bind(id).bind(user.company_id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(created(doc))
}

async fn run_head(conn: &mut PgConnection, company: Uuid, id: Uuid) -> AppResult<(String, String, Decimal, i32, i32)> {
    sqlx::query_as("SELECT status, run_no, total_net, period_year, period_month FROM payroll_runs WHERE id = $1 AND company_id = $2 FOR UPDATE").bind(id).bind(company).fetch_optional(&mut *conn).await?.ok_or_else(|| AppError::not_found("payroll run"))
}

async fn delete_run(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> AppResult<Json<serde_json::Value>> {
    let mut tx = st.db.begin().await?;
    let (status, no, ..) = run_head(&mut tx, user.company_id, id).await?;
    if status != "draft" && status != "rejected" { return Err(AppError::rule(format!("{no} is {status} and cannot be deleted"))); }
    sqlx::query("DELETE FROM payroll_runs WHERE id = $1").bind(id).execute(&mut *tx).await?;
    db::audit(&mut tx, &user, "delete", "payroll_runs", id, json!({"run_no": no})).await?;
    tx.commit().await?;
    Ok(Json(json!({"data": {"id": id, "deleted": true}})))
}

async fn submit_run(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> AppResult<RawJson> {
    let mut tx = st.db.begin().await?;
    let (status, no, net, ..) = run_head(&mut tx, user.company_id, id).await?;
    if status != "draft" { return Err(AppError::rule(format!("{no} is already {status}"))); }
    let new_status = if approvals::submit(&mut tx, &user, "payroll_run", id, &no, net).await? { "pending_approval" } else { "approved" };
    sqlx::query("UPDATE payroll_runs SET status = $2 WHERE id = $1").bind(id).bind(new_status).execute(&mut *tx).await?;
    db::audit(&mut tx, &user, "submit", "payroll_runs", id, json!({"net": net})).await?;
    let doc: String = sqlx::query_scalar(RUN_DOC).bind(id).bind(user.company_id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(data(doc))
}

pub async fn on_approval(conn: &mut PgConnection, user: &AuthUser, id: Uuid, approved: bool) -> AppResult<()> {
    sqlx::query("UPDATE payroll_runs SET status = $3 WHERE id = $1 AND company_id = $2 AND status = 'pending_approval'").bind(id).bind(user.company_id).bind(if approved { "approved" } else { "rejected" }).execute(&mut *conn).await?;
    Ok(())
}

#[derive(Deserialize)]
struct PostRun { posting_date: Option<NaiveDate> }
/// Book the salary expense and liabilities: net salary, provident fund (both shares) and salary TDS.
async fn post_run(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>, body: Option<Json<PostRun>>) -> AppResult<RawJson> {
    let mut tx = st.db.begin().await?;
    let (status, no, _, y, m) = run_head(&mut tx, user.company_id, id).await?;
    if status != "approved" { return Err(AppError::rule(format!("{no} is {status}; it must be approved before posting"))); }
    let (gross, ded, net, pf_er): (Decimal, Decimal, Decimal, Decimal) = sqlx::query_as("SELECT sum(gross - absent_deduction), 0, sum(net_pay), sum(pf_employer) FROM payslips WHERE run_id = $1").bind(id).fetch_one(&mut *tx).await?;
    let _ = ded;
    let (pf_emp, tax): (Decimal, Decimal) = sqlx::query_as("SELECT sum(pf_employee), sum(tax) FROM payslips WHERE run_id = $1").bind(id).fetch_one(&mut *tx).await?;
    let date = match body.and_then(|b| b.0.posting_date) { Some(d) => d, None => { let last = if m == 12 { NaiveDate::from_ymd_opt(y + 1, 1, 1) } else { NaiveDate::from_ymd_opt(y, m as u32 + 1, 1) }.unwrap().pred_opt().unwrap(); last } };
    let je = accounting::post(&mut tx, Post { company: user.company_id, branch: None, date, source_type: "payroll", source_id: Some(id), memo: &format!("Payroll {no} for {y}-{m:02}"), user: user.user_id }, vec![
        JL::dr(Acc::Key("salary_expense"), gross), JL::dr(Acc::Key("pf_expense"), pf_er),
        JL::cr(Acc::Key("salary_payable"), net), JL::cr(Acc::Key("pf_payable"), pf_emp + pf_er), JL::cr(Acc::Key("tds_payable"), tax).memo("Salary TDS"),
    ]).await?;
    sqlx::query("UPDATE payroll_runs SET status = 'posted', journal_entry_id = $2 WHERE id = $1").bind(id).bind(je).execute(&mut *tx).await?;
    db::audit(&mut tx, &user, "post", "payroll_runs", id, json!({"net": net})).await?;
    let doc: String = sqlx::query_scalar(RUN_DOC).bind(id).bind(user.company_id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(data(doc))
}

#[derive(Deserialize)]
struct PayRun { bank_account_id: Uuid, payment_date: Option<NaiveDate> }
async fn pay_run(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>, Json(b): Json<PayRun>) -> AppResult<RawJson> {
    let mut tx = st.db.begin().await?;
    let (status, no, net, ..) = run_head(&mut tx, user.company_id, id).await?;
    if status != "posted" { return Err(AppError::rule(format!("{no} is {status}; post it before paying"))); }
    let bank = finance::bank(&mut tx, user.company_id, b.bank_account_id, "bank_account_id").await?;
    if bank.currency != "BDT" { return Err(AppError::field("bank_account_id", "Pay salaries from a BDT account")); }
    finance::ensure_funds(&mut tx, user.company_id, &bank, net).await?;
    let date = match b.payment_date { Some(d) => d, None => db::today(&mut tx, user.company_id).await? };
    let je = accounting::post(&mut tx, Post { company: user.company_id, branch: None, date, source_type: "payroll_payment", source_id: Some(id), memo: &format!("Salary payment {no}"), user: user.user_id }, vec![JL::dr(Acc::Key("salary_payable"), net), JL::cr(Acc::Id(bank.gl), net)]).await?;
    sqlx::query("UPDATE payroll_runs SET status = 'paid', payment_journal_id = $2 WHERE id = $1").bind(id).bind(je).execute(&mut *tx).await?;
    db::audit(&mut tx, &user, "pay", "payroll_runs", id, json!({"net": net})).await?;
    let doc: String = sqlx::query_scalar(RUN_DOC).bind(id).bind(user.company_id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(data(doc))
}

async fn list_runs(State(st): State<AppState>, user: AuthUser, Query(q): Query<HashMap<String, String>>) -> AppResult<RawJson> {
    let mut qb = db::list_query();
    qb.push("r.*, (SELECT count(*) FROM payslips s WHERE s.run_id = r.id) AS employees FROM payroll_runs r WHERE r.company_id = ");
    qb.push_bind(user.company_id);
    db::filter_eq(&mut qb, &q, "status", "r.status", "");
    db::page_json(&st.db, qb, "period_year DESC, period_month DESC", &q).await
}
async fn get_run(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> AppResult<RawJson> {
    Ok(data(sqlx::query_scalar(RUN_DOC).bind(id).bind(user.company_id).fetch_optional(&st.db).await?.ok_or_else(|| AppError::not_found("payroll run"))?))
}

pub fn api() -> Api {
    let a = resource::routes(resource::routes(Api::new("HR"), &DEPARTMENTS), &EMPLOYEES);
    a.get("/attendance", "hr.read", "Attendance records", list_attendance).query("page,per_page,employee_id,status,from,to")
        .get("/attendance/summary", "hr.read", "Monthly attendance summary per employee", attendance_summary).query("year,month")
        .post("/attendance", "hr.write", "Mark a day's attendance for many employees", mark_attendance).body(r#"{"work_date":"2026-11-10","rows":[{"employee_id":"<uuid>","status":"present","check_in":"09:02:00","check_out":"18:05:00"},{"employee_id":"<uuid>","status":"absent"}]}"#)
        .get("/leave-requests", "hr.read", "Leave requests", list_leave).query("page,per_page,status,employee_id,from")
        .post("/leave-requests", "hr.write", "Request leave", create_leave).body(r#"{"employee_id":"<uuid>","leave_type":"casual","from_date":"2026-11-15","to_date":"2026-11-17","reason":"Family event"}"#)
        .post("/leave-requests/:id/decide", "hr.write", "Approve or reject leave (approved leave is marked in attendance)", decide_leave).body(r#"{"approve":true}"#)
        .tag("Payroll")
        .get("/payroll-runs", "payroll.read", "Payroll runs", list_runs).query("page,per_page,status")
        .post("/payroll-runs", "payroll.write", "Prepare payroll for a month from salaries and attendance", create_run).body(r#"{"year":2026,"month":11}"#)
        .get("/payroll-runs/:id", "payroll.read", "Payroll run with payslips", get_run)
        .delete("/payroll-runs/:id", "payroll.write", "Delete an unposted payroll run", delete_run)
        .post("/payroll-runs/:id/submit", "payroll.write", "Submit for approval", submit_run)
        .post("/payroll-runs/:id/post", "payroll.write", "Book the salary expense and liabilities", post_run).body(r#"{"posting_date":"2026-11-30"}"#)
        .post("/payroll-runs/:id/pay", "payroll.write", "Pay net salaries from a bank account", pay_run).body(r#"{"bank_account_id":"<uuid>","payment_date":"2026-12-01"}"#)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    fn d(s: &str) -> Decimal { Decimal::from_str(s).unwrap() }

    #[test]
    fn payslip_deducts_absence_pf_and_tax() {
        // gross 55,000; 2 of 26 days absent => 4,230.77 deducted; PF 10% of 32,000 = 3,200; tax 1,000
        let (gross, ded, pf_e, pf_r, net) = payslip(d("32000"), d("16000"), d("4000"), d("3000"), d("0"), d("10"), d("1000"), 26, d("2"));
        assert_eq!(gross, d("55000"));
        assert_eq!(ded, d("4230.77"));
        assert_eq!((pf_e, pf_r), (d("3200.00"), d("3200.00")));
        assert_eq!(net, d("46569.23"));
    }

    #[test]
    fn weekend_is_friday_and_saturday() {
        // Sun 15 Nov to Sat 21 Nov 2026 has five working days
        assert_eq!(working_days(NaiveDate::from_ymd_opt(2026, 11, 15).unwrap(), NaiveDate::from_ymd_opt(2026, 11, 21).unwrap()), d("5"));
    }
}
