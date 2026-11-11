//! Bangladesh VAT: purchase and sales registers (Mushak 6.1, 6.2, 6.10), monthly VAT return
//! (Mushak 9.1) computed from the ledger, and deposits of VAT, SD, VDS and TDS to the treasury.
//!
//! Net VAT payable = output VAT - credit notes - input VAT (local and import) - Advance Tax
//!                   - VDS certificates received from customers.
//! A negative result is carried forward as input VAT credit.
use crate::{api::Api, auth::AuthUser, db::{self, created, data, r2, Created, RawJson}, error::{AppError, AppResult}, modules::{accounting::{self, Acc, Post, JL}, finance}, state::AppState};
use axum::{extract::{Path, Query, State}, Json};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::PgConnection;
use std::collections::HashMap;
use uuid::Uuid;

fn month_range(y: i32, m: u32) -> AppResult<(NaiveDate, NaiveDate)> {
    let start = NaiveDate::from_ymd_opt(y, m, 1).ok_or_else(|| AppError::BadRequest("Invalid year or month".into()))?;
    let next = if m == 12 { NaiveDate::from_ymd_opt(y + 1, 1, 1) } else { NaiveDate::from_ymd_opt(y, m + 1, 1) }.unwrap();
    Ok((start, next.pred_opt().unwrap()))
}
fn ym(q: &HashMap<String, String>) -> AppResult<(i32, u32)> {
    let y = q.get("year").and_then(|v| v.parse::<i32>().ok()).ok_or_else(|| AppError::BadRequest("year is required".into()))?;
    let m = q.get("month").and_then(|v| v.parse::<u32>().ok()).filter(|m| (1..=12).contains(m)).ok_or_else(|| AppError::BadRequest("month must be 1 to 12".into()))?;
    Ok((y, m))
}

#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct Vat { pub output_vat: Decimal, pub output_sd: Decimal, pub credit_notes_vat: Decimal, pub input_vat_local: Decimal, pub input_vat_import: Decimal, pub advance_tax: Decimal, pub vds_by_customers: Decimal, pub vds_on_suppliers: Decimal, pub net_payable: Decimal }

impl Vat {
    pub fn finish(mut self) -> Vat {
        self.net_payable = (self.output_vat - self.credit_notes_vat) - self.input_vat_local - self.input_vat_import - self.advance_tax - self.vds_by_customers;
        self
    }
}

/// Sum VAT-related ledger movements of one tax period.
pub async fn compute(conn: &mut PgConnection, company: Uuid, from: NaiveDate, to: NaiveDate) -> AppResult<Vat> {
    let rows: Vec<(String, String, Decimal, Decimal)> = sqlx::query_as("SELECT a.system_key, e.source_type, coalesce(sum(l.debit), 0), coalesce(sum(l.credit), 0) FROM journal_lines l JOIN journal_entries e ON e.id = l.entry_id AND e.entry_date BETWEEN $2 AND $3 JOIN accounts a ON a.id = l.account_id WHERE l.company_id = $1 AND a.system_key IN ('vat_output','vat_input','sd_payable','at_paid','vds_receivable','vds_payable') GROUP BY 1, 2").bind(company).bind(from).bind(to).fetch_all(&mut *conn).await?;
    let mut v = Vat::default();
    for (key, src, dr, cr) in rows {
        match (key.as_str(), src.as_str()) {
            ("vat_output", "sales_invoice") => v.output_vat += cr,
            ("vat_output", "sales_return") => v.credit_notes_vat += dr,
            ("sd_payable", "sales_invoice") => v.output_sd += cr,
            ("sd_payable", "sales_return") => v.output_sd -= dr,
            ("vat_input", "supplier_bill") | ("vat_input", "landed_cost") => v.input_vat_local += dr,
            ("vat_input", "bill_of_entry") => v.input_vat_import += dr,
            ("at_paid", "bill_of_entry") => v.advance_tax += dr,
            ("vds_receivable", "customer_receipt") => v.vds_by_customers += dr,
            ("vds_payable", "supplier_payment") => v.vds_on_suppliers += cr,
            _ => {}
        }
    }
    Ok(v.finish())
}

