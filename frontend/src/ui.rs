//! Shared interface components: icons, cards, badges, tables, modals and the form engine.
use crate::{api, fmt, i18n::t, state};
use leptos::prelude::*;
use serde_json::{json, Map, Value};

// ---------------- icons ----------------
pub fn icon_paths(name: &str) -> &'static str {
    match name {
        "home" => r#"<path d="M3 11 12 3l9 8v9a1 1 0 0 1-1 1h-5v-6H9v6H4a1 1 0 0 1-1-1z"/>"#,
        "ship" => r#"<path d="M3 17c2 1.5 4 1.5 6 0s4-1.5 6 0 4 1.5 6 0"/><path d="M5 14 4 9h16l-1 5"/><path d="M12 9V4h4M8 9V6"/>"#,
        "calc" => r#"<rect x="5" y="3" width="14" height="18" rx="2"/><path d="M8 7h8M8 12h2M12 12h2M16 12h0M8 16h2M12 16h2M16 16h0"/>"#,
        "stamp" => r#"<path d="M5 21h14M7 17h10l-1-5a4 4 0 1 0-8 0z"/><path d="M12 8V5"/>"#,
        "chart" => r#"<path d="M3 20h18"/><rect x="5" y="11" width="3" height="9"/><rect x="10.5" y="6" width="3" height="14"/><rect x="16" y="13" width="3" height="7"/>"#,
        "cart" => r#"<circle cx="9" cy="20" r="1.5"/><circle cx="18" cy="20" r="1.5"/><path d="M3 4h2.5l2.2 11h10.3l2-8H6.5"/>"#,
        "board" => r#"<rect x="3" y="4" width="5" height="16" rx="1.5"/><rect x="10" y="4" width="5" height="10" rx="1.5"/><rect x="17" y="4" width="4" height="13" rx="1.5"/>"#,
        "box" => r#"<path d="M21 8 12 3 3 8v8l9 5 9-5z"/><path d="m3 8 9 5 9-5M12 13v8"/>"#,
        "users" => r#"<circle cx="9" cy="8" r="3.5"/><path d="M2.5 20c.5-3.5 3.2-5.5 6.5-5.5s6 2 6.5 5.5"/><path d="M16 4.5a3.5 3.5 0 0 1 0 7M18 14.8c2 .6 3.3 2.4 3.5 5.2"/>"#,
        "book" => r#"<path d="M6 3h12v18H6z"/><path d="M9 8h6M9 12h6M9 16h4"/>"#,
        "wallet" => r#"<path d="M3 7a2 2 0 0 1 2-2h13v4"/><path d="M3 7v11a2 2 0 0 0 2 2h15V9H5a2 2 0 0 1-2-2z"/><circle cx="16.5" cy="14.5" r="1.2"/>"#,
        "swap" => r#"<path d="M4 8h14l-3-3M20 16H6l3 3"/>"#,
        "id" => r#"<rect x="3" y="5" width="18" height="14" rx="2"/><circle cx="9" cy="11" r="2.3"/><path d="M5.5 17c.5-2 1.8-3 3.5-3s3 1 3.5 3M15 10h4M15 14h3"/>"#,
        "shield" => r#"<path d="M12 3 4 6v6c0 5 3.5 8 8 9 4.5-1 8-4 8-9V6z"/><path d="m9 12 2 2 4-4"/>"#,
        "report" => r#"<path d="M6 3h9l4 4v14H6z"/><path d="M14 3v5h5M9 13h6M9 17h6"/>"#,
        "gear" => r#"<circle cx="12" cy="12" r="3"/><path d="M12 2v3M12 19v3M4.2 4.2l2.1 2.1M17.7 17.7l2.1 2.1M2 12h3M19 12h3M4.2 19.8l2.1-2.1M17.7 6.3l2.1-2.1"/>"#,
        "search" => r#"<circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/>"#,
        "bell" => r#"<path d="M6 9a6 6 0 0 1 12 0c0 6 2 7 2 7H4s2-1 2-7"/><path d="M10 20a2 2 0 0 0 4 0"/>"#,
        "menu" => r#"<path d="M4 6h16M4 12h16M4 18h16"/>"#,
        "plus" => r#"<path d="M12 5v14M5 12h14"/>"#,
        "x" => r#"<path d="M6 6l12 12M18 6 6 18"/>"#,
        "moon" => r#"<path d="M20 14.5A8 8 0 0 1 9.5 4 8 8 0 1 0 20 14.5z"/>"#,
        "sun" => r#"<circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"/>"#,
        "print" => r#"<path d="M6 9V3h12v6M6 18H4v-7h16v7h-2"/><rect x="6" y="14" width="12" height="7"/>"#,
        "out" => r#"<path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4M16 17l5-5-5-5M21 12H9"/>"#,
        "check" => r#"<path d="m5 12 5 5 9-10"/>"#,
        "alert" => r#"<path d="M12 3 2 20h20z"/><path d="M12 10v4M12 17h0"/>"#,
        "globe" => r#"<circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3c3 3 3 15 0 18M12 3c-3 3-3 15 0 18"/>"#,
        "truck" => r#"<path d="M2 6h11v10H2zM13 9h5l3 3v4h-8"/><circle cx="7" cy="18" r="2"/><circle cx="17" cy="18" r="2"/>"#,
        "tag" => r#"<path d="M3 12V4h8l10 10-8 8z"/><circle cx="8" cy="8" r="1.4"/>"#,
        "refresh" => r#"<path d="M20 11A8 8 0 0 0 5.5 7M4 4v4h4M4 13a8 8 0 0 0 14.5 4M20 20v-4h-4"/>"#,
        "up" => r#"<path d="M6 15l6-6 6 6"/>"#,
        _ => r#"<circle cx="12" cy="12" r="8"/>"#,
    }
}
#[component]
pub fn Icon(name: &'static str, #[prop(optional)] size: u32) -> impl IntoView {
    let s = if size == 0 { 18 } else { size };
    view! { <svg viewBox="0 0 24 24" width=s height=s inner_html=icon_paths(name)></svg> }
}

