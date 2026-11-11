//! Ledger Studio: journals, trial balance, account ledger and integrity checks.
use super::common::*;
use crate::{api, crud::*, fmt, i18n::t, state, ui::*};
use leptos::prelude::*;
use leptos_router::hooks::{use_navigate, use_params_map};
use serde_json::{json, Map, Value};

const SOURCES: &[(&str, &str)] = &[("manual", "Manual journals"), ("sales_invoice", "Sales invoices"), ("delivery", "Deliveries"), ("customer_receipt", "Customer receipts"), ("sales_return", "Returns"), ("goods_receipt", "Goods receipts"), ("supplier_bill", "Supplier bills"), ("supplier_payment", "Supplier payments"), ("shipment", "Shipments"), ("foreign_payment", "Foreign payments"), ("bill_of_entry", "Customs duty"), ("landed_cost", "Landed cost"), ("payroll", "Payroll"), ("tax_deposit", "Tax deposits"), ("vat_return", "VAT returns"), ("bank_transfer", "Bank transfers"), ("stock_adjustment", "Stock adjustments"), ("fx_revaluation", "FX revaluation"), ("reversal", "Reversals")];
static JOURNALS: ListCfg = ListCfg { title: "Journals", subtitle: "", endpoint: "/journals", cols: &[col("entry_no", "Journal", CK::Bold), col("entry_date", "Date", CK::Date), col("source_type", "Source", CK::Badge), col("memo", "Description", CK::Text), col("total", "Amount", CK::Money), col("status", "Status", CK::Badge)],
    search: true, filters: &[Filter { param: "source_type", label: "All sources", options: SOURCES }], fields: &[], write: "finance.post", new_label: "Post a journal", edit: false, delete: false, detail: "/journals/", new_route: "/journals/new", extra_query: "", empty_hint: "Every financial event in the business appears here.", date_filter: true };

#[component]
pub fn Ledger() -> impl IntoView {
    let tab = RwSignal::new("journals");
    view! { <div class="page"><PageHead title=t("Ledger Studio") subtitle="The single source of truth. Every document posts balanced journals here; nothing can be changed, only reversed."/>
        <Tabs tabs=vec![("journals", "Journals".to_string()), ("tb", "Trial balance".to_string()), ("gl", "Account ledger".to_string()), ("checks", "Integrity checks".to_string())] active=tab/>
        {move || match tab.get() { "journals" => view! { <ListPage cfg=&JOURNALS headless=true/> }.into_any(), "tb" => view! { <TrialBalance/> }.into_any(), "gl" => view! { <AccountLedger/> }.into_any(), _ => view! { <Checks/> }.into_any() }}</div> }
}

#[component]
fn TrialBalance() -> impl IntoView {
    let as_of = RwSignal::new(fmt::today());
    let res = LocalResource::new(move || { let d = as_of.get(); async move { api::get(&format!("/reports/trial-balance{}", api::qs(&[("as_of", d)]))).await } });
    view! { <div class="card flush"><div class="toolbar"><label class="small mut">"As of"</label><input class="input" type="date" prop:value=move || as_of.get() on:change=move |e| as_of.set(event_target_value(&e))/></div>
        {move || match res.get() { None => view! { <div style="padding:20px"><div class="skel"></div></div> }.into_any(), Some(Err(e)) => view! { <div style="padding:16px"><ErrorBox err=e/></div> }.into_any(), Some(Ok(v)) => { let d = v["data"].clone(); let rows = d["accounts"].as_array().cloned().unwrap_or_default();
            view! { <div class="tbl-wrap"><table><thead><tr><th>"Code"</th><th>"Account"</th><th>"Type"</th><th class="r">"Debit"</th><th class="r">"Credit"</th></tr></thead><tbody>
                {rows.into_iter().map(|r| view! { <tr><td class="mono">{fmt::text(&r["code"])}</td><td>{fmt::text(&r["name"])}</td><td><Badge status=fmt::text(&r["account_type"])/></td><td class="r num">{if fmt::num(&r["balance_dr"]) > 0.0 { fmt::money(&r["balance_dr"]) } else { String::new() }}</td><td class="r num">{if fmt::num(&r["balance_cr"]) > 0.0 { fmt::money(&r["balance_cr"]) } else { String::new() }}</td></tr> }).collect_view()}
                <tr><td colspan="3"><b>"Total"</b></td><td class="r num"><b>{fmt::money(&d["total_debit"])}</b></td><td class="r num"><b>{fmt::money(&d["total_credit"])}</b></td></tr></tbody></table></div>
                <div class="pager"><Badge status=if d["balanced"].as_bool().unwrap_or(false) { "ok" } else { "bad" } label=if d["balanced"].as_bool().unwrap_or(false) { "Balanced" } else { "Out of balance" }/></div> }.into_any() } }}</div> }
}

