//! Role-aware home: the server only returns the blocks this person may see, and we show exactly those.
use crate::{api, fmt, i18n::t, state, ui::*};
use leptos::prelude::*;
use serde_json::Value;

fn greeting() -> String {
    let h = js_sys::Date::new_0().get_hours();
    t(if h < 12 { "Good morning" } else if h < 17 { "Good afternoon" } else { "Good evening" })
}

struct Quick { label: &'static str, href: &'static str, perm: &'static str, icon: &'static str }
const QUICK: &[Quick] = &[
    Quick { label: "New purchase order", href: "/purchase-orders/new", perm: "purchase.write", icon: "cart" },
    Quick { label: "Book a shipment", href: "/shipments/new", perm: "import.write", icon: "ship" },
    Quick { label: "New sales order", href: "/sales-orders/new", perm: "sales.write", icon: "users" },
    Quick { label: "Record a receipt", href: "/customer-receipts/new", perm: "sales.invoice", icon: "wallet" },
    Quick { label: "Transfer stock", href: "/stock-transfers/new", perm: "inventory.write", icon: "box" },
    Quick { label: "Post a journal", href: "/journals/new", perm: "finance.post", icon: "book" },
    Quick { label: "Mark attendance", href: "/hr", perm: "hr.write", icon: "id" },
];

#[component]
pub fn Dashboard() -> impl IntoView {
    let tick = state::app().tick;
    let res = LocalResource::new(move || { let _ = tick.get(); async { api::get("/dashboard").await } });
    let notes = LocalResource::new(move || { let _ = tick.get(); async { api::get("/notifications?unread=true&per_page=6").await } });
    view! {
        <div class="page">
            {move || match res.get() {
                None => view! { <Loading/> }.into_any(),
                Some(Err(e)) => view! { <ErrorBox err=e/> }.into_any(),
                Some(Ok(v)) => { let d = v["data"].clone(); view! { <DashBody d=d notes=notes/> }.into_any() }
            }}
        </div>
    }
}

