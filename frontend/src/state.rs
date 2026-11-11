//! Global application state: session, language, theme, toasts. Kept in context plus a few thread-locals
//! so non-component code (the API client) can sign the user out when a session ends.
use leptos::prelude::*;
use serde::{Deserialize, Serialize};
use gloo_storage::{LocalStorage, Storage};

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Me {
    pub user_id: String, pub full_name: String, pub email: String, pub role: String,
    #[serde(default)] pub permissions: Vec<String>, #[serde(default)] pub is_superadmin: bool, #[serde(default)] pub locale: String,
    #[serde(default)] pub branch_id: Option<String>,
}
impl Me {
    pub fn can(&self, perm: &str) -> bool {
        if perm.is_empty() || perm == "auth" { return true; }
        let (module, action) = perm.split_once('.').unwrap_or((perm, ""));
        self.permissions.iter().any(|p| p == "*" || p == perm || (p.ends_with(".*") && &p[..p.len() - 2] == module) || (p.starts_with("*.") && &p[2..] == action))
    }
    pub fn initials(&self) -> String { self.full_name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect::<String>().to_uppercase() }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompanyInfo { pub id: String, pub name: String, pub name_bn: String, pub code: String, pub bin: String }

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Session { pub me: Me, pub company: CompanyInfo, pub companies: Vec<(String, String)> }

#[derive(Clone, Copy, PartialEq)]
pub enum Kind { Ok, Err, Warn }
#[derive(Clone)]
pub struct Toast { pub id: u32, pub kind: Kind, pub text: String }

#[derive(Clone, Copy)]
pub struct App {
    pub session: RwSignal<Option<Session>>,
    pub bn: RwSignal<bool>,
    pub dark: RwSignal<bool>,
    pub toasts: RwSignal<Vec<Toast>>,
    pub search_open: RwSignal<bool>,
    pub nav_open: RwSignal<bool>,
    pub unread: RwSignal<i64>,
    pub tick: RwSignal<u32>,
}

thread_local! { static APP: std::cell::OnceCell<App> = const { std::cell::OnceCell::new() }; }

pub fn init() -> App {
    let bn = LocalStorage::get::<bool>("weft.bn").unwrap_or(false);
    let dark = LocalStorage::get::<bool>("weft.dark").unwrap_or(false);
    let app = App { session: RwSignal::new(None), bn: RwSignal::new(bn), dark: RwSignal::new(dark), toasts: RwSignal::new(vec![]), search_open: RwSignal::new(false), nav_open: RwSignal::new(false), unread: RwSignal::new(0), tick: RwSignal::new(0) };
    APP.with(|a| { let _ = a.set(app); });
    app
}
pub fn app() -> App { APP.with(|a| *a.get().expect("app state not initialised")) }
pub fn is_bn() -> bool { APP.with(|a| a.get().map(|x| x.bn.get()).unwrap_or(false)) }
/// Reactive version: reading this in a view re-renders when the language changes.
pub fn bn_now() -> bool { app().bn.get() }

pub fn session_expired() {
    crate::api::clear_tokens();
    let a = app();
    if a.session.get_untracked().is_some() { a.session.set(None); }
}

pub fn me() -> Me { app().session.get().map(|s| s.me).unwrap_or_default() }
pub fn can(perm: &str) -> bool { app().session.with(|s| s.as_ref().map(|s| s.me.can(perm)).unwrap_or(false)) }

pub fn toast(kind: Kind, text: impl Into<String>) {
    let a = app();
    let id = (js_sys::Math::random() * 1.0e9) as u32;
    a.toasts.update(|t| t.push(Toast { id, kind, text: text.into() }));
    let toasts = a.toasts;
    leptos::task::spawn_local(async move { gloo_timers::future::TimeoutFuture::new(if kind == Kind::Err { 7000 } else { 3800 }).await; toasts.update(|t| t.retain(|x| x.id != id)); });
}
pub fn ok(text: impl Into<String>) { toast(Kind::Ok, text) }
pub fn fail(text: impl Into<String>) { toast(Kind::Err, text) }
/// Bump to make every list/detail view reload its data.
pub fn refresh() { app().tick.update(|t| *t += 1); }

pub fn apply_theme() {
    let a = app();
    if let Some(el) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.document_element()) {
        let _ = el.set_attribute("data-theme", if a.dark.get_untracked() { "dark" } else { "light" });
        let _ = el.set_attribute("lang", if a.bn.get_untracked() { "bn" } else { "en" });
    }
}
pub fn set_dark(v: bool) { let a = app(); a.dark.set(v); let _ = LocalStorage::set("weft.dark", v); apply_theme(); }
pub fn set_bn(v: bool) { let a = app(); a.bn.set(v); let _ = LocalStorage::set("weft.bn", v); apply_theme(); }
