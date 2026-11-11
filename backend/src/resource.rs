//! Declarative CRUD for master data. A [`Resource`] names a table and its editable
//! fields; this module supplies validated list/get/create/update/delete endpoints,
//! tenant scoping, audit logging and API docs for it.
//!
//! Table and column names come only from `'static` definitions in code, never from requests.
use crate::{api::Api, auth::AuthUser, db::{self, created, data, Created, RawJson}, error::{AppError, AppResult, FieldError}, state::AppState};
use axum::{extract::{Path, Query, State}, Json};
use rust_decimal::Decimal;
use serde_json::{json, Map, Value};
use sqlx::PgConnection;
use std::{collections::HashMap, str::FromStr};
use uuid::Uuid;

#[derive(Clone, Copy)]
pub enum K {
    /// Free text with a maximum length.
    Text(usize),
    Int,
    /// Non-negative decimal.
    Dec,
    Bool,
    /// Reference to a row of the named table in the same company.
    Fk(&'static str),
    /// Reference to a global table keyed by text code (e.g. currencies).
    Code(&'static str),
    Date,
    Enum(&'static [&'static str]),
    Email,
}

pub struct F {
    pub name: &'static str,
    pub kind: K,
    pub required: bool,
}
pub const fn req(name: &'static str, kind: K) -> F { F { name, kind, required: true } }
pub const fn opt(name: &'static str, kind: K) -> F { F { name, kind, required: false } }

pub struct Resource {
    pub path: &'static str,
    pub table: &'static str,
    pub tag: &'static str,
    pub label: &'static str,
    pub read: &'static str,
    pub write: &'static str,
    pub fields: &'static [F],
    pub search: &'static [&'static str],
    pub filters: &'static [&'static str],
    pub order: &'static str,
    /// Extra computed columns for reads, e.g. `, (SELECT name FROM units u WHERE u.id = t.unit_id) AS unit_name`.
    pub extra: &'static str,
    /// `(field, sequence key, prefix)`: fill the field with the next number when it is omitted.
    pub auto_code: Option<(&'static str, &'static str, &'static str)>,
    pub example: &'static str,
}

fn bad(errs: &mut Vec<FieldError>, field: &str, msg: &str) { errs.push(FieldError { field: field.into(), message: msg.into() }); }

/// Validate and normalise a JSON body against the resource definition.
pub async fn validate(conn: &mut PgConnection, r: &Resource, company: Uuid, body: &Value, creating: bool) -> AppResult<Map<String, Value>> {
    let obj = body.as_object().ok_or_else(|| AppError::BadRequest("Request body must be a JSON object".into()))?;
    let mut errs = vec![];
    let mut out = Map::new();
    for key in obj.keys() {
        if !r.fields.iter().any(|f| f.name == key) {
            bad(&mut errs, key, "Unknown field");
        }
    }
    for f in r.fields {
        let v = match obj.get(f.name) {
            None => {
                if creating && f.required && r.auto_code.map(|a| a.0) != Some(f.name) { bad(&mut errs, f.name, "This field is required"); }
                continue;
            }
            Some(v) => v,
        };
        if v.is_null() {
            if f.required { bad(&mut errs, f.name, "This field is required"); } else { out.insert(f.name.into(), Value::Null); }
            continue;
        }
        match f.kind {
            K::Text(max) => match v.as_str().map(str::trim) {
                Some(s) if s.is_empty() && f.required => bad(&mut errs, f.name, "This field is required"),
                Some(s) if s.chars().count() > max => bad(&mut errs, f.name, &format!("Must be at most {max} characters")),
                Some(s) => { out.insert(f.name.into(), if s.is_empty() { Value::Null } else { json!(s) }); }
                None => bad(&mut errs, f.name, "Must be text"),
            },
            K::Email => match v.as_str().map(str::trim) {
                Some(s) if s.is_empty() && !f.required => { out.insert(f.name.into(), Value::Null); }
                Some(s) if s.contains('@') && s.contains('.') && s.len() <= 200 && !s.contains(' ') => { out.insert(f.name.into(), json!(s.to_lowercase())); }
                _ => bad(&mut errs, f.name, "Must be a valid email address"),
            },
            K::Int => match v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok())) {
                Some(n) if n >= 0 => { out.insert(f.name.into(), json!(n)); }
                _ => bad(&mut errs, f.name, "Must be a whole number, zero or more"),
            },
            K::Dec => {
                let s = match v { Value::Number(n) => n.to_string(), Value::String(s) => s.trim().to_string(), _ => String::new() };
                match Decimal::from_str(&s).or_else(|_| Decimal::from_scientific(&s)) {
                    Ok(d) if d >= Decimal::ZERO => { out.insert(f.name.into(), json!(d.to_string())); }
                    Ok(_) => bad(&mut errs, f.name, "Must be zero or more"),
                    Err(_) => bad(&mut errs, f.name, "Must be a number"),
                }
            }
            K::Bool => match v.as_bool() {
                Some(b) => { out.insert(f.name.into(), json!(b)); }
                None => bad(&mut errs, f.name, "Must be true or false"),
            },
            K::Date => match v.as_str().and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()) {
                Some(d) => { out.insert(f.name.into(), json!(d.to_string())); }
                None => bad(&mut errs, f.name, "Must be a date like 2026-07-01"),
            },
            K::Enum(allowed) => match v.as_str() {
                Some(s) if allowed.contains(&s) => { out.insert(f.name.into(), json!(s)); }
                _ => bad(&mut errs, f.name, &format!("Must be one of: {}", allowed.join(", "))),
            },
            K::Fk(table) => match v.as_str().and_then(|s| Uuid::parse_str(s).ok()) {
                Some(id) => {
                    let ok: Option<i32> = sqlx::query_scalar(&format!("SELECT 1 FROM {table} WHERE id = $1 AND company_id = $2")).bind(id).bind(company).fetch_optional(&mut *conn).await?;
                    if ok.is_some() { out.insert(f.name.into(), json!(id)); } else { bad(&mut errs, f.name, "The selected record does not exist"); }
                }
                None => bad(&mut errs, f.name, "Must be a valid id"),
            },
            K::Code(table) => match v.as_str() {
                Some(code) => {
                    let ok: Option<i32> = sqlx::query_scalar(&format!("SELECT 1 FROM {table} WHERE code = $1")).bind(code).fetch_optional(&mut *conn).await?;
                    if ok.is_some() { out.insert(f.name.into(), json!(code)); } else { bad(&mut errs, f.name, "Unknown code") }
                }
                None => bad(&mut errs, f.name, "Must be a code"),
            },
        }
    }
    if errs.is_empty() { Ok(out) } else { Err(AppError::Validation(errs)) }
}

