//! Sales & customers: orders, the order book, deliveries, invoices (Mushak 6.3), receipts and returns.
use super::common::*;
use crate::{api, crud::*, fmt, i18n::t, state, ui::*};
use leptos::prelude::*;
use leptos_router::hooks::{use_navigate, use_params_map};
use serde_json::{json, Value};

const SO_STATUS: &[(&str, &str)] = &[("draft", "Draft"), ("pending_approval", "Waiting for approval"), ("confirmed", "Confirmed"), ("partially_delivered", "Partly delivered"), ("delivered", "Delivered"), ("closed", "Closed"), ("cancelled", "Cancelled")];
static SO_LIST: ListCfg = ListCfg { title: "Sales Orders", subtitle: "", endpoint: "/sales-orders", cols: &[col("so_no", "Order", CK::Bold), col("customer_name", "Customer", CK::Text), col("order_date", "Date", CK::Date), col("delivery_date", "Delivery", CK::Date), col("total", "Total", CK::Money), col("delivered_pct", "Delivered", CK::Pct), col("invoiced_pct", "Invoiced", CK::Pct), col("status", "Status", CK::Badge)],
    search: true, filters: &[Filter { param: "status", label: "All statuses", options: SO_STATUS }], fields: &[], write: "sales.write", new_label: "New order", edit: false, delete: false, detail: "/sales-orders/", new_route: "/sales-orders/new", extra_query: "", empty_hint: "Take your first customer order.", date_filter: true };
static DL_LIST: ListCfg = ListCfg { title: "Deliveries", subtitle: "", endpoint: "/deliveries", cols: &[col("delivery_no", "Challan", CK::Bold), col("delivery_date", "Date", CK::Date), col("so_no", "Order", CK::Mono), col("customer_name", "Customer", CK::Text), col("vehicle_no", "Vehicle", CK::Text), col("cogs_value", "Cost of goods", CK::Money), col("invoiced", "Invoiced", CK::Bool)],
    search: false, filters: &[Filter { param: "uninvoiced", label: "All", options: &[("true", "Not yet invoiced")] }], fields: &[], write: "", new_label: "", edit: false, delete: false, detail: "/deliveries/", new_route: "", extra_query: "", empty_hint: "Deliver from a confirmed order.", date_filter: true };
static INV_LIST: ListCfg = ListCfg { title: "Invoices", subtitle: "", endpoint: "/sales-invoices", cols: &[col("invoice_no", "Invoice", CK::Bold), col("mushak_no", "Mushak 6.3", CK::Mono), col("customer_name", "Customer", CK::Text), col("invoice_date", "Date", CK::Date), col("due_date", "Due", CK::Date), col("total", "Total", CK::Money), col("balance", "Balance", CK::Money), col("status", "Status", CK::Badge)],
    search: true, filters: &[Filter { param: "status", label: "All", options: &[("open", "Open"), ("partial", "Part paid"), ("paid", "Paid")] }, Filter { param: "overdue", label: "Any due date", options: &[("true", "Overdue only")] }], fields: &[], write: "", new_label: "", edit: false, delete: false, detail: "/sales-invoices/", new_route: "", extra_query: "", empty_hint: "Invoices are issued from deliveries.", date_filter: true };
static RC_LIST: ListCfg = ListCfg { title: "Receipts", subtitle: "", endpoint: "/customer-receipts", cols: &[col("receipt_no", "Receipt", CK::Bold), col("receipt_date", "Date", CK::Date), col("customer_name", "Customer", CK::Text), col("bank_account", "Into", CK::Text), col("gross_amount", "Amount", CK::Money), col("vds_amount", "VDS withheld", CK::Money), col("tds_amount", "TDS withheld", CK::Money), col("unallocated", "Not yet applied", CK::Money)],
    search: false, filters: &[], fields: &[], write: "sales.invoice", new_label: "Record receipt", edit: false, delete: false, detail: "", new_route: "/customer-receipts/new", extra_query: "", empty_hint: "", date_filter: true };
static RT_LIST: ListCfg = ListCfg { title: "Returns", subtitle: "", endpoint: "/sales-returns", cols: &[col("return_no", "Credit note", CK::Bold), col("return_date", "Date", CK::Date), col("invoice_no", "Invoice", CK::Mono), col("customer_name", "Customer", CK::Text), col("reason", "Reason", CK::Text), col("total", "Total", CK::Money)],
    search: false, filters: &[], fields: &[], write: "sales.write", new_label: "New return", edit: false, delete: false, detail: "", new_route: "/sales-returns/new", extra_query: "", empty_hint: "", date_filter: true };