#[component]
fn AccountLedger() -> impl IntoView {
    const F: &[FF] = &[ff("account_id", "Account", FK::Lookup("/accounts?is_group=false", &["code", "name"]))];
    let st = FormState::new(F, None);
    let res = LocalResource::new(move || { let a = st.get("account_id"); async move { if a.is_empty() { return Ok(Value::Null); } api::get(&format!("/reports/general-ledger?account_id={a}&per_page=100")).await } });
    view! { <div class="card flush"><div class="toolbar" style="max-width:560px"><Input f=F[0] st=st compact=true/></div>
        {move || match res.get() { None => view! { <div style="padding:20px"><div class="skel"></div></div> }.into_any(), Some(Err(e)) => view! { <div style="padding:16px"><ErrorBox err=e/></div> }.into_any(), Some(Ok(Value::Null)) => view! { <Empty title="Choose an account" hint="See every posting with a running balance."/> }.into_any(),
            Some(Ok(v)) => table(&[col("entry_date", "Date", CK::Date), col("entry_no", "Journal", CK::Mono), col("source_type", "Source", CK::Badge), col("memo", "Description", CK::Text), col("debit", "Debit", CK::Money), col("credit", "Credit", CK::Money), col("balance", "Balance", CK::Money)], v["data"].as_array().cloned().unwrap_or_default(), None) }}</div> }
}

#[component]
fn Checks() -> impl IntoView {
    let tb = LocalResource::new(|| async { api::get("/reports/trial-balance").await });
    let bs = LocalResource::new(|| async { api::get("/reports/balance-sheet").await });
    let sv = LocalResource::new(|| async { api::get("/reports/stock-valuation").await });
    fn item(title: &'static str, text: &'static str, ok: Option<bool>) -> AnyView {
        let badge = match ok { Some(true) => view! { <Badge status="ok" label="Passed"/> }.into_any(), Some(false) => view! { <Badge status="bad" label="Needs attention"/> }.into_any(), None => view! { <Spinner/> }.into_any() };
        view! {
            <div class="card row">
                <div style="width:44px;height:44px;border-radius:14px;display:grid;place-items:center;background:var(--accent-soft);color:var(--teal-d)"><Icon name=if ok == Some(false) { "alert" } else { "check" } size=22/></div>
                <div><b>{title}</b><div class="mut small">{text}</div></div>
                <span class="sp">{badge}</span>
            </div>
        }.into_any()
    }
    let flag = |r: Option<api::ApiResult>, key: &'static str| r.map(|x| x.map(|v| v["data"][key].as_bool().unwrap_or(false)).unwrap_or(false));
    view! {
        <div class="grid" style="max-width:860px">
            {move || item("Debits equal credits", "Across the whole company, every taka debited is credited somewhere.", flag(tb.get(), "balanced"))}
            {move || item("Balance sheet balances", "Assets = liabilities + equity (including current profit).", flag(bs.get(), "balanced"))}
            {move || item("Stock records match the ledger", "The value of stock in warehouses equals the inventory account.", flag(sv.get(), "reconciled"))}
            <div class="alert info"><Icon name="shield"/><div>"These checks use live data. Posted entries are immutable and period closing locks the past, so figures cannot quietly change."</div></div>
        </div>
    }
}

