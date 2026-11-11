//! Thin client for the WEFT REST API: bearer auth, one-shot token refresh, uniform errors.
use gloo_net::http::{Method, RequestBuilder};
use gloo_storage::{LocalStorage, Storage};
use serde_json::Value;

const ACCESS: &str = "weft.access";
const REFRESH: &str = "weft.refresh";

#[derive(Clone, Debug, Default)]
pub struct ApiError { pub status: u16, pub code: String, pub message: String, pub fields: Vec<(String, String)> }
impl ApiError {
    pub fn field(&self, name: &str) -> Option<String> { self.fields.iter().find(|f| f.0 == name).map(|f| f.1.clone()) }
    /// A message that is safe to show to a user.
    pub fn friendly(&self) -> String {
        if !self.fields.is_empty() && self.code == "validation_failed" {
            return self.fields.iter().map(|(f, m)| format!("{}: {}", f.replace('_', " "), m)).collect::<Vec<_>>().join(" · ");
        }
        if self.status == 0 { return "Cannot reach the server. Check your connection and try again.".into(); }
        self.message.clone()
    }
}
pub type ApiResult = Result<Value, ApiError>;

pub fn token() -> Option<String> { LocalStorage::get::<String>(ACCESS).ok() }
pub fn refresh_token() -> Option<String> { LocalStorage::get::<String>(REFRESH).ok() }
pub fn store_tokens(access: &str, refresh: &str) { let _ = LocalStorage::set(ACCESS, access); let _ = LocalStorage::set(REFRESH, refresh); }
pub fn clear_tokens() { LocalStorage::delete(ACCESS); LocalStorage::delete(REFRESH); }

fn base() -> String {
    // The API is served from the same origin in production; `?api=` overrides for development.
    let w = web_sys::window().unwrap();
    if let Ok(s) = w.location().search() { if let Some(p) = s.trim_start_matches('?').split('&').find_map(|kv| kv.strip_prefix("api=")) { return p.to_string(); } }
    if let Ok(Some(v)) = w.local_storage().map(|s| s.and_then(|s| s.get_item("weft.api").ok().flatten())) { return v; }
    String::new()
}

async fn send(method: Method, path: &str, body: Option<&Value>, auth: bool) -> Result<(u16, Value), ApiError> {
    let url = format!("{}/api/v1{}", base(), path);
    let mut rb = RequestBuilder::new(&url).method(method).header("Accept", "application/json");
    if auth { if let Some(t) = token() { rb = rb.header("Authorization", &format!("Bearer {t}")); } }
    let req = match body { Some(b) => rb.header("Content-Type", "application/json").body(b.to_string()), None => rb.build() }
        .map_err(|e| ApiError { message: e.to_string(), ..Default::default() })?;
    let resp = req.send().await.map_err(|e| ApiError { status: 0, code: "network".into(), message: e.to_string(), fields: vec![] })?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    let json: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    Ok((status, json))
}

fn to_error(status: u16, v: &Value) -> ApiError {
    let e = &v["error"];
    ApiError {
        status,
        code: e["code"].as_str().unwrap_or("error").to_string(),
        message: e["message"].as_str().map(str::to_string).unwrap_or_else(|| match status { 401 => "Please sign in again".into(), 403 => "You do not have permission to do this".into(), 404 => "Not found".into(), 413 => "That is too large to send".into(), _ => format!("Request failed ({status})") }),
        fields: e["details"].as_array().map(|a| a.iter().filter_map(|d| Some((d["field"].as_str()?.to_string(), d["message"].as_str()?.to_string()))).collect()).unwrap_or_default(),
    }
}

async fn try_refresh() -> bool {
    let Some(r) = refresh_token() else { return false };
    match send(Method::POST, "/auth/refresh", Some(&serde_json::json!({"refresh_token": r})), false).await {
        Ok((200, v)) => { store_tokens(v["data"]["access_token"].as_str().unwrap_or(""), v["data"]["refresh_token"].as_str().unwrap_or("")); true }
        _ => false,
    }
}

pub async fn request(method: Method, path: &str, body: Option<Value>) -> ApiResult {
    let (mut status, mut json) = send(method.clone(), path, body.as_ref(), true).await?;
    if status == 401 && !path.starts_with("/auth/") && try_refresh().await {
        let r = send(method, path, body.as_ref(), true).await?; status = r.0; json = r.1;
    }
    if status == 401 && !path.starts_with("/auth/") { crate::state::session_expired(); }
    if (200..300).contains(&status) { Ok(json) } else { Err(to_error(status, &json)) }
}
pub async fn get(path: &str) -> ApiResult { request(Method::GET, path, None).await }
pub async fn post(path: &str, body: Value) -> ApiResult { request(Method::POST, path, Some(body)).await }
pub async fn put(path: &str, body: Value) -> ApiResult { request(Method::PUT, path, Some(body)).await }
pub async fn delete(path: &str) -> ApiResult { request(Method::DELETE, path, None).await }
/// Unauthenticated POST (login, setup).
pub async fn post_public(path: &str, body: Value) -> ApiResult {
    let (s, j) = send(Method::POST, path, Some(&body), false).await?;
    if (200..300).contains(&s) { Ok(j) } else { Err(to_error(s, &j)) }
}
pub fn qs(pairs: &[(&str, String)]) -> String {
    let parts: Vec<String> = pairs.iter().filter(|(_, v)| !v.is_empty()).map(|(k, v)| format!("{}={}", k, js_sys::encode_uri_component(v))).collect();
    if parts.is_empty() { String::new() } else { format!("?{}", parts.join("&")) }
}