const CUS_FIELDS: &[FF] = &[ff("name", "Name", FK::Text).req().full(), ff("name_bn", "Name in Bangla", FK::Text).full(), ff("customer_type", "Type", FK::Select(&[("dealer", "Dealer"), ("retail", "Retail"), ("corporate", "Corporate"), ("government", "Government"), ("distributor", "Distributor")])).def("dealer"), ff("district", "District", FK::Text), ff("phone", "Phone", FK::Text), ff("email", "Email", FK::Email), ff("contact_person", "Contact person", FK::Text), ff("bin", "BIN", FK::Text), ff("tin", "TIN", FK::Text),
    ff("credit_limit", "Credit limit (Tk)", FK::Dec).def("0").help("0 means no limit"), ff("payment_terms_days", "Payment terms (days)", FK::Int).def("0"), ff("address", "Address", FK::Area).full(), ff("is_active", "Active", FK::Bool).def("true")];
static CUSTOMERS: ListCfg = ListCfg { title: "Customers", subtitle: "", endpoint: "/customers", cols: &[col("code", "Code", CK::Mono), col("name", "Name", CK::Bold), col("customer_type", "Type", CK::Badge), col("district", "District", CK::Text), col("phone", "Phone", CK::Text), col("credit_limit", "Credit limit", CK::Money), col("outstanding", "Owes you", CK::Money), col("is_active", "Active", CK::Bool)],
    search: true, filters: &[Filter { param: "customer_type", label: "All types", options: &[("dealer", "Dealer"), ("retail", "Retail"), ("corporate", "Corporate"), ("government", "Government"), ("distributor", "Distributor")] }], fields: CUS_FIELDS, write: "sales.write", new_label: "New customer", edit: true, delete: true, detail: "", new_route: "", extra_query: "", empty_hint: "Add your customers.", date_filter: false };

#[component]
pub fn Sales() -> impl IntoView {
    let tab = RwSignal::new("so");
    view! { <div class="page"><PageHead title=t("Sales & Customers") subtitle="From the customer's order to the money in your bank, with VAT and Mushak forms handled."/>
        <Tabs tabs=vec![("so", t("Sales Orders")), ("dl", t("Deliveries")), ("inv", t("Invoices")), ("rc", t("Receipts")), ("rt", t("Returns")), ("cus", t("Customers"))] active=tab/>
        {move || match tab.get() { "so" => view! { <ListPage cfg=&SO_LIST headless=true/> }.into_any(), "dl" => view! { <ListPage cfg=&DL_LIST headless=true/> }.into_any(), "inv" => view! { <ListPage cfg=&INV_LIST headless=true/> }.into_any(), "rc" => view! { <ListPage cfg=&RC_LIST headless=true/> }.into_any(), "rt" => view! { <ListPage cfg=&RT_LIST headless=true/> }.into_any(), _ => view! { <ListPage cfg=&CUSTOMERS headless=true/> }.into_any() }}</div> }
}

