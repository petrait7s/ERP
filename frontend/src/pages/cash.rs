//! Cash & FX: balances, open foreign-currency exposure, exchange rates and a what-if on currency moves.
use super::common::*;
use crate::{api, crud::*, fmt, i18n::t, state, ui::*};
use leptos::prelude::*;
use serde_json::{json, Value};

const RATE_FIELDS: &[FF] = &[ff("currency", "Currency", FK::Select(super::procurement::CURRENCIES)).req().def("USD"), ff("rate_date", "Date", FK::Date).req().def("today"), ff("rate", "Taka per 1 unit", FK::Dec).req(), ff("rate_type", "Rate type", FK::Select(&[("mid", "Mid rate"), ("bc_selling", "Bank selling (BC)"), ("tt_buying", "TT buying"), ("customs", "Customs rate")])).def("mid"), ff("source", "Source", FK::Text).help("e.g. Bangladesh Bank, your bank")];
static RATES: ListCfg = ListCfg { title: "Exchange Rates", subtitle: "", endpoint: "/exchange-rates", cols: &[col("rate_date", "Date", CK::Date), col("currency", "Currency", CK::Bold), col("rate", "Taka per unit", CK::Qty), col("rate_type", "Type", CK::Badge), col("source", "Source", CK::Text)],
    search: false, filters: &[Filter { param: "currency", label: "All currencies", options: super::procurement::CURRENCIES }], fields: RATE_FIELDS, write: "finance.rates", new_label: "Add rate", edit: true, delete: true, detail: "", new_route: "", extra_query: "", empty_hint: "Add today's rates so foreign-currency documents can be valued.", date_filter: false };

#[component]
pub fn Cash() -> impl IntoView {
    let tab = RwSignal::new("overview");
    view! { <div class="page"><PageHead title=t("Cash & FX") subtitle="Where your money is, what you owe in foreign currency, and what a rate move would cost."/>
        <Tabs tabs=vec![("overview", t("Overview")), ("rates", t("Exchange Rates")), ("whatif", "What-if".to_string())] active=tab/>
        {move || match tab.get() { "overview" => view! { <Overview/> }.into_any(), "rates" => view! { <ListPage cfg=&RATES headless=true/> }.into_any(), _ => view! { <WhatIf/> }.into_any() }}</div> }
}

#[component]
fn Overview() -> impl IntoView {
    let tick = state::app().tick;
    let ex = LocalResource::new(move || { let _ = tick.get(); async { api::get("/fx/exposure").await } });
    let banks = LocalResource::new(move || { let _ = tick.get(); async { api::get("/bank-accounts?per_page=100").await } });
    let busy = RwSignal::new(false);
    let revalue = move |_| { busy.set(true); leptos::task::spawn_local(async move { match api::post("/fx/revalue", json!({})).await { Ok(v) => { state::ok(if v["data"]["posted"] == true { format!("Revalued: net {}", fmt::money(&v["data"]["net_loss"])) } else { "Nothing to revalue".into() }); state::refresh(); } Err(e) => state::fail(e.friendly()) } busy.set(false); }); };
    view! { <div class="grid g-main" style="align-items:start">
        <Card title="Bank & cash balances">{move || match banks.get() { None => view! { <div class="skel"></div> }.into_any(), Some(Err(e)) => view! { <ErrorBox err=e/> }.into_any(), Some(Ok(v)) => { let l = v["data"].as_array().cloned().unwrap_or_default(); let total: f64 = l.iter().map(|b| fmt::num(&b["balance_bdt"])).sum();
            view! { <div class="kpi" style="padding:0;border:0;box-shadow:none"><div class="lbl">"Total in Taka"</div><div class="val num">{fmt::money_f(total)}</div></div>
                {l.into_iter().map(|b| view! { <div class="row" style="padding:11px 0;border-top:1px solid var(--line)"><div><b>{fmt::text(&b["name"])}</b><div class="small dim">{fmt::nice(b["kind"].as_str().unwrap_or(""))}" · "{fmt::text(&b["currency"])}</div></div><div class="sp right"><b class="num">{fmt::money(&b["balance_bdt"])}</b>{(!b["balance_fc"].is_null()).then(|| view! { <div class="small dim num">{fmt::fc(&b["balance_fc"], b["currency"].as_str().unwrap_or(""))}</div> })}</div></div> }).collect_view()} }.into_any() } }}</Card>
        <Card title="Foreign-currency exposure" right=ViewFn::from(move || view! { {move || state::can("finance.post").then(|| view! { <button class="btn sm" disabled=move || busy.get() on:click=revalue>"Revalue now"</button> })} })>
            {move || match ex.get() { None => view! { <div class="skel"></div> }.into_any(), Some(Err(e)) => view! { <ErrorBox err=e/> }.into_any(), Some(Ok(v)) => { let d = v["data"].clone(); let cur = d["by_currency"].as_array().cloned().unwrap_or_default();
                if cur.is_empty() { return view! { <Empty title="No foreign-currency exposure" hint="Amounts you owe suppliers abroad appear here."/> }.into_any(); }
                view! { <p class="mut small" style="margin-bottom:10px">"What you will have to pay in foreign currency: shipped invoices not yet paid, plus orders not yet shipped, less advances."</p>
                    {cur.into_iter().map(|c| { let code = fmt::text(&c["currency"]); view! { <div style="padding:12px 0;border-top:1px solid var(--line)"><div class="row"><b>{code.clone()}</b><span class="sp num"><b>{fmt::fc(&c["net_exposure_fc"], &code)}</b></span></div>
                        <div class="row small mut"><span>{format!("≈ {} at {}", fmt::money(&c["net_exposure_bdt"]), fmt::qty(&c["rate"]))}</span><span class="sp">"1% move ="</span><b class="num">{fmt::money(&c["impact_of_1pct_bdt"])}</b></div></div> } }).collect_view()}
                    <div class="alert info" style="margin-top:12px"><Icon name="swap"/><div>"Total exposure "<b>{fmt::money(&d["total_exposure_bdt"])}</b></div></div> }.into_any() } }}</Card>
    </div> }
}

