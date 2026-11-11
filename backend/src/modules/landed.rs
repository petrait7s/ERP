//! Landed cost: spread every import cost over the shipment's products, then receive
//! the goods into stock at their true landed unit cost.
//!
//! Landed cost of a line = goods value (at booked rates) + CD/RD/SD and customs fees
//!                       + its share of freight, insurance, LC, C&F, port, transport and other charges.
//! VAT, AT and AIT paid at import are recoverable and never enter product cost.
use crate::{api::Api, auth::AuthUser, db::{self, created, data, r2, r6, Created, RawJson}, error::{AppError, AppResult}, modules::{accounting::{self, Acc, Post, JL}, finance, import, inventory::{self, Move}, purchase}, state::AppState};
use axum::{extract::{Path, Query, State}, Json};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::json;
use sqlx::PgConnection;
use std::collections::HashMap;
use uuid::Uuid;

/// Split `amount` in proportion to `bases`. The last non-zero share absorbs rounding so the
/// parts always add up exactly. Falls back to `fallback` bases when `bases` sum to zero.
pub fn allocate(amount: Decimal, bases: &[Decimal], fallback: &[Decimal]) -> Vec<Decimal> {
    let use_bases = if bases.iter().sum::<Decimal>().is_zero() { fallback } else { bases };
    let total: Decimal = use_bases.iter().sum();
    let n = use_bases.len();
    if n == 0 { return vec![]; }
    if total.is_zero() {
        let each = r2(amount / Decimal::from(n as i64));
        let mut out = vec![each; n];
        out[n - 1] = amount - each * Decimal::from(n as i64 - 1);
        return out;
    }
    let last = use_bases.iter().rposition(|b| !b.is_zero()).unwrap_or(n - 1);
    let mut out = vec![Decimal::ZERO; n];
    let mut given = Decimal::ZERO;
    for i in 0..n {
        if i == last { continue; }
        out[i] = r2(amount * use_bases[i] / total);
        given += out[i];
    }
    out[last] = amount - given;
    out
}

const METHODS: &[&str] = &["value", "quantity", "weight", "volume"];
const CHARGE_TYPES: &[&str] = &["freight", "insurance", "lc_commission", "bank_charges", "cnf_commission", "port_charges", "shipping_agent", "transport", "labour", "demurrage", "testing", "other"];

struct Line { id: Uuid, product: Uuid, qty: Decimal, value: Decimal, weight: Decimal, volume: Decimal }
fn bases(lines: &[Line], method: &str) -> Vec<Decimal> {
    lines.iter().map(|l| match method { "quantity" => l.qty, "weight" => l.weight, "volume" => l.volume, _ => l.value }).collect()
}

