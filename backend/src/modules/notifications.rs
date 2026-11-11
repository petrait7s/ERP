//! In-app notifications. Role broadcasts fan out to one row per user so each person has their own read state.
use crate::{api::{Api, AUTH}, auth::AuthUser, db::{self, RawJson}, error::{AppError, AppResult}, state::AppState};
use axum::{extract::{Path, Query, State}, Json};
use serde_json::{json, Value};
use sqlx::PgConnection;
use std::collections::HashMap;
use uuid::Uuid;

pub struct Note<'a> {
    pub severity: &'a str,
    pub title: &'a str,
    pub title_bn: Option<&'a str>,
    pub body: &'a str,
    pub entity: Option<&'a str>,
    pub entity_id: Option<Uuid>,
    /// When set, the same notification is not created twice for the same user.
    pub dedupe: Option<String>,
}

pub async fn notify_user(conn: &mut PgConnection, company: Uuid, user: Uuid, n: &Note<'_>) -> AppResult<()> {
    sqlx::query("INSERT INTO notifications (company_id, user_id, severity, title, title_bn, body, entity, entity_id, dedupe_key) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9) ON CONFLICT DO NOTHING")
        .bind(company).bind(user).bind(n.severity).bind(n.title).bind(n.title_bn).bind(n.body).bind(n.entity).bind(n.entity_id)
        .bind(n.dedupe.as_ref().map(|d| format!("{d}:{user}"))).execute(&mut *conn).await?;
    Ok(())
}

/// Notify every active user holding one of the given roles (owners are always included).
pub async fn notify_roles(conn: &mut PgConnection, company: Uuid, roles: &[&str], n: &Note<'_>) -> AppResult<()> {
    let mut codes: Vec<String> = roles.iter().map(|s| s.to_string()).collect();
    if !codes.iter().any(|c| c == "owner") { codes.push("owner".into()); }
    let users: Vec<Uuid> = sqlx::query_scalar("SELECT u.id FROM users u JOIN user_companies uc ON uc.user_id = u.id JOIN roles r ON r.id = uc.role_id WHERE uc.company_id = $1 AND r.code = ANY($2) AND u.is_active")
        .bind(company).bind(&codes).fetch_all(&mut *conn).await?;
    for u in users { notify_user(conn, company, u, n).await?; }
    Ok(())
}

async fn list(State(st): State<AppState>, user: AuthUser, Query(q): Query<HashMap<String, String>>) -> AppResult<RawJson> {
    let mut qb = db::list_query();
    qb.push("n.id, n.severity, n.title, n.title_bn, n.body, n.entity, n.entity_id, n.read_at, n.created_at FROM notifications n WHERE n.company_id = ");
    qb.push_bind(user.company_id);
    qb.push(" AND n.user_id = "); qb.push_bind(user.user_id);
    if q.get("unread").map(String::as_str) == Some("true") { qb.push(" AND n.read_at IS NULL"); }
    db::filter_eq(&mut qb, &q, "severity", "n.severity", "");
    db::page_json(&st.db, qb, "created_at DESC", &q).await
}

async fn unread_count(State(st): State<AppState>, user: AuthUser) -> AppResult<Json<Value>> {
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM notifications WHERE company_id = $1 AND user_id = $2 AND read_at IS NULL").bind(user.company_id).bind(user.user_id).fetch_one(&st.db).await?;
    Ok(Json(json!({"data": {"unread": n}})))
}

async fn mark_read(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    let n = sqlx::query("UPDATE notifications SET read_at = coalesce(read_at, now()) WHERE id = $1 AND user_id = $2 AND company_id = $3").bind(id).bind(user.user_id).bind(user.company_id).execute(&st.db).await?.rows_affected();
    if n == 0 { return Err(AppError::not_found("notification")); }
    Ok(Json(json!({"data": {"id": id, "read": true}})))
}

async fn mark_all(State(st): State<AppState>, user: AuthUser) -> AppResult<Json<Value>> {
    let n = sqlx::query("UPDATE notifications SET read_at = now() WHERE user_id = $1 AND company_id = $2 AND read_at IS NULL").bind(user.user_id).bind(user.company_id).execute(&st.db).await?.rows_affected();
    Ok(Json(json!({"data": {"marked": n}})))
}

pub fn api() -> Api {
    Api::new("Notifications")
        .get("/notifications", AUTH, "Your notifications, newest first", list).query("page,per_page,unread,severity")
        .get("/notifications/unread-count", AUTH, "Number of unread notifications", unread_count)
        .post("/notifications/:id/read", AUTH, "Mark one notification as read", mark_read)
        .post("/notifications/read-all", AUTH, "Mark everything as read", mark_all)
}
