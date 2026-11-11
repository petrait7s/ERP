//! Accounting & finance: chart of accounts, bank and cash, transfers, supplier payments, fiscal periods.
use super::common::*;
use crate::{api, crud::*, fmt, i18n::t, state, ui::*};
use leptos::prelude::*;
use leptos_router::hooks::use_navigate;
use serde_json::{json, Value};

const ACC_FIELDS: &[FF] = &[ff("code", "Code", FK::Text).req(), ff("name", "Name", FK::Text).req(), ff("name_bn", "Name in Bangla", FK::Text), ff("account_type", "Type", FK::Select(&[("asset", "Asset"), ("liability", "Liability"), ("equity", "Equity"), ("income", "Income"), ("expense", "Expense")])).req(), ff("parent_id", "Under", FK::Lookup("/accounts?is_group=true", &["code", "name"])), ff("is_group", "Heading account (holds other accounts)", FK::Bool), ff("is_active", "Active", FK::Bool).def("true")];
static ACCOUNTS: ListCfg = ListCfg { title: "Chart of Accounts", subtitle: "", endpoint: "/accounts", cols: &[col("code", "Code", CK::Mono), col("name", "Account", CK::Bold), col("account_type", "Type", CK::Badge), col("balance", "Balance (Dr − Cr)", CK::Money), col("is_group", "Heading", CK::Bool), col("is_active", "Active", CK::Bool)],
    search: true, filters: &[Filter { param: "account_type", label: "All types", options: &[("asset", "Assets"), ("liability", "Liabilities"), ("equity", "Equity"), ("income", "Income"), ("expense", "Expenses")] }], fields: ACC_FIELDS, write: "finance.write", new_label: "New account", edit: true, delete: true, detail: "", new_route: "", extra_query: "", empty_hint: "", date_filter: false };
const BANK_FIELDS: &[FF] = &[ff("code", "Short code", FK::Text).req(), ff("name", "Name", FK::Text).req().full(), ff("kind", "Kind", FK::Select(&[("bank", "Bank account"), ("cash", "Cash box"), ("mobile", "Mobile wallet (bKash, Nagad)")])).req().def("bank"), ff("currency", "Currency", FK::Select(super::procurement::CURRENCIES)).req().def("BDT"), ff("bank_name", "Bank", FK::Text), ff("branch_name", "Branch", FK::Text), ff("account_no", "Account number", FK::Text), ff("routing_no", "Routing number", FK::Text), ff("swift_code", "SWIFT code", FK::Text)];
static BANKS: ListCfg = ListCfg { title: "Bank & Cash", subtitle: "", endpoint: "/bank-accounts", cols: &[col("code", "Code", CK::Mono), col("name", "Account", CK::Bold), col("kind", "Kind", CK::Badge), col("currency", "Currency", CK::Text), col("account_no", "Number", CK::Mono), col("balance_bdt", "Balance (Tk)", CK::Money), col("balance_fc", "Balance (foreign)", CK::Qty)],
    search: false, filters: &[], fields: BANK_FIELDS, write: "finance.write", new_label: "New account", edit: false, delete: false, detail: "", new_route: "", extra_query: "", empty_hint: "Add your bank accounts and cash box.", date_filter: false };
static TRANSFERS: ListCfg = ListCfg { title: "Transfers", subtitle: "", endpoint: "/bank-transfers", cols: &[col("transfer_no", "Transfer", CK::Bold), col("transfer_date", "Date", CK::Date), col("from_account", "From", CK::Text), col("to_account", "To", CK::Text), col("amount", "Amount", CK::Money), col("charges", "Charges", CK::Money), col("reference", "Reference", CK::Text)],
    search: false, filters: &[], fields: &[], write: "finance.write", new_label: "New transfer", edit: false, delete: false, detail: "", new_route: "/bank-transfers/new", extra_query: "", empty_hint: "", date_filter: true };
static PAYMENTS: ListCfg = ListCfg { title: "Supplier Payments", subtitle: "", endpoint: "/supplier-payments", cols: &[col("payment_no", "Payment", CK::Bold), col("payment_date", "Date", CK::Date), col("supplier_name", "Supplier", CK::Text), col("gross_amount", "Bills settled", CK::Money), col("tds_amount", "TDS", CK::Money), col("vds_amount", "VDS", CK::Money), col("net_amount", "Paid", CK::Money), col("bank_account", "From", CK::Text), col("status", "Status", CK::Badge)],
    search: false, filters: &[Filter { param: "status", label: "All", options: &[("pending_approval", "Waiting for approval"), ("posted", "Posted"), ("rejected", "Rejected")] }], fields: &[], write: "finance.write", new_label: "Pay bills", edit: false, delete: false, detail: "", new_route: "/supplier-payments/new", extra_query: "", empty_hint: "", date_filter: true };

