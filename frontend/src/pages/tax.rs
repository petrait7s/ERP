//! Bangladesh VAT and tax: monthly return (Mushak 9.1), registers (6.1, 6.2, 6.10) and treasury deposits.
use super::common::*;
use crate::{api, crud::*, fmt, i18n::t, state, ui::*};
use leptos::prelude::*;
use serde_json::{json, Value};

static M61: ListCfg = ListCfg { title: "Purchase register (Mushak 6.1)", subtitle: "", endpoint: "/tax/mushak-6-1", cols: &[col("date", "Date", CK::Date), col("kind", "Kind", CK::Badge), col("reference", "Reference", CK::Mono), col("tax_document", "Tax document", CK::Mono), col("party", "Supplier", CK::Text), col("party_bin", "BIN", CK::Mono), col("value", "Value", CK::Money), col("duties", "Import duties", CK::Money), col("vat", "VAT", CK::Money)],
    search: false, filters: &[], fields: &[], write: "", new_label: "", edit: false, delete: false, detail: "", new_route: "", extra_query: "", empty_hint: "", date_filter: true };
static M62: ListCfg = ListCfg { title: "Sales register (Mushak 6.2)", subtitle: "", endpoint: "/tax/mushak-6-2", cols: &[col("date", "Date", CK::Date), col("mushak_no", "Mushak 6.3 no.", CK::Mono), col("invoice_no", "Invoice", CK::Mono), col("customer", "Customer", CK::Text), col("customer_bin", "BIN", CK::Mono), col("value", "Value", CK::Money), col("sd", "SD", CK::Money), col("vat", "VAT", CK::Money), col("total", "Total", CK::Money)],
    search: false, filters: &[], fields: &[], write: "", new_label: "", edit: false, delete: false, detail: "", new_route: "", extra_query: "", empty_hint: "", date_filter: true };
static M610: ListCfg = ListCfg { title: "Large transactions (Mushak 6.10)", subtitle: "", endpoint: "/tax/mushak-6-10", cols: &[col("date", "Date", CK::Date), col("direction", "Direction", CK::Badge), col("document", "Document", CK::Mono), col("party", "Party", CK::Text), col("party_bin", "BIN", CK::Mono), col("amount", "Amount", CK::Money), col("vat", "VAT", CK::Money)],
    search: false, filters: &[], fields: &[], write: "", new_label: "", edit: false, delete: false, detail: "", new_route: "", extra_query: "", empty_hint: "Transactions above the threshold in Business Policies appear here.", date_filter: true };

#[component]
pub fn Tax() -> impl IntoView {
    let tab = RwSignal::new("return");
    view! { <div class="page"><PageHead title=t("VAT & Tax") subtitle="Your monthly VAT return, registers for the tax authority, and what you owe the treasury."/>
        <Tabs tabs=vec![("return", "VAT return (9.1)".to_string()), ("m61", "Purchases (6.1)".to_string()), ("m62", "Sales (6.2)".to_string()), ("m610", "Large (6.10)".to_string()), ("dep", "Treasury deposits".to_string())] active=tab/>
        {move || match tab.get() { "return" => view! { <VatReturn/> }.into_any(), "m61" => view! { <ListPage cfg=&M61 headless=true/> }.into_any(), "m62" => view! { <ListPage cfg=&M62 headless=true/> }.into_any(), "m610" => view! { <ListPage cfg=&M610 headless=true/> }.into_any(), _ => view! { <Deposits/> }.into_any() }}</div> }
}

