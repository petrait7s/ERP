mod common;
use common::*;
use axum::http::Method;
use serde_json::json;

async fn funded_bank(t: &TestApp, amount: i64) -> String {
    let bank = t.id("/bank-accounts", json!({"code": "EBL", "name": "Eastern Bank", "currency": "BDT"})).await;
    let gl: String = t.get(&format!("/bank-accounts/{bank}")).await["data"]["account_id"].as_str().unwrap().into();
    let cap = t.acct("capital").await;
    t.post("/journals", json!({"entry_date": "2026-07-01", "memo": "Capital", "lines": [{"account_id": gl, "debit": amount}, {"account_id": cap, "credit": amount}]})).await;
    bank
}

#[tokio::test]
async fn ledger_is_balanced_immutable_and_period_locked() {
    let t = new_app().await;
    let bank = funded_bank(&t, 1_000_000).await;
    let gl: String = t.get(&format!("/bank-accounts/{bank}")).await["data"]["account_id"].as_str().unwrap().into();
    let cap = t.acct("capital").await;

    // unbalanced journal is rejected by the API...
    let (s, _, m) = t.fail(Method::POST, "/journals", Some(json!({"entry_date": "2026-07-02", "memo": "bad", "lines": [{"account_id": gl, "debit": 100}, {"account_id": cap, "credit": 90}]}))).await;
    assert_eq!(s, 422); assert!(m.contains("balance"), "{m}");
    // ...and by the database itself even when the application is bypassed
    let mut tx = t.db.begin().await.unwrap();
    let company: uuid::Uuid = sqlx::query_scalar("SELECT id FROM companies").fetch_one(&mut *tx).await.unwrap();
    let e: uuid::Uuid = sqlx::query_scalar("INSERT INTO journal_entries (company_id, entry_no, entry_date, source_type) VALUES ($1,'X-1','2026-07-02','manual') RETURNING id").bind(company).fetch_one(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO journal_lines (entry_id, company_id, line_no, account_id, debit, credit) VALUES ($1,$2,1,$3,100,0)").bind(e).bind(company).bind(uuid::Uuid::parse_str(&gl).unwrap()).execute(&mut *tx).await.unwrap();
    assert!(tx.commit().await.is_err(), "deferred constraint must block an unbalanced entry at commit");

    // posted lines cannot be edited or deleted, not even by SQL
    assert!(sqlx::query("UPDATE journal_lines SET debit = debit + 1").execute(&t.db).await.is_err());
    assert!(sqlx::query("DELETE FROM journal_lines").execute(&t.db).await.is_err());
    assert!(sqlx::query("DELETE FROM journal_entries").execute(&t.db).await.is_err());
    assert!(sqlx::query("UPDATE audit_logs SET action = 'x'").execute(&t.db).await.is_err(), "audit log is append-only");
    // group accounts and system accounts are protected
    let grp = t.acct("bank_parent").await;
    assert_eq!(t.fail(Method::POST, "/journals", Some(json!({"entry_date": "2026-07-02", "memo": "x", "lines": [{"account_id": grp, "debit": 5}, {"account_id": cap, "credit": 5}]}))).await.0, 422);
    let sys = t.get("/accounts?q=Retained").await["data"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(t.fail(Method::DELETE, &format!("/accounts/{sys}"), None).await.0, 422, "system accounts cannot be deleted");

    // reversal keeps the original and mirrors it
    let j = t.post("/journals", json!({"entry_date": "2026-07-05", "memo": "Wrong entry", "lines": [{"account_id": gl, "debit": 500}, {"account_id": cap, "credit": 500}]})).await;
    let jid = j["data"]["id"].as_str().unwrap();
    let before = t.bal("capital").await;
    let r = t.post(&format!("/journals/{jid}/reverse"), json!({"reason": "typo", "date": "2026-07-06"})).await;
    assert_eq!(r["data"]["source_type"], "reversal");
    assert_eq!(t.bal("capital").await, before + d("500"));
    assert_eq!(t.get(&format!("/journals/{jid}")).await["data"]["status"], "reversed");
    assert_eq!(t.fail(Method::POST, &format!("/journals/{jid}/reverse"), Some(json!({"reason": "again"}))).await.0, 422);

    // closed periods refuse new postings; reopening restores them
    let p = t.get("/fiscal-periods").await;
    let jul = p["data"].as_array().unwrap().iter().find(|x| x["start_date"].as_str().unwrap().ends_with("-07-01")).cloned().unwrap();
    let pid = jul["id"].as_str().unwrap();
    // must close in order: the first period of the year is July
    t.post(&format!("/fiscal-periods/{pid}/close"), json!({})).await;
    let (s, _, m) = t.fail(Method::POST, "/journals", Some(json!({"entry_date": "2026-07-10", "memo": "late", "lines": [{"account_id": gl, "debit": 1}, {"account_id": cap, "credit": 1}]}))).await;
    assert_eq!(s, 422); assert!(m.contains("closed"), "{m}");
    t.post(&format!("/fiscal-periods/{pid}/reopen"), json!({})).await;
    t.post("/journals", json!({"entry_date": "2026-07-10", "memo": "ok now", "lines": [{"account_id": gl, "debit": 1}, {"account_id": cap, "credit": 1}]})).await;
    assert_eq!(t.get("/reports/trial-balance").await["data"]["balanced"], true);
    t.cleanup().await;
}

#[tokio::test]
async fn stock_valuation_negative_stock_and_concurrency() {
    let t = new_app().await;
    let p = t.id("/products", json!({"sku": "W-1", "name": "Widget", "sale_price": 150, "min_sale_price": 100})).await;
    let wh = t.get("/warehouses").await["data"][0]["id"].as_str().unwrap().to_string();
    let wh2 = t.id("/warehouses", json!({"code": "B", "name": "Branch Store"})).await;
    // opening stock 100 @ 80
    t.post("/stock/opening", json!({"warehouse_id": wh, "reason": "Opening", "lines": [{"product_id": p, "qty_change": 100, "unit_cost": 80}]})).await;
    assert_eq!(t.bal("inventory").await, d("8000.00"));
    assert_eq!(t.bal("opening_equity").await, d("-8000.00"));
    // a no-cost item cannot be added blindly
    let p2 = t.id("/products", json!({"sku": "W-2", "name": "No cost yet"})).await;
    assert_eq!(t.fail(Method::POST, "/stock/adjustments", Some(json!({"warehouse_id": wh, "reason": "found", "lines": [{"product_id": p2, "qty_change": 5}]}))).await.0, 422);
    // transfer moves value at cost
    let tr = t.post("/stock/transfers", json!({"from_warehouse_id": wh, "to_warehouse_id": wh2, "lines": [{"product_id": p, "qty": 40}]})).await;
    assert_eq!(dec(&tr["data"]["lines"][0]["value"]), d("3200.00"));
    // cannot issue more than is on hand
    let (s, _, m) = t.fail(Method::POST, "/stock/transfers", Some(json!({"from_warehouse_id": wh2, "to_warehouse_id": wh, "lines": [{"product_id": p, "qty": 41}]}))).await;
    assert_eq!(s, 422); assert!(m.contains("Not enough stock"), "{m}");
    // weighted average: receive 60 more @ 100 into main (60 on hand @ 80 -> 120 @ 90)
    let supplier = t.id("/suppliers", json!({"name": "Local Maker", "supplier_type": "local"})).await;
    let po = t.post("/purchase-orders", json!({"supplier_id": supplier, "lines": [{"product_id": p, "qty": 60, "unit_price": 100, "vat_rate": 0}]})).await;
    let po_id = po["data"]["id"].as_str().unwrap();
    t.post(&format!("/purchase-orders/{po_id}/submit"), json!({})).await;
    let line = t.get(&format!("/purchase-orders/{po_id}")).await["data"]["lines"][0]["id"].as_str().unwrap().to_string();
    t.post("/goods-receipts", json!({"po_id": po_id, "warehouse_id": wh, "lines": [{"po_line_id": line, "qty": 60}]})).await;
    let st = t.get(&format!("/stock?warehouse_id={wh}")).await;
    assert_eq!(dec(&st["data"][0]["qty"]), d("120"));
    assert_eq!(dec(&st["data"][0]["avg_cost"]), d("90.000000"));
    // over-receiving a PO line is refused
    assert_eq!(t.fail(Method::POST, "/goods-receipts", Some(json!({"po_id": po_id, "warehouse_id": wh, "lines": [{"po_line_id": line, "qty": 1}]}))).await.0, 422);
    // stock ledger always equals GL inventory
    let val = t.get("/reports/stock-valuation").await;
    assert_eq!(val["data"]["reconciled"], true);
    assert_eq!(t.bal("grni").await, d("-6000.00"));

    // concurrency: 20 simultaneous deliveries of 10 each against 160 on hand => exactly 16 succeed
    let cust = t.id("/customers", json!({"name": "Big Buyer", "credit_limit": 0})).await;
    let mut handles = vec![];
    for _ in 0..20 {
        let so = t.post("/sales-orders", json!({"customer_id": cust, "warehouse_id": wh2, "lines": [{"product_id": p, "qty": 10}]})).await;
        let id = so["data"]["id"].as_str().unwrap().to_string();
        t.post(&format!("/sales-orders/{id}/submit"), json!({})).await;
        let line = t.get(&format!("/sales-orders/{id}")).await["data"]["lines"][0]["id"].as_str().unwrap().to_string();
        handles.push((id, line));
    }
    // move the remaining main stock so wh2 holds 160
    t.post("/stock/transfers", json!({"from_warehouse_id": wh, "to_warehouse_id": wh2, "lines": [{"product_id": p, "qty": 120}]})).await;
    let mut tasks = vec![];
    for (id, line) in handles {
        let app = t.app.clone(); let tok = t.token.clone();
        tasks.push(tokio::spawn(async move {
            use tower::ServiceExt;
            let req = axum::http::Request::builder().method("POST").uri("/api/v1/deliveries").header("authorization", format!("Bearer {tok}")).header("content-type", "application/json")
                .body(axum::body::Body::from(json!({"so_id": id, "lines": [{"so_line_id": line, "qty": 10}]}).to_string())).unwrap();
            app.oneshot(req).await.unwrap().status().as_u16()
        }));
    }
    let mut ok = 0; let mut refused = 0;
    for h in tasks { match h.await.unwrap() { 201 => ok += 1, 422 => refused += 1, other => panic!("unexpected status {other}") } }
    assert_eq!((ok, refused), (16, 4), "no overselling under concurrent deliveries");
    let st = t.get(&format!("/stock?warehouse_id={wh2}")).await;
    assert_eq!(dec(&st["data"][0]["qty"]), d("0"));
    assert_eq!(t.get("/reports/stock-valuation").await["data"]["reconciled"], true);
    assert_eq!(t.get("/reports/trial-balance").await["data"]["balanced"], true);

    // concurrent document numbering never repeats
    let mut tasks = vec![];
    for i in 0..15 {
        let app = t.app.clone(); let tok = t.token.clone();
        tasks.push(tokio::spawn(async move {
            use tower::ServiceExt;
            let req = axum::http::Request::builder().method("POST").uri("/api/v1/requisitions").header("authorization", format!("Bearer {tok}")).header("content-type", "application/json")
                .body(axum::body::Body::from(json!({"notes": format!("r{i}"), "lines": [{"product_id": "00000000-0000-0000-0000-000000000000", "qty": 1}]}).to_string())).unwrap();
            app.oneshot(req).await.unwrap().status().as_u16()
        }));
    }
    for h in tasks { assert_eq!(h.await.unwrap(), 422); } // unknown product, but must not deadlock or 500
    t.cleanup().await;
}

#[tokio::test]
async fn local_purchase_bill_payment_with_tds_vds_and_approvals() {
    let t = new_app().await;
    let bank = funded_bank(&t, 50_000_000).await;
    let p = t.id("/products", json!({"sku": "R-1", "name": "Rice bran oil", "sale_price": 200, "vat_rate": 15})).await;
    let wh = t.get("/warehouses").await["data"][0]["id"].as_str().unwrap().to_string();
    let sup = t.id("/suppliers", json!({"name": "Dhaka Oil Mills", "supplier_type": "local", "payment_terms_days": 15, "tds_rate": 5, "vds_rate": 15})).await;

    // PO of Tk 20 lakh needs manager approval; the creator (owner) can approve as owner
    let po = t.post("/purchase-orders", json!({"supplier_id": sup, "order_date": "2026-11-02", "lines": [{"product_id": p, "qty": 10000, "unit_price": 200}]})).await;
    let id = po["data"]["id"].as_str().unwrap();
    assert_eq!(dec(&po["data"]["vat_amount"]), d("300000.00"));
    assert_eq!(dec(&po["data"]["total"]), d("2300000.00"));
    assert_eq!(t.post(&format!("/purchase-orders/{id}/submit"), json!({})).await["data"]["status"], "pending_approval");
    // a draft can no longer be edited; cannot cancel -> yes can cancel pending; but cannot receive before approval
    assert_eq!(t.fail(Method::POST, "/goods-receipts", Some(json!({"po_id": id, "warehouse_id": wh, "lines": []}))).await.0, 422);
    // the requester may not approve their own request unless owner: use a manager to reject then re-edit
    let mgr = t.user_with_role("mgr@example.com", "manager").await;
    let pend = t.get("/approvals?status=pending").await;
    let aid = pend["data"][0]["id"].as_str().unwrap();
    let (s, b) = t.raw(Method::POST, &format!("/api/v1/approvals/{aid}/reject"), Some(&mgr), Some(json!({}))).await;
    assert_eq!(s, 422, "rejection needs a reason: {b}");
    let (s, _) = t.raw(Method::POST, &format!("/api/v1/approvals/{aid}/reject"), Some(&mgr), Some(json!({"comment": "Price too high"}))).await;
    assert_eq!(s, 200);
    assert_eq!(t.get(&format!("/purchase-orders/{id}")).await["data"]["status"], "rejected");
    // the requester is notified
    let n = t.get("/notifications?unread=true").await;
    assert!(n["data"].as_array().unwrap().iter().any(|x| x["title"].as_str().unwrap().contains("rejected")));
    // fix and resubmit; manager approves (level 1 only for < 50 lakh)
    t.put(&format!("/purchase-orders/{id}"), json!({"supplier_id": sup, "order_date": "2026-11-02", "lines": [{"product_id": p, "qty": 10000, "unit_price": 190}]})).await;
    t.post(&format!("/purchase-orders/{id}/submit"), json!({})).await;
    let pend = t.get("/approvals?status=pending").await;
    let aid = pend["data"][0]["id"].as_str().unwrap();
    let (s, _) = t.raw(Method::POST, &format!("/api/v1/approvals/{aid}/approve"), Some(&mgr), Some(json!({}))).await;
    assert_eq!(s, 200);
    let (s, b) = t.raw(Method::POST, &format!("/api/v1/approvals/{aid}/approve"), Some(&mgr), Some(json!({}))).await;
    assert_eq!(s, 422, "already decided: {b}");
    // receive: 10,000 x 190 = 1,900,000 into stock
    let line = t.get(&format!("/purchase-orders/{id}")).await["data"]["lines"][0]["id"].as_str().unwrap().to_string();
    let grn = t.post("/goods-receipts", json!({"po_id": id, "warehouse_id": wh, "receipt_date": "2026-11-05", "lines": [{"po_line_id": line, "qty": 10000}]})).await;
    assert_eq!(dec(&grn["data"]["total_value"]), d("1900000.00"));
    assert_eq!(t.get(&format!("/purchase-orders/{id}")).await["data"]["status"], "received");
    // supplier bill without a Mushak number is refused when VAT is claimed
    let g = grn["data"]["id"].as_str().unwrap();
    assert_eq!(t.fail(Method::POST, "/supplier-bills", Some(json!({"supplier_id": sup, "grn_id": g, "subtotal": 1900000, "vat_amount": 285000}))).await.0, 422);
    let bill = t.post("/supplier-bills", json!({"supplier_id": sup, "grn_id": g, "bill_date": "2026-11-06", "mushak_no": "6.3-000451", "subtotal": 1900000, "vat_amount": 285000})).await;
    assert_eq!(dec(&bill["data"]["total"]), d("2185000.00"));
    assert_eq!(bill["data"]["due_date"], "2026-11-21", "15-day terms");
    assert_eq!(t.bal("grni").await, d("0.00"), "GRNI cleared by the bill");
    assert_eq!(t.bal("ap_local").await, d("-2185000.00"));
    assert_eq!(t.bal("vat_input").await, d("285000.00"));
    // the same receipt cannot be billed twice
    assert_eq!(t.fail(Method::POST, "/supplier-bills", Some(json!({"supplier_id": sup, "grn_id": g, "mushak_no": "x", "subtotal": 1, "vat_amount": 0}))).await.0, 422);

    // pay in full: TDS 5% and VDS 15% of the pre-VAT value (1,900,000) are withheld
    let bid = bill["data"]["id"].as_str().unwrap();
    let pay = t.post("/supplier-payments", json!({"supplier_id": sup, "bank_account_id": bank, "payment_date": "2026-11-20", "allocations": [{"bill_id": bid, "amount": 2185000}]})).await;
    assert_eq!(dec(&pay["data"]["tds_amount"]), d("95000.00"));
    assert_eq!(dec(&pay["data"]["vds_amount"]), d("285000.00"));
    assert_eq!(dec(&pay["data"]["net_amount"]), d("1805000.00"));
    assert_eq!(pay["data"]["status"], "pending_approval", "Tk 21 lakh payment needs the owner");
    let pend = t.get("/approvals?status=pending&entity_type=supplier_payment").await;
    // a manager is not the right role for a payment: the owner must decide
    assert_eq!(t.raw(Method::POST, &format!("/api/v1/approvals/{}/approve", pend["data"][0]["id"].as_str().unwrap()), Some(&mgr), Some(json!({}))).await.0, 403);
    t.post(&format!("/approvals/{}/approve", pend["data"][0]["id"].as_str().unwrap()), json!({})).await;
    assert_eq!(t.get(&format!("/supplier-bills/{bid}")).await["data"]["status"], "paid");
    assert_eq!(t.bal("ap_local").await, d("0.00"));
    assert_eq!(t.bal("tds_payable").await, d("-95000.00"));
    assert_eq!(t.bal("vds_payable").await, d("-285000.00"));
    // deposit VDS to the treasury; cannot deposit more than owed
    let bank_before = t.get(&format!("/bank-accounts/{bank}")).await["data"]["balance_bdt"].clone();
    t.post("/tax/deposits", json!({"kind": "vds", "amount": 285000, "bank_account_id": bank, "challan_no": "TR-6/1"})).await;
    assert_eq!(t.fail(Method::POST, "/tax/deposits", Some(json!({"kind": "vds", "amount": 1, "bank_account_id": bank}))).await.0, 422);
    assert_eq!(dec(&t.get(&format!("/bank-accounts/{bank}")).await["data"]["balance_bdt"]), dec(&bank_before) - d("285000"));
    // supplier-ageing is clean, trial balance balances
    assert_eq!(dec(&t.get("/reports/payables-ageing?as_of=2026-12-01").await["data"]["totals"]["total"]), d("0"));
    assert_eq!(t.get("/reports/trial-balance").await["data"]["balanced"], true);
    t.cleanup().await;
}

#[tokio::test]
async fn sales_credit_limit_returns_and_receipts() {
    let t = new_app().await;
    let bank = funded_bank(&t, 5_000_000).await;
    let wh = t.get("/warehouses").await["data"][0]["id"].as_str().unwrap().to_string();
    let p = t.id("/products", json!({"sku": "F-1", "name": "Fan", "sale_price": 1000, "min_sale_price": 900, "vat_rate": 15, "sd_rate": 0})).await;
    t.post("/stock/opening", json!({"warehouse_id": wh, "reason": "Opening", "lines": [{"product_id": p, "qty_change": 100, "unit_cost": 600}]})).await;
    let c = t.id("/customers", json!({"name": "Rahman Traders", "credit_limit": 100000, "payment_terms_days": 10})).await;

    // price floor and credit limit are enforced
    assert_eq!(t.fail(Method::POST, "/sales-orders", Some(json!({"customer_id": c, "warehouse_id": wh, "lines": [{"product_id": p, "qty": 1, "unit_price": 800}]}))).await.0, 422);
    let big = t.post("/sales-orders", json!({"customer_id": c, "warehouse_id": wh, "lines": [{"product_id": p, "qty": 100}]})).await;
    let (s, _, m) = t.fail(Method::POST, &format!("/sales-orders/{}/submit", big["data"]["id"].as_str().unwrap()), None).await;
    assert_eq!(s, 422); assert!(m.contains("credit limit"), "{m}");
    // within the limit: 80 x 1,000 = 80,000 + 15% = 92,000
    let so = t.post("/sales-orders", json!({"customer_id": c, "warehouse_id": wh, "order_date": "2026-11-10", "lines": [{"product_id": p, "qty": 80}]})).await;
    let so_id = so["data"]["id"].as_str().unwrap().to_string();
    t.post(&format!("/sales-orders/{so_id}/submit"), json!({})).await;
    // a second order now exceeds the remaining headroom (92,000 already committed)
    let o2 = t.post("/sales-orders", json!({"customer_id": c, "warehouse_id": wh, "lines": [{"product_id": p, "qty": 10}]})).await;
    assert_eq!(t.fail(Method::POST, &format!("/sales-orders/{}/submit", o2["data"]["id"].as_str().unwrap()), None).await.0, 422);

    let line = t.get(&format!("/sales-orders/{so_id}")).await["data"]["lines"][0]["id"].as_str().unwrap().to_string();
    // partial delivery 50, invoice it
    let d1 = t.post("/deliveries", json!({"so_id": so_id, "delivery_date": "2026-11-11", "invoice": true, "lines": [{"so_line_id": line, "qty": 50}]})).await;
    assert_eq!(dec(&d1["data"]["cogs_value"]), d("30000.00"));
    assert_eq!(t.get(&format!("/sales-orders/{so_id}")).await["data"]["status"], "partially_delivered");
    let inv_id = d1["data"]["invoice_id"].as_str().unwrap().to_string();
    let inv = t.get(&format!("/sales-invoices/{inv_id}")).await;
    assert_eq!(dec(&inv["data"]["total"]), d("57500.00"));
    assert_eq!(inv["data"]["due_date"], "2026-11-21");
    // cannot deliver more than ordered
    assert_eq!(t.fail(Method::POST, "/deliveries", Some(json!({"so_id": so_id, "lines": [{"so_line_id": line, "qty": 31}]}))).await.0, 422);
    // Mushak 6.3 print data
    let m = t.get(&format!("/sales-invoices/{inv_id}/mushak-6-3")).await;
    assert_eq!(m["data"]["seller"]["bin"], "000123456-0101");
    assert_eq!(dec(&m["data"]["totals"]["grand_total"]), d("57500.00"));
    assert_eq!(m["data"]["items"].as_array().unwrap().len(), 1);

    // return 10 fans: credit note = 11,500, stock back at cost 6,000
    let il: String = inv["data"]["lines"][0]["id"].as_str().unwrap().into();
    let rt = t.post("/sales-returns", json!({"invoice_id": inv_id, "reason": "Defective", "lines": [{"invoice_line_id": il, "qty": 10}]})).await;
    assert_eq!(dec(&rt["data"]["total"]), d("11500.00"));
    assert_eq!(dec(&rt["data"]["vat_amount"]), d("1500.00"));
    assert_eq!(dec(&t.get("/stock").await["data"][0]["qty"]), d("60"));
    assert_eq!(t.fail(Method::POST, "/sales-returns", Some(json!({"invoice_id": inv_id, "reason": "x", "lines": [{"invoice_line_id": il, "qty": 41}]}))).await.0, 422);
    let inv = t.get(&format!("/sales-invoices/{inv_id}")).await;
    assert_eq!(dec(&inv["data"]["balance"]), d("46000.00"));

    // receipt: pay part with an advance on top (unallocated), then allocate the rest
    let r = t.post("/customer-receipts", json!({"customer_id": c, "bank_account_id": bank, "receipt_date": "2026-11-15", "gross_amount": 50000, "vds_amount": 2000, "allocations": [{"invoice_id": inv_id, "amount": 40000}]})).await;
    assert_eq!(dec(&r["data"]["unallocated"]), d("10000.00"));
    assert_eq!(t.fail(Method::POST, "/customer-receipts", Some(json!({"customer_id": c, "bank_account_id": bank, "gross_amount": 10, "allocations": [{"invoice_id": inv_id, "amount": 99999}]}))).await.0, 422);
    let rid = r["data"]["id"].as_str().unwrap();
    t.post(&format!("/customer-receipts/{rid}/allocate"), json!({"allocations": [{"invoice_id": inv_id, "amount": 6000}]})).await;
    let inv = t.get(&format!("/sales-invoices/{inv_id}")).await;
    assert_eq!(inv["data"]["status"], "paid");
    assert_eq!(dec(&t.get(&format!("/customer-receipts/{rid}")).await["data"]["unallocated"]), d("4000.00"));
    // statement ties to the receivable
    let stmt = t.get(&format!("/customers/{c}/statement")).await;
    assert_eq!(dec(&stmt["data"][0]["balance"]), t.bal("ar").await);
    assert_eq!(t.bal("ar").await, d("-4000.00"), "customer advance shows as a credit balance");
    // overdue ageing
    let ag = t.get("/reports/receivables-ageing?as_of=2026-12-31").await;
    assert_eq!(dec(&ag["data"]["totals"]["total"]), d("0"));
    t.cleanup().await;
}

#[tokio::test]
async fn payroll_attendance_leave_and_posting() {
    let t = new_app().await;
    let bank = funded_bank(&t, 2_000_000).await;
    let e1 = t.id("/employees", json!({"name": "Nusrat Jahan", "join_date": "2025-07-01", "basic_salary": 32000, "house_rent": 16000, "medical": 4000, "conveyance": 3000, "pf_rate": 10, "monthly_tax": 1000})).await;
    let e2 = t.id("/employees", json!({"name": "Karim Uddin", "join_date": "2024-01-01", "basic_salary": 20000, "house_rent": 10000, "medical": 2000})).await;
    // auto numbering
    assert_eq!(t.get(&format!("/employees/{e1}")).await["data"]["code"], "EMP-00001");
    // two absent days for e1 in November; Sunday leave (unpaid) approved for e2
    t.post("/attendance", json!({"work_date": "2026-11-09", "rows": [{"employee_id": e1, "status": "absent"}, {"employee_id": e2, "status": "present"}]})).await;
    t.post("/attendance", json!({"work_date": "2026-11-10", "rows": [{"employee_id": e1, "status": "absent"}]})).await;
    let lv = t.post("/leave-requests", json!({"employee_id": e2, "leave_type": "unpaid", "from_date": "2026-11-15", "to_date": "2026-11-16"})).await;
    assert_eq!(dec(&lv["data"]["days"]), d("2"));
    assert_eq!(t.fail(Method::POST, "/leave-requests", Some(json!({"employee_id": e2, "leave_type": "casual", "from_date": "2026-11-16", "to_date": "2026-11-17"}))).await.0, 422, "overlap");
    t.post(&format!("/leave-requests/{}/decide", lv["data"]["id"].as_str().unwrap()), json!({"approve": true})).await;

    let run = t.post("/payroll-runs", json!({"year": 2026, "month": 11})).await;
    let slips = run["data"]["payslips"].as_array().unwrap();
    let s1 = slips.iter().find(|s| s["employee_name"] == "Nusrat Jahan").unwrap();
    assert_eq!(dec(&s1["gross"]), d("55000"));
    assert_eq!(dec(&s1["absent_deduction"]), d("4230.77"));
    assert_eq!(dec(&s1["pf_employee"]), d("3200.00"));
    assert_eq!(dec(&s1["net_pay"]), d("46569.23"));
    let s2 = slips.iter().find(|s| s["employee_name"] == "Karim Uddin").unwrap();
    assert_eq!(dec(&s2["absent_days"]), d("2"), "unpaid leave counts as absence");
    // duplicate month refused
    assert_eq!(t.fail(Method::POST, "/payroll-runs", Some(json!({"year": 2026, "month": 11}))).await.0, 409);
    let id = run["data"]["id"].as_str().unwrap();
    // cannot post or pay before approval
    assert_eq!(t.fail(Method::POST, &format!("/payroll-runs/{id}/post"), Some(json!({}))).await.0, 422);
    let hr = t.user_with_role("hr@example.com", "hr").await;
    assert_eq!(t.raw(Method::POST, &format!("/api/v1/payroll-runs/{id}/pay"), Some(&hr), Some(json!({"bank_account_id": bank}))).await.0, 403, "HR cannot move money");
    let sub = t.post(&format!("/payroll-runs/{id}/submit"), json!({})).await;
    assert_eq!(sub["data"]["status"], "pending_approval", "every payroll needs the owner");
    let pend = t.get("/approvals?status=pending&entity_type=payroll_run").await;
    t.post(&format!("/approvals/{}/approve", pend["data"][0]["id"].as_str().unwrap()), json!({})).await;
    let net = dec(&t.get(&format!("/payroll-runs/{id}")).await["data"]["total_net"]);
    t.post(&format!("/payroll-runs/{id}/post"), json!({})).await;
    // 46,569.23 + (32,000 - 2/26 x 32,000 = 2,461.54 => 29,538.46)
    assert_eq!(net, d("76107.69"));
    assert_eq!(t.bal("salary_payable").await, d("-76107.69"));
    assert_eq!(t.bal("pf_payable").await, d("-3200.00") + d("-3200.00"));
    assert_eq!(t.bal("tds_payable").await, d("-1000.00"));
    t.post(&format!("/payroll-runs/{id}/pay"), json!({"bank_account_id": bank, "payment_date": "2026-12-01"})).await;
    assert_eq!(t.bal("salary_payable").await, d("0.00"));
    assert_eq!(t.get(&format!("/payroll-runs/{id}")).await["data"]["status"], "paid");
    assert_eq!(t.get("/reports/trial-balance").await["data"]["balanced"], true);
    t.cleanup().await;
}

#[tokio::test]
async fn tt_advance_fx_revaluation_lc_limits_and_alerts() {
    let t = new_app().await;
    let bank = funded_bank(&t, 100_000_000).await;
    let p = t.id("/products", json!({"sku": "M-1", "name": "Machine part", "sale_price": 9000})).await;
    let sup = t.id("/suppliers", json!({"name": "Osaka Parts", "supplier_type": "foreign", "currency": "USD", "country": "JP"})).await;
    t.post("/exchange-rates", json!({"currency": "USD", "rate_date": "2026-11-01", "rate": 121})).await;
    // an import order must name its Incoterm
    assert_eq!(t.fail(Method::POST, "/purchase-orders", Some(json!({"supplier_id": sup, "po_type": "import", "lines": [{"product_id": p, "qty": 1, "unit_price": 10}]}))).await.0, 422);
    let mk = |qty: i64| json!({"supplier_id": sup, "incoterm": "FOB", "payment_mode": "tt", "currency": "USD", "order_date": "2026-11-02", "lines": [{"product_id": p, "qty": qty, "unit_price": 50}]});
    let po = t.post("/purchase-orders", mk(100)).await;
    let id = po["data"]["id"].as_str().unwrap().to_string();
    assert_eq!(dec(&po["data"]["total_bdt"]), d("605000.00"), "rate defaults from the rate table");
    // nothing may be paid or shipped before approval
    assert_eq!(t.fail(Method::POST, "/foreign-payments", Some(json!({"kind": "tt_advance", "po_id": id, "bank_account_id": bank, "fc_amount": 100}))).await.0, 422);
    t.post(&format!("/purchase-orders/{id}/submit"), json!({})).await;
    let pend = t.get("/approvals?status=pending").await;
    t.post(&format!("/approvals/{}/approve", pend["data"][0]["id"].as_str().unwrap()), json!({})).await;

    // advance USD 1,000 at 120; more than the order is refused
    t.post("/foreign-payments", json!({"kind": "tt_advance", "po_id": id, "bank_account_id": bank, "payment_date": "2026-11-03", "fc_amount": 1000, "fx_rate": 120, "bank_charges": 1500})).await;
    assert_eq!(t.fail(Method::POST, "/foreign-payments", Some(json!({"kind": "tt_advance", "po_id": id, "bank_account_id": bank, "fc_amount": 4500, "fx_rate": 120}))).await.0, 422);
    assert_eq!(t.bal("supplier_advance").await, d("120000.00"));
    assert_eq!(t.bal("bank_charges").await, d("1500.00"));

    // ship all 100 units, depart at rate 122 -> advance 120,000 + 4,000 x 122 = 488,000
    let line = t.get(&format!("/purchase-orders/{id}")).await["data"]["lines"][0]["id"].as_str().unwrap().to_string();
    let shp = t.post("/shipments", json!({"po_id": id, "lines": [{"po_line_id": line, "qty": 100}], "containers": [{"container_no": "TGHU2208113"}]})).await;
    let sid = shp["data"]["id"].as_str().unwrap().to_string();
    assert_eq!(t.fail(Method::POST, &format!("/foreign-payments"), Some(json!({"kind": "tt_payment", "shipment_id": sid, "bank_account_id": bank, "fc_amount": 10, "fx_rate": 122}))).await.0, 422, "nothing is payable before shipment");
    assert_eq!(t.fail(Method::POST, &format!("/shipments/{sid}/depart"), Some(json!({"fx_rate": 122}))).await.0, 422, "bill of lading number is mandatory");
    t.post(&format!("/shipments/{sid}/depart"), json!({"bl_no": "BL-77", "departed_at": "2026-11-10", "fx_rate": 122})).await;
    assert_eq!(t.bal("git").await, d("608000.00"));
    assert_eq!(t.bal("ap_foreign").await, d("-488000.00"));
    // a second order line cannot be shipped again beyond the ordered quantity
    assert_eq!(t.fail(Method::POST, "/shipments", Some(json!({"po_id": id, "lines": [{"po_line_id": line, "qty": 1}]}))).await.0, 422);

    // exposure: USD 4,000 payable at today's latest rate 121
    let ex = t.get("/fx/exposure").await;
    let usd = ex["data"]["by_currency"].as_array().unwrap().iter().find(|c| c["currency"] == "USD").unwrap().clone();
    assert_eq!(dec(&usd["net_exposure_fc"]), d("4000.00"));
    assert_eq!(dec(&usd["impact_of_1pct_bdt"]), d("4840.00"));
    // revalue at a higher month-end rate: payable grows 4,000 -> loss
    t.post("/exchange-rates", json!({"currency": "USD", "rate_date": "2026-11-30", "rate": 123})).await;
    let rv = t.post("/fx/revalue", json!({"date": "2026-11-30"})).await;
    assert_eq!(dec(&rv["data"]["net_loss"]), d("4000.00"));
    assert_eq!(t.bal("ap_foreign").await, d("-492000.00"));
    assert_eq!(t.bal("fx_gain_loss").await, d("4000.00"));
    assert_eq!(t.post("/fx/revalue", json!({"date": "2026-11-30"})).await["data"]["posted"], false, "second run finds nothing to do");

    // pay in two parts at 123 (no further difference), over-payment refused
    let a = t.post("/foreign-payments", json!({"kind": "tt_payment", "shipment_id": sid, "bank_account_id": bank, "payment_date": "2026-12-01", "fc_amount": 1500, "fx_rate": 123})).await;
    assert_eq!(dec(&a["data"]["fx_gain_loss"]), d("0.00"));
    assert_eq!(t.fail(Method::POST, "/foreign-payments", Some(json!({"kind": "tt_payment", "shipment_id": sid, "bank_account_id": bank, "fc_amount": 2501, "fx_rate": 123}))).await.0, 422);
    t.post("/foreign-payments", json!({"kind": "tt_payment", "shipment_id": sid, "bank_account_id": bank, "payment_date": "2026-12-02", "fc_amount": 2500, "fx_rate": 124})).await;
    assert_eq!(t.bal("ap_foreign").await, d("0.00"));
    assert_eq!(t.bal("fx_gain_loss").await, d("6500.00"), "4,000 revaluation + 2,500 x (124 - 123)");
    assert_eq!(t.get("/reports/trial-balance").await["data"]["balanced"], true);

    // ---- LC rules: shipment cannot exceed LC value plus tolerance; amendments extend it ----
    let po2 = t.post("/purchase-orders", mk(100)).await;
    let id2 = po2["data"]["id"].as_str().unwrap().to_string();
    t.post(&format!("/purchase-orders/{id2}/submit"), json!({})).await;
    let pend = t.get("/approvals?status=pending").await;
    t.post(&format!("/approvals/{}/approve", pend["data"][0]["id"].as_str().unwrap()), json!({})).await;
    let lc = t.post("/lcs", json!({"po_id": id2, "bank_account_id": bank, "amount": 4000, "tolerance_pct": 5, "margin_pct": 10, "expiry_date": "2027-03-01"})).await;
    let lc_id = lc["data"]["id"].as_str().unwrap().to_string();
    // shipments need an opened LC
    let line2 = t.get(&format!("/purchase-orders/{id2}")).await["data"]["lines"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(t.fail(Method::POST, "/shipments", Some(json!({"po_id": id2, "lc_id": lc_id, "lines": [{"po_line_id": line2, "qty": 100}]}))).await.0, 422);
    t.post(&format!("/lcs/{lc_id}/open"), json!({"open_date": "2026-11-12", "fx_rate": 121, "commission": 8000})).await;
    assert_eq!(t.bal("lc_margin").await, d("48400.00"), "10% of USD 4,000 at 121");
    let s2 = t.post("/shipments", json!({"po_id": id2, "lc_id": lc_id, "lines": [{"po_line_id": line2, "qty": 100}]})).await;
    let (st, _, m) = t.fail(Method::POST, &format!("/shipments/{}/depart", s2["data"]["id"].as_str().unwrap()), Some(json!({"bl_no": "BL-2", "fx_rate": 121}))).await;
    assert_eq!(st, 422); assert!(m.contains("exceeds"), "{m}");
    t.post(&format!("/lcs/{lc_id}/amend"), json!({"amount": 5000, "reason": "Quantity increased", "charges": 2500, "amendment_date": "2026-11-13"})).await;
    let doc = t.get(&format!("/lcs/{lc_id}")).await;
    assert_eq!(dec(&doc["data"]["amount"]), d("5000.00"));
    assert_eq!(doc["data"]["amendments"].as_array().unwrap().len(), 1);
    t.post(&format!("/shipments/{}/depart", s2["data"]["id"].as_str().unwrap()), json!({"bl_no": "BL-2", "fx_rate": 121})).await;
    // LC cannot be closed while documents remain unpaid
    assert_eq!(t.fail(Method::POST, &format!("/lcs/{lc_id}/close"), Some(json!({}))).await.0, 422);

    // ---- alerts: LC expiring soon and demurrage, raised once ----
    sqlx::query("UPDATE letters_of_credit SET expiry_date = (now() AT TIME ZONE 'Asia/Dhaka')::date + 5").execute(&t.db).await.unwrap();
    let created = weft_erp::modules::jobs::run_alerts(&t.state).await.unwrap();
    assert!(created >= 1);
    let n = t.get("/notifications?unread=true").await;
    assert!(n["data"].as_array().unwrap().iter().any(|x| x["title"].as_str().unwrap().contains("expires in")));
    let again = weft_erp::modules::jobs::run_alerts(&t.state).await.unwrap();
    assert_eq!(again, 0, "alerts are not repeated");
    let c = t.get("/notifications/unread-count").await["data"]["unread"].as_i64().unwrap();
    assert!(c >= 1);
    t.post("/notifications/read-all", json!({})).await;
    assert_eq!(t.get("/notifications/unread-count").await["data"]["unread"], 0);
    t.cleanup().await;
}
