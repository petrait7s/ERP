//! Application shell: sign-in, navigation by role, global search, notifications and routing.
use crate::{api, fmt, i18n::t, pages, state::{self, CompanyInfo, Me, Session}, ui::*};
use leptos::prelude::*;
use leptos_router::{components::{Route, Router, Routes, A}, hooks::{use_location, use_navigate}, path};
use serde_json::{json, Value};

struct NavItem { href: &'static str, label: &'static str, icon: &'static str, perm: &'static str }
struct NavGroup { title: &'static str, items: &'static [NavItem] }
const NAV: &[NavGroup] = &[
    NavGroup { title: "Overview", items: &[NavItem { href: "/", label: "Dashboard", icon: "home", perm: "auth" }] },
    NavGroup { title: "Operations", items: &[
        NavItem { href: "/imports", label: "Imports & Voyages", icon: "ship", perm: "import.read" },
        NavItem { href: "/landed-costs", label: "Landed Cost", icon: "calc", perm: "import.read" },
        NavItem { href: "/customs", label: "Customs", icon: "stamp", perm: "import.read" },
        NavItem { href: "/replenishment", label: "Replenishment", icon: "chart", perm: "inventory.read" },
        NavItem { href: "/procurement", label: "Procurement", icon: "cart", perm: "purchase.read" },
        NavItem { href: "/order-book", label: "Order Book", icon: "board", perm: "sales.read" },
        NavItem { href: "/inventory", label: "Inventory & Warehouse", icon: "box", perm: "inventory.read" },
        NavItem { href: "/sales", label: "Sales & Customers", icon: "users", perm: "sales.read" },
    ] },
    NavGroup { title: "Finance", items: &[
        NavItem { href: "/ledger", label: "Ledger Studio", icon: "book", perm: "finance.read" },
        NavItem { href: "/accounting", label: "Accounting & Finance", icon: "wallet", perm: "finance.read" },
        NavItem { href: "/cash", label: "Cash & FX", icon: "swap", perm: "finance.read" },
        NavItem { href: "/tax", label: "VAT & Tax", icon: "tag", perm: "tax.read" },
    ] },
    NavGroup { title: "People", items: &[NavItem { href: "/hr", label: "HR & Payroll", icon: "id", perm: "hr.read" }] },
    NavGroup { title: "Control", items: &[
        NavItem { href: "/policy", label: "Policy & Approvals", icon: "shield", perm: "approvals.read" },
        NavItem { href: "/reports", label: "Reports & Analytics", icon: "report", perm: "reports" },
        NavItem { href: "/admin", label: "Administration", icon: "gear", perm: "admin.read" },
    ] },
];
/// The permission a screen needs, found from the navigation table (detail pages inherit their module's).
fn page_perm(path: &str) -> &'static str {
    for g in NAV { for i in g.items { if i.href != "/" && (path == i.href || path.starts_with(&format!("{}/", i.href)) || related(i.href, path)) { return i.perm; } } }
    "auth"
}
fn allowed(perm: &str) -> bool { if perm == "reports" { state::can("finance.read") || state::can("sales.read") || state::can("inventory.read") } else { state::can(perm) } }

pub async fn load_session() -> bool {
    match api::get("/auth/me").await {
        Ok(v) => {
            let me: Me = serde_json::from_value(v["data"]["user"].clone()).unwrap_or_default();
            let c = &v["data"]["company"];
            let company = CompanyInfo { id: fmt::text(&c["id"]), name: c["name"].as_str().unwrap_or("").into(), name_bn: c["name_bn"].as_str().unwrap_or("").into(), code: c["code"].as_str().unwrap_or("").into(), bin: c["bin"].as_str().unwrap_or("").into() };
            if me.locale == "bn" && crate::state::app().bn.get_untracked() == false && gloo_storage::LocalStorage::get::<bool>("weft.bn").is_err() { state::set_bn(true); }
            let companies = api::get("/companies").await.ok().and_then(|v| v["data"].as_array().cloned()).unwrap_or_default().into_iter().map(|c| (fmt::text(&c["id"]), fmt::text(&c["name"]))).collect();
            state::app().session.set(Some(Session { me, company, companies }));
            true
        }
        Err(_) => false,
    }
}
use gloo_storage::Storage;

