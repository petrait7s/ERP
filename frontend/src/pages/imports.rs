//! Imports and voyages: LCs, shipments with live tracking, containers, documents and foreign payments.
use super::common::*;
use crate::{api, crud::*, fmt, i18n::t, state, ui::*};
use leptos::prelude::*;
use leptos_router::hooks::{use_navigate, use_params_map};
use serde_json::{json, Map, Value};

const SHIP_STATUS: &[(&str, &str)] = &[("booked", "Booked"), ("in_transit", "In transit"), ("arrived", "Arrived"), ("customs", "At customs"), ("cleared", "Cleared"), ("received", "Received")];
const SHIPMENT_COLS: &[Col] = &[col("shipment_no", "Shipment", CK::Bold), col("supplier_name", "Supplier", CK::Text), col("container_nos", "Containers", CK::Mono), col("vessel_name", "Vessel", CK::Text), col("eta", "ETA", CK::Date), col("invoice_value", "Value", CK::Fc), col("documents_missing", "Docs missing", CK::Int), col("status", "Status", CK::Badge)];
static SHIPMENTS: ListCfg = ListCfg { title: "Shipments", subtitle: "", endpoint: "/shipments", cols: SHIPMENT_COLS, search: true, filters: &[Filter { param: "status", label: "All statuses", options: SHIP_STATUS }], fields: &[], write: "import.write", new_label: "Book shipment", edit: false, delete: false, detail: "/shipments/", new_route: "/shipments/new", extra_query: "", empty_hint: "Book a shipment against an approved import order.", date_filter: false };
const LC_COLS: &[Col] = &[col("lc_no", "LC", CK::Bold), col("bank_lc_no", "Bank LC no.", CK::Mono), col("supplier_name", "Supplier", CK::Text), col("po_no", "Order", CK::Mono), col("amount", "Amount", CK::Fc), col("margin_bdt", "Margin held", CK::Money), col("expiry_date", "Expires", CK::Date), col("status", "Status", CK::Badge)];
static LCS: ListCfg = ListCfg { title: "Letters of Credit", subtitle: "", endpoint: "/lcs", cols: LC_COLS, search: true, filters: &[Filter { param: "status", label: "All statuses", options: &[("draft", "Draft"), ("opened", "Opened"), ("settled", "Settled"), ("cancelled", "Cancelled")] }], fields: &[], write: "import.write", new_label: "New LC", edit: false, delete: false, detail: "/lcs/", new_route: "/lcs/new", extra_query: "", empty_hint: "Open an LC for an approved import order.", date_filter: false };
const FP_COLS: &[Col] = &[col("payment_no", "Payment", CK::Bold), col("payment_date", "Date", CK::Date), col("kind", "Type", CK::Badge), col("supplier_name", "Supplier", CK::Text), col("fc_amount", "Foreign amount", CK::Fc), col("fx_rate", "Rate", CK::Qty), col("bdt_amount", "Taka", CK::Money), col("fx_gain_loss", "FX diff", CK::Money), col("status", "Status", CK::Badge)];
static FPAY: ListCfg = ListCfg { title: "Foreign Payments", subtitle: "", endpoint: "/foreign-payments", cols: FP_COLS, search: false, filters: &[Filter { param: "kind", label: "All types", options: &[("tt_advance", "TT advance"), ("tt_payment", "TT payment"), ("lc_settlement", "LC settlement"), ("lc_margin", "LC margin"), ("lc_charges", "LC charges")] }], fields: &[], write: "import.pay", new_label: "TT advance", edit: false, delete: false, detail: "", new_route: "/foreign-payments/new", extra_query: "", empty_hint: "", date_filter: true };

