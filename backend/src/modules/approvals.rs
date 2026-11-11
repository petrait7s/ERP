//! Approval engine. Rules say "documents of this type at or above this amount need this role".
//! A document may need several levels; each level is decided in order.
use crate::{api::Api, auth::AuthUser, db::{self, created, data, Created, RawJson}, error::{AppError, AppResult}, modules::{finance, hr, import, inventory, notifications::{self, Note}, purchase}, state::AppState};
use axum::{extract::{Path, Query, State}, Json};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::json;
use sqlx::PgConnection;
use std::collections::HashMap;
use uuid::Uuid;

pub const ENTITY_TYPES: &[&str] = &["purchase_order", "sales_order", "supplier_payment", "foreign_payment", "payroll_run", "stock_adjustment"];

fn label(entity_type: &str) -> &'static str {
    match entity_type { "purchase_order" => "Purchase order", "sales_order" => "Sales order", "supplier_payment" => "Supplier payment", "foreign_payment" => "Foreign payment", "payroll_run" => "Payroll", "stock_adjustment" => "Stock adjustment", _ => "Document" }
}

/// Ask for approval. Returns `false` when no rule applies (the caller proceeds immediately).
pub async fn submit(conn: &mut PgConnection, user: &AuthUser, entity_type: &'static str, entity_id: Uuid, entity_no: &str, amount: Decimal) -> AppResult<bool> {
    // For each level, the rule with the highest threshold that the amount reaches.
    let rules: Vec<(i32, String)> = sqlx::query_as(
        "SELECT DISTINCT ON (level) level, approver_role FROM approval_rules WHERE company_id = $1 AND entity_type = $2 AND is_active AND min_amount <= $3 ORDER BY level, min_amount DESC",
    ).bind(user.company_id).bind(entity_type).bind(amount).fetch_all(&mut *conn).await?;
    if rules.is_empty() { return Ok(false); }
    for (i, (level, role)) in rules.iter().enumerate() {
        sqlx::query("INSERT INTO approval_requests (company_id, entity_type, entity_id, entity_no, amount, level, approver_role, status, requested_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)")
            .bind(user.company_id).bind(entity_type).bind(entity_id).bind(entity_no).bind(amount).bind(level).bind(role).bind(if i == 0 { "pending" } else { "waiting" }).bind(user.user_id)
            .execute(&mut *conn).await?;
    }
    let title = format!("{} {} needs your approval", label(entity_type), entity_no);
    notifications::notify_roles(conn, user.company_id, &[rules[0].1.as_str()], &Note { severity: "warning", title: &title, title_bn: Some("অনুমোদন প্রয়োজন"), body: &format!("Requested by {}. Amount: Tk {}", user.full_name, amount), entity: Some(entity_type), entity_id: Some(entity_id), dedupe: None }).await?;
    Ok(true)
}

async fn on_decided(conn: &mut PgConnection, user: &AuthUser, entity_type: &str, entity_id: Uuid, approved: bool) -> AppResult<()> {
    match entity_type {
        "purchase_order" => purchase::on_approval(conn, user, entity_id, approved).await,
        "sales_order" => crate::modules::sales::on_approval(conn, user, entity_id, approved).await,
        "supplier_payment" => finance::on_payment_approval(conn, user, entity_id, approved).await,
        "foreign_payment" => import::on_payment_approval(conn, user, entity_id, approved).await,
        "payroll_run" => hr::on_approval(conn, user, entity_id, approved).await,
        "stock_adjustment" => inventory::on_adjustment_approval(conn, user, entity_id, approved).await,
        other => Err(AppError::Internal(format!("no approval handler for {other}"))),
    }
}

#[derive(Deserialize)]
struct Decision { comment: Option<String> }

