//! Policy & approvals: the approval inbox, approval rules and business policies.
use super::common::*;
use crate::{api, crud::*, fmt, i18n::t, state, ui::*};
use leptos::prelude::*;
use serde_json::{json, Value};
use std::collections::HashMap;

const ENTITIES: &[(&str, &str)] = &[("purchase_order", "Purchase order"), ("sales_order", "Sales order"), ("supplier_payment", "Supplier payment"), ("foreign_payment", "Foreign payment"), ("payroll_run", "Payroll"), ("stock_adjustment", "Stock adjustment")];
const RULE_FIELDS: &[FF] = &[ff("entity_type", "What needs approval", FK::Select(ENTITIES)).req(), ff("min_amount", "From amount (Tk)", FK::Dec).req().help("The rule applies to documents at or above this amount"), ff("level", "Approval step", FK::Int).def("1").help("Step 1 decides first, then step 2, and so on"), ff("approver_role", "Who approves", FK::LookupCode("/roles", &["name"])).req(), ff("description", "Explanation", FK::Text).full(), ff("is_active", "Active", FK::Bool).def("true")];
static RULES: ListCfg = ListCfg { title: "Approval Rules", subtitle: "", endpoint: "/approval-rules", cols: &[col("entity_type", "Document", CK::Badge), col("min_amount", "From amount", CK::Money), col("level", "Step", CK::Int), col("approver_role", "Approver", CK::Badge), col("description", "Explanation", CK::Text), col("is_active", "Active", CK::Bool)],
    search: false, filters: &[], fields: RULE_FIELDS, write: "admin.write", new_label: "New rule", edit: true, delete: true, detail: "", new_route: "", extra_query: "", empty_hint: "Without rules, documents are approved automatically.", date_filter: false };

#[component]
pub fn Policy() -> impl IntoView {
    let tab = RwSignal::new("inbox");
    view! { <div class="page"><PageHead title=t("Policy & Approvals") subtitle="Who must approve what, and the rules your business runs on."/>
        <Tabs tabs=vec![("inbox", "Waiting for me".to_string()), ("history", "History".to_string()), ("rules", t("Approval Rules")), ("policies", t("Business Policies"))] active=tab/>
        {move || match tab.get() { "inbox" => view! { <Inbox/> }.into_any(), "history" => view! { <History/> }.into_any(), "rules" => view! { <ListPage cfg=&RULES headless=true/> }.into_any(), _ => view! { <Policies/> }.into_any() }}</div> }
}

#[component]
fn Inbox() -> impl IntoView {
    let tick = state::app().tick;
    let res = LocalResource::new(move || { let _ = tick.get(); async { api::get("/approvals?mine=true&per_page=100").await } });
    let rejecting: RwSignal<Option<String>> = RwSignal::new(None); let comment = RwSignal::new(String::new());
    let decide = move |id: String, approve: bool, c: String| leptos::task::spawn_local(async move { let p = format!("/approvals/{id}/{}", if approve { "approve" } else { "reject" }); match api::post(&p, json!({"comment": c})).await { Ok(_) => { state::ok(if approve { "Approved" } else { "Rejected" }); state::refresh(); } Err(e) => state::fail(e.friendly()) } });
    view! { {move || match res.get() { None => view! { <Loading/> }.into_any(), Some(Err(e)) => view! { <ErrorBox err=e/> }.into_any(), Some(Ok(v)) => { let l = v["data"].as_array().cloned().unwrap_or_default();
        if l.is_empty() { return view! { <div class="card"><Empty title="Nothing is waiting for you" hint="Documents that need your decision appear here."/></div> }.into_any(); }
        view! { <div class="grid">{l.into_iter().map(|a| { let id = fmt::text(&a["id"]); let id2 = id.clone(); let et = fmt::text(&a["entity_type"]); let eid = fmt::text(&a["entity_id"]);
            let link = match et.as_str() { "purchase_order" => Some(format!("/purchase-orders/{eid}")), "sales_order" => Some(format!("/sales-orders/{eid}")), "payroll_run" => Some(format!("/payroll-runs/{eid}")), _ => None };
            view! { <div class="card"><div class="row wrap"><Badge status="warn" label=fmt::nice(&et)/><b style="font-size:16px">{fmt::text(&a["entity_no"])}</b><span class="mut">{format!("requested by {}", fmt::text(&a["requested_by_name"]))}</span><b class="sp num" style="font-size:18px">{fmt::money(&a["amount"])}</b></div>
                <div class="row" style="margin-top:14px">{link.map(|l| view! { <a class="btn sm" href=l>"Review details"</a> })}<span class="small dim">{format!("Step {} · {}", a["level"], fmt::datetime(&a["created_at"]))}</span>
                    <span class="sp row"><button class="btn danger" on:click=move |_| { comment.set(String::new()); rejecting.set(Some(id.clone())); }>{t("Reject")}</button><button class="btn primary" on:click=move |_| decide(id2.clone(), true, String::new())><Icon name="check" size=16/>{t("Approve")}</button></span></div></div> } }).collect_view()}</div> }.into_any() } }}
        {move || rejecting.get().map(|id| view! { <Modal title="Reject this request" on_close=Callback::new(move |_| rejecting.set(None)) footer=footer(move || { let id = id.clone(); view! { <button class="btn" on:click=move |_| rejecting.set(None)>{t("Cancel")}</button><button class="btn danger" disabled=move || comment.get().trim().is_empty() on:click=move |_| { decide(id.clone(), false, comment.get_untracked()); rejecting.set(None); }>{t("Reject")}</button> }.into_any() })>
            <div class="field"><label>"Why? (the requester will see this)"<span class="req">"*"</span></label><textarea class="textarea" prop:value=move || comment.get() on:input=move |e| comment.set(event_target_value(&e))></textarea></div></Modal> })} }
}

