//! Stock engine (weighted-average cost per warehouse) plus transfers, adjustments and opening stock.
use crate::{api::Api, auth::AuthUser, db::{self, created, data, r2, r6, Created, RawJson}, error::{AppError, AppResult}, modules::{accounting::{self, Acc, Post, JL}, approvals, notifications::{self, Note}}, state::AppState};
use axum::{extract::{Path, Query, State}, Json};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::json;
use sqlx::PgConnection;
use std::collections::HashMap;
use uuid::Uuid;

pub struct Move<'a> {
    pub company: Uuid,
    pub warehouse: Uuid,
    pub product: Uuid,
    pub date: NaiveDate,
    /// Signed quantity: positive receives, negative issues.
    pub qty: Decimal,
    /// Unit cost for receipts (BDT). Ignored for issues, which leave at average cost.
    pub unit_cost: Option<Decimal>,
    /// Exact receipt value, when it must match another document to the paisa.
    pub value: Option<Decimal>,
    pub move_type: &'a str,
    pub source_type: &'a str,
    pub source_id: Option<Uuid>,
    pub batch: Option<&'a str>,
    pub user: Uuid,
}

/// Apply one stock movement and return its absolute value in BDT.
/// The balance row is locked, so concurrent movements of the same item are serialised.
pub async fn stock_move(conn: &mut PgConnection, m: Move<'_>) -> AppResult<Decimal> {
    if m.qty.is_zero() { return Err(AppError::rule("Quantity cannot be zero")); }
    let (sku, kind): (String, String) = sqlx::query_as("SELECT sku, product_type FROM products WHERE id = $1 AND company_id = $2").bind(m.product).bind(m.company).fetch_optional(&mut *conn).await?.ok_or_else(|| AppError::not_found("product"))?;
    if kind != "goods" { return Err(AppError::rule(format!("{sku} is a service and is not kept in stock"))); }
    sqlx::query("INSERT INTO stock_balances (company_id, warehouse_id, product_id) VALUES ($1,$2,$3) ON CONFLICT DO NOTHING").bind(m.company).bind(m.warehouse).bind(m.product).execute(&mut *conn).await?;
    let (bq, bv, bavg): (Decimal, Decimal, Decimal) = sqlx::query_as("SELECT qty, value, avg_cost FROM stock_balances WHERE company_id = $1 AND warehouse_id = $2 AND product_id = $3 FOR UPDATE").bind(m.company).bind(m.warehouse).bind(m.product).fetch_one(&mut *conn).await?;

    let (value, unit_cost) = if m.qty > Decimal::ZERO {
        let v = match (m.value, m.unit_cost) {
            (Some(v), _) => r2(v),
            (None, Some(c)) => r2(m.qty * c),
            (None, None) => r2(m.qty * bavg),
        };
        if v < Decimal::ZERO { return Err(AppError::rule("Receipt value cannot be negative")); }
        (v, r6(v / m.qty))
    } else {
        let out = -m.qty;
        if out > bq {
            let allow = db::setting_bool(conn, m.company, "inventory.allow_negative_stock", false).await?;
            if !allow {
                let wh: String = sqlx::query_scalar("SELECT name FROM warehouses WHERE id = $1").bind(m.warehouse).fetch_one(&mut *conn).await?;
                return Err(AppError::rule(format!("Not enough stock of {sku} in {wh}: {} available, {} needed", bq.normalize(), out.normalize())));
            }
        }
        let avg = if bq > Decimal::ZERO { bv / bq } else { bavg };
        let v = if out == bq { bv } else { r2(out * avg) };
        (-v, r6(avg))
    };
    let nq = bq + m.qty;
    let nv = bv + value;
    let navg = if nq > Decimal::ZERO { r6(nv / nq) } else { bavg.max(unit_cost) };
    sqlx::query("UPDATE stock_balances SET qty = $4, value = $5, avg_cost = $6, updated_at = now() WHERE company_id = $1 AND warehouse_id = $2 AND product_id = $3")
        .bind(m.company).bind(m.warehouse).bind(m.product).bind(nq).bind(nv).bind(navg).execute(&mut *conn).await?;
    sqlx::query("INSERT INTO stock_moves (company_id, warehouse_id, product_id, move_date, move_type, qty, unit_cost, value, balance_qty, balance_value, source_type, source_id, batch_no, created_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)")
        .bind(m.company).bind(m.warehouse).bind(m.product).bind(m.date).bind(m.move_type).bind(m.qty).bind(unit_cost).bind(value).bind(nq).bind(nv).bind(m.source_type).bind(m.source_id).bind(m.batch).bind(m.user)
        .execute(&mut *conn).await?;

    if m.qty < Decimal::ZERO {
        // Low-stock alert when an issue takes the company-wide quantity to or below the reorder level.
        let (total, level, name): (Decimal, Decimal, String) = sqlx::query_as("SELECT coalesce((SELECT sum(qty) FROM stock_balances b WHERE b.product_id = p.id), 0), p.reorder_level, p.name FROM products p WHERE p.id = $1").bind(m.product).fetch_one(&mut *conn).await?;
        if level > Decimal::ZERO && total <= level {
            notifications::notify_roles(conn, m.company, &["import_officer", "manager"], &Note { severity: "warning", title: &format!("Low stock: {sku} {name}"), title_bn: Some("মজুদ কমে গেছে"), body: &format!("{} left; reorder level is {}", total.normalize(), level.normalize()), entity: Some("products"), entity_id: Some(m.product), dedupe: Some(format!("low:{}:{}", m.product, m.date)) }).await?;
        }
    }
    Ok(value.abs())
}