#[component]
fn DashBody(d: Value, notes: LocalResource<api::ApiResult>) -> impl IntoView {
    let f = d["finance"].clone(); let m = d["month"].clone(); let s = d["sales"].clone(); let i = d["import"].clone(); let inv = d["inventory"].clone(); let ap = d["approvals"].clone(); let tx = d["tax"].clone(); let hr = d["hr"].clone();
    let first = d["greeting_name"].as_str().unwrap_or("").split_whitespace().next().unwrap_or("").to_string();
    let mut tiles: Vec<AnyView> = vec![];
    if f.is_object() {
        tiles.push(view! { <Kpi icon="wallet" label=t("Cash & bank") value=fmt::money_short(fmt::num(&f["cash_and_bank"])) sub=format!("{} {}", f["accounts"].as_array().map(|a| a.len()).unwrap_or(0), "accounts") href="/cash" tip="Money in all bank, cash and mobile accounts"/> }.into_any());
        tiles.push(view! { <Kpi icon="users" label=t("Receivables") value=fmt::money_short(fmt::num(&f["receivables"])) sub=format!("{} overdue", fmt::money_short(fmt::num(&f["receivables_overdue"]))) tone={if fmt::num(&f["receivables_overdue"]) > 0.0 { "warn" } else { "" }} href="/sales" tip="Customers still owe you this much"/> }.into_any());
        tiles.push(view! { <Kpi icon="cart" label=t("Payables") value=fmt::money_short(fmt::num(&f["payables_local"]) + fmt::num(&f["payables_foreign_bdt"])) sub=format!("Foreign {}", fmt::money_short(fmt::num(&f["payables_foreign_bdt"]))) href="/accounting" tip="What you owe suppliers (local and foreign)"/> }.into_any());
    }
    if m.is_object() {
        tiles.push(view! { <Kpi icon="chart" label=t("Sales this month") value=fmt::money_short(fmt::num(&m["revenue"])) sub=format!("{} {}", t("Gross margin"), if m["gross_margin_pct"].is_null() { "—".to_string() } else { fmt::pct(&m["gross_margin_pct"]) }) href="/reports" tip="Revenue before VAT, this month so far"/> }.into_any());
    }
    if i.is_object() {
        tiles.push(view! { <Kpi icon="ship" label=t("Goods in transit") value=fmt::money_short(fmt::num(&i["goods_in_transit_bdt"])) sub=format!("{} LC expiring in 30 days", i["lcs_expiring_30d"]) tone={if i["lcs_expiring_30d"].as_i64().unwrap_or(0) > 0 { "warn" } else { "" }} href="/imports" tip="Value of imports shipped but not yet received into stock"/> }.into_any());
    }
    if inv.is_object() {
        tiles.push(view! { <Kpi icon="box" label=t("Stock value") value=fmt::money_short(fmt::num(&inv["stock_value"])) sub=format!("{} {}", inv["low_stock_items"], t("Low stock items").to_lowercase()) tone={if inv["low_stock_items"].as_i64().unwrap_or(0) > 0 { "warn" } else { "" }} href="/inventory"/> }.into_any());
    }
    if ap.is_object() { tiles.push(view! { <Kpi icon="shield" label=t("Waiting for approval") value=fmt::digits(fmt::text(&ap["waiting_for_me"])) sub="decisions for you" tone={if ap["waiting_for_me"].as_i64().unwrap_or(0) > 0 { "warn" } else { "" }} href="/policy"/> }.into_any()); }
    if tx.is_object() { tiles.push(view! { <Kpi icon="tag" label="VAT payable" value=fmt::money_short(fmt::num(&tx["vat_payable_to_treasury"])) sub="to the treasury" href="/tax"/> }.into_any()); }
    if hr.is_object() { tiles.push(view! { <Kpi icon="id" label="People" value=fmt::digits(fmt::text(&hr["headcount"])) sub=format!("{} absent today · {} leave requests", hr["absent_today"], hr["pending_leave"]) href="/hr"/> }.into_any()); }
    let trend: Vec<(String, f64)> = s["trend_30d"].as_array().map(|a| a.iter().map(|x| (x["date"].as_str().unwrap_or("")[8..].to_string(), fmt::num(&x["sales"]))).collect()).unwrap_or_default();
    let top: Vec<Value> = s["top_customers"].as_array().cloned().unwrap_or_default();
    let soon: Vec<Value> = i["arriving_soon"].as_array().cloned().unwrap_or_default();
    let accounts: Vec<Value> = f["accounts"].as_array().cloned().unwrap_or_default();
    let max_top = top.iter().map(|c| fmt::num(&c["sales"])).fold(1.0_f64, f64::max);
    view! {
        <PageHead title=format!("{}, {}", greeting(), first) subtitle="Here is what is happening in your business today."/>
        <div class="row wrap no-print" style="margin:-6px 0 20px">{QUICK.iter().filter(|q| state::can(q.perm)).map(|q| view! { <a class="btn" href=q.href><Icon name=q.icon size=16/>{q.label}</a> }).collect_view()}</div>
        <div class="grid g4">{tiles}</div>
        <div class="grid g-main" style="margin-top:16px">
            <div class="grid">
                {s.is_object().then(|| view! { <Card title=t("Sales this month") right=ViewFn::from(|| view! { <span class="mut small">"last 30 days"</span> })><BarChart data=trend.clone()/>
                    <div class="row wrap small mut" style="margin-top:8px"><span>{format!("{} open orders", s["open_orders"])}</span><span>"·"</span><span>{format!("{} deliveries waiting to be invoiced", s["deliveries_to_invoice"])}</span></div></Card> })}
                {i.is_object().then(|| view! { <Card title=t("Arriving soon") right=ViewFn::from(|| view! { <a href="/imports">"All shipments →"</a> })>
                    {if soon.is_empty() { view! { <Empty title="No shipments on the way"/> }.into_any() } else { soon.iter().cloned().map(|x| { let id = fmt::text(&x["id"]); view! { <a href=format!("/shipments/{id}") class="row" style="padding:12px 0;border-bottom:1px solid var(--line);color:inherit;text-decoration:none"><Icon name="ship"/><div><b>{fmt::text(&x["shipment_no"])}</b><div class="small mut">{fmt::text(&x["supplier"])}</div></div><span class="sp small">{format!("ETA {}", fmt::date(&x["eta"]))}</span><Badge status=fmt::text(&x["status"])/></a> } }).collect_view().into_any() }}
                    {(fmt::num(&i["demurrage_exposure_bdt"]) > 0.0).then(|| view! { <div class="alert warn" style="margin-top:12px"><Icon name="alert"/><div>{format!("Demurrage running: {}", fmt::money_f(fmt::num(&i["demurrage_exposure_bdt"])))}</div></div> })}</Card> })}
            </div>
            <div class="grid" style="align-content:start">
                <Card title=t("Needs your attention") right=ViewFn::from(|| view! { <span class="mut small">{move || fmt::digits(state::app().unread.get().to_string())}</span> })>
                    {move || match notes.get() { None => view! { <div class="skel"></div> }.into_any(), Some(Err(_)) => view! { <span class="mut">"—"</span> }.into_any(), Some(Ok(v)) => { let l = v["data"].as_array().cloned().unwrap_or_default();
                        if l.is_empty() { view! { <div class="empty" style="padding:22px"><b>"All clear"</b>"Nothing needs you right now."</div> }.into_any() } else { l.into_iter().map(|n| view! { <div class="row" style="padding:10px 0;border-bottom:1px solid var(--line);align-items:flex-start"><Badge status=fmt::text(&n["severity"])/><div><b style="font-size:13.5px">{if state::is_bn() { n["title_bn"].as_str().filter(|x| !x.is_empty()).unwrap_or(n["title"].as_str().unwrap_or("")).to_string() } else { fmt::text(&n["title"]) }}</b><div class="small mut">{fmt::text(&n["body"])}</div></div></div> }).collect_view().into_any() } } }}
                </Card>
                {(!top.is_empty()).then(|| view! { <Card title="Top customers">{top.iter().map(|c| { let w = fmt::num(&c["sales"]) / max_top * 100.0; view! { <div style="margin-bottom:12px"><div class="row"><span>{fmt::text(&c["name"])}</span><b class="sp num">{fmt::money_short(fmt::num(&c["sales"]))}</b></div><div class="bar" style="margin-top:5px"><i style=format!("width:{w:.0}%")></i></div></div> } }).collect_view()}</Card> })}
                {(!accounts.is_empty()).then(|| view! { <Card title="Bank & cash">{accounts.iter().map(|a| view! { <div class="row" style="padding:8px 0;border-bottom:1px solid var(--line)"><div><b>{fmt::text(&a["name"])}</b><div class="small dim">{fmt::nice(a["kind"].as_str().unwrap_or(""))}</div></div><b class="sp num">{fmt::money(&a["balance"])}</b></div> }).collect_view()}</Card> })}
            </div>
        </div>
    }
}
