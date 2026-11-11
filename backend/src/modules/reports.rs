//! Financial statements, ageing, dashboards, replenishment suggestions and global search.
//! Everything is computed from the ledger and sub-ledgers; no figure is stored twice.
use crate::{api::{Api, AUTH}, auth::AuthUser, db::{self, r2, RawJson}, error::{AppError, AppResult}, state::AppState};
use axum::{extract::{Query, State}, Json};
use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;
use serde_json::{json, Value};
use std::collections::HashMap;

async fn as_of(st: &AppState, user: &AuthUser, q: &HashMap<String, String>, key: &str) -> AppResult<NaiveDate> {
    match db::parse_date(q, key)? { Some(d) => Ok(d), None => db::today(&mut *st.db.acquire().await?, user.company_id).await }
}

/// Trial balance as of a date: one row per account with balance.
async fn trial_balance(State(st): State<AppState>, user: AuthUser, Query(q): Query<HashMap<String, String>>) -> AppResult<Json<Value>> {
    let date = as_of(&st, &user, &q, "as_of").await?;
    let rows: Value = sqlx::query_scalar("SELECT coalesce(jsonb_agg(to_jsonb(x) ORDER BY x.code), '[]') FROM (SELECT a.id, a.code, a.name, a.name_bn, a.account_type, coalesce(sum(l.debit), 0) AS debit, coalesce(sum(l.credit), 0) AS credit, greatest(coalesce(sum(l.debit - l.credit), 0), 0) AS balance_dr, greatest(coalesce(sum(l.credit - l.debit), 0), 0) AS balance_cr FROM accounts a LEFT JOIN journal_lines l ON l.account_id = a.id AND l.entry_id IN (SELECT id FROM journal_entries WHERE company_id = $1 AND entry_date <= $2) WHERE a.company_id = $1 AND NOT a.is_group GROUP BY a.id HAVING coalesce(sum(l.debit), 0) <> 0 OR coalesce(sum(l.credit), 0) <> 0) x").bind(user.company_id).bind(date).fetch_one(&st.db).await?;
    let (dr, cr): (Decimal, Decimal) = rows.as_array().map(|a| a.iter().fold((Decimal::ZERO, Decimal::ZERO), |(d, c), r| (d + dec(&r["balance_dr"]), c + dec(&r["balance_cr"])))).unwrap_or_default();
    Ok(Json(json!({"data": {"as_of": date, "accounts": rows, "total_debit": dr, "total_credit": cr, "balanced": dr == cr}})))
}
fn dec(v: &Value) -> Decimal { v.to_string().trim_matches('"').parse().unwrap_or_default() }

/// Income statement for a period.
async fn income_statement(State(st): State<AppState>, user: AuthUser, Query(q): Query<HashMap<String, String>>) -> AppResult<Json<Value>> {
    let to = as_of(&st, &user, &q, "to").await?;
    let from = db::parse_date(&q, "from")?.unwrap_or_else(|| NaiveDate::from_ymd_opt(to.year(), to.month(), 1).unwrap());
    let rows: Vec<(String, String, String, Decimal)> = sqlx::query_as("SELECT a.code, a.name, a.account_type, coalesce(sum(CASE WHEN a.account_type = 'income' THEN l.credit - l.debit ELSE l.debit - l.credit END), 0) FROM accounts a JOIN journal_lines l ON l.account_id = a.id JOIN journal_entries e ON e.id = l.entry_id AND e.entry_date BETWEEN $2 AND $3 WHERE a.company_id = $1 AND a.account_type IN ('income','expense') GROUP BY a.code, a.name, a.account_type HAVING coalesce(sum(l.debit + l.credit), 0) <> 0 ORDER BY a.code").bind(user.company_id).bind(from).bind(to).fetch_all(&st.db).await?;
    let income: Decimal = rows.iter().filter(|r| r.2 == "income").map(|r| r.3).sum();
    let cogs: Decimal = rows.iter().filter(|r| r.0 == "5010").map(|r| r.3).sum();
    let expense: Decimal = rows.iter().filter(|r| r.2 == "expense").map(|r| r.3).sum();
    let line = |t: &str| rows.iter().filter(|r| r.2 == t).map(|r| json!({"code": r.0, "name": r.1, "amount": r.3})).collect::<Vec<_>>();
    Ok(Json(json!({"data": {"from": from, "to": to, "income": line("income"), "expenses": line("expense"), "total_income": income, "cost_of_goods_sold": cogs, "gross_profit": income - cogs, "gross_margin_pct": if income.is_zero() { Value::Null } else { json!(db::r2((income - cogs) / income * Decimal::ONE_HUNDRED)) }, "total_expenses": expense, "net_profit": income - expense}})))
}