#[component]
pub fn App() -> impl IntoView {
    let app = state::app();
    state::apply_theme();
    let booted = RwSignal::new(false);
    leptos::task::spawn_local(async move {
        if api::token().is_some() { load_session().await; }
        booted.set(true);
        if let Some(b) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.get_element_by_id("boot")) { b.remove(); }
    });
    view! {
        <div class="ambient"><i></i><i></i><i></i></div>
        {move || if !booted.get() { ().into_any() } else if app.session.get().is_none() { view! { <Login/> }.into_any() } else { let _ = app.bn.get(); view! { <Shell/> }.into_any() }}
        <Toasts/>
    }
}

#[component]
fn Toasts() -> impl IntoView {
    let app = state::app();
    view! { <div class="toasts" role="status" aria-live="polite">{move || app.toasts.get().into_iter().map(|x| view! { <div class=match x.kind { state::Kind::Err => "toast err", state::Kind::Warn => "toast warn", _ => "toast" }><Icon name=if x.kind == state::Kind::Ok { "check" } else { "alert" }/><span>{x.text}</span></div> }).collect_view()}</div> }
}

#[component]
fn Login() -> impl IntoView {
    let email = RwSignal::new(String::new()); let pw = RwSignal::new(String::new());
    let err: RwSignal<Option<String>> = RwSignal::new(None); let busy = RwSignal::new(false);
    let setup_mode = RwSignal::new(false);
    let f = [RwSignal::new(String::new()), RwSignal::new(String::new()), RwSignal::new(String::new()), RwSignal::new(String::new()), RwSignal::new(String::new())];
    let submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        if busy.get_untracked() { return; }
        busy.set(true); err.set(None);
        leptos::task::spawn_local(async move {
            let r = if setup_mode.get_untracked() {
                api::post_public("/setup", json!({"company_code": f[0].get_untracked(), "company_name": f[1].get_untracked(), "bin": f[2].get_untracked(), "admin_name": f[3].get_untracked(), "admin_email": email.get_untracked(), "admin_password": pw.get_untracked()})).await.map(|_| ())
            } else { Ok(()) };
            let r = match r { Ok(_) => api::post_public("/auth/login", json!({"email": email.get_untracked(), "password": pw.get_untracked()})).await, Err(e) => Err(e) };
            match r {
                Ok(v) => { api::store_tokens(v["data"]["access_token"].as_str().unwrap_or(""), v["data"]["refresh_token"].as_str().unwrap_or("")); load_session().await; }
                Err(e) => err.set(Some(if e.status == 409 && setup_mode.get_untracked() { "This system is already set up. Please sign in.".into() } else { e.friendly() })),
            }
            busy.set(false);
        });
    };
    view! {
        <div class="login">
            <div class="hero"><div class="row" style="position:relative"><div class="logo">"W"</div><b style="font-size:20px">"WEFT"</b></div>
                <div><h1>{move || if state::bn_now() { "আমদানি ব্যবসার সম্পূর্ণ ইআরপি" } else { "Your import business, in one calm place." }}</h1>
                    <p>{move || if state::bn_now() { "এলসি, জাহাজ, শুল্ক, ল্যান্ডেড কস্ট, মজুদ, বিক্রয়, মূসক এবং হিসাব — সব এক জায়গায়।" } else { "From the supplier's purchase order to the customer's receipt: letters of credit, shipments, customs, landed cost, stock, VAT and the books, always in step." }}</p>
                    <ul><li>"Bangladesh VAT, Mushak forms and customs duty built in"</li><li>"BDT first, with live exposure on USD, CNY and EUR payments"</li><li>"Every taka traceable from the invoice to the ledger"</li></ul></div>
                <small style="color:#6F819B;position:relative">"© WEFT ERP"</small></div>
            <div class="panel"><form class="box col" style="gap:16px" on:submit=submit>
                <div class="row"><div><h2>{move || if setup_mode.get() { "Set up your company".to_string() } else { t("Sign in") }}</h2><p class="mut">{move || if setup_mode.get() { "Create the company and the owner account." } else { "Welcome back." }}</p></div>
                    <button type="button" class="lang-btn sp" on:click=move |_| state::set_bn(!state::app().bn.get_untracked())>{move || if state::bn_now() { "English" } else { "বাংলা" }}</button></div>
                {move || setup_mode.get().then(|| view! { <div class="col">
                    <div class="field"><label>"Company code"</label><input class="input" placeholder="PTC" prop:value=move || f[0].get() on:input=move |e| f[0].set(event_target_value(&e))/></div>
                    <div class="field"><label>"Company name"</label><input class="input" prop:value=move || f[1].get() on:input=move |e| f[1].set(event_target_value(&e))/></div>
                    <div class="field"><label>"BIN (VAT registration)"</label><input class="input" placeholder="000123456-0101" prop:value=move || f[2].get() on:input=move |e| f[2].set(event_target_value(&e))/></div>
                    <div class="field"><label>"Your name"</label><input class="input" prop:value=move || f[3].get() on:input=move |e| f[3].set(event_target_value(&e))/></div></div> })}
                <div class="field"><label>{t("Email")}</label><input class="input" type="email" autocomplete="username" prop:value=move || email.get() on:input=move |e| email.set(event_target_value(&e))/></div>
                <div class="field"><label>{t("Password")}</label><input class="input" type="password" autocomplete="current-password" prop:value=move || pw.get() on:input=move |e| pw.set(event_target_value(&e))/></div>
                {move || err.get().map(|e| view! { <div class="alert err"><Icon name="alert"/><div>{e}</div></div> })}
                <button class="btn primary" style="height:46px" disabled=move || busy.get()>{move || if busy.get() { view! { <Spinner/> }.into_any() } else { view! { {move || if setup_mode.get() { "Create company".to_string() } else { t("Sign in") }} }.into_any() }}</button>
                <button type="button" class="btn ghost" on:click=move |_| setup_mode.update(|s| *s = !*s)>{move || if setup_mode.get() { "I already have an account" } else { "First time here? Set up the company" }}</button>
            </form></div>
        </div>
    }
}

