use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use weft_erp::{build_app, config::Config, state::AppState, MIGRATOR};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,tower_http=info,sqlx=warn".into())).init();
    let cfg = match Config::from_env() {
        Ok(c) => c,
        Err(e) => { eprintln!("configuration error: {e}"); std::process::exit(2); }
    };
    if std::env::args().any(|a| a == "--print-api-docs") {
        print!("{}", weft_erp::api::markdown(&weft_erp::all_routes().docs));
        return;
    }
    let db = PgPoolOptions::new().max_connections(20).connect(&cfg.database_url).await.expect("cannot connect to PostgreSQL");
    MIGRATOR.run(&db).await.expect("database migration failed");
    let state = AppState { db, cfg: Arc::new(cfg.clone()) };
    let app = build_app(state);
    let listener = tokio::net::TcpListener::bind(&cfg.bind_addr).await.expect("cannot bind address");
    tracing::info!("WEFT ERP listening on http://{}", cfg.bind_addr);
    axum::serve(listener, app).with_graceful_shutdown(async { let _ = tokio::signal::ctrl_c().await; }).await.expect("server error");
}
