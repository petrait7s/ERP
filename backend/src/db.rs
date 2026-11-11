//! Shared database helpers: numbering, audit trail, settings, JSON reads, decimal rounding.
use crate::{auth::AuthUser, error::{AppError, AppResult}};
use axum::{http::header, response::{IntoResponse, Response}};
use chrono::NaiveDate;
use rust_decimal::{Decimal, RoundingStrategy};
use serde_json::Value;
use sqlx::{PgConnection, PgPool, Postgres, QueryBuilder};
use std::collections::HashMap;
use uuid::Uuid;

/// A JSON document produced by PostgreSQL and passed through untouched,
/// so NUMERIC values keep their exact digits.
pub struct RawJson(pub String);
impl IntoResponse for RawJson {
    fn into_response(self) -> Response {
        ([(header::CONTENT_TYPE, "application/json")], self.0).into_response()
    }
}
/// Same as [`RawJson`] but answers 201 Created.
pub struct Created(pub String);
impl IntoResponse for Created {
    fn into_response(self) -> Response {
        (axum::http::StatusCode::CREATED, [(header::CONTENT_TYPE, "application/json")], self.0).into_response()
    }
}

pub fn data(json: String) -> RawJson { RawJson(format!("{{\"data\":{json}}}")) }
pub fn created(json: String) -> Created { Created(format!("{{\"data\":{json}}}")) }

/// Round money to 2 decimals, half away from zero (paisa).
pub fn r2(d: Decimal) -> Decimal { d.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero) }
pub fn r4(d: Decimal) -> Decimal { d.round_dp_with_strategy(4, RoundingStrategy::MidpointAwayFromZero) }
pub fn r6(d: Decimal) -> Decimal { d.round_dp_with_strategy(6, RoundingStrategy::MidpointAwayFromZero) }
/// `amount * rate / 100`, rounded to money.
pub fn pct(amount: Decimal, rate: Decimal) -> Decimal { r2(amount * rate / Decimal::ONE_HUNDRED) }

/// Next document number for a company, e.g. `PO-00042`. Safe under concurrency (row lock).
pub async fn next_no(conn: &mut PgConnection, company: Uuid, key: &str, default_prefix: &str) -> AppResult<String> {
    let row: (String, i64, i32) = sqlx::query_as(
        "INSERT INTO sequences (company_id, key, prefix, next_no) VALUES ($1, $2, $3, 2)
         ON CONFLICT (company_id, key) DO UPDATE SET next_no = sequences.next_no + 1
         RETURNING prefix, next_no - 1, padding",
    ).bind(company).bind(key).bind(default_prefix).fetch_one(&mut *conn).await?;
    Ok(format!("{}-{:0width$}", row.0, row.1, width = row.2 as usize))
}

pub async fn audit(conn: &mut PgConnection, user: &AuthUser, action: &str, entity: &str, entity_id: impl ToString, detail: Value) -> AppResult<()> {
    sqlx::query("INSERT INTO audit_logs (company_id, user_id, action, entity, entity_id, detail) VALUES ($1,$2,$3,$4,$5,$6)")
        .bind(user.company_id).bind(user.user_id).bind(action).bind(entity).bind(entity_id.to_string()).bind(detail)
        .execute(&mut *conn).await?;
    Ok(())
}

/// Business date "today" in the company's time zone (Asia/Dhaka by default).
pub async fn today(conn: &mut PgConnection, company: Uuid) -> AppResult<NaiveDate> {
    Ok(sqlx::query_scalar("SELECT (now() AT TIME ZONE timezone)::date FROM companies WHERE id = $1")
        .bind(company).fetch_one(&mut *conn).await?)
}

pub async fn setting(conn: &mut PgConnection, company: Uuid, key: &str) -> AppResult<Option<Value>> {
    Ok(sqlx::query_scalar("SELECT value FROM settings WHERE company_id = $1 AND key = $2")
        .bind(company).bind(key).fetch_optional(&mut *conn).await?)
}
pub async fn setting_dec(conn: &mut PgConnection, company: Uuid, key: &str, default: Decimal) -> AppResult<Decimal> {
    Ok(match setting(conn, company, key).await? {
        Some(Value::Number(n)) => n.to_string().parse().unwrap_or(default),
        Some(Value::String(s)) => s.parse().unwrap_or(default),
        _ => default,
    })
}
pub async fn setting_bool(conn: &mut PgConnection, company: Uuid, key: &str, default: bool) -> AppResult<bool> {
    Ok(match setting(conn, company, key).await? { Some(Value::Bool(b)) => b, _ => default })
}