/// Balance sheet as of a date. Current-year profit is shown inside equity.
async fn balance_sheet(State(st): State<AppState>, user: AuthUser, Query(q): Query<HashMap<String, String>>) -> AppResult<Json<Value>> {
    let date = as_of(&st, &user, &q, "as_of").await?;
    let rows: Vec<(String, String, String, Decimal)> = sqlx::query_as("SELECT a.code, a.name, a.account_type, coalesce(sum(l.debit - l.credit), 0) FROM accounts a JOIN journal_lines l ON l.account_id = a.id JOIN journal_entries e ON e.id = l.entry_id AND e.entry_date <= $2 WHERE a.company_id = $1 GROUP BY a.code, a.name, a.account_type HAVING coalesce(sum(l.debit + l.credit), 0) <> 0 ORDER BY a.code").bind(user.company_id).bind(date).fetch_all(&st.db).await?;
    let sum = |t: &str| -> Decimal { rows.iter().filter(|r| r.2 == t).map(|r| r.3).sum() };
    let (assets, liabilities, equity) = (sum("asset"), -sum("liability"), -sum("equity"));
    let profit = -(sum("income") + sum("expense"));
    let part = |t: &str, sign: i64| rows.iter().filter(|r| r.2 == t).map(|r| json!({"code": r.0, "name": r.1, "amount": r.3 * Decimal::from(sign)})).collect::<Vec<_>>();
    Ok(Json(json!({"data": {"as_of": date, "assets": part("asset", 1), "liabilities": part("liability", -1), "equity": part("equity", -1), "retained_profit": profit, "total_assets": assets, "total_liabilities": liabilities, "total_equity": equity + profit, "balanced": assets == liabilities + equity + profit}})))
}

async fn general_ledger(State(st): State<AppState>, user: AuthUser, Query(q): Query<HashMap<String, String>>) -> AppResult<RawJson> {
    let acc = q.get("account_id").ok_or_else(|| AppError::BadRequest("account_id is required".into()))?.clone();
    let mut qb = db::list_query();
    qb.push("e.entry_date, e.entry_no, e.source_type, e.source_id, coalesce(l.memo, e.memo) AS memo, l.debit, l.credit, l.party_type, l.party_id, sum(l.debit - l.credit) OVER (ORDER BY e.entry_date, e.entry_no, l.line_no) AS balance FROM journal_lines l JOIN journal_entries e ON e.id = l.entry_id WHERE l.company_id = ");
    qb.push_bind(user.company_id);
    qb.push(" AND l.account_id = "); qb.push_bind(acc); qb.push("::uuid");
    db::filter_dates(&mut qb, &q, "e.entry_date");
    db::page_json(&st.db, qb, "entry_date DESC, entry_no DESC", &q).await
}

/// Ageing buckets by days past due.
async fn ageing(st: &AppState, company: uuid::Uuid, receivable: bool, date: NaiveDate) -> AppResult<Value> {
    let (table, party_t, party_col, bal) = if receivable { ("sales_invoices", "customers", "customer_id", "(t.total - t.paid_amount - t.returned_amount)") } else { ("supplier_bills", "suppliers", "supplier_id", "(t.total - t.paid_amount)") };
    let sql = format!("SELECT coalesce(jsonb_agg(to_jsonb(x) ORDER BY x.total DESC), '[]') FROM (SELECT p.id AS party_id, p.name AS party_name,
        sum({bal}) FILTER (WHERE $2 - t.due_date <= 0) AS current, sum({bal}) FILTER (WHERE $2 - t.due_date BETWEEN 1 AND 30) AS d1_30, sum({bal}) FILTER (WHERE $2 - t.due_date BETWEEN 31 AND 60) AS d31_60,
        sum({bal}) FILTER (WHERE $2 - t.due_date BETWEEN 61 AND 90) AS d61_90, sum({bal}) FILTER (WHERE $2 - t.due_date > 90) AS d90_plus, sum({bal}) AS total
      FROM {table} t JOIN {party_t} p ON p.id = t.{party_col} WHERE t.company_id = $1 AND t.status IN ('open','partial') AND {bal} > 0 GROUP BY p.id, p.name) x");
    let rows: Value = sqlx::query_scalar(&sql).bind(company).bind(date).fetch_one(&st.db).await?;
    let mut tot = [Decimal::ZERO; 6];
    if let Some(a) = rows.as_array() { for r in a { for (i, k) in ["current", "d1_30", "d31_60", "d61_90", "d90_plus", "total"].iter().enumerate() { tot[i] += dec(&r[*k]); } } }
    Ok(json!({"as_of": date, "rows": rows, "totals": {"current": tot[0], "d1_30": tot[1], "d31_60": tot[2], "d61_90": tot[3], "d90_plus": tot[4], "total": tot[5]}}))
}
async fn ar_ageing(State(st): State<AppState>, user: AuthUser, Query(q): Query<HashMap<String, String>>) -> AppResult<Json<Value>> { let d = as_of(&st, &user, &q, "as_of").await?; Ok(Json(json!({"data": ageing(&st, user.company_id, true, d).await?}))) }
async fn ap_ageing(State(st): State<AppState>, user: AuthUser, Query(q): Query<HashMap<String, String>>) -> AppResult<Json<Value>> { let d = as_of(&st, &user, &q, "as_of").await?; Ok(Json(json!({"data": ageing(&st, user.company_id, false, d).await?}))) }