#[component]
pub fn Accounting() -> impl IntoView {
    let tab = RwSignal::new("coa");
    view! { <div class="page"><PageHead title=t("Accounting & Finance") subtitle="Your accounts, banks, payments and accounting periods."/>
        <Tabs tabs=vec![("coa", t("Chart of Accounts")), ("bank", t("Bank & Cash")), ("tr", t("Transfers")), ("pay", t("Supplier Payments")), ("per", t("Periods"))] active=tab/>
        {move || match tab.get() { "coa" => view! { <ListPage cfg=&ACCOUNTS headless=true/> }.into_any(), "bank" => view! { <ListPage cfg=&BANKS headless=true/> }.into_any(), "tr" => view! { <ListPage cfg=&TRANSFERS headless=true/> }.into_any(), "pay" => view! { <ListPage cfg=&PAYMENTS headless=true/> }.into_any(), _ => view! { <Periods/> }.into_any() }}</div> }
}

#[component]
fn Periods() -> impl IntoView {
    let tick = state::app().tick;
    let res = LocalResource::new(move || { let _ = tick.get(); async { api::get("/fiscal-periods").await } });
    let act = move |id: String, what: &'static str| leptos::task::spawn_local(async move { match api::post(&format!("/fiscal-periods/{id}/{what}"), json!({})).await { Ok(_) => { state::ok(if what == "close" { "Period closed" } else { "Period reopened" }); state::refresh(); } Err(e) => state::fail(e.friendly()) } });
    view! { <div class="card flush"><div class="toolbar"><span class="mut small">"Closing a month locks it: nothing more can be posted into it."</span>
        {move || state::can("finance.post").then(|| view! { <button class="btn sm sp" on:click=move |_| { let y = js_sys::Date::new_0().get_full_year() as i32; leptos::task::spawn_local(async move { match api::post("/fiscal-periods/generate", json!({"start_year": y + 1})).await { Ok(_) => { state::ok("Next year's periods created"); state::refresh(); } Err(e) => state::fail(e.friendly()) } }); }>"Create next fiscal year"</button> })}</div>
        {move || match res.get() { None => view! { <div style="padding:20px"><div class="skel"></div></div> }.into_any(), Some(Err(e)) => view! { <div style="padding:16px"><ErrorBox err=e/></div> }.into_any(), Some(Ok(v)) => { let list = v["data"].as_array().cloned().unwrap_or_default();
            view! { <div class="tbl-wrap"><table><thead><tr><th>"Period"</th><th>"From"</th><th>"To"</th><th>"Status"</th><th></th></tr></thead><tbody>{list.into_iter().map(|p| { let id = fmt::text(&p["id"]); let id2 = id.clone(); let closed = p["status"] == "closed";
                view! { <tr><td><b>{fmt::text(&p["name"])}</b></td><td>{fmt::date(&p["start_date"])}</td><td>{fmt::date(&p["end_date"])}</td><td><Badge status=if closed { "closed_period".to_string() } else { "ok".to_string() } label=if closed { "Closed" } else { "Open" }/></td>
                    <td class="r">{state::can("finance.post").then(|| if closed { view! { <button class="btn sm" on:click=move |_| act(id.clone(), "reopen")>"Reopen"</button> }.into_any() } else { view! { <button class="btn sm" on:click=move |_| act(id2.clone(), "close")>"Close month"</button> }.into_any() })}</td></tr> } }).collect_view()}</tbody></table></div> }.into_any() } }}</div> }
}

static NEW_TRANSFER: NewDoc = NewDoc { title: "Transfer between accounts", subtitle: "Move money between your own bank, cash and mobile accounts. Different currencies need the amount received too.", endpoint: "/bank-transfers", back_label: "Accounting & Finance", back_route: "/accounting",
    header: &[ff("from_bank_account_id", "From", FK::Lookup("/bank-accounts", &["name", "currency"])).req(), ff("to_bank_account_id", "To", FK::Lookup("/bank-accounts", &["name", "currency"])).req(), ff("transfer_date", "Date", FK::Date).def("today"), ff("amount", "Amount leaving", FK::Dec).req(), ff("to_amount", "Amount arriving", FK::Dec).help("Only when the two accounts use different currencies"), ff("charges", "Bank charges (Tk)", FK::Dec), ff("reference", "Reference", FK::Text)],
    lines_title: "", lines: &[], autofill: None, goto: "/accounting", done: "Transfer recorded", submit_label: "Transfer", min_lines: 0 };