/// The order book: every open customer promise, grouped by when it is due.
#[component]
pub fn OrderBook() -> impl IntoView {
    let tick = state::app().tick;
    let res = LocalResource::new(move || { let _ = tick.get(); async { api::get("/sales-orders?per_page=300").await } });
    let nav = use_navigate();
    view! { <div class="page"><PageHead title=t("Order Book") subtitle="Everything you have promised customers, by delivery date. Cards in red are already past their date.">
        {move || state::can("sales.write").then(|| view! { <a class="btn primary" href="/sales-orders/new"><Icon name="plus" size=16/>"New order"</a> })}</PageHead>
        {move || match res.get() { None => view! { <Loading/> }.into_any(), Some(Err(e)) => view! { <ErrorBox err=e/> }.into_any(),
            Some(Ok(v)) => { let today = fmt::today(); let week = fmt::add_days(&today, 7); let two = fmt::add_days(&today, 14); let nav = nav.clone();
                let open: Vec<Value> = v["data"].as_array().cloned().unwrap_or_default().into_iter().filter(|o| matches!(o["status"].as_str(), Some("pending_approval") | Some("confirmed") | Some("partially_delivered") | Some("draft"))).collect();
                let bucket = |o: &Value| -> usize { match o["delivery_date"].as_str() { None => 4, Some(d) if d < today.as_str() => 0, Some(d) if d <= week.as_str() => 1, Some(d) if d <= two.as_str() => 2, _ => 3 } };
                let titles = ["Overdue", "This week", "Next week", "Later", "No date set"];
                let value: f64 = open.iter().map(|o| fmt::num(&o["total"])).sum();
                view! { <div class="grid g3" style="margin-bottom:16px"><Kpi label="Open orders" value=fmt::digits(open.len().to_string()) icon="board"/><Kpi label="Order book value" value=fmt::money_short(value) icon="wallet"/><Kpi label="Overdue" value=fmt::digits(open.iter().filter(|o| bucket(o) == 0).count().to_string()) tone=if open.iter().any(|o| bucket(o) == 0) { "bad" } else { "" } icon="alert"/></div>
                    <div class="kanban">{(0..5).map(|b| { let items: Vec<Value> = open.iter().filter(|o| bucket(o) == b).cloned().collect(); let n = items.len(); let nav = nav.clone();
                        view! { <div class="kcol"><h4>{titles[b]}<span class="badge">{n}</span></h4>{items.into_iter().map(|o| { let id = fmt::text(&o["id"]); let nav = nav.clone(); let del = fmt::num(&o["delivered_pct"]);
                            view! { <div class="kcard" style=if b == 0 { "border-color:var(--bad)" } else { "" } on:click=move |_| nav(&format!("/sales-orders/{id}"), Default::default())>
                                <div class="row"><b>{fmt::text(&o["so_no"])}</b><span class="sp"><Badge status=fmt::text(&o["status"])/></span></div><div class="small mut" style="margin:3px 0 10px">{fmt::text(&o["customer_name"])}</div>
                                <div class="row small"><span>{match o["delivery_date"].as_str() { Some(_) => format!("Due {}", fmt::date(&o["delivery_date"])), None => "No delivery date".into() }}</span><b class="sp num">{fmt::money_short(fmt::num(&o["total"]))}</b></div>
                                <div class="bar" style="margin-top:10px"><i style=format!("width:{del:.0}%")></i></div><div class="small dim" style="margin-top:4px">{fmt::digits(format!("{del:.0}% delivered"))}</div></div> } }).collect_view()}</div> } }).collect_view()}</div> }.into_any() } }}
    </div> }
}

static NEW_SO: NewDoc = NewDoc { title: "New sales order", subtitle: "Prices come from the product; change them per line if you agreed something different.", endpoint: "/sales-orders", back_label: "Sales & Customers", back_route: "/sales",
    header: &[ff("customer_id", "Customer", FK::Lookup("/customers?is_active=true", &["name", "district"])).req(), ff("warehouse_id", "Deliver from", FK::Lookup("/warehouses", &["name"])), ff("order_date", "Order date", FK::Date).def("today"), ff("delivery_date", "Promised delivery", FK::Date), ff("notes", "Notes", FK::Area).full()],
    lines_title: "Items", lines: &[ff("product_id", "Product", FK::Lookup("/products?is_active=true", &["sku", "name"])).req(), ff("qty", "Quantity", FK::Dec).req(), ff("unit_price", "Unit price (Tk)", FK::Dec), ff("discount_pct", "Discount %", FK::Dec)], autofill: Some(("sale_price", "unit_price")), goto: "/sales-orders/{rid}", done: "Order saved as a draft", submit_label: "Save draft", min_lines: 1 };
#[component] pub fn NewSo() -> impl IntoView { view! { <NewDocPage cfg=&NEW_SO/> } }

