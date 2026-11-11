//! Replenishment: stock cover against supplier lead time, with a container-fill check and one-click requisition.
use crate::{api, fmt, i18n::t, state, ui::*};
use leptos::prelude::*;
use leptos_router::hooks::use_navigate;
use serde_json::{json, Value};
use std::collections::HashSet;

#[component]
pub fn Replenishment() -> impl IntoView {
    let tick = state::app().tick;
    let res = LocalResource::new(move || { let _ = tick.get(); async { api::get("/reports/replenishment").await } });
    let filter = RwSignal::new("all");
    let picked: RwSignal<HashSet<String>> = RwSignal::new(HashSet::new());
    let nav = use_navigate();
    view! { <div class="page">
        <PageHead title=t("Replenishment") subtitle="When to reorder so stock never runs out before the next ship arrives. Demand is the last 90 days of sales; lead time comes from each product."/>
        {move || match res.get() { None => view! { <Loading/> }.into_any(), Some(Err(e)) => view! { <ErrorBox err=e/> }.into_any(),
            Some(Ok(v)) => { let all = v["data"].as_array().cloned().unwrap_or_default(); let safety = fmt::num(&v["safety_days"]); let nav = nav.clone();
                let count = |s: &str| all.iter().filter(|r| r["status"] == s).count();
                let (c_over, c_now, c_ok) = (count("overdue"), count("order_now"), count("ok"));
                let all2 = all.clone();
                let rows = move || all2.iter().filter(|r| match filter.get() { "all" => true, f => r["status"] == f }).cloned().collect::<Vec<_>>();
                let all3 = all.clone();
                let selected_cbm = Memo::new(move |_| all3.iter().filter(|r| picked.get().contains(r["product_id"].as_str().unwrap_or(""))).map(|r| fmt::num(&r["suggested_cbm"])).sum::<f64>().max(0.0));
                let all4 = all.clone();
                view! {
                    <div class="grid g4"><Kpi label="Overdue" value=fmt::digits(c_over.to_string()) sub="should have been ordered already" tone={if c_over > 0 { "bad" } else { "" }} icon="alert"/><Kpi label="Order now" value=fmt::digits(c_now.to_string()) sub="within the next two weeks" tone={if c_now > 0 { "warn" } else { "" }} icon="cart"/><Kpi label="Comfortable" value=fmt::digits(c_ok.to_string()) sub="no action needed" icon="check"/><Kpi label="Safety stock" value=fmt::digits(format!("{} days", safety as i64)) sub="added to every lead time" icon="shield" tip="Change it in Policy & Approvals"/></div>
                    <div class="card flush" style="margin-top:16px">
                        <div class="toolbar">{[("all", "All"), ("overdue", "Overdue"), ("order_now", "Order now"), ("ok", "Comfortable"), ("no_demand", "No sales yet")].into_iter().map(|(k, l)| view! { <button class=move || if filter.get() == k { "chip on" } else { "chip" } on:click=move |_| filter.set(k)>{l}</button> }).collect_view()}</div>
                        <div class="tbl-wrap"><table><thead><tr><th></th><th>"Product"</th><th class="r">"On hand"</th><th class="r">"In transit"</th><th class="r">"On order"</th><th class="r">"Sells / day"</th><th style="min-width:160px">"Cover vs lead time"</th><th>"Order by"</th><th class="r">"Suggested qty"</th></tr></thead>
                        <tbody>{move || rows().into_iter().map(|r| { let pid = fmt::text(&r["product_id"]); let pid2 = pid.clone(); let cover = fmt::num(&r["days_of_cover"]); let lead = fmt::num(&r["lead_time_days"]); let mx = (cover.max(lead + safety) * 1.1).max(1.0); let status = fmt::text(&r["status"]);
                            let sug = fmt::num(&r["suggested_qty"]); let sel = move || picked.get().contains(&pid);
                            view! { <tr><td><input type="checkbox" disabled=sug <= 0.0 prop:checked=sel on:change=move |e| { let on = event_target_checked(&e); picked.update(|p| { if on { p.insert(pid2.clone()); } else { p.remove(&pid2); } }); }/></td>
                                <td><b>{fmt::text(&r["sku"])}</b><div class="small mut">{fmt::text(&r["name"])}</div></td><td class="r num">{fmt::qty(&r["on_hand"])}</td><td class="r num">{fmt::qty(&r["in_transit"])}</td><td class="r num">{fmt::qty(&r["on_order"])}</td><td class="r num">{fmt::qty(&r["daily_demand"])}</td>
                                <td>{if r["days_of_cover"].is_null() { view! { <span class="dim small">"No sales yet"</span> }.into_any() } else { view! { <div style="position:relative"><div class=match status.as_str() { "overdue" => "bar bad", "order_now" => "bar warn", _ => "bar" }><i style=format!("width:{:.0}%", cover / mx * 100.0)></i></div><div style=format!("position:absolute;left:{:.0}%;top:-3px;width:2px;height:14px;background:var(--tx)", lead / mx * 100.0) title="Supplier lead time"></div><div class="small dim num" style="margin-top:4px">{fmt::digits(format!("{:.0} d cover · lead {:.0} d", cover, lead))}</div></div> }.into_any() }}</td>
                                <td>{match r["order_by_in_days"].as_f64() { Some(d) if d < 0.0 => view! { <Badge status="overdue" label=fmt::digits(format!("{:.0} days ago", -d))/> }.into_any(), Some(d) => view! { <Badge status=status.clone() label=fmt::digits(format!("in {:.0} days", d))/> }.into_any(), None => view! { <span class="dim">"—"</span> }.into_any() }}</td>
                                <td class="r num">{if sug > 0.0 { fmt::qty(&r["suggested_qty"]) } else { "—".into() }}</td></tr> } }).collect_view()}</tbody></table></div>
                        <div class="pager no-print" style="flex-wrap:wrap"><Icon name="box" size=17/><span>{move || fmt::digits(format!("Selected: {:.1} m³ of a 67 m³ container", selected_cbm.get()))}</span><div class="bar" style="width:160px"><i style=move || format!("width:{:.0}%", (selected_cbm.get() / 67.0 * 100.0).min(100.0))></i></div>
                            <button class="btn primary sp" disabled=move || picked.get().is_empty() on:click=move |_| {
                                let lines: Vec<Value> = all4.iter().filter(|r| picked.get_untracked().contains(r["product_id"].as_str().unwrap_or(""))).map(|r| json!({"product_id": r["product_id"], "qty": fmt::num(&r["suggested_qty"]).to_string()})).collect();
                                let nav = nav.clone();
                                leptos::task::spawn_local(async move { match api::post("/requisitions", json!({"notes": "Created from Replenishment", "lines": lines})).await { Ok(_) => { state::ok("Requisition created"); nav("/procurement", Default::default()); } Err(e) => state::fail(e.friendly()) } });
                            }>"Create requisition"</button></div>
                    </div> }.into_any() } }}
    </div> }
}
