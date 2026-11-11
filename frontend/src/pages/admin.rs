//! Administration: users, roles and permissions, branches, company profile, numbering and the audit log.
use super::common::*;
use crate::{api, crud::*, fmt, i18n::t, state, ui::*};
use leptos::prelude::*;
use serde_json::{json, Value};
use std::collections::BTreeSet;

const USER_FIELDS: &[FF] = &[ff("full_name", "Full name", FK::Text).req(), ff("email", "Email (sign-in)", FK::Email).req().help("Cannot be changed later"), ff("password", "Password", FK::Password).help("At least 10 characters with letters and numbers. Needed for new people only."), ff("role", "Role", FK::LookupCode("/roles", &["name"])).req(), ff("branch_id", "Branch", FK::Lookup("/branches", &["name"])).help("Limits the person to one branch; leave empty for all"), ff("phone", "Phone", FK::Text), ff("is_active", "Active", FK::Bool).def("true")];
static USERS: ListCfg = ListCfg { title: "Users", subtitle: "", endpoint: "/users", cols: &[col("full_name", "Name", CK::Bold), col("email", "Email", CK::Text), col("role_name", "Role", CK::Badge), col("branch_name", "Branch", CK::Text), col("last_login_at", "Last sign-in", CK::DateTime), col("is_active", "Active", CK::Bool)],
    search: true, filters: &[], fields: USER_FIELDS, write: "admin.write", new_label: "Add person", edit: true, delete: false, detail: "", new_route: "", extra_query: "", empty_hint: "", date_filter: false };
static BRANCHES: ListCfg = ListCfg { title: "Branches", subtitle: "", endpoint: "/branches", cols: &[col("code", "Code", CK::Mono), col("name", "Branch", CK::Bold), col("name_bn", "In Bangla", CK::Text), col("address", "Address", CK::Text), col("is_head_office", "Head office", CK::Bool), col("is_active", "Active", CK::Bool)],
    search: false, filters: &[], fields: &[ff("code", "Code", FK::Text).req(), ff("name", "Name", FK::Text).req(), ff("name_bn", "Name in Bangla", FK::Text), ff("address", "Address", FK::Area).full(), ff("phone", "Phone", FK::Text), ff("is_head_office", "Head office", FK::Bool), ff("is_active", "Active", FK::Bool).def("true")], write: "admin.write", new_label: "New branch", edit: true, delete: true, detail: "", new_route: "", extra_query: "", empty_hint: "", date_filter: false };
static AUDIT: ListCfg = ListCfg { title: "Audit Log", subtitle: "", endpoint: "/audit-logs", cols: &[col("created_at", "When", CK::DateTime), col("user_name", "Who", CK::Text), col("action", "Action", CK::Badge), col("entity", "What", CK::Text), col("entity_id", "Record", CK::Mono)],
    search: false, filters: &[Filter { param: "action", label: "All actions", options: &[("create", "Created"), ("update", "Changed"), ("post", "Posted"), ("submit", "Submitted"), ("approve", "Approved"), ("reject", "Rejected"), ("login", "Sign-ins"), ("login_failed", "Failed sign-ins"), ("delete", "Deleted")] }], fields: &[], write: "", new_label: "", edit: false, delete: false, detail: "", new_route: "", extra_query: "", empty_hint: "", date_filter: true };

#[component]
pub fn Admin() -> impl IntoView {
    let tab = RwSignal::new("users");
    view! { <div class="page"><PageHead title=t("Administration") subtitle="People, what they may do, company details and the record of everything that happens."/>
        <Tabs tabs=vec![("users", t("Users")), ("roles", t("Roles")), ("branches", t("Branches")), ("company", t("Company")), ("seq", "Numbering".to_string()), ("audit", t("Audit Log"))] active=tab/>
        {move || match tab.get() { "users" => view! { <ListPage cfg=&USERS headless=true/> }.into_any(), "roles" => view! { <Roles/> }.into_any(), "branches" => view! { <ListPage cfg=&BRANCHES headless=true/> }.into_any(), "company" => view! { <CompanyForm/> }.into_any(), "seq" => view! { <Numbering/> }.into_any(), _ => view! { <ListPage cfg=&AUDIT headless=true/> }.into_any() }}</div> }
}