async fn preview(State(st): State<AppState>, user: AuthUser, Query(q): Query<HashMap<String, String>>) -> AppResult<Json<Value>> {
    let (y, m) = ym(&q)?;
    let (from, to) = month_range(y, m)?;
    let mut conn = st.db.acquire().await?;
    let v = compute(&mut conn, user.company_id, from, to).await?;
    let saved: Option<(Uuid, String)> = sqlx::query_as("SELECT id, status FROM vat_returns WHERE company_id = $1 AND period_year = $2 AND period_month = $3").bind(user.company_id).bind(y).bind(m as i32).fetch_optional(&mut *conn).await?;
    Ok(Json(json!({"data": {"form": "Mushak 9.1", "year": y, "month": m, "from": from, "to": to, "figures": v, "saved_return": saved.map(|s| json!({"id": s.0, "status": s.1}))}})))
}

async fn save_return(State(st): State<AppState>, user: AuthUser, Query(q): Query<HashMap<String, String>>) -> AppResult<Created> {
    let (y, m) = ym(&q)?;
    let (from, to) = month_range(y, m)?;
    let mut tx = st.db.begin().await?;
    let existing: Option<(Uuid, String)> = sqlx::query_as("SELECT id, status FROM vat_returns WHERE company_id = $1 AND period_year = $2 AND period_month = $3 FOR UPDATE").bind(user.company_id).bind(y).bind(m as i32).fetch_optional(&mut *tx).await?;
    if let Some((_, s)) = &existing { if s == "submitted" { return Err(AppError::rule("This month's return is already submitted")); } }
    let v = compute(&mut tx, user.company_id, from, to).await?;
    let id: Uuid = sqlx::query_scalar("INSERT INTO vat_returns (company_id, period_year, period_month, output_vat, output_sd, input_vat_local, input_vat_import, advance_tax, vds_by_customers, vds_on_suppliers, credit_notes_vat, net_payable) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)
        ON CONFLICT (company_id, period_year, period_month) DO UPDATE SET output_vat = EXCLUDED.output_vat, output_sd = EXCLUDED.output_sd, input_vat_local = EXCLUDED.input_vat_local, input_vat_import = EXCLUDED.input_vat_import, advance_tax = EXCLUDED.advance_tax, vds_by_customers = EXCLUDED.vds_by_customers, vds_on_suppliers = EXCLUDED.vds_on_suppliers, credit_notes_vat = EXCLUDED.credit_notes_vat, net_payable = EXCLUDED.net_payable RETURNING id")
        .bind(user.company_id).bind(y).bind(m as i32).bind(v.output_vat).bind(v.output_sd).bind(v.input_vat_local).bind(v.input_vat_import).bind(v.advance_tax).bind(v.vds_by_customers).bind(v.vds_on_suppliers).bind(v.credit_notes_vat).bind(v.net_payable).fetch_one(&mut *tx).await?;
    db::audit(&mut tx, &user, "save", "vat_returns", id, json!({"net_payable": v.net_payable})).await?;
    let doc: String = sqlx::query_scalar("SELECT to_jsonb(r)::text FROM vat_returns r WHERE id = $1").bind(id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(created(doc))
}

#[derive(Deserialize)]
struct SubmitIn { submission_date: Option<NaiveDate> }
/// Submit the return: close the month's VAT accounts into "VAT Payable to Treasury" (or carry the credit forward).
async fn submit_return(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>, body: Option<Json<SubmitIn>>) -> AppResult<RawJson> {
    let mut tx = st.db.begin().await?;
    let (status, y, m): (String, i32, i32) = sqlx::query_as("SELECT status, period_year, period_month FROM vat_returns WHERE id = $1 AND company_id = $2 FOR UPDATE").bind(id).bind(user.company_id).fetch_optional(&mut *tx).await?.ok_or_else(|| AppError::not_found("VAT return"))?;
    if status == "submitted" { return Err(AppError::rule("This return is already submitted")); }
    let earlier: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM vat_returns WHERE company_id = $1 AND status = 'draft' AND (period_year, period_month) < ($2, $3))").bind(user.company_id).bind(y).bind(m).fetch_one(&mut *tx).await?;
    if earlier { return Err(AppError::rule("Submit earlier months' returns first")); }
    let (from, to) = month_range(y, m as u32)?;
    let v = compute(&mut tx, user.company_id, from, to).await?; // recompute so the booking matches the ledger exactly
    let net_out = v.output_vat - v.credit_notes_vat;
    let date = match body.and_then(|b| b.0.submission_date) { Some(d) => d, None => to };
    let mut lines = vec![
        JL::dr(Acc::Key("vat_output"), net_out).memo("Close output VAT"),
        JL::cr(Acc::Key("vat_input"), v.input_vat_local + v.input_vat_import).memo("Input VAT claimed"),
        JL::cr(Acc::Key("at_paid"), v.advance_tax).memo("Advance Tax adjusted"),
        JL::cr(Acc::Key("vds_receivable"), v.vds_by_customers).memo("VDS certificates adjusted"),
    ];
    if v.net_payable >= Decimal::ZERO { lines.push(JL::cr(Acc::Key("vat_payable"), v.net_payable)); } else { lines.push(JL::dr(Acc::Key("vat_input"), -v.net_payable).memo("Credit carried forward")); }
    let je = accounting::post(&mut tx, Post { company: user.company_id, branch: None, date, source_type: "vat_return", source_id: Some(id), memo: &format!("VAT return (Mushak 9.1) {y}-{m:02}"), user: user.user_id }, lines).await?;
    sqlx::query("UPDATE vat_returns SET output_vat = $2, output_sd = $3, input_vat_local = $4, input_vat_import = $5, advance_tax = $6, vds_by_customers = $7, vds_on_suppliers = $8, credit_notes_vat = $9, net_payable = $10, status = 'submitted', journal_entry_id = $11, submitted_by = $12, submitted_at = now() WHERE id = $1")
        .bind(id).bind(v.output_vat).bind(v.output_sd).bind(v.input_vat_local).bind(v.input_vat_import).bind(v.advance_tax).bind(v.vds_by_customers).bind(v.vds_on_suppliers).bind(v.credit_notes_vat).bind(v.net_payable).bind(je).bind(user.user_id).execute(&mut *tx).await?;
    db::audit(&mut tx, &user, "submit", "vat_returns", id, json!({"net_payable": v.net_payable})).await?;
    let doc: String = sqlx::query_scalar("SELECT to_jsonb(r)::text FROM vat_returns r WHERE id = $1").bind(id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(data(doc))
}

async fn list_returns(State(st): State<AppState>, user: AuthUser, Query(q): Query<HashMap<String, String>>) -> AppResult<RawJson> {
    let mut qb = db::list_query();
    qb.push("r.* FROM vat_returns r WHERE r.company_id = ");
    qb.push_bind(user.company_id);
    db::page_json(&st.db, qb, "period_year DESC, period_month DESC", &q).await
}

// ---------------- registers ----------------
/// Mushak 6.1: purchase register (supplier bills and import documents).
async fn mushak_61(State(st): State<AppState>, user: AuthUser, Query(q): Query<HashMap<String, String>>) -> AppResult<RawJson> {
    let mut qb = db::list_query();
    qb.push("x.* FROM (SELECT b.bill_date AS date, 'Local' AS kind, b.bill_no AS reference, b.mushak_no AS tax_document, s.name AS party, s.bin AS party_bin, b.subtotal AS value, 0::numeric AS duties, b.vat_amount AS vat FROM supplier_bills b JOIN suppliers s ON s.id = b.supplier_id WHERE b.company_id = ");
    qb.push_bind(user.company_id);
    qb.push(" AND b.status <> 'cancelled' UNION ALL SELECT e.be_date, 'Import', e.be_no, e.be_no, s.name, s.bin, e.assessable_value, e.cd + e.rd + e.sd, e.vat FROM bills_of_entry e JOIN shipments h ON h.id = e.shipment_id JOIN suppliers s ON s.id = h.supplier_id WHERE e.company_id = ");
    qb.push_bind(user.company_id);
    qb.push(" AND e.status IN ('paid','released')) x WHERE true");
    db::filter_dates(&mut qb, &q, "x.date");
    db::page_json(&st.db, qb, "date DESC, reference", &q).await
}
/// Mushak 6.2: sales register.
async fn mushak_62(State(st): State<AppState>, user: AuthUser, Query(q): Query<HashMap<String, String>>) -> AppResult<RawJson> {
    let mut qb = db::list_query();
    qb.push("i.invoice_date AS date, i.mushak_no, i.invoice_no, c.name AS customer, c.bin AS customer_bin, i.subtotal AS value, i.sd_amount AS sd, i.vat_amount AS vat, i.total FROM sales_invoices i JOIN customers c ON c.id = i.customer_id WHERE i.status <> 'cancelled' AND i.company_id = ");
    qb.push_bind(user.company_id);
    db::filter_dates(&mut qb, &q, "i.invoice_date");
    db::page_json(&st.db, qb, "i.invoice_date DESC, i.mushak_no DESC", &q).await
}
/// Mushak 6.10: transactions above the reporting threshold.
async fn mushak_610(State(st): State<AppState>, user: AuthUser, Query(q): Query<HashMap<String, String>>) -> AppResult<RawJson> {
    let mut conn = st.db.acquire().await?;
    let threshold = db::setting_dec(&mut conn, user.company_id, "tax.mushak_610_threshold", Decimal::from(200_000)).await?;
    drop(conn);
    let mut qb = db::list_query();
    qb.push("x.* FROM (SELECT 'Sale' AS direction, i.invoice_date AS date, i.mushak_no AS document, c.name AS party, c.bin AS party_bin, i.total AS amount, i.vat_amount AS vat FROM sales_invoices i JOIN customers c ON c.id = i.customer_id WHERE i.status <> 'cancelled' AND i.total >= ");
    qb.push_bind(threshold);
    qb.push(" AND i.company_id = "); qb.push_bind(user.company_id);
    qb.push(" UNION ALL SELECT 'Purchase', b.bill_date, coalesce(b.mushak_no, b.bill_no), s.name, s.bin, b.total, b.vat_amount FROM supplier_bills b JOIN suppliers s ON s.id = b.supplier_id WHERE b.status <> 'cancelled' AND b.total >= ");
    qb.push_bind(threshold);
    qb.push(" AND b.company_id = "); qb.push_bind(user.company_id);
    qb.push(") x WHERE true");
    db::filter_dates(&mut qb, &q, "x.date");
    db::page_json(&st.db, qb, "date DESC", &q).await
}

/// What is owed to the treasury right now.
async fn liabilities(State(st): State<AppState>, user: AuthUser) -> AppResult<Json<Value>> {
    let rows: Vec<(String, Decimal)> = sqlx::query_as("SELECT a.system_key, coalesce(sum(l.credit - l.debit), 0) FROM accounts a LEFT JOIN journal_lines l ON l.account_id = a.id WHERE a.company_id = $1 AND a.system_key IN ('vat_payable','vat_output','sd_payable','vds_payable','tds_payable','pf_payable') GROUP BY 1").bind(user.company_id).fetch_all(&st.db).await?;
    let m: HashMap<String, Decimal> = rows.into_iter().collect();
    let g = |k: &str| m.get(k).copied().unwrap_or_default();
    Ok(Json(json!({"data": {"vat_payable": g("vat_payable"), "vat_output_open": g("vat_output"), "supplementary_duty": g("sd_payable"), "vds": g("vds_payable"), "tds": g("tds_payable"), "provident_fund": g("pf_payable")}})))
}

#[derive(Deserialize)]
struct DepositIn { kind: String, amount: Decimal, bank_account_id: Uuid, deposit_date: Option<NaiveDate>, challan_no: Option<String> }
/// Deposit VAT, SD, VDS, TDS or provident fund with a treasury challan.
async fn deposit(State(st): State<AppState>, user: AuthUser, Json(b): Json<DepositIn>) -> AppResult<Json<Value>> {
    let key = match b.kind.as_str() { "vat" => "vat_payable", "sd" => "sd_payable", "vds" => "vds_payable", "tds" => "tds_payable", "pf" => "pf_payable", _ => return Err(AppError::field("kind", "Must be vat, sd, vds, tds or pf")) };
    if b.amount <= Decimal::ZERO { return Err(AppError::field("amount", "Must be more than zero")); }
    let mut tx = st.db.begin().await?;
    let bank = finance::bank(&mut tx, user.company_id, b.bank_account_id, "bank_account_id").await?;
    if bank.currency != "BDT" { return Err(AppError::field("bank_account_id", "Deposit from a BDT account")); }
    let owed: Decimal = sqlx::query_scalar("SELECT coalesce(sum(l.credit - l.debit), 0) FROM journal_lines l JOIN accounts a ON a.id = l.account_id WHERE l.company_id = $1 AND a.system_key = $2").bind(user.company_id).bind(key).fetch_one(&mut *tx).await?;
    let amount = r2(b.amount);
    if amount > owed { return Err(AppError::rule(format!("Only Tk {owed} is payable as {}", b.kind.to_uppercase()))); }
    finance::ensure_funds(&mut tx, user.company_id, &bank, amount).await?;
    let date = match b.deposit_date { Some(d) => d, None => db::today(&mut tx, user.company_id).await? };
    let je = accounting::post(&mut tx, Post { company: user.company_id, branch: None, date, source_type: "tax_deposit", source_id: None, memo: &format!("{} deposited, challan {}", b.kind.to_uppercase(), b.challan_no.as_deref().unwrap_or("-")), user: user.user_id }, vec![JL::dr(Acc::Key(key), amount), JL::cr(Acc::Id(bank.gl), amount)]).await?;
    db::audit(&mut tx, &user, "deposit", "tax", je, json!({"kind": b.kind, "amount": amount, "challan": b.challan_no})).await?;
    tx.commit().await?;
    Ok(Json(json!({"data": {"journal_entry_id": je, "kind": b.kind, "amount": amount}})))
}

pub fn api() -> Api {
    Api::new("Tax (Bangladesh)")
        .get("/tax/vat-return", "tax.read", "Live Mushak 9.1 figures for a month", preview).query("year,month")
        .get("/tax/vat-returns", "tax.read", "Saved VAT returns", list_returns)
        .post("/tax/vat-returns", "tax.write", "Save the month's VAT return as a draft (re-run to refresh)", save_return).query("year,month")
        .post("/tax/vat-returns/:id/submit", "tax.write", "Submit the return and book the VAT payable", submit_return).body(r#"{"submission_date":"2026-12-15"}"#)
        .get("/tax/mushak-6-1", "tax.read", "Purchase register", mushak_61).query("page,per_page,from,to")
        .get("/tax/mushak-6-2", "tax.read", "Sales register", mushak_62).query("page,per_page,from,to")
        .get("/tax/mushak-6-10", "tax.read", "Transactions above the reporting threshold", mushak_610).query("page,per_page,from,to")
        .get("/tax/liabilities", "tax.read", "Amounts owed for VAT, SD, VDS, TDS and provident fund", liabilities)
        .post("/tax/deposits", "tax.write", "Deposit VAT, SD, VDS, TDS or PF to the treasury", deposit).body(r#"{"kind":"vds","amount":18000,"bank_account_id":"<uuid>","challan_no":"TR-6 0012345"}"#)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    fn d(s: &str) -> Decimal { Decimal::from_str(s).unwrap() }

    #[test]
    fn net_vat_subtracts_all_credits() {
        let v = Vat { output_vat: d("500000"), credit_notes_vat: d("10000"), input_vat_local: d("120000"), input_vat_import: d("200000"), advance_tax: d("50000"), vds_by_customers: d("30000"), ..Default::default() }.finish();
        assert_eq!(v.net_payable, d("90000"));
    }

    #[test]
    fn excess_input_is_negative_and_carried_forward() {
        let v = Vat { output_vat: d("100000"), input_vat_import: d("260000"), ..Default::default() }.finish();
        assert_eq!(v.net_payable, d("-160000"));
    }

    #[test]
    fn february_has_28_days_in_2027() {
        let (a, b) = month_range(2027, 2).unwrap();
        assert_eq!((a.to_string(), b.to_string()), ("2027-02-01".into(), "2027-02-28".into()));
    }
}
