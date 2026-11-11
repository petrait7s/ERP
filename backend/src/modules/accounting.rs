//! Double-entry ledger engine. Every financial effect in the system goes through [`post`],
//! which guarantees balanced, immutable, source-linked journal entries.
use crate::{api::Api, auth::AuthUser, db::{self, created, data, r2, Created, RawJson}, error::{AppError, AppResult}, resource::{self, opt, req, Resource, K}, state::AppState};
use axum::{extract::{Path, Query, State}, Json};
use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::json;
use sqlx::PgConnection;
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub enum Acc {
    /// A system account resolved by key, e.g. `"inventory"`.
    Key(&'static str),
    Id(Uuid),
}

#[derive(Clone, Debug)]
pub struct JL {
    pub acc: Acc,
    pub debit: Decimal,
    pub credit: Decimal,
    pub party: Option<(&'static str, Uuid)>,
    pub fc: Option<(String, Decimal)>,
    pub tag: Option<(&'static str, Uuid)>,
    pub memo: Option<String>,
}
impl JL {
    pub fn dr(acc: Acc, amount: Decimal) -> JL { JL { acc, debit: amount, credit: Decimal::ZERO, party: None, fc: None, tag: None, memo: None } }
    pub fn cr(acc: Acc, amount: Decimal) -> JL { JL { acc, debit: Decimal::ZERO, credit: amount, party: None, fc: None, tag: None, memo: None } }
    pub fn party(mut self, kind: &'static str, id: Uuid) -> JL { self.party = Some((kind, id)); self }
    /// Foreign-currency amount carried by this line (always positive; sign follows debit/credit).
    pub fn fc(mut self, currency: &str, amount: Decimal) -> JL { self.fc = Some((currency.to_string(), amount)); self }
    pub fn tag(mut self, kind: &'static str, id: Uuid) -> JL { self.tag = Some((kind, id)); self }
    pub fn memo(mut self, m: impl Into<String>) -> JL { self.memo = Some(m.into()); self }
}

pub struct Post<'a> {
    pub company: Uuid,
    pub branch: Option<Uuid>,
    pub date: NaiveDate,
    pub source_type: &'a str,
    pub source_id: Option<Uuid>,
    pub memo: &'a str,
    pub user: Uuid,
}

pub async fn account_id(conn: &mut PgConnection, company: Uuid, key: &str) -> AppResult<Uuid> {
    sqlx::query_scalar("SELECT id FROM accounts WHERE company_id = $1 AND system_key = $2").bind(company).bind(key).fetch_optional(&mut *conn).await?
        .ok_or_else(|| AppError::Internal(format!("system account '{key}' is missing for company {company}")))
}

/// Post a balanced journal entry. Zero lines are dropped, negative amounts flip sides,
/// everything is rounded to paisa, and the entry is rejected unless debits equal credits.
pub async fn post(conn: &mut PgConnection, p: Post<'_>, lines: Vec<JL>) -> AppResult<Uuid> {
    let mut clean: Vec<JL> = vec![];
    for mut l in lines {
        l.debit = r2(l.debit); l.credit = r2(l.credit);
        if l.debit < Decimal::ZERO { l.credit += -l.debit; l.debit = Decimal::ZERO; }
        if l.credit < Decimal::ZERO { l.debit += -l.credit; l.credit = Decimal::ZERO; }
        if l.debit > Decimal::ZERO && l.credit > Decimal::ZERO {
            let net = l.debit - l.credit;
            if net >= Decimal::ZERO { l.debit = net; l.credit = Decimal::ZERO; } else { l.credit = -net; l.debit = Decimal::ZERO; }
        }
        if l.debit.is_zero() && l.credit.is_zero() { continue; }
        clean.push(l);
    }
    let dr: Decimal = clean.iter().map(|l| l.debit).sum();
    let cr: Decimal = clean.iter().map(|l| l.credit).sum();
    if dr != cr { return Err(AppError::rule(format!("Journal does not balance: debit {dr} vs credit {cr} ({})", p.memo))); }
    if clean.len() < 2 { return Err(AppError::rule("A journal needs at least two lines with amounts")); }

    let entry_no = db::next_no(conn, p.company, "journal", "JV").await?;
    let entry: Uuid = sqlx::query_scalar("INSERT INTO journal_entries (company_id, branch_id, entry_no, entry_date, source_type, source_id, memo, created_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8) RETURNING id")
        .bind(p.company).bind(p.branch).bind(&entry_no).bind(p.date).bind(p.source_type).bind(p.source_id).bind(p.memo).bind(p.user)
        .fetch_one(&mut *conn).await?;
    let mut cache: HashMap<&'static str, Uuid> = HashMap::new();
    for (i, l) in clean.iter().enumerate() {
        let acc = match &l.acc {
            Acc::Id(id) => *id,
            Acc::Key(k) => match cache.get(k) { Some(id) => *id, None => { let id = account_id(conn, p.company, k).await?; cache.insert(k, id); id } },
        };
        let (cur, fc) = match &l.fc { Some((c, a)) => (Some(c.clone()), Some(if l.debit > Decimal::ZERO { r2(*a) } else { -r2(*a) })), None => (None, None) };
        sqlx::query("INSERT INTO journal_lines (entry_id, company_id, line_no, account_id, debit, credit, party_type, party_id, currency, fc_amount, ref_type, ref_id, memo) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)")
            .bind(entry).bind(p.company).bind(i as i32 + 1).bind(acc).bind(l.debit).bind(l.credit)
            .bind(l.party.map(|x| x.0)).bind(l.party.map(|x| x.1)).bind(cur).bind(fc)
            .bind(l.tag.map(|x| x.0)).bind(l.tag.map(|x| x.1)).bind(&l.memo)
            .execute(&mut *conn).await?;
    }
    Ok(entry)
}

/// Reverse an entry with a mirror entry; the original stays for the audit trail.
pub async fn reverse(conn: &mut PgConnection, company: Uuid, entry: Uuid, date: NaiveDate, user: Uuid, reason: &str) -> AppResult<Uuid> {
    let (no, status): (String, String) = sqlx::query_as("SELECT entry_no, status FROM journal_entries WHERE id = $1 AND company_id = $2 FOR UPDATE").bind(entry).bind(company).fetch_optional(&mut *conn).await?.ok_or_else(|| AppError::not_found("journal entry"))?;
    if status != "posted" { return Err(AppError::rule(format!("Journal {no} is already reversed"))); }
    let entry_no = db::next_no(conn, company, "journal", "JV").await?;
    let new: Uuid = sqlx::query_scalar("INSERT INTO journal_entries (company_id, branch_id, entry_no, entry_date, source_type, source_id, memo, reversal_of, created_by) SELECT company_id, branch_id, $3, $4, 'reversal', id, $5, id, $6 FROM journal_entries WHERE id = $1 AND company_id = $2 RETURNING id")
        .bind(entry).bind(company).bind(&entry_no).bind(date).bind(format!("Reversal of {no}: {reason}")).bind(user).fetch_one(&mut *conn).await?;
    sqlx::query("INSERT INTO journal_lines (entry_id, company_id, line_no, account_id, debit, credit, party_type, party_id, currency, fc_amount, ref_type, ref_id, memo) SELECT $2, company_id, line_no, account_id, credit, debit, party_type, party_id, currency, -fc_amount, ref_type, ref_id, memo FROM journal_lines WHERE entry_id = $1")
        .bind(entry).bind(new).execute(&mut *conn).await?;
    sqlx::query("UPDATE journal_entries SET status = 'reversed' WHERE id = $1").bind(entry).execute(&mut *conn).await?;
    Ok(new)
}

// ---------------- HTTP ----------------

static ACCOUNTS: Resource = Resource {
    path: "/accounts", table: "accounts", tag: "Accounting", label: "account", read: "finance.read", write: "finance.write",
    fields: &[req("code", K::Text(20)), req("name", K::Text(160)), opt("name_bn", K::Text(160)), req("account_type", K::Enum(&["asset", "liability", "equity", "income", "expense"])), opt("parent_id", K::Fk("accounts")), opt("is_group", K::Bool), opt("is_active", K::Bool)],
    search: &["code", "name", "name_bn"], filters: &["account_type", "is_group", "is_active", "parent_id"], order: "code",
    extra: ", (SELECT coalesce(sum(l.debit - l.credit), 0) FROM journal_lines l WHERE l.account_id = t.id) AS balance",
    auto_code: None,
    example: r#"{"code":"5340","name":"Marketing Expenses","name_bn":"বিপণন ব্যয়","account_type":"expense","parent_id":"<uuid of 5000>"}"#,
};

const ENTRY_DOC: &str = "SELECT (to_jsonb(e) || jsonb_build_object(
  'created_by_name', (SELECT full_name FROM users u WHERE u.id = e.created_by),
  'total', (SELECT coalesce(sum(debit), 0) FROM journal_lines l WHERE l.entry_id = e.id),
  'lines', (SELECT coalesce(jsonb_agg(jsonb_build_object('line_no', l.line_no, 'account_id', l.account_id, 'account_code', a.code, 'account_name', a.name, 'debit', l.debit, 'credit', l.credit,
              'party_type', l.party_type, 'party_id', l.party_id, 'currency', l.currency, 'fc_amount', l.fc_amount, 'ref_type', l.ref_type, 'ref_id', l.ref_id, 'memo', l.memo) ORDER BY l.line_no), '[]')
            FROM journal_lines l JOIN accounts a ON a.id = l.account_id WHERE l.entry_id = e.id)))::text
  FROM journal_entries e WHERE e.id = $1 AND e.company_id = $2";

pub async fn entry_doc(conn: &mut PgConnection, company: Uuid, id: Uuid) -> AppResult<String> {
    sqlx::query_scalar(ENTRY_DOC).bind(id).bind(company).fetch_optional(&mut *conn).await?.ok_or_else(|| AppError::not_found("journal entry"))
}

async fn list_journals(State(st): State<AppState>, user: AuthUser, Query(q): Query<HashMap<String, String>>) -> AppResult<RawJson> {
    let mut qb = db::list_query();
    qb.push("e.id, e.entry_no, e.entry_date, e.source_type, e.source_id, e.memo, e.status, e.created_at, (SELECT coalesce(sum(debit),0) FROM journal_lines l WHERE l.entry_id = e.id) AS total FROM journal_entries e WHERE e.company_id = ");
    qb.push_bind(user.company_id);
    db::filter_eq(&mut qb, &q, "source_type", "e.source_type", "");
    db::filter_eq(&mut qb, &q, "source_id", "e.source_id", "::uuid");
    db::filter_eq(&mut qb, &q, "status", "e.status", "");
    db::filter_dates(&mut qb, &q, "e.entry_date");
    db::filter_search(&mut qb, &q, &["e.entry_no", "e.memo"]);
    if let Some(acc) = q.get("account_id").filter(|v| !v.is_empty()) {
        qb.push(" AND EXISTS (SELECT 1 FROM journal_lines l WHERE l.entry_id = e.id AND l.account_id = "); qb.push_bind(acc.clone()); qb.push("::uuid)");
    }
    db::page_json(&st.db, qb, "entry_date DESC, entry_no DESC", &q).await
}

async fn get_journal(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> AppResult<RawJson> {
    let mut conn = st.db.acquire().await?;
    Ok(data(entry_doc(&mut conn, user.company_id, id).await?))
}

#[derive(Deserialize)]
struct ManualLine { account_id: Uuid, #[serde(default)] debit: Decimal, #[serde(default)] credit: Decimal, memo: Option<String>, party_type: Option<String>, party_id: Option<Uuid> }
#[derive(Deserialize)]
struct ManualJournal { entry_date: NaiveDate, memo: String, branch_id: Option<Uuid>, lines: Vec<ManualLine> }

async fn create_journal(State(st): State<AppState>, user: AuthUser, Json(b): Json<ManualJournal>) -> AppResult<Created> {
    if b.memo.trim().is_empty() { return Err(AppError::field("memo", "Please describe what this journal is for")); }
    if b.lines.len() < 2 { return Err(AppError::field("lines", "A journal needs at least two lines")); }
    let mut tx = st.db.begin().await?;
    let mut lines = vec![];
    for (i, l) in b.lines.iter().enumerate() {
        if l.debit < Decimal::ZERO || l.credit < Decimal::ZERO || (l.debit > Decimal::ZERO) == (l.credit > Decimal::ZERO) {
            return Err(AppError::field(&format!("lines[{i}]"), "Each line needs either a debit or a credit amount"));
        }
        db::ensure(&mut tx, "accounts", user.company_id, l.account_id, &format!("lines[{i}].account_id")).await?;
        let mut jl = if l.debit > Decimal::ZERO { JL::dr(Acc::Id(l.account_id), l.debit) } else { JL::cr(Acc::Id(l.account_id), l.credit) };
        if let (Some(pt), Some(pid)) = (&l.party_type, l.party_id) {
            let (kind, table): (&'static str, &'static str) = match pt.as_str() { "customer" => ("customer", "customers"), "supplier" => ("supplier", "suppliers"), "employee" => ("employee", "employees"), _ => return Err(AppError::field(&format!("lines[{i}].party_type"), "Must be customer, supplier or employee")) };
            db::ensure(&mut tx, table, user.company_id, pid, &format!("lines[{i}].party_id")).await?;
            jl = jl.party(kind, pid);
        }
        if let Some(m) = &l.memo { jl = jl.memo(m.clone()); }
        lines.push(jl);
    }
    let branch = user.branch_for(b.branch_id)?;
    let id = post(&mut tx, Post { company: user.company_id, branch, date: b.entry_date, source_type: "manual", source_id: None, memo: b.memo.trim(), user: user.user_id }, lines).await?;
    db::audit(&mut tx, &user, "post", "journal_entries", id, json!({"memo": b.memo})).await?;
    let doc = entry_doc(&mut tx, user.company_id, id).await?;
    tx.commit().await?;
    Ok(created(doc))
}

#[derive(Deserialize)]
struct ReverseIn { reason: String, date: Option<NaiveDate> }
async fn reverse_journal(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>, Json(b): Json<ReverseIn>) -> AppResult<Created> {
    if b.reason.trim().is_empty() { return Err(AppError::field("reason", "Please give a reason")); }
    let mut tx = st.db.begin().await?;
    let src: String = sqlx::query_scalar("SELECT source_type FROM journal_entries WHERE id = $1 AND company_id = $2").bind(id).bind(user.company_id).fetch_optional(&mut *tx).await?.ok_or_else(|| AppError::not_found("journal entry"))?;
    if src != "manual" { return Err(AppError::rule("Only manual journals can be reversed here. Correct the source document instead.")); }
    let date = match b.date { Some(d) => d, None => db::today(&mut tx, user.company_id).await? };
    let new = reverse(&mut tx, user.company_id, id, date, user.user_id, b.reason.trim()).await?;
    db::audit(&mut tx, &user, "reverse", "journal_entries", id, json!({"reason": b.reason, "reversal": new})).await?;
    let doc = entry_doc(&mut tx, user.company_id, new).await?;
    tx.commit().await?;
    Ok(created(doc))
}

async fn list_periods(State(st): State<AppState>, user: AuthUser) -> AppResult<RawJson> {
    Ok(data(sqlx::query_scalar("SELECT coalesce(jsonb_agg(to_jsonb(p) ORDER BY p.start_date), '[]')::text FROM fiscal_periods p WHERE company_id = $1").bind(user.company_id).fetch_one(&st.db).await?))
}

async fn set_period(st: AppState, user: AuthUser, id: Uuid, close: bool) -> AppResult<RawJson> {
    let mut tx = st.db.begin().await?;
    let (start, status): (NaiveDate, String) = sqlx::query_as("SELECT start_date, status FROM fiscal_periods WHERE id = $1 AND company_id = $2 FOR UPDATE").bind(id).bind(user.company_id).fetch_optional(&mut *tx).await?.ok_or_else(|| AppError::not_found("period"))?;
    if close {
        if status == "closed" { return Err(AppError::rule("This period is already closed")); }
        let earlier_open: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM fiscal_periods WHERE company_id = $1 AND start_date < $2 AND status = 'open')").bind(user.company_id).bind(start).fetch_one(&mut *tx).await?;
        if earlier_open { return Err(AppError::rule("Close earlier periods first")); }
        sqlx::query("UPDATE fiscal_periods SET status = 'closed', closed_by = $2, closed_at = now() WHERE id = $1").bind(id).bind(user.user_id).execute(&mut *tx).await?;
    } else {
        let later_closed: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM fiscal_periods WHERE company_id = $1 AND start_date > $2 AND status = 'closed')").bind(user.company_id).bind(start).fetch_one(&mut *tx).await?;
        if later_closed { return Err(AppError::rule("Reopen later periods first")); }
        sqlx::query("UPDATE fiscal_periods SET status = 'open', closed_by = NULL, closed_at = NULL WHERE id = $1").bind(id).execute(&mut *tx).await?;
    }
    db::audit(&mut tx, &user, if close { "close" } else { "reopen" }, "fiscal_periods", id, json!({})).await?;
    let doc: String = sqlx::query_scalar("SELECT to_jsonb(p)::text FROM fiscal_periods p WHERE id = $1").bind(id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(data(doc))
}
async fn close_period(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> AppResult<RawJson> { set_period(st, user, id, true).await }
async fn reopen_period(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> AppResult<RawJson> { set_period(st, user, id, false).await }

#[derive(Deserialize)]
struct GenPeriods { start_year: i32 }
/// Create the 12 monthly periods of the fiscal year that starts in `start_year`.
async fn generate_periods(State(st): State<AppState>, user: AuthUser, Json(b): Json<GenPeriods>) -> AppResult<RawJson> {
    if !(2000..=2100).contains(&b.start_year) { return Err(AppError::field("start_year", "Must be a year like 2026")); }
    let mut tx = st.db.begin().await?;
    let m: i32 = sqlx::query_scalar("SELECT fiscal_year_start_month FROM companies WHERE id = $1").bind(user.company_id).fetch_one(&mut *tx).await?;
    let mut start = NaiveDate::from_ymd_opt(b.start_year, m as u32, 1).unwrap();
    for _ in 0..12 {
        let next = if start.month() == 12 { NaiveDate::from_ymd_opt(start.year() + 1, 1, 1) } else { NaiveDate::from_ymd_opt(start.year(), start.month() + 1, 1) }.unwrap();
        sqlx::query("INSERT INTO fiscal_periods (company_id, name, start_date, end_date) VALUES ($1,$2,$3,$4) ON CONFLICT (company_id, start_date) DO NOTHING")
            .bind(user.company_id).bind(start.format("%b %Y").to_string()).bind(start).bind(next.pred_opt().unwrap()).execute(&mut *tx).await?;
        start = next;
    }
    db::audit(&mut tx, &user, "generate", "fiscal_periods", b.start_year, json!({})).await?;
    tx.commit().await?;
    list_periods(State(st), user).await
}

pub fn api() -> Api {
    resource::routes(Api::new("Accounting"), &ACCOUNTS)
        .get("/journals", "finance.read", "List journal entries", list_journals).query("page,per_page,q,source_type,source_id,status,account_id,from,to")
        .get("/journals/:id", "finance.read", "Journal entry with its lines", get_journal)
        .post("/journals", "finance.post", "Post a manual journal (must balance)", create_journal)
        .body(r#"{"entry_date":"2026-07-01","memo":"Owner capital introduced","lines":[{"account_id":"<bank GL uuid>","debit":5000000},{"account_id":"<share capital uuid>","credit":5000000}]}"#)
        .post("/journals/:id/reverse", "finance.post", "Reverse a manual journal with a mirror entry", reverse_journal).body(r#"{"reason":"Posted to the wrong account"}"#)
        .get("/fiscal-periods", "finance.read", "Accounting periods", list_periods)
        .post("/fiscal-periods/generate", "finance.post", "Create the monthly periods of a fiscal year", generate_periods).body(r#"{"start_year":2027}"#)
        .post("/fiscal-periods/:id/close", "finance.post", "Close a period (no more postings)", close_period)
        .post("/fiscal-periods/:id/reopen", "finance.post", "Reopen the latest closed period", reopen_period)
}
