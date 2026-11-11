//! Shared page patterns: create-a-document forms with line items.
use crate::{api, i18n::t, state, ui::*};
use leptos::prelude::*;
use leptos_router::hooks::use_navigate;
use serde_json::{json, Value};

pub struct NewDoc {
    pub title: &'static str, pub subtitle: &'static str, pub endpoint: &'static str, pub back_label: &'static str, pub back_route: &'static str,
    pub header: &'static [FF], pub lines_title: &'static str, pub lines: &'static [FF], pub autofill: Option<(&'static str, &'static str)>,
    /// Where to go after saving; `{rid}` is replaced by the new record's id.
    pub goto: &'static str, pub done: &'static str, pub submit_label: &'static str,
    pub min_lines: usize,
}

#[component]
pub fn NewDocPage(cfg: &'static NewDoc) -> impl IntoView {
    let st = FormState::new(cfg.header, None);
    let rows: RwSignal<Vec<serde_json::Map<String, Value>>> = RwSignal::new(if cfg.lines.is_empty() { vec![] } else { let mut m = serde_json::Map::new(); for c in cfg.lines { m.insert(c.name.into(), json!(c.def)); } vec![m] });
    let nav = use_navigate();
    let submit = move |_| {
        if st.busy.get_untracked() { return; }
        st.busy.set(true); st.err.set(None);
        let mut body = payload_with_bools(cfg.header, &st);
        if !cfg.lines.is_empty() { if let Value::Object(m) = &mut body { m.insert("lines".into(), json!(lines_payload(cfg.lines, &rows.get_untracked()))); } }
        let nav = nav.clone();
        leptos::task::spawn_local(async move {
            let r = api::post(cfg.endpoint, body).await;
            st.busy.set(false);
            match r { Ok(v) => { state::ok(cfg.done); state::refresh(); let rid = v["data"]["id"].as_str().unwrap_or("").to_string(); nav(&cfg.goto.replace("{rid}", &rid), Default::default()); } Err(e) => st.err.set(Some(e)) }
        });
    };
    view! {
        <div class="page">
            <PageHead title=t(cfg.title) subtitle=cfg.subtitle crumbs=vec![(t(cfg.back_label), cfg.back_route.to_string())]/>
            <div class="grid" style="max-width:1100px">
                <Card><FormFields fields=cfg.header.to_vec() st=st/></Card>
                {(!cfg.lines.is_empty()).then(|| view! { <Card title=t(cfg.lines_title)><LinesEditor cols=cfg.lines.to_vec() rows=rows err=st.err autofill=cfg.autofill.unwrap_or(("", ""))/></Card> })}
                <FormError st=st/>
                <div class="row"><a class="btn" href=cfg.back_route>{t("Cancel")}</a><button class="btn primary sp" disabled=move || st.busy.get() on:click=submit>{move || if st.busy.get() { view! { <Spinner/> }.into_any() } else { view! { {t(cfg.submit_label)} }.into_any() }}</button></div>
            </div>
        </div>
    }
}

/// A tab bar that remembers the last tab for the session.
#[component]
pub fn ModulePage(#[prop(into)] title: String, #[prop(into)] subtitle: String, tabs: Vec<(&'static str, String)>, active: RwSignal<&'static str>, #[prop(optional)] children: Option<Children>) -> impl IntoView {
    view! { <div class="page"><PageHead title=title subtitle=subtitle>{children.map(|c| c())}</PageHead><Tabs tabs=tabs active=active/></div> }
}
