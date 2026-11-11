//! Sign-in, companies, branches, users, roles, settings, numbering and the audit log.
use crate::{api::{Api, AUTH, PUBLIC}, auth::{self, AuthUser}, db::{self, created, data, Created, RawJson}, error::{AppError, AppResult}, resource::{self, opt, req, Resource, K}, seed, state::AppState};
use axum::{extract::{Path, Query, State}, Json};
use chrono::{Duration, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use uuid::Uuid;

static BRANCHES: Resource = Resource {
    path: "/branches", table: "branches", tag: "Administration", label: "branch", read: AUTH, write: "admin.write",
    fields: &[req("code", K::Text(20)), req("name", K::Text(120)), opt("name_bn", K::Text(120)), opt("address", K::Text(400)), opt("phone", K::Text(40)), opt("is_head_office", K::Bool), opt("is_active", K::Bool)],
    search: &["code", "name"], filters: &["is_active"], order: "code", extra: "", auto_code: None,
    example: r#"{"code":"CTG","name":"Chattogram Branch","name_bn":"চট্টগ্রাম শাখা","address":"Agrabad C/A, Chattogram"}"#,
};

#[derive(Deserialize)]
struct SetupIn { company_code: String, company_name: String, company_name_bn: Option<String>, bin: Option<String>, tin: Option<String>, address: Option<String>, admin_email: String, admin_password: String, admin_name: String }

/// First-run setup. Works only while the system has no users.
async fn setup(State(st): State<AppState>, Json(b): Json<SetupIn>) -> AppResult<Created> {
    let mut tx = st.db.begin().await?;
    sqlx::query("LOCK TABLE users IN EXCLUSIVE MODE").execute(&mut *tx).await?;
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM users").fetch_one(&mut *tx).await?;
    if n > 0 { return Err(AppError::Conflict("The system is already set up. Please sign in.".into())); }
    let user = seed::create_user(&mut tx, &b.admin_email, &b.admin_password, &b.admin_name, true).await?;
    let company = seed::create_company(&mut tx, seed::NewCompany { code: &b.company_code, name: &b.company_name, name_bn: b.company_name_bn.as_deref(), bin: b.bin.as_deref(), tin: b.tin.as_deref(), address: b.address.as_deref() }, user).await?;
    tx.commit().await?;
    Ok(created(json!({"company_id": company, "user_id": user}).to_string()))
}

async fn session(st: &AppState, user: Uuid, company: Uuid) -> AppResult<Json<Value>> {
    let u = auth::load_user(st, user, company).await?.ok_or_else(|| AppError::Forbidden("You do not have access to this company".into()))?;
    let (access, expires_in) = auth::issue_access(st, user, company)?;
    let refresh = auth::new_refresh_token();
    sqlx::query("INSERT INTO refresh_tokens (user_id, company_id, token_hash, expires_at) VALUES ($1,$2,$3,$4)")
        .bind(user).bind(company).bind(auth::sha256_hex(&refresh)).bind(Utc::now() + Duration::days(st.cfg.refresh_ttl_days)).execute(&st.db).await?;
    let company_doc: Value = sqlx::query_scalar("SELECT to_jsonb(c) FROM companies c WHERE id = $1").bind(company).fetch_one(&st.db).await?;
    let companies: Value = sqlx::query_scalar("SELECT coalesce(jsonb_agg(jsonb_build_object('id', c.id, 'code', c.code, 'name', c.name, 'role', r.code) ORDER BY c.name), '[]') FROM user_companies uc JOIN companies c ON c.id = uc.company_id AND c.is_active JOIN roles r ON r.id = uc.role_id WHERE uc.user_id = $1")
        .bind(user).fetch_one(&st.db).await?;
    Ok(Json(json!({"data": {"access_token": access, "token_type": "Bearer", "expires_in": expires_in, "refresh_token": refresh, "user": u, "company": company_doc, "companies": companies}})))
}

#[derive(Deserialize)]
struct LoginIn { email: String, password: String, company_code: Option<String> }

async fn login(State(st): State<AppState>, Json(b): Json<LoginIn>) -> AppResult<Json<Value>> {
    let invalid = || AppError::Unauthorized("Email or password is incorrect".into());
    let row: Option<(Uuid, String, bool, i32, Option<chrono::DateTime<Utc>>)> = sqlx::query_as("SELECT id, password_hash, is_active, failed_logins, locked_until FROM users WHERE lower(email) = lower($1)")
        .bind(b.email.trim()).fetch_optional(&st.db).await?;
    let Some((id, hash, active, failed, locked)) = row else {
        // Spend comparable time so that unknown emails cannot be told apart by timing.
        let _ = auth::verify_password(&b.password, "$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHRzb21lc2FsdA$Zm9vYmFyYmF6cXV4Zm9vYmFyYmF6cXV4Zm9vYmFyYmE");
        return Err(invalid());
    };
    if let Some(until) = locked { if until > Utc::now() { return Err(AppError::Unauthorized("Too many failed attempts. Please try again later.".into())); } }
    if !active { return Err(invalid()); }
    let company: Option<Uuid> = match &b.company_code {
        Some(code) => sqlx::query_scalar("SELECT c.id FROM user_companies uc JOIN companies c ON c.id = uc.company_id WHERE uc.user_id = $1 AND c.code = upper($2) AND c.is_active").bind(id).bind(code).fetch_optional(&st.db).await?,
        None => sqlx::query_scalar("SELECT uc.company_id FROM user_companies uc JOIN companies c ON c.id = uc.company_id AND c.is_active WHERE uc.user_id = $1 ORDER BY uc.is_default DESC, c.created_at LIMIT 1").bind(id).fetch_optional(&st.db).await?,
    };
    if !auth::verify_password(&b.password, &hash) {
        let mut conn = st.db.acquire().await?;
        let (max, mins) = match company {
            Some(c) => (db::setting_dec(&mut conn, c, "auth.max_failed_logins", 5.into()).await?, db::setting_dec(&mut conn, c, "auth.lockout_minutes", 15.into()).await?),
            None => (5.into(), 15.into()),
        };
        let lock = rust_decimal::Decimal::from(failed + 1) >= max;
        sqlx::query("UPDATE users SET failed_logins = CASE WHEN $2 THEN 0 ELSE failed_logins + 1 END, locked_until = CASE WHEN $2 THEN now() + ($3 || ' minutes')::interval ELSE locked_until END WHERE id = $1")
            .bind(id).bind(lock).bind(mins.to_string()).execute(&mut *conn).await?;
        sqlx::query("INSERT INTO audit_logs (company_id, user_id, action, entity, entity_id, detail) VALUES ($1,$2,'login_failed','users',$2::text,$3)")
            .bind(company).bind(id).bind(json!({"locked": lock})).execute(&mut *conn).await?;
        return Err(invalid());
    }
    let company = company.ok_or_else(|| AppError::Forbidden("Your account is not linked to an active company".into()))?;
    sqlx::query("UPDATE users SET failed_logins = 0, locked_until = NULL, last_login_at = now() WHERE id = $1").bind(id).execute(&st.db).await?;
    sqlx::query("INSERT INTO audit_logs (company_id, user_id, action, entity, entity_id) VALUES ($1,$2,'login','users',$2::text)").bind(company).bind(id).execute(&st.db).await?;
    session(&st, id, company).await
}

#[derive(Deserialize)]
struct RefreshIn { refresh_token: String }

async fn refresh(State(st): State<AppState>, Json(b): Json<RefreshIn>) -> AppResult<Json<Value>> {
    // Rotation: a refresh token works exactly once.
    let row: Option<(Uuid, Uuid)> = sqlx::query_as("UPDATE refresh_tokens SET revoked_at = now() WHERE token_hash = $1 AND revoked_at IS NULL AND expires_at > now() RETURNING user_id, company_id")
        .bind(auth::sha256_hex(&b.refresh_token)).fetch_optional(&st.db).await?;
    let (user, company) = row.ok_or_else(|| AppError::Unauthorized("Session expired. Please sign in again.".into()))?;
    session(&st, user, company).await
}

async fn logout(State(st): State<AppState>, user: AuthUser, Json(b): Json<RefreshIn>) -> AppResult<Json<Value>> {
    sqlx::query("UPDATE refresh_tokens SET revoked_at = now() WHERE token_hash = $1 AND user_id = $2").bind(auth::sha256_hex(&b.refresh_token)).bind(user.user_id).execute(&st.db).await?;
    Ok(Json(json!({"data": {"signed_out": true}})))
}

#[derive(Deserialize)]
struct SwitchIn { company_id: Uuid }
async fn switch_company(State(st): State<AppState>, user: AuthUser, Json(b): Json<SwitchIn>) -> AppResult<Json<Value>> {
    session(&st, user.user_id, b.company_id).await
}

async fn me(State(st): State<AppState>, user: AuthUser) -> AppResult<Json<Value>> {
    let company: Value = sqlx::query_scalar("SELECT to_jsonb(c) FROM companies c WHERE id = $1").bind(user.company_id).fetch_one(&st.db).await?;
    Ok(Json(json!({"data": {"user": user, "company": company}})))
}

#[derive(Deserialize)]
struct ChangePw { current_password: String, new_password: String }
async fn change_password(State(st): State<AppState>, user: AuthUser, Json(b): Json<ChangePw>) -> AppResult<Json<Value>> {
    let hash: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = $1").bind(user.user_id).fetch_one(&st.db).await?;
    if !auth::verify_password(&b.current_password, &hash) { return Err(AppError::field("current_password", "Current password is incorrect")); }
    auth::validate_password(&b.new_password)?;
    let mut tx = st.db.begin().await?;
    sqlx::query("UPDATE users SET password_hash = $2 WHERE id = $1").bind(user.user_id).bind(auth::hash_password(&b.new_password)?).execute(&mut *tx).await?;
    sqlx::query("UPDATE refresh_tokens SET revoked_at = now() WHERE user_id = $1 AND revoked_at IS NULL").bind(user.user_id).execute(&mut *tx).await?;
    db::audit(&mut tx, &user, "change_password", "users", user.user_id, json!({})).await?;
    tx.commit().await?;
    Ok(Json(json!({"data": {"changed": true}})))
}

// ---- companies ----
async fn my_companies(State(st): State<AppState>, user: AuthUser) -> AppResult<RawJson> {
    let s: String = sqlx::query_scalar("SELECT coalesce(jsonb_agg(to_jsonb(c) || jsonb_build_object('role', r.code) ORDER BY c.name), '[]')::text FROM user_companies uc JOIN companies c ON c.id = uc.company_id JOIN roles r ON r.id = uc.role_id WHERE uc.user_id = $1")
        .bind(user.user_id).fetch_one(&st.db).await?;
    Ok(data(s))
}

#[derive(Deserialize)]
struct CompanyIn { code: String, name: String, name_bn: Option<String>, bin: Option<String>, tin: Option<String>, address: Option<String> }
async fn create_company(State(st): State<AppState>, user: AuthUser, Json(b): Json<CompanyIn>) -> AppResult<Created> {
    if !user.is_superadmin { return Err(AppError::Forbidden("Only a system administrator can add companies".into())); }
    let mut tx = st.db.begin().await?;
    let id = seed::create_company(&mut tx, seed::NewCompany { code: &b.code, name: &b.name, name_bn: b.name_bn.as_deref(), bin: b.bin.as_deref(), tin: b.tin.as_deref(), address: b.address.as_deref() }, user.user_id).await?;
    let doc: String = sqlx::query_scalar("SELECT to_jsonb(c)::text FROM companies c WHERE id = $1").bind(id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(created(doc))
}

async fn get_company(State(st): State<AppState>, user: AuthUser) -> AppResult<RawJson> {
    Ok(data(sqlx::query_scalar("SELECT to_jsonb(c)::text FROM companies c WHERE id = $1").bind(user.company_id).fetch_one(&st.db).await?))
}

static COMPANY: Resource = Resource {
    path: "/company", table: "companies", tag: "Administration", label: "company", read: AUTH, write: "admin.write",
    fields: &[opt("name", K::Text(160)), opt("name_bn", K::Text(160)), opt("legal_name", K::Text(200)), opt("bin", K::Text(30)), opt("tin", K::Text(30)), opt("irc_no", K::Text(40)), opt("trade_license", K::Text(60)),
        opt("address", K::Text(400)), opt("phone", K::Text(40)), opt("email", K::Email), opt("locale", K::Enum(&["en", "bn"])), opt("fiscal_year_start_month", K::Int)],
    search: &[], filters: &[], order: "code", extra: "", auto_code: None, example: "",
};
async fn update_company(State(st): State<AppState>, user: AuthUser, Json(b): Json<Value>) -> AppResult<RawJson> {
    let mut tx = st.db.begin().await?;
    let vals = resource::validate(&mut tx, &COMPANY, user.company_id, &b, false).await?;
    if vals.is_empty() { return Err(AppError::BadRequest("Nothing to update".into())); }
    if let Some(m) = vals.get("fiscal_year_start_month").and_then(Value::as_i64) { if !(1..=12).contains(&m) { return Err(AppError::field("fiscal_year_start_month", "Must be 1 to 12")); } }
    let sets = vals.keys().map(|c| format!("{c} = r.{c}")).collect::<Vec<_>>().join(", ");
    sqlx::query(&format!("UPDATE companies t SET {sets} FROM jsonb_populate_record(NULL::companies, $1) r WHERE t.id = $2")).bind(Value::Object(vals.clone())).bind(user.company_id).execute(&mut *tx).await?;
    db::audit(&mut tx, &user, "update", "companies", user.company_id, Value::Object(vals)).await?;
    let doc: String = sqlx::query_scalar("SELECT to_jsonb(c)::text FROM companies c WHERE id = $1").bind(user.company_id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(data(doc))
}

// ---- users ----
const USER_SELECT: &str = "u.id, u.email, u.full_name, u.phone, u.locale, u.is_active, u.last_login_at, u.created_at, r.code AS role, r.name AS role_name, uc.branch_id, (SELECT name FROM branches b WHERE b.id = uc.branch_id) AS branch_name FROM users u JOIN user_companies uc ON uc.user_id = u.id JOIN roles r ON r.id = uc.role_id WHERE uc.company_id = ";

async fn list_users(State(st): State<AppState>, user: AuthUser, Query(q): Query<HashMap<String, String>>) -> AppResult<RawJson> {
    let mut qb = db::list_query();
    qb.push(USER_SELECT); qb.push_bind(user.company_id);
    db::filter_search(&mut qb, &q, &["u.email", "u.full_name"]);
    db::filter_eq(&mut qb, &q, "role", "r.code", "");
    db::page_json(&st.db, qb, "full_name", &q).await
}

async fn user_doc(conn: &mut sqlx::PgConnection, company: Uuid, id: Uuid) -> AppResult<String> {
    sqlx::query_scalar(&format!("SELECT to_jsonb(x)::text FROM (SELECT {USER_SELECT} $1 AND u.id = $2) x")).bind(company).bind(id).fetch_optional(&mut *conn).await?.ok_or_else(|| AppError::not_found("user"))
}

async fn role_id(conn: &mut sqlx::PgConnection, company: Uuid, code: &str) -> AppResult<Uuid> {
    sqlx::query_scalar("SELECT id FROM roles WHERE company_id = $1 AND code = $2").bind(company).bind(code).fetch_optional(&mut *conn).await?.ok_or_else(|| AppError::field("role", "Unknown role"))
}

#[derive(Deserialize)]
struct UserIn { email: String, full_name: String, password: Option<String>, role: String, branch_id: Option<Uuid>, phone: Option<String>, locale: Option<String> }

async fn create_user(State(st): State<AppState>, user: AuthUser, Json(b): Json<UserIn>) -> AppResult<Created> {
    let mut tx = st.db.begin().await?;
    let role = role_id(&mut tx, user.company_id, &b.role).await?;
    if b.role == "owner" && user.role != "owner" && !user.is_superadmin { return Err(AppError::Forbidden("Only an owner can add another owner".into())); }
    if let Some(br) = b.branch_id { db::ensure(&mut tx, "branches", user.company_id, br, "branch_id").await?; }
    let existing: Option<Uuid> = sqlx::query_scalar("SELECT id FROM users WHERE lower(email) = lower($1)").bind(b.email.trim()).fetch_optional(&mut *tx).await?;
    let id = match existing {
        Some(id) => id, // the person already has an account; just grant access to this company
        None => {
            let pw = b.password.as_deref().ok_or_else(|| AppError::field("password", "This field is required"))?;
            let id = seed::create_user(&mut tx, &b.email, pw, &b.full_name, false).await?;
            sqlx::query("UPDATE users SET phone = $2, locale = coalesce($3, 'en') WHERE id = $1").bind(id).bind(&b.phone).bind(&b.locale).execute(&mut *tx).await?;
            id
        }
    };
    let first: bool = sqlx::query_scalar("SELECT NOT EXISTS (SELECT 1 FROM user_companies WHERE user_id = $1)").bind(id).fetch_one(&mut *tx).await?;
    sqlx::query("INSERT INTO user_companies (user_id, company_id, role_id, branch_id, is_default) VALUES ($1,$2,$3,$4,$5)").bind(id).bind(user.company_id).bind(role).bind(b.branch_id).bind(first).execute(&mut *tx).await?;
    db::audit(&mut tx, &user, "create", "users", id, json!({"email": b.email, "role": b.role})).await?;
    let doc = user_doc(&mut tx, user.company_id, id).await?;
    tx.commit().await?;
    Ok(created(doc))
}

#[derive(Deserialize)]
struct UserUpd { full_name: Option<String>, role: Option<String>, branch_id: Option<Option<Uuid>>, phone: Option<String>, locale: Option<String>, is_active: Option<bool> }

async fn update_user(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>, Json(b): Json<UserUpd>) -> AppResult<RawJson> {
    let mut tx = st.db.begin().await?;
    let current_role: String = sqlx::query_scalar("SELECT r.code FROM user_companies uc JOIN roles r ON r.id = uc.role_id WHERE uc.user_id = $1 AND uc.company_id = $2").bind(id).bind(user.company_id).fetch_optional(&mut *tx).await?.ok_or_else(|| AppError::not_found("user"))?;
    if (current_role == "owner" || b.role.as_deref() == Some("owner")) && user.role != "owner" && !user.is_superadmin { return Err(AppError::Forbidden("Only an owner can change an owner".into())); }
    if let Some(loc) = &b.locale { if loc != "en" && loc != "bn" { return Err(AppError::field("locale", "Must be en or bn")); } }
    if let Some(code) = &b.role {
        let rid = role_id(&mut tx, user.company_id, code).await?;
        if current_role == "owner" && code != "owner" {
            let owners: i64 = sqlx::query_scalar("SELECT count(*) FROM user_companies uc JOIN roles r ON r.id = uc.role_id JOIN users u ON u.id = uc.user_id AND u.is_active WHERE uc.company_id = $1 AND r.code = 'owner'").bind(user.company_id).fetch_one(&mut *tx).await?;
            if owners <= 1 { return Err(AppError::rule("A company must keep at least one owner")); }
        }
        sqlx::query("UPDATE user_companies SET role_id = $3 WHERE user_id = $1 AND company_id = $2").bind(id).bind(user.company_id).bind(rid).execute(&mut *tx).await?;
    }
    if let Some(br) = b.branch_id {
        if let Some(br) = br { db::ensure(&mut tx, "branches", user.company_id, br, "branch_id").await?; }
        sqlx::query("UPDATE user_companies SET branch_id = $3 WHERE user_id = $1 AND company_id = $2").bind(id).bind(user.company_id).bind(br).execute(&mut *tx).await?;
    }
    if b.is_active == Some(false) && id == user.user_id { return Err(AppError::rule("You cannot deactivate your own account")); }
    sqlx::query("UPDATE users SET full_name = coalesce($2, full_name), phone = coalesce($3, phone), locale = coalesce($4, locale), is_active = coalesce($5, is_active) WHERE id = $1")
        .bind(id).bind(b.full_name.as_ref().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())).bind(&b.phone).bind(&b.locale).bind(b.is_active).execute(&mut *tx).await?;
    if b.is_active == Some(false) { sqlx::query("UPDATE refresh_tokens SET revoked_at = now() WHERE user_id = $1 AND revoked_at IS NULL").bind(id).execute(&mut *tx).await?; }
    db::audit(&mut tx, &user, "update", "users", id, json!({"role": b.role, "is_active": b.is_active})).await?;
    let doc = user_doc(&mut tx, user.company_id, id).await?;
    tx.commit().await?;
    Ok(data(doc))
}

#[derive(Deserialize)]
struct ResetPw { new_password: String }
async fn reset_password(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>, Json(b): Json<ResetPw>) -> AppResult<Json<Value>> {
    auth::validate_password(&b.new_password)?;
    let mut tx = st.db.begin().await?;
    let _ = user_doc(&mut tx, user.company_id, id).await?;
    sqlx::query("UPDATE users SET password_hash = $2, failed_logins = 0, locked_until = NULL WHERE id = $1").bind(id).bind(auth::hash_password(&b.new_password)?).execute(&mut *tx).await?;
    sqlx::query("UPDATE refresh_tokens SET revoked_at = now() WHERE user_id = $1 AND revoked_at IS NULL").bind(id).execute(&mut *tx).await?;
    db::audit(&mut tx, &user, "reset_password", "users", id, json!({})).await?;
    tx.commit().await?;
    Ok(Json(json!({"data": {"reset": true}})))
}

// ---- roles ----
async fn list_roles(State(st): State<AppState>, user: AuthUser) -> AppResult<RawJson> {
    Ok(data(sqlx::query_scalar("SELECT coalesce(jsonb_agg(to_jsonb(r) || jsonb_build_object('users', (SELECT count(*) FROM user_companies uc WHERE uc.role_id = r.id)) ORDER BY r.is_system DESC, r.name), '[]')::text FROM roles r WHERE company_id = $1").bind(user.company_id).fetch_one(&st.db).await?))
}

#[derive(Deserialize)]
struct RoleIn { code: Option<String>, name: Option<String>, description: Option<String>, permissions: Option<Vec<String>> }

fn check_perms(perms: &[String]) -> AppResult<()> {
    for p in perms {
        let ok = p == "*" || seed::PERMISSIONS.iter().any(|(k, _)| k == p) || {
            let (m, a) = p.split_once('.').unwrap_or(("", ""));
            (a == "*" && seed::PERMISSIONS.iter().any(|(k, _)| k.starts_with(&format!("{m}.")))) || (m == "*" && seed::PERMISSIONS.iter().any(|(k, _)| k.ends_with(&format!(".{a}"))))
        };
        if !ok { return Err(AppError::field("permissions", format!("Unknown permission: {p}"))); }
    }
    Ok(())
}

async fn create_role(State(st): State<AppState>, user: AuthUser, Json(b): Json<RoleIn>) -> AppResult<Created> {
    let code = b.code.as_deref().map(str::trim).filter(|s| !s.is_empty()).ok_or_else(|| AppError::field("code", "This field is required"))?.to_lowercase();
    let name = b.name.as_deref().map(str::trim).filter(|s| !s.is_empty()).ok_or_else(|| AppError::field("name", "This field is required"))?;
    let perms = b.permissions.unwrap_or_default();
    check_perms(&perms)?;
    if perms.iter().any(|p| p == "*") && user.role != "owner" { return Err(AppError::Forbidden("Only an owner can grant full access".into())); }
    let mut tx = st.db.begin().await?;
    let doc: (Uuid, String) = sqlx::query_as("INSERT INTO roles (company_id, code, name, description, permissions) VALUES ($1,$2,$3,$4,$5) RETURNING id, to_jsonb(roles)::text")
        .bind(user.company_id).bind(&code).bind(name).bind(&b.description).bind(&perms).fetch_one(&mut *tx).await?;
    db::audit(&mut tx, &user, "create", "roles", doc.0, json!({"code": code, "permissions": perms})).await?;
    tx.commit().await?;
    Ok(created(doc.1))
}

async fn update_role(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>, Json(b): Json<RoleIn>) -> AppResult<RawJson> {
    let mut tx = st.db.begin().await?;
    let code: String = sqlx::query_scalar("SELECT code FROM roles WHERE id = $1 AND company_id = $2").bind(id).bind(user.company_id).fetch_optional(&mut *tx).await?.ok_or_else(|| AppError::not_found("role"))?;
    if let Some(p) = &b.permissions {
        check_perms(p)?;
        if code == "owner" { return Err(AppError::rule("The owner role always has full access and cannot be changed")); }
        if p.iter().any(|x| x == "*") && user.role != "owner" { return Err(AppError::Forbidden("Only an owner can grant full access".into())); }
    }
    let doc: String = sqlx::query_scalar("UPDATE roles SET name = coalesce($2, name), description = coalesce($3, description), permissions = coalesce($4, permissions) WHERE id = $1 RETURNING to_jsonb(roles)::text")
        .bind(id).bind(&b.name).bind(&b.description).bind(&b.permissions).fetch_one(&mut *tx).await?;
    db::audit(&mut tx, &user, "update", "roles", id, json!({"permissions": b.permissions})).await?;
    tx.commit().await?;
    Ok(data(doc))
}

async fn delete_role(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    let mut tx = st.db.begin().await?;
    let sys: bool = sqlx::query_scalar("SELECT is_system FROM roles WHERE id = $1 AND company_id = $2").bind(id).bind(user.company_id).fetch_optional(&mut *tx).await?.ok_or_else(|| AppError::not_found("role"))?;
    if sys { return Err(AppError::rule("Built-in roles cannot be deleted")); }
    sqlx::query("DELETE FROM roles WHERE id = $1").bind(id).execute(&mut *tx).await.map_err(|_| AppError::Conflict("This role is still assigned to users".into()))?;
    db::audit(&mut tx, &user, "delete", "roles", id, json!({})).await?;
    tx.commit().await?;
    Ok(Json(json!({"data": {"id": id, "deleted": true}})))
}

// ---- settings / policies ----
async fn get_settings(State(st): State<AppState>, user: AuthUser) -> AppResult<Json<Value>> {
    let rows: Vec<(String, Value)> = sqlx::query_as("SELECT key, value FROM settings WHERE company_id = $1").bind(user.company_id).fetch_all(&st.db).await?;
    let map: HashMap<String, Value> = rows.into_iter().collect();
    let out: Vec<Value> = seed::policies().into_iter().map(|(k, kind, def, desc)| json!({"key": k, "kind": kind, "value": map.get(k).cloned().unwrap_or(def.clone()), "default": def, "description": desc})).collect();
    Ok(Json(json!({"data": out})))
}

async fn put_settings(State(st): State<AppState>, user: AuthUser, Json(b): Json<HashMap<String, Value>>) -> AppResult<Json<Value>> {
    let registry = seed::policies();
    let mut tx = st.db.begin().await?;
    for (k, v) in &b {
        let (_, kind, _, _) = registry.iter().find(|p| p.0 == k).ok_or_else(|| AppError::field(k, "Unknown setting"))?;
        let ok = match *kind { "bool" => v.is_boolean(), _ => v.as_f64().map(|n| n >= 0.0).unwrap_or(false) };
        if !ok { return Err(AppError::field(k, if *kind == "bool" { "Must be true or false" } else { "Must be a number, zero or more" })); }
        sqlx::query("INSERT INTO settings (company_id, key, value) VALUES ($1,$2,$3) ON CONFLICT (company_id, key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()")
            .bind(user.company_id).bind(k).bind(v).execute(&mut *tx).await?;
    }
    db::audit(&mut tx, &user, "update", "settings", user.company_id, json!(b)).await?;
    tx.commit().await?;
    get_settings(State(st), user).await
}

async fn list_sequences(State(st): State<AppState>, user: AuthUser) -> AppResult<RawJson> {
    Ok(data(sqlx::query_scalar("SELECT coalesce(jsonb_agg(jsonb_build_object('key', key, 'prefix', prefix, 'next_no', next_no, 'padding', padding) ORDER BY key), '[]')::text FROM sequences WHERE company_id = $1").bind(user.company_id).fetch_one(&st.db).await?))
}
#[derive(Deserialize)]
struct SeqIn { prefix: Option<String>, padding: Option<i32> }
async fn update_sequence(State(st): State<AppState>, user: AuthUser, Path(key): Path<String>, Json(b): Json<SeqIn>) -> AppResult<Json<Value>> {
    if let Some(p) = &b.prefix { if p.is_empty() || p.len() > 12 || !p.chars().all(|c| c.is_ascii_alphanumeric() || c == '/' || c == '-') { return Err(AppError::field("prefix", "Use 1 to 12 letters, digits, - or /")); } }
    if let Some(p) = b.padding { if !(1..=10).contains(&p) { return Err(AppError::field("padding", "Must be 1 to 10")); } }
    let mut tx = st.db.begin().await?;
    let n = sqlx::query("UPDATE sequences SET prefix = coalesce($3, prefix), padding = coalesce($4, padding) WHERE company_id = $1 AND key = $2").bind(user.company_id).bind(&key).bind(&b.prefix).bind(b.padding).execute(&mut *tx).await?.rows_affected();
    if n == 0 { return Err(AppError::not_found("sequence")); }
    db::audit(&mut tx, &user, "update", "sequences", &key, json!({"prefix": b.prefix, "padding": b.padding})).await?;
    tx.commit().await?;
    Ok(Json(json!({"data": {"key": key, "updated": true}})))
}

async fn audit_logs(State(st): State<AppState>, user: AuthUser, Query(q): Query<HashMap<String, String>>) -> AppResult<RawJson> {
    let mut qb = db::list_query();
    qb.push("a.id, a.action, a.entity, a.entity_id, a.detail, a.created_at, a.user_id, (SELECT full_name FROM users u WHERE u.id = a.user_id) AS user_name FROM audit_logs a WHERE a.company_id = ");
    qb.push_bind(user.company_id);
    db::filter_eq(&mut qb, &q, "entity", "a.entity", "");
    db::filter_eq(&mut qb, &q, "entity_id", "a.entity_id", "");
    db::filter_eq(&mut qb, &q, "action", "a.action", "");
    db::filter_eq(&mut qb, &q, "user_id", "a.user_id", "::uuid");
    db::filter_dates(&mut qb, &q, "(a.created_at AT TIME ZONE 'Asia/Dhaka')::date");
    db::page_json(&st.db, qb, "id DESC", &q).await
}

async fn meta(State(st): State<AppState>, _user: AuthUser) -> AppResult<Json<Value>> {
    let currencies: Value = sqlx::query_scalar("SELECT jsonb_agg(to_jsonb(c) ORDER BY code) FROM currencies c").fetch_one(&st.db).await?;
    Ok(Json(json!({"data": {
        "currencies": currencies,
        "permissions": seed::PERMISSIONS.iter().map(|(k, d)| json!({"key": k, "description": d})).collect::<Vec<_>>(),
        "role_templates": seed::ROLES.iter().map(|(c, n, d, p)| json!({"code": c, "name": n, "description": d, "permissions": p})).collect::<Vec<_>>(),
        "enums": {
            "incoterms": ["EXW", "FOB", "CFR", "CIF", "CPT", "CIP", "DAP", "DDP"],
            "payment_modes": ["credit", "cash", "lc", "tt"],
            "lc_types": ["sight", "usance", "deferred", "upas"],
            "shipment_modes": ["sea", "air", "land"],
            "shipment_statuses": ["booked", "in_transit", "arrived", "customs", "cleared", "received", "cancelled"],
            "container_sizes": ["20GP", "40GP", "40HC", "45HC", "LCL", "AIR", "TRUCK"],
            "import_documents": {"PI": "Proforma Invoice", "CI": "Commercial Invoice", "PL": "Packing List", "BL": "Bill of Lading / Airway Bill", "COO": "Certificate of Origin", "LC": "Letter of Credit copy", "LCA": "LC Authorisation Form", "INS": "Insurance Cover Note", "IP": "Import Permit", "BSTI": "BSTI Clearance", "PHYTO": "Phytosanitary / Radiation Certificate", "TEST": "Test Report", "BE": "Bill of Entry", "DO": "Delivery Order", "OTHER": "Other"},
            "charge_types": ["freight", "insurance", "lc_commission", "bank_charges", "cnf_commission", "port_charges", "shipping_agent", "transport", "labour", "demurrage", "testing", "other"],
            "allocation_methods": ["value", "quantity", "weight", "volume"],
            "payment_methods": ["bank", "cash", "cheque", "bkash", "other"],
            "customer_types": ["dealer", "retail", "corporate", "government", "distributor"],
            "account_types": ["asset", "liability", "equity", "income", "expense"],
            "locales": ["en", "bn"]
        },
        "timezone": "Asia/Dhaka",
        "base_currency": "BDT"
    }})))
}

pub fn api() -> Api {
    let a = Api::new("Authentication")
        .post("/setup", PUBLIC, "First-run setup: create the first company and its owner", setup)
        .body(r#"{"company_code":"PTC","company_name":"PETRA Trade Corporation","company_name_bn":"পেট্রা ট্রেড কর্পোরেশন","bin":"000123456-0101","tin":"123456789012","address":"Motijheel C/A, Dhaka","admin_email":"owner@example.com","admin_password":"Str0ngPassw0rd","admin_name":"Business Owner"}"#)
        .post("/auth/login", PUBLIC, "Sign in", login).body(r#"{"email":"owner@example.com","password":"Str0ngPassw0rd","company_code":"PTC"}"#)
        .post("/auth/refresh", PUBLIC, "Exchange a refresh token for a new session (single use)", refresh).body(r#"{"refresh_token":"<token>"}"#)
        .post("/auth/logout", AUTH, "Sign out and revoke the refresh token", logout).body(r#"{"refresh_token":"<token>"}"#)
        .post("/auth/switch-company", AUTH, "Start a session in another company you belong to", switch_company).body(r#"{"company_id":"<uuid>"}"#)
        .get("/auth/me", AUTH, "Current user, role, permissions and company", me)
        .post("/auth/change-password", AUTH, "Change your own password", change_password).body(r#"{"current_password":"old","new_password":"NewStr0ngPass"}"#)
        .get("/meta", AUTH, "Lookups for forms: currencies, enums, permission catalogue", meta);
    let b = Api::new("Administration")
        .get("/companies", AUTH, "Companies you belong to", my_companies)
        .post("/companies", AUTH, "Add a company (system administrator only)", create_company).body(r#"{"code":"PTC2","name":"PETRA Distribution Ltd","bin":"000987654-0101"}"#)
        .get("/company", AUTH, "Current company profile", get_company)
        .put("/company", "admin.write", "Update company profile", update_company).body(r#"{"address":"Level 8, Motijheel C/A, Dhaka","irc_no":"BA-0123456"}"#)
        .get("/users", "admin.read", "Users of this company", list_users).query("page,per_page,q,role")
        .post("/users", "admin.write", "Add a user to this company", create_user).body(r#"{"email":"accounts@example.com","full_name":"Rahim Uddin","password":"Str0ngPassw0rd","role":"accountant","branch_id":null}"#)
        .put("/users/:id", "admin.write", "Update a user's name, role, branch or status", update_user).body(r#"{"role":"manager","is_active":true}"#)
        .post("/users/:id/reset-password", "admin.write", "Set a new password for a user", reset_password).body(r#"{"new_password":"NewStr0ngPass1"}"#)
        .get("/roles", "admin.read", "Roles with their permissions", list_roles)
        .post("/roles", "admin.write", "Create a custom role", create_role).body(r#"{"code":"store_keeper","name":"Store Keeper","permissions":["inventory.read","inventory.write","catalog.read"]}"#)
        .put("/roles/:id", "admin.write", "Update a role", update_role).body(r#"{"permissions":["inventory.*","catalog.read"]}"#)
        .delete("/roles/:id", "admin.write", "Delete a custom role", delete_role)
        .get("/settings", "admin.read", "Business policies and their current values", get_settings)
        .put("/settings", "admin.write", "Change business policies", put_settings).body(r#"{"sales.enforce_credit_limit":true,"customs.landing_charge_pct":1}"#)
        .get("/sequences", "admin.read", "Document numbering", list_sequences)
        .put("/sequences/:key", "admin.write", "Change a document prefix or padding", update_sequence).body(r#"{"prefix":"PO/25-26","padding":5}"#)
        .get("/audit-logs", "admin.read", "Who did what and when", audit_logs).query("page,per_page,entity,entity_id,action,user_id,from,to");
    resource::routes(a.merge(b), &BRANCHES)
}