static JOURNAL_DOC: DocCfg = DocCfg { endpoint: "/journals", list_route: "/ledger", list_label: "Ledger Studio", title_key: "entry_no", subtitle_key: "memo", status_key: "status", steps: &[],
    facts: &[fact("Date", "entry_date", CK::Date), fact("Source", "source_type", CK::Badge), fact("Amount", "total", CK::Money), fact("Posted by", "created_by_name", CK::Text), fact("Posted at", "created_at", CK::DateTime)],
    tables: &[dtable("Lines", "lines", &[col("account_code", "Code", CK::Mono), col("account_name", "Account", CK::Text), col("party_type", "Party", CK::Text), col("memo", "Memo", CK::Text), col("debit", "Debit", CK::Money), col("credit", "Credit", CK::Money), col("fc_amount", "Foreign amount", CK::Qty), col("currency", "Cur.", CK::Text)])],
    actions: &[action("Reverse", "/reverse", "finance.post").when(&["posted"]).danger().fields(&[ff("reason", "Reason", FK::Text).req().full(), ff("date", "Reversal date", FK::Date).def("today")]).done("Reversed").goto("/journals/{rid}")], notes_key: "" };
#[component] pub fn JournalPage() -> impl IntoView { let id = use_params_map().with_untracked(|p| p.get("id").unwrap_or_default()); view! { <div class="page"><DocPage cfg=&JOURNAL_DOC id=id/></div> } }

#[component]
pub fn NewJournal() -> impl IntoView {
    const H: &[FF] = &[ff("entry_date", "Date", FK::Date).req().def("today"), ff("memo", "What is this for?", FK::Text).req().full()];
    const L: &[FF] = &[ff("account_id", "Account", FK::Lookup("/accounts?is_group=false", &["code", "name"])).req(), ff("debit", "Debit", FK::Dec), ff("credit", "Credit", FK::Dec), ff("memo", "Note", FK::Text)];
    let st = FormState::new(H, None);
    let rows: RwSignal<Vec<Map<String, Value>>> = RwSignal::new((0..2).map(|_| { let mut m = Map::new(); for c in L { m.insert(c.name.into(), json!("")); } m }).collect());
    let nav = use_navigate();
    let sums = move || { let r = rows.get(); let g = |k: &str| r.iter().map(|m| m.get(k).and_then(|v| v.as_str()).and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0)).sum::<f64>(); (g("debit"), g("credit")) };
    let submit = move |_| { if st.busy.get_untracked() { return; } st.busy.set(true); st.err.set(None); let mut body = payload(H, &st);
        let lines: Vec<Value> = rows.get_untracked().iter().filter(|m| m.get("account_id").and_then(|v| v.as_str()).map(|s| !s.is_empty()).unwrap_or(false)).map(|m| { let mut o = Map::new(); for k in ["account_id", "memo"] { if let Some(v) = m.get(k).and_then(|v| v.as_str()).filter(|s| !s.is_empty()) { o.insert(k.into(), json!(v)); } } for k in ["debit", "credit"] { if let Some(v) = m.get(k).and_then(|v| v.as_str()).filter(|s| !s.is_empty()) { o.insert(k.into(), json!(v)); } } Value::Object(o) }).collect();
        if let Value::Object(m) = &mut body { m.insert("lines".into(), json!(lines)); } let nav = nav.clone();
        leptos::task::spawn_local(async move { let r = api::post("/journals", body).await; st.busy.set(false); match r { Ok(v) => { state::ok("Journal posted"); nav(&format!("/journals/{}", v["data"]["id"].as_str().unwrap_or("")), Default::default()); } Err(e) => st.err.set(Some(e)) } }); };
    view! { <div class="page"><PageHead title="Post a journal" subtitle="For adjustments the system does not make by itself. Debits must equal credits." crumbs=vec![(t("Ledger Studio"), "/ledger".to_string())]/>
        <div class="grid" style="max-width:1100px"><Card><FormFields fields=H.to_vec() st=st/></Card>
        <Card title="Lines"><LinesEditor cols=L.to_vec() rows=rows err=st.err/>
            <div class="row" style="margin-top:14px;justify-content:flex-end;gap:24px"><span>"Debit "<b class="num">{move || fmt::money_f(sums().0)}</b></span><span>"Credit "<b class="num">{move || fmt::money_f(sums().1)}</b></span>
                {move || { let (d, c) = sums(); if (d - c).abs() < 0.005 && d > 0.0 { view! { <Badge status="ok" label="Balanced"/> }.into_any() } else { view! { <Badge status="warn" label=format!("Difference {}", fmt::money_f(d - c))/> }.into_any() } }}</div></Card>
        <FormError st=st/><div class="row"><a class="btn" href="/ledger">{t("Cancel")}</a><button class="btn primary sp" disabled=move || st.busy.get() on:click=submit>"Post journal"</button></div></div></div> }
}