pub async fn fetch_one(conn: &mut PgConnection, r: &Resource, company: Uuid, id: Uuid) -> AppResult<String> {
    let sql = format!("SELECT to_jsonb(x)::text FROM (SELECT t.*{} FROM {} t WHERE t.id = $1 AND t.company_id = $2) x", r.extra, r.table);
    sqlx::query_scalar(&sql).bind(id).bind(company).fetch_optional(&mut *conn).await?.ok_or_else(|| AppError::not_found(r.label))
}

pub async fn insert(conn: &mut PgConnection, r: &Resource, user: &AuthUser, body: &Value) -> AppResult<Uuid> {
    let mut vals = validate(conn, r, user.company_id, body, true).await?;
    if let Some((field, key, prefix)) = r.auto_code {
        if !vals.contains_key(field) {
            let code = db::next_no(conn, user.company_id, key, prefix).await?;
            vals.insert(field.into(), json!(code));
        }
    }
    let cols: Vec<&str> = r.fields.iter().map(|f| f.name).filter(|n| vals.contains_key(*n)).collect();
    let sql = format!(
        "INSERT INTO {t} (company_id{c}) SELECT $1{rc} FROM jsonb_populate_record(NULL::{t}, $2) r RETURNING id",
        t = r.table,
        c = cols.iter().map(|c| format!(", {c}")).collect::<String>(),
        rc = cols.iter().map(|c| format!(", r.{c}")).collect::<String>(),
    );
    let id: Uuid = sqlx::query_scalar(&sql).bind(user.company_id).bind(Value::Object(vals.clone())).fetch_one(&mut *conn).await?;
    db::audit(conn, user, "create", r.table, id, Value::Object(vals)).await?;
    Ok(id)
}

