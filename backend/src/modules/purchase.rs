//! Procurement: requisitions, purchase orders (local and import), goods receipts for local
//! purchases and supplier bills. Import receipts are handled by the landed-cost module.
use crate::{api::Api, auth::AuthUser, db::{self, created, data, pct, r2, r6, Created, RawJson}, error::{AppError, AppResult}, modules::{accounting::{self, Acc, Post, JL}, approvals, finance, inventory::{self, Move}}, state::AppState};
use axum::{extract::{Path, Query, State}, Json};
use chrono::{Duration, NaiveDate};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::json;
use sqlx::PgConnection;
use std::collections::HashMap;
use uuid::Uuid;

// ---------------- requisitions ----------------
#[derive(Deserialize)]
struct ReqLine { product_id: Uuid, qty: Decimal, notes: Option<String> }
#[derive(Deserialize)]
struct ReqIn { request_date: Option<NaiveDate>, required_date: Option<NaiveDate>, notes: Option<String>, branch_id: Option<Uuid>, lines: Vec<ReqLine> }

const REQ_DOC: &str = "SELECT (to_jsonb(r) || jsonb_build_object('created_by_name', (SELECT full_name FROM users u WHERE u.id = r.created_by),
  'lines', (SELECT coalesce(jsonb_agg(jsonb_build_object('id', l.id, 'product_id', l.product_id, 'sku', p.sku, 'product_name', p.name, 'qty', l.qty, 'notes', l.notes)), '[]') FROM purchase_requisition_lines l JOIN products p ON p.id = l.product_id WHERE l.requisition_id = r.id)))::text FROM purchase_requisitions r WHERE r.id = $1 AND r.company_id = $2";