/// Recalculate a draft costing from the shipment, its Bill of Entry and its charges.
pub async fn recompute(conn: &mut PgConnection, company: Uuid, id: Uuid) -> AppResult<()> {
    let (shipment, method, status): (Uuid, String, String) = sqlx::query_as("SELECT shipment_id, allocation_method, status FROM landed_costs WHERE id = $1 AND company_id = $2 FOR UPDATE").bind(id).bind(company).fetch_optional(&mut *conn).await?.ok_or_else(|| AppError::not_found("landed cost"))?;
    if status != "draft" { return Ok(()); }
    let (goods_fc, invoice_fc, invoice_bdt): (Decimal, Decimal, Decimal) = sqlx::query_as("SELECT goods_value, invoice_value, invoice_bdt FROM shipments WHERE id = $1").bind(shipment).fetch_one(&mut *conn).await?;
    let rows: Vec<(Uuid, Uuid, Decimal, Decimal, Decimal, Decimal)> = sqlx::query_as("SELECT l.id, l.product_id, l.qty, l.line_value, l.qty * p.weight_kg, l.qty * p.volume_cbm FROM shipment_lines l JOIN products p ON p.id = l.product_id WHERE l.shipment_id = $1 ORDER BY l.id").bind(shipment).fetch_all(&mut *conn).await?;
    let lines: Vec<Line> = rows.into_iter().map(|r| Line { id: r.0, product: r.1, qty: r.2, value: r.3, weight: r.4, volume: r.5 }).collect();
    let value_bases = bases(&lines, "value");
    // The supplier's invoice may include freight and insurance (CFR/CIF); that part is a charge to allocate.
    let goods_bdt = if invoice_fc.is_zero() { Decimal::ZERO } else { r2(invoice_bdt * goods_fc / invoice_fc) };
    let supplier_extra = invoice_bdt - goods_bdt;
    let goods = allocate(goods_bdt, &value_bases, &value_bases);
    let mut charges = allocate(supplier_extra, &bases(&lines, &method), &value_bases);
    let mut charges_total = supplier_extra;
    let extra: Vec<(Decimal, Option<String>)> = sqlx::query_as("SELECT amount, allocation_method FROM landed_cost_charges WHERE landed_cost_id = $1").bind(id).fetch_all(&mut *conn).await?;
    for (amount, m) in extra {
        let part = allocate(amount, &bases(&lines, m.as_deref().unwrap_or(&method)), &value_bases);
        for (i, p) in part.iter().enumerate() { charges[i] += *p; }
        charges_total += amount;
    }
    // Duties are exact per line from the Bill of Entry; customs fees are spread by value.
    let mut duties = vec![Decimal::ZERO; lines.len()];
    let mut duties_total = Decimal::ZERO;
    let be: Option<(Uuid, Decimal)> = sqlx::query_as("SELECT id, other_fees FROM bills_of_entry WHERE shipment_id = $1").bind(shipment).fetch_optional(&mut *conn).await?;
    if let Some((be_id, fees)) = be {
        let per: Vec<(Uuid, Decimal)> = sqlx::query_as("SELECT shipment_line_id, cd + rd + sd FROM bill_of_entry_lines WHERE be_id = $1").bind(be_id).fetch_all(&mut *conn).await?;
        for (i, l) in lines.iter().enumerate() { duties[i] = per.iter().filter(|p| p.0 == l.id).map(|p| p.1).sum(); }
        for (i, f) in allocate(fees, &value_bases, &value_bases).iter().enumerate() { duties[i] += *f; }
        duties_total = duties.iter().sum();
    }
    sqlx::query("DELETE FROM landed_cost_lines WHERE landed_cost_id = $1").bind(id).execute(&mut *conn).await?;
    for (i, l) in lines.iter().enumerate() {
        let total = goods[i] + duties[i] + charges[i];
        sqlx::query("INSERT INTO landed_cost_lines (landed_cost_id, shipment_line_id, product_id, qty, goods_bdt, duties_bdt, charges_bdt, landed_total, unit_landed_cost) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)")
            .bind(id).bind(l.id).bind(l.product).bind(l.qty).bind(goods[i]).bind(duties[i]).bind(charges[i]).bind(total).bind(r6(total / l.qty)).execute(&mut *conn).await?;
    }
    sqlx::query("UPDATE landed_costs SET goods_bdt = $2, duties_bdt = $3, charges_bdt = $4, total_landed = $5 WHERE id = $1").bind(id).bind(goods_bdt).bind(duties_total).bind(charges_total).bind(goods_bdt + duties_total + charges_total).execute(&mut *conn).await?;
    Ok(())
}

const LC_DOC: &str = "SELECT (to_jsonb(c) || jsonb_build_object('shipment_no', h.shipment_no, 'shipment_status', h.status, 'po_no', (SELECT po_no FROM purchase_orders o WHERE o.id = h.po_id), 'supplier_name', (SELECT name FROM suppliers s WHERE s.id = h.supplier_id), 'currency', h.currency, 'invoice_value', h.invoice_value, 'fx_rate', h.fx_rate,
  'bill_of_entry', (SELECT jsonb_build_object('be_no', b.be_no, 'status', b.status, 'cd', b.cd, 'rd', b.rd, 'sd', b.sd, 'vat', b.vat, 'ait', b.ait, 'at', b.at, 'other_fees', b.other_fees) FROM bills_of_entry b WHERE b.shipment_id = h.id),
  'charges', (SELECT coalesce(jsonb_agg(to_jsonb(g) || jsonb_build_object('supplier_name', (SELECT name FROM suppliers s WHERE s.id = g.supplier_id), 'bank_account', (SELECT name FROM bank_accounts b WHERE b.id = g.bank_account_id)) ORDER BY g.charge_type), '[]') FROM landed_cost_charges g WHERE g.landed_cost_id = c.id),
  'lines', (SELECT coalesce(jsonb_agg(to_jsonb(l) || jsonb_build_object('sku', p.sku, 'product_name', p.name, 'sale_price', p.sale_price,
      'margin_pct', CASE WHEN p.sale_price > 0 THEN round((p.sale_price - l.unit_landed_cost) / p.sale_price * 100, 2) END,
      'cost_uplift_pct', CASE WHEN l.goods_bdt > 0 THEN round((l.landed_total - l.goods_bdt) / l.goods_bdt * 100, 2) END) ORDER BY p.sku), '[]') FROM landed_cost_lines l JOIN products p ON p.id = l.product_id WHERE l.landed_cost_id = c.id)))::text
  FROM landed_costs c JOIN shipments h ON h.id = c.shipment_id WHERE c.id = $1 AND c.company_id = $2";

