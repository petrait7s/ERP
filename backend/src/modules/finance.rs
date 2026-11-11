//! Cash and bank, exchange rates, supplier payments (with TDS/VDS), FX revaluation and exposure.
use crate::{api::Api, auth::AuthUser, db::{self, created, data, pct, r2, Created, RawJson}, error::{AppError, AppResult}, modules::{accounting::{self, Acc, Post, JL}, approvals}, resource::{self, opt, req, Resource, K}, state::AppState};
use axum::{extract::{Path, Query, State}, Json};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::PgConnection;
use std::collections::HashMap;
use uuid::Uuid;

/// BDT per one unit of `currency` on or before `date` (latest known mid rate, else any type).
pub async fn rate_on(conn: &mut PgConnection, company: Uuid, currency: &str, date: NaiveDate) -> AppResult<Decimal> {
    if currency == "BDT" { return Ok(Decimal::ONE); }
    sqlx::query_scalar("SELECT rate FROM exchange_rates WHERE company_id = $1 AND currency = $2 AND rate_date <= $3 ORDER BY rate_date DESC, (rate_type = 'mid') DESC LIMIT 1")
        .bind(company).bind(currency).bind(date).fetch_optional(&mut *conn).await?
        .ok_or_else(|| AppError::rule(format!("No exchange rate for {currency} on or before {date}. Add one under Cash & FX, or enter the rate on the document.")))
}

pub struct Bank { pub gl: Uuid, pub currency: String, pub kind: String, pub name: String }
pub async fn bank(conn: &mut PgConnection, company: Uuid, id: Uuid, field: &str) -> AppResult<Bank> {
    let r: (Uuid, String, String, String, bool) = sqlx::query_as("SELECT account_id, currency, kind, name, is_active FROM bank_accounts WHERE id = $1 AND company_id = $2").bind(id).bind(company).fetch_optional(&mut *conn).await?.ok_or_else(|| AppError::field(field, "Bank or cash account does not exist"))?;
    if !r.4 { return Err(AppError::field(field, "This account is inactive")); }
    Ok(Bank { gl: r.0, currency: r.1, kind: r.2, name: r.3 })
}

/// Cash boxes and mobile wallets cannot be overdrawn; bank accounts may have an overdraft facility.
pub async fn ensure_funds(conn: &mut PgConnection, company: Uuid, b: &Bank, amount_bdt: Decimal) -> AppResult<()> {
    if b.kind == "bank" { return Ok(()); }
    let bal: Decimal = sqlx::query_scalar("SELECT coalesce(sum(debit - credit), 0) FROM journal_lines WHERE company_id = $1 AND account_id = $2").bind(company).bind(b.gl).fetch_one(&mut *conn).await?;
    if bal < amount_bdt { return Err(AppError::rule(format!("{} has only Tk {} available", b.name, bal))); }
    Ok(())
}

static RATES: Resource = Resource {
    path: "/exchange-rates", table: "exchange_rates", tag: "Cash & FX", label: "exchange rate", read: "finance.read", write: "finance.rates",
    fields: &[req("currency", K::Code("currencies")), req("rate_date", K::Date), req("rate", K::Dec), opt("rate_type", K::Enum(&["mid", "bc_selling", "tt_buying", "customs"])), opt("source", K::Text(80))],
    search: &["currency"], filters: &["currency", "rate_type", "rate_date"], order: "rate_date DESC, currency", extra: "", auto_code: None,
    example: r#"{"currency":"USD","rate_date":"2026-11-10","rate":121.5,"rate_type":"mid","source":"Bangladesh Bank"}"#,
};

async fn latest_rates(State(st): State<AppState>, user: AuthUser) -> AppResult<RawJson> {
    Ok(data(sqlx::query_scalar("SELECT coalesce(jsonb_agg(to_jsonb(x) ORDER BY x.currency, x.rate_type), '[]')::text FROM (SELECT DISTINCT ON (currency, rate_type) currency, rate_type, rate, rate_date, source FROM exchange_rates WHERE company_id = $1 ORDER BY currency, rate_type, rate_date DESC) x").bind(user.company_id).fetch_one(&st.db).await?))
}