// ---------------- HTTP ----------------

async fn list_stock(State(st): State<AppState>, user: AuthUser, Query(q): Query<HashMap<String, String>>) -> AppResult<RawJson> {
    let mut qb = db::list_query();
    qb.push("b.warehouse_id, w.code AS warehouse_code, w.name AS warehouse_name, b.product_id, p.sku, p.name AS product_name, p.name_bn AS product_name_bn, (SELECT code FROM units u WHERE u.id = p.unit_id) AS unit, b.qty, b.avg_cost, b.value, p.reorder_level, p.sale_price, (p.reorder_level > 0 AND b.qty <= p.reorder_level) AS low FROM stock_balances b JOIN products p ON p.id = b.product_id JOIN warehouses w ON w.id = b.warehouse_id WHERE b.company_id = ");
    qb.push_bind(user.company_id);
    db::filter_eq(&mut qb, &q, "warehouse_id", "b.warehouse_id", "::uuid");
    db::filter_eq(&mut qb, &q, "product_id", "b.product_id", "::uuid");
    db::filter_eq(&mut qb, &q, "category_id", "p.category_id", "::uuid");
    db::filter_search(&mut qb, &q, &["p.sku", "p.name", "p.name_bn"]);
    if q.get("in_stock").map(String::as_str) == Some("true") { qb.push(" AND b.qty > 0"); }
    if q.get("low").map(String::as_str) == Some("true") { qb.push(" AND p.reorder_level > 0 AND b.qty <= p.reorder_level"); }
    if let Some(br) = user.branch_id { qb.push(" AND w.branch_id = "); qb.push_bind(br); }
    db::page_json(&st.db, qb, "sku, warehouse_code", &q).await
}

async fn list_moves(State(st): State<AppState>, user: AuthUser, Query(q): Query<HashMap<String, String>>) -> AppResult<RawJson> {
    let mut qb = db::list_query();
    qb.push("m.id, m.move_date, m.move_type, m.qty, m.unit_cost, m.value, m.balance_qty, m.balance_value, m.source_type, m.source_id, m.batch_no, m.created_at, p.sku, p.name AS product_name, w.code AS warehouse_code FROM stock_moves m JOIN products p ON p.id = m.product_id JOIN warehouses w ON w.id = m.warehouse_id WHERE m.company_id = ");
    qb.push_bind(user.company_id);
    db::filter_eq(&mut qb, &q, "warehouse_id", "m.warehouse_id", "::uuid");
    db::filter_eq(&mut qb, &q, "product_id", "m.product_id", "::uuid");
    db::filter_eq(&mut qb, &q, "move_type", "m.move_type", "");
    db::filter_eq(&mut qb, &q, "source_id", "m.source_id", "::uuid");
    db::filter_dates(&mut qb, &q, "m.move_date");
    db::page_json(&st.db, qb, "created_at DESC", &q).await
}

