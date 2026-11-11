mod common;
use common::*;
use axum::http::Method;
use serde_json::json;

#[tokio::test]
async fn authentication_and_session_rules() {
    let t = new_app().await;
    // no token
    let (s, b) = t.raw(Method::GET, "/api/v1/products", None, None).await;
    assert_eq!(s, 401); assert_eq!(b["error"]["code"], "unauthorized");
    // garbage token
    let (s, _) = t.raw(Method::GET, "/api/v1/products", Some("not.a.jwt"), None).await;
    assert_eq!(s, 401);
    // wrong password gives the same message as an unknown user
    let (s1, b1) = t.raw(Method::POST, "/api/v1/auth/login", None, Some(json!({"email": "owner@example.com", "password": "wrong-password-1"}))).await;
    let (s2, b2) = t.raw(Method::POST, "/api/v1/auth/login", None, Some(json!({"email": "nobody@example.com", "password": "wrong-password-1"}))).await;
    assert_eq!((s1, s2), (401, 401));
    assert_eq!(b1["error"]["message"], b2["error"]["message"]);
    // setup cannot be run twice
    let (s, _) = t.raw(Method::POST, "/api/v1/setup", None, Some(json!({"company_code": "X", "company_name": "X", "admin_email": "x@example.com", "admin_password": "Str0ngPassw0rd", "admin_name": "X"}))).await;
    assert_eq!(s, 409);
    // password strength
    let (s, b) = t.raw(Method::POST, "/api/v1/users", Some(&t.token), Some(json!({"email": "weak@example.com", "full_name": "Weak", "password": "short", "role": "viewer"}))).await;
    assert_eq!(s, 422); assert_eq!(b["error"]["details"][0]["field"], "password");

    // refresh tokens rotate and work exactly once
    let (_, login) = t.raw(Method::POST, "/api/v1/auth/login", None, Some(json!({"email": "owner@example.com", "password": "Str0ngPassw0rd"}))).await;
    let refresh = login["data"]["refresh_token"].as_str().unwrap();
    let (s, r) = t.raw(Method::POST, "/api/v1/auth/refresh", None, Some(json!({"refresh_token": refresh}))).await;
    assert_eq!(s, 200); assert_ne!(r["data"]["refresh_token"].as_str().unwrap(), refresh);
    let (s, _) = t.raw(Method::POST, "/api/v1/auth/refresh", None, Some(json!({"refresh_token": refresh}))).await;
    assert_eq!(s, 401, "a used refresh token is dead");

    // lock-out after repeated failures, then even the right password is refused
    t.user_with_role("clerk@example.com", "viewer").await;
    for _ in 0..5 { t.raw(Method::POST, "/api/v1/auth/login", None, Some(json!({"email": "clerk@example.com", "password": "bad-password-99"}))).await; }
    let (s, b) = t.raw(Method::POST, "/api/v1/auth/login", None, Some(json!({"email": "clerk@example.com", "password": "Str0ngPassw0rd"}))).await;
    assert_eq!(s, 401); assert!(b["error"]["message"].as_str().unwrap().contains("Too many"));

    // passwords are stored as Argon2 hashes, never in the clear
    let h: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE email = 'owner@example.com'").fetch_one(&t.db).await.unwrap();
    assert!(h.starts_with("$argon2"));

    // deactivated users lose access immediately, even with a valid token
    let tok = t.user_with_role("temp@example.com", "viewer").await;
    let (s, _) = t.raw(Method::GET, "/api/v1/products", Some(&tok), None).await; assert_eq!(s, 200);
    let uid: String = sqlx::query_scalar::<_, uuid::Uuid>("SELECT id FROM users WHERE email = 'temp@example.com'").fetch_one(&t.db).await.unwrap().to_string();
    t.put(&format!("/users/{uid}"), json!({"is_active": false})).await;
    let (s, _) = t.raw(Method::GET, "/api/v1/products", Some(&tok), None).await; assert_eq!(s, 401);
    t.cleanup().await;
}