async fn create_requisition(State(st): State<AppState>, user: AuthUser, Json(b): Json<ReqIn>) -> AppResult<Created> {
    if b.lines.is_empty() { return Err(AppError::field("lines", "Add at least one item")); }
    let mut tx = st.db.begin().await?;
    let date = match b.request_date { Some(d) => d, None => db::today(&mut tx, user.company_id).await? };
    let no = db::next_no(&mut tx, user.company_id, "requisition", "PR").await?;
    let branch = user.branch_for(b.branch_id)?;
    let id: Uuid = sqlx::query_scalar("INSERT INTO purchase_requisitions (company_id, branch_id, requisition_no, request_date, required_date, notes, created_by) VALUES ($1,$2,$3,$4,$5,$6,$7) RETURNING id")
        .bind(user.company_id).bind(branch).bind(&no).bind(date).bind(b.required_date).bind(&b.notes).bind(user.user_id).fetch_one(&mut *tx).await?;
    for (i, l) in b.lines.iter().enumerate() {
        if l.qty <= Decimal::ZERO { return Err(AppError::field(&format!("lines[{i}].qty"), "Must be more than zero")); }
        db::ensure(&mut tx, "products", user.company_id, l.product_id, &format!("lines[{i}].product_id")).await?;
        sqlx::query("INSERT INTO purchase_requisition_lines (requisition_id, product_id, qty, notes) VALUES ($1,$2,$3,$4)").bind(id).bind(l.product_id).bind(l.qty).bind(&l.notes).execute(&mut *tx).await?;
    }
    db::audit(&mut tx, &user, "create", "purchase_requisitions", id, json!({"requisition_no": no})).await?;
    let doc: String = sqlx::query_scalar(REQ_DOC).bind(id).bind(user.company_id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(created(doc))
}
async fn list_requisitions(State(st): State<AppState>, user: AuthUser, Query(q): Query<HashMap<String, String>>) -> AppResult<RawJson> {
    let mut qb = db::list_query();
    qb.push("r.*, (SELECT count(*) FROM purchase_requisition_lines l WHERE l.requisition_id = r.id) AS line_count FROM purchase_requisitions r WHERE r.company_id = ");
    qb.push_bind(user.company_id);
    db::filter_eq(&mut qb, &q, "status", "r.status", "");
    db::page_json(&st.db, qb, "r.created_at DESC", &q).await
}
async fn get_requisition(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> AppResult<RawJson> {
    Ok(data(sqlx::query_scalar(REQ_DOC).bind(id).bind(user.company_id).fetch_optional(&st.db).await?.ok_or_else(|| AppError::not_found("requisition"))?))
}
async fn cancel_requisition(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> AppResult<RawJson> {
    let n = sqlx::query("UPDATE purchase_requisitions SET status = 'cancelled' WHERE id = $1 AND company_id = $2 AND status = 'open'").bind(id).bind(user.company_id).execute(&st.db).await?.rows_affected();
    if n == 0 { return Err(AppError::rule("Only an open requisition can be cancelled")); }
    get_requisition(State(st), user, Path(id)).await
}

// ---------------- purchase orders ----------------
#[derive(Deserialize)]
struct PoLine { product_id: Uuid, qty: Decimal, unit_price: Decimal, vat_rate: Option<Decimal>, description: Option<String> }
#[derive(Deserialize)]
struct PoIn {
    po_type: Option<String>, supplier_id: Uuid, order_date: Option<NaiveDate>, expected_date: Option<NaiveDate>, currency: Option<String>, fx_rate: Option<Decimal>,
    incoterm: Option<String>, payment_mode: Option<String>, port_of_loading: Option<String>, port_of_discharge: Option<String>, pi_no: Option<String>, pi_date: Option<NaiveDate>,
    #[serde(default)] discount: Decimal, notes: Option<String>, requisition_id: Option<Uuid>, branch_id: Option<Uuid>, lines: Vec<PoLine>,
}

pub const PO_DOC: &str = "SELECT (to_jsonb(o) || jsonb_build_object('supplier_name', s.name, 'supplier_code', s.code, 'created_by_name', (SELECT full_name FROM users u WHERE u.id = o.created_by), 'approved_by_name', (SELECT full_name FROM users u WHERE u.id = o.approved_by),
  'lines', (SELECT coalesce(jsonb_agg(jsonb_build_object('id', l.id, 'line_no', l.line_no, 'product_id', l.product_id, 'sku', p.sku, 'product_name', p.name, 'description', l.description, 'unit', (SELECT code FROM units u WHERE u.id = p.unit_id), 'qty', l.qty, 'unit_price', l.unit_price, 'vat_rate', l.vat_rate, 'line_total', l.line_total, 'shipped_qty', l.shipped_qty, 'received_qty', l.received_qty, 'billed_qty', l.billed_qty) ORDER BY l.line_no), '[]') FROM purchase_order_lines l JOIN products p ON p.id = l.product_id WHERE l.po_id = o.id),
  'approvals', (SELECT coalesce(jsonb_agg(jsonb_build_object('id', a.id, 'level', a.level, 'approver_role', a.approver_role, 'status', a.status, 'comment', a.comment, 'decided_at', a.decided_at) ORDER BY a.created_at, a.level), '[]') FROM approval_requests a WHERE a.entity_type = 'purchase_order' AND a.entity_id = o.id)))::text
  FROM purchase_orders o JOIN suppliers s ON s.id = o.supplier_id WHERE o.id = $1 AND o.company_id = $2";

async fn save_po(conn: &mut PgConnection, user: &AuthUser, b: &PoIn, existing: Option<Uuid>) -> AppResult<Uuid> {
    if b.lines.is_empty() { return Err(AppError::field("lines", "Add at least one item")); }
    let (sup_type, sup_cur, active): (String, String, bool) = sqlx::query_as("SELECT supplier_type, currency, is_active FROM suppliers WHERE id = $1 AND company_id = $2").bind(b.supplier_id).bind(user.company_id).fetch_optional(&mut *conn).await?.ok_or_else(|| AppError::field("supplier_id", "Supplier does not exist"))?;
    if !active { return Err(AppError::field("supplier_id", "This supplier is inactive")); }
    let po_type = b.po_type.clone().unwrap_or_else(|| if sup_type == "foreign" { "import".into() } else { "local".into() });
    if po_type != "local" && po_type != "import" { return Err(AppError::field("po_type", "Must be local or import")); }
    let currency = b.currency.clone().unwrap_or(sup_cur);
    let date = match b.order_date { Some(d) => d, None => db::today(conn, user.company_id).await? };
    let fx = match b.fx_rate { Some(r) if r > Decimal::ZERO => if currency == "BDT" { Decimal::ONE } else { r }, Some(_) => return Err(AppError::field("fx_rate", "Must be more than zero")), None => finance::rate_on(conn, user.company_id, &currency, date).await? };
    let payment_mode = b.payment_mode.clone().unwrap_or_else(|| if po_type == "import" { "lc".into() } else { "credit".into() });
    if !["credit", "cash", "lc", "tt"].contains(&payment_mode.as_str()) { return Err(AppError::field("payment_mode", "Must be credit, cash, lc or tt")); }
    if po_type == "import" {
        match b.incoterm.as_deref() { Some(i) if ["EXW", "FOB", "CFR", "CIF", "CPT", "CIP", "DAP", "DDP"].contains(&i) => {}, _ => return Err(AppError::field("incoterm", "Choose an Incoterm for an import order (for example FOB or CFR)")) }
    } else if currency != "BDT" { return Err(AppError::field("currency", "A local purchase order must be in BDT")); }
    if b.discount < Decimal::ZERO { return Err(AppError::field("discount", "Must be zero or more")); }

    let mut subtotal = Decimal::ZERO; let mut vat = Decimal::ZERO;
    let mut rows = vec![];
    for (i, l) in b.lines.iter().enumerate() {
        if l.qty <= Decimal::ZERO { return Err(AppError::field(&format!("lines[{i}].qty"), "Must be more than zero")); }
        if l.unit_price < Decimal::ZERO { return Err(AppError::field(&format!("lines[{i}].unit_price"), "Must be zero or more")); }
        let pv: Decimal = sqlx::query_scalar("SELECT vat_rate FROM products WHERE id = $1 AND company_id = $2 AND is_active").bind(l.product_id).bind(user.company_id).fetch_optional(&mut *conn).await?.ok_or_else(|| AppError::field(&format!("lines[{i}].product_id"), "Product does not exist or is inactive"))?;
        // Import VAT is assessed at customs, not on the supplier's invoice.
        let rate = if po_type == "import" { Decimal::ZERO } else { l.vat_rate.unwrap_or(pv) };
        if rate < Decimal::ZERO || rate > Decimal::ONE_HUNDRED { return Err(AppError::field(&format!("lines[{i}].vat_rate"), "Must be between 0 and 100")); }
        let total = r2(l.qty * l.unit_price);
        subtotal += total; vat += pct(total, rate);
        rows.push((l, rate, total));
    }
    if b.discount > subtotal { return Err(AppError::field("discount", "Discount cannot be more than the order value")); }
    if !b.discount.is_zero() && !subtotal.is_zero() { vat = r2(vat * (subtotal - b.discount) / subtotal); }
    let total = subtotal - b.discount + vat;
    let branch = user.branch_for(b.branch_id)?;
    if let Some(r) = b.requisition_id { db::ensure(conn, "purchase_requisitions", user.company_id, r, "requisition_id").await?; }

    let id = match existing {
        Some(id) => {
            sqlx::query("UPDATE purchase_orders SET po_type=$3, supplier_id=$4, order_date=$5, expected_date=$6, currency=$7, fx_rate=$8, incoterm=$9, payment_mode=$10, port_of_loading=$11, port_of_discharge=$12, pi_no=$13, pi_date=$14, subtotal=$15, discount=$16, vat_amount=$17, total=$18, total_bdt=$19, notes=$20, requisition_id=$21 WHERE id=$1 AND company_id=$2")
                .bind(id).bind(user.company_id).bind(&po_type).bind(b.supplier_id).bind(date).bind(b.expected_date).bind(&currency).bind(fx).bind(&b.incoterm).bind(&payment_mode).bind(&b.port_of_loading).bind(&b.port_of_discharge).bind(&b.pi_no).bind(b.pi_date)
                .bind(subtotal).bind(b.discount).bind(vat).bind(total).bind(r2(total * fx)).bind(&b.notes).bind(b.requisition_id).execute(&mut *conn).await?;
            sqlx::query("DELETE FROM purchase_order_lines WHERE po_id = $1").bind(id).execute(&mut *conn).await?;
            id
        }
        None => {
            let no = db::next_no(conn, user.company_id, "purchase_order", "PO").await?;
            sqlx::query_scalar("INSERT INTO purchase_orders (company_id, branch_id, po_no, po_type, supplier_id, order_date, expected_date, currency, fx_rate, incoterm, payment_mode, port_of_loading, port_of_discharge, pi_no, pi_date, subtotal, discount, vat_amount, total, total_bdt, notes, requisition_id, created_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22,$23) RETURNING id")
                .bind(user.company_id).bind(branch).bind(&no).bind(&po_type).bind(b.supplier_id).bind(date).bind(b.expected_date).bind(&currency).bind(fx).bind(&b.incoterm).bind(&payment_mode).bind(&b.port_of_loading).bind(&b.port_of_discharge).bind(&b.pi_no).bind(b.pi_date)
                .bind(subtotal).bind(b.discount).bind(vat).bind(total).bind(r2(total * fx)).bind(&b.notes).bind(b.requisition_id).bind(user.user_id).fetch_one(&mut *conn).await?
        }
    };
    for (i, (l, rate, total)) in rows.iter().enumerate() {
        sqlx::query("INSERT INTO purchase_order_lines (po_id, line_no, product_id, description, qty, unit_price, vat_rate, line_total) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)")
            .bind(id).bind(i as i32 + 1).bind(l.product_id).bind(&l.description).bind(l.qty).bind(l.unit_price).bind(rate).bind(total).execute(&mut *conn).await?;
    }
    Ok(id)
}

async fn create_po(State(st): State<AppState>, user: AuthUser, Json(b): Json<PoIn>) -> AppResult<Created> {
    let mut tx = st.db.begin().await?;
    let id = save_po(&mut tx, &user, &b, None).await?;
    db::audit(&mut tx, &user, "create", "purchase_orders", id, json!({"supplier_id": b.supplier_id})).await?;
    let doc: String = sqlx::query_scalar(PO_DOC).bind(id).bind(user.company_id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(created(doc))
}

async fn po_status(conn: &mut PgConnection, company: Uuid, id: Uuid) -> AppResult<(String, String, Decimal, String)> {
    sqlx::query_as("SELECT status, po_no, total_bdt, po_type FROM purchase_orders WHERE id = $1 AND company_id = $2 FOR UPDATE").bind(id).bind(company).fetch_optional(&mut *conn).await?.ok_or_else(|| AppError::not_found("purchase order"))
}

async fn update_po(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>, Json(b): Json<PoIn>) -> AppResult<RawJson> {
    let mut tx = st.db.begin().await?;
    let (status, ..) = po_status(&mut tx, user.company_id, id).await?;
    if status != "draft" && status != "rejected" { return Err(AppError::rule("Only a draft order can be edited")); }
    save_po(&mut tx, &user, &b, Some(id)).await?;
    sqlx::query("UPDATE purchase_orders SET status = 'draft' WHERE id = $1").bind(id).execute(&mut *tx).await?;
    db::audit(&mut tx, &user, "update", "purchase_orders", id, json!({})).await?;
    let doc: String = sqlx::query_scalar(PO_DOC).bind(id).bind(user.company_id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(data(doc))
}

async fn submit_po(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> AppResult<RawJson> {
    let mut tx = st.db.begin().await?;
    let (status, no, total, _) = po_status(&mut tx, user.company_id, id).await?;
    if status != "draft" { return Err(AppError::rule(format!("Order {no} is already {status}"))); }
    if approvals::submit(&mut tx, &user, "purchase_order", id, &no, total).await? {
        sqlx::query("UPDATE purchase_orders SET status = 'pending_approval' WHERE id = $1").bind(id).execute(&mut *tx).await?;
    } else {
        sqlx::query("UPDATE purchase_orders SET status = 'approved', approved_by = $2, approved_at = now() WHERE id = $1").bind(id).bind(user.user_id).execute(&mut *tx).await?;
        mark_requisition(&mut tx, id).await?;
    }
    db::audit(&mut tx, &user, "submit", "purchase_orders", id, json!({"total_bdt": total})).await?;
    let doc: String = sqlx::query_scalar(PO_DOC).bind(id).bind(user.company_id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(data(doc))
}

async fn mark_requisition(conn: &mut PgConnection, po: Uuid) -> AppResult<()> {
    sqlx::query("UPDATE purchase_requisitions SET status = 'ordered' WHERE id = (SELECT requisition_id FROM purchase_orders WHERE id = $1) AND status = 'open'").bind(po).execute(&mut *conn).await?;
    Ok(())
}

pub async fn on_approval(conn: &mut PgConnection, user: &AuthUser, id: Uuid, approved: bool) -> AppResult<()> {
    if approved {
        sqlx::query("UPDATE purchase_orders SET status = 'approved', approved_by = $2, approved_at = now() WHERE id = $1 AND company_id = $3 AND status = 'pending_approval'").bind(id).bind(user.user_id).bind(user.company_id).execute(&mut *conn).await?;
        mark_requisition(conn, id).await
    } else {
        sqlx::query("UPDATE purchase_orders SET status = 'rejected' WHERE id = $1 AND company_id = $2 AND status = 'pending_approval'").bind(id).bind(user.company_id).execute(&mut *conn).await?;
        Ok(())
    }
}

async fn cancel_po(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> AppResult<RawJson> {
    let mut tx = st.db.begin().await?;
    let (status, no, ..) = po_status(&mut tx, user.company_id, id).await?;
    if !["draft", "pending_approval", "approved", "rejected"].contains(&status.as_str()) { return Err(AppError::rule(format!("Order {no} is {status} and cannot be cancelled"))); }
    let used: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM purchase_order_lines WHERE po_id = $1 AND (received_qty > 0 OR shipped_qty > 0)) OR EXISTS (SELECT 1 FROM letters_of_credit WHERE po_id = $1 AND status = 'opened') OR EXISTS (SELECT 1 FROM foreign_payments WHERE po_id = $1 AND status = 'posted')").bind(id).fetch_one(&mut *tx).await?;
    if used { return Err(AppError::rule("This order already has shipments, receipts, an open LC or payments and cannot be cancelled")); }
    sqlx::query("UPDATE purchase_orders SET status = 'cancelled' WHERE id = $1").bind(id).execute(&mut *tx).await?;
    sqlx::query("UPDATE approval_requests SET status = 'cancelled' WHERE entity_type = 'purchase_order' AND entity_id = $1 AND status IN ('pending','waiting')").bind(id).execute(&mut *tx).await?;
    db::audit(&mut tx, &user, "cancel", "purchase_orders", id, json!({})).await?;
    let doc: String = sqlx::query_scalar(PO_DOC).bind(id).bind(user.company_id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(data(doc))
}

async fn close_po(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> AppResult<RawJson> {
    let mut tx = st.db.begin().await?;
    let (status, no, ..) = po_status(&mut tx, user.company_id, id).await?;
    if !["approved", "partially_received", "received"].contains(&status.as_str()) { return Err(AppError::rule(format!("Order {no} is {status} and cannot be closed"))); }
    sqlx::query("UPDATE purchase_orders SET status = 'closed' WHERE id = $1").bind(id).execute(&mut *tx).await?;
    db::audit(&mut tx, &user, "close", "purchase_orders", id, json!({})).await?;
    let doc: String = sqlx::query_scalar(PO_DOC).bind(id).bind(user.company_id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(data(doc))
}

async fn list_pos(State(st): State<AppState>, user: AuthUser, Query(q): Query<HashMap<String, String>>) -> AppResult<RawJson> {
    let mut qb = db::list_query();
    qb.push("o.id, o.po_no, o.po_type, o.order_date, o.expected_date, o.currency, o.fx_rate, o.incoterm, o.payment_mode, o.total, o.total_bdt, o.status, o.pi_no, o.supplier_id, s.name AS supplier_name, o.created_at, (SELECT coalesce(sum(l.received_qty) / nullif(sum(l.qty), 0) * 100, 0) FROM purchase_order_lines l WHERE l.po_id = o.id) AS received_pct FROM purchase_orders o JOIN suppliers s ON s.id = o.supplier_id WHERE o.company_id = ");
    qb.push_bind(user.company_id);
    db::filter_eq(&mut qb, &q, "status", "o.status", "");
    db::filter_eq(&mut qb, &q, "po_type", "o.po_type", "");
    db::filter_eq(&mut qb, &q, "supplier_id", "o.supplier_id", "::uuid");
    db::filter_dates(&mut qb, &q, "o.order_date");
    db::filter_search(&mut qb, &q, &["o.po_no", "s.name", "o.pi_no"]);
    if let Some(br) = user.branch_id { qb.push(" AND o.branch_id = "); qb.push_bind(br); }
    db::page_json(&st.db, qb, "o.created_at DESC", &q).await
}
async fn get_po(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> AppResult<RawJson> {
    Ok(data(sqlx::query_scalar(PO_DOC).bind(id).bind(user.company_id).fetch_optional(&st.db).await?.ok_or_else(|| AppError::not_found("purchase order"))?))
}

/// Recompute a PO's receipt status from its lines.
pub async fn refresh_po_status(conn: &mut PgConnection, po: Uuid) -> AppResult<()> {
    sqlx::query("UPDATE purchase_orders o SET status = CASE WHEN x.rec >= x.qty THEN 'received' WHEN x.rec > 0 THEN 'partially_received' ELSE o.status END FROM (SELECT sum(qty) AS qty, sum(received_qty) AS rec FROM purchase_order_lines WHERE po_id = $1) x WHERE o.id = $1 AND o.status IN ('approved','partially_received')")
        .bind(po).execute(&mut *conn).await?;
    Ok(())
}

// ---------------- goods receipts (local purchases) ----------------
#[derive(Deserialize)]
struct GrnLine { po_line_id: Uuid, qty: Decimal, batch_no: Option<String> }
#[derive(Deserialize)]
struct GrnIn { po_id: Uuid, warehouse_id: Uuid, receipt_date: Option<NaiveDate>, notes: Option<String>, lines: Vec<GrnLine> }

pub const GRN_DOC: &str = "SELECT (to_jsonb(g) || jsonb_build_object('po_no', o.po_no, 'supplier_id', o.supplier_id, 'supplier_name', s.name, 'warehouse', (SELECT name FROM warehouses w WHERE w.id = g.warehouse_id), 'shipment_no', (SELECT shipment_no FROM shipments sh WHERE sh.id = g.shipment_id),
  'lines', (SELECT coalesce(jsonb_agg(jsonb_build_object('id', l.id, 'po_line_id', l.po_line_id, 'product_id', l.product_id, 'sku', p.sku, 'product_name', p.name, 'qty', l.qty, 'unit_cost', l.unit_cost, 'value', l.value, 'batch_no', l.batch_no)), '[]') FROM goods_receipt_lines l JOIN products p ON p.id = l.product_id WHERE l.grn_id = g.id)))::text
  FROM goods_receipts g JOIN purchase_orders o ON o.id = g.po_id JOIN suppliers s ON s.id = o.supplier_id WHERE g.id = $1 AND g.company_id = $2";

async fn create_grn(State(st): State<AppState>, user: AuthUser, Json(b): Json<GrnIn>) -> AppResult<Created> {
    if b.lines.is_empty() { return Err(AppError::field("lines", "Add at least one item")); }
    let mut tx = st.db.begin().await?;
    let (status, po_no, _, po_type) = po_status(&mut tx, user.company_id, b.po_id).await?;
    if po_type != "local" { return Err(AppError::rule("Import orders are received from their shipment after landed cost is finalised")); }
    if status != "approved" && status != "partially_received" { return Err(AppError::rule(format!("Order {po_no} is {status}; only approved orders can be received"))); }
    db::ensure(&mut tx, "warehouses", user.company_id, b.warehouse_id, "warehouse_id").await?;
    let (fx, subtotal, discount, branch): (Decimal, Decimal, Decimal, Option<Uuid>) = sqlx::query_as("SELECT fx_rate, subtotal, discount, branch_id FROM purchase_orders WHERE id = $1").bind(b.po_id).fetch_one(&mut *tx).await?;
    let factor = if subtotal.is_zero() { Decimal::ONE } else { (subtotal - discount) / subtotal };
    let date = match b.receipt_date { Some(d) => d, None => db::today(&mut tx, user.company_id).await? };
    let no = db::next_no(&mut tx, user.company_id, "goods_receipt", "GRN").await?;
    let id: Uuid = sqlx::query_scalar("INSERT INTO goods_receipts (company_id, branch_id, grn_no, po_id, warehouse_id, receipt_date, notes, created_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8) RETURNING id")
        .bind(user.company_id).bind(branch).bind(&no).bind(b.po_id).bind(b.warehouse_id).bind(date).bind(&b.notes).bind(user.user_id).fetch_one(&mut *tx).await?;
    let mut total = Decimal::ZERO;
    for (i, l) in b.lines.iter().enumerate() {
        if l.qty <= Decimal::ZERO { return Err(AppError::field(&format!("lines[{i}].qty"), "Must be more than zero")); }
        let (product, qty, received, price): (Uuid, Decimal, Decimal, Decimal) = sqlx::query_as("SELECT product_id, qty, received_qty, unit_price FROM purchase_order_lines WHERE id = $1 AND po_id = $2 FOR UPDATE").bind(l.po_line_id).bind(b.po_id).fetch_optional(&mut *tx).await?.ok_or_else(|| AppError::field(&format!("lines[{i}].po_line_id"), "This line is not on the order"))?;
        if l.qty > qty - received { return Err(AppError::field(&format!("lines[{i}].qty"), format!("Only {} left to receive on this line", (qty - received).normalize()))); }
        let unit_cost = r6(price * fx * factor);
        let value = inventory::stock_move(&mut tx, Move { company: user.company_id, warehouse: b.warehouse_id, product, date, qty: l.qty, unit_cost: Some(unit_cost), value: None, move_type: "receipt", source_type: "goods_receipt", source_id: Some(id), batch: l.batch_no.as_deref(), user: user.user_id }).await?;
        total += value;
        sqlx::query("INSERT INTO goods_receipt_lines (grn_id, po_line_id, product_id, qty, unit_cost, value, batch_no) VALUES ($1,$2,$3,$4,$5,$6,$7)").bind(id).bind(l.po_line_id).bind(product).bind(l.qty).bind(unit_cost).bind(value).bind(&l.batch_no).execute(&mut *tx).await?;
        sqlx::query("UPDATE purchase_order_lines SET received_qty = received_qty + $2 WHERE id = $1").bind(l.po_line_id).bind(l.qty).execute(&mut *tx).await?;
    }
    let je = accounting::post(&mut tx, Post { company: user.company_id, branch, date, source_type: "goods_receipt", source_id: Some(id), memo: &format!("Goods received {no} against {po_no}"), user: user.user_id },
        vec![JL::dr(Acc::Key("inventory"), total), JL::cr(Acc::Key("grni"), total)]).await?;
    sqlx::query("UPDATE goods_receipts SET total_value = $2, journal_entry_id = $3 WHERE id = $1").bind(id).bind(total).bind(je).execute(&mut *tx).await?;
    refresh_po_status(&mut tx, b.po_id).await?;
    db::audit(&mut tx, &user, "post", "goods_receipts", id, json!({"grn_no": no, "value": total})).await?;
    let doc: String = sqlx::query_scalar(GRN_DOC).bind(id).bind(user.company_id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(created(doc))
}

async fn list_grns(State(st): State<AppState>, user: AuthUser, Query(q): Query<HashMap<String, String>>) -> AppResult<RawJson> {
    let mut qb = db::list_query();
    qb.push("g.id, g.grn_no, g.receipt_date, g.total_value, g.billed, g.po_id, o.po_no, o.po_type, g.shipment_id, s.name AS supplier_name, (SELECT name FROM warehouses w WHERE w.id = g.warehouse_id) AS warehouse, g.created_at FROM goods_receipts g JOIN purchase_orders o ON o.id = g.po_id JOIN suppliers s ON s.id = o.supplier_id WHERE g.company_id = ");
    qb.push_bind(user.company_id);
    db::filter_eq(&mut qb, &q, "po_id", "g.po_id", "::uuid");
    db::filter_eq(&mut qb, &q, "supplier_id", "o.supplier_id", "::uuid");
    if q.get("unbilled").map(String::as_str) == Some("true") { qb.push(" AND NOT g.billed AND o.po_type = 'local'"); }
    db::filter_dates(&mut qb, &q, "g.receipt_date");
    db::page_json(&st.db, qb, "g.created_at DESC", &q).await
}
async fn get_grn(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> AppResult<RawJson> {
    Ok(data(sqlx::query_scalar(GRN_DOC).bind(id).bind(user.company_id).fetch_optional(&st.db).await?.ok_or_else(|| AppError::not_found("goods receipt"))?))
}

// ---------------- supplier bills ----------------
#[derive(Deserialize)]
struct BillIn { supplier_id: Uuid, grn_id: Option<Uuid>, supplier_invoice_no: Option<String>, mushak_no: Option<String>, bill_date: Option<NaiveDate>, due_date: Option<NaiveDate>, subtotal: Decimal, #[serde(default)] vat_amount: Decimal, expense_account_id: Option<Uuid>, notes: Option<String> }

pub const BILL_DOC: &str = "SELECT (to_jsonb(b) || jsonb_build_object('supplier_name', s.name, 'balance', b.total - b.paid_amount, 'grn_no', (SELECT grn_no FROM goods_receipts g WHERE g.id = b.grn_id), 'po_no', (SELECT po_no FROM purchase_orders o WHERE o.id = b.po_id)))::text FROM supplier_bills b JOIN suppliers s ON s.id = b.supplier_id WHERE b.id = $1 AND b.company_id = $2";

async fn create_bill(State(st): State<AppState>, user: AuthUser, Json(b): Json<BillIn>) -> AppResult<Created> {
    if b.subtotal <= Decimal::ZERO { return Err(AppError::field("subtotal", "Must be more than zero")); }
    if b.vat_amount < Decimal::ZERO { return Err(AppError::field("vat_amount", "Must be zero or more")); }
    let mut tx = st.db.begin().await?;
    let (sup_type, terms): (String, i32) = sqlx::query_as("SELECT supplier_type, payment_terms_days FROM suppliers WHERE id = $1 AND company_id = $2").bind(b.supplier_id).bind(user.company_id).fetch_optional(&mut *tx).await?.ok_or_else(|| AppError::field("supplier_id", "Supplier does not exist"))?;
    if sup_type == "foreign" { return Err(AppError::rule("Foreign suppliers are settled through LC or TT payments, not supplier bills")); }
    if b.vat_amount > Decimal::ZERO && b.mushak_no.as_deref().map(str::trim).unwrap_or("").is_empty() {
        return Err(AppError::field("mushak_no", "Enter the supplier's Mushak 6.3 number to claim input VAT"));
    }
    let date = match b.bill_date { Some(d) => d, None => db::today(&mut tx, user.company_id).await? };
    let due = b.due_date.unwrap_or(date + Duration::days(terms as i64));
    let subtotal = r2(b.subtotal); let vat = r2(b.vat_amount); let total = subtotal + vat;
    let ap = JL::cr(Acc::Key("ap_local"), total).party("supplier", b.supplier_id);
    let (mut lines, po_id, branch) = match (b.grn_id, b.expense_account_id) {
        (Some(grn), _) => {
            let (value, billed, po, supplier, branch): (Decimal, bool, Uuid, Uuid, Option<Uuid>) = sqlx::query_as("SELECT g.total_value, g.billed, g.po_id, o.supplier_id, g.branch_id FROM goods_receipts g JOIN purchase_orders o ON o.id = g.po_id WHERE g.id = $1 AND g.company_id = $2 AND o.po_type = 'local' FOR UPDATE OF g").bind(grn).bind(user.company_id).fetch_optional(&mut *tx).await?.ok_or_else(|| AppError::field("grn_id", "Goods receipt does not exist"))?;
            if billed { return Err(AppError::rule("This goods receipt is already billed")); }
            if supplier != b.supplier_id { return Err(AppError::field("grn_id", "This goods receipt belongs to another supplier")); }
            sqlx::query("UPDATE goods_receipts SET billed = true WHERE id = $1").bind(grn).execute(&mut *tx).await?;
            sqlx::query("UPDATE purchase_order_lines pl SET billed_qty = billed_qty + gl.qty FROM goods_receipt_lines gl WHERE gl.grn_id = $1 AND gl.po_line_id = pl.id").bind(grn).execute(&mut *tx).await?;
            // Any difference between the bill and the received value is a purchase price variance.
            (vec![JL::dr(Acc::Key("grni"), value), JL::dr(Acc::Key("inventory_adjustment"), subtotal - value).memo("Purchase price variance")], Some(po), branch)
        }
        (None, Some(acc)) => {
            let kind: String = sqlx::query_scalar("SELECT account_type FROM accounts WHERE id = $1 AND company_id = $2 AND NOT is_group").bind(acc).bind(user.company_id).fetch_optional(&mut *tx).await?.ok_or_else(|| AppError::field("expense_account_id", "Account does not exist"))?;
            if kind != "expense" && kind != "asset" { return Err(AppError::field("expense_account_id", "Choose an expense or asset account")); }
            (vec![JL::dr(Acc::Id(acc), subtotal)], None, user.branch_id)
        }
        (None, None) => return Err(AppError::field("grn_id", "Link a goods receipt, or choose an expense account for a service bill")),
    };
    lines.push(JL::dr(Acc::Key("vat_input"), vat));
    lines.push(ap);
    let no = db::next_no(&mut tx, user.company_id, "supplier_bill", "BILL").await?;
    let id: Uuid = sqlx::query_scalar("INSERT INTO supplier_bills (company_id, branch_id, bill_no, supplier_id, po_id, grn_id, supplier_invoice_no, mushak_no, bill_date, due_date, subtotal, vat_amount, total, expense_account_id, notes, created_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16) RETURNING id")
        .bind(user.company_id).bind(branch).bind(&no).bind(b.supplier_id).bind(po_id).bind(b.grn_id).bind(&b.supplier_invoice_no).bind(&b.mushak_no).bind(date).bind(due).bind(subtotal).bind(vat).bind(total).bind(b.expense_account_id).bind(&b.notes).bind(user.user_id).fetch_one(&mut *tx).await?;
    let je = accounting::post(&mut tx, Post { company: user.company_id, branch, date, source_type: "supplier_bill", source_id: Some(id), memo: &format!("Supplier bill {no}"), user: user.user_id }, lines).await?;
    sqlx::query("UPDATE supplier_bills SET journal_entry_id = $2 WHERE id = $1").bind(id).bind(je).execute(&mut *tx).await?;
    db::audit(&mut tx, &user, "post", "supplier_bills", id, json!({"bill_no": no, "total": total})).await?;
    let doc: String = sqlx::query_scalar(BILL_DOC).bind(id).bind(user.company_id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(created(doc))
}

async fn list_bills(State(st): State<AppState>, user: AuthUser, Query(q): Query<HashMap<String, String>>) -> AppResult<RawJson> {
    let mut qb = db::list_query();
    qb.push("b.id, b.bill_no, b.bill_date, b.due_date, b.subtotal, b.vat_amount, b.total, b.paid_amount, b.total - b.paid_amount AS balance, b.status, b.supplier_id, s.name AS supplier_name, b.supplier_invoice_no, b.mushak_no, (b.status IN ('open','partial') AND b.due_date < (now() AT TIME ZONE 'Asia/Dhaka')::date) AS overdue FROM supplier_bills b JOIN suppliers s ON s.id = b.supplier_id WHERE b.company_id = ");
    qb.push_bind(user.company_id);
    db::filter_eq(&mut qb, &q, "status", "b.status", "");
    db::filter_eq(&mut qb, &q, "supplier_id", "b.supplier_id", "::uuid");
    if q.get("unpaid").map(String::as_str) == Some("true") { qb.push(" AND b.status IN ('open','partial')"); }
    db::filter_dates(&mut qb, &q, "b.bill_date");
    db::filter_search(&mut qb, &q, &["b.bill_no", "s.name", "b.supplier_invoice_no"]);
    db::page_json(&st.db, qb, "bill_date DESC, bill_no DESC", &q).await
}
async fn get_bill(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> AppResult<RawJson> {
    Ok(data(sqlx::query_scalar(BILL_DOC).bind(id).bind(user.company_id).fetch_optional(&st.db).await?.ok_or_else(|| AppError::not_found("supplier bill"))?))
}

pub fn api() -> Api {
    Api::new("Procurement")
        .get("/requisitions", "purchase.read", "Purchase requisitions", list_requisitions).query("page,per_page,status")
        .post("/requisitions", "purchase.write", "Ask for items to be bought", create_requisition).body(r#"{"required_date":"2026-12-15","notes":"Stock for winter season","lines":[{"product_id":"<uuid>","qty":1200}]}"#)
        .get("/requisitions/:id", "purchase.read", "One requisition with lines", get_requisition)
        .post("/requisitions/:id/cancel", "purchase.write", "Cancel an open requisition", cancel_requisition)
        .get("/purchase-orders", "purchase.read", "Purchase orders", list_pos).query("page,per_page,q,status,po_type,supplier_id,from,to")
        .post("/purchase-orders", "purchase.write", "Create a draft purchase order (local or import)", create_po)
        .body(r#"{"po_type":"import","supplier_id":"<uuid>","currency":"USD","fx_rate":121.50,"incoterm":"CFR","payment_mode":"lc","port_of_loading":"Shenzhen","port_of_discharge":"Chattogram","pi_no":"PI-554","pi_date":"2026-11-01","expected_date":"2026-12-28","lines":[{"product_id":"<uuid>","qty":2400,"unit_price":18.5}]}"#)
        .get("/purchase-orders/:id", "purchase.read", "One purchase order with lines and approvals", get_po)
        .put("/purchase-orders/:id", "purchase.write", "Replace a draft purchase order", update_po)
        .post("/purchase-orders/:id/submit", "purchase.write", "Submit for approval (auto-approves when no rule applies)", submit_po)
        .post("/purchase-orders/:id/cancel", "purchase.write", "Cancel an order that has no shipments, receipts or payments", cancel_po)
        .post("/purchase-orders/:id/close", "purchase.write", "Close an order; nothing more will be received", close_po)
        .get("/goods-receipts", "purchase.read", "Goods receipts (GRN)", list_grns).query("page,per_page,po_id,supplier_id,unbilled,from,to")
        .get("/goods-receipts/:id", "purchase.read", "One goods receipt with lines", get_grn)
        .post("/goods-receipts", "inventory.write", "Receive a local purchase order into a warehouse", create_grn)
        .body(r#"{"po_id":"<uuid>","warehouse_id":"<uuid>","receipt_date":"2026-11-12","lines":[{"po_line_id":"<uuid>","qty":100,"batch_no":"B-2231"}]}"#)
        .get("/supplier-bills", "purchase.read", "Supplier bills", list_bills).query("page,per_page,q,status,supplier_id,unpaid,from,to")
        .get("/supplier-bills/:id", "purchase.read", "One supplier bill", get_bill)
        .post("/supplier-bills", "purchase.bill", "Record a local supplier's bill against a goods receipt, or a service bill", create_bill)
        .body(r#"{"supplier_id":"<uuid>","grn_id":"<uuid>","supplier_invoice_no":"INV-889","mushak_no":"6.3-000451","bill_date":"2026-11-12","subtotal":250000,"vat_amount":37500}"#)
}