async fn list(r: &'static Resource, st: AppState, user: AuthUser, q: HashMap<String, String>) -> AppResult<RawJson> {
    let mut qb = db::list_query();
    qb.push(format!("t.*{} FROM {} t WHERE t.company_id = ", r.extra, r.table));
    qb.push_bind(user.company_id);
    let cols: Vec<String> = r.search.iter().map(|c| format!("t.{c}")).collect();
    db::filter_search(&mut qb, &q, &cols.iter().map(String::as_str).collect::<Vec<_>>());
    for f in r.filters {
        if let Some(v) = q.get(*f).filter(|v| !v.is_empty()) {
            qb.push(format!(" AND t.{f}::text = "));
            qb.push_bind(v.clone());
        }
    }
    let order = match q.get("sort") {
        Some(s) if r.fields.iter().any(|f| f.name == s) => format!("{s} {}", if q.get("order").map(String::as_str) == Some("desc") { "DESC" } else { "ASC" }),
        _ => r.order.to_string(),
    };
    db::page_json(&st.db, qb, &order, &q).await
}

async fn get_one(r: &'static Resource, st: AppState, user: AuthUser, id: Uuid) -> AppResult<RawJson> {
    let mut conn = st.db.acquire().await?;
    Ok(data(fetch_one(&mut conn, r, user.company_id, id).await?))
}

async fn create(r: &'static Resource, st: AppState, user: AuthUser, body: Value) -> AppResult<Created> {
    let mut tx = st.db.begin().await?;
    let id = insert(&mut tx, r, &user, &body).await?;
    let doc = fetch_one(&mut tx, r, user.company_id, id).await?;
    tx.commit().await?;
    Ok(created(doc))
}

async fn update(r: &'static Resource, st: AppState, user: AuthUser, id: Uuid, body: Value) -> AppResult<RawJson> {
    let mut tx = st.db.begin().await?;
    let vals = validate(&mut tx, r, user.company_id, &body, false).await?;
    if vals.is_empty() { return Err(AppError::BadRequest("Nothing to update".into())); }
    let sets = vals.keys().map(|c| format!("{c} = r.{c}")).collect::<Vec<_>>().join(", ");
    let sql = format!("UPDATE {t} t SET {sets} FROM jsonb_populate_record(NULL::{t}, $1) r WHERE t.id = $2 AND t.company_id = $3", t = r.table);
    let n = sqlx::query(&sql).bind(Value::Object(vals.clone())).bind(id).bind(user.company_id).execute(&mut *tx).await?.rows_affected();
    if n == 0 { return Err(AppError::not_found(r.label)); }
    db::audit(&mut tx, &user, "update", r.table, id, Value::Object(vals)).await?;
    let doc = fetch_one(&mut tx, r, user.company_id, id).await?;
    tx.commit().await?;
    Ok(data(doc))
}

async fn remove(r: &'static Resource, st: AppState, user: AuthUser, id: Uuid) -> AppResult<Json<Value>> {
    let mut tx = st.db.begin().await?;
    let res = sqlx::query(&format!("DELETE FROM {} WHERE id = $1 AND company_id = $2", r.table)).bind(id).bind(user.company_id).execute(&mut *tx).await;
    match res {
        Ok(d) if d.rows_affected() == 0 => return Err(AppError::not_found(r.label)),
        Ok(_) => {}
        Err(sqlx::Error::Database(e)) if e.code().as_deref() == Some("23503") => {
            return Err(AppError::Conflict(format!("This {} is already used in transactions. Mark it inactive instead of deleting it.", r.label)));
        }
        Err(e) => return Err(e.into()),
    }
    db::audit(&mut tx, &user, "delete", r.table, id, json!({})).await?;
    tx.commit().await?;
    Ok(Json(json!({"data": {"id": id, "deleted": true}})))
}

/// Register the five standard routes for a resource.
pub fn routes(api: Api, r: &'static Resource) -> Api {
    let item = format!("{}/:id", r.path);
    let l = r.label;
    api.tag(r.tag).get(r.path, r.read, &format!("List {l} records"), move |State(st): State<AppState>, user: AuthUser, Query(q): Query<HashMap<String, String>>| list(r, st, user, q))
        .query(Box::leak(format!("page,per_page,q,sort,order{}", r.filters.iter().map(|f| format!(",{f}")).collect::<String>()).into_boxed_str()))
        .post(r.path, r.write, &format!("Create a {l}"), move |State(st): State<AppState>, user: AuthUser, Json(b): Json<Value>| create(r, st, user, b))
        .body(r.example)
        .get(&item, r.read, &format!("Get one {l}"), move |State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>| get_one(r, st, user, id))
        .put(&item, r.write, &format!("Update a {l} (send only the fields to change)"), move |State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>, Json(b): Json<Value>| update(r, st, user, id, b))
        .delete(&item, r.write, &format!("Delete a {l} (refused when already used)"), move |State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>| remove(r, st, user, id))
}
