//! Import operations: letters of credit, shipments, containers, voyage tracking,
//! import documents and foreign payments (TT and LC settlement).
//!
//! Accounting model
//! - TT advance:      Dr Advance to Suppliers / Cr Bank (tagged to the purchase order)
//! - LC margin:       Dr LC Margin / Cr Bank;  LC charges: Dr Goods in Transit / Cr Bank
//! - Goods shipped:   Dr Goods in Transit / Cr Advance (prepaid part, at the advance rate) and
//!                    Cr Accounts Payable - Foreign (rest, at the bill-of-lading rate)
//! - Payment:         Dr Accounts Payable / Cr Bank (+ LC margin released), difference to FX gain/loss
use crate::{api::Api, auth::AuthUser, db::{self, created, data, r2, Created, RawJson}, error::{AppError, AppResult}, modules::{accounting::{self, Acc, Post, JL}, approvals, finance}, state::AppState};
use axum::{extract::{Path, Query, State}, Json};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::json;
use sqlx::PgConnection;
use std::collections::HashMap;
use uuid::Uuid;

/// Foreign-currency and book (BDT) balance of a ledger account for one tag, e.g. the payable of a shipment.
/// Returned with the account's natural sign (positive = balance outstanding).
pub async fn tagged_balance(conn: &mut PgConnection, company: Uuid, key: &str, tag: (&str, Uuid), credit_normal: bool) -> AppResult<(Decimal, Decimal)> {
    let (fc, bdt): (Decimal, Decimal) = sqlx::query_as("SELECT coalesce(sum(l.fc_amount), 0), coalesce(sum(l.debit - l.credit), 0) FROM journal_lines l JOIN accounts a ON a.id = l.account_id WHERE l.company_id = $1 AND a.system_key = $2 AND l.ref_type = $3 AND l.ref_id = $4")
        .bind(company).bind(key).bind(tag.0).bind(tag.1).fetch_one(&mut *conn).await?;
    Ok(if credit_normal { (-fc, -bdt) } else { (fc, bdt) })
}

// ---------------- letters of credit ----------------
#[derive(Deserialize)]
struct LcIn { po_id: Uuid, bank_account_id: Uuid, lc_type: Option<String>, amount: Option<Decimal>, #[serde(default)] tolerance_pct: Decimal, #[serde(default)] margin_pct: Decimal, bank_lc_no: Option<String>, advising_bank: Option<String>, expiry_date: Option<NaiveDate>, latest_shipment_date: Option<NaiveDate>, #[serde(default)] usance_days: i32, insurance_cover_note: Option<String>, notes: Option<String> }

const LC_DOC: &str = "SELECT (to_jsonb(c) || jsonb_build_object('po_no', o.po_no, 'supplier_name', s.name, 'bank_account', (SELECT name FROM bank_accounts b WHERE b.id = c.bank_account_id), 'outstanding', c.amount - c.settled_amount,
  'amendments', (SELECT coalesce(jsonb_agg(to_jsonb(a) ORDER BY a.amendment_no), '[]') FROM lc_amendments a WHERE a.lc_id = c.id),
  'payments', (SELECT coalesce(jsonb_agg(jsonb_build_object('id', p.id, 'payment_no', p.payment_no, 'kind', p.kind, 'payment_date', p.payment_date, 'fc_amount', p.fc_amount, 'bdt_amount', p.bdt_amount, 'status', p.status) ORDER BY p.created_at), '[]') FROM foreign_payments p WHERE p.lc_id = c.id),
  'shipments', (SELECT coalesce(jsonb_agg(jsonb_build_object('id', h.id, 'shipment_no', h.shipment_no, 'status', h.status, 'invoice_value', h.invoice_value)), '[]') FROM shipments h WHERE h.lc_id = c.id)))::text
  FROM letters_of_credit c JOIN purchase_orders o ON o.id = c.po_id JOIN suppliers s ON s.id = c.supplier_id WHERE c.id = $1 AND c.company_id = $2";

async fn lc_doc(conn: &mut PgConnection, company: Uuid, id: Uuid) -> AppResult<String> {
    sqlx::query_scalar(LC_DOC).bind(id).bind(company).fetch_optional(&mut *conn).await?.ok_or_else(|| AppError::not_found("letter of credit"))
}

async fn create_lc(State(st): State<AppState>, user: AuthUser, Json(b): Json<LcIn>) -> AppResult<Created> {
    let mut tx = st.db.begin().await?;
    let (status, po_type, supplier, currency, total, po_no): (String, String, Uuid, String, Decimal, String) = sqlx::query_as("SELECT status, po_type, supplier_id, currency, total, po_no FROM purchase_orders WHERE id = $1 AND company_id = $2").bind(b.po_id).bind(user.company_id).fetch_optional(&mut *tx).await?.ok_or_else(|| AppError::field("po_id", "Purchase order does not exist"))?;
    if po_type != "import" { return Err(AppError::field("po_id", "An LC can only be opened for an import order")); }
    if status != "approved" && status != "partially_received" { return Err(AppError::rule(format!("Order {po_no} is {status}; approve it before opening an LC"))); }
    let bank = finance::bank(&mut tx, user.company_id, b.bank_account_id, "bank_account_id").await?;
    if bank.kind != "bank" || bank.currency != "BDT" { return Err(AppError::field("bank_account_id", "Choose a BDT bank account")); }
    let lc_type = b.lc_type.clone().unwrap_or_else(|| "sight".into());
    if !["sight", "usance", "deferred", "upas"].contains(&lc_type.as_str()) { return Err(AppError::field("lc_type", "Must be sight, usance, deferred or upas")); }
    let amount = r2(b.amount.unwrap_or(total));
    if amount <= Decimal::ZERO { return Err(AppError::field("amount", "Must be more than zero")); }
    if b.margin_pct < Decimal::ZERO || b.margin_pct > Decimal::ONE_HUNDRED { return Err(AppError::field("margin_pct", "Must be between 0 and 100")); }
    if b.tolerance_pct < Decimal::ZERO || b.tolerance_pct > Decimal::from(10) { return Err(AppError::field("tolerance_pct", "Must be between 0 and 10")); }
    if let (Some(e), Some(s)) = (b.expiry_date, b.latest_shipment_date) { if s > e { return Err(AppError::field("latest_shipment_date", "Must be on or before the expiry date")); } }
    let no = db::next_no(&mut tx, user.company_id, "lc", "LC").await?;
    let id: Uuid = sqlx::query_scalar("INSERT INTO letters_of_credit (company_id, lc_no, bank_lc_no, po_id, supplier_id, bank_account_id, advising_bank, lc_type, currency, amount, tolerance_pct, margin_pct, expiry_date, latest_shipment_date, usance_days, insurance_cover_note, notes, created_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18) RETURNING id")
        .bind(user.company_id).bind(&no).bind(&b.bank_lc_no).bind(b.po_id).bind(supplier).bind(b.bank_account_id).bind(&b.advising_bank).bind(&lc_type).bind(&currency).bind(amount).bind(b.tolerance_pct).bind(b.margin_pct).bind(b.expiry_date).bind(b.latest_shipment_date).bind(b.usance_days).bind(&b.insurance_cover_note).bind(&b.notes).bind(user.user_id).fetch_one(&mut *tx).await?;
    db::audit(&mut tx, &user, "create", "letters_of_credit", id, json!({"lc_no": no, "amount": amount, "currency": currency})).await?;
    let doc = lc_doc(&mut tx, user.company_id, id).await?;
    tx.commit().await?;
    Ok(created(doc))
}

/// Record a posted bank debit related to an LC (margin or charges).
async fn lc_bank_payment(conn: &mut PgConnection, user: &AuthUser, lc: Uuid, kind: &str, date: NaiveDate, amount: Decimal, memo: &str) -> AppResult<()> {
    if amount <= Decimal::ZERO { return Ok(()); }
    let (lc_no, po, supplier, bank_id, currency): (String, Uuid, Uuid, Uuid, String) = sqlx::query_as("SELECT lc_no, po_id, supplier_id, bank_account_id, currency FROM letters_of_credit WHERE id = $1").bind(lc).fetch_one(&mut *conn).await?;
    let bank = finance::bank(conn, user.company_id, bank_id, "bank_account_id").await?;
    let no = db::next_no(conn, user.company_id, "foreign_payment", "FP").await?;
    let id: Uuid = sqlx::query_scalar("INSERT INTO foreign_payments (company_id, payment_no, kind, po_id, lc_id, supplier_id, bank_account_id, payment_date, currency, bdt_amount, reference, created_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12) RETURNING id")
        .bind(user.company_id).bind(&no).bind(kind).bind(po).bind(lc).bind(supplier).bind(bank_id).bind(date).bind(&currency).bind(amount).bind(memo).bind(user.user_id).fetch_one(&mut *conn).await?;
    let debit = if kind == "lc_margin" { JL::dr(Acc::Key("lc_margin"), amount).tag("lc", lc) } else { JL::dr(Acc::Key("git"), amount).tag("lc", lc).memo(memo.to_string()) };
    let je = accounting::post(conn, Post { company: user.company_id, branch: None, date, source_type: "foreign_payment", source_id: Some(id), memo: &format!("{memo} - {lc_no}"), user: user.user_id }, vec![debit, JL::cr(Acc::Id(bank.gl), amount)]).await?;
    sqlx::query("UPDATE foreign_payments SET journal_entry_id = $2 WHERE id = $1").bind(id).bind(je).execute(&mut *conn).await?;
    Ok(())
}