async fn decide(st: AppState, user: AuthUser, id: Uuid, approve: bool, comment: Option<String>) -> AppResult<RawJson> {
    let mut tx = st.db.begin().await?;
    let (etype, eid, eno, level, role, status, requester): (String, Uuid, String, i32, String, String, Option<Uuid>) = sqlx::query_as(
        "SELECT entity_type, entity_id, entity_no, level, approver_role, status, requested_by FROM approval_requests WHERE id = $1 AND company_id = $2 FOR UPDATE",
    ).bind(id).bind(user.company_id).fetch_optional(&mut *tx).await?.ok_or_else(|| AppError::not_found("approval request"))?;
    if status != "pending" { return Err(AppError::rule(format!("This request is already {status}"))); }
    if user.role != role && user.role != "owner" { return Err(AppError::Forbidden(format!("This request must be decided by the {role} role"))); }
    if requester == Some(user.user_id) && user.role != "owner" { return Err(AppError::Forbidden("You cannot approve your own request".into())); }
    if !approve && comment.as_deref().map(str::trim).unwrap_or("").is_empty() { return Err(AppError::field("comment", "Please say why you are rejecting")); }

    sqlx::query("UPDATE approval_requests SET status = $2, decided_by = $3, decided_at = now(), comment = $4 WHERE id = $1")
        .bind(id).bind(if approve { "approved" } else { "rejected" }).bind(user.user_id).bind(&comment).execute(&mut *tx).await?;
    let mut finished = !approve;
    if approve {
        let next: Option<(Uuid, String)> = sqlx::query_as("UPDATE approval_requests SET status = 'pending' WHERE id = (SELECT id FROM approval_requests WHERE company_id = $1 AND entity_type = $2 AND entity_id = $3 AND status = 'waiting' ORDER BY level LIMIT 1) RETURNING id, approver_role")
            .bind(user.company_id).bind(&etype).bind(eid).fetch_optional(&mut *tx).await?;
        match next {
            Some((_, next_role)) => {
                let title = format!("{} {} needs your approval", label(&etype), eno);
                notifications::notify_roles(&mut tx, user.company_id, &[next_role.as_str()], &Note { severity: "warning", title: &title, title_bn: Some("অনুমোদন প্রয়োজন"), body: &format!("Level {} approved by {}", level, user.full_name), entity: Some(&etype), entity_id: Some(eid), dedupe: None }).await?;
            }
            None => finished = true,
        }
    } else {
        sqlx::query("UPDATE approval_requests SET status = 'cancelled' WHERE company_id = $1 AND entity_type = $2 AND entity_id = $3 AND status = 'waiting'").bind(user.company_id).bind(&etype).bind(eid).execute(&mut *tx).await?;
    }
    if finished {
        on_decided(&mut tx, &user, &etype, eid, approve).await?;
        if let Some(r) = requester {
            let title = format!("{} {} was {}", label(&etype), eno, if approve { "approved" } else { "rejected" });
            notifications::notify_user(&mut tx, user.company_id, r, &Note { severity: if approve { "success" } else { "critical" }, title: &title, title_bn: None, body: comment.as_deref().unwrap_or(""), entity: Some(&etype), entity_id: Some(eid), dedupe: None }).await?;
        }
    }
    db::audit(&mut tx, &user, if approve { "approve" } else { "reject" }, &etype, eid, json!({"level": level, "comment": comment})).await?;
    let doc: String = sqlx::query_scalar("SELECT to_jsonb(a)::text FROM approval_requests a WHERE id = $1").bind(id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(data(doc))
}

async fn approve(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>, body: Option<Json<Decision>>) -> AppResult<RawJson> { decide(st, user, id, true, body.and_then(|b| b.0.comment)).await }
async fn reject(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>, Json(b): Json<Decision>) -> AppResult<RawJson> { decide(st, user, id, false, b.comment).await }

async fn list(State(st): State<AppState>, user: AuthUser, Query(q): Query<HashMap<String, String>>) -> AppResult<RawJson> {
    let mut qb = db::list_query();
    qb.push("a.*, (SELECT full_name FROM users u WHERE u.id = a.requested_by) AS requested_by_name, (SELECT full_name FROM users u WHERE u.id = a.decided_by) AS decided_by_name, (a.status = 'pending' AND (a.approver_role = ");
    qb.push_bind(user.role.clone());
    qb.push(" OR "); qb.push_bind(user.role == "owner"); qb.push(")) AS can_decide FROM approval_requests a WHERE a.company_id = ");
    qb.push_bind(user.company_id);
    db::filter_eq(&mut qb, &q, "status", "a.status", "");
    db::filter_eq(&mut qb, &q, "entity_type", "a.entity_type", "");
    db::filter_eq(&mut qb, &q, "entity_id", "a.entity_id", "::uuid");
    if q.get("mine").map(String::as_str) == Some("true") {
        qb.push(" AND a.status = 'pending' AND (a.approver_role = "); qb.push_bind(user.role.clone()); qb.push(" OR "); qb.push_bind(user.role == "owner"); qb.push(")");
    }
    db::page_json(&st.db, qb, "a.created_at DESC", &q).await
}

async fn list_rules(State(st): State<AppState>, user: AuthUser) -> AppResult<RawJson> {
    Ok(data(sqlx::query_scalar("SELECT coalesce(jsonb_agg(to_jsonb(r) ORDER BY r.entity_type, r.min_amount, r.level), '[]')::text FROM approval_rules r WHERE company_id = $1").bind(user.company_id).fetch_one(&st.db).await?))
}

#[derive(Deserialize)]
struct RuleIn { entity_type: String, min_amount: Decimal, level: Option<i32>, approver_role: String, description: Option<String>, is_active: Option<bool> }

async fn check_rule(conn: &mut PgConnection, company: Uuid, b: &RuleIn) -> AppResult<()> {
    if !ENTITY_TYPES.contains(&b.entity_type.as_str()) { return Err(AppError::field("entity_type", format!("Must be one of: {}", ENTITY_TYPES.join(", ")))); }
    if b.min_amount < Decimal::ZERO { return Err(AppError::field("min_amount", "Must be zero or more")); }
    if !(1..=5).contains(&b.level.unwrap_or(1)) { return Err(AppError::field("level", "Must be 1 to 5")); }
    let ok: Option<i32> = sqlx::query_scalar("SELECT 1 FROM roles WHERE company_id = $1 AND code = $2").bind(company).bind(&b.approver_role).fetch_optional(&mut *conn).await?;
    ok.map(|_| ()).ok_or_else(|| AppError::field("approver_role", "Unknown role"))
}

async fn create_rule(State(st): State<AppState>, user: AuthUser, Json(b): Json<RuleIn>) -> AppResult<Created> {
    let mut tx = st.db.begin().await?;
    check_rule(&mut tx, user.company_id, &b).await?;
    let (id, doc): (Uuid, String) = sqlx::query_as("INSERT INTO approval_rules (company_id, entity_type, min_amount, level, approver_role, description, is_active) VALUES ($1,$2,$3,$4,$5,$6,$7) RETURNING id, to_jsonb(approval_rules)::text")
        .bind(user.company_id).bind(&b.entity_type).bind(b.min_amount).bind(b.level.unwrap_or(1)).bind(&b.approver_role).bind(&b.description).bind(b.is_active.unwrap_or(true)).fetch_one(&mut *tx).await?;
    db::audit(&mut tx, &user, "create", "approval_rules", id, json!({"entity_type": b.entity_type, "min_amount": b.min_amount, "approver_role": b.approver_role})).await?;
    tx.commit().await?;
    Ok(created(doc))
}

async fn update_rule(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>, Json(b): Json<RuleIn>) -> AppResult<RawJson> {
    let mut tx = st.db.begin().await?;
    check_rule(&mut tx, user.company_id, &b).await?;
    let doc: String = sqlx::query_scalar("UPDATE approval_rules SET entity_type = $3, min_amount = $4, level = $5, approver_role = $6, description = coalesce($7, description), is_active = coalesce($8, is_active) WHERE id = $1 AND company_id = $2 RETURNING to_jsonb(approval_rules)::text")
        .bind(id).bind(user.company_id).bind(&b.entity_type).bind(b.min_amount).bind(b.level.unwrap_or(1)).bind(&b.approver_role).bind(&b.description).bind(b.is_active).fetch_optional(&mut *tx).await?.ok_or_else(|| AppError::not_found("approval rule"))?;
    db::audit(&mut tx, &user, "update", "approval_rules", id, json!({"min_amount": b.min_amount, "approver_role": b.approver_role, "is_active": b.is_active})).await?;
    tx.commit().await?;
    Ok(data(doc))
}

async fn delete_rule(State(st): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> AppResult<Json<serde_json::Value>> {
    let mut tx = st.db.begin().await?;
    let n = sqlx::query("DELETE FROM approval_rules WHERE id = $1 AND company_id = $2").bind(id).bind(user.company_id).execute(&mut *tx).await?.rows_affected();
    if n == 0 { return Err(AppError::not_found("approval rule")); }
    db::audit(&mut tx, &user, "delete", "approval_rules", id, json!({})).await?;
    tx.commit().await?;
    Ok(Json(json!({"data": {"id": id, "deleted": true}})))
}

pub fn api() -> Api {
    Api::new("Approvals")
        .get("/approvals", "approvals.read", "Approval requests", list).query("page,per_page,status,entity_type,entity_id,mine")
        .post("/approvals/:id/approve", "approvals.decide", "Approve a pending request", approve).body(r#"{"comment":"Price matches the proforma"}"#)
        .post("/approvals/:id/reject", "approvals.decide", "Reject a pending request (comment required)", reject).body(r#"{"comment":"Negotiate a better price first"}"#)
        .get("/approval-rules", "approvals.read", "Approval rules", list_rules)
        .post("/approval-rules", "admin.write", "Add an approval rule", create_rule).body(r#"{"entity_type":"purchase_order","min_amount":1000000,"level":1,"approver_role":"manager","description":"POs of Tk 10 lakh or more need a manager"}"#)
        .put("/approval-rules/:id", "admin.write", "Change an approval rule", update_rule)
        .delete("/approval-rules/:id", "admin.write", "Remove an approval rule", delete_rule)
}