#[component]
fn VatReturn() -> impl IntoView {
    let now = js_sys::Date::new_0();
    let year = RwSignal::new(now.get_full_year() as i32); let month = RwSignal::new(now.get_month() + 1);
    let tick = state::app().tick;
    let res = LocalResource::new(move || { let _ = tick.get(); let (y, m) = (year.get(), month.get()); async move { api::get(&format!("/tax/vat-return?year={y}&month={m}")).await } });
    let act = move |submit: bool| { let (y, m) = (year.get_untracked(), month.get_untracked()); leptos::task::spawn_local(async move {
        let r = api::post(&format!("/tax/vat-returns?year={y}&month={m}"), json!({})).await;
        match r { Ok(v) => { if submit { match api::post(&format!("/tax/vat-returns/{}/submit", v["data"]["id"].as_str().unwrap_or("")), json!({})).await { Ok(_) => { state::ok("VAT return submitted"); state::refresh(); } Err(e) => state::fail(e.friendly()) } } else { state::ok("Draft saved"); state::refresh(); } } Err(e) => state::fail(e.friendly()) } }); };
    view! { <div class="card"><div class="row wrap" style="margin-bottom:18px"><select class="select" style="width:auto" prop:value=move || month.get().to_string() on:change=move |e| month.set(event_target_value(&e).parse().unwrap_or(1))>{(1..=12u32).map(|m| view! { <option value=m.to_string()>{["January","February","March","April","May","June","July","August","September","October","November","December"][(m - 1) as usize]}</option> }).collect_view()}</select>
        <input class="input" style="width:110px" type="number" prop:value=move || year.get().to_string() on:change=move |e| year.set(event_target_value(&e).parse().unwrap_or(2026))/></div>
        {move || match res.get() { None => view! { <div class="skel"></div> }.into_any(), Some(Err(e)) => view! { <ErrorBox err=e/> }.into_any(), Some(Ok(v)) => { let d = v["data"].clone(); let f = d["figures"].clone(); let net = fmt::num(&f["net_payable"]); let saved = d["saved_return"].clone();
            view! { <table><tbody>
                {[("Output VAT on sales", "output_vat", false), ("Less: VAT on credit notes", "credit_notes_vat", true), ("Less: Input VAT on local purchases", "input_vat_local", true), ("Less: Input VAT paid at import", "input_vat_import", true), ("Less: Advance tax (AT) paid at import", "advance_tax", true), ("Less: VAT withheld by customers (VDS certificates)", "vds_by_customers", true)].into_iter().map(|(l, k, minus)| view! { <tr><td>{l}</td><td class="r num">{if minus { format!("− {}", fmt::money(&f[k])) } else { fmt::money(&f[k]) }}</td></tr> }).collect_view()}
                <tr><td><b>{if net >= 0.0 { "VAT payable to the treasury" } else { "Credit carried forward" }}</b></td><td class="r num"><b class={if net >= 0.0 { "" } else { "up" }}>{fmt::money_f(net.abs())}</b></td></tr>
                <tr><td class="dim small">"Supplementary duty collected (deposited separately)"</td><td class="r num dim">{fmt::money(&f["output_sd"])}</td></tr>
                <tr><td class="dim small">"VDS you withheld from suppliers (deposit separately)"</td><td class="r num dim">{fmt::money(&f["vds_on_suppliers"])}</td></tr></tbody></table>
                <div class="row" style="margin-top:18px">{match saved["status"].as_str() { Some("submitted") => view! { <Badge status="ok" label="Submitted"/> }.into_any(), Some(_) => view! { <Badge status="warn" label="Draft saved"/> }.into_any(), None => view! { <span class="dim small">"Not saved yet"</span> }.into_any() }}
                    {(state::can("tax.write") && saved["status"].as_str() != Some("submitted")).then(|| view! { <span class="sp row"><button class="btn" on:click=move |_| act(false)>"Save draft"</button><button class="btn primary" on:click=move |_| act(true)>"Submit return"</button></span> })}</div> }.into_any() } }}</div> }
}

#[component]
fn Deposits() -> impl IntoView {
    let tick = state::app().tick;
    let res = LocalResource::new(move || { let _ = tick.get(); async { api::get("/tax/liabilities").await } });
    let dialog: RwSignal<Option<(&'static str, &'static str)>> = RwSignal::new(None);
    view! { <div class="grid g3">{move || match res.get() { None => view! { <Loading/> }.into_any(), Some(Err(e)) => view! { <ErrorBox err=e/> }.into_any(), Some(Ok(v)) => { let d = v["data"].clone();
        [("vat", "VAT payable (after return)", "vat_payable"), ("sd", "Supplementary duty", "supplementary_duty"), ("vds", "VDS withheld from suppliers", "vds"), ("tds", "Tax withheld (TDS)", "tds"), ("pf", "Provident fund", "provident_fund")].into_iter().map(|(kind, label, key)| { let amt = fmt::num(&d[key]);
            view! { <div class="card"><div class="lbl dim small">{label}</div><div class="val num" style="font-size:24px;font-weight:700;margin:6px 0 12px">{fmt::money_f(amt)}</div>{(amt > 0.0 && state::can("tax.write")).then(|| view! { <button class="btn sm" on:click=move |_| dialog.set(Some((kind, label)))>"Record deposit"</button> })}</div> } }).collect_view().into_any() } }}
        {move || dialog.get().map(|(kind, label)| view! { <DepositDialog kind=kind label=label on_close=closer(move || dialog.set(None))/> })}</div> }
}
#[component]
fn DepositDialog(kind: &'static str, label: &'static str, on_close: Callback<()>) -> impl IntoView {
    const F: &[FF] = &[ff("amount", "Amount (Tk)", FK::Dec).req(), ff("bank_account_id", "Paid from", FK::Lookup("/bank-accounts?currency=BDT", &["name"])).req(), ff("deposit_date", "Date", FK::Date).def("today"), ff("challan_no", "Treasury challan no.", FK::Text)];
    let st = FormState::new(F, None);
    let save = move |_| { st.busy.set(true); st.err.set(None); let mut b = payload(F, &st); if let Value::Object(m) = &mut b { m.insert("kind".into(), json!(kind)); }
        leptos::task::spawn_local(async move { let r = api::post("/tax/deposits", b).await; st.done(); match r { Ok(_) => { state::ok("Deposit recorded"); state::refresh(); on_close.run(()); } Err(e) => st.fail(e) } }); };
    view! { <Modal title=format!("Deposit: {label}") on_close=on_close footer=footer(move || view! { <button class="btn" on:click=move |_| on_close.run(())>{t("Cancel")}</button><button class="btn primary" disabled=move || st.busy.get() on:click=save>"Record deposit"</button> }.into_any())><FormFields fields=F.to_vec() st=st/><FormError st=st/></Modal> }
}
