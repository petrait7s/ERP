//! Customs: Bills of Entry, the duty calculator and the HS code tariff.
use super::common::*;
use crate::{api, crud::*, fmt, i18n::t, state, ui::*};
use leptos::prelude::*;
use leptos_router::hooks::use_params_map;
use serde_json::{json, Value};

const BE_COLS: &[Col] = &[col("be_no", "Bill of Entry", CK::Bold), col("be_date", "Date", CK::Date), col("shipment_no", "Shipment", CK::Mono), col("supplier_name", "Supplier", CK::Text), col("assessable_value", "Assessable value", CK::Money), col("total_payable", "Duties payable", CK::Money), col("status", "Status", CK::Badge)];
static BE_LIST: ListCfg = ListCfg { title: "Bills of Entry", subtitle: "", endpoint: "/bills-of-entry", cols: BE_COLS, search: true, filters: &[Filter { param: "status", label: "All statuses", options: &[("assessed", "Assessed"), ("paid", "Paid"), ("released", "Released")] }], fields: &[], write: "import.write", new_label: "Lodge Bill of Entry", edit: false, delete: false, detail: "/bills-of-entry/", new_route: "/bills-of-entry/new", extra_query: "", empty_hint: "Lodge a Bill of Entry when a shipment is on its way.", date_filter: true };
const HS_FIELDS: &[FF] = &[ff("code", "HS code", FK::Text).req().help("e.g. 9405.42.00"), ff("description", "Description", FK::Text).req().full(), ff("cd_rate", "Customs duty %", FK::Dec).def("0"), ff("rd_rate", "Regulatory duty %", FK::Dec).def("0"), ff("sd_rate", "Supplementary duty %", FK::Dec).def("0"), ff("vat_rate", "VAT %", FK::Dec).def("15"), ff("ait_rate", "Advance income tax %", FK::Dec).def("5"), ff("at_rate", "Advance tax %", FK::Dec).def("5"), ff("is_active", "Active", FK::Bool).def("true")];
static HS_LIST: ListCfg = ListCfg { title: "HS Codes", subtitle: "", endpoint: "/hs-codes", cols: &[col("code", "HS code", CK::Bold), col("description", "Description", CK::Text), col("cd_rate", "CD %", CK::Pct), col("rd_rate", "RD %", CK::Pct), col("sd_rate", "SD %", CK::Pct), col("vat_rate", "VAT %", CK::Pct), col("ait_rate", "AIT %", CK::Pct), col("at_rate", "AT %", CK::Pct), col("is_active", "Active", CK::Bool)],
    search: true, filters: &[], fields: HS_FIELDS, write: "catalog.write", new_label: "New HS code", edit: true, delete: true, detail: "", new_route: "", extra_query: "", empty_hint: "Add the tariff lines you import under. Rates can be changed whenever the tariff changes.", date_filter: false };

#[component]
pub fn Customs() -> impl IntoView {
    let tab = RwSignal::new("be");
    view! { <div class="page"><PageHead title=t("Customs") subtitle="Bills of Entry, duty estimates and your tariff schedule. Duties are worked out per product from its HS code."/>
        <Tabs tabs=vec![("be", t("Bills of Entry")), ("calc", "Duty calculator".to_string()), ("hs", t("HS Codes"))] active=tab/>
        {move || match tab.get() { "be" => view! { <ListPage cfg=&BE_LIST headless=true/> }.into_any(), "calc" => view! { <DutyCalculator/> }.into_any(), _ => view! { <ListPage cfg=&HS_LIST headless=true/> }.into_any() }}</div> }
}

#[component]
fn DutyCalculator() -> impl IntoView {
    const F: &[FF] = &[ff("hs_code_id", "HS code", FK::Lookup("/hs-codes", &["code", "description"])).req(), ff("cif_value", "CIF value (foreign currency)", FK::Dec).req().help("Cost + insurance + freight"), ff("fx_rate", "Customs exchange rate (BDT)", FK::Dec).req().def("122")];
    let st = FormState::new(F, None);
    let out: RwSignal<Option<Value>> = RwSignal::new(None);
    let calc = move |_| { st.busy.set(true); st.err.set(None); let body = payload(F, &st);
        leptos::task::spawn_local(async move { match api::post("/customs/duty-calculator", body).await { Ok(v) => out.set(Some(v["data"].clone())), Err(e) => st.fail(e) } st.done(); }); };
    view! { <div class="grid g2" style="align-items:start">
        <Card title="Estimate import duties"><FormFields fields=F.to_vec() st=st/><FormError st=st/><div style="margin-top:16px"><button class="btn primary" disabled=move || st.busy.get() on:click=calc>"Calculate"</button></div>
            <p class="help" style="margin-top:12px">"Assessable value = CIF in Taka + landing charge. SD applies on AV + CD + RD; VAT and AT on AV + CD + RD + SD."</p></Card>
        {move || out.get().map(|o| { let d = o["duty"].clone(); view! { <Card title="Result">
            <div class="facts"><div class="fact"><small>"CIF in Taka"</small><div>{fmt::money(&o["cif_bdt"])}</div></div><div class="fact"><small>"Assessable value"</small><div>{fmt::money(&d["assessable_value"])}</div></div></div>
            <table style="margin-top:14px"><tbody>
                {[("Customs duty (CD)", "cd"), ("Regulatory duty (RD)", "rd"), ("Supplementary duty (SD)", "sd"), ("VAT", "vat"), ("Advance income tax (AIT)", "ait"), ("Advance tax (AT)", "at")].into_iter().map(|(l, k)| view! { <tr><td>{l}</td><td class="r num">{fmt::money(&d[k])}</td></tr> }).collect_view()}
                <tr><td><b>"Total payable to customs"</b></td><td class="r num"><b>{fmt::money(&o["total_payable"])}</b></td></tr></tbody></table>
            <div class="alert info" style="margin-top:14px"><Icon name="check"/><div>{format!("Added to product cost: {} · Recoverable (VAT, AT, AIT): {}", fmt::money(&o["added_to_cost"]), fmt::money(&o["recoverable"]))}</div></div></Card> } })}
    </div> }
}