#[component]
fn Shell() -> impl IntoView {
    let app = state::app();
    view! {
        <Router>
            <div class="shell">
                <Sidebar/>
                {move || app.nav_open.get().then(|| view! { <div class="scrim" on:click=move |_| app.nav_open.set(false)></div> })}
                <div class="main"><Topbar/>
                    <main><MainArea/></main>
                </div>
            </div>
            <SearchPalette/>
        </Router>
    }
}

#[component]
fn MainArea() -> impl IntoView {
    let loc = use_location();
    view! {
                        <Show when=move || allowed(page_perm(&loc.pathname.get())) fallback=|| view! { <div class="page"><div class="card"><Empty title="This area is not part of your role" hint="Ask an administrator if you need access."/><div style="text-align:center;padding-bottom:20px"><a class="btn" href="/">"Back to dashboard"</a></div></div></div> }>
                        <Routes fallback=|| view! { <div class="page"><Empty title="Page not found" hint="Use the menu to find what you need."/></div> }>
                            <Route path=path!("/") view=pages::dashboard::Dashboard/>
                            <Route path=path!("/imports") view=pages::imports::Imports/>
                            <Route path=path!("/shipments/:id") view=pages::imports::ShipmentPage/>
                            <Route path=path!("/shipments/new") view=pages::imports::NewShipment/>
                            <Route path=path!("/lcs/new") view=pages::imports::NewLc/>
                            <Route path=path!("/lcs/:id") view=pages::imports::LcPage/>
                            <Route path=path!("/foreign-payments/new") view=pages::imports::NewForeignPayment/>
                            <Route path=path!("/landed-costs") view=pages::landed::LandedCosts/>
                            <Route path=path!("/landed-costs/:id") view=pages::landed::LandedCostPage/>
                            <Route path=path!("/customs") view=pages::customs::Customs/>
                            <Route path=path!("/bills-of-entry/new") view=pages::customs::NewBe/>
                            <Route path=path!("/bills-of-entry/:id") view=pages::customs::BePage/>
                            <Route path=path!("/replenishment") view=pages::replenishment::Replenishment/>
                            <Route path=path!("/procurement") view=pages::procurement::Procurement/>
                            <Route path=path!("/purchase-orders/new") view=pages::procurement::NewPo/>
                            <Route path=path!("/purchase-orders/:id") view=pages::procurement::PoPage/>
                            <Route path=path!("/requisitions/new") view=pages::procurement::NewRequisition/>
                            <Route path=path!("/goods-receipts/:id") view=pages::procurement::GrnPage/>
                            <Route path=path!("/supplier-bills/new") view=pages::procurement::NewBill/>
                            <Route path=path!("/supplier-bills/:id") view=pages::procurement::BillPage/>
                            <Route path=path!("/order-book") view=pages::sales::OrderBook/>
                            <Route path=path!("/inventory") view=pages::inventory::Inventory/>
                            <Route path=path!("/stock-transfers/new") view=pages::inventory::NewTransfer/>
                            <Route path=path!("/stock-adjustments/new") view=pages::inventory::NewAdjustment/>
                            <Route path=path!("/stock-opening/new") view=pages::inventory::NewOpening/>
                            <Route path=path!("/sales") view=pages::sales::Sales/>
                            <Route path=path!("/sales-orders/new") view=pages::sales::NewSo/>
                            <Route path=path!("/sales-orders/:id") view=pages::sales::SoPage/>
                            <Route path=path!("/deliveries/:id") view=pages::sales::DeliveryPage/>
                            <Route path=path!("/sales-invoices/:id") view=pages::sales::InvoicePage/>
                            <Route path=path!("/customer-receipts/new") view=pages::sales::NewReceipt/>
                            <Route path=path!("/sales-returns/new") view=pages::sales::NewReturn/>
                            <Route path=path!("/ledger") view=pages::ledger::Ledger/>
                            <Route path=path!("/journals/new") view=pages::ledger::NewJournal/>
                            <Route path=path!("/journals/:id") view=pages::ledger::JournalPage/>
                            <Route path=path!("/accounting") view=pages::accounting::Accounting/>
                            <Route path=path!("/supplier-payments/new") view=pages::accounting::NewSupplierPayment/>
                            <Route path=path!("/bank-transfers/new") view=pages::accounting::NewBankTransfer/>
                            <Route path=path!("/cash") view=pages::cash::Cash/>
                            <Route path=path!("/tax") view=pages::tax::Tax/>
                            <Route path=path!("/hr") view=pages::hr::Hr/>
                            <Route path=path!("/payroll-runs/:id") view=pages::hr::PayrollPage/>
                            <Route path=path!("/policy") view=pages::policy::Policy/>
                            <Route path=path!("/reports") view=pages::reports::Reports/>
                            <Route path=path!("/admin") view=pages::admin::Admin/>
                        </Routes>
                        </Show>
                    }
}