#[component]
pub fn Imports() -> impl IntoView {
    let tab = RwSignal::new("board");
    view! { <div class="page">
        <PageHead title=t("Imports & Voyages") subtitle="Every order, letter of credit and container from the supplier's factory to your warehouse.">
            {move || state::can("import.write").then(|| view! { <a class="btn primary" href="/shipments/new"><Icon name="plus" size=16/>"Book shipment"</a> })}
        </PageHead>
        <Tabs tabs=vec![("board", "Voyage board".to_string()), ("shipments", t("Shipments")), ("lcs", t("Letters of Credit")), ("pay", t("Foreign Payments"))] active=tab/>
        {move || match tab.get() { "board" => view! { <VoyageBoard/> }.into_any(), "shipments" => view! { <ListPage cfg=&SHIPMENTS headless=true/> }.into_any(), "lcs" => view! { <ListPage cfg=&LCS headless=true/> }.into_any(), _ => view! { <ListPage cfg=&FPAY headless=true/> }.into_any() }}
    </div> }
}

#[component]
fn VoyageBoard() -> impl IntoView {
    let tick = state::app().tick;
    let res = LocalResource::new(move || { let _ = tick.get(); async { api::get("/shipments?active=true&per_page=200").await } });
    let nav = use_navigate();
    view! { {move || match res.get() {
        None => view! { <Loading/> }.into_any(), Some(Err(e)) => view! { <ErrorBox err=e/> }.into_any(),
        Some(Ok(v)) => { let all = v["data"].as_array().cloned().unwrap_or_default(); let nav = nav.clone();
            if all.is_empty() { return view! { <div class="card"><Empty title="No shipments in progress" hint="Book a shipment when your supplier confirms the cargo is ready."/></div> }.into_any(); }
            view! { <div class="kanban">{SHIP_STATUS.iter().take(5).map(|(st, label)| { let items: Vec<Value> = all.iter().filter(|s| s["status"] == *st).cloned().collect(); let n = items.len(); let nav = nav.clone();
                view! { <div class="kcol"><h4>{t(label)}<span class="badge">{n}</span></h4>
                    {items.into_iter().map(|s| { let id = fmt::text(&s["id"]); let nav = nav.clone(); let pct = fmt::num(&s["progress_pct"]); let late = s["days_to_eta"].as_i64().map(|d| d < 0).unwrap_or(false); let missing = s["documents_missing"].as_i64().unwrap_or(0);
                        view! { <div class="kcard" on:click=move |_| nav(&format!("/shipments/{id}"), Default::default())>
                            <div class="row"><b>{fmt::text(&s["shipment_no"])}</b><span class="sp small mono">{fmt::text(&s["container_nos"])}</span></div>
                            <div class="small mut" style="margin:2px 0 8px">{fmt::text(&s["supplier_name"])}</div>
                            <div class="row small"><Icon name="ship" size=15/><span>{fmt::text(&s["port_of_loading"])}" → "{fmt::text(&s["port_of_discharge"])}</span></div>
                            <div class=if late { "bar bad" } else { "bar" } style="margin:10px 0 6px"><i style=format!("width:{pct:.0}%")></i></div>
                            <div class="row small"><span>{match s["days_to_eta"].as_i64() { Some(d) if d < 0 => format!("{} days late", -d), Some(d) => format!("ETA in {d} days"), None => "No ETA".into() }}</span><b class="sp num">{fmt::fc(&s["invoice_value"], s["currency"].as_str().unwrap_or(""))}</b></div>
                            {(missing > 0).then(|| view! { <div class="badge warn" style="margin-top:8px">{format!("{missing} documents missing")}</div> })}
                        </div> } }).collect_view()}
                    {(n == 0).then(|| view! { <div class="dim small" style="padding:10px">"None"</div> })}</div> } }).collect_view()}</div> }.into_any() } } }}
}

