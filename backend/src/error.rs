//! Uniform API error type. Every error leaves the server as
//! `{"error": {"code", "message", "details"}}` with a matching HTTP status.
use axum::{http::StatusCode, response::{IntoResponse, Response}, Json};
use serde::Serialize;
use serde_json::json;

#[derive(Debug, Serialize, Clone)]
pub struct FieldError {
    pub field: String,
    pub message: String,
}

#[derive(Debug)]
pub enum AppError {
    BadRequest(String),
    Validation(Vec<FieldError>),
    Unauthorized(String),
    Forbidden(String),
    NotFound(String),
    Conflict(String),
    /// A business rule rejected the request (HTTP 422).
    Rule(String),
    Internal(String),
}

pub type AppResult<T> = Result<T, AppError>;

impl AppError {
    pub fn rule(msg: impl Into<String>) -> Self { AppError::Rule(msg.into()) }
    pub fn not_found(what: &str) -> Self { AppError::NotFound(format!("{what} not found")) }
    pub fn field(field: &str, message: impl Into<String>) -> Self {
        AppError::Validation(vec![FieldError { field: field.into(), message: message.into() }])
    }
    fn parts(&self) -> (StatusCode, &'static str, String) {
        match self {
            AppError::BadRequest(m) => (StatusCode::BAD_REQUEST, "bad_request", m.clone()),
            AppError::Validation(_) => (StatusCode::UNPROCESSABLE_ENTITY, "validation_failed", "One or more fields are invalid".into()),
            AppError::Unauthorized(m) => (StatusCode::UNAUTHORIZED, "unauthorized", m.clone()),
            AppError::Forbidden(m) => (StatusCode::FORBIDDEN, "forbidden", m.clone()),
            AppError::NotFound(m) => (StatusCode::NOT_FOUND, "not_found", m.clone()),
            AppError::Conflict(m) => (StatusCode::CONFLICT, "conflict", m.clone()),
            AppError::Rule(m) => (StatusCode::UNPROCESSABLE_ENTITY, "business_rule", m.clone()),
            AppError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal_error", "Something went wrong on our side".into()),
        }
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{:?}", self) }
}
impl std::error::Error for AppError {}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code, message) = self.parts();
        if let AppError::Internal(m) = &self {
            tracing::error!(error = %m, "internal error");
        }
        let details = match &self {
            AppError::Validation(f) => json!(f),
            _ => json!([]),
        };
        (status, Json(json!({ "error": { "code": code, "message": message, "details": details } }))).into_response()
    }
}

impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        match &e {
            sqlx::Error::RowNotFound => AppError::NotFound("record not found".into()),
            sqlx::Error::Database(db) => {
                let code = db.code().map(|c| c.to_string()).unwrap_or_default();
                let constraint = db.constraint().unwrap_or("").to_string();
                match code.as_str() {
                    "23505" => AppError::Conflict(format!("A record with the same value already exists ({constraint})")),
                    "23503" => AppError::Conflict(format!("The record is referenced by, or refers to, another record ({constraint})")),
                    "23514" => AppError::Rule(format!("Value violates rule {constraint}")),
                    "23502" => AppError::BadRequest(format!("A required value is missing: {}", db.message())),
                    "P0001" => AppError::Rule(db.message().to_string()),
                    "22P02" | "22007" | "22008" | "22003" => AppError::BadRequest(format!("Invalid value: {}", db.message())),
                    _ => AppError::Internal(e.to_string()),
                }
            }
            _ => AppError::Internal(e.to_string()),
        }
    }
}