#[component]
fn Sidebar() -> impl IntoView {
    let app = state::app();
    let loc = use_location();
    let pending = RwSignal::new(0i64);
    Effect::new(move |_| { let _ = app.tick.get(); if state::can("approvals.read") { leptos::task::spawn_local(async move { if let Ok(v) = api::get("/approvals?mine=true&per_page=1").await { pending.set(v["total"].as_i64().unwrap_or(0)); } }); } });
    view! {
        <aside class=move || if app.nav_open.get() { "side open" } else { "side" }>
            <div class="brand"><div class="logo">"W"</div><div><b>"WEFT"</b><small>{move || if state::bn_now() { "আমদানি ইআরপি" } else { "Import ERP" }}</small></div></div>
            <div class="company-pill"><span class="dot"></span><span style="overflow:hidden;text-overflow:ellipsis;white-space:nowrap">{move || app.session.get().map(|s| if state::bn_now() && !s.company.name_bn.is_empty() { s.company.name_bn } else { s.company.name }).unwrap_or_default()}</span></div>
            <nav class="nav" on:click=move |_| app.nav_open.set(false)>
                {NAV.iter().map(|g| { let items: Vec<&'static NavItem> = g.items.iter().collect(); view! {
                    {move || { let vis: Vec<&NavItem> = items.iter().copied().filter(|i| allowed(i.perm)).collect(); if vis.is_empty() { ().into_any() } else { view! {
                        <h6>{t(g.title)}</h6>
                        {vis.into_iter().map(|i| { let href = i.href; let active = move || { let p = loc.pathname.get(); if href == "/" { p == "/" } else { p == href || p.starts_with(&format!("{href}/")) || related(href, &p) } };
                            view! { <A href=href attr:class=move || if active() { "active" } else { "" }><svg viewBox="0 0 24 24" width="19" height="19" inner_html=icon_paths(i.icon)></svg><span>{move || t(i.label)}</span>
                                {(href == "/policy").then(|| view! { {move || (pending.get() > 0).then(|| view! { <span class="count">{pending.get()}</span> })} })}</A> } }).collect_view()}
                    }.into_any() } }}
                } }).collect_view()}
            </nav>
            <div class="side-foot"><div class="avatar">{move || state::me().initials()}</div><div style="min-width:0;flex:1"><b>{move || state::me().full_name}</b><small>{move || fmt::nice(&state::me().role)}</small></div>
                <button class="icon-btn" style="background:transparent;border-color:rgba(255,255,255,.12);color:#B8C6DA" title=t("Sign out") on:click=move |_| { let rt = api::refresh_token(); leptos::task::spawn_local(async move { if let Some(r) = rt { let _ = api::post("/auth/logout", json!({"refresh_token": r})).await; } state::session_expired(); }); }><Icon name="out"/></button></div>
        </aside>
    }
}
fn related(module: &str, path: &str) -> bool {
    let groups: &[(&str, &[&str])] = &[("/imports", &["/shipments", "/lcs", "/foreign-payments"]), ("/customs", &["/bills-of-entry"]), ("/procurement", &["/purchase-orders", "/requisitions", "/goods-receipts", "/supplier-bills"]),
        ("/inventory", &["/stock-transfers", "/stock-adjustments"]), ("/sales", &["/sales-orders", "/deliveries", "/sales-invoices", "/customer-receipts", "/sales-returns"]), ("/ledger", &["/journals"]), ("/accounting", &["/supplier-payments", "/bank-transfers"]), ("/hr", &["/payroll-runs"])];
    groups.iter().any(|(m, ps)| *m == module && ps.iter().any(|p| path.starts_with(p)))
}