static NEW_BE: NewDoc = NewDoc { title: "Lodge a Bill of Entry", subtitle: "Duties are calculated for every item on the shipment from its HS code.", endpoint: "/bills-of-entry", back_label: "Customs", back_route: "/customs",
    header: &[ff("shipment_id", "Shipment", FK::Lookup("/shipments?active=true", &["shipment_no", "supplier_name"])).req(), ff("be_no", "Bill of Entry number", FK::Text).req(), ff("be_date", "Date", FK::Date).def("today"), ff("customs_rate", "Customs exchange rate", FK::Dec).req(), ff("cnf_agent", "C&F agent", FK::Text), ff("other_fees", "Other customs fees (Tk)", FK::Dec)],
    lines_title: "", lines: &[], autofill: None, goto: "/bills-of-entry/{rid}", done: "Duties assessed", submit_label: "Lodge", min_lines: 0 };
#[component] pub fn NewBe() -> impl IntoView { view! { <NewDocPage cfg=&NEW_BE/> } }

static BE_DOC: DocCfg = DocCfg {
    endpoint: "/bills-of-entry", list_route: "/customs", list_label: "Customs", title_key: "be_no", subtitle_key: "supplier_name", status_key: "status", steps: &[("assessed", "Assessed"), ("paid", "Duties paid"), ("released", "Released")],
    facts: &[fact("Shipment", "shipment_no", CK::Mono), fact("Date", "be_date", CK::Date), fact("Customs station", "customs_station", CK::Text), fact("C&F agent", "cnf_agent", CK::Text), fact("Customs rate", "customs_rate", CK::Qty), fact("Assessable value", "assessable_value", CK::Money),
        fact("Customs duty", "cd", CK::Money), fact("Regulatory duty", "rd", CK::Money), fact("Supplementary duty", "sd", CK::Money), fact("VAT", "vat", CK::Money), fact("Advance income tax", "ait", CK::Money), fact("Advance tax", "at", CK::Money), fact("Other fees", "other_fees", CK::Money),
        fact("Total payable", "total_payable", CK::Money), fact("Added to cost", "capitalised", CK::Money), fact("Recoverable", "recoverable", CK::Money)],
    tables: &[dtable("Duty per item", "lines", &[col("sku", "SKU", CK::Mono), col("product_name", "Product", CK::Text), col("hs_code", "HS code", CK::Mono), col("cif_value", "CIF (foreign)", CK::Qty), col("assessable_value", "Assessable", CK::Money), col("cd", "CD", CK::Money), col("rd", "RD", CK::Money), col("sd", "SD", CK::Money), col("vat", "VAT", CK::Money), col("ait", "AIT", CK::Money), col("at", "AT", CK::Money)])],
    actions: &[
        action("Pay duties", "/pay", "import.pay").when(&["assessed"]).primary().fields(&[ff("bank_account_id", "Pay from", FK::Lookup("/bank-accounts?kind=bank", &["name"])).req(), ff("paid_date", "Date", FK::Date).def("today")]).done("Duties paid"),
        action("Customs released", "/release", "import.write").when(&["paid"]).primary().fields(&[ff("released_date", "Release date", FK::Date).def("today")]).done("Goods released by customs"),
        action("Remove", "", "import.write").when(&["assessed"]).method("DELETE").danger().confirm("Remove this assessment? You can lodge the Bill of Entry again.").done("Removed").goto("/customs"),
    ], notes_key: "" };
#[component] pub fn BePage() -> impl IntoView { let id = use_params_map().with_untracked(|p| p.get("id").unwrap_or_default()); view! { <div class="page"><DocPage cfg=&BE_DOC id=id/></div> } }
