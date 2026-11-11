//! Sales: orders, deliveries (stock out + cost of goods), invoices (Mushak 6.3), customer
//! receipts with VDS/TDS withholding, and credit notes (Mushak 6.7).
use crate::{api::Api, auth::AuthUser, db::{self, created, data, pct, r2, Created, RawJson}, error::{AppError, AppResult}, modules::{accounting::{self, Acc, Post, JL}, approvals, finance, inventory::{self, Move}}, state::AppState};
use axum::{extract::{Path, Query, State}, Json};
use chrono::{Duration, NaiveDate};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::json;
use sqlx::PgConnection;
use std::collections::HashMap;
use uuid::Uuid;

/// Amounts for one sales line: (net after discount, SD, VAT, total). SD is charged on net; VAT on net + SD.
pub fn line_amounts(qty: Decimal, price: Decimal, disc_pct: Decimal, sd_rate: Decimal, vat_rate: Decimal) -> (Decimal, Decimal, Decimal, Decimal) {
    let net = r2(qty * price * (Decimal::ONE_HUNDRED - disc_pct) / Decimal::ONE_HUNDRED);
    let sd = pct(net, sd_rate);
    let vat = pct(net + sd, vat_rate);
    (net, sd, vat, net + sd + vat)
}

// ---------------- orders ----------------
#[derive(Deserialize)]
struct SoLine { product_id: Uuid, qty: Decimal, unit_price: Option<Decimal>, #[serde(default)] discount_pct: Decimal, vat_rate: Option<Decimal>, sd_rate: Option<Decimal> }
#[derive(Deserialize)]
struct SoIn { customer_id: Uuid, warehouse_id: Option<Uuid>, order_date: Option<NaiveDate>, delivery_date: Option<NaiveDate>, salesperson_id: Option<Uuid>, notes: Option<String>, branch_id: Option<Uuid>, lines: Vec<SoLine> }

const SO_DOC: &str = "SELECT (to_jsonb(o) || jsonb_build_object('customer_name', c.name, 'customer_code', c.code, 'credit_limit', c.credit_limit, 'warehouse', (SELECT name FROM warehouses w WHERE w.id = o.warehouse_id), 'salesperson', (SELECT name FROM employees e WHERE e.id = o.salesperson_id),
  'lines', (SELECT coalesce(jsonb_agg(jsonb_build_object('id', l.id, 'line_no', l.line_no, 'product_id', l.product_id, 'sku', p.sku, 'product_name', p.name, 'unit', (SELECT code FROM units u WHERE u.id = p.unit_id), 'qty', l.qty, 'unit_price', l.unit_price, 'discount_pct', l.discount_pct, 'sd_rate', l.sd_rate, 'vat_rate', l.vat_rate, 'net_amount', l.net_amount, 'sd_amount', l.sd_amount, 'vat_amount', l.vat_amount, 'line_total', l.line_total, 'delivered_qty', l.delivered_qty, 'invoiced_qty', l.invoiced_qty) ORDER BY l.line_no), '[]') FROM sales_order_lines l JOIN products p ON p.id = l.product_id WHERE l.so_id = o.id),
  'approvals', (SELECT coalesce(jsonb_agg(jsonb_build_object('level', a.level, 'approver_role', a.approver_role, 'status', a.status, 'comment', a.comment) ORDER BY a.level), '[]') FROM approval_requests a WHERE a.entity_type = 'sales_order' AND a.entity_id = o.id)))::text
  FROM sales_orders o JOIN customers c ON c.id = o.customer_id WHERE o.id = $1 AND o.company_id = $2";

async fn save_so(conn: &mut PgConnection, user: &AuthUser, b: &SoIn, existing: Option<Uuid>) -> AppResult<Uuid> {
    if b.lines.is_empty() { return Err(AppError::field("lines", "Add at least one item")); }
    let active: bool = sqlx::query_scalar("SELECT is_active FROM customers WHERE id = $1 AND company_id = $2").bind(b.customer_id).bind(user.company_id).fetch_optional(&mut *conn).await?.ok_or_else(|| AppError::field("customer_id", "Customer does not exist"))?;
    if !active { return Err(AppError::field("customer_id", "This customer is inactive")); }
    let wh = match b.warehouse_id { Some(w) => w, None => sqlx::query_scalar("SELECT id FROM warehouses WHERE company_id = $1 AND is_active ORDER BY code LIMIT 1").bind(user.company_id).fetch_optional(&mut *conn).await?.ok_or_else(|| AppError::field("warehouse_id", "Create a warehouse first"))? };
    db::ensure(conn, "warehouses", user.company_id, wh, "warehouse_id").await?;
    if let Some(s) = b.salesperson_id { db::ensure(conn, "employees", user.company_id, s, "salesperson_id").await?; }
    let enforce_min = db::setting_bool(conn, user.company_id, "sales.block_below_min_price", true).await?;
    let date = match b.order_date { Some(d) => d, None => db::today(conn, user.company_id).await? };
    let (mut sub, mut sd, mut vat, mut disc) = (Decimal::ZERO, Decimal::ZERO, Decimal::ZERO, Decimal::ZERO);
    let mut rows = vec![];
    for (i, l) in b.lines.iter().enumerate() {
        if l.qty <= Decimal::ZERO { return Err(AppError::field(&format!("lines[{i}].qty"), "Must be more than zero")); }
        if l.discount_pct < Decimal::ZERO || l.discount_pct > Decimal::ONE_HUNDRED { return Err(AppError::field(&format!("lines[{i}].discount_pct"), "Must be between 0 and 100")); }
        let (sku, sale, min, pvat, psd, kind, active): (String, Decimal, Decimal, Decimal, Decimal, String, bool) = sqlx::query_as("SELECT sku, sale_price, min_sale_price, vat_rate, sd_rate, product_type, is_active FROM products WHERE id = $1 AND company_id = $2").bind(l.product_id).bind(user.company_id).fetch_optional(&mut *conn).await?.ok_or_else(|| AppError::field(&format!("lines[{i}].product_id"), "Product does not exist"))?;
        if !active { return Err(AppError::field(&format!("lines[{i}].product_id"), format!("{sku} is inactive"))); }
        let price = l.unit_price.unwrap_or(sale);
        if price < Decimal::ZERO { return Err(AppError::field(&format!("lines[{i}].unit_price"), "Must be zero or more")); }
        let effective = price * (Decimal::ONE_HUNDRED - l.discount_pct) / Decimal::ONE_HUNDRED;
        if enforce_min && kind == "goods" && effective < min { return Err(AppError::rule(format!("{sku}: price Tk {} after discount is below the minimum Tk {}", db::r2(effective), min))); }
        let (vr, sr) = (l.vat_rate.unwrap_or(pvat), l.sd_rate.unwrap_or(psd));
        if vr < Decimal::ZERO || vr > Decimal::ONE_HUNDRED || sr < Decimal::ZERO || sr > Decimal::from(500) { return Err(AppError::field(&format!("lines[{i}].vat_rate"), "Rate is out of range")); }
        let (net, s, v, total) = line_amounts(l.qty, price, l.discount_pct, sr, vr);
        sub += net; sd += s; vat += v; disc += r2(l.qty * price) - net;
        rows.push((l, price, sr, vr, net, s, v, total));
    }
    let total = sub + sd + vat;
    let branch = user.branch_for(b.branch_id)?;
    let id = match existing {
        Some(id) => {
            sqlx::query("UPDATE sales_orders SET customer_id=$3, warehouse_id=$4, order_date=$5, delivery_date=$6, salesperson_id=$7, subtotal=$8, discount=$9, sd_amount=$10, vat_amount=$11, total=$12, notes=$13, status='draft' WHERE id=$1 AND company_id=$2")
                .bind(id).bind(user.company_id).bind(b.customer_id).bind(wh).bind(date).bind(b.delivery_date).bind(b.salesperson_id).bind(sub).bind(disc).bind(sd).bind(vat).bind(total).bind(&b.notes).execute(&mut *conn).await?;
            sqlx::query("DELETE FROM sales_order_lines WHERE so_id = $1").bind(id).execute(&mut *conn).await?;
            id
        }
        None => {
            let no = db::next_no(conn, user.company_id, "sales_order", "SO").await?;
            sqlx::query_scalar("INSERT INTO sales_orders (company_id, branch_id, so_no, customer_id, warehouse_id, order_date, delivery_date, salesperson_id, subtotal, discount, sd_amount, vat_amount, total, notes, created_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15) RETURNING id")
                .bind(user.company_id).bind(branch).bind(&no).bind(b.customer_id).bind(wh).bind(date).bind(b.delivery_date).bind(b.salesperson_id).bind(sub).bind(disc).bind(sd).bind(vat).bind(total).bind(&b.notes).bind(user.user_id).fetch_one(&mut *conn).await?
        }
    };
    for (i, (l, price, sr, vr, net, s, v, t)) in rows.iter().enumerate() {
        sqlx::query("INSERT INTO sales_order_lines (so_id, line_no, product_id, qty, unit_price, discount_pct, sd_rate, vat_rate, net_amount, sd_amount, vat_amount, line_total) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)")
            .bind(id).bind(i as i32 + 1).bind(l.product_id).bind(l.qty).bind(price).bind(l.discount_pct).bind(sr).bind(vr).bind(net).bind(s).bind(v).bind(t).execute(&mut *conn).await?;
    }
    Ok(id)
}