async fn doc(conn: &mut PgConnection, company: Uuid, id: Uuid) -> AppResult<String> {
    recompute(conn, company, id).await?;
    sqlx::query_scalar(LC_DOC).bind(id).bind(company).fetch_optional(&mut *conn).await?.ok_or_else(|| AppError::not_found("landed cost"))
}

#[derive(Deserialize)]
struct CostIn { shipment_id: Uuid, allocation_method: Option<String>, costing_date: Option<NaiveDate> }

async fn create(State(st): State<AppState>, user: AuthUser, Json(b): Json<CostIn>) -> AppResult<Created> {
    let method = b.allocation_method.clone().unwrap_or_else(|| "value".into());
    if !METHODS.contains(&method.as_str()) { return Err(AppError::field("allocation_method", "Must be value, quantity, weight or volume")); }
    let mut tx = st.db.begin().await?;
    let (status, no, lc): (String, String, Option<Uuid>) = sqlx::query_as("SELECT status, shipment_no, lc_id FROM shipments WHERE id = $1 AND company_id = $2").bind(b.shipment_id).bind(user.company_id).fetch_optional(&mut *tx).await?.ok_or_else(|| AppError::field("shipment_id", "Shipment does not exist"))?;
    if status == "cancelled" || status == "received" { return Err(AppError::rule(format!("{no} is {status}"))); }
    let date = match b.costing_date { Some(d) => d, None => db::today(&mut tx, user.company_id).await? };
    let cno = db::next_no(&mut tx, user.company_id, "landed_cost", "LCS").await?;
    let id: Uuid = sqlx::query_scalar("INSERT INTO landed_costs (company_id, shipment_id, costing_no, costing_date, allocation_method, created_by) VALUES ($1,$2,$3,$4,$5,$6) RETURNING id")
        .bind(user.company_id).bind(b.shipment_id).bind(&cno).bind(date).bind(&method).bind(user.user_id).fetch_one(&mut *tx).await
        .map_err(|e| match AppError::from(e) { AppError::Conflict(_) => AppError::Conflict(format!("{no} already has a landed cost sheet")), o => o })?;
    // LC opening and amendment charges already paid through the bank belong to this shipment's cost.
    if let Some(lc) = lc {
        sqlx::query("INSERT INTO landed_cost_charges (landed_cost_id, charge_type, description, amount, source, source_id) SELECT $1, 'lc_commission', f.reference, f.bdt_amount, 'foreign_payment', f.id FROM foreign_payments f WHERE f.lc_id = $2 AND f.kind = 'lc_charges' AND f.status = 'posted' AND NOT EXISTS (SELECT 1 FROM landed_cost_charges g WHERE g.source_id = f.id)")
            .bind(id).bind(lc).execute(&mut *tx).await?;
    }
    db::audit(&mut tx, &user, "create", "landed_costs", id, json!({"costing_no": cno})).await?;
    let d = doc(&mut tx, user.company_id, id).await?;
    tx.commit().await?;
    Ok(created(d))
}

async fn draft(conn: &mut PgConnection, company: Uuid, id: Uuid) -> AppResult<(Uuid, String)> {
    let (status, shipment, no): (String, Uuid, String) = sqlx::query_as("SELECT status, shipment_id, costing_no FROM landed_costs WHERE id = $1 AND company_id = $2 FOR UPDATE").bind(id).bind(company).fetch_optional(&mut *conn).await?.ok_or_else(|| AppError::not_found("landed cost"))?;
    if status != "draft" { return Err(AppError::rule(format!("{no} is finalised and can no longer be changed"))); }
    Ok((shipment, no))
}