async fn stock_valuation(State(st): State<AppState>, user: AuthUser) -> AppResult<Json<Value>> {
    let rows: Value = sqlx::query_scalar("SELECT coalesce(jsonb_agg(to_jsonb(x) ORDER BY x.value DESC), '[]') FROM (SELECT p.id AS product_id, p.sku, p.name, coalesce(c.name, 'Uncategorised') AS category, sum(b.qty) AS qty, sum(b.value) AS value, CASE WHEN sum(b.qty) > 0 THEN round(sum(b.value) / sum(b.qty), 4) END AS avg_cost, p.sale_price FROM stock_balances b JOIN products p ON p.id = b.product_id LEFT JOIN categories c ON c.id = p.category_id WHERE b.company_id = $1 AND (b.qty <> 0 OR b.value <> 0) GROUP BY p.id, c.name) x").bind(user.company_id).fetch_one(&st.db).await?;
    let gl: Decimal = sqlx::query_scalar("SELECT coalesce(sum(l.debit - l.credit), 0) FROM journal_lines l JOIN accounts a ON a.id = l.account_id AND a.system_key = 'inventory' WHERE l.company_id = $1").bind(user.company_id).fetch_one(&st.db).await?;
    let sub: Decimal = rows.as_array().map(|a| a.iter().map(|r| dec(&r["value"])).sum()).unwrap_or_default();
    Ok(Json(json!({"data": {"items": rows, "stock_value": sub, "ledger_inventory": gl, "reconciled": gl == sub}})))
}

async fn sales_analysis(State(st): State<AppState>, user: AuthUser, Query(q): Query<HashMap<String, String>>) -> AppResult<Json<Value>> {
    let to = as_of(&st, &user, &q, "to").await?;
    let from = db::parse_date(&q, "from")?.unwrap_or_else(|| NaiveDate::from_ymd_opt(to.year(), to.month(), 1).unwrap());
    let by_customer: Value = sqlx::query_scalar("SELECT coalesce(jsonb_agg(to_jsonb(x) ORDER BY x.sales DESC), '[]') FROM (SELECT c.id, c.name, count(*) AS invoices, sum(i.subtotal) AS sales, sum(i.vat_amount) AS vat, sum(i.total) AS total FROM sales_invoices i JOIN customers c ON c.id = i.customer_id WHERE i.company_id = $1 AND i.status <> 'cancelled' AND i.invoice_date BETWEEN $2 AND $3 GROUP BY c.id) x").bind(user.company_id).bind(from).bind(to).fetch_one(&st.db).await?;
    let by_product: Value = sqlx::query_scalar("SELECT coalesce(jsonb_agg(to_jsonb(x) ORDER BY x.sales DESC), '[]') FROM (SELECT p.id, p.sku, p.name, sum(l.qty - l.returned_qty) AS qty, sum(l.net_amount) AS sales, sum(l.cost_value) AS cost, sum(l.net_amount) - sum(l.cost_value) AS margin, CASE WHEN sum(l.net_amount) > 0 THEN round((sum(l.net_amount) - sum(l.cost_value)) / sum(l.net_amount) * 100, 2) END AS margin_pct FROM sales_invoice_lines l JOIN sales_invoices i ON i.id = l.invoice_id JOIN products p ON p.id = l.product_id WHERE i.company_id = $1 AND i.status <> 'cancelled' AND i.invoice_date BETWEEN $2 AND $3 GROUP BY p.id) x").bind(user.company_id).bind(from).bind(to).fetch_one(&st.db).await?;
    let by_day: Value = sqlx::query_scalar("SELECT coalesce(jsonb_agg(to_jsonb(x) ORDER BY x.date), '[]') FROM (SELECT i.invoice_date AS date, sum(i.subtotal) AS sales FROM sales_invoices i WHERE i.company_id = $1 AND i.status <> 'cancelled' AND i.invoice_date BETWEEN $2 AND $3 GROUP BY 1) x").bind(user.company_id).bind(from).bind(to).fetch_one(&st.db).await?;
    Ok(Json(json!({"data": {"from": from, "to": to, "by_customer": by_customer, "by_product": by_product, "by_day": by_day}})))
}

