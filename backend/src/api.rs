//! Route builder: one call registers the handler, enforces its permission,
//! and records it for the generated OpenAPI document. Docs can never drift from routing.
use crate::{auth::AuthUser, error::AppError, state::AppState};
use axum::{extract::Request, handler::Handler, middleware::{self, Next}, response::{IntoResponse, Response}, routing::{delete, get, patch, post, put, MethodRouter}, Router};
use serde_json::{json, Map, Value};

/// No token needed.
pub const PUBLIC: &str = "public";
/// Any signed-in user.
pub const AUTH: &str = "auth";

#[derive(Clone, Debug)]
pub struct Doc {
    pub method: &'static str,
    pub path: String,
    pub permission: &'static str,
    pub summary: String,
    pub tag: &'static str,
    pub body: Option<&'static str>,
    pub query: Option<&'static str>,
}

pub struct Api {
    pub router: Router<AppState>,
    pub docs: Vec<Doc>,
    tag: &'static str,
}

async fn guard(perm: &'static str, req: Request, next: Next) -> Response {
    if perm != PUBLIC {
        match req.extensions().get::<AuthUser>() {
            None => return AppError::Unauthorized("Please sign in".into()).into_response(),
            Some(u) => {
                if perm != AUTH && !u.can(perm) {
                    return AppError::Forbidden(format!("You do not have permission: {perm}")).into_response();
                }
            }
        }
    }
    next.run(req).await
}