/// A close/cancel callback that stays valid even if the dialog that asked for it is torn down first
/// (an async save may finish after its dialog has already been removed from the page).
pub fn closer(f: impl Fn() + Send + Sync + 'static) -> Callback<()> {
    let root = Owner::new_root(None);
    let cb = root.with(|| Callback::new(move |_| f()));
    std::mem::forget(root);
    cb
}

// ---------------- layout ----------------
#[component]
pub fn PageHead(#[prop(into)] title: String, #[prop(optional, into)] subtitle: String, #[prop(optional)] crumbs: Option<Vec<(String, String)>>, #[prop(optional)] children: Option<Children>) -> impl IntoView {
    view! {
        <div>
            {crumbs.map(|c| view! { <div class="crumbs">{c.into_iter().enumerate().map(|(i, (label, href))| view! { {if i > 0 { " / " } else { "" }} <a href=href>{label}</a> }).collect_view()}</div> })}
            <div class="page-head">
                <div><h1>{title.clone()}</h1>{(!subtitle.is_empty()).then(|| view! { <p>{subtitle.clone()}</p> })}</div>
                <div class="actions no-print">{children.map(|c| c())}</div>
            </div>
        </div>
    }
}
#[component]
pub fn Card(#[prop(optional, into)] title: String, #[prop(optional, into)] class: String, #[prop(optional)] right: Option<ViewFn>, children: Children) -> impl IntoView {
    view! { <div class=format!("card {class}")>{(!title.is_empty()).then(|| view! { <h3>{title.clone()}<span class="r">{right.clone().map(|r| r.run())}</span></h3> })}{children()}</div> }
}