#[derive(Deserialize)]
struct LcOpen { open_date: Option<NaiveDate>, bank_lc_no: Option<String>, fx_rate: Option<Decimal>, margin_bdt: Option<Decimal>, #[serde(default)] commission: Decimal, #[serde(default)] swift_charges: Decimal, #[serde(default)] other_charges: Decimal }

async fn open_lc(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>, Json(b): Json<LcOpen>) -> AppResult<RawJson> {
    let mut tx = st.db.begin().await?;
    let (status, lc_no, currency, amount, margin_pct): (String, String, String, Decimal, Decimal) = sqlx::query_as("SELECT status, lc_no, currency, amount, margin_pct FROM letters_of_credit WHERE id = $1 AND company_id = $2 FOR UPDATE").bind(id).bind(user.company_id).fetch_optional(&mut *tx).await?.ok_or_else(|| AppError::not_found("letter of credit"))?;
    if status != "draft" { return Err(AppError::rule(format!("{lc_no} is already {status}"))); }
    if b.commission < Decimal::ZERO || b.swift_charges < Decimal::ZERO || b.other_charges < Decimal::ZERO { return Err(AppError::field("commission", "Charges must be zero or more")); }
    let date = match b.open_date { Some(d) => d, None => db::today(&mut tx, user.company_id).await? };
    let rate = match b.fx_rate { Some(r) if r > Decimal::ZERO => r, Some(_) => return Err(AppError::field("fx_rate", "Must be more than zero")), None => finance::rate_on(&mut tx, user.company_id, &currency, date).await? };
    let margin = r2(b.margin_bdt.unwrap_or(amount * rate * margin_pct / Decimal::ONE_HUNDRED));
    if margin < Decimal::ZERO { return Err(AppError::field("margin_bdt", "Must be zero or more")); }
    sqlx::query("UPDATE letters_of_credit SET status = 'opened', open_date = $2, bank_lc_no = coalesce($3, bank_lc_no), margin_bdt = $4 WHERE id = $1").bind(id).bind(date).bind(&b.bank_lc_no).bind(margin).execute(&mut *tx).await?;
    lc_bank_payment(&mut tx, &user, id, "lc_margin", date, margin, "LC margin").await?;
    lc_bank_payment(&mut tx, &user, id, "lc_charges", date, r2(b.commission), "LC opening commission").await?;
    lc_bank_payment(&mut tx, &user, id, "lc_charges", date, r2(b.swift_charges), "SWIFT charges").await?;
    lc_bank_payment(&mut tx, &user, id, "lc_charges", date, r2(b.other_charges), "LC other charges").await?;
    db::audit(&mut tx, &user, "open", "letters_of_credit", id, json!({"margin_bdt": margin, "commission": b.commission, "rate": rate})).await?;
    let doc = lc_doc(&mut tx, user.company_id, id).await?;
    tx.commit().await?;
    Ok(data(doc))
}

#[derive(Deserialize)]
struct LcAmend { amount: Option<Decimal>, expiry_date: Option<NaiveDate>, latest_shipment_date: Option<NaiveDate>, tolerance_pct: Option<Decimal>, reason: String, #[serde(default)] charges: Decimal, amendment_date: Option<NaiveDate> }

async fn amend_lc(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>, Json(b): Json<LcAmend>) -> AppResult<RawJson> {
    if b.reason.trim().is_empty() { return Err(AppError::field("reason", "Please give a reason")); }
    let mut tx = st.db.begin().await?;
    let (status, settled): (String, Decimal) = sqlx::query_as("SELECT status, settled_amount FROM letters_of_credit WHERE id = $1 AND company_id = $2 FOR UPDATE").bind(id).bind(user.company_id).fetch_optional(&mut *tx).await?.ok_or_else(|| AppError::not_found("letter of credit"))?;
    if status != "opened" { return Err(AppError::rule("Only an opened LC can be amended")); }
    if let Some(a) = b.amount { if a <= Decimal::ZERO || a < settled { return Err(AppError::field("amount", "Must be more than zero and not below what is already settled")); } }
    let date = match b.amendment_date { Some(d) => d, None => db::today(&mut tx, user.company_id).await? };
    let changes = json!({"amount": b.amount, "expiry_date": b.expiry_date, "latest_shipment_date": b.latest_shipment_date, "tolerance_pct": b.tolerance_pct});
    sqlx::query("INSERT INTO lc_amendments (lc_id, amendment_no, amendment_date, changes, reason, created_by) VALUES ($1, (SELECT coalesce(max(amendment_no), 0) + 1 FROM lc_amendments WHERE lc_id = $1), $2, $3, $4, $5)").bind(id).bind(date).bind(&changes).bind(b.reason.trim()).bind(user.user_id).execute(&mut *tx).await?;
    sqlx::query("UPDATE letters_of_credit SET amount = coalesce($2, amount), expiry_date = coalesce($3, expiry_date), latest_shipment_date = coalesce($4, latest_shipment_date), tolerance_pct = coalesce($5, tolerance_pct) WHERE id = $1")
        .bind(id).bind(b.amount.map(r2)).bind(b.expiry_date).bind(b.latest_shipment_date).bind(b.tolerance_pct).execute(&mut *tx).await?;
    lc_bank_payment(&mut tx, &user, id, "lc_charges", date, r2(b.charges), "LC amendment charges").await?;
    db::audit(&mut tx, &user, "amend", "letters_of_credit", id, changes).await?;
    let doc = lc_doc(&mut tx, user.company_id, id).await?;
    tx.commit().await?;
    Ok(data(doc))
}

#[derive(Deserialize)]
struct LcClose { close_date: Option<NaiveDate>, cancel: Option<bool> }
/// Close an LC: any margin still held by the bank is released back to the account.
async fn close_lc(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>, body: Option<Json<LcClose>>) -> AppResult<RawJson> {
    let b = body.map(|b| b.0).unwrap_or(LcClose { close_date: None, cancel: None });
    let mut tx = st.db.begin().await?;
    let (status, lc_no, margin, bank_id, utilized, settled): (String, String, Decimal, Uuid, Decimal, Decimal) = sqlx::query_as("SELECT status, lc_no, margin_bdt, bank_account_id, utilized_amount, settled_amount FROM letters_of_credit WHERE id = $1 AND company_id = $2 FOR UPDATE").bind(id).bind(user.company_id).fetch_optional(&mut *tx).await?.ok_or_else(|| AppError::not_found("letter of credit"))?;
    let cancel = b.cancel.unwrap_or(false);
    match status.as_str() {
        "draft" if cancel => { sqlx::query("UPDATE letters_of_credit SET status = 'cancelled' WHERE id = $1").bind(id).execute(&mut *tx).await?; }
        "opened" => {
            if settled < utilized { return Err(AppError::rule(format!("{lc_no} still has documents to pay: {} of {} settled", settled, utilized))); }
            if cancel && utilized > Decimal::ZERO { return Err(AppError::rule("An LC with shipments cannot be cancelled; close it instead")); }
            let date = match b.close_date { Some(d) => d, None => db::today(&mut tx, user.company_id).await? };
            if margin > Decimal::ZERO {
                let bank = finance::bank(&mut tx, user.company_id, bank_id, "bank_account_id").await?;
                accounting::post(&mut tx, Post { company: user.company_id, branch: None, date, source_type: "lc_close", source_id: Some(id), memo: &format!("Margin released on closing {lc_no}"), user: user.user_id },
                    vec![JL::dr(Acc::Id(bank.gl), margin), JL::cr(Acc::Key("lc_margin"), margin).tag("lc", id)]).await?;
            }
            sqlx::query("UPDATE letters_of_credit SET status = $2, margin_bdt = 0 WHERE id = $1").bind(id).bind(if cancel { "cancelled" } else { "settled" }).execute(&mut *tx).await?;
        }
        _ => return Err(AppError::rule(format!("{lc_no} is {status} and cannot be closed"))),
    }
    db::audit(&mut tx, &user, if cancel { "cancel" } else { "close" }, "letters_of_credit", id, json!({"margin_released": margin})).await?;
    let doc = lc_doc(&mut tx, user.company_id, id).await?;
    tx.commit().await?;
    Ok(data(doc))
}

async fn list_lcs(State(st): State<AppState>, user: AuthUser, Query(q): Query<HashMap<String, String>>) -> AppResult<RawJson> {
    let mut qb = db::list_query();
    qb.push("c.id, c.lc_no, c.bank_lc_no, c.lc_type, c.currency, c.amount, c.utilized_amount, c.settled_amount, c.margin_bdt, c.open_date, c.expiry_date, c.latest_shipment_date, c.status, c.po_id, o.po_no, s.name AS supplier_name, CASE WHEN c.status = 'opened' AND c.expiry_date IS NOT NULL THEN c.expiry_date - (now() AT TIME ZONE 'Asia/Dhaka')::date END AS days_to_expiry FROM letters_of_credit c JOIN purchase_orders o ON o.id = c.po_id JOIN suppliers s ON s.id = c.supplier_id WHERE c.company_id = ");
    qb.push_bind(user.company_id);
    db::filter_eq(&mut qb, &q, "status", "c.status", "");
    db::filter_eq(&mut qb, &q, "po_id", "c.po_id", "::uuid");
    db::filter_search(&mut qb, &q, &["c.lc_no", "c.bank_lc_no", "s.name", "o.po_no"]);
    db::page_json(&st.db, qb, "created_at DESC", &q).await
}
async fn get_lc(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> AppResult<RawJson> {
    let mut conn = st.db.acquire().await?;
    Ok(data(lc_doc(&mut conn, user.company_id, id).await?))
}

// ---------------- shipments ----------------
#[derive(Deserialize)]
struct ShipLine { po_line_id: Uuid, qty: Decimal }
#[derive(Deserialize)]
struct ContainerIn { container_no: String, size: Option<String>, seal_no: Option<String>, gross_weight_kg: Option<Decimal>, packages: Option<i32> }
#[derive(Deserialize)]
struct ShipIn {
    po_id: Uuid, lc_id: Option<Uuid>, mode: Option<String>, bl_no: Option<String>, bl_date: Option<NaiveDate>, vessel_name: Option<String>, voyage_no: Option<String>, carrier: Option<String>,
    port_of_loading: Option<String>, port_of_discharge: Option<String>, etd: Option<NaiveDate>, eta: Option<NaiveDate>, commercial_invoice_no: Option<String>, commercial_invoice_date: Option<NaiveDate>,
    #[serde(default)] freight: Decimal, #[serde(default)] insurance: Decimal, free_days: Option<i32>, #[serde(default)] demurrage_per_day: Decimal, notes: Option<String>,
    lines: Vec<ShipLine>, #[serde(default)] containers: Vec<ContainerIn>,
}

pub const SHIPMENT_DOC: &str = "SELECT (to_jsonb(h) || jsonb_build_object('po_no', o.po_no, 'incoterm', o.incoterm, 'supplier_name', s.name, 'lc_no', (SELECT lc_no FROM letters_of_credit c WHERE c.id = h.lc_id),
  'lines', (SELECT coalesce(jsonb_agg(jsonb_build_object('id', l.id, 'po_line_id', l.po_line_id, 'product_id', l.product_id, 'sku', p.sku, 'product_name', p.name, 'hs_code', (SELECT code FROM hs_codes x WHERE x.id = l.hs_code_id), 'qty', l.qty, 'unit_price', l.unit_price, 'line_value', l.line_value, 'received_qty', l.received_qty)), '[]') FROM shipment_lines l JOIN products p ON p.id = l.product_id WHERE l.shipment_id = h.id),
  'containers', (SELECT coalesce(jsonb_agg(to_jsonb(c) || jsonb_build_object('days_over_free', CASE WHEN c.free_until IS NOT NULL AND c.delivered_at IS NULL THEN greatest(0, (now() AT TIME ZONE 'Asia/Dhaka')::date - c.free_until) ELSE greatest(0, coalesce(c.delivered_at - c.free_until, 0)) END) ORDER BY c.container_no), '[]') FROM containers c WHERE c.shipment_id = h.id),
  'events', (SELECT coalesce(jsonb_agg(jsonb_build_object('id', e.id, 'event_time', e.event_time, 'event_type', e.event_type, 'location', e.location, 'description', e.description, 'container_id', e.container_id, 'source', e.source) ORDER BY e.event_time), '[]') FROM tracking_events e WHERE e.shipment_id = h.id),
  'documents', (SELECT coalesce(jsonb_agg(jsonb_build_object('doc_type', d.doc_type, 'doc_no', d.doc_no, 'doc_date', d.doc_date, 'status', d.status, 'is_required', d.is_required, 'file_url', d.file_url, 'notes', d.notes) ORDER BY d.doc_type), '[]') FROM import_documents d WHERE d.shipment_id = h.id),
  'documents_missing', (SELECT count(*) FROM import_documents d WHERE d.shipment_id = h.id AND d.is_required AND d.status IN ('pending','requested')),
  'payments', (SELECT coalesce(jsonb_agg(jsonb_build_object('id', f.id, 'payment_no', f.payment_no, 'kind', f.kind, 'payment_date', f.payment_date, 'fc_amount', f.fc_amount, 'fx_rate', f.fx_rate, 'bdt_amount', f.bdt_amount, 'status', f.status) ORDER BY f.created_at), '[]') FROM foreign_payments f WHERE f.shipment_id = h.id),
  'payable_fc', (SELECT coalesce(-sum(jl.fc_amount), 0) FROM journal_lines jl JOIN accounts a ON a.id = jl.account_id AND a.system_key = 'ap_foreign' WHERE jl.ref_type = 'shipment' AND jl.ref_id = h.id),
  'bill_of_entry', (SELECT jsonb_build_object('id', b.id, 'be_no', b.be_no, 'status', b.status, 'total_payable', b.total_payable) FROM bills_of_entry b WHERE b.shipment_id = h.id),
  'landed_cost', (SELECT jsonb_build_object('id', c.id, 'costing_no', c.costing_no, 'status', c.status, 'total_landed', c.total_landed) FROM landed_costs c WHERE c.shipment_id = h.id),
  'demurrage_estimate', h.demurrage_per_day * (SELECT coalesce(sum(greatest(0, coalesce(c.delivered_at, (now() AT TIME ZONE 'Asia/Dhaka')::date) - c.free_until)), 0) FROM containers c WHERE c.shipment_id = h.id AND c.free_until IS NOT NULL)))::text
  FROM shipments h JOIN purchase_orders o ON o.id = h.po_id JOIN suppliers s ON s.id = h.supplier_id WHERE h.id = $1 AND h.company_id = $2";

pub async fn shipment_doc(conn: &mut PgConnection, company: Uuid, id: Uuid) -> AppResult<String> {
    sqlx::query_scalar(SHIPMENT_DOC).bind(id).bind(company).fetch_optional(&mut *conn).await?.ok_or_else(|| AppError::not_found("shipment"))
}

async fn add_event(conn: &mut PgConnection, shipment: Uuid, container: Option<Uuid>, kind: &str, time: DateTime<Utc>, location: Option<&str>, description: &str, source: &str, user: Uuid) -> AppResult<()> {
    sqlx::query("INSERT INTO tracking_events (shipment_id, container_id, event_time, event_type, location, description, source, created_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)")
        .bind(shipment).bind(container).bind(time).bind(kind).bind(location).bind(description).bind(source).bind(user).execute(&mut *conn).await?;
    Ok(())
}

fn check_container_no(no: &str) -> bool {
    // ISO 6346: four letters and seven digits, e.g. MSKU7731450. LCL/air cargo may use any reference.
    let n: Vec<char> = no.chars().filter(|c| !c.is_whitespace() && *c != '-').collect();
    !n.is_empty() && n.len() <= 20
}

async fn create_shipment(State(st): State<AppState>, user: AuthUser, Json(b): Json<ShipIn>) -> AppResult<Created> {
    if b.lines.is_empty() { return Err(AppError::field("lines", "Choose the order lines in this shipment")); }
    if b.freight < Decimal::ZERO || b.insurance < Decimal::ZERO || b.demurrage_per_day < Decimal::ZERO { return Err(AppError::field("freight", "Amounts must be zero or more")); }
    let mode = b.mode.clone().unwrap_or_else(|| "sea".into());
    if !["sea", "air", "land"].contains(&mode.as_str()) { return Err(AppError::field("mode", "Must be sea, air or land")); }
    if let (Some(d), Some(a)) = (b.etd, b.eta) { if a < d { return Err(AppError::field("eta", "Arrival cannot be before departure")); } }
    let mut tx = st.db.begin().await?;
    let (status, po_type, supplier, currency, fx, subtotal, discount, po_no, branch, pol, pod): (String, String, Uuid, String, Decimal, Decimal, Decimal, String, Option<Uuid>, Option<String>, Option<String>) = sqlx::query_as("SELECT status, po_type, supplier_id, currency, fx_rate, subtotal, discount, po_no, branch_id, port_of_loading, port_of_discharge FROM purchase_orders WHERE id = $1 AND company_id = $2 FOR UPDATE").bind(b.po_id).bind(user.company_id).fetch_optional(&mut *tx).await?.ok_or_else(|| AppError::field("po_id", "Purchase order does not exist"))?;
    if po_type != "import" { return Err(AppError::field("po_id", "Shipments are for import orders")); }
    if status != "approved" && status != "partially_received" { return Err(AppError::rule(format!("Order {po_no} is {status}; approve it before booking a shipment"))); }
    if let Some(lc) = b.lc_id {
        let ok: Option<String> = sqlx::query_scalar("SELECT status FROM letters_of_credit WHERE id = $1 AND company_id = $2 AND po_id = $3").bind(lc).bind(user.company_id).bind(b.po_id).fetch_optional(&mut *tx).await?;
        match ok.as_deref() { Some("opened") => {}, Some(s) => return Err(AppError::field("lc_id", format!("This LC is {s}; it must be opened"))), None => return Err(AppError::field("lc_id", "This LC does not belong to the order")) }
    }
    let factor = if subtotal.is_zero() { Decimal::ONE } else { (subtotal - discount) / subtotal };
    let free_days = match b.free_days { Some(d) if d >= 0 => d, Some(_) => return Err(AppError::field("free_days", "Must be zero or more")), None => db::setting_dec(&mut tx, user.company_id, "import.default_free_days", 4.into()).await?.to_string().parse().unwrap_or(4) };
    let no = db::next_no(&mut tx, user.company_id, "shipment", "SHP").await?;
    let id: Uuid = sqlx::query_scalar("INSERT INTO shipments (company_id, branch_id, shipment_no, po_id, lc_id, supplier_id, mode, bl_no, bl_date, vessel_name, voyage_no, carrier, port_of_loading, port_of_discharge, etd, eta, commercial_invoice_no, commercial_invoice_date, currency, fx_rate, freight, insurance, free_days, demurrage_per_day, notes, created_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,coalesce($14,'Chattogram'),$15,$16,$17,$18,$19,$20,$21,$22,$23,$24,$25,$26) RETURNING id")
        .bind(user.company_id).bind(branch).bind(&no).bind(b.po_id).bind(b.lc_id).bind(supplier).bind(&mode).bind(&b.bl_no).bind(b.bl_date).bind(&b.vessel_name).bind(&b.voyage_no).bind(&b.carrier)
        .bind(b.port_of_loading.clone().or(pol)).bind(b.port_of_discharge.clone().or(pod)).bind(b.etd).bind(b.eta).bind(&b.commercial_invoice_no).bind(b.commercial_invoice_date).bind(&currency).bind(fx)
        .bind(r2(b.freight)).bind(r2(b.insurance)).bind(free_days).bind(r2(b.demurrage_per_day)).bind(&b.notes).bind(user.user_id).fetch_one(&mut *tx).await?;
    let mut goods = Decimal::ZERO;
    for (i, l) in b.lines.iter().enumerate() {
        if l.qty <= Decimal::ZERO { return Err(AppError::field(&format!("lines[{i}].qty"), "Must be more than zero")); }
        let (product, qty, shipped, price, hs): (Uuid, Decimal, Decimal, Decimal, Option<Uuid>) = sqlx::query_as("SELECT l.product_id, l.qty, l.shipped_qty, l.unit_price, p.hs_code_id FROM purchase_order_lines l JOIN products p ON p.id = l.product_id WHERE l.id = $1 AND l.po_id = $2 FOR UPDATE OF l").bind(l.po_line_id).bind(b.po_id).fetch_optional(&mut *tx).await?.ok_or_else(|| AppError::field(&format!("lines[{i}].po_line_id"), "This line is not on the order"))?;
        if l.qty > qty - shipped { return Err(AppError::field(&format!("lines[{i}].qty"), format!("Only {} left to ship on this line", (qty - shipped).normalize()))); }
        let unit = price * factor;
        let value = r2(l.qty * unit);
        goods += value;
        sqlx::query("INSERT INTO shipment_lines (shipment_id, po_line_id, product_id, hs_code_id, qty, unit_price, line_value) VALUES ($1,$2,$3,$4,$5,$6,$7)").bind(id).bind(l.po_line_id).bind(product).bind(hs).bind(l.qty).bind(db::r6(unit)).bind(value).execute(&mut *tx).await?;
        sqlx::query("UPDATE purchase_order_lines SET shipped_qty = shipped_qty + $2 WHERE id = $1").bind(l.po_line_id).bind(l.qty).execute(&mut *tx).await?;
    }
    let invoice = goods + r2(b.freight) + r2(b.insurance);
    sqlx::query("UPDATE shipments SET goods_value = $2, invoice_value = $3, invoice_bdt = $4 WHERE id = $1").bind(id).bind(goods).bind(invoice).bind(r2(invoice * fx)).execute(&mut *tx).await?;
    for (i, c) in b.containers.iter().enumerate() {
        if !check_container_no(&c.container_no) { return Err(AppError::field(&format!("containers[{i}].container_no"), "Enter the container number")); }
        sqlx::query("INSERT INTO containers (shipment_id, container_no, size, seal_no, gross_weight_kg, packages) VALUES ($1,$2,coalesce($3,'40HC'),$4,$5,$6)")
            .bind(id).bind(c.container_no.trim().to_uppercase()).bind(&c.size).bind(&c.seal_no).bind(c.gross_weight_kg).bind(c.packages).execute(&mut *tx).await?;
    }
    // Standard Bangladesh import document checklist.
    let mut docs = vec![("PI", true), ("CI", true), ("PL", true), ("BL", true), ("COO", true), ("INS", true), ("BE", true), ("IP", false), ("BSTI", false)];
    if b.lc_id.is_some() { docs.push(("LC", true)); docs.push(("LCA", true)); }
    for (d, required) in docs {
        sqlx::query("INSERT INTO import_documents (shipment_id, doc_type, is_required, updated_by) VALUES ($1,$2,$3,$4)").bind(id).bind(d).bind(required).bind(user.user_id).execute(&mut *tx).await?;
    }
    add_event(&mut tx, id, None, "booked", Utc::now(), b.port_of_loading.as_deref(), "Shipment booked", "system", user.user_id).await?;
    db::audit(&mut tx, &user, "create", "shipments", id, json!({"shipment_no": no, "invoice_value": invoice, "currency": currency})).await?;
    let doc = shipment_doc(&mut tx, user.company_id, id).await?;
    tx.commit().await?;
    Ok(created(doc))
}

async fn shipment_head(conn: &mut PgConnection, company: Uuid, id: Uuid) -> AppResult<(String, String)> {
    sqlx::query_as("SELECT status, shipment_no FROM shipments WHERE id = $1 AND company_id = $2 FOR UPDATE").bind(id).bind(company).fetch_optional(&mut *conn).await?.ok_or_else(|| AppError::not_found("shipment"))
}

#[derive(Deserialize)]
struct ShipUpd { bl_no: Option<String>, bl_date: Option<NaiveDate>, vessel_name: Option<String>, voyage_no: Option<String>, carrier: Option<String>, etd: Option<NaiveDate>, eta: Option<NaiveDate>, commercial_invoice_no: Option<String>, commercial_invoice_date: Option<NaiveDate>, free_days: Option<i32>, demurrage_per_day: Option<Decimal>, notes: Option<String> }

async fn update_shipment(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>, Json(b): Json<ShipUpd>) -> AppResult<RawJson> {
    let mut tx = st.db.begin().await?;
    let (status, no) = shipment_head(&mut tx, user.company_id, id).await?;
    if status == "received" || status == "cancelled" { return Err(AppError::rule(format!("{no} is {status} and can no longer be edited"))); }
    let old_eta: Option<NaiveDate> = sqlx::query_scalar("SELECT eta FROM shipments WHERE id = $1").bind(id).fetch_one(&mut *tx).await?;
    sqlx::query("UPDATE shipments SET bl_no = coalesce($2, bl_no), bl_date = coalesce($3, bl_date), vessel_name = coalesce($4, vessel_name), voyage_no = coalesce($5, voyage_no), carrier = coalesce($6, carrier), etd = coalesce($7, etd), eta = coalesce($8, eta), commercial_invoice_no = coalesce($9, commercial_invoice_no), commercial_invoice_date = coalesce($10, commercial_invoice_date), free_days = coalesce($11, free_days), demurrage_per_day = coalesce($12, demurrage_per_day), notes = coalesce($13, notes) WHERE id = $1")
        .bind(id).bind(&b.bl_no).bind(b.bl_date).bind(&b.vessel_name).bind(&b.voyage_no).bind(&b.carrier).bind(b.etd).bind(b.eta).bind(&b.commercial_invoice_no).bind(b.commercial_invoice_date).bind(b.free_days).bind(b.demurrage_per_day).bind(&b.notes).execute(&mut *tx).await?;
    if let (Some(new), Some(old)) = (b.eta, old_eta) {
        if new != old {
            add_event(&mut tx, id, None, if new > old { "delay" } else { "note" }, Utc::now(), None, &format!("ETA changed from {old} to {new}"), "system", user.user_id).await?;
        }
    }
    db::audit(&mut tx, &user, "update", "shipments", id, json!({"eta": b.eta, "bl_no": b.bl_no})).await?;
    let doc = shipment_doc(&mut tx, user.company_id, id).await?;
    tx.commit().await?;
    Ok(data(doc))
}

#[derive(Deserialize)]
struct DepartIn { departed_at: Option<NaiveDate>, bl_no: Option<String>, bl_date: Option<NaiveDate>, fx_rate: Option<Decimal> }

/// Goods are on board: the supplier's invoice becomes payable and the goods become ours (in transit).
async fn depart(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>, Json(b): Json<DepartIn>) -> AppResult<RawJson> {
    let mut tx = st.db.begin().await?;
    let (status, no) = shipment_head(&mut tx, user.company_id, id).await?;
    if status != "booked" { return Err(AppError::rule(format!("{no} is already {status}"))); }
    let (po, lc, supplier, currency, invoice, branch, bl_no): (Uuid, Option<Uuid>, Uuid, String, Decimal, Option<Uuid>, Option<String>) = sqlx::query_as("SELECT po_id, lc_id, supplier_id, currency, invoice_value, branch_id, bl_no FROM shipments WHERE id = $1").bind(id).fetch_one(&mut *tx).await?;
    if b.bl_no.as_deref().or(bl_no.as_deref()).map(str::trim).unwrap_or("").is_empty() { return Err(AppError::field("bl_no", "Enter the bill of lading (or airway bill) number")); }
    let date = match b.departed_at.or(b.bl_date) { Some(d) => d, None => db::today(&mut tx, user.company_id).await? };
    let rate = match b.fx_rate { Some(r) if r > Decimal::ZERO => r, Some(_) => return Err(AppError::field("fx_rate", "Must be more than zero")), None => finance::rate_on(&mut tx, user.company_id, &currency, date).await? };
    if let Some(lc) = lc {
        let (amount, tol, utilized, lc_no): (Decimal, Decimal, Decimal, String) = sqlx::query_as("SELECT amount, tolerance_pct, utilized_amount, lc_no FROM letters_of_credit WHERE id = $1 FOR UPDATE").bind(lc).fetch_one(&mut *tx).await?;
        let limit = r2(amount * (Decimal::ONE_HUNDRED + tol) / Decimal::ONE_HUNDRED);
        if utilized + invoice > limit { return Err(AppError::rule(format!("Invoice {invoice} {currency} exceeds what is left on {lc_no} ({} {currency}). Amend the LC first.", limit - utilized))); }
        sqlx::query("UPDATE letters_of_credit SET utilized_amount = utilized_amount + $2 WHERE id = $1").bind(lc).bind(invoice).execute(&mut *tx).await?;
    }
    // The prepaid part keeps the rate at which the advance was paid; the rest is booked at today's rate.
    let (adv_fc, adv_bdt) = tagged_balance(&mut tx, user.company_id, "supplier_advance", ("purchase_order", po), false).await?;
    let use_fc = adv_fc.min(invoice).max(Decimal::ZERO);
    let use_bdt = if use_fc.is_zero() { Decimal::ZERO } else if use_fc == adv_fc { adv_bdt } else { r2(adv_bdt * use_fc / adv_fc) };
    let rest_fc = invoice - use_fc;
    let rest_bdt = r2(rest_fc * rate);
    let total_bdt = use_bdt + rest_bdt;
    let mut lines = vec![JL::dr(Acc::Key("git"), total_bdt).tag("shipment", id).memo("Goods value per commercial invoice")];
    if use_fc > Decimal::ZERO { lines.push(JL::cr(Acc::Key("supplier_advance"), use_bdt).party("supplier", supplier).fc(&currency, use_fc).tag("purchase_order", po).memo("Advance applied")); }
    if rest_fc > Decimal::ZERO { lines.push(JL::cr(Acc::Key("ap_foreign"), rest_bdt).party("supplier", supplier).fc(&currency, rest_fc).tag("shipment", id)); }
    let je = accounting::post(&mut tx, Post { company: user.company_id, branch, date, source_type: "shipment", source_id: Some(id), memo: &format!("Goods shipped {no}"), user: user.user_id }, lines).await?;
    sqlx::query("UPDATE shipments SET status = 'in_transit', departed_at = $2, bl_no = coalesce($3, bl_no), bl_date = coalesce($4, bl_date, $2), fx_rate = $5, invoice_bdt = $6, booking_journal_id = $7 WHERE id = $1")
        .bind(id).bind(date).bind(&b.bl_no).bind(b.bl_date).bind(rate).bind(total_bdt).bind(je).execute(&mut *tx).await?;
    sqlx::query("UPDATE containers SET status = 'at_sea' WHERE shipment_id = $1").bind(id).execute(&mut *tx).await?;
    sqlx::query("UPDATE import_documents SET status = 'received', doc_no = coalesce($2, doc_no), updated_at = now() WHERE shipment_id = $1 AND doc_type = 'BL' AND status IN ('pending','requested')").bind(id).bind(&b.bl_no).execute(&mut *tx).await?;
    add_event(&mut tx, id, None, "departed", date.and_hms_opt(6, 0, 0).unwrap().and_utc(), None, "Vessel departed; goods in transit", "system", user.user_id).await?;
    db::audit(&mut tx, &user, "depart", "shipments", id, json!({"rate": rate, "invoice_bdt": total_bdt, "advance_applied_fc": use_fc})).await?;
    let doc = shipment_doc(&mut tx, user.company_id, id).await?;
    tx.commit().await?;
    Ok(data(doc))
}

#[derive(Deserialize)]
struct ArriveIn { arrived_at: Option<NaiveDate> }
async fn arrive(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>, body: Option<Json<ArriveIn>>) -> AppResult<RawJson> {
    let mut tx = st.db.begin().await?;
    let (status, no) = shipment_head(&mut tx, user.company_id, id).await?;
    if status != "in_transit" { return Err(AppError::rule(format!("{no} is {status}; only a shipment in transit can arrive"))); }
    let date = match body.and_then(|b| b.0.arrived_at) { Some(d) => d, None => db::today(&mut tx, user.company_id).await? };
    let (free, pod): (i32, String) = sqlx::query_as("SELECT free_days, port_of_discharge FROM shipments WHERE id = $1").bind(id).fetch_one(&mut *tx).await?;
    sqlx::query("UPDATE shipments SET status = 'arrived', arrived_at = $2 WHERE id = $1").bind(id).bind(date).execute(&mut *tx).await?;
    sqlx::query("UPDATE containers SET status = 'discharged', discharged_at = $2, free_until = $3 WHERE shipment_id = $1").bind(id).bind(date).bind(date + Duration::days(free as i64)).execute(&mut *tx).await?;
    add_event(&mut tx, id, None, "arrived", date.and_hms_opt(6, 0, 0).unwrap().and_utc(), Some(&pod), &format!("Arrived at {pod}; free time ends {}", date + Duration::days(free as i64)), "system", user.user_id).await?;
    db::audit(&mut tx, &user, "arrive", "shipments", id, json!({"arrived_at": date})).await?;
    let doc = shipment_doc(&mut tx, user.company_id, id).await?;
    tx.commit().await?;
    Ok(data(doc))
}

async fn cancel_shipment(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> AppResult<RawJson> {
    let mut tx = st.db.begin().await?;
    let (status, no) = shipment_head(&mut tx, user.company_id, id).await?;
    if status != "booked" { return Err(AppError::rule(format!("{no} is {status}; only a booked shipment can be cancelled"))); }
    sqlx::query("UPDATE purchase_order_lines pl SET shipped_qty = shipped_qty - sl.qty FROM shipment_lines sl WHERE sl.shipment_id = $1 AND sl.po_line_id = pl.id").bind(id).execute(&mut *tx).await?;
    sqlx::query("UPDATE shipments SET status = 'cancelled' WHERE id = $1").bind(id).execute(&mut *tx).await?;
    db::audit(&mut tx, &user, "cancel", "shipments", id, json!({})).await?;
    let doc = shipment_doc(&mut tx, user.company_id, id).await?;
    tx.commit().await?;
    Ok(data(doc))
}

async fn list_shipments(State(st): State<AppState>, user: AuthUser, Query(q): Query<HashMap<String, String>>) -> AppResult<RawJson> {
    let mut qb = db::list_query();
    qb.push("h.id, h.shipment_no, h.mode, h.bl_no, h.vessel_name, h.voyage_no, h.carrier, h.port_of_loading, h.port_of_discharge, h.etd, h.eta, h.departed_at, h.arrived_at, h.currency, h.invoice_value, h.invoice_bdt, h.status, h.po_id, o.po_no, s.name AS supplier_name, (SELECT lc_no FROM letters_of_credit c WHERE c.id = h.lc_id) AS lc_no,
      (SELECT count(*) FROM containers c WHERE c.shipment_id = h.id) AS container_count,
      (SELECT string_agg(c.container_no, ', ') FROM containers c WHERE c.shipment_id = h.id) AS container_nos,
      (SELECT count(*) FROM import_documents d WHERE d.shipment_id = h.id AND d.is_required AND d.status IN ('pending','requested')) AS documents_missing,
      CASE WHEN h.eta IS NOT NULL AND h.status IN ('booked','in_transit') THEN h.eta - (now() AT TIME ZONE 'Asia/Dhaka')::date END AS days_to_eta,
      (SELECT min(c.free_until) FROM containers c WHERE c.shipment_id = h.id AND c.delivered_at IS NULL) AS free_until,
      CASE WHEN h.etd IS NOT NULL AND h.eta IS NOT NULL AND h.eta > h.etd AND h.status = 'in_transit' THEN least(100, greatest(0, round(((now() AT TIME ZONE 'Asia/Dhaka')::date - coalesce(h.departed_at, h.etd))::numeric / (h.eta - coalesce(h.departed_at, h.etd)) * 100))) WHEN h.status IN ('arrived','customs','cleared','received') THEN 100 ELSE 0 END AS progress_pct
      FROM shipments h JOIN purchase_orders o ON o.id = h.po_id JOIN suppliers s ON s.id = h.supplier_id WHERE h.company_id = ");
    qb.push_bind(user.company_id);
    db::filter_eq(&mut qb, &q, "status", "h.status", "");
    db::filter_eq(&mut qb, &q, "po_id", "h.po_id", "::uuid");
    db::filter_eq(&mut qb, &q, "supplier_id", "h.supplier_id", "::uuid");
    if q.get("active").map(String::as_str) == Some("true") { qb.push(" AND h.status NOT IN ('received','cancelled')"); }
    db::filter_search(&mut qb, &q, &["h.shipment_no", "h.bl_no", "h.vessel_name", "s.name", "o.po_no", "(SELECT string_agg(c.container_no, ' ') FROM containers c WHERE c.shipment_id = h.id)"]);
    if let Some(br) = user.branch_id { qb.push(" AND h.branch_id = "); qb.push_bind(br); }
    db::page_json(&st.db, qb, "created_at DESC", &q).await
}
async fn get_shipment(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> AppResult<RawJson> {
    let mut conn = st.db.acquire().await?;
    Ok(data(shipment_doc(&mut conn, user.company_id, id).await?))
}

// ---- containers, tracking, documents ----
async fn add_container(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>, Json(c): Json<ContainerIn>) -> AppResult<RawJson> {
    if !check_container_no(&c.container_no) { return Err(AppError::field("container_no", "Enter the container number")); }
    let mut tx = st.db.begin().await?;
    let (status, no) = shipment_head(&mut tx, user.company_id, id).await?;
    if status == "received" || status == "cancelled" { return Err(AppError::rule(format!("{no} is {status}"))); }
    sqlx::query("INSERT INTO containers (shipment_id, container_no, size, seal_no, gross_weight_kg, packages, status, discharged_at, free_until) SELECT $1,$2,coalesce($3,'40HC'),$4,$5,$6, CASE WHEN h.arrived_at IS NOT NULL THEN 'discharged' WHEN h.departed_at IS NOT NULL THEN 'at_sea' ELSE 'loaded' END, h.arrived_at, h.arrived_at + h.free_days FROM shipments h WHERE h.id = $1")
        .bind(id).bind(c.container_no.trim().to_uppercase()).bind(&c.size).bind(&c.seal_no).bind(c.gross_weight_kg).bind(c.packages).execute(&mut *tx).await?;
    db::audit(&mut tx, &user, "add_container", "shipments", id, json!({"container_no": c.container_no})).await?;
    let doc = shipment_doc(&mut tx, user.company_id, id).await?;
    tx.commit().await?;
    Ok(data(doc))
}

#[derive(Deserialize)]
struct ContainerUpd { status: Option<String>, seal_no: Option<String>, free_until: Option<NaiveDate>, delivered_at: Option<NaiveDate>, returned_at: Option<NaiveDate> }
async fn update_container(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>, Json(b): Json<ContainerUpd>) -> AppResult<RawJson> {
    if let Some(s) = &b.status { if !["loaded", "at_sea", "discharged", "at_depot", "delivered", "returned"].contains(&s.as_str()) { return Err(AppError::field("status", "Unknown container status")); } }
    let mut tx = st.db.begin().await?;
    let (shipment, cno): (Uuid, String) = sqlx::query_as("SELECT c.shipment_id, c.container_no FROM containers c JOIN shipments h ON h.id = c.shipment_id WHERE c.id = $1 AND h.company_id = $2").bind(id).bind(user.company_id).fetch_optional(&mut *tx).await?.ok_or_else(|| AppError::not_found("container"))?;
    let status = b.status.clone().or_else(|| if b.returned_at.is_some() { Some("returned".into()) } else if b.delivered_at.is_some() { Some("delivered".into()) } else { None });
    sqlx::query("UPDATE containers SET status = coalesce($2, status), seal_no = coalesce($3, seal_no), free_until = coalesce($4, free_until), delivered_at = coalesce($5, delivered_at), returned_at = coalesce($6, returned_at) WHERE id = $1")
        .bind(id).bind(&status).bind(&b.seal_no).bind(b.free_until).bind(b.delivered_at).bind(b.returned_at).execute(&mut *tx).await?;
    if let Some(d) = b.delivered_at { add_event(&mut tx, shipment, Some(id), "gate_out", d.and_hms_opt(6, 0, 0).unwrap().and_utc(), None, &format!("{cno} left the port"), "system", user.user_id).await?; }
    if let Some(d) = b.returned_at { add_event(&mut tx, shipment, Some(id), "empty_returned", d.and_hms_opt(6, 0, 0).unwrap().and_utc(), None, &format!("{cno} empty returned"), "system", user.user_id).await?; }
    db::audit(&mut tx, &user, "update_container", "shipments", shipment, json!({"container": cno, "status": status})).await?;
    let doc = shipment_doc(&mut tx, user.company_id, shipment).await?;
    tx.commit().await?;
    Ok(data(doc))
}

#[derive(Deserialize)]
struct EventIn { event_type: String, event_time: Option<DateTime<Utc>>, location: Option<String>, description: Option<String>, container_id: Option<Uuid>, source: Option<String> }
async fn create_event(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>, Json(b): Json<EventIn>) -> AppResult<RawJson> {
    const TYPES: &[&str] = &["booked", "gate_in", "loaded", "departed", "transshipment", "arrived", "discharged", "customs_hold", "customs_released", "gate_out", "delivered", "empty_returned", "delay", "note"];
    if !TYPES.contains(&b.event_type.as_str()) { return Err(AppError::field("event_type", format!("Must be one of: {}", TYPES.join(", ")))); }
    let mut tx = st.db.begin().await?;
    shipment_head(&mut tx, user.company_id, id).await?;
    if let Some(c) = b.container_id {
        let ok: Option<i32> = sqlx::query_scalar("SELECT 1 FROM containers WHERE id = $1 AND shipment_id = $2").bind(c).bind(id).fetch_optional(&mut *tx).await?;
        if ok.is_none() { return Err(AppError::field("container_id", "This container is not on the shipment")); }
    }
    add_event(&mut tx, id, b.container_id, &b.event_type, b.event_time.unwrap_or_else(Utc::now), b.location.as_deref(), b.description.as_deref().unwrap_or(""), b.source.as_deref().unwrap_or("manual"), user.user_id).await?;
    let doc = shipment_doc(&mut tx, user.company_id, id).await?;
    tx.commit().await?;
    Ok(data(doc))
}

#[derive(Deserialize)]
struct DocIn { doc_no: Option<String>, doc_date: Option<NaiveDate>, status: Option<String>, is_required: Option<bool>, file_url: Option<String>, notes: Option<String> }
async fn upsert_document(State(st): State<AppState>, user: AuthUser, Path((id, doc_type)): Path<(Uuid, String)>, Json(b): Json<DocIn>) -> AppResult<RawJson> {
    const TYPES: &[&str] = &["PI", "CI", "PL", "BL", "COO", "LC", "LCA", "INS", "IP", "BSTI", "PHYTO", "TEST", "BE", "DO", "OTHER"];
    let doc_type = doc_type.to_uppercase();
    if !TYPES.contains(&doc_type.as_str()) { return Err(AppError::BadRequest(format!("Unknown document type. Use one of: {}", TYPES.join(", ")))); }
    if let Some(s) = &b.status { if !["pending", "requested", "received", "verified", "submitted"].contains(&s.as_str()) { return Err(AppError::field("status", "Must be pending, requested, received, verified or submitted")); } }
    if let Some(u) = &b.file_url { if !(u.starts_with("https://") || u.starts_with("http://")) || u.len() > 500 { return Err(AppError::field("file_url", "Must be a web link")); } }
    let mut tx = st.db.begin().await?;
    shipment_head(&mut tx, user.company_id, id).await?;
    sqlx::query("INSERT INTO import_documents (shipment_id, doc_type, doc_no, doc_date, status, is_required, file_url, notes, updated_by) VALUES ($1,$2,$3,$4,coalesce($5,'received'),coalesce($6,true),$7,$8,$9)
                 ON CONFLICT (shipment_id, doc_type) DO UPDATE SET doc_no = coalesce(EXCLUDED.doc_no, import_documents.doc_no), doc_date = coalesce(EXCLUDED.doc_date, import_documents.doc_date), status = coalesce($5, import_documents.status), is_required = coalesce($6, import_documents.is_required), file_url = coalesce(EXCLUDED.file_url, import_documents.file_url), notes = coalesce(EXCLUDED.notes, import_documents.notes), updated_by = $9, updated_at = now()")
        .bind(id).bind(&doc_type).bind(&b.doc_no).bind(b.doc_date).bind(&b.status).bind(b.is_required).bind(&b.file_url).bind(&b.notes).bind(user.user_id).execute(&mut *tx).await?;
    db::audit(&mut tx, &user, "update_document", "shipments", id, json!({"doc_type": doc_type, "status": b.status})).await?;
    let doc = shipment_doc(&mut tx, user.company_id, id).await?;
    tx.commit().await?;
    Ok(data(doc))
}

// ---------------- foreign payments ----------------
#[derive(Deserialize)]
struct FpIn { kind: String, po_id: Option<Uuid>, shipment_id: Option<Uuid>, bank_account_id: Uuid, payment_date: Option<NaiveDate>, fc_amount: Decimal, fx_rate: Option<Decimal>, #[serde(default)] bank_charges: Decimal, margin_applied: Option<Decimal>, reference: Option<String> }

const FP_DOC: &str = "SELECT (to_jsonb(f) || jsonb_build_object('supplier_name', (SELECT name FROM suppliers s WHERE s.id = f.supplier_id), 'po_no', (SELECT po_no FROM purchase_orders o WHERE o.id = f.po_id), 'shipment_no', (SELECT shipment_no FROM shipments h WHERE h.id = f.shipment_id), 'lc_no', (SELECT lc_no FROM letters_of_credit c WHERE c.id = f.lc_id), 'bank_account', (SELECT name FROM bank_accounts b WHERE b.id = f.bank_account_id)))::text FROM foreign_payments f WHERE f.id = $1 AND f.company_id = $2";

/// Post a stored TT or LC payment.
pub async fn post_foreign_payment(conn: &mut PgConnection, user: &AuthUser, id: Uuid) -> AppResult<()> {
    let (no, kind, po, lc, shipment, supplier, bank_id, date, currency, fc, rate, bdt, charges, margin_req): (String, String, Option<Uuid>, Option<Uuid>, Option<Uuid>, Option<Uuid>, Uuid, NaiveDate, String, Decimal, Decimal, Decimal, Decimal, Decimal) =
        sqlx::query_as("SELECT payment_no, kind, po_id, lc_id, shipment_id, supplier_id, bank_account_id, payment_date, currency, fc_amount, fx_rate, bdt_amount, bank_charges, margin_applied FROM foreign_payments WHERE id = $1 AND company_id = $2 FOR UPDATE").bind(id).bind(user.company_id).fetch_one(&mut *conn).await?;
    let supplier = supplier.ok_or_else(|| AppError::Internal("payment without supplier".into()))?;
    let bank = finance::bank(conn, user.company_id, bank_id, "bank_account_id").await?;
    let bank_cr = |amount: Decimal, fc_part: Decimal| { let l = JL::cr(Acc::Id(bank.gl), amount); if bank.currency != "BDT" { l.fc(&bank.currency, fc_part) } else { l } };
    let mut lines;
    let mut fx_diff = Decimal::ZERO;
    let mut margin = Decimal::ZERO;
    match kind.as_str() {
        "tt_advance" => {
            let po = po.ok_or_else(|| AppError::Internal("advance without order".into()))?;
            lines = vec![JL::dr(Acc::Key("supplier_advance"), bdt).party("supplier", supplier).fc(&currency, fc).tag("purchase_order", po), bank_cr(bdt, fc)];
        }
        "tt_payment" | "lc_settlement" => {
            let sh = shipment.ok_or_else(|| AppError::Internal("payment without shipment".into()))?;
            let (out_fc, out_bdt) = tagged_balance(conn, user.company_id, "ap_foreign", ("shipment", sh), true).await?;
            if fc > out_fc { return Err(AppError::rule(format!("Only {out_fc} {currency} is still payable on this shipment"))); }
            let book = if fc == out_fc { out_bdt } else { r2(out_bdt * fc / out_fc) };
            fx_diff = bdt - book; // paying more BDT than booked = loss
            lines = vec![JL::dr(Acc::Key("ap_foreign"), book).party("supplier", supplier).fc(&currency, fc).tag("shipment", sh)];
            if kind == "lc_settlement" {
                let lc = lc.ok_or_else(|| AppError::Internal("settlement without LC".into()))?;
                let held: Decimal = sqlx::query_scalar("SELECT margin_bdt FROM letters_of_credit WHERE id = $1 FOR UPDATE").bind(lc).fetch_one(&mut *conn).await?;
                margin = margin_req.min(held).min(bdt);
                lines.push(JL::cr(Acc::Key("lc_margin"), margin).tag("lc", lc).memo("Margin applied"));
                sqlx::query("UPDATE letters_of_credit SET margin_bdt = margin_bdt - $2, settled_amount = settled_amount + $3 WHERE id = $1").bind(lc).bind(margin).bind(fc).execute(&mut *conn).await?;
            }
            lines.push(bank_cr(bdt - margin, fc));
            lines.push(JL::dr(Acc::Key("fx_gain_loss"), fx_diff).memo("Exchange difference on settlement"));
        }
        other => return Err(AppError::Internal(format!("cannot post payment kind {other}"))),
    }
    lines.push(JL::dr(Acc::Key("bank_charges"), charges));
    lines.push(JL::cr(Acc::Id(bank.gl), charges));
    finance::ensure_funds(conn, user.company_id, &bank, bdt - margin + charges).await?;
    let je = accounting::post(conn, Post { company: user.company_id, branch: None, date, source_type: "foreign_payment", source_id: Some(id), memo: &format!("{} {no}: {fc} {currency} at {rate}", kind.replace('_', " ")), user: user.user_id }, lines).await?;
    sqlx::query("UPDATE foreign_payments SET status = 'posted', journal_entry_id = $2, fx_gain_loss = $3, margin_applied = $4 WHERE id = $1").bind(id).bind(je).bind(fx_diff).bind(margin).execute(&mut *conn).await?;
    Ok(())
}

pub async fn on_payment_approval(conn: &mut PgConnection, user: &AuthUser, id: Uuid, approved: bool) -> AppResult<()> {
    if approved { post_foreign_payment(conn, user, id).await } else {
        sqlx::query("UPDATE foreign_payments SET status = 'rejected' WHERE id = $1 AND company_id = $2").bind(id).bind(user.company_id).execute(&mut *conn).await?;
        Ok(())
    }
}

async fn create_foreign_payment(State(st): State<AppState>, user: AuthUser, Json(b): Json<FpIn>) -> AppResult<Created> {
    if b.fc_amount <= Decimal::ZERO { return Err(AppError::field("fc_amount", "Must be more than zero")); }
    if b.bank_charges < Decimal::ZERO { return Err(AppError::field("bank_charges", "Must be zero or more")); }
    let mut tx = st.db.begin().await?;
    let bank = finance::bank(&mut tx, user.company_id, b.bank_account_id, "bank_account_id").await?;
    let date = match b.payment_date { Some(d) => d, None => db::today(&mut tx, user.company_id).await? };
    let (po, lc, shipment, supplier, currency): (Option<Uuid>, Option<Uuid>, Option<Uuid>, Uuid, String) = match b.kind.as_str() {
        "tt_advance" => {
            let po = b.po_id.ok_or_else(|| AppError::field("po_id", "Choose the purchase order this advance is for"))?;
            let (status, po_type, supplier, currency, total, no): (String, String, Uuid, String, Decimal, String) = sqlx::query_as("SELECT status, po_type, supplier_id, currency, total, po_no FROM purchase_orders WHERE id = $1 AND company_id = $2").bind(po).bind(user.company_id).fetch_optional(&mut *tx).await?.ok_or_else(|| AppError::field("po_id", "Purchase order does not exist"))?;
            if po_type != "import" { return Err(AppError::field("po_id", "Advances by TT are for import orders")); }
            if status != "approved" && status != "partially_received" { return Err(AppError::rule(format!("Order {no} is {status}; approve it before paying an advance"))); }
            let paid: Decimal = sqlx::query_scalar("SELECT coalesce(sum(fc_amount), 0) FROM foreign_payments WHERE po_id = $1 AND kind = 'tt_advance' AND status IN ('posted','pending_approval')").bind(po).fetch_one(&mut *tx).await?;
            if paid + b.fc_amount > total { return Err(AppError::field("fc_amount", format!("Advances would exceed the order value ({} {currency} already paid of {total})", paid))); }
            (Some(po), None, None, supplier, currency)
        }
        "tt_payment" | "lc_settlement" => {
            let sh = b.shipment_id.ok_or_else(|| AppError::field("shipment_id", "Choose the shipment being paid"))?;
            let (status, po, lc, supplier, currency, no): (String, Uuid, Option<Uuid>, Uuid, String, String) = sqlx::query_as("SELECT status, po_id, lc_id, supplier_id, currency, shipment_no FROM shipments WHERE id = $1 AND company_id = $2").bind(sh).bind(user.company_id).fetch_optional(&mut *tx).await?.ok_or_else(|| AppError::field("shipment_id", "Shipment does not exist"))?;
            if status == "booked" || status == "cancelled" { return Err(AppError::rule(format!("{no} is {status}; nothing is payable until the goods are shipped"))); }
            if b.kind == "lc_settlement" && lc.is_none() { return Err(AppError::rule(format!("{no} is not under an LC. Use a TT payment instead."))); }
            let (out_fc, _) = tagged_balance(&mut tx, user.company_id, "ap_foreign", ("shipment", sh), true).await?;
            let pending: Decimal = sqlx::query_scalar("SELECT coalesce(sum(fc_amount), 0) FROM foreign_payments WHERE shipment_id = $1 AND status = 'pending_approval'").bind(sh).fetch_one(&mut *tx).await?;
            if b.fc_amount > out_fc - pending { return Err(AppError::field("fc_amount", format!("Only {} {currency} is still payable on this shipment", out_fc - pending))); }
            (Some(po), if b.kind == "lc_settlement" { lc } else { None }, Some(sh), supplier, currency)
        }
        _ => return Err(AppError::field("kind", "Must be tt_advance, tt_payment or lc_settlement")),
    };
    if bank.currency != "BDT" && bank.currency != currency { return Err(AppError::field("bank_account_id", format!("Pay from a BDT account or a {currency} account"))); }
    let rate = match b.fx_rate { Some(r) if r > Decimal::ZERO => r, Some(_) => return Err(AppError::field("fx_rate", "Must be more than zero")), None => finance::rate_on(&mut tx, user.company_id, &currency, date).await? };
    let fc = r2(b.fc_amount);
    let bdt = r2(fc * rate);
    let margin = r2(b.margin_applied.unwrap_or(bdt)).max(Decimal::ZERO); // by default use all the margin the bank holds
    let no = db::next_no(&mut tx, user.company_id, "foreign_payment", "FP").await?;
    let id: Uuid = sqlx::query_scalar("INSERT INTO foreign_payments (company_id, payment_no, kind, po_id, lc_id, shipment_id, supplier_id, bank_account_id, payment_date, currency, fc_amount, fx_rate, bdt_amount, margin_applied, bank_charges, reference, status, created_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,'pending_approval',$17) RETURNING id")
        .bind(user.company_id).bind(&no).bind(&b.kind).bind(po).bind(lc).bind(shipment).bind(supplier).bind(b.bank_account_id).bind(date).bind(&currency).bind(fc).bind(rate).bind(bdt).bind(if b.kind == "lc_settlement" { margin } else { Decimal::ZERO }).bind(r2(b.bank_charges)).bind(&b.reference).bind(user.user_id).fetch_one(&mut *tx).await?;
    let needs = approvals::submit(&mut tx, &user, "foreign_payment", id, &no, bdt).await?;
    if !needs { post_foreign_payment(&mut tx, &user, id).await?; }
    db::audit(&mut tx, &user, if needs { "submit" } else { "post" }, "foreign_payments", id, json!({"payment_no": no, "kind": b.kind, "fc": fc, "currency": currency, "rate": rate, "bdt": bdt})).await?;
    let doc: String = sqlx::query_scalar(FP_DOC).bind(id).bind(user.company_id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(created(doc))
}

async fn list_foreign_payments(State(st): State<AppState>, user: AuthUser, Query(q): Query<HashMap<String, String>>) -> AppResult<RawJson> {
    let mut qb = db::list_query();
    qb.push("f.*, (SELECT name FROM suppliers s WHERE s.id = f.supplier_id) AS supplier_name, (SELECT po_no FROM purchase_orders o WHERE o.id = f.po_id) AS po_no, (SELECT shipment_no FROM shipments h WHERE h.id = f.shipment_id) AS shipment_no, (SELECT lc_no FROM letters_of_credit c WHERE c.id = f.lc_id) AS lc_no FROM foreign_payments f WHERE f.company_id = ");
    qb.push_bind(user.company_id);
    db::filter_eq(&mut qb, &q, "kind", "f.kind", "");
    db::filter_eq(&mut qb, &q, "status", "f.status", "");
    db::filter_eq(&mut qb, &q, "po_id", "f.po_id", "::uuid");
    db::filter_eq(&mut qb, &q, "shipment_id", "f.shipment_id", "::uuid");
    db::filter_dates(&mut qb, &q, "f.payment_date");
    db::page_json(&st.db, qb, "created_at DESC", &q).await
}
async fn get_foreign_payment(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> AppResult<RawJson> {
    Ok(data(sqlx::query_scalar(FP_DOC).bind(id).bind(user.company_id).fetch_optional(&st.db).await?.ok_or_else(|| AppError::not_found("payment"))?))
}

pub fn api() -> Api {
    Api::new("Import: LC & payments")
        .get("/lcs", "import.read", "Letters of credit", list_lcs).query("page,per_page,q,status,po_id")
        .post("/lcs", "import.write", "Draft an LC for an approved import order", create_lc)
        .body(r#"{"po_id":"<uuid>","bank_account_id":"<uuid>","lc_type":"sight","margin_pct":20,"tolerance_pct":5,"advising_bank":"Bank of China, Shenzhen","expiry_date":"2027-02-28","latest_shipment_date":"2027-01-31"}"#)
        .get("/lcs/:id", "import.read", "One LC with amendments, payments and shipments", get_lc)
        .post("/lcs/:id/open", "import.pay", "Open the LC: bank takes margin and charges", open_lc).body(r#"{"open_date":"2026-11-15","bank_lc_no":"0000123426010045","fx_rate":121.5,"commission":18500,"swift_charges":3500}"#)
        .post("/lcs/:id/amend", "import.write", "Amend amount, dates or tolerance", amend_lc).body(r#"{"latest_shipment_date":"2027-02-15","expiry_date":"2027-03-15","reason":"Factory delay","charges":2500}"#)
        .post("/lcs/:id/close", "import.pay", "Close the LC and release remaining margin", close_lc).body(r#"{"close_date":"2027-03-01"}"#)
        .get("/foreign-payments", "import.read", "TT and LC payments", list_foreign_payments).query("page,per_page,kind,status,po_id,shipment_id,from,to")
        .get("/foreign-payments/:id", "import.read", "One foreign payment", get_foreign_payment)
        .post("/foreign-payments", "import.pay", "Pay a foreign supplier: TT advance, TT payment or LC settlement (may need approval)", create_foreign_payment)
        .body(r#"{"kind":"lc_settlement","shipment_id":"<uuid>","bank_account_id":"<uuid>","payment_date":"2027-01-20","fc_amount":44400,"fx_rate":122.1,"bank_charges":4200,"reference":"PAD 2027/118"}"#)
        .tag("Import: shipments")
        .get("/shipments", "import.read", "Shipments with ETA, progress and missing documents", list_shipments).query("page,per_page,q,status,po_id,supplier_id,active")
        .post("/shipments", "import.write", "Book a shipment against an import order", create_shipment)
        .body(r#"{"po_id":"<uuid>","lc_id":"<uuid>","mode":"sea","vessel_name":"MV Maersk Chattogram","voyage_no":"246E","carrier":"Maersk","etd":"2026-12-05","eta":"2026-12-24","commercial_invoice_no":"LM-CI-2231","freight":1800,"insurance":0,"free_days":4,"demurrage_per_day":2400,"lines":[{"po_line_id":"<uuid>","qty":2400}],"containers":[{"container_no":"MSKU7731450","size":"40HC","seal_no":"ML-889123"}]}"#)
        .get("/shipments/:id", "import.read", "Shipment with lines, containers, tracking, documents, payments and costing", get_shipment)
        .put("/shipments/:id", "import.write", "Update vessel, dates and references", update_shipment).body(r#"{"eta":"2026-12-27","vessel_name":"MV Maersk Chattogram"}"#)
        .post("/shipments/:id/depart", "import.write", "Goods on board: books Goods in Transit and the supplier payable", depart).body(r#"{"departed_at":"2026-12-05","bl_no":"MAEU226884512","fx_rate":121.8}"#)
        .post("/shipments/:id/arrive", "import.write", "Vessel arrived: starts the port free-time clock", arrive).body(r#"{"arrived_at":"2026-12-24"}"#)
        .post("/shipments/:id/cancel", "import.write", "Cancel a booked shipment", cancel_shipment)
        .post("/shipments/:id/containers", "import.write", "Add a container", add_container).body(r#"{"container_no":"TGHU2208113","size":"20GP"}"#)
        .put("/containers/:id", "import.write", "Update container status and dates", update_container).body(r#"{"delivered_at":"2026-12-29"}"#)
        .post("/shipments/:id/events", "import.write", "Add a tracking event", create_event).body(r#"{"event_type":"transshipment","event_time":"2026-12-14T08:00:00Z","location":"Colombo","description":"Transshipped to feeder vessel"}"#)
        .put("/shipments/:id/documents/:doc_type", "import.write", "Update one document on the checklist", upsert_document).body(r#"{"doc_no":"COO-88123","doc_date":"2026-12-03","status":"received"}"#)
}