/// Import pipeline: where every shipment stands and what is at risk.
async fn import_pipeline(State(st): State<AppState>, user: AuthUser) -> AppResult<Json<Value>> {
    let pipeline: Value = sqlx::query_scalar("SELECT coalesce(jsonb_agg(to_jsonb(x) ORDER BY x.eta NULLS LAST), '[]') FROM (SELECT h.id, h.shipment_no, h.status, h.eta, h.vessel_name, h.port_of_loading, h.port_of_discharge, h.currency, h.invoice_value, h.invoice_bdt, s.name AS supplier, o.po_no,
          (SELECT count(*) FROM import_documents d WHERE d.shipment_id = h.id AND d.is_required AND d.status IN ('pending','requested')) AS documents_missing,
          (SELECT min(c.free_until) FROM containers c WHERE c.shipment_id = h.id AND c.delivered_at IS NULL) AS free_until,
          (SELECT coalesce(sum(greatest(0, coalesce(c.delivered_at, (now() AT TIME ZONE 'Asia/Dhaka')::date) - c.free_until)), 0) * h.demurrage_per_day FROM containers c WHERE c.shipment_id = h.id AND c.free_until IS NOT NULL) AS demurrage_bdt
        FROM shipments h JOIN suppliers s ON s.id = h.supplier_id JOIN purchase_orders o ON o.id = h.po_id WHERE h.company_id = $1 AND h.status NOT IN ('received','cancelled')) x").bind(user.company_id).fetch_one(&st.db).await?;
    let by_status: Value = sqlx::query_scalar("SELECT coalesce(jsonb_object_agg(status, n), '{}') FROM (SELECT status, count(*) AS n FROM shipments WHERE company_id = $1 GROUP BY status) x").bind(user.company_id).fetch_one(&st.db).await?;
    let lcs: Value = sqlx::query_scalar("SELECT coalesce(jsonb_agg(to_jsonb(x) ORDER BY x.expiry_date), '[]') FROM (SELECT lc_no, currency, amount, amount - settled_amount AS outstanding, expiry_date, expiry_date - (now() AT TIME ZONE 'Asia/Dhaka')::date AS days_left FROM letters_of_credit WHERE company_id = $1 AND status = 'opened') x").bind(user.company_id).fetch_one(&st.db).await?;
    Ok(Json(json!({"data": {"shipments": pipeline, "by_status": by_status, "open_lcs": lcs}})))
}

/// Replenishment: stock cover against supplier lead time, with a suggested order quantity.
async fn replenishment(State(st): State<AppState>, user: AuthUser) -> AppResult<Json<Value>> {
    let mut conn = st.db.acquire().await?;
    let safety = db::setting_dec(&mut conn, user.company_id, "replenishment.safety_days", Decimal::from(10)).await?;
    drop(conn);
    let rows: Value = sqlx::query_scalar("SELECT coalesce(jsonb_agg(to_jsonb(x) ORDER BY x.days_of_cover NULLS LAST), '[]') FROM (
        SELECT p.id AS product_id, p.sku, p.name, p.lead_time_days, p.reorder_qty, p.volume_cbm, p.weight_kg,
          coalesce(s.qty, 0) AS on_hand, coalesce(t.qty, 0) AS in_transit, coalesce(o.qty, 0) AS on_order, coalesce(d.qty, 0) AS sold_90d, round(coalesce(d.qty, 0) / 90.0, 4) AS daily_demand,
          CASE WHEN coalesce(d.qty, 0) > 0 THEN round((coalesce(s.qty, 0) + coalesce(t.qty, 0)) / (d.qty / 90.0), 1) END AS days_of_cover
        FROM products p
        LEFT JOIN (SELECT product_id, sum(qty) AS qty FROM stock_balances WHERE company_id = $1 GROUP BY 1) s ON s.product_id = p.id
        LEFT JOIN (SELECT sl.product_id, sum(sl.qty - sl.received_qty) AS qty FROM shipment_lines sl JOIN shipments h ON h.id = sl.shipment_id WHERE h.company_id = $1 AND h.status IN ('in_transit','arrived','customs','cleared') GROUP BY 1) t ON t.product_id = p.id
        LEFT JOIN (SELECT pl.product_id, sum(pl.qty - pl.shipped_qty - pl.received_qty * (CASE WHEN o.po_type = 'local' THEN 1 ELSE 0 END)) AS qty FROM purchase_order_lines pl JOIN purchase_orders o ON o.id = pl.po_id WHERE o.company_id = $1 AND o.status IN ('approved','partially_received') AND pl.qty > pl.shipped_qty GROUP BY 1) o ON o.product_id = p.id
        LEFT JOIN (SELECT l.product_id, sum(l.qty - l.returned_qty) AS qty FROM sales_invoice_lines l JOIN sales_invoices i ON i.id = l.invoice_id WHERE i.company_id = $1 AND i.status <> 'cancelled' AND i.invoice_date >= (now() AT TIME ZONE 'Asia/Dhaka')::date - 90 GROUP BY 1) d ON d.product_id = p.id
        WHERE p.company_id = $1 AND p.is_active AND p.product_type = 'goods') x").bind(user.company_id).fetch_one(&st.db).await?;
    let mut out = vec![];
    if let Some(items) = rows.as_array() {
        for r in items {
            let daily = dec(&r["daily_demand"]);
            let lead = Decimal::from(r["lead_time_days"].as_i64().unwrap_or(0));
            let (on_hand, transit, ordered) = (dec(&r["on_hand"]), dec(&r["in_transit"]), dec(&r["on_order"]));
            let reorder_level_hit = false;
            let target = daily * (lead + safety + Decimal::from(60));
            let position = on_hand + transit + ordered;
            let mut suggest = if position < target { target - position } else { Decimal::ZERO };
            let moq = dec(&r["reorder_qty"]);
            if suggest > Decimal::ZERO && moq > Decimal::ZERO { suggest = (suggest / moq).ceil() * moq; }
            let order_by_days = if daily > Decimal::ZERO { Some(((on_hand + transit) / daily - lead - safety).round_dp(0)) } else { None };
            let status = match order_by_days { Some(d) if d < Decimal::ZERO => "overdue", Some(d) if d < Decimal::from(14) => "order_now", Some(_) => "ok", None => "no_demand" };
            let _ = reorder_level_hit;
            let mut row = r.clone();
            row["order_by_in_days"] = order_by_days.map(|d| json!(d)).unwrap_or(Value::Null);
            row["suggested_qty"] = json!(suggest.round_dp(0));
            row["suggested_cbm"] = json!((suggest * dec(&r["volume_cbm"])).round_dp(2));
            row["status"] = json!(status);
            out.push(row);
        }
    }
    Ok(Json(json!({"data": out, "safety_days": safety})))
}