/// A currency what-if over the real exposure numbers.
#[component]
fn WhatIf() -> impl IntoView {
    let tick = state::app().tick;
    let ex = LocalResource::new(move || { let _ = tick.get(); async { api::get("/fx/exposure").await } });
    let shock = RwSignal::new(5.0f64); let hedge = RwSignal::new(0.0f64);
    view! { <div class="grid g2" style="align-items:start">
        <Card title="Try a currency move"><p class="mut small" style="margin-bottom:16px">"Slide to see what a stronger foreign currency would cost on what you currently owe. This is a calculation on your live exposure; nothing is posted."</p>
            <div class="field"><label>{move || fmt::digits(format!("Foreign currency gets stronger by {:.1}%", shock.get()))}</label><input type="range" min="-10" max="15" step="0.5" prop:value=move || shock.get().to_string() on:input=move |e| shock.set(event_target_value(&e).parse().unwrap_or(0.0)) style="accent-color:var(--teal)"/></div>
            <div class="field" style="margin-top:16px"><label>{move || fmt::digits(format!("Share you hedge with a forward contract: {:.0}%", hedge.get()))}</label><input type="range" min="0" max="100" step="5" prop:value=move || hedge.get().to_string() on:input=move |e| hedge.set(event_target_value(&e).parse().unwrap_or(0.0)) style="accent-color:var(--teal)"/></div></Card>
        <Card title="Effect on your costs">{move || match ex.get() { None => view! { <div class="skel"></div> }.into_any(), Some(Err(e)) => view! { <ErrorBox err=e/> }.into_any(), Some(Ok(v)) => { let total = fmt::num(&v["data"]["total_exposure_bdt"]); let open = total * (1.0 - hedge.get() / 100.0); let cost = open * shock.get() / 100.0;
            view! { <div class="facts"><div class="fact"><small>"Total exposure"</small><div>{fmt::money_f(total)}</div></div><div class="fact"><small>"Unhedged"</small><div>{fmt::money_f(open)}</div></div></div>
                <div class="kpi" style="padding:18px 0 0;border:0;box-shadow:none"><div class="lbl">{move || if shock.get() >= 0.0 { "Extra cost" } else { "Saving" }}</div><div class={move || if cost > 0.0 { "val dn num" } else { "val up num" }}>{fmt::money_f(cost.abs())}</div></div>
                <p class="help">"Cost of paying the same foreign-currency amount after the move."</p> }.into_any() } }}</Card></div> }
}
