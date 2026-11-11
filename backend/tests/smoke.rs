mod common;
use common::*;
use axum::http::Method;

/// Call every parameter-free GET route of the documented API on a fresh company.
/// Catches SQL mistakes in list and report queries that only show up when they actually run.
#[tokio::test]
async fn every_list_and_report_endpoint_answers() {
    let t = new_app().await;
    let (_, spec) = t.raw(Method::GET, "/api/v1/openapi.json", None, None).await;
    let mut called = 0;
    for (path, ops) in spec["paths"].as_object().unwrap() {
        if path.contains('{') || ops.get("get").is_none() || path.ends_with("openapi.json") { continue; }
        let query = match path.as_str() {
            "/api/v1/reports/general-ledger" => continue,
            "/api/v1/tax/vat-return" | "/api/v1/attendance/summary" => "?year=2026&month=12",
            "/api/v1/search" => "?q=ab",
            _ => "?per_page=5",
        };
        let (s, b) = t.raw(Method::GET, &format!("{path}{query}"), Some(&t.token), None).await;
        assert_eq!(s, 200, "GET {path}{query} -> {s}: {b}");
        called += 1;
    }
    assert!(called >= 60, "only {called} endpoints exercised");
    t.cleanup().await;
}