#[component] pub fn NewBankTransfer() -> impl IntoView { view! { <NewDocPage cfg=&NEW_TRANSFER/> } }

/// Pay supplier bills. TDS and VDS are worked out from the supplier's rates and withheld automatically.
#[component]
pub fn NewSupplierPayment() -> impl IntoView {
    const F: &[FF] = &[ff("supplier_id", "Supplier", FK::Lookup("/suppliers?supplier_type=local", &["name"])).req(), ff("bank_account_id", "Pay from", FK::Lookup("/bank-accounts?currency=BDT", &["name"])).req(), ff("payment_date", "Date", FK::Date).def("today"), ff("method", "Method", FK::Select(&[("bank", "Bank transfer"), ("cheque", "Cheque"), ("cash", "Cash"), ("bkash", "bKash"), ("other", "Other")])).def("bank"), ff("reference", "Reference (cheque no.)", FK::Text)];
    let st = FormState::new(F, None); let bills: RwSignal<Vec<Value>> = RwSignal::new(vec![]); let amts: RwSignal<Vec<String>> = RwSignal::new(vec![]); let nav = use_navigate();
    Effect::new(move |_| { let s = st.get("supplier_id"); if s.is_empty() { bills.set(vec![]); return; }
        leptos::task::spawn_local(async move { if let Ok(v) = api::get(&format!("/supplier-bills?supplier_id={s}&unpaid=true&per_page=100")).await { let l = v["data"].as_array().cloned().unwrap_or_default(); amts.set(vec![String::new(); l.len()]); bills.set(l); } }); });
    let total = move || amts.get().iter().map(|a| a.parse::<f64>().unwrap_or(0.0)).sum::<f64>();
    let submit = move |_| { if st.busy.get_untracked() { return; } st.busy.set(true); st.err.set(None); let mut body = payload(F, &st);
        let a: Vec<Value> = bills.get_untracked().iter().zip(amts.get_untracked()).filter(|(_, x)| x.parse::<f64>().map(|v| v > 0.0).unwrap_or(false)).map(|(b, x)| json!({"bill_id": b["id"], "amount": x})).collect();
        if let Value::Object(m) = &mut body { m.insert("allocations".into(), json!(a)); } let nav = nav.clone();
        leptos::task::spawn_local(async move { let r = api::post("/supplier-payments", body).await; st.done(); match r { Ok(v) => { state::ok(if v["data"]["status"] == "pending_approval" { "Payment sent for approval" } else { "Payment made" }); state::refresh(); nav("/accounting", Default::default()); } Err(e) => st.fail(e) } }); };
    view! { <div class="page"><PageHead title="Pay supplier bills" subtitle="Choose the bills to settle. Tax and VAT the supplier's rates require are withheld automatically." crumbs=vec![(t("Accounting & Finance"), "/accounting".to_string())]/>
        <div class="grid" style="max-width:1100px"><Card><FormFields fields=F.to_vec() st=st/></Card>
        {move || (!bills.get().is_empty()).then(|| view! { <Card title="Open bills"><div class="tbl-wrap"><table><thead><tr><th>"Bill"</th><th>"Due"</th><th class="r">"Balance"</th><th class="r">"Pay now"</th></tr></thead><tbody>
            {bills.get().into_iter().enumerate().map(|(i, b)| { let bal = fmt::num(&b["balance"]); view! { <tr><td><b>{fmt::text(&b["bill_no"])}</b><div class="small dim">{fmt::text(&b["supplier_invoice_no"])}</div></td><td>{fmt::date(&b["due_date"])}{b["overdue"].as_bool().unwrap_or(false).then(|| view! { " " <Badge status="overdue"/> })}</td><td class="r num">{fmt::money(&b["balance"])}</td>
                <td class="r"><div class="row" style="justify-content:flex-end"><button class="btn ghost sm" on:click=move |_| amts.update(|a| a[i] = format!("{bal:.2}"))>"Full"</button><input class="input" style="width:140px;text-align:right" type="number" min="0" step="any" prop:value=move || amts.get().get(i).cloned().unwrap_or_default() on:input=move |e| amts.update(|a| if i < a.len() { a[i] = event_target_value(&e) })/></div></td></tr> } }).collect_view()}</tbody></table></div>
            <div class="row" style="margin-top:14px;justify-content:flex-end"><span class="mut">"Total to settle "</span><b class="num">{move || fmt::money_f(total())}</b></div></Card> })}
        <FormError st=st/><div class="row"><a class="btn" href="/accounting">{t("Cancel")}</a><button class="btn primary sp" disabled=move || st.busy.get() on:click=submit>"Make payment"</button></div></div></div> }
}
