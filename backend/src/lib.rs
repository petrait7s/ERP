//! WEFT ERP backend: Bangladesh-first import ERP on Axum, SQLx and PostgreSQL.
pub mod api;
pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod modules;
pub mod resource;
pub mod seed;
pub mod state;

use axum::{extract::State, http::{header, HeaderValue, Method}, middleware, routing::get, Json, Router};
use serde_json::{json, Value};
use state::AppState;
use std::sync::Arc;
use tower_http::{cors::{AllowOrigin, CorsLayer}, limit::RequestBodyLimitLayer, set_header::SetResponseHeaderLayer, trace::TraceLayer};

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// Every module's routes with their documentation.
pub fn all_routes() -> api::Api {
    modules::admin::api()
        .merge(modules::masters::api())
        .merge(modules::accounting::api())
        .merge(modules::finance::api())
        .merge(modules::inventory::api())
        .merge(modules::purchase::api())
        .merge(modules::import::api())
        .merge(modules::customs::api())
        .merge(modules::landed::api())
        .merge(modules::sales::api())
        .merge(modules::hr::api())
        .merge(modules::tax::api())
        .merge(modules::approvals::api())
        .merge(modules::notifications::api())
        .merge(modules::reports::api())
}

async fn health(State(st): State<AppState>) -> Json<Value> {
    let db_ok = sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(&st.db).await.is_ok();
    Json(json!({"status": if db_ok { "ok" } else { "degraded" }, "database": db_ok, "version": env!("CARGO_PKG_VERSION")}))
}

pub fn build_app(state: AppState) -> Router {
    let routes = all_routes();
    let spec = Arc::new(api::openapi(&routes.docs));
    let cors = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::PATCH, Method::DELETE, Method::OPTIONS])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE, header::ACCEPT_LANGUAGE])
        .allow_origin(if state.cfg.cors_origins.is_empty() { AllowOrigin::any() } else { AllowOrigin::list(state.cfg.cors_origins.iter().filter_map(|o| o.parse::<HeaderValue>().ok())) });
    let v1 = routes.router
        .route("/openapi.json", get(move || { let s = spec.clone(); async move { Json((*s).clone()) } }))
        .layer(middleware::from_fn_with_state(state.clone(), auth::attach_user));
    let mut app = Router::new()
        .route("/health", get(health))
        .nest("/api/v1", v1);
    if let Some(dir) = &state.cfg.static_dir {
        // The web app is a single-page application: unknown paths fall back to index.html.
        let index = std::path::Path::new(dir).join("index.html");
        app = app.fallback_service(tower_http::services::ServeDir::new(dir).fallback(tower_http::services::ServeFile::new(index)));
    }
    app
        .layer(RequestBodyLimitLayer::new(2 * 1024 * 1024))
        .layer(SetResponseHeaderLayer::if_not_present(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")))
        .layer(SetResponseHeaderLayer::if_not_present(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY")))
        .layer(SetResponseHeaderLayer::if_not_present(header::CACHE_CONTROL, HeaderValue::from_static("no-store")))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