/// Role-aware dashboard: every block is returned only when the caller may see it.
async fn dashboard(State(st): State<AppState>, user: AuthUser) -> AppResult<Json<Value>> {
    let c = user.company_id;
    let today = db::today(&mut *st.db.acquire().await?, c).await?;
    let month_start = NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap();
    let mut out = serde_json::Map::new();
    out.insert("as_of".into(), json!(today));
    out.insert("role".into(), json!(user.role));
    out.insert("greeting_name".into(), json!(user.full_name));
    async fn scalar(st: &AppState, sql: &str, c: uuid::Uuid, extra: Option<NaiveDate>) -> AppResult<Decimal> {
        let q = sqlx::query_scalar::<_, Decimal>(sql).bind(c);
        Ok(match extra { Some(d) => q.bind(d), None => q }.fetch_one(&st.db).await?)
    }
    if user.can("finance.read") {
        let cash = scalar(&st, "SELECT coalesce(sum(l.debit - l.credit), 0) FROM journal_lines l JOIN bank_accounts b ON b.account_id = l.account_id WHERE l.company_id = $1", c, None).await?;
        let ar = scalar(&st, "SELECT coalesce(sum(total - paid_amount - returned_amount), 0) FROM sales_invoices WHERE company_id = $1 AND status IN ('open','partial')", c, None).await?;
        let ar_over = scalar(&st, "SELECT coalesce(sum(total - paid_amount - returned_amount), 0) FROM sales_invoices WHERE company_id = $1 AND status IN ('open','partial') AND due_date < $2", c, Some(today)).await?;
        let ap = scalar(&st, "SELECT coalesce(sum(total - paid_amount), 0) FROM supplier_bills WHERE company_id = $1 AND status IN ('open','partial')", c, None).await?;
        let ap_fc = scalar(&st, "SELECT coalesce(sum(l.credit - l.debit), 0) FROM journal_lines l JOIN accounts a ON a.id = l.account_id AND a.system_key = 'ap_foreign' WHERE l.company_id = $1", c, None).await?;
        let bank_rows: Value = sqlx::query_scalar("SELECT coalesce(jsonb_agg(jsonb_build_object('id', b.id, 'name', b.name, 'kind', b.kind, 'currency', b.currency, 'balance', (SELECT coalesce(sum(l.debit - l.credit), 0) FROM journal_lines l WHERE l.account_id = b.account_id)) ORDER BY b.code), '[]') FROM bank_accounts b WHERE b.company_id = $1 AND b.is_active").bind(c).fetch_one(&st.db).await?;
        out.insert("finance".into(), json!({"cash_and_bank": cash, "receivables": ar, "receivables_overdue": ar_over, "payables_local": ap, "payables_foreign_bdt": ap_fc, "accounts": bank_rows}));
        let m: Value = sqlx::query_scalar("SELECT jsonb_build_object('sales', coalesce(sum(CASE WHEN a.account_type = 'income' THEN l.credit - l.debit END), 0), 'cogs', coalesce(sum(CASE WHEN a.system_key = 'cogs' THEN l.debit - l.credit END), 0), 'expenses', coalesce(sum(CASE WHEN a.account_type = 'expense' THEN l.debit - l.credit END), 0)) FROM journal_lines l JOIN journal_entries e ON e.id = l.entry_id AND e.entry_date >= $2 JOIN accounts a ON a.id = l.account_id WHERE l.company_id = $1 AND a.account_type IN ('income','expense')").bind(c).bind(month_start).fetch_one(&st.db).await?;
        let (sales, cogs, exp) = (dec(&m["sales"]), dec(&m["cogs"]), dec(&m["expenses"]));
        out.insert("month".into(), json!({"from": month_start, "revenue": sales, "cost_of_goods_sold": cogs, "gross_profit": sales - cogs, "gross_margin_pct": if sales.is_zero() { Value::Null } else { json!(r2((sales - cogs) / sales * Decimal::ONE_HUNDRED)) }, "net_profit": sales - exp}));
    }
    if user.can("sales.read") {
        let trend: Value = sqlx::query_scalar("SELECT coalesce(jsonb_agg(jsonb_build_object('date', d::date, 'sales', coalesce((SELECT sum(subtotal) FROM sales_invoices i WHERE i.company_id = $1 AND i.status <> 'cancelled' AND i.invoice_date = d::date), 0)) ORDER BY d), '[]') FROM generate_series($2::date - 29, $2::date, interval '1 day') d").bind(c).bind(today).fetch_one(&st.db).await?;
        let open_orders: i64 = sqlx::query_scalar("SELECT count(*) FROM sales_orders WHERE company_id = $1 AND status IN ('confirmed','partially_delivered')").bind(c).fetch_one(&st.db).await?;
        let to_invoice: i64 = sqlx::query_scalar("SELECT count(*) FROM deliveries WHERE company_id = $1 AND NOT invoiced").bind(c).fetch_one(&st.db).await?;
        let top: Value = sqlx::query_scalar("SELECT coalesce(jsonb_agg(to_jsonb(x)), '[]') FROM (SELECT c.name, sum(i.subtotal) AS sales FROM sales_invoices i JOIN customers c ON c.id = i.customer_id WHERE i.company_id = $1 AND i.status <> 'cancelled' AND i.invoice_date >= $2 GROUP BY c.name ORDER BY 2 DESC LIMIT 5) x").bind(c).bind(month_start).fetch_one(&st.db).await?;
        out.insert("sales".into(), json!({"trend_30d": trend, "open_orders": open_orders, "deliveries_to_invoice": to_invoice, "top_customers": top}));
    }
    if user.can("import.read") {
        let in_transit = scalar(&st, "SELECT coalesce(sum(l.debit - l.credit), 0) FROM journal_lines l JOIN accounts a ON a.id = l.account_id AND a.system_key = 'git' WHERE l.company_id = $1", c, None).await?;
        let rows: Vec<(String, i64)> = sqlx::query_as("SELECT status, count(*) FROM shipments WHERE company_id = $1 AND status NOT IN ('received','cancelled') GROUP BY 1").bind(c).fetch_all(&st.db).await?;
        let lc_exp: i64 = sqlx::query_scalar("SELECT count(*) FROM letters_of_credit WHERE company_id = $1 AND status = 'opened' AND expiry_date <= $2 + 30").bind(c).bind(today).fetch_one(&st.db).await?;
        let dem: Decimal = sqlx::query_scalar("SELECT coalesce(sum(h.demurrage_per_day * greatest(0, coalesce(c.delivered_at, $2) - c.free_until)), 0) FROM containers c JOIN shipments h ON h.id = c.shipment_id WHERE h.company_id = $1 AND c.free_until IS NOT NULL AND c.returned_at IS NULL").bind(c).bind(today).fetch_one(&st.db).await?;
        let soon: Value = sqlx::query_scalar("SELECT coalesce(jsonb_agg(to_jsonb(x) ORDER BY x.eta), '[]') FROM (SELECT h.id, h.shipment_no, h.eta, h.status, s.name AS supplier FROM shipments h JOIN suppliers s ON s.id = h.supplier_id WHERE h.company_id = $1 AND h.status IN ('booked','in_transit') AND h.eta IS NOT NULL ORDER BY h.eta LIMIT 5) x").bind(c).fetch_one(&st.db).await?;
        out.insert("import".into(), json!({"goods_in_transit_bdt": in_transit, "shipments_by_status": rows.into_iter().collect::<HashMap<_, _>>(), "lcs_expiring_30d": lc_exp, "demurrage_exposure_bdt": dem, "arriving_soon": soon}));
    }
    if user.can("inventory.read") {
        let value = scalar(&st, "SELECT coalesce(sum(value), 0) FROM stock_balances WHERE company_id = $1", c, None).await?;
        let low: i64 = sqlx::query_scalar("SELECT count(*) FROM (SELECT p.id FROM products p JOIN stock_balances b ON b.product_id = p.id WHERE p.company_id = $1 AND p.reorder_level > 0 GROUP BY p.id, p.reorder_level HAVING sum(b.qty) <= p.reorder_level) x").bind(c).fetch_one(&st.db).await?;
        out.insert("inventory".into(), json!({"stock_value": value, "low_stock_items": low}));
    }
    if user.can("approvals.read") {
        let mine: i64 = sqlx::query_scalar("SELECT count(*) FROM approval_requests WHERE company_id = $1 AND status = 'pending' AND (approver_role = $2 OR $3)").bind(c).bind(&user.role).bind(user.role == "owner").fetch_one(&st.db).await?;
        out.insert("approvals".into(), json!({"waiting_for_me": mine}));
    }
    if user.can("tax.read") {
        let v: Decimal = scalar(&st, "SELECT coalesce(sum(l.credit - l.debit), 0) FROM journal_lines l JOIN accounts a ON a.id = l.account_id AND a.system_key IN ('vat_payable') WHERE l.company_id = $1", c, None).await?;
        out.insert("tax".into(), json!({"vat_payable_to_treasury": v}));
    }
    if user.can("hr.read") {
        let heads: i64 = sqlx::query_scalar("SELECT count(*) FROM employees WHERE company_id = $1 AND status = 'active'").bind(c).fetch_one(&st.db).await?;
        let absent: i64 = sqlx::query_scalar("SELECT count(*) FROM attendance WHERE company_id = $1 AND work_date = $2 AND status = 'absent'").bind(c).bind(today).fetch_one(&st.db).await?;
        let leave: i64 = sqlx::query_scalar("SELECT count(*) FROM leave_requests WHERE company_id = $1 AND status = 'pending'").bind(c).fetch_one(&st.db).await?;
        out.insert("hr".into(), json!({"headcount": heads, "absent_today": absent, "pending_leave": leave}));
    }
    let unread: i64 = sqlx::query_scalar("SELECT count(*) FROM notifications WHERE company_id = $1 AND user_id = $2 AND read_at IS NULL").bind(c).bind(user.user_id).fetch_one(&st.db).await?;
    out.insert("notifications_unread".into(), json!(unread));
    Ok(Json(json!({"data": Value::Object(out)})))
}