#[tokio::test]
async fn role_based_access_control() {
    let t = new_app().await;
    let supplier = t.id("/suppliers", json!({"name": "Local Steel", "supplier_type": "local"})).await;
    let product = t.id("/products", json!({"sku": "P1", "name": "Product", "sale_price": 100})).await;
    let wh = t.user_with_role("store@example.com", "warehouse").await;
    let sales = t.user_with_role("sales@example.com", "sales").await;
    let acct = t.user_with_role("acct@example.com", "accountant").await;
    let viewer = t.user_with_role("view@example.com", "viewer").await;

    let po = json!({"supplier_id": supplier, "lines": [{"product_id": product, "qty": 1, "unit_price": 10}]});
    // warehouse staff cannot create purchase orders or journals; sales cannot touch the ledger
    assert_eq!(t.raw(Method::POST, "/api/v1/purchase-orders", Some(&wh), Some(po.clone())).await.0, 403);
    assert_eq!(t.raw(Method::GET, "/api/v1/journals", Some(&sales), None).await.0, 403);
    assert_eq!(t.raw(Method::GET, "/api/v1/payroll-runs", Some(&sales), None).await.0, 403);
    assert_eq!(t.raw(Method::GET, "/api/v1/audit-logs", Some(&acct), None).await.0, 403, "audit log is admin only");
    assert_eq!(t.raw(Method::POST, "/api/v1/users", Some(&acct), Some(json!({"email": "x@example.com", "full_name": "X", "password": "Str0ngPassw0rd", "role": "owner"}))).await.0, 403);
    // viewers read but never write
    assert_eq!(t.raw(Method::GET, "/api/v1/products", Some(&viewer), None).await.0, 200);
    assert_eq!(t.raw(Method::POST, "/api/v1/products", Some(&viewer), Some(json!({"sku": "Z", "name": "Z"}))).await.0, 403);
    assert_eq!(t.raw(Method::POST, "/api/v1/sales-orders", Some(&viewer), Some(json!({}))).await.0, 403);
    // accountant can read ledger and post journals, sales can read products
    assert_eq!(t.raw(Method::GET, "/api/v1/journals", Some(&acct), None).await.0, 200);
    assert_eq!(t.raw(Method::GET, "/api/v1/products", Some(&sales), None).await.0, 200);
    // nobody but a system admin can add a company
    assert_eq!(t.raw(Method::POST, "/api/v1/companies", Some(&acct), Some(json!({"code": "X", "name": "X"}))).await.0, 403);
    // an admin cannot grant themselves owner-level powers
    let admin = t.user_with_role("admin@example.com", "admin").await;
    assert_eq!(t.raw(Method::POST, "/api/v1/roles", Some(&admin), Some(json!({"code": "god", "name": "God", "permissions": ["*"]}))).await.0, 403);

    // custom role with narrow permissions
    t.post("/roles", json!({"code": "store_keeper", "name": "Store Keeper", "permissions": ["inventory.read", "catalog.read"]})).await;
    let keeper = t.user_with_role("keeper@example.com", "store_keeper").await;
    assert_eq!(t.raw(Method::GET, "/api/v1/stock", Some(&keeper), None).await.0, 200);
    assert_eq!(t.raw(Method::POST, "/api/v1/stock/transfers", Some(&keeper), Some(json!({}))).await.0, 403);
    // unknown permission strings are refused when defining roles
    let (s, _, _) = t.fail(Method::POST, "/roles", Some(json!({"code": "bad", "name": "Bad", "permissions": ["rocket.launch"]}))).await;
    assert_eq!(s, 422);

    // dashboard only shows what the role may see
    let d = t.raw(Method::GET, "/api/v1/dashboard", Some(&sales), None).await.1;
    assert!(d["data"]["sales"].is_object() && d["data"]["finance"].is_null() && d["data"]["hr"].is_null());
    let d = t.raw(Method::GET, "/api/v1/dashboard", Some(&acct), None).await.1;
    assert!(d["data"]["finance"].is_object() && d["data"]["hr"].is_object());
    t.cleanup().await;
}

