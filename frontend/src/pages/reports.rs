//! Reports & analytics: financial statements, ageing, stock valuation, sales analysis and the import pipeline.
use crate::{api, fmt, i18n::t, ui::*};
use leptos::prelude::*;
use serde_json::Value;
use wasm_bindgen::JsCast;

fn download_csv(name: &str, header: &[&str], rows: &[Vec<String>]) {
    let esc = |s: &str| if s.contains([',', '"', '\n']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() };
    let mut out = String::from("\u{feff}"); out.push_str(&header.iter().map(|h| esc(h)).collect::<Vec<_>>().join(",")); out.push('\n');
    for r in rows { out.push_str(&r.iter().map(|c| esc(c)).collect::<Vec<_>>().join(",")); out.push('\n'); }
    let parts = js_sys::Array::new(); parts.push(&wasm_bindgen::JsValue::from_str(&out));
    let mut opts = web_sys::BlobPropertyBag::new(); opts.set_type("text/csv;charset=utf-8");
    if let Ok(blob) = web_sys::Blob::new_with_str_sequence_and_options(&parts, &opts) { if let Ok(url) = web_sys::Url::create_object_url_with_blob(&blob) {
        if let Some(doc) = web_sys::window().and_then(|w| w.document()) { if let Ok(a) = doc.create_element("a") { let a: web_sys::HtmlAnchorElement = a.unchecked_into(); a.set_href(&url); a.set_download(name); a.click(); let _ = web_sys::Url::revoke_object_url(&url); } } } }
}

const REPORTS: &[(&str, &str, &str)] = &[("is", "Profit & loss", "Income, cost of goods and expenses for a period"), ("bs", "Balance sheet", "What you own and owe at a date"), ("tb", "Trial balance", "Every account's balance"), ("ar", "Receivables ageing", "Who owes you, and for how long"), ("ap", "Payables ageing", "Who you owe, and for how long"),
    ("stock", "Stock valuation", "Value of stock per product, checked against the ledger"), ("sales", "Sales analysis", "By customer, product (with margin) and day"), ("imp", "Import pipeline", "Shipments in flight, demurrage and LC expiry")];

#[component]
pub fn Reports() -> impl IntoView {
    let sel = RwSignal::new("is");
    let from = RwSignal::new(fmt::month_start()); let to = RwSignal::new(fmt::today());
    view! { <div class="page"><PageHead title=t("Reports & Analytics") subtitle="Every figure is calculated live from the ledger. Download any table as a spreadsheet."/>
        <div class="grid" style="grid-template-columns:260px minmax(0,1fr);align-items:start">
            <div class="card" style="padding:10px">{REPORTS.iter().map(|(k, l, d)| { let k = *k; view! { <button class=move || if sel.get() == k { "chip on" } else { "chip" } style="display:block;width:100%;text-align:left;border-radius:12px;margin-bottom:6px;padding:10px 12px" on:click=move |_| sel.set(k)><b>{*l}</b><div class="small dim" style="font-weight:400">{*d}</div></button> } }).collect_view()}</div>
            <div class="grid"><div class="card row wrap no-print"><label class="small mut">"From"</label><input class="input" style="width:auto" type="date" prop:value=move || from.get() on:change=move |e| from.set(event_target_value(&e))/><label class="small mut">"To / as of"</label><input class="input" style="width:auto" type="date" prop:value=move || to.get() on:change=move |e| to.set(event_target_value(&e))/>
                <button class="btn sp" on:click=move |_| { if let Some(w) = web_sys::window() { let _ = w.print(); } }><Icon name="print" size=16/>"Print"</button></div>
                {move || view! { <ReportBody kind=sel.get() from=from.get() to=to.get()/> }}</div></div></div> }
}