pub fn badge_class(status: &str) -> &'static str {
    match status {
        "paid" | "posted" | "approved" | "received" | "delivered" | "cleared" | "released" | "settled" | "active" | "confirmed" | "finalized" | "verified" | "present" | "success" | "ok" | "ordered" | "closed" | "reconciled" | "submitted_ok" => "ok",
        "pending_approval" | "partial" | "partially_received" | "partially_delivered" | "arrived" | "customs" | "open" | "waiting" | "requested" | "pending" | "late" | "warning" | "half_day" | "on_leave" | "draft" | "assessed" | "order_now" | "low" => "warn",
        "rejected" | "cancelled" | "overdue" | "expired" | "failed" | "absent" | "critical" | "reversed" | "terminated" | "resigned" | "closed_period" => "bad",
        "in_transit" | "opened" | "booked" | "submitted" | "approved_level" | "info" | "at_sea" | "discharged" | "leave" | "sight" | "usance" => "info",
        "no_demand" | "inactive" => "",
        _ => "teal",
    }
}
#[component]
pub fn Badge(#[prop(into)] status: String, #[prop(optional, into)] label: String) -> impl IntoView {
    let cls = badge_class(&status);
    let text = if label.is_empty() { fmt::nice(&status) } else { label };
    view! { <span class=format!("badge {cls}")>{text}</span> }
}
#[component]
pub fn Spinner() -> impl IntoView { view! { <span class="spin"></span> } }
#[component]
pub fn Loading() -> impl IntoView {
    view! { <div class="card"><div class="col"><div class="skel" style="width:30%"></div><div class="skel" style="width:90%"></div><div class="skel" style="width:75%"></div><div class="skel" style="width:85%"></div><div class="skel" style="width:60%"></div></div></div> }
}
#[component]
pub fn Empty(#[prop(into)] title: String, #[prop(optional, into)] hint: String) -> impl IntoView {
    view! { <div class="empty"><div class="big">"◌"</div><b>{title}</b><span>{hint}</span></div> }
}
#[component]
pub fn ErrorBox(err: api::ApiError) -> impl IntoView {
    view! { <div class="alert err"><Icon name="alert"/><div><b>"We could not load this"</b><div>{err.friendly()}</div></div></div> }
}
#[component]
pub fn Kpi(#[prop(into)] label: String, #[prop(into)] value: String, #[prop(optional, into)] sub: String, #[prop(optional, into)] icon: String, #[prop(optional, into)] tone: String, #[prop(optional, into)] href: String, #[prop(optional, into)] tip: String) -> impl IntoView {
    let ic: &'static str = Box::leak(icon.into_boxed_str());
    let inner = view! {
        <div class=format!("card kpi {tone}")>
            {(!ic.is_empty()).then(|| view! { <div class="ic"><Icon name=ic/></div> })}
            <div class="lbl">{label}{(!tip.is_empty()).then(|| view! { <span class="tip" title=tip.clone()>"?"</span> })}</div>
            <div class="val num">{value}</div><div class="sub">{sub}</div>
        </div>
    };
    if href.is_empty() { inner.into_any() } else { view! { <a href=href style="color:inherit;text-decoration:none">{inner}</a> }.into_any() }
}
#[component]
pub fn Tabs(tabs: Vec<(&'static str, String)>, active: RwSignal<&'static str>) -> impl IntoView {
    view! { <div class="tabs">{tabs.into_iter().map(|(k, l)| view! { <button class:on=move || active.get() == k on:click=move |_| active.set(k)>{l}</button> }).collect_view()}</div> }
}
/// Buttons shown at the bottom of a dialog.
#[derive(Clone)]
pub struct Footer(pub std::rc::Rc<dyn Fn() -> AnyView>);
pub fn footer(f: impl Fn() -> AnyView + 'static) -> Footer { Footer(std::rc::Rc::new(f)) }

#[component]
pub fn Modal(#[prop(into)] title: String, on_close: Callback<()>, #[prop(optional)] wide: bool, children: Children, #[prop(optional)] footer: Option<Footer>) -> impl IntoView {
    view! {
        <div class="overlay" on:mousedown=move |_| on_close.run(())>
            <div class=if wide { "modal wide" } else { "modal" } on:mousedown=|e| e.stop_propagation()>
                <header><h2>{title}</h2><button class="icon-btn sp" style="width:34px;height:34px" on:click=move |_| on_close.run(()) aria-label="Close"><Icon name="x"/></button></header>
                <div class="body">{children()}</div>
                {footer.map(|f| view! { <footer>{f.0()}</footer> })}
            </div>
        </div>
    }
}

// ---------------- tables ----------------
#[derive(Clone, Copy, PartialEq)]
pub enum CK { Text, Money, Qty, Date, DateTime, Badge, Pct, Bool, Mono, Fc, Int, Bold }
#[derive(Clone, Copy)]
pub struct Col { pub key: &'static str, pub label: &'static str, pub kind: CK }
pub const fn col(key: &'static str, label: &'static str, kind: CK) -> Col { Col { key, label, kind } }

pub fn dig<'a>(v: &'a Value, path: &str) -> &'a Value {
    let mut cur = v;
    for p in path.split('.') { cur = &cur[p]; }
    cur
}
pub fn cell(row: &Value, c: &Col) -> AnyView {
    let v = dig(row, c.key);
    match c.kind {
        CK::Money => view! { <span class="num">{fmt::money(v)}</span> }.into_any(),
        CK::Qty => view! { <span class="num">{fmt::qty(v)}</span> }.into_any(),
        CK::Int => view! { <span class="num">{fmt::digits(fmt::text(v))}</span> }.into_any(),
        CK::Pct => view! { <span class="num">{fmt::pct(v)}</span> }.into_any(),
        CK::Date => view! { <span>{fmt::date(v)}</span> }.into_any(),
        CK::DateTime => view! { <span>{fmt::datetime(v)}</span> }.into_any(),
        CK::Mono => view! { <span class="mono">{fmt::text(v)}</span> }.into_any(),
        CK::Bold => view! { <b>{fmt::text(v)}</b> }.into_any(),
        CK::Bool => view! { <span>{if v.as_bool().unwrap_or(false) { "✓" } else { "—" }}</span> }.into_any(),
        CK::Fc => { let cur = row["currency"].as_str().unwrap_or("").to_string(); view! { <span class="num">{fmt::fc(v, &cur)}</span> }.into_any() }
        CK::Badge => { let s = v.as_str().unwrap_or("").to_string(); if s.is_empty() { view! { <span>"—"</span> }.into_any() } else { view! { <Badge status=s/> }.into_any() } }
        CK::Text => view! { <span>{fmt::text(v)}</span> }.into_any(),
    }
}
pub fn is_right(k: CK) -> bool { matches!(k, CK::Money | CK::Qty | CK::Int | CK::Pct | CK::Fc) }

