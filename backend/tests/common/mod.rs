//! Test harness: every test gets its own freshly migrated PostgreSQL database and a real HTTP app.
#![allow(dead_code)]
use axum::{body::Body, http::{header, Method, Request}, Router};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sqlx::{postgres::PgPoolOptions, PgPool};
use std::sync::Arc;
use tower::ServiceExt;
use weft_erp::{build_app, config::Config, state::AppState, MIGRATOR};

pub struct TestApp { pub state: AppState, pub app: Router, pub db: PgPool, pub token: String, pub company: String, name: String }

fn admin_url() -> String { std::env::var("TEST_ADMIN_URL").unwrap_or_else(|_| "postgres://erp:erp@localhost:5432/postgres".into()) }
fn db_url(name: &str) -> String { admin_url().rsplit_once('/').map(|(h, _)| format!("{h}/{name}")).unwrap() }

pub async fn new_app() -> TestApp {
    let name = format!("erp_t_{}", uuid::Uuid::new_v4().simple());
    let admin = PgPoolOptions::new().max_connections(1).connect(&admin_url()).await.expect("admin db");
    sqlx::query(&format!("CREATE DATABASE {name}")).execute(&admin).await.unwrap();
    let db = PgPoolOptions::new().max_connections(10).connect(&db_url(&name)).await.unwrap();
    MIGRATOR.run(&db).await.unwrap();
    let cfg = Config { database_url: db_url(&name), bind_addr: "127.0.0.1:0".into(), jwt_secret: "test-secret-test-secret-test-secret-1234".into(), access_ttl_minutes: 60, refresh_ttl_days: 1, cors_origins: vec![], run_jobs: false, static_dir: None };
    let state = AppState { db: db.clone(), cfg: Arc::new(cfg) };
    let app = build_app(state.clone());
    let mut t = TestApp { state, app, db, token: String::new(), company: String::new(), name };
    let (s, b) = t.raw(Method::POST, "/api/v1/setup", None, Some(json!({"company_code": "PTC", "company_name": "PETRA Trade Corporation", "bin": "000123456-0101", "admin_email": "owner@example.com", "admin_password": "Str0ngPassw0rd", "admin_name": "Owner"}))).await;
    assert_eq!(s, 201, "setup failed: {b}");
    t.company = b["data"]["company_id"].as_str().unwrap().to_string();
    t.token = t.login("owner@example.com", "Str0ngPassw0rd").await;
    t
}

impl TestApp {
    pub async fn raw(&self, method: Method, path: &str, token: Option<&str>, body: Option<Value>) -> (u16, Value) {
        let mut req = Request::builder().method(method).uri(path);
        if let Some(t) = token { req = req.header(header::AUTHORIZATION, format!("Bearer {t}")); }
        let body = match body { Some(b) => { req = req.header(header::CONTENT_TYPE, "application/json"); Body::from(b.to_string()) }, None => Body::empty() };
        let resp = self.app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
        let status = resp.status().as_u16();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }
    pub async fn login(&self, email: &str, pw: &str) -> String {
        let (s, b) = self.raw(Method::POST, "/api/v1/auth/login", None, Some(json!({"email": email, "password": pw}))).await;
        assert_eq!(s, 200, "login failed: {b}");
        b["data"]["access_token"].as_str().unwrap().to_string()
    }
    pub async fn get(&self, path: &str) -> Value { let (s, b) = self.raw(Method::GET, &format!("/api/v1{path}"), Some(&self.token), None).await; assert_eq!(s, 200, "GET {path}: {b}"); b }
    pub async fn post(&self, path: &str, body: Value) -> Value { let (s, b) = self.raw(Method::POST, &format!("/api/v1{path}"), Some(&self.token), Some(body)).await; assert!(s == 200 || s == 201, "POST {path} -> {s}: {b}"); b }
    pub async fn put(&self, path: &str, body: Value) -> Value { let (s, b) = self.raw(Method::PUT, &format!("/api/v1{path}"), Some(&self.token), Some(body)).await; assert_eq!(s, 200, "PUT {path}: {b}"); b }
    /// Expect a failure and return (status, error code, message).
    pub async fn fail(&self, method: Method, path: &str, body: Option<Value>) -> (u16, String, String) {
        let (s, b) = self.raw(method, &format!("/api/v1{path}"), Some(&self.token), body).await;
        assert!(s >= 400, "expected an error but got {s}: {b}");
        (s, b["error"]["code"].as_str().unwrap_or("").to_string(), b["error"]["message"].as_str().unwrap_or("").to_string())
    }
    pub async fn id(&self, path: &str, body: Value) -> String { self.post(path, body).await["data"]["id"].as_str().unwrap().to_string() }
    pub async fn cleanup(self) {
        self.db.close().await;
        let admin = PgPoolOptions::new().max_connections(1).connect(&admin_url()).await.unwrap();
        let _ = sqlx::query(&format!("DROP DATABASE IF EXISTS {} WITH (FORCE)", self.name)).execute(&admin).await;
    }
    pub async fn acct(&self, key: &str) -> String { sqlx::query_scalar::<_, uuid::Uuid>("SELECT id FROM accounts WHERE system_key = $1").bind(key).fetch_one(&self.db).await.unwrap().to_string() }
    /// Ledger balance (debit - credit) of a system account.
    pub async fn bal(&self, key: &str) -> rust_decimal::Decimal {
        sqlx::query_scalar("SELECT coalesce(sum(l.debit - l.credit), 0) FROM journal_lines l JOIN accounts a ON a.id = l.account_id WHERE a.system_key = $1").bind(key).fetch_one(&self.db).await.unwrap()
    }
    /// Add a user with the given role and return their token.
    pub async fn user_with_role(&self, email: &str, role: &str) -> String {
        self.post("/users", json!({"email": email, "full_name": email, "password": "Str0ngPassw0rd", "role": role})).await;
        self.login(email, "Str0ngPassw0rd").await
    }
}

pub fn dec(v: &Value) -> rust_decimal::Decimal {
    match v { Value::Number(n) => n.to_string().parse().unwrap(), Value::String(s) => s.parse().unwrap(), other => panic!("not a number: {other}") }
}
pub fn d(s: &str) -> rust_decimal::Decimal { s.parse().unwrap() }