static SO_DOC: DocCfg = DocCfg { endpoint: "/sales-orders", list_route: "/sales", list_label: "Sales & Customers", title_key: "so_no", subtitle_key: "customer_name", status_key: "status", steps: &[("draft", "Draft"), ("confirmed", "Confirmed"), ("partially_delivered", "Delivering"), ("delivered", "Delivered")],
    facts: &[fact("Order date", "order_date", CK::Date), fact("Promised", "delivery_date", CK::Date), fact("Deliver from", "warehouse", CK::Text), fact("Salesperson", "salesperson", CK::Text), fact("Credit limit", "credit_limit", CK::Money), fact("Subtotal", "subtotal", CK::Money), fact("Discount", "discount", CK::Money), fact("SD", "sd_amount", CK::Money), fact("VAT", "vat_amount", CK::Money), fact("Total", "total", CK::Money)],
    tables: &[dtable("Items", "lines", &[col("sku", "SKU", CK::Mono), col("product_name", "Product", CK::Text), col("qty", "Qty", CK::Qty), col("unit_price", "Price", CK::Money), col("discount_pct", "Disc %", CK::Pct), col("vat_rate", "VAT %", CK::Pct), col("line_total", "Total", CK::Money), col("delivered_qty", "Delivered", CK::Qty), col("invoiced_qty", "Invoiced", CK::Qty)])],
    actions: &[
        action("Confirm order", "/submit", "sales.write").when(&["draft", "rejected"]).primary().confirm("Confirm this order? Credit limit and approval rules are checked.").done("Order confirmed"),
        action("Deliver goods", "", "inventory.write").when(&["confirmed", "partially_delivered"]).primary().abs("/deliveries", "so_id").fields(&[ff("delivery_date", "Delivery date", FK::Date).def("today"), ff("vehicle_no", "Vehicle number", FK::Text), ff("driver_name", "Driver", FK::Text), ff("invoice", "Invoice the customer now", FK::Bool).def("true").help("Issue the Mushak 6.3 invoice with the delivery")]).lines(LineQty { key: "lines", send: "so_line_id", show: "product_name", total: "qty", done: "delivered_qty", label: "Deliver now" }).done("Delivered").goto("/deliveries/{rid}"),
        action("Close order", "/close", "sales.write").when(&["partially_delivered", "delivered"]).confirm("Close this order?").done("Order closed"),
        action("Cancel order", "/cancel", "sales.write").when(&["draft", "pending_approval", "confirmed", "rejected"]).danger().confirm("Cancel this order?").done("Order cancelled"),
    ], notes_key: "notes" };
#[component] pub fn SoPage() -> impl IntoView { let id = use_params_map().with_untracked(|p| p.get("id").unwrap_or_default()); view! { <div class="page"><DocPage cfg=&SO_DOC id=id/></div> } }

static DL_DOC: DocCfg = DocCfg { endpoint: "/deliveries", list_route: "/sales", list_label: "Sales & Customers", title_key: "delivery_no", subtitle_key: "customer_name", status_key: "", steps: &[],
    facts: &[fact("Order", "so_no", CK::Mono), fact("From", "warehouse", CK::Text), fact("Date", "delivery_date", CK::Date), fact("Vehicle", "vehicle_no", CK::Text), fact("Driver", "driver_name", CK::Text), fact("Cost of goods", "cogs_value", CK::Money), fact("Invoice", "invoice_no", CK::Mono)],
    tables: &[dtable("Items delivered", "lines", &[col("sku", "SKU", CK::Mono), col("product_name", "Product", CK::Text), col("qty", "Qty", CK::Qty), col("cost_value", "Cost", CK::Money)])],
    actions: &[action("Create invoice", "", "sales.invoice").primary().hide_if("invoiced").abs("/sales-invoices", "delivery_id").fields(&[ff("invoice_date", "Invoice date", FK::Date).def("today"), ff("due_date", "Due date", FK::Date).help("Leave empty to use the customer's payment terms")]).done("Invoice issued").goto("/sales-invoices/{rid}")], notes_key: "notes" };
#[component] pub fn DeliveryPage() -> impl IntoView { let id = use_params_map().with_untracked(|p| p.get("id").unwrap_or_default()); view! { <div class="page"><DocPage cfg=&DL_DOC id=id/></div> } }

static INV_DOC: DocCfg = DocCfg { endpoint: "/sales-invoices", list_route: "/sales", list_label: "Sales & Customers", title_key: "invoice_no", subtitle_key: "customer_name", status_key: "status", steps: &[],
    facts: &[fact("Mushak 6.3 no.", "mushak_no", CK::Mono), fact("Order", "so_no", CK::Mono), fact("Delivery", "delivery_no", CK::Mono), fact("Date", "invoice_date", CK::Date), fact("Due", "due_date", CK::Date), fact("Subtotal", "subtotal", CK::Money), fact("SD", "sd_amount", CK::Money), fact("VAT", "vat_amount", CK::Money), fact("Total", "total", CK::Money), fact("Paid", "paid_amount", CK::Money), fact("Credit notes", "returned_amount", CK::Money), fact("Balance", "balance", CK::Money)],
    tables: &[dtable("Items", "lines", &[col("sku", "SKU", CK::Mono), col("product_name", "Product", CK::Text), col("qty", "Qty", CK::Qty), col("unit_price", "Price", CK::Money), col("discount_pct", "Disc %", CK::Pct), col("net_amount", "Value", CK::Money), col("vat_amount", "VAT", CK::Money), col("line_total", "Total", CK::Money), col("returned_qty", "Returned", CK::Qty)]),
        dtable("Payments received", "receipts", &[col("receipt_no", "Receipt", CK::Mono), col("receipt_date", "Date", CK::Date), col("amount", "Amount", CK::Money)]), dtable("Credit notes", "returns", &[col("return_no", "Credit note", CK::Mono), col("return_date", "Date", CK::Date), col("total", "Amount", CK::Money)])],
    actions: &[], notes_key: "notes" };
