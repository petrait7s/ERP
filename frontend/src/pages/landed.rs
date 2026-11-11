//! Landed cost: what an imported product really costs once freight, duty and port charges are spread over it.
use crate::{crud::*, i18n::t, ui::*};
use leptos::prelude::*;
use leptos_router::hooks::use_params_map;

const COLS: &[Col] = &[col("costing_no", "Costing", CK::Bold), col("shipment_no", "Shipment", CK::Mono), col("supplier_name", "Supplier", CK::Text), col("goods_bdt", "Goods", CK::Money), col("duties_bdt", "Duties", CK::Money), col("charges_bdt", "Charges", CK::Money), col("total_landed", "Landed total", CK::Money), col("cost_uplift_pct", "Uplift", CK::Pct), col("status", "Status", CK::Badge)];
static LIST: ListCfg = ListCfg { title: "Landed Cost", subtitle: "The true cost of every imported product: goods + freight + duties + port and clearing charges, spread over the items.", endpoint: "/landed-costs", cols: COLS, search: false,
    filters: &[Filter { param: "status", label: "All", options: &[("draft", "Draft"), ("finalized", "Finalised")] }], fields: &[], write: "", new_label: "", edit: false, delete: false, detail: "/landed-costs/", new_route: "", extra_query: "", empty_hint: "Start a cost sheet from a shipment's page once it has a Bill of Entry.", date_filter: false };
#[component]
pub fn LandedCosts() -> impl IntoView { view! { <div class="page"><ListPage cfg=&LIST/>
    <div class="alert info" style="margin-top:16px"><Icon name="calc"/><div><b>"How it works. "</b>"Open a shipment, lodge its Bill of Entry, then choose “Start landed cost”. Add the freight, C&F, port and transport bills, then finalise. Stock is received at the resulting unit cost, so margins are always real."</div></div></div> } }

static DOC: DocCfg = DocCfg {
    endpoint: "/landed-costs", list_route: "/landed-costs", list_label: "Landed Cost", title_key: "costing_no", subtitle_key: "supplier_name", status_key: "status", steps: &[("draft", "Collecting costs"), ("finalized", "Finalised")],
    facts: &[fact("Shipment", "shipment_no", CK::Mono), fact("Order", "po_no", CK::Mono), fact("Spread by", "allocation_method", CK::Badge), fact("Goods", "goods_bdt", CK::Money), fact("Duties and fees", "duties_bdt", CK::Money), fact("Freight and charges", "charges_bdt", CK::Money), fact("Landed total", "total_landed", CK::Money), fact("Date", "costing_date", CK::Date)],
    tables: &[
        dtable("Cost per product", "lines", &[col("sku", "SKU", CK::Mono), col("product_name", "Product", CK::Text), col("qty", "Qty", CK::Qty), col("goods_bdt", "Goods", CK::Money), col("duties_bdt", "Duties", CK::Money), col("charges_bdt", "Charges", CK::Money), col("landed_total", "Landed", CK::Money), col("unit_landed_cost", "Per unit", CK::Money), col("sale_price", "Sale price", CK::Money), col("margin_pct", "Margin", CK::Pct)]),
        DocTable { title: "Charges added", key: "charges", cols: &[col("charge_type", "Type", CK::Badge), col("description", "Description", CK::Text), col("amount", "Amount", CK::Money), col("vat_amount", "VAT", CK::Money), col("supplier_name", "Owed to", CK::Text), col("bank_account", "Paid from", CK::Text)],
            row_action: Some(RowAction { label: "Remove", method: "DELETE", path: "/charges/{id}", body: "", perm: "import.write", show_if: "", show_not: "" }) },
    ],
    actions: &[
        action("Add charge", "/charges", "import.write").when(&["draft"]).primary().fields(&[ff("charge_type", "Type", FK::Select(&[("freight", "Freight"), ("insurance", "Insurance"), ("lc_commission", "LC commission"), ("bank_charges", "Bank charges"), ("cnf_commission", "C&F commission"), ("port_charges", "Port charges"), ("shipping_agent", "Shipping agent"), ("transport", "Transport"), ("labour", "Labour"), ("demurrage", "Demurrage"), ("testing", "Testing"), ("other", "Other")])).req(),
        ff("description", "Description", FK::Text), ff("amount", "Amount (Tk, before VAT)", FK::Dec).req(), ff("vat_amount", "VAT on it (Tk)", FK::Dec).help("Recoverable input VAT: it does not add to cost"), ff("supplier_id", "Owed to (supplier)", FK::Lookup("/suppliers?supplier_type=service", &["name"])).help("Choose a service provider you will pay later, or…"), ff("bank_account_id", "…paid from", FK::Lookup("/bank-accounts", &["name"])).help("…the account it was already paid from"),
        ff("allocation_method", "Spread by", FK::Select(&[("value", "Value"), ("weight", "Weight"), ("quantity", "Quantity"), ("volume", "Volume")])).help("Leave empty to use the sheet's method")]).done("Charge added"),
        action("Change how costs are spread", "/method", "import.write").when(&["draft"]).method("PUT").fields(&[ff("allocation_method", "Spread shared charges by", FK::Select(&[("value", "Value"), ("weight", "Weight"), ("quantity", "Quantity"), ("volume", "Volume")])).req()]).done("Recalculated"),
        action("Finalise", "/finalize", "import.write").when(&["draft"]).confirm("Lock this costing? Charges are posted to the books and the goods can then be received at this landed cost.").done("Landed cost finalised"),
    ], notes_key: "" };
#[component]
pub fn LandedCostPage() -> impl IntoView { let id = use_params_map().with_untracked(|p| p.get("id").unwrap_or_default()); view! { <div class="page"><DocPage cfg=&DOC id=id/></div> } }