#[derive(Deserialize)]
struct ChargeIn { charge_type: String, description: Option<String>, amount: Decimal, #[serde(default)] vat_amount: Decimal, allocation_method: Option<String>, supplier_id: Option<Uuid>, bank_account_id: Option<Uuid> }

async fn add_charge(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>, Json(b): Json<ChargeIn>) -> AppResult<RawJson> {
    if !CHARGE_TYPES.contains(&b.charge_type.as_str()) { return Err(AppError::field("charge_type", format!("Must be one of: {}", CHARGE_TYPES.join(", ")))); }
    if b.amount <= Decimal::ZERO { return Err(AppError::field("amount", "Must be more than zero")); }
    if b.vat_amount < Decimal::ZERO { return Err(AppError::field("vat_amount", "Must be zero or more")); }
    if let Some(m) = &b.allocation_method { if !METHODS.contains(&m.as_str()) { return Err(AppError::field("allocation_method", "Must be value, quantity, weight or volume")); } }
    if b.supplier_id.is_some() && b.bank_account_id.is_some() { return Err(AppError::field("bank_account_id", "Choose either a supplier to owe or an account it was paid from, not both")); }
    let mut tx = st.db.begin().await?;
    draft(&mut tx, user.company_id, id).await?;
    if let Some(s) = b.supplier_id {
        let kind: String = sqlx::query_scalar("SELECT supplier_type FROM suppliers WHERE id = $1 AND company_id = $2").bind(s).bind(user.company_id).fetch_optional(&mut *tx).await?.ok_or_else(|| AppError::field("supplier_id", "Supplier does not exist"))?;
        if kind == "foreign" { return Err(AppError::field("supplier_id", "Choose a local service provider (C&F agent, transporter, port)")); }
    }
    if let Some(bk) = b.bank_account_id { if finance::bank(&mut tx, user.company_id, bk, "bank_account_id").await?.currency != "BDT" { return Err(AppError::field("bank_account_id", "Choose a BDT account")); } }
    sqlx::query("INSERT INTO landed_cost_charges (landed_cost_id, charge_type, description, amount, vat_amount, allocation_method, supplier_id, bank_account_id) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)")
        .bind(id).bind(&b.charge_type).bind(&b.description).bind(r2(b.amount)).bind(r2(b.vat_amount)).bind(&b.allocation_method).bind(b.supplier_id).bind(b.bank_account_id).execute(&mut *tx).await?;
    db::audit(&mut tx, &user, "add_charge", "landed_costs", id, json!({"charge_type": b.charge_type, "amount": b.amount})).await?;
    let d = doc(&mut tx, user.company_id, id).await?;
    tx.commit().await?;
    Ok(data(d))
}

async fn remove_charge(State(st): State<AppState>, user: AuthUser, Path((id, charge)): Path<(Uuid, Uuid)>) -> AppResult<RawJson> {
    let mut tx = st.db.begin().await?;
    draft(&mut tx, user.company_id, id).await?;
    let n = sqlx::query("DELETE FROM landed_cost_charges WHERE id = $1 AND landed_cost_id = $2 AND source = 'manual'").bind(charge).bind(id).execute(&mut *tx).await?.rows_affected();
    if n == 0 { return Err(AppError::rule("This charge cannot be removed (bank-posted LC charges stay on the sheet)")); }
    db::audit(&mut tx, &user, "remove_charge", "landed_costs", id, json!({"charge_id": charge})).await?;
    let d = doc(&mut tx, user.company_id, id).await?;
    tx.commit().await?;
    Ok(data(d))
}

#[derive(Deserialize)]
struct MethodIn { allocation_method: String }
async fn set_method(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>, Json(b): Json<MethodIn>) -> AppResult<RawJson> {
    if !METHODS.contains(&b.allocation_method.as_str()) { return Err(AppError::field("allocation_method", "Must be value, quantity, weight or volume")); }
    let mut tx = st.db.begin().await?;
    draft(&mut tx, user.company_id, id).await?;
    sqlx::query("UPDATE landed_costs SET allocation_method = $2 WHERE id = $1").bind(id).bind(&b.allocation_method).execute(&mut *tx).await?;
    let d = doc(&mut tx, user.company_id, id).await?;
    tx.commit().await?;
    Ok(data(d))
}

/// Lock the costing: post the outstanding charges and prove that Goods in Transit for the
/// shipment equals the landed total to the paisa.
async fn finalize(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> AppResult<RawJson> {
    let mut tx = st.db.begin().await?;
    let (shipment, no) = draft(&mut tx, user.company_id, id).await?;
    let (sh_status, sh_no, lc): (String, String, Option<Uuid>) = sqlx::query_as("SELECT status, shipment_no, lc_id FROM shipments WHERE id = $1 FOR UPDATE").bind(shipment).fetch_one(&mut *tx).await?;
    if !["in_transit", "arrived", "customs", "cleared"].contains(&sh_status.as_str()) { return Err(AppError::rule(format!("{sh_no} is {sh_status}; costing is finalised after the goods are shipped and before they are received"))); }
    let be: Option<String> = sqlx::query_scalar("SELECT status FROM bills_of_entry WHERE shipment_id = $1").bind(shipment).fetch_optional(&mut *tx).await?;
    match be.as_deref() { Some("paid") | Some("released") => {}, Some(_) => return Err(AppError::rule("Pay the Bill of Entry before finalising: unpaid duties are not yet part of the cost")), None => return Err(AppError::rule("Lodge and pay the Bill of Entry before finalising the landed cost")) }
    recompute(&mut tx, user.company_id, id).await?;
    let date: NaiveDate = sqlx::query_scalar("SELECT costing_date FROM landed_costs WHERE id = $1").bind(id).fetch_one(&mut *tx).await?;
    let charges: Vec<(Uuid, String, Option<String>, Decimal, Decimal, Option<Uuid>, Option<Uuid>, String)> = sqlx::query_as("SELECT id, charge_type, description, amount, vat_amount, supplier_id, bank_account_id, source FROM landed_cost_charges WHERE landed_cost_id = $1").bind(id).fetch_all(&mut *tx).await?;
    let git = accounting::account_id(&mut tx, user.company_id, "git").await?;
    let mut lines = vec![];
    let mut bills = vec![];
    for (cid, kind, desc, amount, vat, supplier, bank_id, source) in &charges {
        let memo = desc.clone().unwrap_or_else(|| kind.replace('_', " "));
        lines.push(JL::dr(Acc::Key("git"), *amount).tag("shipment", shipment).memo(memo.clone()));
        if source == "foreign_payment" {
            // Already paid through the bank against the LC; move it onto this shipment.
            lines.push(JL::cr(Acc::Key("git"), *amount).tag("lc", lc.unwrap_or(shipment)).memo(memo));
            continue;
        }
        lines.push(JL::dr(Acc::Key("vat_input"), *vat).memo(format!("VAT on {memo}")));
        let total = *amount + *vat;
        match (supplier, bank_id) {
            (Some(s), _) => { lines.push(JL::cr(Acc::Key("ap_local"), total).party("supplier", *s).memo(memo.clone())); bills.push((*cid, *s, *amount, *vat, memo)); }
            (None, Some(b)) => { let bank = finance::bank(&mut tx, user.company_id, *b, "bank_account_id").await?; finance::ensure_funds(&mut tx, user.company_id, &bank, total).await?; lines.push(JL::cr(Acc::Id(bank.gl), total).memo(memo)); }
            (None, None) => lines.push(JL::cr(Acc::Key("accrued_expenses"), total).memo(memo)),
        }
    }
    let je = if lines.is_empty() { None } else {
        Some(accounting::post(&mut tx, Post { company: user.company_id, branch: None, date, source_type: "landed_cost", source_id: Some(id), memo: &format!("Import charges, costing {no} for {sh_no}"), user: user.user_id }, lines).await?)
    };
    // Charges owed to service providers become payable bills so they flow into supplier payments.
    for (_, supplier, amount, vat, memo) in bills {
        let bill_no = db::next_no(&mut tx, user.company_id, "supplier_bill", "BILL").await?;
        let terms: i32 = sqlx::query_scalar("SELECT payment_terms_days FROM suppliers WHERE id = $1").bind(supplier).fetch_one(&mut *tx).await?;
        sqlx::query("INSERT INTO supplier_bills (company_id, bill_no, supplier_id, bill_date, due_date, subtotal, vat_amount, total, expense_account_id, notes, journal_entry_id, created_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)")
            .bind(user.company_id).bind(&bill_no).bind(supplier).bind(date).bind(date + chrono::Duration::days(terms as i64)).bind(amount).bind(vat).bind(amount + vat).bind(git).bind(format!("{memo} - {sh_no} ({no})")).bind(je).bind(user.user_id).execute(&mut *tx).await?;
    }
    let total: Decimal = sqlx::query_scalar("SELECT total_landed FROM landed_costs WHERE id = $1").bind(id).fetch_one(&mut *tx).await?;
    let (_, in_transit) = import::tagged_balance(&mut tx, user.company_id, "git", ("shipment", shipment), false).await?;
    if in_transit != total {
        return Err(AppError::rule(format!("Costing does not reconcile: Goods in Transit for {sh_no} is Tk {in_transit} but the landed total is Tk {total}. Check that the shipment and Bill of Entry are fully posted.")));
    }
    sqlx::query("UPDATE landed_costs SET status = 'finalized', journal_entry_id = $2, finalized_by = $3, finalized_at = now() WHERE id = $1").bind(id).bind(je).bind(user.user_id).execute(&mut *tx).await?;
    db::audit(&mut tx, &user, "finalize", "landed_costs", id, json!({"total_landed": total})).await?;
    let d: String = sqlx::query_scalar(LC_DOC).bind(id).bind(user.company_id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(data(d))
}

async fn list(State(st): State<AppState>, user: AuthUser, Query(q): Query<HashMap<String, String>>) -> AppResult<RawJson> {
    let mut qb = db::list_query();
    qb.push("c.id, c.costing_no, c.costing_date, c.allocation_method, c.goods_bdt, c.duties_bdt, c.charges_bdt, c.total_landed, c.status, c.shipment_id, h.shipment_no, (SELECT name FROM suppliers s WHERE s.id = h.supplier_id) AS supplier_name, CASE WHEN c.goods_bdt > 0 THEN round((c.total_landed - c.goods_bdt) / c.goods_bdt * 100, 2) END AS cost_uplift_pct FROM landed_costs c JOIN shipments h ON h.id = c.shipment_id WHERE c.company_id = ");
    qb.push_bind(user.company_id);
    db::filter_eq(&mut qb, &q, "status", "c.status", "");
    db::filter_eq(&mut qb, &q, "shipment_id", "c.shipment_id", "::uuid");
    db::page_json(&st.db, qb, "c.created_at DESC", &q).await
}
async fn get(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> AppResult<RawJson> {
    let mut tx = st.db.begin().await?;
    let d = doc(&mut tx, user.company_id, id).await?;
    tx.commit().await?;
    Ok(data(d))
}

// ---------------- receive an import shipment ----------------
#[derive(Deserialize)]
struct RecvLine { shipment_line_id: Uuid, qty: Decimal, batch_no: Option<String> }
#[derive(Deserialize)]
struct RecvIn { warehouse_id: Uuid, receipt_date: Option<NaiveDate>, notes: Option<String>, #[serde(default)] close_short: bool, lines: Vec<RecvLine> }

async fn receive(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>, Json(b): Json<RecvIn>) -> AppResult<Created> {
    if b.lines.is_empty() { return Err(AppError::field("lines", "Enter the quantities received")); }
    let mut tx = st.db.begin().await?;
    let (status, no, po, branch): (String, String, Uuid, Option<Uuid>) = sqlx::query_as("SELECT status, shipment_no, po_id, branch_id FROM shipments WHERE id = $1 AND company_id = $2 FOR UPDATE").bind(id).bind(user.company_id).fetch_optional(&mut *tx).await?.ok_or_else(|| AppError::not_found("shipment"))?;
    if status != "cleared" { return Err(AppError::rule(format!("{no} is {status}; goods can be received after customs release"))); }
    let costing: Option<(Uuid, String)> = sqlx::query_as("SELECT id, status FROM landed_costs WHERE shipment_id = $1").bind(id).fetch_optional(&mut *tx).await?;
    let costing = match costing { Some((c, s)) if s == "finalized" => c, _ => return Err(AppError::rule(format!("Finalise the landed cost of {no} first, so stock is valued at its true cost"))) };
    db::ensure(&mut tx, "warehouses", user.company_id, b.warehouse_id, "warehouse_id").await?;
    let date = match b.receipt_date { Some(d) => d, None => db::today(&mut tx, user.company_id).await? };
    let grn_no = db::next_no(&mut tx, user.company_id, "goods_receipt", "GRN").await?;
    let grn: Uuid = sqlx::query_scalar("INSERT INTO goods_receipts (company_id, branch_id, grn_no, po_id, shipment_id, warehouse_id, receipt_date, notes, billed, created_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,true,$9) RETURNING id")
        .bind(user.company_id).bind(branch).bind(&grn_no).bind(po).bind(id).bind(b.warehouse_id).bind(date).bind(&b.notes).bind(user.user_id).fetch_one(&mut *tx).await?;
    let mut total = Decimal::ZERO;
    for (i, l) in b.lines.iter().enumerate() {
        if l.qty <= Decimal::ZERO { return Err(AppError::field(&format!("lines[{i}].qty"), "Must be more than zero")); }
        let (po_line, product, qty, rec_qty, rec_val, landed): (Uuid, Uuid, Decimal, Decimal, Decimal, Decimal) = sqlx::query_as("SELECT l.po_line_id, l.product_id, l.qty, l.received_qty, l.received_value, c.landed_total FROM shipment_lines l JOIN landed_cost_lines c ON c.shipment_line_id = l.id AND c.landed_cost_id = $3 WHERE l.id = $1 AND l.shipment_id = $2 FOR UPDATE OF l").bind(l.shipment_line_id).bind(id).bind(costing).fetch_optional(&mut *tx).await?.ok_or_else(|| AppError::field(&format!("lines[{i}].shipment_line_id"), "This line is not on the shipment"))?;
        let remaining = qty - rec_qty;
        if l.qty > remaining { return Err(AppError::field(&format!("lines[{i}].qty"), format!("Only {} left to receive on this line", remaining.normalize()))); }
        // The final receipt takes whatever value is left so the line clears exactly.
        let value = if l.qty == remaining { landed - rec_val } else { r2(landed * l.qty / qty) };
        inventory::stock_move(&mut tx, Move { company: user.company_id, warehouse: b.warehouse_id, product, date, qty: l.qty, unit_cost: None, value: Some(value), move_type: "receipt", source_type: "goods_receipt", source_id: Some(grn), batch: l.batch_no.as_deref(), user: user.user_id }).await?;
        total += value;
        sqlx::query("INSERT INTO goods_receipt_lines (grn_id, po_line_id, product_id, qty, unit_cost, value, batch_no) VALUES ($1,$2,$3,$4,$5,$6,$7)").bind(grn).bind(po_line).bind(product).bind(l.qty).bind(r6(value / l.qty)).bind(value).bind(&l.batch_no).execute(&mut *tx).await?;
        sqlx::query("UPDATE shipment_lines SET received_qty = received_qty + $2, received_value = received_value + $3 WHERE id = $1").bind(l.shipment_line_id).bind(l.qty).bind(value).execute(&mut *tx).await?;
        sqlx::query("UPDATE purchase_order_lines SET received_qty = received_qty + $2 WHERE id = $1").bind(po_line).bind(l.qty).execute(&mut *tx).await?;
    }
    let mut jl = vec![JL::dr(Acc::Key("inventory"), total), JL::cr(Acc::Key("git"), total).tag("shipment", id)];
    let mut shortage = Decimal::ZERO;
    if b.close_short {
        // Goods that will never arrive (short-landed or damaged) leave Goods in Transit as a loss.
        shortage = sqlx::query_scalar("SELECT coalesce(sum(c.landed_total - l.received_value), 0) FROM shipment_lines l JOIN landed_cost_lines c ON c.shipment_line_id = l.id AND c.landed_cost_id = $2 WHERE l.shipment_id = $1 AND l.received_qty < l.qty").bind(id).bind(costing).fetch_one(&mut *tx).await?;
        sqlx::query("UPDATE shipment_lines l SET received_value = c.landed_total FROM landed_cost_lines c WHERE c.shipment_line_id = l.id AND c.landed_cost_id = $2 AND l.shipment_id = $1").bind(id).bind(costing).execute(&mut *tx).await?;
        jl.push(JL::dr(Acc::Key("inventory_adjustment"), shortage).memo("Short-landed goods written off"));
        jl.push(JL::cr(Acc::Key("git"), shortage).tag("shipment", id));
    }
    let je = accounting::post(&mut tx, Post { company: user.company_id, branch, date, source_type: "goods_receipt", source_id: Some(grn), memo: &format!("Import received {grn_no} from {no}"), user: user.user_id }, jl).await?;
    sqlx::query("UPDATE goods_receipts SET total_value = $2, journal_entry_id = $3 WHERE id = $1").bind(grn).bind(total).bind(je).execute(&mut *tx).await?;
    let open: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM shipment_lines WHERE shipment_id = $1 AND received_qty < qty)").bind(id).fetch_one(&mut *tx).await?;
    if !open || b.close_short {
        sqlx::query("UPDATE shipments SET status = 'received' WHERE id = $1").bind(id).execute(&mut *tx).await?;
        sqlx::query("UPDATE containers SET status = 'delivered', delivered_at = coalesce(delivered_at, $2) WHERE shipment_id = $1 AND status NOT IN ('delivered','returned')").bind(id).bind(date).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO tracking_events (shipment_id, event_time, event_type, description, source, created_by) VALUES ($1, now(), 'delivered', $2, 'system', $3)").bind(id).bind(format!("Received into warehouse, {grn_no}")).bind(user.user_id).execute(&mut *tx).await?;
    }
    purchase::refresh_po_status(&mut tx, po).await?;
    db::audit(&mut tx, &user, "post", "goods_receipts", grn, json!({"grn_no": grn_no, "shipment": no, "value": total, "shortage_written_off": shortage})).await?;
    let d: String = sqlx::query_scalar(purchase::GRN_DOC).bind(grn).bind(user.company_id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(created(d))
}

pub fn api() -> Api {
    Api::new("Import: landed cost")
        .get("/landed-costs", "import.read", "Landed cost sheets", list).query("page,per_page,status,shipment_id")
        .post("/landed-costs", "import.write", "Start a landed cost sheet for a shipment", create).body(r#"{"shipment_id":"<uuid>","allocation_method":"value"}"#)
        .get("/landed-costs/:id", "import.read", "Cost sheet: charges, duty, landed unit cost and margin per product", get)
        .put("/landed-costs/:id/method", "import.write", "Change how shared charges are spread", set_method).body(r#"{"allocation_method":"weight"}"#)
        .post("/landed-costs/:id/charges", "import.write", "Add a charge (freight, C&F, port, transport...)", add_charge)
        .body(r#"{"charge_type":"cnf_commission","description":"C&F agent bill 2231","amount":45000,"vat_amount":6750,"supplier_id":"<uuid>"}"#)
        .delete("/landed-costs/:id/charges/:charge_id", "import.write", "Remove a manually added charge", remove_charge)
        .post("/landed-costs/:id/finalize", "import.write", "Lock the costing and post the charges", finalize)
        .post("/shipments/:id/receive", "inventory.write", "Receive a cleared shipment into a warehouse at landed cost", receive)
        .body(r#"{"warehouse_id":"<uuid>","receipt_date":"2026-12-29","lines":[{"shipment_line_id":"<uuid>","qty":2400,"batch_no":"LM-2231"}]}"#)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    fn d(s: &str) -> Decimal { Decimal::from_str(s).unwrap() }

    #[test]
    fn allocation_always_adds_up() {
        let parts = allocate(d("100.00"), &[d("1"), d("1"), d("1")], &[]);
        assert_eq!(parts, vec![d("33.33"), d("33.33"), d("33.34")]);
        assert_eq!(parts.iter().sum::<Decimal>(), d("100.00"));
    }

    #[test]
    fn allocation_by_weight_ignores_weightless_lines() {
        let parts = allocate(d("900"), &[d("200"), d("0"), d("100")], &[d("1"), d("1"), d("1")]);
        assert_eq!(parts, vec![d("600.00"), d("0"), d("300.00")]);
    }

    #[test]
    fn allocation_falls_back_when_basis_is_missing() {
        // No weights recorded at all: fall back to value.
        let parts = allocate(d("1000"), &[d("0"), d("0")], &[d("750"), d("250")]);
        assert_eq!(parts, vec![d("750.00"), d("250.00")]);
    }
}