#[component] pub fn InvoicePage() -> impl IntoView {
    let id = use_params_map().with_untracked(|p| p.get("id").unwrap_or_default()); let id2 = id.clone();
    let show = RwSignal::new(false);
    view! { <div class="page"><PageHead title="" subtitle=""><button class="btn" on:click=move |_| show.set(true)><Icon name="print" size=16/>"Print Mushak 6.3"</button></PageHead></div>
        <div class="page" style="padding-top:0"><DocPage cfg=&INV_DOC id=id/></div>
        {move || show.get().then(|| view! { <MushakDialog id=id2.clone() on_close=closer(move || show.set(false))/> })} }
}
#[component]
fn MushakDialog(id: String, on_close: Callback<()>) -> impl IntoView {
    let res = LocalResource::new(move || { let id = id.clone(); async move { api::get(&format!("/sales-invoices/{id}/mushak-6-3")).await } });
    view! { <Modal title="Mushak 6.3 · Tax invoice" on_close=on_close wide=true footer=footer(move || view! { <button class="btn" on:click=move |_| on_close.run(())>{t("Cancel")}</button><button class="btn primary" on:click=move |_| { if let Some(w) = web_sys::window() { let _ = w.print(); } }><Icon name="print" size=16/>"Print"</button> }.into_any())>
        {move || match res.get() { None => view! { <Loading/> }.into_any(), Some(Err(e)) => view! { <ErrorBox err=e/> }.into_any(), Some(Ok(v)) => { let d = v["data"].clone(); let s = d["seller"].clone(); let b = d["buyer"].clone(); let tot = d["totals"].clone();
            view! { <div id="mushak" style="padding:8px 4px"><div style="text-align:center"><b style="font-size:17px">{fmt::text(&s["name"])}</b><div class="small">{fmt::text(&s["address"])}</div><div class="small">"BIN: "{fmt::text(&s["bin"])}</div><h3 style="margin:12px 0 2px;color:var(--tx)">"কর চালানপত্র · Tax Invoice"</h3><div class="small dim">"Mushak 6.3"</div></div>
                <div class="grid g2" style="margin:14px 0"><div class="small"><b>"Buyer: "</b>{fmt::text(&b["name"])}<br/>{fmt::text(&b["address"])}<br/>"BIN: "{fmt::text(&b["bin"])}</div><div class="small right"><b>"Serial: "</b>{fmt::text(&d["serial_no"])}<br/>"Invoice: "{fmt::text(&d["invoice_no"])}<br/>"Date: "{fmt::date(&d["issue_date"])}" "{fmt::text(&d["issue_time"])}<br/>"Vehicle: "{fmt::text(&d["vehicle_no"])}</div></div>
                <div class="tbl-wrap"><table><thead><tr><th>"SL"</th><th>"Description"</th><th class="r">"Qty"</th><th class="r">"Unit price"</th><th class="r">"Value"</th><th class="r">"SD"</th><th class="r">"VAT"</th><th class="r">"Total"</th></tr></thead><tbody>
                {d["items"].as_array().cloned().unwrap_or_default().into_iter().map(|i| view! { <tr><td>{fmt::text(&i["sl"])}</td><td>{fmt::text(&i["description"])}</td><td class="r num">{fmt::qty(&i["qty"])}</td><td class="r num">{fmt::money(&i["unit_price"])}</td><td class="r num">{fmt::money(&i["value_excl_tax"])}</td><td class="r num">{fmt::money(&i["sd"])}</td><td class="r num">{fmt::money(&i["vat"])}</td><td class="r num">{fmt::money(&i["total"])}</td></tr> }).collect_view()}
                <tr><td colspan="4"><b>"Total"</b></td><td class="r num"><b>{fmt::money(&tot["value_excl_tax"])}</b></td><td class="r num"><b>{fmt::money(&tot["sd"])}</b></td><td class="r num"><b>{fmt::money(&tot["vat"])}</b></td><td class="r num"><b>{fmt::money(&tot["grand_total"])}</b></td></tr></tbody></table></div></div> }.into_any() } }}
    </Modal> }
}