// ---------------- bank and cash accounts ----------------
const BANK_SELECT: &str = "b.*, a.code AS gl_code, (SELECT coalesce(sum(l.debit - l.credit), 0) FROM journal_lines l WHERE l.account_id = b.account_id) AS balance_bdt, CASE WHEN b.currency = 'BDT' THEN NULL ELSE (SELECT coalesce(sum(l.fc_amount), 0) FROM journal_lines l WHERE l.account_id = b.account_id AND l.currency = b.currency) END AS balance_fc FROM bank_accounts b JOIN accounts a ON a.id = b.account_id WHERE b.company_id = ";

async fn list_banks(State(st): State<AppState>, user: AuthUser, Query(q): Query<HashMap<String, String>>) -> AppResult<RawJson> {
    let mut qb = db::list_query();
    qb.push(BANK_SELECT); qb.push_bind(user.company_id);
    db::filter_eq(&mut qb, &q, "kind", "b.kind", "");
    db::filter_eq(&mut qb, &q, "currency", "b.currency", "");
    db::page_json(&st.db, qb, "code", &q).await
}

#[derive(Deserialize)]
struct BankIn { code: String, name: String, kind: Option<String>, bank_name: Option<String>, branch_name: Option<String>, account_no: Option<String>, routing_no: Option<String>, swift_code: Option<String>, currency: Option<String>, branch_id: Option<Uuid> }