/// A plain table over JSON rows. `on_row` makes rows clickable.
pub fn table(cols: &[Col], rows: Vec<Value>, on_row: Option<Callback<Value>>) -> AnyView {
    if rows.is_empty() { return view! { <Empty title=t("Nothing here yet")/> }.into_any(); }
    let cols: Vec<Col> = cols.to_vec();
    view! {
        <div class="tbl-wrap"><table>
            <thead><tr>{cols.iter().map(|c| view! { <th class=if is_right(c.kind) { "r" } else { "" }>{t(c.label)}</th> }).collect_view()}</tr></thead>
            <tbody>{rows.into_iter().map(|r| {
                let rr = r.clone();
                let cols = cols.clone();
                view! { <tr class=if on_row.is_some() { "click" } else { "" } on:click=move |_| if let Some(cb) = on_row { cb.run(rr.clone()) }>
                    {cols.iter().map(|c| view! { <td class=if is_right(c.kind) { "r" } else { "" }>{cell(&r, c)}</td> }).collect_view()}
                </tr> }
            }).collect_view()}</tbody>
        </table></div>
    }.into_any()
}

// ---------------- forms ----------------
#[derive(Clone, Copy, PartialEq)]
pub enum FK {
    Text, Area, Email, Int, Dec, Date, Bool, Password,
    Select(&'static [(&'static str, &'static str)]),
    /// Options loaded from an endpoint; `.1` lists the keys that make up each label.
    Lookup(&'static str, &'static [&'static str]),
    /// Like `Lookup`, but the stored value is the record's `code` (used for roles).
    LookupCode(&'static str, &'static [&'static str]),
}
#[derive(Clone, Copy)]
pub struct FF { pub name: &'static str, pub label: &'static str, pub kind: FK, pub req: bool, pub help: &'static str, pub def: &'static str, pub full: bool }
pub const fn ff(name: &'static str, label: &'static str, kind: FK) -> FF { FF { name, label, kind, req: false, help: "", def: "", full: false } }
impl FF {
    pub const fn req(mut self) -> Self { self.req = true; self }
    pub const fn help(mut self, h: &'static str) -> Self { self.help = h; self }
    pub const fn def(mut self, d: &'static str) -> Self { self.def = d; self }
    pub const fn full(mut self) -> Self { self.full = true; self }
}

#[derive(Clone, Copy)]
pub struct FormState { pub vals: RwSignal<Map<String, Value>>, pub err: RwSignal<Option<api::ApiError>>, pub busy: RwSignal<bool> }
impl FormState {
    pub fn new(fields: &[FF], initial: Option<&Value>) -> Self {
        let mut m = Map::new();
        for f in fields {
            let from = initial.and_then(|i| i.get(f.name)).filter(|v| !v.is_null());
            let s = match from { Some(Value::String(s)) => s.clone(), Some(Value::Bool(b)) => b.to_string(), Some(Value::Number(n)) => n.to_string(), _ => if f.kind == FK::Date && f.def == "today" { fmt::today() } else { f.def.to_string() } };
            if f.kind == FK::Date && s.len() > 10 { m.insert(f.name.into(), json!(&s[..10])); } else { m.insert(f.name.into(), json!(s)); }
        }
        FormState { vals: RwSignal::new(m), err: RwSignal::new(None), busy: RwSignal::new(false) }
    }
    /// Safe even when the dialog that owns this form has already closed.
    pub fn done(&self) { let _ = self.busy.try_set(false); }
    pub fn fail(&self, e: api::ApiError) { let _ = self.err.try_set(Some(e)); }
    pub fn get(&self, name: &str) -> String { self.vals.with(|m| m.get(name).and_then(|v| v.as_str()).unwrap_or("").to_string()) }
    pub fn set(&self, name: &str, v: String) { self.vals.update(|m| { m.insert(name.into(), json!(v)); }); }
}

/// Turn what the user typed into the JSON the API expects (empty fields are left out).
pub fn payload(fields: &[FF], st: &FormState) -> Value {
    let mut out = Map::new();
    st.vals.with(|m| {
        for f in fields {
            let raw = m.get(f.name).and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
            if raw.is_empty() { continue; }
            match f.kind {
                FK::Int => { if let Ok(n) = raw.parse::<i64>() { out.insert(f.name.into(), json!(n)); } }
                FK::Dec => { out.insert(f.name.into(), json!(raw)); }
                FK::Bool => { out.insert(f.name.into(), json!(raw == "true")); }
                _ => { out.insert(f.name.into(), json!(raw)); }
            }
        }
    });
    Value::Object(out)
}
/// Checkbox fields always have a value, even when unticked.
pub fn payload_with_bools(fields: &[FF], st: &FormState) -> Value {
    let mut p = payload(fields, st);
    for f in fields { if f.kind == FK::Bool { if let Value::Object(m) = &mut p { m.insert(f.name.into(), json!(st.get(f.name) == "true")); } } }
    p
}

pub async fn options(endpoint: &str) -> Vec<Value> {
    let sep = if endpoint.contains('?') { '&' } else { '?' };
    api::get(&format!("{endpoint}{sep}per_page=500")).await.ok().and_then(|v| v["data"].as_array().cloned()).unwrap_or_default()
}
pub fn option_label(v: &Value, keys: &[&str]) -> String {
    let bn = state::is_bn();
    keys.iter().filter_map(|k| { let key: &str = if bn && *k == "name" && v["name_bn"].as_str().map(|s| !s.is_empty()).unwrap_or(false) { "name_bn" } else { *k }; v.get(key).and_then(|x| x.as_str().map(str::to_string).or_else(|| if x.is_number() { Some(x.to_string()) } else { None })) }).collect::<Vec<_>>().join(" · ")
}

#[component]
pub fn Input(f: FF, st: FormState, #[prop(optional)] compact: bool) -> impl IntoView {
    let name = f.name;
    let invalid = move || st.err.get().and_then(|e| e.field(name));
    let cls = move || format!("{} {}", match f.kind { FK::Area => "textarea", FK::Select(_) | FK::Lookup(..) | FK::LookupCode(..) => "select", _ => "input" }, if invalid().is_some() { "err" } else { "" });
    let control = match f.kind {
        FK::Area => view! { <textarea class=cls prop:value=move || st.get(name) on:input=move |e| st.set(name, event_target_value(&e))></textarea> }.into_any(),
        FK::Bool => view! { <label class="check"><input type="checkbox" prop:checked=move || st.get(name) == "true" on:change=move |e| st.set(name, event_target_checked(&e).to_string())/><span>{f.help}</span></label> }.into_any(),
        FK::Select(opts) => view! { <select class=cls prop:value=move || st.get(name) on:change=move |e| st.set(name, event_target_value(&e))>
            {(!f.req).then(|| view! { <option value="">"—"</option> })}
            {opts.iter().map(|(v, l)| view! { <option value=*v selected=move || st.get(name) == *v>{t(l)}</option> }).collect_view()}
        </select> }.into_any(),
        FK::Lookup(endpoint, keys) | FK::LookupCode(endpoint, keys) => {
            let by_code = matches!(f.kind, FK::LookupCode(..));
            let res = LocalResource::new(move || options(endpoint));
            view! { <select class=cls prop:value=move || st.get(name) on:change=move |e| st.set(name, event_target_value(&e))>
                <option value="">{if f.req { "Choose…" } else { "—" }}</option>
                {move || res.get().map(|opts| opts.into_iter().map(|o| { let id = o[if by_code { "code" } else { "id" }].as_str().unwrap_or("").to_string(); let id2 = id.clone(); let label = option_label(&o, keys); view! { <option value=id selected=move || st.get(name) == id2>{label}</option> } }).collect_view())}
            </select> }.into_any()
        }
        other => {
            let ty = match other { FK::Int | FK::Dec => "number", FK::Date => "date", FK::Email => "email", FK::Password => "password", _ => "text" };
            let step = if other == FK::Dec { "any" } else { "1" };
            view! { <input class=cls type=ty step=step min=if matches!(other, FK::Int | FK::Dec) { "0" } else { "" } prop:value=move || st.get(name) on:input=move |e| st.set(name, event_target_value(&e)) autocomplete="off"/> }.into_any()
        }
    };
    view! {
        <div class=if f.full { "field full" } else { "field" }>
            {(f.kind != FK::Bool && !compact).then(|| view! { <label>{t(f.label)}{f.req.then(|| view! { <span class="req">"*"</span> })}{(!f.help.is_empty()).then(|| view! { <span class="tip" title=f.help>"?"</span> })}</label> })}
            {(f.kind == FK::Bool).then(|| view! { <label>{t(f.label)}</label> })}
            {control}
            {move || invalid().map(|m| view! { <span class="err-msg">{m}</span> })}
        </div>
    }
}
#[component]
pub fn FormFields(fields: Vec<FF>, st: FormState) -> impl IntoView {
    view! { <div class="form-grid">{fields.into_iter().map(|f| view! { <Input f=f st=st/> }).collect_view()}</div> }
}
#[component]
pub fn FormError(st: FormState) -> impl IntoView {
    view! { {move || st.err.get().filter(|e| e.fields.is_empty() || e.code != "validation_failed").map(|e| view! { <div class="alert err" style="margin-top:14px"><Icon name="alert"/><div>{e.friendly()}</div></div> })}
            {move || st.err.get().filter(|e| e.code == "validation_failed" && !e.fields.is_empty()).map(|_| view! { <div class="alert err" style="margin-top:14px"><Icon name="alert"/><div>"Please fix the highlighted fields."</div></div> })} }
}

/// Editable table of line items (products, quantities, prices).
#[component]
pub fn LinesEditor(cols: Vec<FF>, rows: RwSignal<Vec<Map<String, Value>>>, #[prop(optional)] err: Option<RwSignal<Option<api::ApiError>>>,
                   /// When a product is picked, copy this key of the product (e.g. sale_price) into this column.
                   #[prop(optional)] autofill: Option<(&'static str, &'static str)>) -> impl IntoView {
    let opts: Vec<(&'static str, LocalResource<Vec<Value>>)> = cols.iter().filter_map(|c| if let FK::Lookup(e, _) = c.kind { Some((c.name, LocalResource::new(move || options(e)))) } else { None }).collect();
    let opts = StoredValue::new_local(opts);
    let cols2 = cols.clone();
    let add = move |_| { let mut m = Map::new(); for c in &cols2 { m.insert(c.name.into(), json!(c.def)); } rows.update(|r| r.push(m)); };
    let cols3 = cols.clone();
    view! {
        <div class="tbl-wrap" style="border:1px solid var(--line);border-radius:12px">
            <table><thead><tr>{cols.iter().map(|c| view! { <th>{t(c.label)}{c.req.then(|| "*")}</th> }).collect_view()}<th></th></tr></thead>
            <tbody>{move || rows.get().into_iter().enumerate().map(|(i, row)| {
                let cols4 = cols3.clone();
                view! { <tr>{cols4.into_iter().map(|c| {
                    let name = c.name; let val = row.get(name).and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let set = move |v: String| rows.update(|r| { if let Some(m) = r.get_mut(i) { m.insert(name.into(), json!(v)); } });
                    let ctl = match c.kind {
                        FK::Lookup(_, keys) => {
                            let res = opts.with_value(|o| o.iter().find(|x| x.0 == name).map(|x| x.1));
                            let val2 = val.clone();
                            view! { <select class="select" style="min-width:220px" prop:value=val.clone() on:change=move |e| {
                                    let id = event_target_value(&e); set(id.clone());
                                    if let (Some((src, target)), Some(res)) = (autofill, res) { if let Some(list) = res.get() { if let Some(p) = list.iter().find(|p| p["id"].as_str() == Some(&id)) {
                                        let cur = rows.with(|r| r.get(i).and_then(|m| m.get(target)).and_then(|v| v.as_str()).unwrap_or("").to_string());
                                        if cur.is_empty() { let v = p[src].as_f64().map(|x| x.to_string()).unwrap_or_default(); rows.update(|r| { if let Some(m) = r.get_mut(i) { m.insert(target.into(), json!(v)); } }); } } } }
                                }>
                                <option value="">"Choose…"</option>
                                {move || res.and_then(|r| r.get()).map(|o| o.into_iter().map(|p| { let id = p["id"].as_str().unwrap_or("").to_string(); let sel = id == val2; let label = option_label(&p, keys); view! { <option value=id selected=sel>{label}</option> } }).collect_view())}
                            </select> }.into_any()
                        }
                        FK::Select(os) => view! { <select class="select" prop:value=val.clone() on:change=move |e| set(event_target_value(&e))>{os.iter().map(|(v, l)| view! { <option value=*v selected=*v == val>{*l}</option> }).collect_view()}</select> }.into_any(),
                        FK::Text => view! { <input class="input" style="min-width:140px" prop:value=val.clone() on:input=move |e| set(event_target_value(&e))/> }.into_any(),
                        _ => view! { <input class="input" style="min-width:90px" type="number" step="any" min=if name == "qty_change" { "" } else { "0" } prop:value=val.clone() on:input=move |e| set(event_target_value(&e))/> }.into_any(),
                    };
                    view! { <td>{ctl}</td> }
                }).collect_view()}
                <td><button class="btn ghost sm" on:click=move |_| rows.update(|r| { r.remove(i); }) aria-label="Remove line"><Icon name="x" size=15/></button></td></tr> }
            }).collect_view()}</tbody></table>
        </div>
        <div class="row" style="margin-top:10px"><button class="btn sm" on:click=add><Icon name="plus" size=15/>{t("Add line")}</button>
            {move || err.and_then(|e| e.get()).and_then(|e| e.fields.iter().find(|f| f.0.starts_with("lines")).map(|f| f.1.clone())).map(|m| view! { <span class="err-msg">{m}</span> })}</div>
    }
}
/// Convert editor rows to API line objects. Whole numbers become numbers, decimals stay exact strings.
pub fn lines_payload(cols: &[FF], rows: &[Map<String, Value>]) -> Vec<Value> {
    // Untouched rows are ignored, so a spare blank line never blocks saving.
    rows.iter().map(|r| { let mut o = Map::new(); for c in cols { let raw = r.get(c.name).and_then(|v| v.as_str()).unwrap_or("").trim(); if raw.is_empty() { continue; } o.insert(c.name.into(), json!(raw)); } Value::Object(o) }).filter(|o| o.as_object().map(|m| !m.is_empty()).unwrap_or(false)).collect()
}

/// Ask before doing something that cannot be undone.
#[component]
pub fn Confirm(#[prop(into)] title: String, #[prop(into)] text: String, #[prop(into)] action: String, on_close: Callback<()>, on_yes: Callback<()>, #[prop(optional)] danger: bool) -> impl IntoView {
    let action2 = action.clone();
    view! { <Modal title=title on_close=on_close footer=footer(move || { let a = action2.clone(); view! {
        <button class="btn" on:click=move |_| on_close.run(())>{t("Cancel")}</button>
        <button class=if danger { "btn danger" } else { "btn primary" } on:click=move |_| { on_yes.run(()); on_close.run(()) }>{a}</button> } .into_any() })>
        <p class="mut" style="padding:6px 0 14px">{text}</p></Modal> }
}

// ---------------- charts ----------------
#[component]
pub fn BarChart(data: Vec<(String, f64)>, #[prop(optional, into)] color: String, #[prop(optional)] height: u32) -> impl IntoView {
    let h = if height == 0 { 180.0 } else { height as f64 };
    let w = 640.0; let n = data.len().max(1) as f64; let max = data.iter().map(|d| d.1).fold(0.0_f64, f64::max).max(1.0);
    let bw = (w - 40.0) / n;
    let col = if color.is_empty() { "var(--teal)".to_string() } else { color };
    view! { <svg class="chart" viewBox=format!("0 0 {w} {}", h + 22.0) width="100%" role="img">
        <line x1="0" x2=w y1=h y2=h stroke="var(--line)"/>
        {data.into_iter().enumerate().map(|(i, (label, v))| { let bh = (v / max * (h - 14.0)).max(if v > 0.0 { 2.0 } else { 0.0 }); let x = 20.0 + i as f64 * bw;
            view! { <g><rect x=x + bw * 0.18 y=h - bh width=bw * 0.64 height=bh rx="4" fill=col.clone() opacity=".9"><title>{format!("{label}: {}", fmt::money_f(v))}</title></rect>
              {(n <= 16.0 || i % 3 == 0).then(|| view! { <text x=x + bw / 2.0 y=h + 14.0 text-anchor="middle">{label.clone()}</text> })}</g> }
        }).collect_view()}
    </svg> }
}
#[component]
pub fn LineChart(data: Vec<(String, f64)>, #[prop(optional, into)] color: String) -> impl IntoView {
    let (w, h) = (640.0, 170.0); let n = data.len().max(2) as f64;
    let max = data.iter().map(|d| d.1).fold(f64::MIN, f64::max).max(1.0); let min = data.iter().map(|d| d.1).fold(f64::MAX, f64::min).min(0.0);
    let span = (max - min).max(1.0);
    let pts: Vec<(f64, f64)> = data.iter().enumerate().map(|(i, d)| (16.0 + i as f64 / (n - 1.0) * (w - 32.0), h - 12.0 - (d.1 - min) / span * (h - 28.0))).collect();
    let path = pts.iter().enumerate().map(|(i, p)| format!("{}{:.1} {:.1}", if i == 0 { "M" } else { "L" }, p.0, p.1)).collect::<Vec<_>>().join(" ");
    let area = format!("{path} L{:.1} {h} L16 {h}Z", pts.last().map(|p| p.0).unwrap_or(16.0));
    let col = if color.is_empty() { "var(--teal)".to_string() } else { color };
    let lbls: Vec<(f64, String)> = data.iter().enumerate().filter(|(i, _)| *i % (data.len() / 6).max(1) == 0).map(|(i, d)| (pts[i].0, d.0.clone())).collect();
    view! { <svg class="chart" viewBox=format!("0 0 {w} {}", h + 18.0) width="100%" role="img">
        <defs><linearGradient id="lg1" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color=col.clone() stop-opacity=".35"/><stop offset="1" stop-color=col.clone() stop-opacity="0"/></linearGradient></defs>
        <path d=area fill="url(#lg1)"/><path d=path fill="none" stroke=col.clone() stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round"/>
        {lbls.into_iter().map(|(x, l)| view! { <text x=x y=h + 12.0 text-anchor="middle">{l}</text> }).collect_view()}
    </svg> }
}