/// Record money received and apply it to the customer's invoices.
#[component]
pub fn NewReceipt() -> impl IntoView {
    const F: &[FF] = &[ff("customer_id", "Customer", FK::Lookup("/customers", &["name"])).req(), ff("bank_account_id", "Received into", FK::Lookup("/bank-accounts", &["name"])).req(), ff("receipt_date", "Date", FK::Date).def("today"), ff("gross_amount", "Amount (Tk)", FK::Dec).req().help("The full invoice amount, including anything the customer withheld"),
        ff("vds_amount", "VAT withheld by customer (Tk)", FK::Dec).help("VDS: they deposit it with the treasury and give you a certificate"), ff("tds_amount", "Income tax withheld (Tk)", FK::Dec), ff("method", "Method", FK::Select(&[("bank", "Bank"), ("cash", "Cash"), ("cheque", "Cheque"), ("bkash", "bKash"), ("other", "Other")])).def("bank"), ff("reference", "Reference", FK::Text)];
    let st = FormState::new(F, None);
    let invs: RwSignal<Vec<Value>> = RwSignal::new(vec![]); let alloc: RwSignal<Vec<String>> = RwSignal::new(vec![]);
    let nav = use_navigate();
    let cust_changed = Effect::new(move |_| { let c = st.get("customer_id"); if c.is_empty() { invs.set(vec![]); return; }
        leptos::task::spawn_local(async move { if let Ok(v) = api::get(&format!("/sales-invoices?customer_id={c}&unpaid=true&per_page=100")).await { let mut l = v["data"].as_array().cloned().unwrap_or_default(); l.sort_by(|a, b| a["invoice_date"].as_str().cmp(&b["invoice_date"].as_str())); alloc.set(vec![String::new(); l.len()]); invs.set(l); } }); });
    let _ = cust_changed;
    let auto = move |_| { let mut left = st.get("gross_amount").parse::<f64>().unwrap_or(0.0); let l = invs.get_untracked(); alloc.set(l.iter().map(|i| { let take = left.min(fmt::num(&i["balance"])); left -= take; if take > 0.0 { format!("{take:.2}") } else { String::new() } }).collect()); };
    let submit = move |_| { if st.busy.get_untracked() { return; } st.busy.set(true); st.err.set(None);
        let mut body = payload(F, &st); let a: Vec<Value> = invs.get_untracked().iter().zip(alloc.get_untracked()).filter(|(_, x)| x.parse::<f64>().map(|v| v > 0.0).unwrap_or(false)).map(|(i, x)| json!({"invoice_id": i["id"], "amount": x})).collect();
        if let Value::Object(m) = &mut body { m.insert("allocations".into(), json!(a)); } let nav = nav.clone();
        leptos::task::spawn_local(async move { let r = api::post("/customer-receipts", body).await; st.done(); match r { Ok(_) => { state::ok("Receipt recorded"); state::refresh(); nav("/sales", Default::default()); } Err(e) => st.fail(e) } }); };
    view! { <div class="page"><PageHead title="Record a receipt" subtitle="Apply the payment to invoices, oldest first. Anything left over stays as the customer's advance." crumbs=vec![(t("Sales & Customers"), "/sales".to_string())]/>
        <div class="grid" style="max-width:1100px"><Card><FormFields fields=F.to_vec() st=st/></Card>
        {move || (!invs.get().is_empty()).then(|| view! { <Card title="Apply to invoices" right=ViewFn::from(move || view! { <button class="btn sm" on:click=auto>"Auto-apply, oldest first"</button> })><div class="tbl-wrap"><table><thead><tr><th>"Invoice"</th><th>"Date"</th><th>"Due"</th><th class="r">"Balance"</th><th class="r">"Apply"</th></tr></thead><tbody>
            {invs.get().into_iter().enumerate().map(|(i, v)| view! { <tr><td><b>{fmt::text(&v["invoice_no"])}</b></td><td>{fmt::date(&v["invoice_date"])}</td><td>{fmt::date(&v["due_date"])}</td><td class="r num">{fmt::money(&v["balance"])}</td><td class="r"><input class="input" style="width:150px;text-align:right" type="number" min="0" step="any" prop:value=move || alloc.get().get(i).cloned().unwrap_or_default() on:input=move |e| alloc.update(|a| if i < a.len() { a[i] = event_target_value(&e) })/></td></tr> }).collect_view()}</tbody></table></div></Card> })}
        <FormError st=st/><div class="row"><a class="btn" href="/sales">{t("Cancel")}</a><button class="btn primary sp" disabled=move || st.busy.get() on:click=submit>{move || if st.busy.get() { view! { <Spinner/> }.into_any() } else { view! { "Record receipt" }.into_any() }}</button></div></div></div> }
}