/// One search box for everything the caller is allowed to open.
async fn search(State(st): State<AppState>, user: AuthUser, Query(q): Query<HashMap<String, String>>) -> AppResult<Json<Value>> {
    let term = q.get("q").map(|s| s.trim().to_string()).unwrap_or_default();
    if term.chars().count() < 2 { return Err(AppError::field("q", "Type at least 2 characters")); }
    let like = format!("%{}%", term.replace('%', "\\%").replace('_', "\\_"));
    let c = user.company_id;
    let mut results: Vec<Value> = vec![];
    async fn run(st: &AppState, sql: &str, c: uuid::Uuid, like: &str, out: &mut Vec<Value>) -> AppResult<()> {
        let rows: Value = sqlx::query_scalar(sql).bind(c).bind(like).fetch_one(&st.db).await?;
        if let Some(a) = rows.as_array() { out.extend(a.iter().cloned()); }
        Ok(())
    }
    if user.can("catalog.read") { run(&st, "SELECT coalesce(jsonb_agg(x), '[]') FROM (SELECT jsonb_build_object('type','product','id',id,'title',sku || ' · ' || name,'subtitle',coalesce(brand,''),'route','/products/' || id) x FROM products WHERE company_id = $1 AND (sku ILIKE $2 OR name ILIKE $2 OR name_bn ILIKE $2 OR barcode ILIKE $2) LIMIT 6) s", c, &like, &mut results).await?; }
    if user.can("sales.read") {
        run(&st, "SELECT coalesce(jsonb_agg(x), '[]') FROM (SELECT jsonb_build_object('type','customer','id',id,'title',name,'subtitle',code || coalesce(' · ' || phone,''),'route','/customers/' || id) x FROM customers WHERE company_id = $1 AND (name ILIKE $2 OR name_bn ILIKE $2 OR code ILIKE $2 OR phone ILIKE $2 OR bin ILIKE $2) LIMIT 6) s", c, &like, &mut results).await?;
        run(&st, "SELECT coalesce(jsonb_agg(x), '[]') FROM (SELECT jsonb_build_object('type','sales_order','id',o.id,'title',o.so_no,'subtitle',cu.name || ' · ' || o.status,'route','/sales-orders/' || o.id) x FROM sales_orders o JOIN customers cu ON cu.id = o.customer_id WHERE o.company_id = $1 AND (o.so_no ILIKE $2 OR cu.name ILIKE $2) ORDER BY o.created_at DESC LIMIT 5) s", c, &like, &mut results).await?;
        run(&st, "SELECT coalesce(jsonb_agg(x), '[]') FROM (SELECT jsonb_build_object('type','invoice','id',i.id,'title',i.invoice_no,'subtitle',cu.name || ' · ' || i.mushak_no,'route','/sales-invoices/' || i.id) x FROM sales_invoices i JOIN customers cu ON cu.id = i.customer_id WHERE i.company_id = $1 AND (i.invoice_no ILIKE $2 OR i.mushak_no ILIKE $2 OR cu.name ILIKE $2) ORDER BY i.created_at DESC LIMIT 5) s", c, &like, &mut results).await?;
    }
    if user.can("purchase.read") {
        run(&st, "SELECT coalesce(jsonb_agg(x), '[]') FROM (SELECT jsonb_build_object('type','supplier','id',id,'title',name,'subtitle',code || ' · ' || country,'route','/suppliers/' || id) x FROM suppliers WHERE company_id = $1 AND (name ILIKE $2 OR code ILIKE $2 OR contact_person ILIKE $2) LIMIT 6) s", c, &like, &mut results).await?;
        run(&st, "SELECT coalesce(jsonb_agg(x), '[]') FROM (SELECT jsonb_build_object('type','purchase_order','id',o.id,'title',o.po_no,'subtitle',s.name || ' · ' || o.status,'route','/purchase-orders/' || o.id) x FROM purchase_orders o JOIN suppliers s ON s.id = o.supplier_id WHERE o.company_id = $1 AND (o.po_no ILIKE $2 OR s.name ILIKE $2 OR o.pi_no ILIKE $2) ORDER BY o.created_at DESC LIMIT 5) s", c, &like, &mut results).await?;
    }
    if user.can("import.read") {
        run(&st, "SELECT coalesce(jsonb_agg(x), '[]') FROM (SELECT jsonb_build_object('type','shipment','id',h.id,'title',h.shipment_no,'subtitle',coalesce(h.bl_no,'') || ' · ' || h.status,'route','/shipments/' || h.id) x FROM shipments h WHERE h.company_id = $1 AND (h.shipment_no ILIKE $2 OR h.bl_no ILIKE $2 OR h.vessel_name ILIKE $2 OR EXISTS (SELECT 1 FROM containers c WHERE c.shipment_id = h.id AND c.container_no ILIKE $2)) ORDER BY h.created_at DESC LIMIT 5) s", c, &like, &mut results).await?;
        run(&st, "SELECT coalesce(jsonb_agg(x), '[]') FROM (SELECT jsonb_build_object('type','lc','id',id,'title',lc_no,'subtitle',coalesce(bank_lc_no,'') || ' · ' || status,'route','/lcs/' || id) x FROM letters_of_credit WHERE company_id = $1 AND (lc_no ILIKE $2 OR bank_lc_no ILIKE $2) LIMIT 5) s", c, &like, &mut results).await?;
    }
    if user.can("hr.read") { run(&st, "SELECT coalesce(jsonb_agg(x), '[]') FROM (SELECT jsonb_build_object('type','employee','id',id,'title',name,'subtitle',code || coalesce(' · ' || designation,''),'route','/employees/' || id) x FROM employees WHERE company_id = $1 AND (name ILIKE $2 OR code ILIKE $2 OR phone ILIKE $2) LIMIT 5) s", c, &like, &mut results).await?; }
    Ok(Json(json!({"data": results, "total": results.len()})))
}

