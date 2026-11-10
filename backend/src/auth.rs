//! Authentication (Argon2 passwords, JWT access tokens, rotating refresh tokens)
//! and the per-request user context used for authorization and tenant scoping.
use crate::{error::{AppError, AppResult}, state::AppState};
use argon2::{password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString}, Argon2};
use axum::{async_trait, extract::{FromRequestParts, Request, State}, http::{header, request::Parts}, middleware::Next, response::Response};
use chrono::Utc;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: Uuid,
    pub cid: Uuid,
    pub exp: i64,
    pub iat: i64,
}

/// Who is calling, for which company, and what they may do.
#[derive(Debug, Clone, Serialize)]
pub struct AuthUser {
    pub user_id: Uuid,
    pub company_id: Uuid,
    pub full_name: String,
    pub email: String,
    pub role: String,
    /// `None` means the user may work in every branch of the company.
    pub branch_id: Option<Uuid>,
    pub permissions: Vec<String>,
    pub is_superadmin: bool,
    pub locale: String,
}

impl AuthUser {
    /// Permission strings are `module.action`. Grants may be exact, `module.*`, `*.action` or `*`.
    pub fn can(&self, perm: &str) -> bool {
        let (module, action) = perm.split_once('.').unwrap_or((perm, ""));
        self.permissions.iter().any(|p| {
            p == "*" || p == perm || (p.ends_with(".*") && &p[..p.len() - 2] == module) || (p.starts_with("*.") && &p[2..] == action)
        })
    }
    pub fn require(&self, perm: &str) -> AppResult<()> {
        if self.can(perm) { Ok(()) } else { Err(AppError::Forbidden(format!("You do not have permission: {perm}"))) }
    }
    /// Branch to stamp on new documents: the user's own branch, else the requested one.
    pub fn branch_for(&self, requested: Option<Uuid>) -> AppResult<Option<Uuid>> {
        match (self.branch_id, requested) {
            (Some(own), Some(req)) if own != req => Err(AppError::Forbidden("You can only work in your own branch".into())),
            (Some(own), _) => Ok(Some(own)),
            (None, r) => Ok(r),
        }
    }
}

#[async_trait]
impl<S: Send + Sync> FromRequestParts<S> for AuthUser {
    type Rejection = AppError;
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        parts.extensions.get::<AuthUser>().cloned().ok_or_else(|| AppError::Unauthorized("Please sign in".into()))
    }
}

pub fn hash_password(password: &str) -> AppResult<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default().hash_password(password.as_bytes(), &salt).map(|h| h.to_string()).map_err(|e| AppError::Internal(e.to_string()))
}
pub fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash).map(|h| Argon2::default().verify_password(password.as_bytes(), &h).is_ok()).unwrap_or(false)
}
pub fn validate_password(p: &str) -> AppResult<()> {
    if p.len() < 10 || !p.chars().any(|c| c.is_ascii_digit()) || !p.chars().any(|c| c.is_alphabetic()) {
        return Err(AppError::field("password", "Password needs at least 10 characters with letters and numbers"));
    }
    Ok(())
}

pub fn issue_access(st: &AppState, user: Uuid, company: Uuid) -> AppResult<(String, i64)> {
    let now = Utc::now().timestamp();
    let exp = now + st.cfg.access_ttl_minutes * 60;
    let token = encode(&Header::default(), &Claims { sub: user, cid: company, exp, iat: now }, &EncodingKey::from_secret(st.cfg.jwt_secret.as_bytes()))
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok((token, st.cfg.access_ttl_minutes * 60))
}

pub fn sha256_hex(s: &str) -> String { hex::encode(Sha256::digest(s.as_bytes())) }

pub fn new_refresh_token() -> String {
    use rand::RngCore;
    let mut b = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut b);
    hex::encode(b)
}

/// Load the caller's live context (role, permissions, branch). Done on every request so
/// that deactivating a user or changing a role takes effect immediately.
pub async fn load_user(st: &AppState, user: Uuid, company: Uuid) -> AppResult<Option<AuthUser>> {
    let row: Option<(Uuid, String, String, String, Option<Uuid>, Vec<String>, bool, String)> = sqlx::query_as(
        "SELECT u.id, u.full_name, u.email, r.code, uc.branch_id, r.permissions, u.is_superadmin, u.locale
         FROM users u
         JOIN user_companies uc ON uc.user_id = u.id AND uc.company_id = $2
         JOIN roles r ON r.id = uc.role_id
         JOIN companies c ON c.id = uc.company_id AND c.is_active
         WHERE u.id = $1 AND u.is_active",
    ).bind(user).bind(company).fetch_optional(&st.db).await?;
    Ok(row.map(|r| AuthUser { user_id: r.0, company_id: company, full_name: r.1, email: r.2, role: r.3, branch_id: r.4, permissions: r.5, is_superadmin: r.6, locale: r.7 }))
}

/// Global middleware: attach [`AuthUser`] when a valid bearer token is present.
/// It never rejects; each route's guard decides what is required.
pub async fn attach_user(State(st): State<AppState>, mut req: Request, next: Next) -> Response {
    let token = req.headers().get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).map(str::to_string);
    if let Some(token) = token {
        if let Ok(data) = decode::<Claims>(&token, &DecodingKey::from_secret(st.cfg.jwt_secret.as_bytes()), &Validation::default()) {
            if let Ok(Some(user)) = load_user(&st, data.claims.sub, data.claims.cid).await {
                req.extensions_mut().insert(user);
            }
        }
    }
    next.run(req).await
}