static HISTORY: ListCfg = ListCfg { title: "History", subtitle: "", endpoint: "/approvals", cols: &[col("entity_no", "Document", CK::Bold), col("entity_type", "Type", CK::Badge), col("amount", "Amount", CK::Money), col("level", "Step", CK::Int), col("approver_role", "Approver role", CK::Badge), col("status", "Decision", CK::Badge), col("decided_by_name", "Decided by", CK::Text), col("decided_at", "When", CK::DateTime), col("comment", "Comment", CK::Text)],
    search: false, filters: &[Filter { param: "status", label: "All decisions", options: &[("approved", "Approved"), ("rejected", "Rejected"), ("pending", "Pending"), ("cancelled", "Cancelled")] }, Filter { param: "entity_type", label: "All documents", options: ENTITIES }], fields: &[], write: "", new_label: "", edit: false, delete: false, detail: "", new_route: "", extra_query: "", empty_hint: "", date_filter: false };
#[component] fn History() -> impl IntoView { view! { <ListPage cfg=&HISTORY headless=true/> } }

#[component]
fn Policies() -> impl IntoView {
    let tick = state::app().tick;
    let res = LocalResource::new(move || { let _ = tick.get(); async { api::get("/settings").await } });
    let edits: RwSignal<HashMap<String, Value>> = RwSignal::new(HashMap::new());
    let save = move |_| { let m = edits.get_untracked(); if m.is_empty() { return; }
        leptos::task::spawn_local(async move { match api::put("/settings", json!(m)).await { Ok(_) => { state::ok("Policies saved"); edits.set(HashMap::new()); state::refresh(); } Err(e) => state::fail(e.friendly()) } }); };
    view! { <div class="card">{move || match res.get() { None => view! { <div class="skel"></div> }.into_any(), Some(Err(e)) => view! { <ErrorBox err=e/> }.into_any(), Some(Ok(v)) => { let l = v["data"].as_array().cloned().unwrap_or_default(); let can = state::can("admin.write");
        view! { {l.into_iter().map(|p| { let key = fmt::text(&p["key"]); let kind = fmt::text(&p["kind"]); let k2 = key.clone(); let k3 = key.clone(); let cur = p["value"].clone();
            view! { <div class="row" style="padding:14px 0;border-bottom:1px solid var(--line);gap:20px"><div style="flex:1"><b>{fmt::text(&p["description"])}</b><div class="small dim mono">{key.clone()}</div></div>
                {if kind == "bool" { let on = cur.as_bool().unwrap_or(false); view! { <label class="check" style="height:auto"><input type="checkbox" disabled=!can checked=on on:change=move |e| edits.update(|m| { m.insert(k2.clone(), json!(event_target_checked(&e))); })/><span>"On"</span></label> }.into_any() } else { view! { <input class="input" style="width:150px;text-align:right" type="number" min="0" step="any" disabled=!can prop:value=fmt::text(&cur) on:input=move |e| { if let Ok(n) = event_target_value(&e).parse::<f64>() { edits.update(|m| { m.insert(k3.clone(), json!(n)); }); } }/> }.into_any() }}</div> } }).collect_view()}
            {can.then(|| view! { <div class="row" style="margin-top:16px"><button class="btn primary sp" disabled=move || edits.get().is_empty() on:click=save>"Save changes"</button></div> })} }.into_any() } }}</div> }
}