/// Verify that `id` exists in `table` for this company. `table` must be a compile-time constant.
pub async fn ensure(conn: &mut PgConnection, table: &'static str, company: Uuid, id: Uuid, label: &str) -> AppResult<()> {
    let ok: Option<i32> = sqlx::query_scalar(&format!("SELECT 1 FROM {table} WHERE id = $1 AND company_id = $2"))
        .bind(id).bind(company).fetch_optional(&mut *conn).await?;
    ok.map(|_| ()).ok_or_else(|| AppError::field(label, format!("{label} does not exist")))
}

/// Start a paged list query. Push the select list and FROM/WHERE after this prefix,
/// then finish with [`page_json`].
pub fn list_query<'a>() -> QueryBuilder<'a, Postgres> {
    QueryBuilder::new("WITH x AS (SELECT count(*) OVER() AS _total, ")
}

pub struct Page { pub page: i64, pub per_page: i64 }
pub fn page_of(q: &HashMap<String, String>) -> Page {
    let page = q.get("page").and_then(|v| v.parse::<i64>().ok()).unwrap_or(1).max(1);
    let per_page = q.get("per_page").and_then(|v| v.parse::<i64>().ok()).unwrap_or(50).clamp(1, 500);
    Page { page, per_page }
}

/// Finish a [`list_query`]: apply ordering and paging, return `{data,total,page,per_page}`.
/// `order` must be a trusted SQL fragment (never user input).
pub async fn page_json(pool: &PgPool, mut qb: QueryBuilder<'_, Postgres>, order: &str, q: &HashMap<String, String>) -> AppResult<RawJson> {
    let p = page_of(q);
    qb.push(format!(" ORDER BY {order} LIMIT "));
    qb.push_bind(p.per_page);
    qb.push(" OFFSET ");
    qb.push_bind((p.page - 1) * p.per_page);
    qb.push(format!(
        ") SELECT jsonb_build_object('data', coalesce(jsonb_agg(to_jsonb(x) - '_total'), '[]'::jsonb), 'total', coalesce(max(x._total), 0), 'page', {}, 'per_page', {})::text FROM x",
        p.page, p.per_page
    ));
    let s: String = qb.build_query_scalar().fetch_one(pool).await?;
    Ok(RawJson(s))
}

/// Optional exact-match filter from the query string, bound safely.
pub fn filter_eq<'a>(qb: &mut QueryBuilder<'a, Postgres>, q: &HashMap<String, String>, param: &str, column: &str, cast: &str) {
    if let Some(v) = q.get(param).filter(|v| !v.is_empty()) {
        qb.push(format!(" AND {column} = "));
        qb.push_bind(v.clone());
        qb.push(cast.to_string());
    }
}
/// Optional date range filter (`from`, `to`) on a date column.
pub fn filter_dates<'a>(qb: &mut QueryBuilder<'a, Postgres>, q: &HashMap<String, String>, column: &str) {
    if let Some(v) = q.get("from").filter(|v| !v.is_empty()) {
        qb.push(format!(" AND {column} >= ")); qb.push_bind(v.clone()); qb.push("::date");
    }
    if let Some(v) = q.get("to").filter(|v| !v.is_empty()) {
        qb.push(format!(" AND {column} <= ")); qb.push_bind(v.clone()); qb.push("::date");
    }
}
/// Optional case-insensitive search over several columns.
pub fn filter_search<'a>(qb: &mut QueryBuilder<'a, Postgres>, q: &HashMap<String, String>, columns: &[&str]) {
    if let Some(v) = q.get("q").filter(|v| !v.trim().is_empty()) {
        let like = format!("%{}%", v.trim().replace('%', "\\%").replace('_', "\\_"));
        qb.push(" AND (");
        for (i, c) in columns.iter().enumerate() {
            if i > 0 { qb.push(" OR "); }
            qb.push(format!("{c} ILIKE "));
            qb.push_bind(like.clone());
        }
        qb.push(")");
    }
}

pub fn parse_date(q: &HashMap<String, String>, key: &str) -> AppResult<Option<NaiveDate>> {
    match q.get(key).filter(|v| !v.is_empty()) {
        None => Ok(None),
        Some(v) => NaiveDate::parse_from_str(v, "%Y-%m-%d").map(Some).map_err(|_| AppError::BadRequest(format!("{key} must be a date like 2026-07-01"))),
    }
}