async fn create_bank(State(st): State<AppState>, user: AuthUser, Json(b): Json<BankIn>) -> AppResult<Created> {
    let code = b.code.trim().to_uppercase();
    if code.is_empty() || code.len() > 20 { return Err(AppError::field("code", "Enter a short code (up to 20 characters)")); }
    if b.name.trim().is_empty() { return Err(AppError::field("name", "This field is required")); }
    let kind = b.kind.clone().unwrap_or_else(|| "bank".into());
    if !["bank", "cash", "mobile"].contains(&kind.as_str()) { return Err(AppError::field("kind", "Must be bank, cash or mobile")); }
    let currency = b.currency.clone().unwrap_or_else(|| "BDT".into());
    let mut tx = st.db.begin().await?;
    let known: Option<i32> = sqlx::query_scalar("SELECT 1 FROM currencies WHERE code = $1").bind(&currency).fetch_optional(&mut *tx).await?;
    if known.is_none() { return Err(AppError::field("currency", "Unknown currency")); }
    if let Some(br) = b.branch_id { db::ensure(&mut tx, "branches", user.company_id, br, "branch_id").await?; }
    // Each bank or cash account gets its own ledger account under "Bank Accounts".
    let parent = accounting::account_id(&mut tx, user.company_id, "bank_parent").await?;
    let gl: Uuid = sqlx::query_scalar("INSERT INTO accounts (company_id, code, name, account_type, parent_id) VALUES ($1, $2, $3, 'asset', $4) RETURNING id")
        .bind(user.company_id).bind(format!("1020-{code}")).bind(b.name.trim()).bind(parent).fetch_one(&mut *tx).await?;
    let id: Uuid = sqlx::query_scalar("INSERT INTO bank_accounts (company_id, branch_id, account_id, code, name, kind, bank_name, branch_name, account_no, routing_no, swift_code, currency) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12) RETURNING id")
        .bind(user.company_id).bind(b.branch_id).bind(gl).bind(&code).bind(b.name.trim()).bind(&kind).bind(&b.bank_name).bind(&b.branch_name).bind(&b.account_no).bind(&b.routing_no).bind(&b.swift_code).bind(&currency).fetch_one(&mut *tx).await?;
    db::audit(&mut tx, &user, "create", "bank_accounts", id, json!({"code": code, "currency": currency})).await?;
    let doc: String = sqlx::query_scalar(&format!("SELECT to_jsonb(x)::text FROM (SELECT {BANK_SELECT} $1 AND b.id = $2) x")).bind(user.company_id).bind(id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(created(doc))
}

static BANK_EDIT: Resource = Resource {
    path: "/bank-accounts", table: "bank_accounts", tag: "Cash & FX", label: "bank account", read: "finance.read", write: "finance.write",
    fields: &[opt("name", K::Text(120)), opt("bank_name", K::Text(120)), opt("branch_name", K::Text(120)), opt("account_no", K::Text(40)), opt("routing_no", K::Text(20)), opt("swift_code", K::Text(20)), opt("is_active", K::Bool)],
    search: &[], filters: &[], order: "code", extra: "", auto_code: None, example: "",
};
async fn update_bank(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>, Json(b): Json<Value>) -> AppResult<RawJson> {
    let mut tx = st.db.begin().await?;
    let vals = resource::validate(&mut tx, &BANK_EDIT, user.company_id, &b, false).await?;
    if vals.is_empty() { return Err(AppError::BadRequest("Nothing to update".into())); }
    let sets = vals.keys().map(|c| format!("{c} = r.{c}")).collect::<Vec<_>>().join(", ");
    let n = sqlx::query(&format!("UPDATE bank_accounts t SET {sets} FROM jsonb_populate_record(NULL::bank_accounts, $1) r WHERE t.id = $2 AND t.company_id = $3")).bind(Value::Object(vals.clone())).bind(id).bind(user.company_id).execute(&mut *tx).await?.rows_affected();
    if n == 0 { return Err(AppError::not_found("bank account")); }
    db::audit(&mut tx, &user, "update", "bank_accounts", id, Value::Object(vals)).await?;
    let doc: String = sqlx::query_scalar(&format!("SELECT to_jsonb(x)::text FROM (SELECT {BANK_SELECT} $1 AND b.id = $2) x")).bind(user.company_id).bind(id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(data(doc))
}

async fn get_bank(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> AppResult<RawJson> {
    Ok(data(sqlx::query_scalar(&format!("SELECT to_jsonb(x)::text FROM (SELECT {BANK_SELECT} $1 AND b.id = $2) x")).bind(user.company_id).bind(id).fetch_optional(&st.db).await?.ok_or_else(|| AppError::not_found("bank account"))?))
}

async fn bank_statement(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>, Query(q): Query<HashMap<String, String>>) -> AppResult<RawJson> {
    let gl: Uuid = sqlx::query_scalar("SELECT account_id FROM bank_accounts WHERE id = $1 AND company_id = $2").bind(id).bind(user.company_id).fetch_optional(&st.db).await?.ok_or_else(|| AppError::not_found("bank account"))?;
    let mut qb = db::list_query();
    qb.push("e.entry_date, e.entry_no, e.source_type, e.source_id, coalesce(l.memo, e.memo) AS memo, l.debit AS money_in, l.credit AS money_out, l.fc_amount, sum(l.debit - l.credit) OVER (ORDER BY e.entry_date, e.entry_no, l.line_no) AS balance FROM journal_lines l JOIN journal_entries e ON e.id = l.entry_id WHERE l.account_id = ");
    qb.push_bind(gl);
    db::filter_dates(&mut qb, &q, "e.entry_date");
    db::page_json(&st.db, qb, "entry_date DESC, entry_no DESC", &q).await
}

// ---------------- transfers ----------------
#[derive(Deserialize)]
struct TransferIn { from_bank_account_id: Uuid, to_bank_account_id: Uuid, transfer_date: Option<NaiveDate>, amount: Decimal, to_amount: Option<Decimal>, #[serde(default)] charges: Decimal, reference: Option<String> }

async fn create_transfer(State(st): State<AppState>, user: AuthUser, Json(b): Json<TransferIn>) -> AppResult<Created> {
    if b.amount <= Decimal::ZERO { return Err(AppError::field("amount", "Must be more than zero")); }
    if b.charges < Decimal::ZERO { return Err(AppError::field("charges", "Must be zero or more")); }
    if b.from_bank_account_id == b.to_bank_account_id { return Err(AppError::field("to_bank_account_id", "Choose a different account")); }
    let mut tx = st.db.begin().await?;
    let from = bank(&mut tx, user.company_id, b.from_bank_account_id, "from_bank_account_id").await?;
    let to = bank(&mut tx, user.company_id, b.to_bank_account_id, "to_bank_account_id").await?;
    let date = match b.transfer_date { Some(d) => d, None => db::today(&mut tx, user.company_id).await? };
    let to_amount = match (from.currency == to.currency, b.to_amount) {
        (true, _) => b.amount,
        (false, Some(t)) if t > Decimal::ZERO => t,
        _ => return Err(AppError::field("to_amount", "Enter the amount received in the destination currency")),
    };
    // Value the movement in BDT: use the BDT side when there is one, else the source currency rate.
    let bdt = if from.currency == "BDT" { b.amount } else if to.currency == "BDT" { to_amount } else { r2(b.amount * rate_on(&mut tx, user.company_id, &from.currency, date).await?) };
    ensure_funds(&mut tx, user.company_id, &from, bdt + b.charges).await?;
    let no = db::next_no(&mut tx, user.company_id, "bank_transfer", "TRF").await?;
    let id: Uuid = sqlx::query_scalar("INSERT INTO bank_transfers (company_id, transfer_no, transfer_date, from_bank_account_id, to_bank_account_id, amount, to_amount, bdt_amount, charges, reference, created_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11) RETURNING id")
        .bind(user.company_id).bind(&no).bind(date).bind(b.from_bank_account_id).bind(b.to_bank_account_id).bind(r2(b.amount)).bind(r2(to_amount)).bind(bdt).bind(r2(b.charges)).bind(&b.reference).bind(user.user_id).fetch_one(&mut *tx).await?;
    let mut dr = JL::dr(Acc::Id(to.gl), bdt); if to.currency != "BDT" { dr = dr.fc(&to.currency, to_amount); }
    let mut cr = JL::cr(Acc::Id(from.gl), bdt); if from.currency != "BDT" { cr = cr.fc(&from.currency, b.amount); }
    let je = accounting::post(&mut tx, Post { company: user.company_id, branch: None, date, source_type: "bank_transfer", source_id: Some(id), memo: &format!("Transfer {no}: {} to {}", from.name, to.name), user: user.user_id },
        vec![dr, cr, JL::dr(Acc::Key("bank_charges"), b.charges), JL::cr(Acc::Id(from.gl), b.charges)]).await?;
    sqlx::query("UPDATE bank_transfers SET journal_entry_id = $2 WHERE id = $1").bind(id).bind(je).execute(&mut *tx).await?;
    db::audit(&mut tx, &user, "post", "bank_transfers", id, json!({"transfer_no": no, "bdt": bdt})).await?;
    let doc: String = sqlx::query_scalar("SELECT to_jsonb(t)::text FROM bank_transfers t WHERE id = $1").bind(id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(created(doc))
}

async fn list_transfers(State(st): State<AppState>, user: AuthUser, Query(q): Query<HashMap<String, String>>) -> AppResult<RawJson> {
    let mut qb = db::list_query();
    qb.push("t.*, (SELECT name FROM bank_accounts b WHERE b.id = t.from_bank_account_id) AS from_account, (SELECT name FROM bank_accounts b WHERE b.id = t.to_bank_account_id) AS to_account FROM bank_transfers t WHERE t.company_id = ");
    qb.push_bind(user.company_id);
    db::filter_dates(&mut qb, &q, "t.transfer_date");
    db::page_json(&st.db, qb, "created_at DESC", &q).await
}

// ---------------- supplier payments ----------------
#[derive(Deserialize)]
struct Alloc { bill_id: Uuid, amount: Decimal }
#[derive(Deserialize)]
struct PayIn { supplier_id: Uuid, bank_account_id: Uuid, payment_date: Option<NaiveDate>, method: Option<String>, reference: Option<String>, tds_amount: Option<Decimal>, vds_amount: Option<Decimal>, allocations: Vec<Alloc> }

const PAY_DOC: &str = "SELECT (to_jsonb(p) || jsonb_build_object('supplier_name', s.name, 'bank_account', (SELECT name FROM bank_accounts b WHERE b.id = p.bank_account_id),
  'allocations', (SELECT coalesce(jsonb_agg(jsonb_build_object('bill_id', a.bill_id, 'bill_no', b.bill_no, 'amount', a.amount)), '[]') FROM supplier_payment_allocations a JOIN supplier_bills b ON b.id = a.bill_id WHERE a.payment_id = p.id)))::text
  FROM supplier_payments p JOIN suppliers s ON s.id = p.supplier_id WHERE p.id = $1 AND p.company_id = $2";

/// Post a stored supplier payment: settle bills, withhold TDS/VDS, pay the net from the bank.
pub async fn post_supplier_payment(conn: &mut PgConnection, user: &AuthUser, id: Uuid) -> AppResult<()> {
    let (no, supplier, bank_id, date, gross, tds, vds, net): (String, Uuid, Uuid, NaiveDate, Decimal, Decimal, Decimal, Decimal) = sqlx::query_as("SELECT payment_no, supplier_id, bank_account_id, payment_date, gross_amount, tds_amount, vds_amount, net_amount FROM supplier_payments WHERE id = $1 AND company_id = $2 FOR UPDATE").bind(id).bind(user.company_id).fetch_one(&mut *conn).await?;
    let allocs: Vec<(Uuid, Decimal)> = sqlx::query_as("SELECT bill_id, amount FROM supplier_payment_allocations WHERE payment_id = $1").bind(id).fetch_all(&mut *conn).await?;
    for (bill, amount) in allocs {
        let (bno, total, paid, status): (String, Decimal, Decimal, String) = sqlx::query_as("SELECT bill_no, total, paid_amount, status FROM supplier_bills WHERE id = $1 FOR UPDATE").bind(bill).fetch_one(&mut *conn).await?;
        if status == "cancelled" || amount > total - paid { return Err(AppError::rule(format!("Bill {bno} has only Tk {} left to pay", total - paid))); }
        sqlx::query("UPDATE supplier_bills SET paid_amount = paid_amount + $2, status = CASE WHEN paid_amount + $2 >= total THEN 'paid' ELSE 'partial' END WHERE id = $1").bind(bill).bind(amount).execute(&mut *conn).await?;
    }
    let b = bank(conn, user.company_id, bank_id, "bank_account_id").await?;
    ensure_funds(conn, user.company_id, &b, net).await?;
    let je = accounting::post(conn, Post { company: user.company_id, branch: None, date, source_type: "supplier_payment", source_id: Some(id), memo: &format!("Supplier payment {no}"), user: user.user_id }, vec![
        JL::dr(Acc::Key("ap_local"), gross).party("supplier", supplier),
        JL::cr(Acc::Id(b.gl), net),
        JL::cr(Acc::Key("tds_payable"), tds).party("supplier", supplier),
        JL::cr(Acc::Key("vds_payable"), vds).party("supplier", supplier),
    ]).await?;
    sqlx::query("UPDATE supplier_payments SET status = 'posted', journal_entry_id = $2 WHERE id = $1").bind(id).bind(je).execute(&mut *conn).await?;
    Ok(())
}

pub async fn on_payment_approval(conn: &mut PgConnection, user: &AuthUser, id: Uuid, approved: bool) -> AppResult<()> {
    if approved { post_supplier_payment(conn, user, id).await } else {
        sqlx::query("UPDATE supplier_payments SET status = 'rejected' WHERE id = $1 AND company_id = $2").bind(id).bind(user.company_id).execute(&mut *conn).await?;
        Ok(())
    }
}

async fn create_payment(State(st): State<AppState>, user: AuthUser, Json(b): Json<PayIn>) -> AppResult<Created> {
    if b.allocations.is_empty() { return Err(AppError::field("allocations", "Choose at least one bill to pay")); }
    let method = b.method.clone().unwrap_or_else(|| "bank".into());
    if !["bank", "cash", "cheque", "bkash", "other"].contains(&method.as_str()) { return Err(AppError::field("method", "Unknown payment method")); }
    let mut tx = st.db.begin().await?;
    let (tds_rate, vds_rate): (Decimal, Decimal) = sqlx::query_as("SELECT tds_rate, vds_rate FROM suppliers WHERE id = $1 AND company_id = $2").bind(b.supplier_id).bind(user.company_id).fetch_optional(&mut *tx).await?.ok_or_else(|| AppError::field("supplier_id", "Supplier does not exist"))?;
    let bk = bank(&mut tx, user.company_id, b.bank_account_id, "bank_account_id").await?;
    if bk.currency != "BDT" { return Err(AppError::field("bank_account_id", "Pay local suppliers from a BDT account")); }
    let date = match b.payment_date { Some(d) => d, None => db::today(&mut tx, user.company_id).await? };
    let (mut gross, mut base) = (Decimal::ZERO, Decimal::ZERO);
    for (i, a) in b.allocations.iter().enumerate() {
        if a.amount <= Decimal::ZERO { return Err(AppError::field(&format!("allocations[{i}].amount"), "Must be more than zero")); }
        let (supplier, subtotal, total, paid, status): (Uuid, Decimal, Decimal, Decimal, String) = sqlx::query_as("SELECT supplier_id, subtotal, total, paid_amount, status FROM supplier_bills WHERE id = $1 AND company_id = $2").bind(a.bill_id).bind(user.company_id).fetch_optional(&mut *tx).await?.ok_or_else(|| AppError::field(&format!("allocations[{i}].bill_id"), "Bill does not exist"))?;
        if supplier != b.supplier_id { return Err(AppError::field(&format!("allocations[{i}].bill_id"), "This bill belongs to another supplier")); }
        if status == "cancelled" || r2(a.amount) > total - paid { return Err(AppError::field(&format!("allocations[{i}].amount"), format!("Only Tk {} is left to pay on this bill", total - paid))); }
        gross += r2(a.amount);
        base += r2(a.amount) * subtotal / total; // withholding applies to the value before VAT
    }
    let tds = r2(b.tds_amount.unwrap_or_else(|| pct(base, tds_rate)));
    let vds = r2(b.vds_amount.unwrap_or_else(|| pct(base, vds_rate)));
    if tds < Decimal::ZERO || vds < Decimal::ZERO || tds + vds >= gross { return Err(AppError::field("tds_amount", "Withholding must be zero or more and less than the payment")); }
    let net = gross - tds - vds;
    let no = db::next_no(&mut tx, user.company_id, "supplier_payment", "PAY").await?;
    let id: Uuid = sqlx::query_scalar("INSERT INTO supplier_payments (company_id, payment_no, supplier_id, bank_account_id, payment_date, gross_amount, tds_amount, vds_amount, net_amount, method, reference, status, created_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,'pending_approval',$12) RETURNING id")
        .bind(user.company_id).bind(&no).bind(b.supplier_id).bind(b.bank_account_id).bind(date).bind(gross).bind(tds).bind(vds).bind(net).bind(&method).bind(&b.reference).bind(user.user_id).fetch_one(&mut *tx).await?;
    for a in &b.allocations {
        sqlx::query("INSERT INTO supplier_payment_allocations (payment_id, bill_id, amount) VALUES ($1,$2,$3)").bind(id).bind(a.bill_id).bind(r2(a.amount)).execute(&mut *tx).await.map_err(|_| AppError::field("allocations", "Each bill may appear only once"))?;
    }
    let needs = approvals::submit(&mut tx, &user, "supplier_payment", id, &no, gross).await?;
    if !needs { post_supplier_payment(&mut tx, &user, id).await?; }
    db::audit(&mut tx, &user, if needs { "submit" } else { "post" }, "supplier_payments", id, json!({"payment_no": no, "gross": gross, "tds": tds, "vds": vds})).await?;
    let doc: String = sqlx::query_scalar(PAY_DOC).bind(id).bind(user.company_id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(created(doc))
}

async fn list_payments(State(st): State<AppState>, user: AuthUser, Query(q): Query<HashMap<String, String>>) -> AppResult<RawJson> {
    let mut qb = db::list_query();
    qb.push("p.*, s.name AS supplier_name, (SELECT name FROM bank_accounts b WHERE b.id = p.bank_account_id) AS bank_account FROM supplier_payments p JOIN suppliers s ON s.id = p.supplier_id WHERE p.company_id = ");
    qb.push_bind(user.company_id);
    db::filter_eq(&mut qb, &q, "supplier_id", "p.supplier_id", "::uuid");
    db::filter_eq(&mut qb, &q, "status", "p.status", "");
    db::filter_dates(&mut qb, &q, "p.payment_date");
    db::page_json(&st.db, qb, "created_at DESC", &q).await
}
async fn get_payment(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> AppResult<RawJson> {
    Ok(data(sqlx::query_scalar(PAY_DOC).bind(id).bind(user.company_id).fetch_optional(&st.db).await?.ok_or_else(|| AppError::not_found("payment"))?))
}

// ---------------- FX exposure and revaluation ----------------
async fn fx_exposure(State(st): State<AppState>, user: AuthUser) -> AppResult<Json<Value>> {
    let mut conn = st.db.acquire().await?;
    let today = db::today(&mut conn, user.company_id).await?;
    // Three sources of future foreign-currency outflow.
    let rows: Vec<(String, Decimal, Decimal, Decimal, Decimal)> = sqlx::query_as(
        "WITH ap AS (SELECT l.currency, -sum(l.fc_amount) AS fc, sum(l.credit - l.debit) AS book FROM journal_lines l JOIN accounts a ON a.id = l.account_id AND a.system_key = 'ap_foreign' WHERE l.company_id = $1 AND l.currency IS NOT NULL GROUP BY l.currency),
              adv AS (SELECT l.currency, sum(l.fc_amount) AS fc FROM journal_lines l JOIN accounts a ON a.id = l.account_id AND a.system_key = 'supplier_advance' WHERE l.company_id = $1 AND l.currency IS NOT NULL GROUP BY l.currency),
              po AS (SELECT o.currency, sum((l.qty - l.shipped_qty) * l.unit_price) AS fc FROM purchase_orders o JOIN purchase_order_lines l ON l.po_id = o.id WHERE o.company_id = $1 AND o.po_type = 'import' AND o.status IN ('approved','partially_received') AND l.qty > l.shipped_qty GROUP BY o.currency),
              cur AS (SELECT currency FROM ap UNION SELECT currency FROM po UNION SELECT currency FROM adv)
         SELECT c.currency, coalesce(ap.fc, 0), coalesce(ap.book, 0), coalesce(po.fc, 0), coalesce(adv.fc, 0) FROM cur c LEFT JOIN ap USING (currency) LEFT JOIN po USING (currency) LEFT JOIN adv USING (currency) ORDER BY 1",
    ).bind(user.company_id).fetch_all(&mut *conn).await?;
    let mut out = vec![]; let mut total = Decimal::ZERO;
    for (cur, payable, book, open_po, advance) in rows {
        let rate = rate_on(&mut conn, user.company_id, &cur, today).await.unwrap_or(Decimal::ZERO);
        let net_fc = payable + open_po - advance;
        let bdt = r2(net_fc * rate);
        total += bdt;
        out.push(json!({"currency": cur, "payables_fc": payable, "payables_book_bdt": book, "open_orders_fc": r2(open_po), "advances_fc": advance, "net_exposure_fc": r2(net_fc), "rate": rate, "net_exposure_bdt": bdt,
            "unrealised_loss_bdt": r2(payable * rate) - book, "impact_of_1pct_bdt": r2(bdt / Decimal::ONE_HUNDRED)}));
    }
    let lcs: Value = sqlx::query_scalar("SELECT coalesce(jsonb_agg(jsonb_build_object('lc_no', lc_no, 'currency', currency, 'amount', amount, 'outstanding', amount - settled_amount, 'expiry_date', expiry_date, 'margin_bdt', margin_bdt) ORDER BY expiry_date), '[]') FROM letters_of_credit WHERE company_id = $1 AND status = 'opened'").bind(user.company_id).fetch_one(&mut *conn).await?;
    Ok(Json(json!({"data": {"as_of": today, "by_currency": out, "total_exposure_bdt": total, "open_lcs": lcs}})))
}

#[derive(Deserialize)]
struct RevalueIn { date: Option<NaiveDate> }
/// Revalue open foreign payables and foreign-currency bank balances at the current rate.
async fn fx_revalue(State(st): State<AppState>, user: AuthUser, Json(b): Json<RevalueIn>) -> AppResult<Json<Value>> {
    let mut tx = st.db.begin().await?;
    let date = match b.date { Some(d) => d, None => db::today(&mut tx, user.company_id).await? };
    let mut lines = vec![]; let mut net_loss = Decimal::ZERO; let mut detail = vec![];
    // Each open foreign invoice (one per shipment) is revalued on its own, so later payments
    // find the adjusted book value on the same shipment.
    let ap: Vec<(Uuid, Uuid, String, Decimal, Decimal)> = sqlx::query_as("SELECT l.ref_id, max(l.party_id::text)::uuid, max(l.currency), -coalesce(sum(l.fc_amount), 0), sum(l.credit - l.debit) FROM journal_lines l JOIN accounts a ON a.id = l.account_id AND a.system_key = 'ap_foreign' JOIN journal_entries e ON e.id = l.entry_id AND e.entry_date <= $2 WHERE l.company_id = $1 AND l.ref_type = 'shipment' GROUP BY l.ref_id HAVING coalesce(sum(l.fc_amount), 0) <> 0").bind(user.company_id).bind(date).fetch_all(&mut *tx).await?;
    for (shipment, party, cur, fc, book) in ap {
        let rate = rate_on(&mut tx, user.company_id, &cur, date).await?;
        let diff = r2(fc * rate) - book; // payable grew = loss
        if diff.is_zero() { continue; }
        lines.push(JL::cr(Acc::Key("ap_foreign"), diff).party("supplier", party).tag("shipment", shipment).memo(format!("Revalue {cur} {fc} at {rate}")));
        net_loss += diff;
        detail.push(json!({"kind": "payable", "shipment_id": shipment, "supplier_id": party, "currency": cur, "fc": fc, "rate": rate, "loss": diff}));
    }
    let banks: Vec<(Uuid, String, String, Decimal, Decimal)> = sqlx::query_as("SELECT b.account_id, b.name, b.currency, coalesce(sum(l.fc_amount), 0), coalesce(sum(l.debit - l.credit), 0) FROM bank_accounts b LEFT JOIN journal_lines l ON l.account_id = b.account_id LEFT JOIN journal_entries e ON e.id = l.entry_id WHERE b.company_id = $1 AND b.currency <> 'BDT' AND (e.entry_date IS NULL OR e.entry_date <= $2) GROUP BY 1, 2, 3").bind(user.company_id).bind(date).fetch_all(&mut *tx).await?;
    for (gl, name, cur, fc, book) in banks {
        let rate = rate_on(&mut tx, user.company_id, &cur, date).await?;
        let diff = r2(fc * rate) - book; // asset grew = gain
        if diff.is_zero() { continue; }
        lines.push(JL::dr(Acc::Id(gl), diff).memo(format!("Revalue {cur} {fc} at {rate}")));
        net_loss -= diff;
        detail.push(json!({"kind": "bank", "account": name, "currency": cur, "fc": fc, "rate": rate, "gain": diff}));
    }
    if lines.is_empty() { return Ok(Json(json!({"data": {"posted": false, "message": "Nothing to revalue", "net_loss": 0}}))); }
    lines.push(JL::dr(Acc::Key("fx_gain_loss"), net_loss));
    let je = accounting::post(&mut tx, Post { company: user.company_id, branch: None, date, source_type: "fx_revaluation", source_id: None, memo: &format!("FX revaluation as of {date}"), user: user.user_id }, lines).await?;
    db::audit(&mut tx, &user, "post", "fx_revaluation", je, json!({"net_loss": net_loss})).await?;
    tx.commit().await?;
    Ok(Json(json!({"data": {"posted": true, "journal_entry_id": je, "net_loss": net_loss, "lines": detail}})))
}

pub fn api() -> Api {
    resource::routes(Api::new("Cash & FX"), &RATES)
        .get("/exchange-rates-latest", "finance.read", "Latest rate per currency and type", latest_rates)
        .get("/bank-accounts", "finance.read", "Bank, cash and mobile accounts with balances", list_banks).query("page,per_page,kind,currency")
        .post("/bank-accounts", "finance.write", "Add a bank or cash account (creates its ledger account)", create_bank)
        .body(r#"{"code":"EBL-CA","name":"Eastern Bank Current A/C","kind":"bank","bank_name":"Eastern Bank PLC","branch_name":"Motijheel","account_no":"1011220001234","routing_no":"095274241","currency":"BDT"}"#)
        .get("/bank-accounts/:id", "finance.read", "One bank or cash account with balance", get_bank)
        .put("/bank-accounts/:id", "finance.write", "Update bank account details", update_bank)
        .get("/bank-accounts/:id/statement", "finance.read", "Account statement with running balance", bank_statement).query("page,per_page,from,to")
        .get("/bank-transfers", "finance.read", "Transfers between own accounts", list_transfers).query("page,per_page,from,to")
        .post("/bank-transfers", "finance.write", "Move money between own accounts (any currency)", create_transfer)
        .body(r#"{"from_bank_account_id":"<uuid>","to_bank_account_id":"<uuid>","transfer_date":"2026-11-12","amount":500000,"charges":115,"reference":"Cash deposit"}"#)
        .get("/supplier-payments", "finance.read", "Payments to local suppliers", list_payments).query("page,per_page,supplier_id,status,from,to")
        .get("/supplier-payments/:id", "finance.read", "One supplier payment", get_payment)
        .post("/supplier-payments", "finance.write", "Pay supplier bills; TDS and VDS are withheld automatically (may need approval)", create_payment)
        .body(r#"{"supplier_id":"<uuid>","bank_account_id":"<uuid>","payment_date":"2026-11-20","method":"cheque","reference":"CHQ 004512","allocations":[{"bill_id":"<uuid>","amount":287500}]}"#)
        .get("/fx/exposure", "finance.read", "Open foreign-currency exposure by currency, with open LCs", fx_exposure)
        .post("/fx/revalue", "finance.post", "Revalue foreign payables and FC bank balances at the latest rate", fx_revalue).body(r#"{"date":"2026-11-30"}"#)
}