#[tokio::test]
async fn companies_are_isolated_and_branch_users_are_scoped() {
    let t = new_app().await;
    let product_a = t.id("/products", json!({"sku": "A-1", "name": "Alpha product"})).await;
    // second company, owned by the same system administrator
    let c2 = t.post("/companies", json!({"code": "SEC", "name": "Second Company"})).await;
    let c2_id = c2["data"]["id"].as_str().unwrap().to_string();
    let (s, b) = t.raw(Method::POST, "/api/v1/auth/switch-company", Some(&t.token), Some(json!({"company_id": c2_id}))).await;
    assert_eq!(s, 200);
    let tok2 = b["data"]["access_token"].as_str().unwrap();
    // company 2 sees none of company 1's data and cannot fetch it by id
    let (_, list) = t.raw(Method::GET, "/api/v1/products", Some(tok2), None).await;
    assert_eq!(list["total"], 0);
    assert_eq!(t.raw(Method::GET, &format!("/api/v1/products/{product_a}"), Some(tok2), None).await.0, 404);
    assert_eq!(t.raw(Method::PUT, &format!("/api/v1/products/{product_a}"), Some(tok2), Some(json!({"name": "Hijacked"}))).await.0, 404);
    assert_eq!(t.raw(Method::DELETE, &format!("/api/v1/products/{product_a}"), Some(tok2), None).await.0, 404);
    // the same SKU may exist in both companies
    assert_eq!(t.raw(Method::POST, "/api/v1/products", Some(tok2), Some(json!({"sku": "A-1", "name": "Other company's A-1"}))).await.0, 201);
    // a foreign id inside a request body is rejected like a missing record
    let (s, _) = t.raw(Method::POST, "/api/v1/products", Some(tok2), Some(json!({"sku": "B-1", "name": "B", "category_id": product_a}))).await;
    assert_eq!(s, 422);
    // each company has its own chart of accounts, numbering and roles
    let coa: i64 = sqlx::query_scalar("SELECT count(*) FROM accounts WHERE company_id = $1::uuid").bind(&c2_id).fetch_one(&t.db).await.unwrap();
    assert!(coa > 40);
    // switching to a company you do not belong to fails
    let tok_clerk = t.user_with_role("clerk@example.com", "viewer").await;
    let (s, _) = t.raw(Method::POST, "/api/v1/auth/switch-company", Some(&tok_clerk), Some(json!({"company_id": c2_id}))).await;
    assert_eq!(s, 403);

    // branch-scoped user is limited to their own branch
    let br = t.id("/branches", json!({"code": "CTG", "name": "Chattogram"})).await;
    let br2 = t.id("/branches", json!({"code": "SYL", "name": "Sylhet"})).await;
    t.post("/users", json!({"email": "ctg@example.com", "full_name": "CTG Sales", "password": "Str0ngPassw0rd", "role": "sales", "branch_id": br})).await;
    let ctg = t.login("ctg@example.com", "Str0ngPassw0rd").await;
    let cust = t.id("/customers", json!({"name": "Buyer"})).await;
    let wh = t.get("/warehouses").await["data"][0]["id"].as_str().unwrap().to_string();
    let prod = t.id("/products", json!({"sku": "S-1", "name": "S", "sale_price": 10})).await;
    let (s, b) = t.raw(Method::POST, "/api/v1/sales-orders", Some(&ctg), Some(json!({"customer_id": cust, "warehouse_id": wh, "lines": [{"product_id": prod, "qty": 1}]}))).await;
    assert_eq!(s, 201);
    assert_eq!(b["data"]["branch_id"], json!(br), "documents are stamped with the user's branch");
    let (s, _) = t.raw(Method::POST, "/api/v1/sales-orders", Some(&ctg), Some(json!({"customer_id": cust, "warehouse_id": wh, "branch_id": br2, "lines": [{"product_id": prod, "qty": 1}]}))).await;
    assert_eq!(s, 403, "cannot post into another branch");
    // an owner sees the order; the CTG user sees only CTG orders
    assert_eq!(t.get("/sales-orders").await["total"], 1);
    t.post("/sales-orders", json!({"customer_id": cust, "warehouse_id": wh, "branch_id": br2, "lines": [{"product_id": prod, "qty": 1}]})).await;
    assert_eq!(t.get("/sales-orders").await["total"], 2);
    let (_, mine) = t.raw(Method::GET, "/api/v1/sales-orders", Some(&ctg), None).await;
    assert_eq!(mine["total"], 1);
    t.cleanup().await;
}