const SHIP_STEPS: &[(&str, &str)] = &[("booked", "Booked"), ("in_transit", "At sea"), ("arrived", "Arrived"), ("customs", "Customs"), ("cleared", "Cleared"), ("received", "In stock")];
const EVENT_TYPES: &[(&str, &str)] = &[("gate_in", "Gate in"), ("loaded", "Loaded"), ("departed", "Departed"), ("transshipment", "Transshipment"), ("arrived", "Arrived"), ("discharged", "Discharged"), ("customs_hold", "Customs hold"), ("customs_released", "Customs released"), ("gate_out", "Gate out"), ("delivered", "Delivered"), ("delay", "Delay"), ("note", "Note")];
const DOC_STATUS: &[(&str, &str)] = &[("pending", "Pending"), ("requested", "Requested from supplier"), ("received", "Received"), ("verified", "Verified"), ("submitted", "Submitted to customs")];
static SHIPMENT_DOC: DocCfg = DocCfg {
    endpoint: "/shipments", list_route: "/imports", list_label: "Imports & Voyages", title_key: "shipment_no", subtitle_key: "supplier_name", status_key: "status", steps: SHIP_STEPS,
    facts: &[fact("Purchase order", "po_no", CK::Mono), fact("LC", "lc_no", CK::Mono), fact("Mode", "mode", CK::Badge), fact("Vessel", "vessel_name", CK::Text), fact("Voyage", "voyage_no", CK::Text), fact("Bill of lading", "bl_no", CK::Mono),
        fact("From", "port_of_loading", CK::Text), fact("To", "port_of_discharge", CK::Text), fact("ETD", "etd", CK::Date), fact("ETA", "eta", CK::Date), fact("Arrived", "arrived_at", CK::Date),
        fact("Invoice (foreign)", "invoice_value", CK::Fc), fact("Booking rate", "fx_rate", CK::Qty), fact("Invoice (Taka)", "invoice_bdt", CK::Money), fact("Still to pay supplier", "payable_fc", CK::Fc), fact("Demurrage so far", "demurrage_estimate", CK::Money), fact("Documents missing", "documents_missing", CK::Int)],
    tables: &[
        dtable("Items", "lines", &[col("sku", "SKU", CK::Mono), col("product_name", "Product", CK::Text), col("hs_code", "HS code", CK::Mono), col("qty", "Qty", CK::Qty), col("unit_price", "Unit price", CK::Qty), col("line_value", "Value", CK::Qty), col("received_qty", "Received", CK::Qty)]),
        dtable("Containers", "containers", &[col("container_no", "Container", CK::Mono), col("size", "Size", CK::Text), col("seal_no", "Seal", CK::Mono), col("status", "Status", CK::Badge), col("free_until", "Free until", CK::Date), col("days_over_free", "Days over free time", CK::Int)]),
        DocTable { title: "Import documents", key: "documents", cols: &[col("doc_type", "Type", CK::Bold), col("doc_no", "Number", CK::Mono), col("doc_date", "Date", CK::Date), col("status", "Status", CK::Badge), col("is_required", "Required", CK::Bool)],
            row_action: Some(RowAction { label: "Mark received", method: "PUT", path: "/documents/{doc_type}", body: r#"{"status":"received"}"#, perm: "import.write", show_if: "", show_not: "received|verified|submitted" }) },
        dtable("Voyage tracking", "events", &[col("event_time", "When", CK::DateTime), col("event_type", "Event", CK::Badge), col("location", "Place", CK::Text), col("description", "Details", CK::Text)]),
        dtable("Payments to supplier", "payments", &[col("payment_no", "Payment", CK::Mono), col("payment_date", "Date", CK::Date), col("kind", "Type", CK::Badge), col("fc_amount", "Foreign", CK::Qty), col("fx_rate", "Rate", CK::Qty), col("bdt_amount", "Taka", CK::Money), col("status", "Status", CK::Badge)]),
    ],
    actions: &[
        action("Mark departed", "/depart", "import.write").when(&["booked"]).primary().fields(&[ff("bl_no", "Bill of lading no.", FK::Text).req().help("Printed on the bill of lading or airway bill"), ff("departed_at", "Departure date", FK::Date).def("today"), ff("fx_rate", "Exchange rate (BDT per unit)", FK::Dec).help("Leave empty to use the latest rate in the system")]).done("Shipment departed: goods are now in transit"),
        action("Mark arrived", "/arrive", "import.write").when(&["in_transit"]).primary().fields(&[ff("arrived_at", "Arrival date", FK::Date).def("today")]).done("Arrival recorded: the port free-time clock has started"),
        action("Lodge Bill of Entry", "", "import.write").when(&["in_transit", "arrived"]).abs("/bills-of-entry", "shipment_id").fields(&[ff("be_no", "Bill of Entry number", FK::Text).req(), ff("be_date", "Date", FK::Date).def("today"), ff("customs_rate", "Customs exchange rate", FK::Dec).req().help("The rate customs publishes for the week"), ff("cnf_agent", "C&F agent", FK::Text), ff("other_fees", "Other customs fees (Tk)", FK::Dec)]).done("Duties assessed").goto("/bills-of-entry/{rid}"),
        action("Start landed cost", "", "import.write").when(&["arrived", "customs", "cleared"]).abs("/landed-costs", "shipment_id").fields(&[ff("allocation_method", "Spread shared costs by", FK::Select(&[("value", "Value"), ("weight", "Weight"), ("quantity", "Quantity"), ("volume", "Volume")])).req().def("value")]).done("Landed cost sheet started").goto("/landed-costs/{rid}"),
        action("Pay supplier", "", "import.pay").when(&["in_transit", "arrived", "customs", "cleared", "received"]).abs("/foreign-payments", "shipment_id").fields(&[ff("kind", "How", FK::Select(&[("tt_payment", "Telegraphic transfer"), ("lc_settlement", "LC settlement (documents)")])).req(), ff("bank_account_id", "Pay from", FK::Lookup("/bank-accounts?kind=bank", &["name", "currency"])).req(), ff("payment_date", "Date", FK::Date).def("today"), ff("fc_amount", "Amount in foreign currency", FK::Dec).req(), ff("fx_rate", "Exchange rate", FK::Dec).help("Leave empty to use the latest rate"), ff("bank_charges", "Bank charges (Tk)", FK::Dec)]).done("Payment recorded"),
        action("Receive into warehouse", "/receive", "inventory.write").when(&["cleared"]).primary().fields(&[ff("warehouse_id", "Warehouse", FK::Lookup("/warehouses", &["name"])).req(), ff("receipt_date", "Date received", FK::Date).def("today")]).lines(LineQty { key: "lines", send: "shipment_line_id", show: "product_name", total: "qty", done: "received_qty", label: "Received now" }).done("Goods received into stock at landed cost"),
        action("Add tracking update", "/events", "import.write").when(&["booked", "in_transit", "arrived", "customs", "cleared"]).fields(&[ff("event_type", "What happened", FK::Select(EVENT_TYPES)).req(), ff("location", "Place", FK::Text), ff("description", "Details", FK::Area).full()]).done("Tracking updated"),
        action("Add container", "/containers", "import.write").when(&["booked", "in_transit", "arrived", "customs", "cleared"]).fields(&[ff("container_no", "Container number", FK::Text).req(), ff("size", "Size", FK::Select(&[("40HC", "40ft high cube"), ("40GP", "40ft"), ("20GP", "20ft"), ("LCL", "LCL"), ("AIR", "Air cargo")])).def("40HC"), ff("seal_no", "Seal number", FK::Text)]).done("Container added"),
        action("Cancel booking", "/cancel", "import.write").when(&["booked"]).danger().confirm("Cancel this booking? The quantities go back to the purchase order.").done("Booking cancelled"),
    ],
    notes_key: "notes",
};
#[component]
pub fn ShipmentPage() -> impl IntoView { let id = use_params_map().with_untracked(|p| p.get("id").unwrap_or_default()); view! { <div class="page"><DocPage cfg=&SHIPMENT_DOC id=id/></div> } }

/// Book a shipment: choose the order, then say how much of each line is on board.
#[component]
pub fn NewShipment() -> impl IntoView {
    let po = RwSignal::new(String::new());
    let po_doc: RwSignal<Option<Value>> = RwSignal::new(None);
    let qtys: RwSignal<Vec<String>> = RwSignal::new(vec![]);
    const FIELDS: &[FF] = &[ff("lc_id", "Letter of credit", FK::Lookup("/lcs?status=opened", &["lc_no", "supplier_name"])), ff("mode", "Mode", FK::Select(&[("sea", "Sea"), ("air", "Air"), ("land", "Land")])).def("sea"), ff("vessel_name", "Vessel", FK::Text), ff("voyage_no", "Voyage", FK::Text), ff("carrier", "Shipping line", FK::Text),
        ff("etd", "Departure (ETD)", FK::Date), ff("eta", "Arrival (ETA)", FK::Date), ff("commercial_invoice_no", "Commercial invoice no.", FK::Text), ff("freight", "Freight on the invoice (foreign)", FK::Dec).help("Only if the supplier's invoice includes freight (CFR/CIF)"), ff("insurance", "Insurance on the invoice (foreign)", FK::Dec),
        ff("free_days", "Free days at port", FK::Int).def("4").help("Days before the port starts charging demurrage"), ff("demurrage_per_day", "Demurrage per container per day (Tk)", FK::Dec), ff("containers", "Container numbers", FK::Area).help("One per line or separated by commas").full(), ff("notes", "Notes", FK::Area).full()];
    let st = FormState::new(FIELDS, None);
    let nav = use_navigate();
    let pick = move |e: leptos::ev::Event| { let id = event_target_value(&e); po.set(id.clone()); po_doc.set(None); if id.is_empty() { return; }
        leptos::task::spawn_local(async move { if let Ok(v) = api::get(&format!("/purchase-orders/{id}")).await { let d = v["data"].clone(); qtys.set(d["lines"].as_array().map(|a| a.iter().map(|l| (fmt::num(&l["qty"]) - fmt::num(&l["shipped_qty"])).max(0.0).to_string()).collect()).unwrap_or_default()); po_doc.set(Some(d)); } }); };
    let submit = move |_| {
        if st.busy.get_untracked() { return; } st.busy.set(true); st.err.set(None);
        let mut body = payload(FIELDS, &st); let d = po_doc.get_untracked().unwrap_or(Value::Null);
        let lines: Vec<Value> = d["lines"].as_array().cloned().unwrap_or_default().iter().zip(qtys.get_untracked()).filter(|(_, q)| q.parse::<f64>().map(|x| x > 0.0).unwrap_or(false)).map(|(l, q)| json!({"po_line_id": l["id"], "qty": q})).collect();
        let conts: Vec<Value> = st.get("containers").split(|c: char| c == ',' || c == '\n').map(str::trim).filter(|s| !s.is_empty()).map(|c| json!({"container_no": c})).collect();
        if let Value::Object(m) = &mut body { m.remove("containers"); m.insert("po_id".into(), json!(po.get_untracked())); m.insert("lines".into(), json!(lines)); m.insert("containers".into(), json!(conts)); }
        let nav = nav.clone();
        leptos::task::spawn_local(async move { let r = api::post("/shipments", body).await; st.busy.set(false); match r { Ok(v) => { state::ok("Shipment booked"); nav(&format!("/shipments/{}", v["data"]["id"].as_str().unwrap_or("")), Default::default()); } Err(e) => st.err.set(Some(e)) } });
    };
    view! { <div class="page"><PageHead title="Book a shipment" subtitle="Record what the supplier has put on board against an approved import order." crumbs=vec![(t("Imports & Voyages"), "/imports".to_string())]/>
        <div class="grid" style="max-width:1100px">
            <Card><div class="field"><label>"Import purchase order"<span class="req">"*"</span></label>
                <select class="select" on:change=pick><option value="">"Choose an approved import order…"</option>
                {move || { let r = LocalResource::new(|| options("/purchase-orders?po_type=import")); r.get().map(|l| l.into_iter().filter(|o| matches!(o["status"].as_str(), Some("approved") | Some("partially_received"))).map(|o| { let id = fmt::text(&o["id"]); let lbl = format!("{} · {} · {}", fmt::text(&o["po_no"]), fmt::text(&o["supplier_name"]), fmt::fc(&o["total"], o["currency"].as_str().unwrap_or(""))); view! { <option value=id>{lbl}</option> } }).collect_view()) }}
                </select></div></Card>
            {move || po_doc.get().map(|d| view! { <Card title="What is on board">
                <div class="tbl-wrap"><table><thead><tr><th>"Item"</th><th class="r">"Ordered"</th><th class="r">"Already shipped"</th><th class="r">"On this shipment"</th></tr></thead><tbody>
                {d["lines"].as_array().cloned().unwrap_or_default().into_iter().enumerate().map(|(i, l)| view! { <tr><td><b>{fmt::text(&l["sku"])}</b>" · "{fmt::text(&l["product_name"])}</td><td class="r num">{fmt::qty(&l["qty"])}</td><td class="r num">{fmt::qty(&l["shipped_qty"])}</td>
                    <td class="r"><input class="input" style="width:130px;text-align:right" type="number" min="0" step="any" prop:value=move || qtys.get().get(i).cloned().unwrap_or_default() on:input=move |e| qtys.update(|q| if i < q.len() { q[i] = event_target_value(&e) })/></td></tr> }).collect_view()}
                </tbody></table></div></Card> })}
            {move || po_doc.get().map(|_| view! { <Card title="Voyage details"><FormFields fields=FIELDS.to_vec() st=st/></Card> })}
            <FormError st=st/>
            <div class="row"><a class="btn" href="/imports">{t("Cancel")}</a><button class="btn primary sp" disabled=move || st.busy.get() || po_doc.get().is_none() on:click=submit>{move || if st.busy.get() { view! { <Spinner/> }.into_any() } else { view! { "Book shipment" }.into_any() }}</button></div>
        </div></div> }
}

static NEW_LC: NewDoc = NewDoc { title: "New letter of credit", subtitle: "Draft the LC for an approved import order. The bank takes the margin when you open it.", endpoint: "/lcs", back_label: "Imports & Voyages", back_route: "/imports",
    header: &[ff("po_id", "Import purchase order", FK::Lookup("/purchase-orders?po_type=import&status=approved", &["po_no", "supplier_name"])).req(), ff("bank_account_id", "Issuing bank account", FK::Lookup("/bank-accounts?kind=bank", &["name"])).req(), ff("lc_type", "LC type", FK::Select(&[("sight", "At sight"), ("usance", "Usance"), ("deferred", "Deferred"), ("upas", "UPAS")])).req().def("sight"),
        ff("amount", "LC amount (foreign)", FK::Dec).help("Leave empty to use the order value"), ff("margin_pct", "Margin %", FK::Dec).def("20").help("The share of the LC value the bank holds in cash"), ff("tolerance_pct", "Tolerance %", FK::Dec).def("5"), ff("latest_shipment_date", "Latest shipment date", FK::Date), ff("expiry_date", "Expiry date", FK::Date).req(),
        ff("usance_days", "Usance days", FK::Int).help("Only for usance LCs"), ff("advising_bank", "Advising bank", FK::Text), ff("notes", "Notes", FK::Area).full()],
    lines_title: "", lines: &[], autofill: None, goto: "/lcs/{rid}", done: "LC drafted", submit_label: "Save draft", min_lines: 0 };
#[component] pub fn NewLc() -> impl IntoView { view! { <NewDocPage cfg=&NEW_LC/> } }

static NEW_FP: NewDoc = NewDoc { title: "Advance payment by TT", subtitle: "Send an advance to a foreign supplier. It is held against the order until the goods ship.", endpoint: "/foreign-payments", back_label: "Imports & Voyages", back_route: "/imports",
    header: &[ff("kind", "Type", FK::Select(&[("tt_advance", "TT advance")])).req().def("tt_advance"), ff("po_id", "Import purchase order", FK::Lookup("/purchase-orders?po_type=import&status=approved", &["po_no", "supplier_name"])).req(), ff("bank_account_id", "Pay from", FK::Lookup("/bank-accounts?kind=bank", &["name", "currency"])).req(), ff("payment_date", "Date", FK::Date).def("today"),
        ff("fc_amount", "Amount in foreign currency", FK::Dec).req(), ff("fx_rate", "Exchange rate", FK::Dec).help("Leave empty to use the latest rate"), ff("bank_charges", "Bank charges (Tk)", FK::Dec), ff("reference", "Reference", FK::Text)],
    lines_title: "", lines: &[], autofill: None, goto: "/imports", done: "Payment recorded", submit_label: "Send payment", min_lines: 0 };
#[component] pub fn NewForeignPayment() -> impl IntoView { view! { <NewDocPage cfg=&NEW_FP/> } }

static LC_DOC: DocCfg = DocCfg {
    endpoint: "/lcs", list_route: "/imports", list_label: "Imports & Voyages", title_key: "lc_no", subtitle_key: "supplier_name", status_key: "status", steps: &[("draft", "Drafted"), ("opened", "Opened"), ("settled", "Settled")],
    facts: &[fact("Bank LC no.", "bank_lc_no", CK::Mono), fact("Order", "po_no", CK::Mono), fact("Type", "lc_type", CK::Badge), fact("LC value", "amount", CK::Fc), fact("Used by shipments", "utilized_amount", CK::Fc), fact("Paid", "settled_amount", CK::Fc), fact("Still to pay", "outstanding", CK::Fc), fact("Margin held by bank", "margin_bdt", CK::Money),
        fact("Opened", "open_date", CK::Date), fact("Latest shipment", "latest_shipment_date", CK::Date), fact("Expires", "expiry_date", CK::Date), fact("Tolerance", "tolerance_pct", CK::Pct), fact("Issuing account", "bank_account", CK::Text), fact("Advising bank", "advising_bank", CK::Text)],
    tables: &[dtable("Shipments under this LC", "shipments", &[col("shipment_no", "Shipment", CK::Bold), col("status", "Status", CK::Badge), col("invoice_value", "Invoice value", CK::Qty)]),
        dtable("Amendments", "amendments", &[col("amendment_no", "No.", CK::Int), col("amendment_date", "Date", CK::Date), col("reason", "Reason", CK::Text)]),
        dtable("Bank payments", "payments", &[col("payment_no", "Payment", CK::Mono), col("payment_date", "Date", CK::Date), col("kind", "Type", CK::Badge), col("fc_amount", "Foreign", CK::Qty), col("bdt_amount", "Taka", CK::Money), col("status", "Status", CK::Badge)])],
    actions: &[
        action("Open the LC", "/open", "import.pay").when(&["draft"]).primary().fields(&[ff("open_date", "Opening date", FK::Date).def("today"), ff("bank_lc_no", "Bank's LC number", FK::Text), ff("fx_rate", "Exchange rate for margin", FK::Dec).help("Leave empty to use the latest rate"), ff("commission", "Opening commission (Tk)", FK::Dec), ff("swift_charges", "SWIFT charges (Tk)", FK::Dec), ff("other_charges", "Other charges (Tk)", FK::Dec)]).done("LC opened: margin and charges paid"),
        action("Amend", "/amend", "import.write").when(&["opened"]).fields(&[ff("amount", "New LC value (foreign)", FK::Dec), ff("expiry_date", "New expiry date", FK::Date), ff("latest_shipment_date", "New latest shipment date", FK::Date), ff("tolerance_pct", "Tolerance %", FK::Dec), ff("charges", "Amendment charges (Tk)", FK::Dec), ff("reason", "Reason", FK::Text).req().full()]).done("LC amended"),
        action("Close & release margin", "/close", "import.pay").when(&["opened"]).confirm("Close the LC? Any margin the bank still holds returns to your account. All shipment documents must be paid first.").done("LC closed"),
        action("Cancel", "/close", "import.write").when(&["draft"]).danger().fixed(r#"{"cancel":true}"#).confirm("Cancel this draft LC?").done("LC cancelled"),
    ], notes_key: "notes" };
#[component] pub fn LcPage() -> impl IntoView { let id = use_params_map().with_untracked(|p| p.get("id").unwrap_or_default()); view! { <div class="page"><DocPage cfg=&LC_DOC id=id/></div> } }