pub fn api() -> Api {
    Api::new("Reports & dashboards")
        .get("/dashboard", AUTH, "Role-aware dashboard: only blocks the user may see", dashboard)
        .get("/search", AUTH, "Global search across records the user may open", search).query("q")
        .get("/reports/trial-balance", "finance.read", "Trial balance", trial_balance).query("as_of")
        .get("/reports/income-statement", "finance.read", "Profit and loss for a period", income_statement).query("from,to")
        .get("/reports/balance-sheet", "finance.read", "Balance sheet", balance_sheet).query("as_of")
        .get("/reports/general-ledger", "finance.read", "Ledger of one account with running balance", general_ledger).query("account_id,page,per_page,from,to")
        .get("/reports/receivables-ageing", "finance.read", "Customer ageing", ar_ageing).query("as_of")
        .get("/reports/payables-ageing", "finance.read", "Supplier ageing", ap_ageing).query("as_of")
        .get("/reports/stock-valuation", "inventory.read", "Stock value per product, reconciled to the ledger", stock_valuation)
        .get("/reports/sales-analysis", "sales.read", "Sales by customer, product (with margin) and day", sales_analysis).query("from,to")
        .get("/reports/import-pipeline", "import.read", "Shipments in flight, demurrage and LC expiry", import_pipeline)
        .get("/reports/replenishment", "inventory.read", "Reorder suggestions from cover, lead time and demand", replenishment)
}