/// Return goods against an invoice and issue a credit note.
#[component]
pub fn NewReturn() -> impl IntoView {
    const F: &[FF] = &[ff("invoice_id", "Invoice", FK::Lookup("/sales-invoices", &["invoice_no", "customer_name"])).req(), ff("warehouse_id", "Goods go back to", FK::Lookup("/warehouses", &["name"])), ff("return_date", "Date", FK::Date).def("today"), ff("reason", "Reason", FK::Text).req().full()];
    let st = FormState::new(F, None); let doc: RwSignal<Option<Value>> = RwSignal::new(None); let qtys: RwSignal<Vec<String>> = RwSignal::new(vec![]); let nav = use_navigate();
    Effect::new(move |_| { let id = st.get("invoice_id"); if id.is_empty() { doc.set(None); return; } leptos::task::spawn_local(async move { if let Ok(v) = api::get(&format!("/sales-invoices/{id}")).await { let d = v["data"].clone(); qtys.set(vec![String::new(); d["lines"].as_array().map(|a| a.len()).unwrap_or(0)]); doc.set(Some(d)); } }); });
    let submit = move |_| { if st.busy.get_untracked() { return; } st.busy.set(true); st.err.set(None); let mut body = payload(F, &st); let d = doc.get_untracked().unwrap_or(Value::Null);
        let lines: Vec<Value> = d["lines"].as_array().cloned().unwrap_or_default().iter().zip(qtys.get_untracked()).filter(|(_, q)| q.parse::<f64>().map(|x| x > 0.0).unwrap_or(false)).map(|(l, q)| json!({"invoice_line_id": l["id"], "qty": q})).collect();
        if let Value::Object(m) = &mut body { m.insert("lines".into(), json!(lines)); } let nav = nav.clone();
        leptos::task::spawn_local(async move { let r = api::post("/sales-returns", body).await; st.done(); match r { Ok(_) => { state::ok("Credit note issued"); state::refresh(); nav("/sales", Default::default()); } Err(e) => st.fail(e) } }); };
    view! { <div class="page"><PageHead title="Customer return" subtitle="Choose the invoice, then how much of each item came back. A credit note (Mushak 6.7) is issued and stock goes back at its original cost." crumbs=vec![(t("Sales & Customers"), "/sales".to_string())]/>
        <div class="grid" style="max-width:1100px"><Card><FormFields fields=F.to_vec() st=st/></Card>
        {move || doc.get().map(|d| view! { <Card title="Items returned"><div class="tbl-wrap"><table><thead><tr><th>"Item"</th><th class="r">"Invoiced"</th><th class="r">"Already returned"</th><th class="r">"Returning now"</th></tr></thead><tbody>
            {d["lines"].as_array().cloned().unwrap_or_default().into_iter().enumerate().map(|(i, l)| view! { <tr><td><b>{fmt::text(&l["sku"])}</b>" · "{fmt::text(&l["product_name"])}</td><td class="r num">{fmt::qty(&l["qty"])}</td><td class="r num">{fmt::qty(&l["returned_qty"])}</td><td class="r"><input class="input" style="width:130px;text-align:right" type="number" min="0" step="any" prop:value=move || qtys.get().get(i).cloned().unwrap_or_default() on:input=move |e| qtys.update(|q| if i < q.len() { q[i] = event_target_value(&e) })/></td></tr> }).collect_view()}</tbody></table></div></Card> })}
        <FormError st=st/><div class="row"><a class="btn" href="/sales">{t("Cancel")}</a><button class="btn primary sp" disabled=move || st.busy.get() on:click=submit>"Issue credit note"</button></div></div></div> }
}