#[derive(Deserialize)]
struct TransferLine { product_id: Uuid, qty: Decimal }
#[derive(Deserialize)]
struct TransferIn { from_warehouse_id: Uuid, to_warehouse_id: Uuid, transfer_date: Option<NaiveDate>, notes: Option<String>, lines: Vec<TransferLine> }

const TRANSFER_DOC: &str = "SELECT (to_jsonb(t) || jsonb_build_object('from_warehouse', (SELECT name FROM warehouses w WHERE w.id = t.from_warehouse_id), 'to_warehouse', (SELECT name FROM warehouses w WHERE w.id = t.to_warehouse_id),
  'lines', (SELECT coalesce(jsonb_agg(jsonb_build_object('product_id', l.product_id, 'sku', p.sku, 'product_name', p.name, 'qty', l.qty, 'value', l.value)), '[]') FROM stock_transfer_lines l JOIN products p ON p.id = l.product_id WHERE l.transfer_id = t.id)))::text FROM stock_transfers t WHERE t.id = $1 AND t.company_id = $2";

async fn create_transfer(State(st): State<AppState>, user: AuthUser, Json(b): Json<TransferIn>) -> AppResult<Created> {
    if b.lines.is_empty() { return Err(AppError::field("lines", "Add at least one item")); }
    if b.from_warehouse_id == b.to_warehouse_id { return Err(AppError::field("to_warehouse_id", "Choose a different destination warehouse")); }
    let mut tx = st.db.begin().await?;
    db::ensure(&mut tx, "warehouses", user.company_id, b.from_warehouse_id, "from_warehouse_id").await?;
    db::ensure(&mut tx, "warehouses", user.company_id, b.to_warehouse_id, "to_warehouse_id").await?;
    let date = match b.transfer_date { Some(d) => d, None => db::today(&mut tx, user.company_id).await? };
    let no = db::next_no(&mut tx, user.company_id, "stock_transfer", "ST").await?;
    let id: Uuid = sqlx::query_scalar("INSERT INTO stock_transfers (company_id, transfer_no, transfer_date, from_warehouse_id, to_warehouse_id, notes, created_by) VALUES ($1,$2,$3,$4,$5,$6,$7) RETURNING id")
        .bind(user.company_id).bind(&no).bind(date).bind(b.from_warehouse_id).bind(b.to_warehouse_id).bind(&b.notes).bind(user.user_id).fetch_one(&mut *tx).await?;
    for (i, l) in b.lines.iter().enumerate() {
        if l.qty <= Decimal::ZERO { return Err(AppError::field(&format!("lines[{i}].qty"), "Must be more than zero")); }
        let v = stock_move(&mut tx, Move { company: user.company_id, warehouse: b.from_warehouse_id, product: l.product_id, date, qty: -l.qty, unit_cost: None, value: None, move_type: "transfer_out", source_type: "stock_transfer", source_id: Some(id), batch: None, user: user.user_id }).await?;
        stock_move(&mut tx, Move { company: user.company_id, warehouse: b.to_warehouse_id, product: l.product_id, date, qty: l.qty, unit_cost: None, value: Some(v), move_type: "transfer_in", source_type: "stock_transfer", source_id: Some(id), batch: None, user: user.user_id }).await?;
        sqlx::query("INSERT INTO stock_transfer_lines (transfer_id, product_id, qty, value) VALUES ($1,$2,$3,$4)").bind(id).bind(l.product_id).bind(l.qty).bind(v).execute(&mut *tx).await?;
    }
    db::audit(&mut tx, &user, "post", "stock_transfers", id, json!({"transfer_no": no})).await?;
    let doc: String = sqlx::query_scalar(TRANSFER_DOC).bind(id).bind(user.company_id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(created(doc))
}

async fn list_transfers(State(st): State<AppState>, user: AuthUser, Query(q): Query<HashMap<String, String>>) -> AppResult<RawJson> {
    let mut qb = db::list_query();
    qb.push("t.*, (SELECT name FROM warehouses w WHERE w.id = t.from_warehouse_id) AS from_warehouse, (SELECT name FROM warehouses w WHERE w.id = t.to_warehouse_id) AS to_warehouse, (SELECT coalesce(sum(value),0) FROM stock_transfer_lines l WHERE l.transfer_id = t.id) AS total_value FROM stock_transfers t WHERE t.company_id = ");
    qb.push_bind(user.company_id);
    db::filter_dates(&mut qb, &q, "t.transfer_date");
    db::page_json(&st.db, qb, "created_at DESC", &q).await
}
async fn get_transfer(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> AppResult<RawJson> {
    Ok(data(sqlx::query_scalar(TRANSFER_DOC).bind(id).bind(user.company_id).fetch_optional(&st.db).await?.ok_or_else(|| AppError::not_found("transfer"))?))
}

#[derive(Deserialize)]
struct AdjLine { product_id: Uuid, qty_change: Decimal, unit_cost: Option<Decimal> }
#[derive(Deserialize)]
struct AdjIn { warehouse_id: Uuid, adjustment_date: Option<NaiveDate>, reason: String, lines: Vec<AdjLine> }

const ADJ_DOC: &str = "SELECT (to_jsonb(a) || jsonb_build_object('warehouse', (SELECT name FROM warehouses w WHERE w.id = a.warehouse_id),
  'lines', (SELECT coalesce(jsonb_agg(jsonb_build_object('product_id', l.product_id, 'sku', p.sku, 'product_name', p.name, 'qty_change', l.qty_change, 'unit_cost', l.unit_cost, 'value', l.value)), '[]') FROM stock_adjustment_lines l JOIN products p ON p.id = l.product_id WHERE l.adjustment_id = a.id)))::text FROM stock_adjustments a WHERE a.id = $1 AND a.company_id = $2";