#[component]
fn Topbar() -> impl IntoView {
    let app = state::app();
    let notes_open = RwSignal::new(false);
    let notes: RwSignal<Vec<Value>> = RwSignal::new(vec![]);
    let load = move || leptos::task::spawn_local(async move {
        if let Ok(v) = api::get("/notifications/unread-count").await { app.unread.set(v["data"]["unread"].as_i64().unwrap_or(0)); }
    });
    Effect::new(move |_| { let _ = app.tick.get(); load(); });
    leptos::task::spawn_local(async move { loop { gloo_timers::future::TimeoutFuture::new(60_000).await; if app.session.get_untracked().is_none() { break; } load(); } });
    let nav = StoredValue::new_local(use_navigate());
    let open_notes = move |_| { notes_open.update(|o| *o = !*o); if notes_open.get_untracked() { leptos::task::spawn_local(async move { if let Ok(v) = api::get("/notifications?per_page=12").await { notes.set(v["data"].as_array().cloned().unwrap_or_default()); } }); } };
    let kb = window_event_listener(leptos::ev::keydown, move |e| { if (e.ctrl_key() || e.meta_key()) && e.key().to_lowercase() == "k" { e.prevent_default(); app.search_open.set(true); } });
    on_cleanup(move || kb.remove());
    view! {
        <header class="top no-print">
            <button class="icon-btn menu-btn" on:click=move |_| app.nav_open.set(true) aria-label="Menu"><Icon name="menu"/></button>
            <button class="search-btn" on:click=move |_| app.search_open.set(true)><Icon name="search" size=17/><span>{move || t("Search everything…")}</span><kbd>"Ctrl K"</kbd></button>
            <div class="sp"></div>
            <button class="lang-btn" title="Language" on:click=move |_| state::set_bn(!app.bn.get_untracked())>{move || if state::bn_now() { "EN" } else { "বাংলা" }}</button>
            <button class="icon-btn" title="Theme" on:click=move |_| state::set_dark(!app.dark.get_untracked())>{move || if app.dark.get() { view! { <Icon name="sun"/> }.into_any() } else { view! { <Icon name="moon"/> }.into_any() }}</button>
            <div style="position:relative">
                <button class="icon-btn" title=t("Notifications") on:click=open_notes><Icon name="bell"/>{move || (app.unread.get() > 0).then(|| view! { <span class="badge-dot">{app.unread.get().min(99)}</span> })}</button>
                {move || notes_open.get().then(|| view! { <div class="pop">
                    <div class="row" style="padding:14px 16px;border-bottom:1px solid var(--line)"><b>{t("Notifications")}</b><button class="btn ghost sm sp" on:click=move |_| leptos::task::spawn_local(async move { let _ = api::post("/notifications/read-all", json!({})).await; app.unread.set(0); notes_open.set(false); })>{t("Mark all as read")}</button></div>
                    <div style="max-height:60vh;overflow:auto">{move || { let l = notes.get(); if l.is_empty() { view! { <Empty title="You are all caught up"/> }.into_any() } else { l.into_iter().map(|n| { let unread = n["read_at"].is_null(); let id = fmt::text(&n["id"]); let route = entity_route(&n);
                        view! { <div class=if unread { "it unread" } else { "it" } on:click=move |_| { let id = id.clone(); let route = route.clone(); notes_open.set(false); leptos::task::spawn_local(async move { let _ = api::post(&format!("/notifications/{id}/read"), json!({})).await; state::refresh(); if let Some(r) = route { nav.with_value(|n| n(&r, Default::default())); } }); }>
                            <Badge status=n["severity"].as_str().unwrap_or("info").to_string()/><div><b style="font-size:13.5px">{if state::bn_now() { n["title_bn"].as_str().filter(|s| !s.is_empty()).unwrap_or(n["title"].as_str().unwrap_or("")).to_string() } else { n["title"].as_str().unwrap_or("").to_string() }}</b><div class="small mut">{fmt::text(&n["body"])}</div><small class="dim">{fmt::datetime(&n["created_at"])}</small></div></div> } }).collect_view().into_any() } }}</div>
                </div> })}
            </div>
        </header>
    }
}
fn entity_route(n: &Value) -> Option<String> {
    let id = n["entity_id"].as_str()?;
    let base = match n["entity"].as_str()? { "shipments" => "/shipments/", "letters_of_credit" => "/lcs/", "purchase_order" => "/purchase-orders/", "sales_order" => "/sales-orders/", "supplier_payment" | "foreign_payment" | "payroll_run" | "stock_adjustment" => return Some("/policy".into()), "supplier_bills" => "/supplier-bills/", "products" => return Some("/inventory".into()), "customers" => return Some("/sales".into()), _ => return None };
    Some(format!("{base}{id}"))
}

