//! Config-driven list pages and document pages. Each module declares *what* it shows; these components
//! provide search, filters, paging, forms, validation messages, workflow actions and refresh.
use crate::{api, fmt, i18n::t, state, ui::*};
use gloo_net::http::Method;
use leptos::prelude::*;
use leptos_router::hooks::use_navigate;
use serde_json::{json, Value};

// ---------------- list pages ----------------
#[derive(Clone, Copy)]
pub struct Filter { pub param: &'static str, pub label: &'static str, pub options: &'static [(&'static str, &'static str)] }
pub struct ListCfg {
    pub title: &'static str, pub subtitle: &'static str, pub endpoint: &'static str, pub cols: &'static [Col],
    pub search: bool, pub filters: &'static [Filter],
    /// Fields for the create/edit dialog (empty = read-only list).
    pub fields: &'static [FF], pub write: &'static str, pub new_label: &'static str, pub edit: bool, pub delete: bool,
    /// Row click goes to `detail + id`.
    pub detail: &'static str,
    /// "New" button goes here instead of opening a dialog.
    pub new_route: &'static str,
    pub extra_query: &'static str, pub empty_hint: &'static str,
    pub date_filter: bool,
}
pub const fn list(title: &'static str, endpoint: &'static str, cols: &'static [Col]) -> ListCfg {
    ListCfg { title, subtitle: "", endpoint, cols, search: true, filters: &[], fields: &[], write: "", new_label: "New", edit: false, delete: false, detail: "", new_route: "", extra_query: "", empty_hint: "", date_filter: false }
}

#[component]
pub fn ListPage(cfg: &'static ListCfg, #[prop(optional)] headless: bool, #[prop(optional)] children: Option<Children>) -> impl IntoView {
    let nav = use_navigate();
    let page = RwSignal::new(1u32);
    let q = RwSignal::new(String::new());
    let q_box = RwSignal::new(String::new());
    let fvals = RwSignal::new(vec![String::new(); cfg.filters.len()]);
    let from = RwSignal::new(String::new());
    let to = RwSignal::new(String::new());
    let tick = state::app().tick;
    let dialog: RwSignal<Option<Option<Value>>> = RwSignal::new(None); // Some(None) = create, Some(Some(row)) = edit
    let res = LocalResource::new(move || {
        let _ = tick.get();
        let mut pairs = vec![("page", page.get().to_string()), ("per_page", "25".to_string()), ("q", q.get())];
        let fv = fvals.get();
        for (i, f) in cfg.filters.iter().enumerate() { pairs.push((f.param, fv[i].clone())); }
        if cfg.date_filter { pairs.push(("from", from.get())); pairs.push(("to", to.get())); }
        let url = format!("{}{}{}", cfg.endpoint, api::qs(&pairs), cfg.extra_query);
        async move { api::get(&url).await }
    });
    let debounce = RwSignal::new(0u32);
    let on_search = move |e: leptos::ev::Event| {
        let v = event_target_value(&e); q_box.set(v.clone());
        let n = debounce.get_untracked() + 1; debounce.set(n);
        leptos::task::spawn_local(async move { gloo_timers::future::TimeoutFuture::new(300).await; if debounce.get_untracked() == n { q.set(v); page.set(1); } });
    };
    let can_write = move || cfg.write.is_empty() || state::can(cfg.write);
    let open_row = {
        let nav = nav.clone();
        move |row: Value| {
            if !cfg.detail.is_empty() { nav(&format!("{}{}", cfg.detail, row["id"].as_str().unwrap_or("")), Default::default()); }
            else if cfg.edit && can_write() && !cfg.fields.is_empty() { dialog.set(Some(Some(row))); }
        }
    };
    let clickable = !cfg.detail.is_empty() || (cfg.edit && !cfg.fields.is_empty());
    let new_click = { let nav = nav.clone(); move |_| if !cfg.new_route.is_empty() { nav(cfg.new_route, Default::default()) } else { dialog.set(Some(None)) } };
    let new_click2 = new_click.clone();
    view! {
        {(!headless).then(|| view! { <PageHead title=t(cfg.title) subtitle=cfg.subtitle>
            {children.map(|c| c())}
            {move || ((!cfg.fields.is_empty() || !cfg.new_route.is_empty()) && can_write()).then(|| view! { <button class="btn primary" on:click=new_click.clone()><Icon name="plus" size=16/>{t(cfg.new_label)}</button> })}
        </PageHead> })}
        <div class="card flush">
            <div class="toolbar no-print">
                {(headless && (!cfg.fields.is_empty() || !cfg.new_route.is_empty())).then(|| view! { {move || can_write().then(|| view! { <button class="btn primary sm" on:click=new_click2.clone()><Icon name="plus" size=15/>{t(cfg.new_label)}</button> })} })}
                {cfg.search.then(|| view! { <input class="input search" placeholder=t("Search") prop:value=move || q_box.get() on:input=on_search/> })}
                {cfg.filters.iter().enumerate().map(|(i, f)| view! { <select class="select" on:change=move |e| { fvals.update(|v| v[i] = event_target_value(&e)); page.set(1); }>
                    <option value="">{t(f.label)}</option>{f.options.iter().map(|(v, l)| view! { <option value=*v>{t(l)}</option> }).collect_view()}</select> }).collect_view()}
                {cfg.date_filter.then(|| view! { <input class="input" type="date" title="From" on:change=move |e| { from.set(event_target_value(&e)); page.set(1); }/><input class="input" type="date" title="To" on:change=move |e| { to.set(event_target_value(&e)); page.set(1); }/> })}
                <button class="btn ghost sm sp" on:click=move |_| state::refresh() title="Reload"><Icon name="refresh" size=15/></button>
            </div>
            {move || match res.get() {
                None => view! { <div style="padding:20px"><div class="col"><div class="skel" style="width:90%"></div><div class="skel" style="width:70%"></div><div class="skel" style="width:80%"></div></div></div> }.into_any(),
                Some(Err(e)) => view! { <div style="padding:16px"><ErrorBox err=e/></div> }.into_any(),
                Some(Ok(v)) => {
                    let rows = v["data"].as_array().cloned().unwrap_or_default();
                    let total = v["total"].as_i64().unwrap_or(rows.len() as i64);
                    let pages = ((total as f64) / 25.0).ceil().max(1.0) as u32;
                    let open = open_row.clone();
                    view! {
                        {if rows.is_empty() { view! { <Empty title=t("Nothing here yet") hint=cfg.empty_hint/> }.into_any() } else { table(cfg.cols, rows, if clickable { Some(Callback::new(move |r| open(r))) } else { None }) }}
                        <div class="pager no-print"><span>{fmt::digits(format!("{total} {}", if total == 1 { "record" } else { "records" }))}</span>
                            <span class="sp"></span>
                            <button class="btn sm" disabled=move || page.get() <= 1 on:click=move |_| page.update(|p| *p -= 1)>"‹"</button>
                            <span>{move || fmt::digits(format!("{} / {}", page.get(), pages))}</span>
                            <button class="btn sm" disabled=move || page.get() >= pages on:click=move |_| page.update(|p| *p += 1)>"›"</button></div>
                    }.into_any()
                }
            }}
        </div>
        {move || dialog.get().map(|row| view! { <RecordDialog cfg=cfg row=row on_close=Callback::new(move |_| dialog.set(None))/> })}
    }
}

#[component]
fn RecordDialog(cfg: &'static ListCfg, row: Option<Value>, on_close: Callback<()>) -> impl IntoView {
    let editing = row.is_some();
    let id = row.as_ref().and_then(|r| r["id"].as_str()).unwrap_or("").to_string();
    let st = FormState::new(cfg.fields, row.as_ref());
    let confirm_delete = RwSignal::new(false);
    let id2 = id.clone();
    let save = move |_| {
        if st.busy.get_untracked() { return; }
        st.busy.set(true); st.err.set(None);
        let body = payload_with_bools(cfg.fields, &st); let id = id2.clone();
        leptos::task::spawn_local(async move {
            let r = if editing { api::put(&format!("{}/{}", cfg.endpoint, id), body).await } else { api::post(cfg.endpoint, body).await };
            st.busy.set(false);
            match r { Ok(_) => { state::ok(if editing { "Saved" } else { "Created" }); state::refresh(); on_close.run(()); } Err(e) => st.err.set(Some(e)) }
        });
    };
    let del = { let id = id.clone(); move |_| {
        let id = id.clone();
        leptos::task::spawn_local(async move { match api::delete(&format!("{}/{}", cfg.endpoint, id)).await { Ok(_) => { state::ok("Deleted"); state::refresh(); on_close.run(()); } Err(e) => state::fail(e.friendly()) } });
    } };
    let title = format!("{} {}", if editing { t("Edit") } else { t("New") }, t(cfg.title).to_lowercase());
    view! {
        <Modal title=title on_close=on_close footer=footer(move || view! {
            {(editing && cfg.delete).then(|| view! { <button class="btn danger" style="margin-right:auto" on:click=move |_| confirm_delete.set(true)>{t("Delete")}</button> })}
            <button class="btn" on:click=move |_| on_close.run(())>{t("Cancel")}</button>
            <button class="btn primary" on:click=save.clone() disabled=move || st.busy.get()>{move || if st.busy.get() { view! { <Spinner/> }.into_any() } else { view! { {t("Save")} }.into_any() }}</button> }.into_any())>
            <FormFields fields=cfg.fields.to_vec() st=st/>
            <FormError st=st/>
            {move || confirm_delete.get().then(|| view! { <Confirm title="Delete this record?" text="This cannot be undone. Records already used in transactions cannot be deleted; mark them inactive instead." action="Delete" danger=true on_close=Callback::new(move |_| confirm_delete.set(false)) on_yes=Callback::new(del.clone())/> })}
        </Modal>
    }
}

// ---------------- document pages ----------------
#[derive(Clone, Copy)]
pub struct Fact { pub label: &'static str, pub key: &'static str, pub kind: CK }
pub const fn fact(label: &'static str, key: &'static str, kind: CK) -> Fact { Fact { label, key, kind } }
/// Ask for a quantity per line of the document (e.g. what arrived).
#[derive(Clone, Copy)]
pub struct LineQty { pub key: &'static str, pub send: &'static str, pub show: &'static str, pub total: &'static str, pub done: &'static str, pub label: &'static str }
#[derive(Clone, Copy)]
pub struct DocAction {
    pub label: &'static str,
    /// Appended to `endpoint/{id}`, unless `abs` is set.
    pub path: &'static str, pub abs: &'static str, pub inject_id: &'static str,
    pub method: &'static str, pub perm: &'static str,
    /// Show only in these statuses (empty = always).
    pub when: &'static [&'static str],
    pub primary: bool, pub danger: bool, pub confirm: &'static str,
    pub fields: &'static [FF], pub line_qty: Option<LineQty>,
    pub done: &'static str, pub goto: &'static str,
    /// JSON object merged into every request body, e.g. `{"cancel":true}`.
    pub fixed: &'static str,
    /// Hide the button when this field of the document is set (e.g. `invoiced`).
    pub hide_if: &'static str,
}
pub const fn action(label: &'static str, path: &'static str, perm: &'static str) -> DocAction {
    DocAction { label, path, abs: "", inject_id: "", method: "POST", perm, when: &[], primary: false, danger: false, confirm: "", fields: &[], line_qty: None, done: "Done", goto: "", fixed: "", hide_if: "" }
}
impl DocAction {
    pub const fn when(mut self, w: &'static [&'static str]) -> Self { self.when = w; self }
    pub const fn primary(mut self) -> Self { self.primary = true; self }
    pub const fn danger(mut self) -> Self { self.danger = true; self }
    pub const fn confirm(mut self, c: &'static str) -> Self { self.confirm = c; self }
    pub const fn fields(mut self, f: &'static [FF]) -> Self { self.fields = f; self }
    pub const fn lines(mut self, l: LineQty) -> Self { self.line_qty = Some(l); self }
    pub const fn done(mut self, d: &'static str) -> Self { self.done = d; self }
    pub const fn goto(mut self, g: &'static str) -> Self { self.goto = g; self }
    pub const fn abs(mut self, p: &'static str, inject: &'static str) -> Self { self.abs = p; self.inject_id = inject; self }
    pub const fn hide_if(mut self, k: &'static str) -> Self { self.hide_if = k; self }
    pub const fn fixed(mut self, j: &'static str) -> Self { self.fixed = j; self }
    pub const fn method(mut self, m: &'static str) -> Self { self.method = m; self }
}
/// A button on a table row: calls `method path` with `{field}` placeholders filled from the row.
#[derive(Clone, Copy)]
pub struct RowAction { pub label: &'static str, pub method: &'static str, pub path: &'static str, pub body: &'static str, pub perm: &'static str, pub show_if: &'static str, pub show_not: &'static str }
#[derive(Clone, Copy)]
pub struct DocTable { pub title: &'static str, pub key: &'static str, pub cols: &'static [Col], pub row_action: Option<RowAction> }
pub const fn dtable(title: &'static str, key: &'static str, cols: &'static [Col]) -> DocTable { DocTable { title, key, cols, row_action: None } }
pub struct DocCfg {
    pub endpoint: &'static str, pub list_route: &'static str, pub list_label: &'static str, pub title_key: &'static str, pub subtitle_key: &'static str,
    pub status_key: &'static str, pub steps: &'static [(&'static str, &'static str)],
    pub facts: &'static [Fact], pub tables: &'static [DocTable], pub actions: &'static [DocAction], pub notes_key: &'static str,
}

fn subst(tpl: &str, row: &Value) -> String {
    let mut out = String::new(); let mut rest = tpl;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        if let Some(j) = rest[i..].find('}') { let key = &rest[i + 1..i + j]; out.push_str(&dig(row, key).as_str().map(str::to_string).unwrap_or_else(|| fmt::text(dig(row, key)))); rest = &rest[i + j + 1..]; } else { rest = &rest[i..]; break; }
    }
    out.push_str(rest); out
}

#[component]
pub fn DocPage(cfg: &'static DocCfg, id: String, #[prop(optional)] extra: Option<Callback<Value, AnyView>>) -> impl IntoView {
    let tick = state::app().tick;
    let id_s = StoredValue::new(id.clone());
    let res = LocalResource::new(move || { let _ = tick.get(); let url = format!("{}/{}", cfg.endpoint, id_s.get_value()); async move { api::get(&url).await } });
    let acting: RwSignal<Option<DocAction>> = RwSignal::new(None);
    view! {
        {move || match res.get() {
            None => view! { <Loading/> }.into_any(),
            Some(Err(e)) => view! { <PageHead title=t(cfg.list_label) crumbs=vec![(t(cfg.list_label), cfg.list_route.to_string())]/><ErrorBox err=e/> }.into_any(),
            Some(Ok(v)) => {
                let d = v["data"].clone();
                let status = d[cfg.status_key].as_str().unwrap_or("").to_string();
                let title = fmt::text(&d[cfg.title_key]);
                let sub = if cfg.subtitle_key.is_empty() { String::new() } else { fmt::text(dig(&d, cfg.subtitle_key)) };
                let acts: Vec<DocAction> = cfg.actions.iter().filter(|a| (a.when.is_empty() || a.when.contains(&status.as_str())) && (a.perm.is_empty() || state::can(a.perm)) && (a.hide_if.is_empty() || !truthy(&d[a.hide_if]))).copied().collect();
                let step_idx = cfg.steps.iter().position(|s| s.0 == status);
                let d2 = d.clone(); let d_extra = d.clone(); let d_tables = d.clone(); let d_notes = d.clone(); let d_appr = d.clone();
                view! {
                    <PageHead title=title subtitle=if sub == "—" { String::new() } else { sub } crumbs=vec![(t(cfg.list_label), cfg.list_route.to_string())]>
                        {(!cfg.status_key.is_empty()).then(|| view! { <Badge status=status.clone()/> })}
                        {acts.into_iter().map(|a| view! { <button class=format!("btn {}{}", if a.primary { "primary" } else { "" }, if a.danger { " danger" } else { "" }) on:click=move |_| acting.set(Some(a))>{t(a.label)}</button> }).collect_view()}
                    </PageHead>
                    {(!cfg.steps.is_empty()).then(|| view! { <div class="card" style="padding:14px 20px 4px"><div class="steps">{cfg.steps.iter().enumerate().map(|(i, s)| { let cls = match step_idx { Some(c) if i < c => "step done", Some(c) if i == c => "step now", _ => "step" }; view! { <div class=cls><i></i>{t(s.1)}</div> } }).collect_view()}</div></div> })}
                    <div class="grid" style="margin-top:16px">
                        <Card><div class="facts">{cfg.facts.iter().map(|f| { let c = Col { key: f.key, label: f.label, kind: f.kind }; view! { <div class="fact"><small>{t(f.label)}</small><div>{cell(&d2, &c)}</div></div> } }).collect_view()}</div>
                            {(!cfg.notes_key.is_empty() && d_notes[cfg.notes_key].as_str().map(|s| !s.is_empty()).unwrap_or(false)).then(|| view! { <p class="mut" style="margin-top:16px">{fmt::text(&d_notes[cfg.notes_key])}</p> })}</Card>
                        {extra.map(|e| e.run(d_extra.clone()))}
                        {cfg.tables.iter().map(|tb| { let rows = d_tables[tb.key].as_array().cloned().unwrap_or_default();
                            if rows.is_empty() && tb.key != "lines" { return ().into_any(); }
                            let ra = tb.row_action; let doc_id = id_s.get_value();
                            view! { <div class="card flush"><div style="padding:18px 20px 0"><h3 style="margin-bottom:10px">{t(tb.title)}</h3></div>
                                {if let Some(a) = ra { render_with_row_action(tb.cols, rows, a, cfg.endpoint, doc_id) } else { table(tb.cols, rows, None) }}</div> }.into_any()
                        }).collect_view()}
                        {approvals_card(&d_appr)}
                    </div>
                    {move || acting.get().map(|a| view! { <ActionDialog cfg=cfg action=a doc=d.clone() on_close=Callback::new(move |_| acting.set(None))/> })}
                }.into_any()
            }
        }}
    }
}

fn truthy(v: &Value) -> bool { match v { Value::Null => false, Value::Bool(b) => *b, Value::String(s) => !s.is_empty(), _ => true } }

fn approvals_card(d: &Value) -> AnyView {
    let list = d["approvals"].as_array().cloned().unwrap_or_default();
    if list.is_empty() { return ().into_any(); }
    view! { <Card title=t("Approvals")><div class="timeline">{list.into_iter().map(|a| view! { <div class="tl"><div class="row"><b>{format!("Level {} · {}", a["level"], fmt::nice(a["approver_role"].as_str().unwrap_or("")))}</b><Badge status=a["status"].as_str().unwrap_or("").to_string()/></div>
        {a["comment"].as_str().filter(|c| !c.is_empty()).map(|c| view! { <small>{c.to_string()}</small> })}</div> }).collect_view()}</div></Card> }.into_any()
}

fn render_with_row_action(cols: &'static [Col], rows: Vec<Value>, a: RowAction, endpoint: &'static str, doc_id: String) -> AnyView {
    view! { <div class="tbl-wrap"><table>
        <thead><tr>{cols.iter().map(|c| view! { <th class=if is_right(c.kind) { "r" } else { "" }>{t(c.label)}</th> }).collect_view()}<th></th></tr></thead>
        <tbody>{rows.into_iter().map(|r| { let r2 = r.clone(); let doc = doc_id.clone();
            let show = state::can(a.perm) && (a.show_if.is_empty() || r[a.show_if].as_str().is_some()) && (a.show_not.is_empty() || !a.show_not.split('|').any(|s| fmt::text(&r["status"]) == s));
            view! { <tr>{cols.iter().map(|c| view! { <td class=if is_right(c.kind) { "r" } else { "" }>{cell(&r, c)}</td> }).collect_view()}
                <td class="r">{show.then(|| { let r3 = r2.clone(); let doc = doc.clone(); view! { <button class="btn sm" on:click=move |_| {
                    let path = format!("{}/{}{}", endpoint, doc, subst(a.path, &r3)); let body: Value = serde_json::from_str(&subst(a.body, &r3)).unwrap_or(json!({}));
                    let m = match a.method { "PUT" => Method::PUT, "DELETE" => Method::DELETE, _ => Method::POST }; let nobody = a.method == "DELETE";
                    leptos::task::spawn_local(async move { match api::request(m, &path, if nobody { None } else { Some(body) }).await { Ok(_) => { state::ok("Updated"); state::refresh(); } Err(e) => state::fail(e.friendly()) } });
                }>{t(a.label)}</button> } })}</td></tr> }
        }).collect_view()}</tbody></table></div> }.into_any()
}

#[component]
fn ActionDialog(cfg: &'static DocCfg, action: DocAction, doc: Value, on_close: Callback<()>) -> impl IntoView {
    let nav = StoredValue::new_local(use_navigate());
    let st = FormState::new(action.fields, None);
    let doc_id = doc["id"].as_str().unwrap_or("").to_string();
    let rows: RwSignal<Vec<(String, String, String, String)>> = RwSignal::new(match action.line_qty { Some(lq) => doc[lq.key].as_array().cloned().unwrap_or_default().iter().filter_map(|l| {
        let remaining = fmt::num(&l[lq.total]) - fmt::num(&l[lq.done]); if remaining <= 0.0 { return None; }
        Some((l["id"].as_str().unwrap_or("").to_string(), fmt::text(&l[lq.show]), fmt::qty(&json!(remaining)), remaining.to_string())) }).collect(), None => vec![] });
    let qtys = RwSignal::new(rows.get_untracked().iter().map(|r| r.3.clone()).collect::<Vec<_>>());
    let needs_form = !action.fields.is_empty() || action.line_qty.is_some();
    let run = move |_: ()| {
        if st.busy.get_untracked() { return; }
        st.busy.set(true); st.err.set(None);
        let mut body = payload_with_bools(action.fields, &st);
        if let Some(lq) = action.line_qty {
            let ls: Vec<Value> = rows.get_untracked().iter().zip(qtys.get_untracked()).filter(|(_, q)| !q.is_empty() && q.parse::<f64>().map(|x| x > 0.0).unwrap_or(false)).map(|(r, q)| json!({ lq.send: r.0, "qty": q })).collect();
            if ls.is_empty() { st.busy.set(false); st.err.set(Some(api::ApiError { status: 422, code: "business_rule".into(), message: "Enter a quantity for at least one line".into(), fields: vec![] })); return; }
            if let Value::Object(m) = &mut body { m.insert("lines".into(), json!(ls)); }
        }
        if !action.fixed.is_empty() { if let (Value::Object(m), Ok(Value::Object(f))) = (&mut body, serde_json::from_str::<Value>(action.fixed)) { for (k, v) in f { m.insert(k, v); } } }
        if !action.inject_id.is_empty() { if let Value::Object(m) = &mut body { m.insert(action.inject_id.into(), json!(doc_id.clone())); } }
        let path = if action.abs.is_empty() { format!("{}/{}{}", cfg.endpoint, doc_id, action.path) } else { action.abs.to_string() };
        let method = match action.method { "PUT" => Method::PUT, "DELETE" => Method::DELETE, _ => Method::POST };
        let goto = action.goto; let did = doc_id.clone();
        leptos::task::spawn_local(async move {
            let r = api::request(method, &path, if action.method == "DELETE" { None } else { Some(body) }).await;
            st.busy.set(false);
            match r {
                Ok(v) => { state::ok(action.done); state::refresh(); on_close.run(());
                    if !goto.is_empty() { let rid = v["data"]["id"].as_str().unwrap_or("").to_string(); nav.with_value(|n| n(&goto.replace("{id}", &did).replace("{rid}", &rid), Default::default())); } }
                Err(e) => st.err.set(Some(e)),
            }
        });
    };
    if !needs_form {
        // simple confirmation, or run straight away
        if action.confirm.is_empty() { Effect::new({ let run = run.clone(); move |_| run(()) }); return ().into_any(); }
        return view! { <Confirm title=action.label text=action.confirm action=action.label danger=action.danger on_close=on_close on_yes=Callback::new(move |_| run(()))/> }.into_any();
    }
    let run2 = run.clone();
    view! { <Modal title=t(action.label) on_close=on_close wide=action.line_qty.is_some() footer=footer(move || { let run = run2.clone(); view! {
            <button class="btn" on:click=move |_| on_close.run(())>{t("Cancel")}</button>
            <button class=if action.danger { "btn danger" } else { "btn primary" } disabled=move || st.busy.get() on:click=move |_| run(())>{move || if st.busy.get() { view! { <Spinner/> }.into_any() } else { view! { {t(action.label)} }.into_any() }}</button> }.into_any() })>
        {(!action.confirm.is_empty()).then(|| view! { <p class="mut" style="margin-bottom:14px">{action.confirm}</p> })}
        {action.line_qty.map(|lq| view! { <div class="tbl-wrap" style="border:1px solid var(--line);border-radius:12px;margin-bottom:16px"><table>
            <thead><tr><th>{t("Item")}</th><th class="r">{t("Remaining")}</th><th class="r">{lq.label}</th></tr></thead>
            <tbody>{rows.get_untracked().into_iter().enumerate().map(|(i, r)| view! { <tr><td>{r.1.clone()}</td><td class="r num">{r.2.clone()}</td><td class="r"><input class="input" style="width:130px;text-align:right" type="number" min="0" step="any" prop:value=move || qtys.get()[i].clone() on:input=move |e| qtys.update(|q| q[i] = event_target_value(&e))/></td></tr> }).collect_view()}</tbody></table></div> })}
        {(!action.fields.is_empty()).then(|| view! { <FormFields fields=action.fields.to_vec() st=st/> })}
        <FormError st=st/>
    </Modal> }.into_any()
}