#[component]
fn Roles() -> impl IntoView {
    let tick = state::app().tick;
    let roles = LocalResource::new(move || { let _ = tick.get(); async { api::get("/roles").await } });
    let meta = LocalResource::new(|| async { api::get("/meta").await });
    let edit: RwSignal<Option<Value>> = RwSignal::new(None);
    view! { <div class="card flush"><div class="toolbar">{move || state::can("admin.write").then(|| view! { <button class="btn primary sm" on:click=move |_| edit.set(Some(json!({"code": "", "name": "", "permissions": []})))><Icon name="plus" size=15/>"New role"</button> })}<span class="small mut">"Each person has one role. A role is a list of things they may do."</span></div>
        {move || match roles.get() { None => view! { <div style="padding:20px"><div class="skel"></div></div> }.into_any(), Some(Err(e)) => view! { <ErrorBox err=e/> }.into_any(), Some(Ok(v)) => { let l = v["data"].as_array().cloned().unwrap_or_default();
            view! { <div class="tbl-wrap"><table><thead><tr><th>"Role"</th><th>"About"</th><th class="r">"People"</th><th>"Type"</th></tr></thead><tbody>{l.into_iter().map(|r| { let r2 = r.clone(); view! { <tr class="click" on:click=move |_| edit.set(Some(r2.clone()))><td><b>{fmt::text(&r["name"])}</b><div class="small dim mono">{fmt::text(&r["code"])}</div></td><td class="mut">{fmt::text(&r["description"])}</td><td class="r num">{fmt::text(&r["users"])}</td><td><Badge status=if r["is_system"].as_bool().unwrap_or(false) { "info".to_string() } else { "teal".to_string() } label=if r["is_system"].as_bool().unwrap_or(false) { "Built-in" } else { "Custom" }/></td></tr> } }).collect_view()}</tbody></table></div> }.into_any() } }}
        {move || edit.get().map(|r| view! { <RoleDialog role=r meta=meta on_close=Callback::new(move |_| edit.set(None))/> })}</div> }
}
#[component]
fn RoleDialog(role: Value, meta: LocalResource<api::ApiResult>, on_close: Callback<()>) -> impl IntoView {
    let is_new = role["id"].is_null(); let id = fmt::text(&role["id"]); let owner = role["code"] == "owner";
    let sel: RwSignal<BTreeSet<String>> = RwSignal::new(role["permissions"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default());
    let name = RwSignal::new(fmt::text(&role["name"]).replace('—', "")); let code = RwSignal::new(fmt::text(&role["code"]).replace('—', "")); let err: RwSignal<Option<String>> = RwSignal::new(None);
    let save = move |_| { let body = json!({"code": code.get_untracked(), "name": name.get_untracked(), "permissions": sel.get_untracked().into_iter().collect::<Vec<_>>()}); let id = id.clone();
        leptos::task::spawn_local(async move { let r = if is_new { api::post("/roles", body).await } else { api::put(&format!("/roles/{id}"), body).await }; match r { Ok(_) => { state::ok("Role saved"); state::refresh(); on_close.run(()); } Err(e) => err.set(Some(e.friendly())) } }); };
    view! { <Modal title=if is_new { "New role" } else { "Role permissions" } wide=true on_close=on_close footer=footer(move || view! { <button class="btn" on:click=move |_| on_close.run(())>{t("Cancel")}</button>{(!owner && state::can("admin.write")).then(|| view! { <button class="btn primary" on:click=save.clone()>{t("Save")}</button> })} }.into_any())>
        {owner.then(|| view! { <div class="alert info" style="margin-bottom:14px"><Icon name="shield"/><div>"The owner role always has full access and cannot be changed."</div></div> })}
        <div class="form-grid" style="margin-bottom:18px"><div class="field"><label>"Name"</label><input class="input" prop:value=move || name.get() on:input=move |e| name.set(event_target_value(&e))/></div><div class="field"><label>"Code"</label><input class="input" disabled=!is_new prop:value=move || code.get() on:input=move |e| code.set(event_target_value(&e))/></div></div>
        {move || match meta.get() { Some(Ok(v)) => { let perms = v["data"]["permissions"].as_array().cloned().unwrap_or_default(); let mods: BTreeSet<String> = perms.iter().filter_map(|p| p["key"].as_str().and_then(|k| k.split('.').next()).map(str::to_string)).collect();
            view! { <div class="grid g2">{mods.into_iter().map(|m| { let mine: Vec<Value> = perms.iter().filter(|p| p["key"].as_str().map(|k| k.starts_with(&format!("{m}."))).unwrap_or(false)).cloned().collect(); view! { <div class="card" style="padding:14px"><h3>{fmt::nice(&m)}</h3>{mine.into_iter().map(|p| { let k = fmt::text(&p["key"]); let k2 = k.clone(); let k3 = k.clone();
                view! { <label class="check" style="height:auto;padding:6px 0;align-items:flex-start"><input type="checkbox" disabled=owner prop:checked=move || sel.get().contains(&k) || sel.get().contains("*") || sel.get().contains(&format!("{}.*", k3.split('.').next().unwrap_or(""))) on:change=move |e| { let on = event_target_checked(&e); sel.update(|s| { if on { s.insert(k2.clone()); } else { s.remove(&k2); } }); }/><span><b class="small">{fmt::text(&p["key"])}</b><div class="small mut">{fmt::text(&p["description"])}</div></span></label> } }).collect_view()}</div> } }).collect_view()}</div> }.into_any() }, _ => view! { <div class="skel"></div> }.into_any() }}
        {move || err.get().map(|e| view! { <div class="alert err" style="margin-top:14px">{e}</div> })}</Modal> }
}

#[component]
fn CompanyForm() -> impl IntoView {
    const F: &[FF] = &[ff("name", "Company name", FK::Text).req().full(), ff("name_bn", "Name in Bangla", FK::Text).full(), ff("bin", "BIN (VAT registration)", FK::Text), ff("tin", "TIN", FK::Text), ff("irc_no", "Import Registration Certificate (IRC)", FK::Text), ff("trade_license", "Trade licence", FK::Text), ff("phone", "Phone", FK::Text), ff("email", "Email", FK::Email), ff("address", "Address", FK::Area).full()];
    let res = LocalResource::new(|| async { api::get("/company").await });
    view! { {move || match res.get() { None => view! { <Loading/> }.into_any(), Some(Err(e)) => view! { <ErrorBox err=e/> }.into_any(), Some(Ok(v)) => { let d = v["data"].clone(); let st = FormState::new(F, Some(&d));
        let save = move |_| { st.busy.set(true); st.err.set(None); let b = payload(F, &st); leptos::task::spawn_local(async move { let r = api::put("/company", b).await; st.busy.set(false); match r { Ok(_) => { state::ok("Company details saved"); let _ = crate::app::load_session().await; } Err(e) => st.err.set(Some(e)) } }); };
        view! { <div class="card" style="max-width:900px"><div class="alert info" style="margin-bottom:16px"><Icon name="globe"/><div>"Currency: Bangladeshi Taka (BDT) · Time zone: Bangladesh Standard Time (UTC+6) · Fiscal year starts in July"</div></div><FormFields fields=F.to_vec() st=st/><FormError st=st/>{state::can("admin.write").then(|| view! { <div style="margin-top:18px"><button class="btn primary" disabled=move || st.busy.get() on:click=save>{t("Save")}</button></div> })}</div> }.into_any() } }}}
}

#[component]
fn Numbering() -> impl IntoView {
    let tick = state::app().tick;
    let res = LocalResource::new(move || { let _ = tick.get(); async { api::get("/sequences").await } });
    view! { <div class="card flush">{move || match res.get() { None => view! { <div style="padding:20px"><div class="skel"></div></div> }.into_any(), Some(Err(e)) => view! { <ErrorBox err=e/> }.into_any(), Some(Ok(v)) => { let l = v["data"].as_array().cloned().unwrap_or_default();
        view! { <div class="tbl-wrap"><table><thead><tr><th>"Document"</th><th>"Prefix"</th><th class="r">"Next number"</th><th>"Preview"</th></tr></thead><tbody>{l.into_iter().map(|s| { let key = fmt::text(&s["key"]); let pad = s["padding"].as_i64().unwrap_or(5) as usize; let next = s["next_no"].as_i64().unwrap_or(1); let prefix = fmt::text(&s["prefix"]); let k2 = key.clone();
            view! { <tr><td><b>{fmt::nice(&key)}</b></td><td><input class="input" style="width:140px;height:34px" value=prefix.clone() disabled=!state::can("admin.write") on:change=move |e| { let p = event_target_value(&e); let k = k2.clone(); leptos::task::spawn_local(async move { match api::put(&format!("/sequences/{k}"), json!({"prefix": p})).await { Ok(_) => { state::ok("Prefix saved"); state::refresh(); } Err(e) => state::fail(e.friendly()) } }); }/></td><td class="r num">{fmt::digits(next.to_string())}</td><td class="mono">{format!("{}-{:0w$}", prefix, next, w = pad)}</td></tr> } }).collect_view()}</tbody></table></div> }.into_any() } }}</div> }
}
