//! Master data: products, categories, units, HS codes, suppliers, customers, warehouses.
use crate::{api::Api, resource::{self, opt, req, Resource, K}};

static UNITS: Resource = Resource {
    path: "/units", table: "units", tag: "Catalogue", label: "unit", read: "catalog.read", write: "catalog.write",
    fields: &[req("code", K::Text(12)), req("name", K::Text(60)), opt("name_bn", K::Text(60))],
    search: &["code", "name"], filters: &[], order: "code", extra: "", auto_code: None,
    example: r#"{"code":"ROLL","name":"Roll","name_bn":"রোল"}"#,
};

static CATEGORIES: Resource = Resource {
    path: "/categories", table: "categories", tag: "Catalogue", label: "category", read: "catalog.read", write: "catalog.write",
    fields: &[req("code", K::Text(30)), req("name", K::Text(120)), opt("name_bn", K::Text(120)), opt("parent_id", K::Fk("categories")), opt("is_active", K::Bool)],
    search: &["code", "name", "name_bn"], filters: &["parent_id", "is_active"], order: "code",
    extra: ", (SELECT name FROM categories p WHERE p.id = t.parent_id) AS parent_name", auto_code: None,
    example: r#"{"code":"LED","name":"LED Lighting","name_bn":"এলইডি লাইট"}"#,
};

static HS_CODES: Resource = Resource {
    path: "/hs-codes", table: "hs_codes", tag: "Catalogue", label: "HS code", read: "catalog.read", write: "catalog.write",
    fields: &[req("code", K::Text(16)), req("description", K::Text(400)), opt("cd_rate", K::Dec), opt("rd_rate", K::Dec), opt("sd_rate", K::Dec), opt("vat_rate", K::Dec), opt("ait_rate", K::Dec), opt("at_rate", K::Dec), opt("is_active", K::Bool)],
    search: &["code", "description"], filters: &["is_active"], order: "code", extra: "", auto_code: None,
    example: r#"{"code":"9405.42.00","description":"LED luminaires and lighting fittings","cd_rate":25,"rd_rate":3,"sd_rate":0,"vat_rate":15,"ait_rate":5,"at_rate":5}"#,
};

static PRODUCTS: Resource = Resource {
    path: "/products", table: "products", tag: "Catalogue", label: "product", read: "catalog.read", write: "catalog.write",
    fields: &[req("sku", K::Text(40)), req("name", K::Text(200)), opt("name_bn", K::Text(200)), opt("description", K::Text(1000)), opt("category_id", K::Fk("categories")), opt("unit_id", K::Fk("units")), opt("hs_code_id", K::Fk("hs_codes")),
        opt("product_type", K::Enum(&["goods", "service"])), opt("brand", K::Text(80)), opt("origin_country", K::Text(60)), opt("barcode", K::Text(60)),
        opt("sale_price", K::Dec), opt("min_sale_price", K::Dec), opt("vat_rate", K::Dec), opt("sd_rate", K::Dec), opt("reorder_level", K::Dec), opt("reorder_qty", K::Dec),
        opt("lead_time_days", K::Int), opt("weight_kg", K::Dec), opt("volume_cbm", K::Dec), opt("is_active", K::Bool)],
    search: &["sku", "name", "name_bn", "barcode", "brand"], filters: &["category_id", "is_active", "product_type", "hs_code_id"], order: "sku",
    extra: ", (SELECT name FROM categories c WHERE c.id = t.category_id) AS category_name, (SELECT code FROM units u WHERE u.id = t.unit_id) AS unit_code, (SELECT code FROM hs_codes h WHERE h.id = t.hs_code_id) AS hs_code, (SELECT coalesce(sum(qty), 0) FROM stock_balances b WHERE b.product_id = t.id) AS stock_qty",
    auto_code: None,
    example: r#"{"sku":"LED-60W","name":"LED Panel Light 60W","name_bn":"এলইডি প্যানেল লাইট ৬০ ওয়াট","category_id":"<uuid>","unit_id":"<uuid>","hs_code_id":"<uuid>","sale_price":4200,"min_sale_price":3600,"vat_rate":15,"reorder_level":300,"reorder_qty":1200,"lead_time_days":45,"weight_kg":2.1,"volume_cbm":0.008}"#,
};