/// Post a stored adjustment: move the stock and book the difference.
pub async fn post_adjustment(conn: &mut PgConnection, user: &AuthUser, id: Uuid) -> AppResult<()> {
    let (no, date, wh, opening): (String, NaiveDate, Uuid, bool) = sqlx::query_as("SELECT adjustment_no, adjustment_date, warehouse_id, is_opening FROM stock_adjustments WHERE id = $1 AND company_id = $2 FOR UPDATE").bind(id).bind(user.company_id).fetch_one(&mut *conn).await?;
    let lines: Vec<(Uuid, Uuid, Decimal, Option<Decimal>)> = sqlx::query_as("SELECT id, product_id, qty_change, unit_cost FROM stock_adjustment_lines WHERE adjustment_id = $1").bind(id).fetch_all(&mut *conn).await?;
    let (mut inc, mut dec) = (Decimal::ZERO, Decimal::ZERO);
    for (lid, product, qty, cost) in lines {
        let v = stock_move(conn, Move { company: user.company_id, warehouse: wh, product, date, qty, unit_cost: cost, value: None, move_type: if opening { "opening" } else if qty > Decimal::ZERO { "adjustment_in" } else { "adjustment_out" }, source_type: "stock_adjustment", source_id: Some(id), batch: None, user: user.user_id }).await?;
        if qty > Decimal::ZERO { inc += v } else { dec += v }
        sqlx::query("UPDATE stock_adjustment_lines SET value = $2 WHERE id = $1").bind(lid).bind(v).execute(&mut *conn).await?;
    }
    let contra = if opening { "opening_equity" } else { "inventory_adjustment" };
    let mut jl = vec![];
    if inc > Decimal::ZERO { jl.push(JL::dr(Acc::Key("inventory"), inc)); jl.push(JL::cr(Acc::Key(contra), inc)); }
    if dec > Decimal::ZERO { jl.push(JL::dr(Acc::Key(contra), dec)); jl.push(JL::cr(Acc::Key("inventory"), dec)); }
    let je = if jl.is_empty() { None } else {
        Some(accounting::post(conn, Post { company: user.company_id, branch: None, date, source_type: "stock_adjustment", source_id: Some(id), memo: &format!("Stock adjustment {no}"), user: user.user_id }, jl).await?)
    };
    sqlx::query("UPDATE stock_adjustments SET status = 'posted', journal_entry_id = $2, total_value = $3 WHERE id = $1").bind(id).bind(je).bind(inc + dec).execute(&mut *conn).await?;
    Ok(())
}