async fn create_so(State(st): State<AppState>, user: AuthUser, Json(b): Json<SoIn>) -> AppResult<Created> {
    let mut tx = st.db.begin().await?;
    let id = save_so(&mut tx, &user, &b, None).await?;
    db::audit(&mut tx, &user, "create", "sales_orders", id, json!({"customer_id": b.customer_id})).await?;
    let doc: String = sqlx::query_scalar(SO_DOC).bind(id).bind(user.company_id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(created(doc))
}

async fn so_head(conn: &mut PgConnection, company: Uuid, id: Uuid) -> AppResult<(String, String, Uuid, Decimal)> {
    sqlx::query_as("SELECT status, so_no, customer_id, total FROM sales_orders WHERE id = $1 AND company_id = $2 FOR UPDATE").bind(id).bind(company).fetch_optional(&mut *conn).await?.ok_or_else(|| AppError::not_found("sales order"))
}

async fn update_so(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>, Json(b): Json<SoIn>) -> AppResult<RawJson> {
    let mut tx = st.db.begin().await?;
    let (status, no, ..) = so_head(&mut tx, user.company_id, id).await?;
    if status != "draft" && status != "rejected" { return Err(AppError::rule(format!("Order {no} is {status}; only a draft can be edited"))); }
    save_so(&mut tx, &user, &b, Some(id)).await?;
    db::audit(&mut tx, &user, "update", "sales_orders", id, json!({})).await?;
    let doc: String = sqlx::query_scalar(SO_DOC).bind(id).bind(user.company_id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(data(doc))
}

/// Outstanding receivable of a customer plus confirmed orders not yet invoiced.
async fn exposure(conn: &mut PgConnection, company: Uuid, customer: Uuid, exclude_order: Option<Uuid>) -> AppResult<Decimal> {
    let ar: Decimal = sqlx::query_scalar("SELECT coalesce(sum(total - paid_amount - returned_amount), 0) FROM sales_invoices WHERE company_id = $1 AND customer_id = $2 AND status IN ('open','partial')").bind(company).bind(customer).fetch_one(&mut *conn).await?;
    let open: Decimal = sqlx::query_scalar("SELECT coalesce(sum(l.line_total * (l.qty - l.invoiced_qty) / l.qty), 0) FROM sales_order_lines l JOIN sales_orders o ON o.id = l.so_id WHERE o.company_id = $1 AND o.customer_id = $2 AND o.status IN ('confirmed','partially_delivered','delivered') AND ($3::uuid IS NULL OR o.id <> $3)").bind(company).bind(customer).bind(exclude_order).fetch_one(&mut *conn).await?;
    Ok(ar + open)
}

async fn confirm_so(conn: &mut PgConnection, user: &AuthUser, id: Uuid) -> AppResult<()> {
    sqlx::query("UPDATE sales_orders SET status = 'confirmed' WHERE id = $1 AND company_id = $2").bind(id).bind(user.company_id).execute(&mut *conn).await?;
    Ok(())
}

async fn submit_so(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> AppResult<RawJson> {
    let mut tx = st.db.begin().await?;
    let (status, no, customer, total) = so_head(&mut tx, user.company_id, id).await?;
    if status != "draft" { return Err(AppError::rule(format!("Order {no} is already {status}"))); }
    if db::setting_bool(&mut tx, user.company_id, "sales.enforce_credit_limit", true).await? {
        let (limit, name): (Decimal, String) = sqlx::query_as("SELECT credit_limit, name FROM customers WHERE id = $1").bind(customer).fetch_one(&mut *tx).await?;
        if limit > Decimal::ZERO {
            let would = exposure(&mut tx, user.company_id, customer, Some(id)).await? + total;
            if would > limit { return Err(AppError::rule(format!("{name} would owe Tk {would}, above the credit limit of Tk {limit}. Collect a payment or raise the limit first."))); }
        }
    }
    if approvals::submit(&mut tx, &user, "sales_order", id, &no, total).await? {
        sqlx::query("UPDATE sales_orders SET status = 'pending_approval' WHERE id = $1").bind(id).execute(&mut *tx).await?;
    } else { confirm_so(&mut tx, &user, id).await?; }
    db::audit(&mut tx, &user, "submit", "sales_orders", id, json!({"total": total})).await?;
    let doc: String = sqlx::query_scalar(SO_DOC).bind(id).bind(user.company_id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(data(doc))
}

pub async fn on_approval(conn: &mut PgConnection, user: &AuthUser, id: Uuid, approved: bool) -> AppResult<()> {
    if approved { confirm_so(conn, user, id).await } else {
        sqlx::query("UPDATE sales_orders SET status = 'rejected' WHERE id = $1 AND company_id = $2").bind(id).bind(user.company_id).execute(&mut *conn).await?;
        Ok(())
    }
}

async fn cancel_so(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> AppResult<RawJson> {
    let mut tx = st.db.begin().await?;
    let (status, no, ..) = so_head(&mut tx, user.company_id, id).await?;
    if !["draft", "pending_approval", "confirmed", "rejected"].contains(&status.as_str()) { return Err(AppError::rule(format!("Order {no} is {status} and cannot be cancelled"))); }
    let delivered: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM sales_order_lines WHERE so_id = $1 AND delivered_qty > 0)").bind(id).fetch_one(&mut *tx).await?;
    if delivered { return Err(AppError::rule("Goods were already delivered against this order. Close it instead.")); }
    sqlx::query("UPDATE sales_orders SET status = 'cancelled' WHERE id = $1").bind(id).execute(&mut *tx).await?;
    sqlx::query("UPDATE approval_requests SET status = 'cancelled' WHERE entity_type = 'sales_order' AND entity_id = $1 AND status IN ('pending','waiting')").bind(id).execute(&mut *tx).await?;
    db::audit(&mut tx, &user, "cancel", "sales_orders", id, json!({})).await?;
    let doc: String = sqlx::query_scalar(SO_DOC).bind(id).bind(user.company_id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(data(doc))
}
async fn close_so(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> AppResult<RawJson> {
    let mut tx = st.db.begin().await?;
    let (status, no, ..) = so_head(&mut tx, user.company_id, id).await?;
    if !["partially_delivered", "delivered"].contains(&status.as_str()) { return Err(AppError::rule(format!("Order {no} is {status} and cannot be closed"))); }
    sqlx::query("UPDATE sales_orders SET status = 'closed' WHERE id = $1").bind(id).execute(&mut *tx).await?;
    db::audit(&mut tx, &user, "close", "sales_orders", id, json!({})).await?;
    let doc: String = sqlx::query_scalar(SO_DOC).bind(id).bind(user.company_id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(data(doc))
}

async fn list_so(State(st): State<AppState>, user: AuthUser, Query(q): Query<HashMap<String, String>>) -> AppResult<RawJson> {
    let mut qb = db::list_query();
    qb.push("o.id, o.so_no, o.order_date, o.delivery_date, o.subtotal, o.vat_amount, o.total, o.status, o.customer_id, c.name AS customer_name, (SELECT coalesce(sum(l.delivered_qty) / nullif(sum(l.qty), 0) * 100, 0) FROM sales_order_lines l WHERE l.so_id = o.id) AS delivered_pct, (SELECT coalesce(sum(l.invoiced_qty) / nullif(sum(l.qty), 0) * 100, 0) FROM sales_order_lines l WHERE l.so_id = o.id) AS invoiced_pct, o.created_at FROM sales_orders o JOIN customers c ON c.id = o.customer_id WHERE o.company_id = ");
    qb.push_bind(user.company_id);
    db::filter_eq(&mut qb, &q, "status", "o.status", "");
    db::filter_eq(&mut qb, &q, "customer_id", "o.customer_id", "::uuid");
    db::filter_dates(&mut qb, &q, "o.order_date");
    db::filter_search(&mut qb, &q, &["o.so_no", "c.name"]);
    if let Some(br) = user.branch_id { qb.push(" AND o.branch_id = "); qb.push_bind(br); }
    db::page_json(&st.db, qb, "o.created_at DESC", &q).await
}
async fn get_so(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> AppResult<RawJson> {
    Ok(data(sqlx::query_scalar(SO_DOC).bind(id).bind(user.company_id).fetch_optional(&st.db).await?.ok_or_else(|| AppError::not_found("sales order"))?))
}

// ---------------- deliveries ----------------
#[derive(Deserialize)]
struct DlLine { so_line_id: Uuid, qty: Decimal }
#[derive(Deserialize)]
struct DlIn { so_id: Uuid, warehouse_id: Option<Uuid>, delivery_date: Option<NaiveDate>, vehicle_no: Option<String>, driver_name: Option<String>, notes: Option<String>, #[serde(default)] invoice: bool, lines: Vec<DlLine> }

const DL_DOC: &str = "SELECT (to_jsonb(d) || jsonb_build_object('so_no', o.so_no, 'customer_name', c.name, 'warehouse', (SELECT name FROM warehouses w WHERE w.id = d.warehouse_id), 'invoice_no', (SELECT invoice_no FROM sales_invoices i WHERE i.delivery_id = d.id LIMIT 1), 'invoice_id', (SELECT id FROM sales_invoices i WHERE i.delivery_id = d.id LIMIT 1),
  'lines', (SELECT coalesce(jsonb_agg(jsonb_build_object('so_line_id', l.so_line_id, 'product_id', l.product_id, 'sku', p.sku, 'product_name', p.name, 'qty', l.qty, 'cost_value', l.cost_value)), '[]') FROM delivery_lines l JOIN products p ON p.id = l.product_id WHERE l.delivery_id = d.id)))::text
  FROM deliveries d JOIN sales_orders o ON o.id = d.so_id JOIN customers c ON c.id = d.customer_id WHERE d.id = $1 AND d.company_id = $2";

async fn create_delivery(State(st): State<AppState>, user: AuthUser, Json(b): Json<DlIn>) -> AppResult<Created> {
    if b.lines.is_empty() { return Err(AppError::field("lines", "Enter the quantities delivered")); }
    let mut tx = st.db.begin().await?;
    let (status, so_no, customer, _) = so_head(&mut tx, user.company_id, b.so_id).await?;
    if !["confirmed", "partially_delivered"].contains(&status.as_str()) { return Err(AppError::rule(format!("Order {so_no} is {status}; only confirmed orders can be delivered"))); }
    let (so_wh, branch): (Uuid, Option<Uuid>) = sqlx::query_as("SELECT warehouse_id, branch_id FROM sales_orders WHERE id = $1").bind(b.so_id).fetch_one(&mut *tx).await?;
    let wh = b.warehouse_id.unwrap_or(so_wh);
    db::ensure(&mut tx, "warehouses", user.company_id, wh, "warehouse_id").await?;
    let date = match b.delivery_date { Some(d) => d, None => db::today(&mut tx, user.company_id).await? };
    let no = db::next_no(&mut tx, user.company_id, "delivery", "DC").await?;
    let id: Uuid = sqlx::query_scalar("INSERT INTO deliveries (company_id, branch_id, delivery_no, so_id, customer_id, warehouse_id, delivery_date, vehicle_no, driver_name, notes, created_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11) RETURNING id")
        .bind(user.company_id).bind(branch).bind(&no).bind(b.so_id).bind(customer).bind(wh).bind(date).bind(&b.vehicle_no).bind(&b.driver_name).bind(&b.notes).bind(user.user_id).fetch_one(&mut *tx).await?;
    let mut cogs = Decimal::ZERO;
    for (i, l) in b.lines.iter().enumerate() {
        if l.qty <= Decimal::ZERO { return Err(AppError::field(&format!("lines[{i}].qty"), "Must be more than zero")); }
        let (product, qty, delivered): (Uuid, Decimal, Decimal) = sqlx::query_as("SELECT product_id, qty, delivered_qty FROM sales_order_lines WHERE id = $1 AND so_id = $2 FOR UPDATE").bind(l.so_line_id).bind(b.so_id).fetch_optional(&mut *tx).await?.ok_or_else(|| AppError::field(&format!("lines[{i}].so_line_id"), "This line is not on the order"))?;
        if l.qty > qty - delivered { return Err(AppError::field(&format!("lines[{i}].qty"), format!("Only {} left to deliver on this line", (qty - delivered).normalize()))); }
        let cost = inventory::stock_move(&mut tx, Move { company: user.company_id, warehouse: wh, product, date, qty: -l.qty, unit_cost: None, value: None, move_type: "issue", source_type: "delivery", source_id: Some(id), batch: None, user: user.user_id }).await?;
        cogs += cost;
        sqlx::query("INSERT INTO delivery_lines (delivery_id, so_line_id, product_id, qty, cost_value) VALUES ($1,$2,$3,$4,$5)").bind(id).bind(l.so_line_id).bind(product).bind(l.qty).bind(cost).execute(&mut *tx).await?;
        sqlx::query("UPDATE sales_order_lines SET delivered_qty = delivered_qty + $2 WHERE id = $1").bind(l.so_line_id).bind(l.qty).execute(&mut *tx).await?;
    }
    let je = accounting::post(&mut tx, Post { company: user.company_id, branch, date, source_type: "delivery", source_id: Some(id), memo: &format!("Delivery {no} for {so_no}"), user: user.user_id }, vec![JL::dr(Acc::Key("cogs"), cogs), JL::cr(Acc::Key("inventory"), cogs)]).await?;
    sqlx::query("UPDATE deliveries SET cogs_value = $2, journal_entry_id = $3 WHERE id = $1").bind(id).bind(cogs).bind(je).execute(&mut *tx).await?;
    sqlx::query("UPDATE sales_orders o SET status = CASE WHEN NOT EXISTS (SELECT 1 FROM sales_order_lines l WHERE l.so_id = o.id AND l.delivered_qty < l.qty) THEN 'delivered' ELSE 'partially_delivered' END WHERE id = $1").bind(b.so_id).execute(&mut *tx).await?;
    if b.invoice { invoice_delivery(&mut tx, &user, id, date, None).await?; }
    db::audit(&mut tx, &user, "post", "deliveries", id, json!({"delivery_no": no, "cogs": cogs})).await?;
    let doc: String = sqlx::query_scalar(DL_DOC).bind(id).bind(user.company_id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(created(doc))
}

async fn list_deliveries(State(st): State<AppState>, user: AuthUser, Query(q): Query<HashMap<String, String>>) -> AppResult<RawJson> {
    let mut qb = db::list_query();
    qb.push("d.id, d.delivery_no, d.delivery_date, d.so_id, o.so_no, d.customer_id, c.name AS customer_name, d.cogs_value, d.invoiced, d.vehicle_no, d.created_at FROM deliveries d JOIN sales_orders o ON o.id = d.so_id JOIN customers c ON c.id = d.customer_id WHERE d.company_id = ");
    qb.push_bind(user.company_id);
    db::filter_eq(&mut qb, &q, "so_id", "d.so_id", "::uuid");
    db::filter_eq(&mut qb, &q, "customer_id", "d.customer_id", "::uuid");
    if q.get("uninvoiced").map(String::as_str) == Some("true") { qb.push(" AND NOT d.invoiced"); }
    db::filter_dates(&mut qb, &q, "d.delivery_date");
    db::page_json(&st.db, qb, "d.created_at DESC", &q).await
}
async fn get_delivery(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> AppResult<RawJson> {
    Ok(data(sqlx::query_scalar(DL_DOC).bind(id).bind(user.company_id).fetch_optional(&st.db).await?.ok_or_else(|| AppError::not_found("delivery"))?))
}

// ---------------- invoices ----------------
pub const INV_DOC: &str = "SELECT (to_jsonb(i) || jsonb_build_object('customer_name', c.name, 'customer_code', c.code, 'balance', i.total - i.paid_amount - i.returned_amount, 'so_no', (SELECT so_no FROM sales_orders o WHERE o.id = i.so_id), 'delivery_no', (SELECT delivery_no FROM deliveries d WHERE d.id = i.delivery_id),
  'overdue', (i.status IN ('open','partial') AND i.due_date < (now() AT TIME ZONE 'Asia/Dhaka')::date),
  'lines', (SELECT coalesce(jsonb_agg(jsonb_build_object('id', l.id, 'product_id', l.product_id, 'sku', p.sku, 'product_name', p.name, 'unit', (SELECT code FROM units u WHERE u.id = p.unit_id), 'qty', l.qty, 'unit_price', l.unit_price, 'discount_pct', l.discount_pct, 'sd_rate', l.sd_rate, 'vat_rate', l.vat_rate, 'net_amount', l.net_amount, 'sd_amount', l.sd_amount, 'vat_amount', l.vat_amount, 'line_total', l.line_total, 'returned_qty', l.returned_qty) ORDER BY p.sku), '[]') FROM sales_invoice_lines l JOIN products p ON p.id = l.product_id WHERE l.invoice_id = i.id),
  'receipts', (SELECT coalesce(jsonb_agg(jsonb_build_object('receipt_no', r.receipt_no, 'receipt_date', r.receipt_date, 'amount', a.amount)), '[]') FROM customer_receipt_allocations a JOIN customer_receipts r ON r.id = a.receipt_id WHERE a.invoice_id = i.id),
  'returns', (SELECT coalesce(jsonb_agg(jsonb_build_object('return_no', s.return_no, 'return_date', s.return_date, 'total', s.total)), '[]') FROM sales_returns s WHERE s.invoice_id = i.id)))::text
  FROM sales_invoices i JOIN customers c ON c.id = i.customer_id WHERE i.id = $1 AND i.company_id = $2";

/// Invoice everything delivered on a delivery challan and book revenue, VAT and SD.
async fn invoice_delivery(conn: &mut PgConnection, user: &AuthUser, delivery: Uuid, date: NaiveDate, due: Option<NaiveDate>) -> AppResult<Uuid> {
    let (so, customer, invoiced, dno, branch): (Uuid, Uuid, bool, String, Option<Uuid>) = sqlx::query_as("SELECT so_id, customer_id, invoiced, delivery_no, branch_id FROM deliveries WHERE id = $1 AND company_id = $2 FOR UPDATE").bind(delivery).bind(user.company_id).fetch_optional(&mut *conn).await?.ok_or_else(|| AppError::not_found("delivery"))?;
    if invoiced { return Err(AppError::rule(format!("Delivery {dno} is already invoiced"))); }
    let terms: i32 = sqlx::query_scalar("SELECT payment_terms_days FROM customers WHERE id = $1").bind(customer).fetch_one(&mut *conn).await?;
    let rows: Vec<(Uuid, Uuid, Uuid, Decimal, Decimal, Decimal, Decimal, Decimal, Decimal)> = sqlx::query_as("SELECT dl.so_line_id, dl.product_id, dl.id, dl.qty, dl.cost_value, sl.unit_price, sl.discount_pct, sl.sd_rate, sl.vat_rate FROM delivery_lines dl JOIN sales_order_lines sl ON sl.id = dl.so_line_id WHERE dl.delivery_id = $1 ORDER BY sl.line_no").bind(delivery).fetch_all(&mut *conn).await?;
    let inv_no = db::next_no(conn, user.company_id, "sales_invoice", "INV").await?;
    let mushak = db::next_no(conn, user.company_id, "mushak_63", "M63").await?;
    let inv: Uuid = sqlx::query_scalar("INSERT INTO sales_invoices (company_id, branch_id, invoice_no, mushak_no, so_id, delivery_id, customer_id, invoice_date, due_date, subtotal, total, created_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,0,0,$10) RETURNING id")
        .bind(user.company_id).bind(branch).bind(&inv_no).bind(&mushak).bind(so).bind(delivery).bind(customer).bind(date).bind(due.unwrap_or(date + Duration::days(terms as i64))).bind(user.user_id).fetch_one(&mut *conn).await?;
    let (mut sub, mut sd, mut vat, mut cost) = (Decimal::ZERO, Decimal::ZERO, Decimal::ZERO, Decimal::ZERO);
    for (so_line, product, _, qty, line_cost, price, disc, sdr, vr) in rows {
        let (net, s, v, total) = line_amounts(qty, price, disc, sdr, vr);
        sub += net; sd += s; vat += v; cost += line_cost;
        sqlx::query("INSERT INTO sales_invoice_lines (invoice_id, so_line_id, product_id, qty, unit_price, discount_pct, sd_rate, vat_rate, net_amount, sd_amount, vat_amount, line_total, cost_value) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)")
            .bind(inv).bind(so_line).bind(product).bind(qty).bind(price).bind(disc).bind(sdr).bind(vr).bind(net).bind(s).bind(v).bind(total).bind(line_cost).execute(&mut *conn).await?;
        sqlx::query("UPDATE sales_order_lines SET invoiced_qty = invoiced_qty + $2 WHERE id = $1").bind(so_line).bind(qty).execute(&mut *conn).await?;
    }
    let total = sub + sd + vat;
    let je = accounting::post(conn, Post { company: user.company_id, branch, date, source_type: "sales_invoice", source_id: Some(inv), memo: &format!("Sales invoice {inv_no} (Mushak 6.3 {mushak})"), user: user.user_id }, vec![
        JL::dr(Acc::Key("ar"), total).party("customer", customer).tag("invoice", inv),
        JL::cr(Acc::Key("sales"), sub), JL::cr(Acc::Key("vat_output"), vat), JL::cr(Acc::Key("sd_payable"), sd),
    ]).await?;
    sqlx::query("UPDATE sales_invoices SET subtotal = $2, sd_amount = $3, vat_amount = $4, total = $5, journal_entry_id = $6 WHERE id = $1").bind(inv).bind(sub).bind(sd).bind(vat).bind(total).bind(je).execute(&mut *conn).await?;
    sqlx::query("UPDATE deliveries SET invoiced = true WHERE id = $1").bind(delivery).execute(&mut *conn).await?;
    let _ = cost;
    db::audit(conn, user, "post", "sales_invoices", inv, json!({"invoice_no": inv_no, "mushak_no": mushak, "total": total})).await?;
    Ok(inv)
}

#[derive(Deserialize)]
struct InvIn { delivery_id: Uuid, invoice_date: Option<NaiveDate>, due_date: Option<NaiveDate> }
async fn create_invoice(State(st): State<AppState>, user: AuthUser, Json(b): Json<InvIn>) -> AppResult<Created> {
    let mut tx = st.db.begin().await?;
    let date = match b.invoice_date { Some(d) => d, None => db::today(&mut tx, user.company_id).await? };
    let id = invoice_delivery(&mut tx, &user, b.delivery_id, date, b.due_date).await?;
    let doc: String = sqlx::query_scalar(INV_DOC).bind(id).bind(user.company_id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(created(doc))
}

async fn list_invoices(State(st): State<AppState>, user: AuthUser, Query(q): Query<HashMap<String, String>>) -> AppResult<RawJson> {
    let mut qb = db::list_query();
    qb.push("i.id, i.invoice_no, i.mushak_no, i.invoice_date, i.due_date, i.subtotal, i.sd_amount, i.vat_amount, i.total, i.paid_amount, i.returned_amount, i.total - i.paid_amount - i.returned_amount AS balance, i.status, i.customer_id, c.name AS customer_name, (i.status IN ('open','partial') AND i.due_date < (now() AT TIME ZONE 'Asia/Dhaka')::date) AS overdue FROM sales_invoices i JOIN customers c ON c.id = i.customer_id WHERE i.company_id = ");
    qb.push_bind(user.company_id);
    db::filter_eq(&mut qb, &q, "status", "i.status", "");
    db::filter_eq(&mut qb, &q, "customer_id", "i.customer_id", "::uuid");
    if q.get("unpaid").map(String::as_str) == Some("true") { qb.push(" AND i.status IN ('open','partial')"); }
    if q.get("overdue").map(String::as_str) == Some("true") { qb.push(" AND i.status IN ('open','partial') AND i.due_date < (now() AT TIME ZONE 'Asia/Dhaka')::date"); }
    db::filter_dates(&mut qb, &q, "i.invoice_date");
    db::filter_search(&mut qb, &q, &["i.invoice_no", "i.mushak_no", "c.name"]);
    if let Some(br) = user.branch_id { qb.push(" AND i.branch_id = "); qb.push_bind(br); }
    db::page_json(&st.db, qb, "invoice_date DESC, invoice_no DESC", &q).await
}
async fn get_invoice(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> AppResult<RawJson> {
    Ok(data(sqlx::query_scalar(INV_DOC).bind(id).bind(user.company_id).fetch_optional(&st.db).await?.ok_or_else(|| AppError::not_found("invoice"))?))
}

/// Mushak 6.3 print data: seller, buyer, items and totals as the NBR form expects them.
async fn mushak_63(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> AppResult<RawJson> {
    let s: String = sqlx::query_scalar("SELECT jsonb_build_object('form', 'Mushak 6.3', 'title', 'Tax Invoice / কর চালানপত্র', 'serial_no', i.mushak_no, 'invoice_no', i.invoice_no, 'issue_date', i.invoice_date, 'issue_time', to_char(i.created_at AT TIME ZONE 'Asia/Dhaka', 'HH24:MI'),
          'seller', jsonb_build_object('name', co.name, 'name_bn', co.name_bn, 'bin', co.bin, 'address', co.address),
          'buyer', jsonb_build_object('name', c.name, 'bin', c.bin, 'address', c.address, 'phone', c.phone),
          'delivery_address', c.address, 'vehicle_no', (SELECT vehicle_no FROM deliveries d WHERE d.id = i.delivery_id),
          'items', (SELECT coalesce(jsonb_agg(jsonb_build_object('sl', r.rn, 'description', r.name, 'unit', r.unit, 'qty', r.qty, 'unit_price', r.unit_price, 'value_excl_tax', r.net_amount, 'sd_rate', r.sd_rate, 'sd', r.sd_amount, 'vat_rate', r.vat_rate, 'vat', r.vat_amount, 'total', r.line_total) ORDER BY r.rn), '[]')
                    FROM (SELECT row_number() OVER (ORDER BY p.sku) AS rn, p.name, (SELECT code FROM units u WHERE u.id = p.unit_id) AS unit, l.* FROM sales_invoice_lines l JOIN products p ON p.id = l.product_id WHERE l.invoice_id = i.id) r),
          'totals', jsonb_build_object('value_excl_tax', i.subtotal, 'sd', i.sd_amount, 'vat', i.vat_amount, 'grand_total', i.total))::text
        FROM sales_invoices i JOIN customers c ON c.id = i.customer_id JOIN companies co ON co.id = i.company_id WHERE i.id = $1 AND i.company_id = $2").bind(id).bind(user.company_id).fetch_optional(&st.db).await?.ok_or_else(|| AppError::not_found("invoice"))?;
    Ok(data(s))
}

// ---------------- customer receipts ----------------
#[derive(Deserialize)]
struct RAlloc { invoice_id: Uuid, amount: Decimal }
#[derive(Deserialize)]
struct RcIn { customer_id: Uuid, bank_account_id: Uuid, receipt_date: Option<NaiveDate>, gross_amount: Decimal, #[serde(default)] vds_amount: Decimal, #[serde(default)] tds_amount: Decimal, method: Option<String>, reference: Option<String>, #[serde(default)] allocations: Vec<RAlloc> }

const RC_DOC: &str = "SELECT (to_jsonb(r) || jsonb_build_object('customer_name', c.name, 'bank_account', (SELECT name FROM bank_accounts b WHERE b.id = r.bank_account_id),
  'allocations', (SELECT coalesce(jsonb_agg(jsonb_build_object('invoice_id', a.invoice_id, 'invoice_no', i.invoice_no, 'amount', a.amount)), '[]') FROM customer_receipt_allocations a JOIN sales_invoices i ON i.id = a.invoice_id WHERE a.receipt_id = r.id)))::text
  FROM customer_receipts r JOIN customers c ON c.id = r.customer_id WHERE r.id = $1 AND r.company_id = $2";

async fn allocate_to(conn: &mut PgConnection, company: Uuid, receipt: Uuid, customer: Uuid, allocs: &[RAlloc]) -> AppResult<Decimal> {
    let mut sum = Decimal::ZERO;
    for (i, a) in allocs.iter().enumerate() {
        let amount = r2(a.amount);
        if amount <= Decimal::ZERO { return Err(AppError::field(&format!("allocations[{i}].amount"), "Must be more than zero")); }
        let (cust, total, paid, ret, status): (Uuid, Decimal, Decimal, Decimal, String) = sqlx::query_as("SELECT customer_id, total, paid_amount, returned_amount, status FROM sales_invoices WHERE id = $1 AND company_id = $2 FOR UPDATE").bind(a.invoice_id).bind(company).fetch_optional(&mut *conn).await?.ok_or_else(|| AppError::field(&format!("allocations[{i}].invoice_id"), "Invoice does not exist"))?;
        if cust != customer { return Err(AppError::field(&format!("allocations[{i}].invoice_id"), "This invoice belongs to another customer")); }
        if status == "cancelled" || amount > total - paid - ret { return Err(AppError::field(&format!("allocations[{i}].amount"), format!("Only Tk {} is outstanding on this invoice", total - paid - ret))); }
        sqlx::query("INSERT INTO customer_receipt_allocations (receipt_id, invoice_id, amount) VALUES ($1,$2,$3) ON CONFLICT (receipt_id, invoice_id) DO UPDATE SET amount = customer_receipt_allocations.amount + EXCLUDED.amount").bind(receipt).bind(a.invoice_id).bind(amount).execute(&mut *conn).await?;
        sqlx::query("UPDATE sales_invoices SET paid_amount = paid_amount + $2, status = CASE WHEN paid_amount + $2 + returned_amount >= total THEN 'paid' ELSE 'partial' END WHERE id = $1").bind(a.invoice_id).bind(amount).execute(&mut *conn).await?;
        sum += amount;
    }
    Ok(sum)
}

async fn create_receipt(State(st): State<AppState>, user: AuthUser, Json(b): Json<RcIn>) -> AppResult<Created> {
    if b.gross_amount <= Decimal::ZERO { return Err(AppError::field("gross_amount", "Must be more than zero")); }
    if b.vds_amount < Decimal::ZERO || b.tds_amount < Decimal::ZERO || b.vds_amount + b.tds_amount >= b.gross_amount { return Err(AppError::field("vds_amount", "VDS and TDS must be zero or more and less than the amount received")); }
    let method = b.method.clone().unwrap_or_else(|| "bank".into());
    if !["bank", "cash", "cheque", "bkash", "other"].contains(&method.as_str()) { return Err(AppError::field("method", "Unknown payment method")); }
    let mut tx = st.db.begin().await?;
    db::ensure(&mut tx, "customers", user.company_id, b.customer_id, "customer_id").await?;
    let bk = finance::bank(&mut tx, user.company_id, b.bank_account_id, "bank_account_id").await?;
    if bk.currency != "BDT" { return Err(AppError::field("bank_account_id", "Receive customer payments into a BDT account")); }
    let date = match b.receipt_date { Some(d) => d, None => db::today(&mut tx, user.company_id).await? };
    let gross = r2(b.gross_amount); let vds = r2(b.vds_amount); let tds = r2(b.tds_amount); let net = gross - vds - tds;
    let mut seen = std::collections::HashSet::new();
    if !b.allocations.iter().all(|a| seen.insert(a.invoice_id)) { return Err(AppError::field("allocations", "Each invoice may appear only once")); }
    let alloc_sum: Decimal = b.allocations.iter().map(|a| r2(a.amount)).sum();
    if alloc_sum > gross { return Err(AppError::field("allocations", "Allocations are more than the amount received")); }
    let no = db::next_no(&mut tx, user.company_id, "customer_receipt", "RCT").await?;
    let id: Uuid = sqlx::query_scalar("INSERT INTO customer_receipts (company_id, receipt_no, customer_id, bank_account_id, receipt_date, gross_amount, vds_amount, tds_amount, net_amount, method, reference, unallocated, created_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13) RETURNING id")
        .bind(user.company_id).bind(&no).bind(b.customer_id).bind(b.bank_account_id).bind(date).bind(gross).bind(vds).bind(tds).bind(net).bind(&method).bind(&b.reference).bind(gross - alloc_sum).bind(user.user_id).fetch_one(&mut *tx).await?;
    allocate_to(&mut tx, user.company_id, id, b.customer_id, &b.allocations).await?;
    let je = accounting::post(&mut tx, Post { company: user.company_id, branch: None, date, source_type: "customer_receipt", source_id: Some(id), memo: &format!("Receipt {no}"), user: user.user_id }, vec![
        JL::dr(Acc::Id(bk.gl), net), JL::dr(Acc::Key("vds_receivable"), vds).party("customer", b.customer_id), JL::dr(Acc::Key("ait_paid"), tds).party("customer", b.customer_id).memo("Tax withheld by customer"),
        JL::cr(Acc::Key("ar"), gross).party("customer", b.customer_id),
    ]).await?;
    sqlx::query("UPDATE customer_receipts SET journal_entry_id = $2 WHERE id = $1").bind(id).bind(je).execute(&mut *tx).await?;
    db::audit(&mut tx, &user, "post", "customer_receipts", id, json!({"receipt_no": no, "gross": gross, "vds": vds, "tds": tds})).await?;
    let doc: String = sqlx::query_scalar(RC_DOC).bind(id).bind(user.company_id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(created(doc))
}

#[derive(Deserialize)]
struct AllocIn { allocations: Vec<RAlloc> }
/// Apply the unallocated part of an earlier receipt (advance) to invoices.
async fn allocate_receipt(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>, Json(b): Json<AllocIn>) -> AppResult<RawJson> {
    if b.allocations.is_empty() { return Err(AppError::field("allocations", "Choose at least one invoice")); }
    let mut tx = st.db.begin().await?;
    let (customer, un): (Uuid, Decimal) = sqlx::query_as("SELECT customer_id, unallocated FROM customer_receipts WHERE id = $1 AND company_id = $2 FOR UPDATE").bind(id).bind(user.company_id).fetch_optional(&mut *tx).await?.ok_or_else(|| AppError::not_found("receipt"))?;
    let sum = allocate_to(&mut tx, user.company_id, id, customer, &b.allocations).await?;
    if sum > un { return Err(AppError::field("allocations", format!("Only Tk {un} of this receipt is unallocated"))); }
    sqlx::query("UPDATE customer_receipts SET unallocated = unallocated - $2 WHERE id = $1").bind(id).bind(sum).execute(&mut *tx).await?;
    db::audit(&mut tx, &user, "allocate", "customer_receipts", id, json!({"amount": sum})).await?;
    let doc: String = sqlx::query_scalar(RC_DOC).bind(id).bind(user.company_id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(data(doc))
}

async fn list_receipts(State(st): State<AppState>, user: AuthUser, Query(q): Query<HashMap<String, String>>) -> AppResult<RawJson> {
    let mut qb = db::list_query();
    qb.push("r.*, c.name AS customer_name, (SELECT name FROM bank_accounts b WHERE b.id = r.bank_account_id) AS bank_account FROM customer_receipts r JOIN customers c ON c.id = r.customer_id WHERE r.company_id = ");
    qb.push_bind(user.company_id);
    db::filter_eq(&mut qb, &q, "customer_id", "r.customer_id", "::uuid");
    if q.get("unallocated").map(String::as_str) == Some("true") { qb.push(" AND r.unallocated > 0"); }
    db::filter_dates(&mut qb, &q, "r.receipt_date");
    db::page_json(&st.db, qb, "r.created_at DESC", &q).await
}
async fn get_receipt(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> AppResult<RawJson> {
    Ok(data(sqlx::query_scalar(RC_DOC).bind(id).bind(user.company_id).fetch_optional(&st.db).await?.ok_or_else(|| AppError::not_found("receipt"))?))
}

// ---------------- returns (credit notes) ----------------
#[derive(Deserialize)]
struct RtLine { invoice_line_id: Uuid, qty: Decimal }
#[derive(Deserialize)]
struct RtIn { invoice_id: Uuid, warehouse_id: Option<Uuid>, return_date: Option<NaiveDate>, reason: String, lines: Vec<RtLine> }

const RT_DOC: &str = "SELECT (to_jsonb(s) || jsonb_build_object('invoice_no', i.invoice_no, 'customer_name', c.name,
  'lines', (SELECT coalesce(jsonb_agg(jsonb_build_object('product_id', l.product_id, 'sku', p.sku, 'product_name', p.name, 'qty', l.qty, 'net_amount', l.net_amount, 'sd_amount', l.sd_amount, 'vat_amount', l.vat_amount)), '[]') FROM sales_return_lines l JOIN products p ON p.id = l.product_id WHERE l.return_id = s.id)))::text
  FROM sales_returns s JOIN sales_invoices i ON i.id = s.invoice_id JOIN customers c ON c.id = s.customer_id WHERE s.id = $1 AND s.company_id = $2";

async fn create_return(State(st): State<AppState>, user: AuthUser, Json(b): Json<RtIn>) -> AppResult<Created> {
    if b.lines.is_empty() { return Err(AppError::field("lines", "Choose the items returned")); }
    if b.reason.trim().is_empty() { return Err(AppError::field("reason", "Please give a reason")); }
    let mut tx = st.db.begin().await?;
    let (customer, inv_no, status, total, paid, ret, so, delivery): (Uuid, String, String, Decimal, Decimal, Decimal, Option<Uuid>, Option<Uuid>) = sqlx::query_as("SELECT customer_id, invoice_no, status, total, paid_amount, returned_amount, so_id, delivery_id FROM sales_invoices WHERE id = $1 AND company_id = $2 FOR UPDATE").bind(b.invoice_id).bind(user.company_id).fetch_optional(&mut *tx).await?.ok_or_else(|| AppError::field("invoice_id", "Invoice does not exist"))?;
    if status == "cancelled" { return Err(AppError::rule(format!("Invoice {inv_no} is cancelled"))); }
    let wh = match b.warehouse_id { Some(w) => w, None => match delivery { Some(d) => sqlx::query_scalar("SELECT warehouse_id FROM deliveries WHERE id = $1").bind(d).fetch_one(&mut *tx).await?, None => return Err(AppError::field("warehouse_id", "Choose the warehouse receiving the goods")) } };
    db::ensure(&mut tx, "warehouses", user.company_id, wh, "warehouse_id").await?;
    let date = match b.return_date { Some(d) => d, None => db::today(&mut tx, user.company_id).await? };
    let no = db::next_no(&mut tx, user.company_id, "sales_return", "CN").await?;
    let id: Uuid = sqlx::query_scalar("INSERT INTO sales_returns (company_id, return_no, invoice_id, customer_id, warehouse_id, return_date, reason, subtotal, total, created_by) VALUES ($1,$2,$3,$4,$5,$6,$7,0,0,$8) RETURNING id").bind(user.company_id).bind(&no).bind(b.invoice_id).bind(customer).bind(wh).bind(date).bind(b.reason.trim()).bind(user.user_id).fetch_one(&mut *tx).await?;
    let (mut sub, mut sd, mut vat, mut cost) = (Decimal::ZERO, Decimal::ZERO, Decimal::ZERO, Decimal::ZERO);
    for (i, l) in b.lines.iter().enumerate() {
        if l.qty <= Decimal::ZERO { return Err(AppError::field(&format!("lines[{i}].qty"), "Must be more than zero")); }
        let (product, qty, returned, net, s, v, c): (Uuid, Decimal, Decimal, Decimal, Decimal, Decimal, Decimal) = sqlx::query_as("SELECT product_id, qty, returned_qty, net_amount, sd_amount, vat_amount, cost_value FROM sales_invoice_lines WHERE id = $1 AND invoice_id = $2 FOR UPDATE").bind(l.invoice_line_id).bind(b.invoice_id).fetch_optional(&mut *tx).await?.ok_or_else(|| AppError::field(&format!("lines[{i}].invoice_line_id"), "This line is not on the invoice"))?;
        if l.qty > qty - returned { return Err(AppError::field(&format!("lines[{i}].qty"), format!("Only {} can still be returned on this line", (qty - returned).normalize()))); }
        let (rn, rs, rv, rc) = (r2(net * l.qty / qty), r2(s * l.qty / qty), r2(v * l.qty / qty), r2(c * l.qty / qty));
        inventory::stock_move(&mut tx, Move { company: user.company_id, warehouse: wh, product, date, qty: l.qty, unit_cost: None, value: Some(rc), move_type: "sales_return", source_type: "sales_return", source_id: Some(id), batch: None, user: user.user_id }).await?;
        sqlx::query("INSERT INTO sales_return_lines (return_id, invoice_line_id, product_id, qty, net_amount, sd_amount, vat_amount, cost_value) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)").bind(id).bind(l.invoice_line_id).bind(product).bind(l.qty).bind(rn).bind(rs).bind(rv).bind(rc).execute(&mut *tx).await?;
        sqlx::query("UPDATE sales_invoice_lines SET returned_qty = returned_qty + $2 WHERE id = $1").bind(l.invoice_line_id).bind(l.qty).execute(&mut *tx).await?;
        sub += rn; sd += rs; vat += rv; cost += rc;
    }
    let credit = sub + sd + vat;
    if credit > total - paid - ret { return Err(AppError::rule(format!("Credit note of Tk {credit} is more than the unpaid balance of Tk {} on {inv_no}. Refund the customer from the bank for amounts already paid.", total - paid - ret))); }
    let je = accounting::post(&mut tx, Post { company: user.company_id, branch: None, date, source_type: "sales_return", source_id: Some(id), memo: &format!("Credit note {no} against {inv_no} (Mushak 6.7)"), user: user.user_id }, vec![
        JL::dr(Acc::Key("sales_returns"), sub), JL::dr(Acc::Key("vat_output"), vat), JL::dr(Acc::Key("sd_payable"), sd), JL::cr(Acc::Key("ar"), credit).party("customer", customer).tag("invoice", b.invoice_id),
        JL::dr(Acc::Key("inventory"), cost), JL::cr(Acc::Key("cogs"), cost),
    ]).await?;
    sqlx::query("UPDATE sales_returns SET subtotal = $2, sd_amount = $3, vat_amount = $4, total = $5, cost_value = $6, journal_entry_id = $7 WHERE id = $1").bind(id).bind(sub).bind(sd).bind(vat).bind(credit).bind(cost).bind(je).execute(&mut *tx).await?;
    sqlx::query("UPDATE sales_invoices SET returned_amount = returned_amount + $2, status = CASE WHEN paid_amount + returned_amount + $2 >= total THEN 'paid' WHEN paid_amount + returned_amount + $2 > 0 THEN 'partial' ELSE status END WHERE id = $1").bind(b.invoice_id).bind(credit).execute(&mut *tx).await?;
    let _ = so;
    db::audit(&mut tx, &user, "post", "sales_returns", id, json!({"return_no": no, "total": credit})).await?;
    let doc: String = sqlx::query_scalar(RT_DOC).bind(id).bind(user.company_id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(created(doc))
}

async fn list_returns(State(st): State<AppState>, user: AuthUser, Query(q): Query<HashMap<String, String>>) -> AppResult<RawJson> {
    let mut qb = db::list_query();
    qb.push("s.id, s.return_no, s.return_date, s.reason, s.subtotal, s.sd_amount, s.vat_amount, s.total, s.invoice_id, i.invoice_no, c.name AS customer_name FROM sales_returns s JOIN sales_invoices i ON i.id = s.invoice_id JOIN customers c ON c.id = s.customer_id WHERE s.company_id = ");
    qb.push_bind(user.company_id);
    db::filter_eq(&mut qb, &q, "invoice_id", "s.invoice_id", "::uuid");
    db::filter_dates(&mut qb, &q, "s.return_date");
    db::page_json(&st.db, qb, "s.created_at DESC", &q).await
}
async fn get_return(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> AppResult<RawJson> {
    Ok(data(sqlx::query_scalar(RT_DOC).bind(id).bind(user.company_id).fetch_optional(&st.db).await?.ok_or_else(|| AppError::not_found("credit note"))?))
}

/// Customer statement: invoices, credit notes and receipts in date order with a running balance.
async fn statement(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>, Query(q): Query<HashMap<String, String>>) -> AppResult<RawJson> {
    db::ensure(&mut *st.db.acquire().await?, "customers", user.company_id, id, "customer").await.map_err(|_| AppError::not_found("customer"))?;
    let mut qb = db::list_query();
    qb.push("e.entry_date, e.entry_no, e.source_type, e.memo, l.debit, l.credit, sum(l.debit - l.credit) OVER (ORDER BY e.entry_date, e.entry_no, l.line_no) AS balance FROM journal_lines l JOIN journal_entries e ON e.id = l.entry_id JOIN accounts a ON a.id = l.account_id AND a.system_key = 'ar' WHERE l.party_type = 'customer' AND l.party_id = ");
    qb.push_bind(id);
    db::filter_dates(&mut qb, &q, "e.entry_date");
    db::page_json(&st.db, qb, "entry_date DESC, entry_no DESC", &q).await
}

pub fn api() -> Api {
    Api::new("Sales")
        .get("/sales-orders", "sales.read", "Sales orders", list_so).query("page,per_page,q,status,customer_id,from,to")
        .post("/sales-orders", "sales.write", "Create a draft sales order (prices default from the product)", create_so)
        .body(r#"{"customer_id":"<uuid>","warehouse_id":"<uuid>","delivery_date":"2026-12-05","lines":[{"product_id":"<uuid>","qty":120,"unit_price":4200,"discount_pct":2}]}"#)
        .get("/sales-orders/:id", "sales.read", "One order with lines, delivery and invoice progress", get_so)
        .put("/sales-orders/:id", "sales.write", "Replace a draft order", update_so)
        .post("/sales-orders/:id/submit", "sales.write", "Confirm the order (checks credit limit; may need approval)", submit_so)
        .post("/sales-orders/:id/cancel", "sales.write", "Cancel an order with no deliveries", cancel_so)
        .post("/sales-orders/:id/close", "sales.write", "Close a delivered or part-delivered order", close_so)
        .get("/deliveries", "sales.read", "Delivery challans", list_deliveries).query("page,per_page,so_id,customer_id,uninvoiced,from,to")
        .get("/deliveries/:id", "sales.read", "One delivery", get_delivery)
        .post("/deliveries", "inventory.write", "Deliver goods: stock leaves, cost of goods is booked. Set invoice=true to invoice at once", create_delivery)
        .body(r#"{"so_id":"<uuid>","vehicle_no":"Dhaka Metro-Ta 11-2345","invoice":true,"lines":[{"so_line_id":"<uuid>","qty":120}]}"#)
        .get("/sales-invoices", "sales.read", "Sales invoices", list_invoices).query("page,per_page,q,status,customer_id,unpaid,overdue,from,to")
        .get("/sales-invoices/:id", "sales.read", "One invoice with lines, receipts and returns", get_invoice)
        .get("/sales-invoices/:id/mushak-6-3", "sales.read", "Mushak 6.3 print data", mushak_63)
        .post("/sales-invoices", "sales.invoice", "Invoice a delivery (Mushak 6.3 number issued automatically)", create_invoice).body(r#"{"delivery_id":"<uuid>","invoice_date":"2026-12-02"}"#)
        .get("/customer-receipts", "sales.read", "Customer receipts", list_receipts).query("page,per_page,customer_id,unallocated,from,to")
        .get("/customer-receipts/:id", "sales.read", "One receipt with allocations", get_receipt)
        .post("/customer-receipts", "sales.invoice", "Record money received; VDS and TDS withheld by the customer are booked as assets", create_receipt)
        .body(r#"{"customer_id":"<uuid>","bank_account_id":"<uuid>","gross_amount":500000,"vds_amount":30000,"tds_amount":5000,"method":"bank","reference":"EFT 88231","allocations":[{"invoice_id":"<uuid>","amount":500000}]}"#)
        .post("/customer-receipts/:id/allocate", "sales.invoice", "Apply an advance or unallocated receipt to invoices", allocate_receipt).body(r#"{"allocations":[{"invoice_id":"<uuid>","amount":120000}]}"#)
        .get("/sales-returns", "sales.read", "Credit notes (Mushak 6.7)", list_returns).query("page,per_page,invoice_id,from,to")
        .get("/sales-returns/:id", "sales.read", "One credit note", get_return)
        .post("/sales-returns", "sales.write", "Return goods against an invoice: credit note, stock back at original cost", create_return)
        .body(r#"{"invoice_id":"<uuid>","reason":"Damaged in transit","lines":[{"invoice_line_id":"<uuid>","qty":5}]}"#)
        .get("/customers/:id/statement", "sales.read", "Customer ledger statement with running balance", statement).query("page,per_page,from,to")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    fn d(s: &str) -> Decimal { Decimal::from_str(s).unwrap() }

    #[test]
    fn line_amounts_apply_discount_sd_then_vat() {
        // 10 x 1,000 with 10% discount => 9,000; SD 20% => 1,800; VAT 15% of 10,800 => 1,620
        let (net, sd, vat, total) = line_amounts(d("10"), d("1000"), d("10"), d("20"), d("15"));
        assert_eq!((net, sd, vat, total), (d("9000.00"), d("1800.00"), d("1620.00"), d("12420.00")));
    }

    #[test]
    fn standard_vat_only() {
        let (net, sd, vat, total) = line_amounts(d("3"), d("4200.50"), d("0"), d("0"), d("15"));
        assert_eq!(net, d("12601.50"));
        assert_eq!(sd, d("0"));
        assert_eq!(vat, d("1890.23")); // 1890.225 rounds half away from zero
        assert_eq!(total, d("14491.73"));
    }
}