static SUPPLIERS: Resource = Resource {
    path: "/suppliers", table: "suppliers", tag: "Procurement", label: "supplier", read: "purchase.read", write: "purchase.write",
    fields: &[req("code", K::Text(30)), req("name", K::Text(200)), opt("supplier_type", K::Enum(&["local", "foreign", "service"])), opt("country", K::Text(60)), opt("currency", K::Code("currencies")),
        opt("bin", K::Text(30)), opt("tin", K::Text(30)), opt("address", K::Text(400)), opt("contact_person", K::Text(120)), opt("phone", K::Text(40)), opt("email", K::Email), opt("bank_details", K::Text(600)),
        opt("payment_terms_days", K::Int), opt("tds_rate", K::Dec), opt("vds_rate", K::Dec), opt("is_active", K::Bool)],
    search: &["code", "name", "contact_person", "phone"], filters: &["supplier_type", "is_active", "currency"], order: "name", extra: "",
    auto_code: Some(("code", "supplier", "SUP")),
    example: r#"{"name":"Shenzhen Lumina Optoelectronics Co. Ltd","supplier_type":"foreign","country":"CN","currency":"USD","contact_person":"Ms. Li","email":"sales@lumina.example","payment_terms_days":0}"#,
};

static CUSTOMERS: Resource = Resource {
    path: "/customers", table: "customers", tag: "Sales", label: "customer", read: "sales.read", write: "sales.write",
    fields: &[req("code", K::Text(30)), req("name", K::Text(200)), opt("name_bn", K::Text(200)), opt("customer_type", K::Enum(&["dealer", "retail", "corporate", "government", "distributor"])),
        opt("bin", K::Text(30)), opt("tin", K::Text(30)), opt("address", K::Text(400)), opt("district", K::Text(60)), opt("contact_person", K::Text(120)), opt("phone", K::Text(40)), opt("email", K::Email),
        opt("credit_limit", K::Dec), opt("payment_terms_days", K::Int), opt("is_active", K::Bool)],
    search: &["code", "name", "name_bn", "phone", "contact_person"], filters: &["customer_type", "is_active", "district"], order: "name",
    extra: ", (SELECT coalesce(sum(i.total - i.paid_amount - i.returned_amount), 0) FROM sales_invoices i WHERE i.customer_id = t.id AND i.status IN ('open','partial')) AS outstanding",
    auto_code: Some(("code", "customer", "CUS")),
    example: r#"{"name":"Apex Electric Store","name_bn":"এপেক্স ইলেকট্রিক স্টোর","customer_type":"dealer","bin":"001234567-0203","address":"Nawabpur Road, Dhaka","district":"Dhaka","phone":"01711000000","credit_limit":1500000,"payment_terms_days":30}"#,
};

static WAREHOUSES: Resource = Resource {
    path: "/warehouses", table: "warehouses", tag: "Inventory", label: "warehouse", read: "inventory.read", write: "admin.write",
    fields: &[req("code", K::Text(20)), req("name", K::Text(120)), opt("name_bn", K::Text(120)), opt("branch_id", K::Fk("branches")), opt("address", K::Text(400)), opt("is_active", K::Bool)],
    search: &["code", "name"], filters: &["branch_id", "is_active"], order: "code",
    extra: ", (SELECT name FROM branches b WHERE b.id = t.branch_id) AS branch_name, (SELECT coalesce(sum(value), 0) FROM stock_balances s WHERE s.warehouse_id = t.id) AS stock_value",
    auto_code: None,
    example: r#"{"code":"CTG-01","name":"Chattogram Depot","branch_id":"<uuid>","address":"Sagorika Road, Chattogram"}"#,
};

pub fn api() -> Api {
    let mut api = Api::new("Catalogue");
    for r in [&UNITS, &CATEGORIES, &HS_CODES, &PRODUCTS, &SUPPLIERS, &CUSTOMERS, &WAREHOUSES] {
        api = resource::routes(api, r);
    }
    api
}