impl Api {
    pub fn new(tag: &'static str) -> Self { Api { router: Router::new(), docs: vec![], tag } }
    /// Change the documentation section for routes added from here on.
    pub fn tag(mut self, tag: &'static str) -> Self { self.tag = tag; self }

    fn add(mut self, method: &'static str, path: &str, perm: &'static str, summary: &str, mr: MethodRouter<AppState>) -> Self {
        let mr = mr.layer(middleware::from_fn(move |req: Request, next: Next| guard(perm, req, next)));
        self.router = self.router.route(path, mr);
        self.docs.push(Doc { method, path: path.to_string(), permission: perm, summary: summary.to_string(), tag: self.tag, body: None, query: None });
        self
    }
    pub fn get<H, T>(self, path: &str, perm: &'static str, summary: &str, h: H) -> Self where H: Handler<T, AppState>, T: 'static { self.add("GET", path, perm, summary, get(h)) }
    pub fn post<H, T>(self, path: &str, perm: &'static str, summary: &str, h: H) -> Self where H: Handler<T, AppState>, T: 'static { self.add("POST", path, perm, summary, post(h)) }
    pub fn put<H, T>(self, path: &str, perm: &'static str, summary: &str, h: H) -> Self where H: Handler<T, AppState>, T: 'static { self.add("PUT", path, perm, summary, put(h)) }
    pub fn patch<H, T>(self, path: &str, perm: &'static str, summary: &str, h: H) -> Self where H: Handler<T, AppState>, T: 'static { self.add("PATCH", path, perm, summary, patch(h)) }
    pub fn delete<H, T>(self, path: &str, perm: &'static str, summary: &str, h: H) -> Self where H: Handler<T, AppState>, T: 'static { self.add("DELETE", path, perm, summary, delete(h)) }

    /// Attach an example request body (JSON text) to the most recently added route.
    pub fn body(mut self, example: &'static str) -> Self { if let Some(d) = self.docs.last_mut() { d.body = Some(example); } self }
    /// Document query parameters (comma separated) for the most recently added route.
    pub fn query(mut self, params: &'static str) -> Self { if let Some(d) = self.docs.last_mut() { d.query = Some(params); } self }

    pub fn merge(mut self, other: Api) -> Self {
        self.router = self.router.merge(other.router);
        self.docs.extend(other.docs);
        self
    }
}

/// Build an OpenAPI 3.0 document from the registered routes.
pub fn openapi(docs: &[Doc]) -> Value {
    let mut paths: Map<String, Value> = Map::new();
    for d in docs {
        let mut params = vec![];
        let oa_path = d.path.split('/').map(|seg| {
            if let Some(name) = seg.strip_prefix(':') {
                params.push(json!({"name": name, "in": "path", "required": true, "schema": {"type": "string"}}));
                format!("{{{name}}}")
            } else { seg.to_string() }
        }).collect::<Vec<_>>().join("/");
        if let Some(q) = d.query {
            for name in q.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                params.push(json!({"name": name, "in": "query", "required": false, "schema": {"type": "string"}}));
            }
        }
        let mut op = json!({
            "tags": [d.tag],
            "summary": d.summary,
            "operationId": format!("{}_{}", d.method.to_lowercase(), d.path.replace(['/', ':', '-'], "_").trim_matches('_')),
            "parameters": params,
            "x-permission": d.permission,
            "responses": {
                "200": {"description": "Success", "content": {"application/json": {"schema": {"type": "object"}}}},
                "401": {"$ref": "#/components/responses/Error"},
                "403": {"$ref": "#/components/responses/Error"},
                "422": {"$ref": "#/components/responses/Error"}
            }
        });
        if d.permission != PUBLIC { op["security"] = json!([{"bearerAuth": []}]); }
        if let Some(b) = d.body {
            let example: Value = serde_json::from_str(b).unwrap_or(Value::String(b.to_string()));
            op["requestBody"] = json!({"required": true, "content": {"application/json": {"schema": {"type": "object"}, "example": example}}});
        }
        let full = format!("/api/v1{oa_path}");
        paths.entry(full).or_insert_with(|| json!({})).as_object_mut().unwrap().insert(d.method.to_lowercase(), op);
    }
    json!({
        "openapi": "3.0.3",
        "info": {
            "title": "WEFT ERP API",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Bangladesh-first import ERP. All amounts are JSON numbers in BDT unless a currency field says otherwise. Dates are ISO `YYYY-MM-DD` in Bangladesh Standard Time. Lists return `{data,total,page,per_page}`; single records return `{data}`; errors return `{error:{code,message,details}}`."
        },
        "servers": [{"url": "/"}],
        "paths": paths,
        "components": {
            "securitySchemes": {"bearerAuth": {"type": "http", "scheme": "bearer", "bearerFormat": "JWT"}},
            "responses": {"Error": {"description": "Error", "content": {"application/json": {"schema": {"type": "object", "properties": {"error": {"type": "object", "properties": {"code": {"type": "string"}, "message": {"type": "string"}, "details": {"type": "array", "items": {"type": "object"}}}}}}}}}}
        }
    })
}

/// Render the route list as Markdown (written to `docs/API.md`).
pub fn markdown(docs: &[Doc]) -> String {
    let mut out = String::from("# WEFT ERP API reference\n\nBase URL: `/api/v1`. Send `Authorization: Bearer <access_token>` on every call except those marked public.\n\n## Conventions\n\n- Lists: `GET` returns `{\"data\": [...], \"total\": n, \"page\": 1, \"per_page\": 50}`. Query: `page`, `per_page` (max 500), `q` (search), plus the filters listed per route.\n- Single record: `{\"data\": {...}}`. Create returns HTTP 201.\n- Errors: `{\"error\": {\"code\", \"message\", \"details\": [{\"field\", \"message\"}]}}` with status 400, 401, 403, 404, 409 or 422.\n- Money is BDT with 2 decimals unless a `currency` field is present. Quantities have up to 4 decimals. Requests accept numbers or numeric strings.\n- Dates are `YYYY-MM-DD` (Bangladesh Standard Time, UTC+6). Timestamps are ISO-8601 UTC.\n- Permissions are `module.action`. A role may hold exact permissions, `module.*`, `*.read` or `*`.\n\n");
    let mut tags: Vec<&str> = vec![];
    for d in docs { if !tags.contains(&d.tag) { tags.push(d.tag); } }
    for tag in tags {
        out.push_str(&format!("## {tag}\n\n| Method | Path | Permission | Purpose |\n|---|---|---|---|\n"));
        for d in docs.iter().filter(|d| d.tag == tag) {
            out.push_str(&format!("| {} | `{}` | `{}` | {}{} |\n", d.method, d.path, d.permission, d.summary, d.query.map(|q| format!(" Query: `{q}`.")).unwrap_or_default()));
        }
        out.push('\n');
        for d in docs.iter().filter(|d| d.tag == tag && d.body.is_some()) {
            out.push_str(&format!("**{} `{}`** example body:\n\n```json\n{}\n```\n\n", d.method, d.path, d.body.unwrap()));
        }
    }
    out
}
