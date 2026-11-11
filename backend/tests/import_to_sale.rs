mod common;
use common::*;
use axum::http::Method;
use serde_json::json;

/// The whole business cycle with exact figures: buy abroad on LC, ship, clear customs,
/// cost the goods, receive, sell, invoice, collect, file VAT. Every step is checked against the ledger.
#[tokio::test]
async fn full_import_to_sale_cycle() {
    let t = new_app().await;

    // ---- master data ----
    let hs = t.id("/hs-codes", json!({"code": "9405.42.00", "description": "LED luminaires", "cd_rate": 25, "rd_rate": 3, "sd_rate": 0, "vat_rate": 15, "ait_rate": 5, "at_rate": 5})).await;
    let product = t.id("/products", json!({"sku": "LED-60W", "name": "LED Panel Light 60W", "hs_code_id": hs, "sale_price": 4200, "min_sale_price": 3000, "vat_rate": 15, "weight_kg": 2, "volume_cbm": 0.01})).await;
    let supplier = t.id("/suppliers", json!({"name": "Shenzhen Lumina Co", "supplier_type": "foreign", "country": "CN", "currency": "USD"})).await;
    let customer = t.id("/customers", json!({"name": "Apex Electric Store", "customer_type": "dealer", "credit_limit": 5000000, "payment_terms_days": 30})).await;
    let cnf = t.id("/suppliers", json!({"name": "Karnaphuli C&F", "supplier_type": "service", "payment_terms_days": 7})).await;
    let wh: String = t.get("/warehouses").await["data"][0]["id"].as_str().unwrap().into();

    // bank with opening capital
    let bank = t.id("/bank-accounts", json!({"code": "EBL", "name": "Eastern Bank Current", "kind": "bank", "currency": "BDT"})).await;
    let bank_gl: String = t.get(&format!("/bank-accounts/{bank}")).await["data"]["account_id"].as_str().unwrap().into();
    let capital = t.acct("capital").await;
    t.post("/journals", json!({"entry_date": "2026-11-01", "memo": "Capital introduced", "lines": [{"account_id": bank_gl, "debit": 20000000}, {"account_id": capital, "credit": 20000000}]})).await;
    t.post("/exchange-rates", json!({"currency": "USD", "rate_date": "2026-11-01", "rate": "120.00"})).await;

    // ---- purchase order (import, CFR) 1000 pcs @ USD 20 ----
    let po = t.post("/purchase-orders", json!({"supplier_id": supplier, "po_type": "import", "currency": "USD", "incoterm": "CFR", "payment_mode": "lc", "order_date": "2026-11-02", "lines": [{"product_id": product, "qty": 1000, "unit_price": 20}]})).await;
    let po_id = po["data"]["id"].as_str().unwrap().to_string();
    assert_eq!(dec(&po["data"]["total"]), d("20000.00"));
    assert_eq!(dec(&po["data"]["fx_rate"]), d("120"));
    assert_eq!(dec(&po["data"]["total_bdt"]), d("2400000.00"));
    let po = t.post(&format!("/purchase-orders/{po_id}/submit"), json!({})).await;
    assert_eq!(po["data"]["status"], "pending_approval", "BDT 24 lakh needs manager approval");
    // the owner approves
    let appr = t.get("/approvals?status=pending").await;
    let aid = appr["data"][0]["id"].as_str().unwrap();
    t.post(&format!("/approvals/{aid}/approve"), json!({"comment": "ok"})).await;
    assert_eq!(t.get(&format!("/purchase-orders/{po_id}")).await["data"]["status"], "approved");
    let po_line: String = t.get(&format!("/purchase-orders/{po_id}")).await["data"]["lines"][0]["id"].as_str().unwrap().into();

    // ---- TT advance 10% at rate 119.50 ----
    let adv = t.post("/foreign-payments", json!({"kind": "tt_advance", "po_id": po_id, "bank_account_id": bank, "payment_date": "2026-11-03", "fc_amount": 2000, "fx_rate": "119.50"})).await;
    assert_eq!(adv["data"]["status"], "posted");
    assert_eq!(t.bal("supplier_advance").await, d("239000.00"));

    // ---- LC: 20% margin on 20,000 x 120, charges 25,000 ----
    let lc = t.post("/lcs", json!({"po_id": po_id, "bank_account_id": bank, "margin_pct": 20, "tolerance_pct": 5, "expiry_date": "2027-02-28", "latest_shipment_date": "2027-01-31"})).await;
    let lc_id = lc["data"]["id"].as_str().unwrap().to_string();
    let opened = t.post(&format!("/lcs/{lc_id}/open"), json!({"open_date": "2026-11-05", "fx_rate": "120", "commission": 20000, "swift_charges": 5000})).await;
    assert_eq!(opened["data"]["status"], "opened");
    assert_eq!(t.bal("lc_margin").await, d("480000.00"));

    // ---- shipment, freight USD 1,000 on top (CFR already includes it; here we model as supplier extra) ----
    let shp = t.post("/shipments", json!({"po_id": po_id, "lc_id": lc_id, "vessel_name": "MV Maersk Chattogram", "etd": "2026-11-10", "eta": "2026-12-01", "freight": 1000, "free_days": 4, "demurrage_per_day": 2000,
        "lines": [{"po_line_id": po_line, "qty": 1000}], "containers": [{"container_no": "MSKU7731450", "size": "40HC"}]})).await;
    let sid = shp["data"]["id"].as_str().unwrap().to_string();
    assert_eq!(dec(&shp["data"]["goods_value"]), d("20000.00"));
    assert_eq!(dec(&shp["data"]["invoice_value"]), d("21000.00"));
    assert!(shp["data"]["documents"].as_array().unwrap().len() >= 9);

    // ---- depart at BL rate 121: advance of USD 2,000 stays at 119.50; USD 19,000 at 121 ----
    let dep = t.post(&format!("/shipments/{sid}/depart"), json!({"departed_at": "2026-11-10", "bl_no": "MAEU226884512", "fx_rate": "121"})).await;
    assert_eq!(dep["data"]["status"], "in_transit");
    // git = 239,000 + 19,000 x 121 = 239,000 + 2,299,000 = 2,538,000
    assert_eq!(t.bal("git").await - d("25000"), d("2538000.00"), "goods in transit (excluding LC charges 25,000)");
    assert_eq!(t.bal("supplier_advance").await, d("0.00"));
    assert_eq!(t.bal("ap_foreign").await, d("-2299000.00"));
    t.post(&format!("/shipments/{sid}/events"), json!({"event_type": "transshipment", "location": "Colombo", "description": "Transshipped"})).await;

    // ---- arrive ----
    let arr = t.post(&format!("/shipments/{sid}/arrive"), json!({"arrived_at": "2026-12-01"})).await;
    assert_eq!(arr["data"]["status"], "arrived");
    assert_eq!(arr["data"]["containers"][0]["free_until"], "2026-12-05");

    // ---- Bill of Entry at customs rate 122 ----
    // C&F USD 21,000 + 1% default insurance (USD 210) = CIF USD 21,210 x 122 = 2,587,620 ; AV (+1% landing) = 2,613,496.20
    // CD 25% = 653,374.05 ; RD 3% = 78,404.89 ; SD 0 ; base = 3,345,275.14
    // VAT 15% = 501,791.27 ; AIT 5% of AV = 130,674.81 ; AT 5% of base = 167,263.76
    let be = t.post("/bills-of-entry", json!({"shipment_id": sid, "be_no": "C-1284551", "be_date": "2026-12-02", "customs_rate": "122", "other_fees": 3500})).await;
    let d0 = &be["data"];
    assert_eq!(dec(&d0["assessable_value"]), d("2613496.20"));
    assert_eq!(dec(&d0["cd"]), d("653374.05"));
    assert_eq!(dec(&d0["rd"]), d("78404.89"));
    assert_eq!(dec(&d0["vat"]), d("501791.27"));
    assert_eq!(dec(&d0["ait"]), d("130674.81"));
    assert_eq!(dec(&d0["at"]), d("167263.76"));
    assert_eq!(dec(&d0["total_payable"]), d("1535008.78")); // incl. 3,500 fees
    let be_id = d0["id"].as_str().unwrap().to_string();
    t.post(&format!("/bills-of-entry/{be_id}/pay"), json!({"bank_account_id": bank, "paid_date": "2026-12-02"})).await;
    t.post(&format!("/bills-of-entry/{be_id}/release"), json!({"released_date": "2026-12-03"})).await;
    assert_eq!(t.bal("vat_input").await, d("501791.27"));

    // ---- landed cost: C&F charge 50,000 + 7,500 VAT owed to the agent ----
    let lcs = t.post("/landed-costs", json!({"shipment_id": sid, "allocation_method": "value", "costing_date": "2026-12-03"})).await;
    let lcs_id = lcs["data"]["id"].as_str().unwrap().to_string();
    t.post(&format!("/landed-costs/{lcs_id}/charges"), json!({"charge_type": "cnf_commission", "amount": 50000, "vat_amount": 7500, "supplier_id": cnf})).await;
    let fin = t.post(&format!("/landed-costs/{lcs_id}/finalize"), json!({})).await;
    // goods 2,538,000 + CD/RD 731,778.94 + fees 3,500 + LC charges 25,000 + C&F 50,000 = 3,348,278.94
    assert_eq!(dec(&fin["data"]["total_landed"]), d("3348278.94"));
    assert_eq!(dec(&fin["data"]["lines"][0]["unit_landed_cost"]), d("3348.278940"));
    assert_eq!(t.bal("git").await, d("3348278.94"));

    // ---- receive into stock at landed cost ----
    let sl: String = t.get(&format!("/shipments/{sid}")).await["data"]["lines"][0]["id"].as_str().unwrap().into();
    t.post(&format!("/shipments/{sid}/receive"), json!({"warehouse_id": wh, "receipt_date": "2026-12-04", "lines": [{"shipment_line_id": sl, "qty": 1000}]})).await;
    assert_eq!(t.bal("git").await, d("0.00"), "nothing left in transit");
    assert_eq!(t.bal("inventory").await, d("3348278.94"));
    let stock = t.get("/stock").await;
    assert_eq!(dec(&stock["data"][0]["qty"]), d("1000"));
    assert_eq!(dec(&stock["data"][0]["avg_cost"]), d("3348.278940"));
    assert_eq!(t.get(&format!("/shipments/{sid}")).await["data"]["status"], "received");

    // ---- pay the supplier via LC at settlement rate 122.20 ----
    let pay = t.post("/foreign-payments", json!({"kind": "lc_settlement", "shipment_id": sid, "bank_account_id": bank, "payment_date": "2026-12-10", "fc_amount": 19000, "fx_rate": "122.20", "bank_charges": 4200})).await;
    // Tk 23 lakh is above the Tk 20 lakh owner-approval rule, so it waits for the owner
    assert_eq!(pay["data"]["status"], "pending_approval");
    assert_eq!(t.bal("ap_foreign").await, d("-2299000.00"), "nothing posted before approval");
    let pending = t.get("/approvals?status=pending&entity_type=foreign_payment").await;
    t.post(&format!("/approvals/{}/approve", pending["data"][0]["id"].as_str().unwrap()), json!({})).await;
    let pay = t.get(&format!("/foreign-payments/{}", pay["data"]["id"].as_str().unwrap())).await;
    assert_eq!(pay["data"]["status"], "posted");
    // book 2,299,000 vs paid 2,321,800 => loss 22,800
    assert_eq!(dec(&pay["data"]["fx_gain_loss"]), d("22800.00"));
    assert_eq!(t.bal("ap_foreign").await, d("0.00"));
    assert_eq!(t.bal("fx_gain_loss").await, d("22800.00"));
    assert_eq!(t.bal("lc_margin").await, d("0.00"), "all margin applied");

    // ---- sell 100 pcs: 4,200 each, 15% VAT ----
    let so = t.post("/sales-orders", json!({"customer_id": customer, "warehouse_id": wh, "order_date": "2026-12-12", "lines": [{"product_id": product, "qty": 100}]})).await;
    let so_id = so["data"]["id"].as_str().unwrap().to_string();
    assert_eq!(dec(&so["data"]["total"]), d("483000.00"));
    let so = t.post(&format!("/sales-orders/{so_id}/submit"), json!({})).await;
    assert_eq!(so["data"]["status"], "confirmed");
    let so_line: String = t.get(&format!("/sales-orders/{so_id}")).await["data"]["lines"][0]["id"].as_str().unwrap().into();
    let dl = t.post("/deliveries", json!({"so_id": so_id, "delivery_date": "2026-12-13", "invoice": true, "lines": [{"so_line_id": so_line, "qty": 100}]})).await;
    assert_eq!(dec(&dl["data"]["cogs_value"]), d("334827.89"));
    let inv_id = dl["data"]["invoice_id"].as_str().unwrap().to_string();
    let inv = t.get(&format!("/sales-invoices/{inv_id}")).await;
    assert_eq!(dec(&inv["data"]["subtotal"]), d("420000.00"));
    assert_eq!(dec(&inv["data"]["vat_amount"]), d("63000.00"));
    assert_eq!(dec(&inv["data"]["total"]), d("483000.00"));
    assert!(inv["data"]["mushak_no"].as_str().unwrap().starts_with("M63-"));
    assert_eq!(t.bal("ar").await, d("483000.00"));
    assert_eq!(t.bal("vat_output").await, d("-63000.00"));

    // customer pays with 15,000 VDS withheld, 5,000 TDS
    t.post("/customer-receipts", json!({"customer_id": customer, "bank_account_id": bank, "receipt_date": "2026-12-20", "gross_amount": 483000, "vds_amount": 15000, "tds_amount": 5000, "allocations": [{"invoice_id": inv_id, "amount": 483000}]})).await;
    assert_eq!(t.get(&format!("/sales-invoices/{inv_id}")).await["data"]["status"], "paid");
    assert_eq!(t.bal("ar").await, d("0.00"));
    assert_eq!(t.bal("vds_receivable").await, d("15000.00"));

    // ---- reports reconcile ----
    let tb = t.get("/reports/trial-balance").await;
    assert_eq!(tb["data"]["balanced"], true);
    let val = t.get("/reports/stock-valuation").await;
    assert_eq!(val["data"]["reconciled"], true, "stock ledger equals GL inventory");
    assert_eq!(dec(&val["data"]["stock_value"]), d("3013451.05")); // 3,348,278.94 - 334,827.89
    let is = t.get("/reports/income-statement?from=2026-12-01&to=2026-12-31").await;
    assert_eq!(dec(&is["data"]["total_income"]), d("420000.00"));
    assert_eq!(dec(&is["data"]["gross_profit"]), d("85172.11")); // 420,000 - 334,827.89
    let bs = t.get("/reports/balance-sheet").await;
    assert_eq!(bs["data"]["balanced"], true);

    // ---- VAT return, December: output 63,000; input local 7,500; import 501,791.27; AT 167,263.76; VDS 15,000 ----
    let v = t.get("/tax/vat-return?year=2026&month=12").await;
    assert_eq!(dec(&v["data"]["figures"]["output_vat"]), d("63000.00"));
    assert_eq!(dec(&v["data"]["figures"]["input_vat_import"]), d("501791.27"));
    assert_eq!(dec(&v["data"]["figures"]["input_vat_local"]), d("7500.00"));
    assert_eq!(dec(&v["data"]["figures"]["net_payable"]), d("-628555.03"), "excess input VAT is carried forward");
    let ret = t.post("/tax/vat-returns?year=2026&month=12", json!({})).await;
    let rid = ret["data"]["id"].as_str().unwrap();
    t.post(&format!("/tax/vat-returns/{rid}/submit"), json!({})).await;
    assert_eq!(t.bal("vat_output").await, d("0.00"));
    assert_eq!(t.bal("vat_input").await, d("628555.03"), "credit carried forward");
    assert_eq!(t.get("/reports/trial-balance").await["data"]["balanced"], true);

    // ---- audit trail captured the key steps ----
    let audit = t.get("/audit-logs?entity=shipments").await;
    assert!(audit["total"].as_i64().unwrap() >= 3);
    let (s, _, _) = t.fail(Method::DELETE, &format!("/products/{product}"), None).await;
    assert_eq!(s, 409, "a product with history cannot be deleted");

    t.cleanup().await;
}