pub async fn on_adjustment_approval(conn: &mut PgConnection, user: &AuthUser, id: Uuid, approved: bool) -> AppResult<()> {
    if approved { post_adjustment(conn, user, id).await } else {
        sqlx::query("UPDATE stock_adjustments SET status = 'rejected' WHERE id = $1 AND company_id = $2").bind(id).bind(user.company_id).execute(&mut *conn).await?;
        Ok(())
    }
}

async fn save_adjustment(st: AppState, user: AuthUser, b: AdjIn, opening: bool) -> AppResult<Created> {
    if b.lines.is_empty() { return Err(AppError::field("lines", "Add at least one item")); }
    if b.reason.trim().is_empty() { return Err(AppError::field("reason", "Please give a reason")); }
    let mut tx = st.db.begin().await?;
    db::ensure(&mut tx, "warehouses", user.company_id, b.warehouse_id, "warehouse_id").await?;
    let date = match b.adjustment_date { Some(d) => d, None => db::today(&mut tx, user.company_id).await? };
    let no = db::next_no(&mut tx, user.company_id, "stock_adjustment", "ADJ").await?;
    let id: Uuid = sqlx::query_scalar("INSERT INTO stock_adjustments (company_id, adjustment_no, adjustment_date, warehouse_id, reason, is_opening, created_by) VALUES ($1,$2,$3,$4,$5,$6,$7) RETURNING id")
        .bind(user.company_id).bind(&no).bind(date).bind(b.warehouse_id).bind(b.reason.trim()).bind(opening).bind(user.user_id).fetch_one(&mut *tx).await?;
    let mut estimate = Decimal::ZERO;
    for (i, l) in b.lines.iter().enumerate() {
        if l.qty_change.is_zero() { return Err(AppError::field(&format!("lines[{i}].qty_change"), "Cannot be zero")); }
        if opening && l.qty_change < Decimal::ZERO { return Err(AppError::field(&format!("lines[{i}].qty_change"), "Opening stock must be positive")); }
        db::ensure(&mut tx, "products", user.company_id, l.product_id, &format!("lines[{i}].product_id")).await?;
        let avg: Option<Decimal> = sqlx::query_scalar("SELECT avg_cost FROM stock_balances WHERE company_id = $1 AND warehouse_id = $2 AND product_id = $3").bind(user.company_id).bind(b.warehouse_id).bind(l.product_id).fetch_optional(&mut *tx).await?;
        if l.qty_change > Decimal::ZERO && l.unit_cost.is_none() && avg.unwrap_or_default().is_zero() {
            return Err(AppError::field(&format!("lines[{i}].unit_cost"), "Enter a unit cost: this item has no cost yet"));
        }
        if let Some(c) = l.unit_cost { if c < Decimal::ZERO { return Err(AppError::field(&format!("lines[{i}].unit_cost"), "Must be zero or more")); } }
        estimate += r2(l.qty_change.abs() * l.unit_cost.or(avg).unwrap_or_default());
        sqlx::query("INSERT INTO stock_adjustment_lines (adjustment_id, product_id, qty_change, unit_cost) VALUES ($1,$2,$3,$4)").bind(id).bind(l.product_id).bind(l.qty_change).bind(l.unit_cost).execute(&mut *tx).await?;
    }
    sqlx::query("UPDATE stock_adjustments SET total_value = $2 WHERE id = $1").bind(id).bind(estimate).execute(&mut *tx).await?;
    let needs = !opening && approvals::submit(&mut tx, &user, "stock_adjustment", id, &no, estimate).await?;
    if needs {
        sqlx::query("UPDATE stock_adjustments SET status = 'pending_approval' WHERE id = $1").bind(id).execute(&mut *tx).await?;
    } else {
        post_adjustment(&mut tx, &user, id).await?;
    }
    db::audit(&mut tx, &user, if needs { "submit" } else { "post" }, "stock_adjustments", id, json!({"adjustment_no": no, "value": estimate, "opening": opening})).await?;
    let doc: String = sqlx::query_scalar(ADJ_DOC).bind(id).bind(user.company_id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(created(doc))
}
async fn create_adjustment(State(st): State<AppState>, user: AuthUser, Json(b): Json<AdjIn>) -> AppResult<Created> { save_adjustment(st, user, b, false).await }
async fn create_opening(State(st): State<AppState>, user: AuthUser, Json(b): Json<AdjIn>) -> AppResult<Created> { save_adjustment(st, user, b, true).await }

async fn list_adjustments(State(st): State<AppState>, user: AuthUser, Query(q): Query<HashMap<String, String>>) -> AppResult<RawJson> {
    let mut qb = db::list_query();
    qb.push("a.*, (SELECT name FROM warehouses w WHERE w.id = a.warehouse_id) AS warehouse FROM stock_adjustments a WHERE a.company_id = ");
    qb.push_bind(user.company_id);
    db::filter_eq(&mut qb, &q, "status", "a.status", "");
    db::filter_dates(&mut qb, &q, "a.adjustment_date");
    db::page_json(&st.db, qb, "created_at DESC", &q).await
}
async fn get_adjustment(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> AppResult<RawJson> {
    Ok(data(sqlx::query_scalar(ADJ_DOC).bind(id).bind(user.company_id).fetch_optional(&st.db).await?.ok_or_else(|| AppError::not_found("adjustment"))?))
}

pub fn api() -> Api {
    Api::new("Inventory")
        .get("/stock", "inventory.read", "Stock on hand by warehouse and product", list_stock).query("page,per_page,q,warehouse_id,product_id,category_id,in_stock,low")
        .get("/stock/moves", "inventory.read", "Stock ledger: every movement with running balance", list_moves).query("page,per_page,warehouse_id,product_id,move_type,source_id,from,to")
        .get("/stock/transfers", "inventory.read", "Transfers between warehouses", list_transfers).query("page,per_page,from,to")
        .get("/stock/transfers/:id", "inventory.read", "One transfer with lines", get_transfer)
        .post("/stock/transfers", "inventory.write", "Move stock between warehouses at cost", create_transfer)
        .body(r#"{"from_warehouse_id":"<uuid>","to_warehouse_id":"<uuid>","transfer_date":"2026-11-12","lines":[{"product_id":"<uuid>","qty":200}]}"#)
        .get("/stock/adjustments", "inventory.read", "Stock adjustments", list_adjustments).query("page,per_page,status,from,to")
        .get("/stock/adjustments/:id", "inventory.read", "One adjustment with lines", get_adjustment)
        .post("/stock/adjustments", "inventory.write", "Correct stock after a count, damage or loss (may need approval)", create_adjustment)
        .body(r#"{"warehouse_id":"<uuid>","reason":"Damaged in handling","lines":[{"product_id":"<uuid>","qty_change":-5}]}"#)
        .post("/stock/opening", "finance.write", "Enter opening stock with its cost", create_opening)
        .body(r#"{"warehouse_id":"<uuid>","adjustment_date":"2026-07-01","reason":"Opening stock","lines":[{"product_id":"<uuid>","qty_change":500,"unit_cost":2850.75}]}"#)
}