#[component]
fn SearchPalette() -> impl IntoView {
    let app = state::app();
    let q = RwSignal::new(String::new());
    let results: RwSignal<Vec<Value>> = RwSignal::new(vec![]);
    let sel = RwSignal::new(0usize);
    let seq = RwSignal::new(0u32);
    let nav = use_navigate();
    let go = { let nav = nav.clone(); move |r: Value| { app.search_open.set(false); q.set(String::new()); results.set(vec![]); nav(&route_for(&r), Default::default()); } };
    let on_input = move |e: leptos::ev::Event| {
        let v = event_target_value(&e); q.set(v.clone()); sel.set(0);
        let n = seq.get_untracked() + 1; seq.set(n);
        if v.trim().chars().count() < 2 { results.set(vec![]); return; }
        leptos::task::spawn_local(async move { gloo_timers::future::TimeoutFuture::new(200).await; if seq.get_untracked() != n { return; }
            if let Ok(r) = api::get(&format!("/search{}", api::qs(&[("q", v)]))).await { if seq.get_untracked() == n { results.set(r["data"].as_array().cloned().unwrap_or_default()); } } });
    };
    let go2 = go.clone();
    let on_key = move |e: leptos::ev::KeyboardEvent| match e.key().as_str() {
        "Escape" => app.search_open.set(false),
        "ArrowDown" => { e.prevent_default(); sel.update(|s| *s = (*s + 1).min(results.get_untracked().len().saturating_sub(1))); }
        "ArrowUp" => { e.prevent_default(); sel.update(|s| *s = s.saturating_sub(1)); }
        "Enter" => { if let Some(r) = results.get_untracked().get(sel.get_untracked()).cloned() { go2(r); } }
        _ => {}
    };
    let input_ref = NodeRef::<leptos::html::Input>::new();
    Effect::new(move |_| { if let Some(el) = input_ref.get() { let _ = el.focus(); } });
    let on_input = StoredValue::new_local(on_input);
    let on_key = StoredValue::new_local(on_key);
    let go = StoredValue::new_local(go);
    let list_view = move || {
        let l = results.get();
        if l.is_empty() {
            let msg = if q.get().trim().chars().count() < 2 { "Type a name, number, SKU, container or invoice" } else { "No matches" };
            view! { <div class="empty" style="padding:28px">{msg}</div> }.into_any()
        } else {
            l.into_iter().enumerate().map(|(i, r)| {
                let r2 = r.clone();
                view! { <div class=move || if sel.get() == i { "it on" } else { "it" } on:click=move |_| go.with_value(|g| g(r2.clone()))>
                    <span class="t">{fmt::nice(r["type"].as_str().unwrap_or(""))}</span>
                    <div><b>{fmt::text(&r["title"])}</b><div class="small dim">{fmt::text(&r["subtitle"])}</div></div></div> }
            }).collect_view().into_any()
        }
    };
    view! {
        {move || app.search_open.get().then(|| view! {
            <div class="overlay" on:mousedown=move |_| app.search_open.set(false)>
                <div class="search-pal" on:mousedown=|e| e.stop_propagation()>
                    <input node_ref=input_ref placeholder=t("Search everything…") prop:value=move || q.get() on:input=move |e| on_input.with_value(|f| f(e)) on:keydown=move |e| on_key.with_value(|f| f(e))/>
                    <div class="res">{list_view}</div>
                </div>
            </div>
        })}
    }
}
fn route_for(r: &Value) -> String {
    let id = r["id"].as_str().unwrap_or("");
    match r["type"].as_str().unwrap_or("") {
        "product" => "/inventory".into(), "customer" => "/sales".into(), "supplier" => "/procurement".into(), "employee" => "/hr".into(),
        "sales_order" => format!("/sales-orders/{id}"), "invoice" => format!("/sales-invoices/{id}"), "purchase_order" => format!("/purchase-orders/{id}"),
        "shipment" => format!("/shipments/{id}"), "lc" => format!("/lcs/{id}"), _ => "/".into(),
    }
}