#[tokio::test]
async fn input_validation_and_error_format() {
    let t = new_app().await;
    // unknown fields, bad types and missing required fields are all reported per field
    let (s, c, _) = t.fail(Method::POST, "/products", Some(json!({"name": "No sku", "sale_price": "abc", "rogue": 1}))).await;
    assert_eq!((s, c.as_str()), (422, "validation_failed"));
    let (_, b) = t.raw(Method::POST, "/api/v1/products", Some(&t.token), Some(json!({"name": "No sku", "sale_price": "abc", "rogue": 1}))).await;
    let fields: Vec<&str> = b["error"]["details"].as_array().unwrap().iter().map(|e| e["field"].as_str().unwrap()).collect();
    assert!(fields.contains(&"sku") && fields.contains(&"sale_price") && fields.contains(&"rogue"));
    // negative money and out-of-range values
    assert_eq!(t.fail(Method::POST, "/products", Some(json!({"sku": "N", "name": "N", "sale_price": -5}))).await.0, 422);
    // duplicates conflict
    t.post("/products", json!({"sku": "DUP", "name": "One"})).await;
    assert_eq!(t.fail(Method::POST, "/products", Some(json!({"sku": "DUP", "name": "Two"}))).await.0, 409);
    // malformed JSON and malformed ids
    let (s, _) = t.raw(Method::POST, "/api/v1/products", Some(&t.token), None).await; assert!(s == 400 || s == 415 || s == 422);
    assert_eq!(t.raw(Method::GET, "/api/v1/products/not-a-uuid", Some(&t.token), None).await.0, 400);
    assert_eq!(t.raw(Method::GET, "/api/v1/products/00000000-0000-0000-0000-000000000000", Some(&t.token), None).await.0, 404);
    // SQL metacharacters are data, not code
    t.post("/products", json!({"sku": "X'; DROP TABLE products;--", "name": "Bobby"})).await;
    let n = t.get("/products?q=%27%3B%20DROP").await;
    assert_eq!(n["total"], 1);
    assert_eq!(t.get("/products").await["total"], 2);
    // unknown sort columns are ignored rather than injected
    assert_eq!(t.raw(Method::GET, "/api/v1/products?sort=name;drop%20table%20x", Some(&t.token), None).await.0, 200);
    // pagination is bounded
    let p = t.get("/products?per_page=9999").await; assert_eq!(p["per_page"], 500);
    // Bangla text and Unicode survive a round trip
    let id = t.id("/products", json!({"sku": "BN-1", "name": "LED Panel", "name_bn": "এলইডি প্যানেল লাইট"})).await;
    assert_eq!(t.get(&format!("/products/{id}")).await["data"]["name_bn"], "এলইডি প্যানেল লাইট");
    assert_eq!(t.get("/products?q=%E0%A6%8F%E0%A6%B2").await["total"], 1);
    // security headers are present
    let resp = {
        use tower::ServiceExt;
        t.app.clone().oneshot(axum::http::Request::builder().uri("/health").body(axum::body::Body::empty()).unwrap()).await.unwrap()
    };
    assert_eq!(resp.headers()["x-content-type-options"], "nosniff");
    assert_eq!(resp.headers()["x-frame-options"], "DENY");
    // oversized body is refused
    let big = "x".repeat(3 * 1024 * 1024);
    let (s, _) = t.raw(Method::POST, "/api/v1/products", Some(&t.token), Some(json!({"sku": "BIG", "name": big}))).await;
    assert_eq!(s, 413);
    t.cleanup().await;
}

#[tokio::test]
async fn openapi_document_covers_every_route() {
    let t = new_app().await;
    let (s, spec) = t.raw(Method::GET, "/api/v1/openapi.json", None, None).await;
    assert_eq!(s, 200);
    assert_eq!(spec["openapi"], "3.0.3");
    let paths = spec["paths"].as_object().unwrap();
    assert!(paths.len() >= 130, "documented paths: {}", paths.len());
    for must in ["/api/v1/auth/login", "/api/v1/purchase-orders", "/api/v1/shipments/{id}/depart", "/api/v1/bills-of-entry", "/api/v1/tax/vat-return", "/api/v1/payroll-runs/{id}/post"] {
        assert!(paths.contains_key(must), "missing {must}");
    }
    // every protected operation declares the permission it needs
    assert_eq!(paths["/api/v1/journals"]["post"]["x-permission"], "finance.post");
    assert!(paths["/api/v1/auth/login"]["post"]["security"].is_null());
    t.cleanup().await;
}