#[component]
fn ReportBody(kind: &'static str, from: String, to: String) -> impl IntoView {
    let path = match kind { "is" => format!("/reports/income-statement?from={from}&to={to}"), "bs" => format!("/reports/balance-sheet?as_of={to}"), "tb" => format!("/reports/trial-balance?as_of={to}"), "ar" => format!("/reports/receivables-ageing?as_of={to}"), "ap" => format!("/reports/payables-ageing?as_of={to}"),
        "stock" => "/reports/stock-valuation".to_string(), "sales" => format!("/reports/sales-analysis?from={from}&to={to}"), _ => "/reports/import-pipeline".to_string() };
    let res = LocalResource::new(move || { let p = path.clone(); async move { api::get(&p).await } });
    view! { {move || match res.get() { None => view! { <Loading/> }.into_any(), Some(Err(e)) => view! { <ErrorBox err=e/> }.into_any(), Some(Ok(v)) => { let d = v["data"].clone(); match kind {
        "is" => {
            let (income, expenses) = (statement(&d["income"]), statement(&d["expenses"]));
            let margin = if d["gross_margin_pct"].is_null() { String::new() } else { format!("{} margin", fmt::pct(&d["gross_margin_pct"])) };
            let (ti, gp, te, np) = (fmt::money_short(fmt::num(&d["total_income"])), fmt::money_short(fmt::num(&d["gross_profit"])), fmt::money_short(fmt::num(&d["total_expenses"])), fmt::money_short(fmt::num(&d["net_profit"])));
            let loss = fmt::num(&d["net_profit"]) < 0.0;
            view! { <div class="grid g4"><Kpi label="Income" value=ti/><Kpi label="Gross profit" value=gp sub=margin/><Kpi label="Expenses" value=te/><Kpi label="Net profit" value=np tone=if loss { "bad" } else { "" }/></div>
                <div class="grid g2"><Card title="Income">{income}</Card><Card title="Expenses">{expenses}</Card></div> }.into_any() }
        "bs" => {
            let (assets, liabs, equity) = (statement(&d["assets"]), statement(&d["liabilities"]), statement(&d["equity"]));
            let (ta, profit, tle) = (fmt::money(&d["total_assets"]), fmt::money(&d["retained_profit"]), fmt::money_f(fmt::num(&d["total_liabilities"]) + fmt::num(&d["total_equity"])));
            let ok = d["balanced"].as_bool().unwrap_or(false);
            view! { <div class="grid g2"><Card title="Assets">{assets}<div class="row" style="padding:10px 0"><b>"Total assets"</b><b class="sp num">{ta}</b></div></Card>
                <Card title="Liabilities & equity">{liabs}{equity}<div class="row" style="padding:8px 0"><span>"Profit for the period"</span><b class="sp num">{profit}</b></div><div class="row" style="padding:10px 0"><b>"Total liabilities & equity"</b><b class="sp num">{tle}</b></div></Card></div>
                <Badge status=if ok { "ok" } else { "bad" } label=if ok { "The balance sheet balances" } else { "Out of balance" }/> }.into_any() }
        "tb" => { let rows = d["accounts"].as_array().cloned().unwrap_or_default(); let csv = rows.iter().map(|r| vec![fmt::text(&r["code"]), fmt::text(&r["name"]), fmt::num(&r["balance_dr"]).to_string(), fmt::num(&r["balance_cr"]).to_string()]).collect::<Vec<_>>();
            view! { <Card title="Trial balance" right=ViewFn::from(move || { let csv = csv.clone(); view! { <button class="btn sm" on:click=move |_| download_csv("trial-balance.csv", &["Code", "Account", "Debit", "Credit"], &csv)>"Download CSV"</button> } })>{table(&[col("code", "Code", CK::Mono), col("name", "Account", CK::Text), col("balance_dr", "Debit", CK::Money), col("balance_cr", "Credit", CK::Money)], rows, None)}</Card> }.into_any() }
        "ar" | "ap" => { let rows = d["rows"].as_array().cloned().unwrap_or_default(); let t = d["totals"].clone(); let csv = rows.iter().map(|r| vec![fmt::text(&r["party_name"]), fmt::num(&r["current"]).to_string(), fmt::num(&r["d1_30"]).to_string(), fmt::num(&r["d31_60"]).to_string(), fmt::num(&r["d61_90"]).to_string(), fmt::num(&r["d90_plus"]).to_string(), fmt::num(&r["total"]).to_string()]).collect::<Vec<_>>();
            view! { <div class="grid g4"><Kpi label="Not yet due" value=fmt::money_short(fmt::num(&t["current"]))/><Kpi label="1 to 30 days late" value=fmt::money_short(fmt::num(&t["d1_30"])) tone="warn"/><Kpi label="31 to 90 days late" value=fmt::money_short(fmt::num(&t["d31_60"]) + fmt::num(&t["d61_90"])) tone="warn"/><Kpi label="Over 90 days" value=fmt::money_short(fmt::num(&t["d90_plus"])) tone={if fmt::num(&t["d90_plus"]) > 0.0 { "bad" } else { "" }}/></div>
                <Card title=if kind == "ar" { "By customer" } else { "By supplier" } right=ViewFn::from(move || { let csv = csv.clone(); view! { <button class="btn sm" on:click=move |_| download_csv("ageing.csv", &["Party", "Current", "1-30", "31-60", "61-90", "90+", "Total"], &csv)>"Download CSV"</button> } })>{table(&[col("party_name", "Name", CK::Bold), col("current", "Not due", CK::Money), col("d1_30", "1–30", CK::Money), col("d31_60", "31–60", CK::Money), col("d61_90", "61–90", CK::Money), col("d90_plus", "90+", CK::Money), col("total", "Total", CK::Money)], rows, None)}</Card> }.into_any() }
        "stock" => { let rows = d["items"].as_array().cloned().unwrap_or_default(); let csv = rows.iter().map(|r| vec![fmt::text(&r["sku"]), fmt::text(&r["name"]), fmt::num(&r["qty"]).to_string(), fmt::num(&r["value"]).to_string()]).collect::<Vec<_>>();
            view! { <div class="grid g3"><Kpi label="Stock value" value=fmt::money_short(fmt::num(&d["stock_value"]))/><Kpi label="Inventory in the ledger" value=fmt::money_short(fmt::num(&d["ledger_inventory"]))/><Kpi label="Check" value=if d["reconciled"].as_bool().unwrap_or(false) { "Matches" } else { "Differs" } tone=if d["reconciled"].as_bool().unwrap_or(false) { "" } else { "bad" }/></div>
                <Card title="By product" right=ViewFn::from(move || { let csv = csv.clone(); view! { <button class="btn sm" on:click=move |_| download_csv("stock-valuation.csv", &["SKU", "Product", "Qty", "Value"], &csv)>"Download CSV"</button> } })>{table(&[col("sku", "SKU", CK::Mono), col("name", "Product", CK::Bold), col("category", "Category", CK::Text), col("qty", "Qty", CK::Qty), col("avg_cost", "Avg cost", CK::Money), col("value", "Value", CK::Money), col("sale_price", "Sale price", CK::Money)], rows, None)}</Card> }.into_any() }
        "sales" => { let days: Vec<(String, f64)> = d["by_day"].as_array().map(|a| a.iter().map(|x| (fmt::text(&x["date"]).get(8..).unwrap_or("").to_string(), fmt::num(&x["sales"]))).collect()).unwrap_or_default(); let prod = d["by_product"].as_array().cloned().unwrap_or_default(); let csv = prod.iter().map(|r| vec![fmt::text(&r["sku"]), fmt::text(&r["name"]), fmt::num(&r["qty"]).to_string(), fmt::num(&r["sales"]).to_string(), fmt::num(&r["margin"]).to_string()]).collect::<Vec<_>>();
            view! { <Card title="Sales by day"><BarChart data=days/></Card>
                <Card title="By customer">{table(&[col("name", "Customer", CK::Bold), col("invoices", "Invoices", CK::Int), col("sales", "Sales", CK::Money), col("vat", "VAT", CK::Money), col("total", "Total", CK::Money)], d["by_customer"].as_array().cloned().unwrap_or_default(), None)}</Card>
                <Card title="By product, with margin" right=ViewFn::from(move || { let csv = csv.clone(); view! { <button class="btn sm" on:click=move |_| download_csv("sales-by-product.csv", &["SKU", "Product", "Qty", "Sales", "Margin"], &csv)>"Download CSV"</button> } })>{table(&[col("sku", "SKU", CK::Mono), col("name", "Product", CK::Bold), col("qty", "Qty", CK::Qty), col("sales", "Sales", CK::Money), col("cost", "Cost", CK::Money), col("margin", "Margin", CK::Money), col("margin_pct", "Margin %", CK::Pct)], prod, None)}</Card> }.into_any() }
        _ => {
            let ships = d["shipments"].as_array().cloned().unwrap_or_default(); let lcs = d["open_lcs"].as_array().cloned().unwrap_or_default();
            view! { <Card title="Shipments in progress">{table(&[col("shipment_no", "Shipment", CK::Bold), col("supplier", "Supplier", CK::Text), col("po_no", "Order", CK::Mono), col("status", "Status", CK::Badge), col("eta", "ETA", CK::Date), col("invoice_bdt", "Value (Tk)", CK::Money), col("documents_missing", "Docs missing", CK::Int), col("free_until", "Free until", CK::Date), col("demurrage_bdt", "Demurrage", CK::Money)], ships, None)}</Card>
                <Card title="Open letters of credit">{table(&[col("lc_no", "LC", CK::Bold), col("currency", "Cur.", CK::Text), col("amount", "Amount", CK::Qty), col("outstanding", "Still to pay", CK::Qty), col("expiry_date", "Expires", CK::Date), col("days_left", "Days left", CK::Int)], lcs, None)}</Card> }.into_any() } } } } }}
}
fn statement(v: &Value) -> AnyView {
    let rows = v.as_array().cloned().unwrap_or_default();
    view! { {rows.into_iter().map(|r| view! { <div class="row" style="padding:7px 0;border-bottom:1px solid var(--line)"><span class="mono dim small" style="width:52px">{fmt::text(&r["code"])}</span><span>{fmt::text(&r["name"])}</span><span class="sp num">{fmt::money(&r["amount"])}</span></div> }).collect_view()} }.into_any()
}
