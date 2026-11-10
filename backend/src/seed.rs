//! Company bootstrap: everything a new Bangladesh company needs on day one
//! (roles, chart of accounts, numbering, policies, approval rules, periods).
use crate::{auth, error::{AppError, AppResult}};
use chrono::{Datelike, NaiveDate};
use serde_json::{json, Value};
use sqlx::PgConnection;
use uuid::Uuid;

pub struct NewCompany<'a> {
    pub code: &'a str,
    pub name: &'a str,
    pub name_bn: Option<&'a str>,
    pub bin: Option<&'a str>,
    pub tin: Option<&'a str>,
    pub address: Option<&'a str>,
}

/// Role templates: (code, name, description, permissions).
pub const ROLES: &[(&str, &str, &str, &[&str])] = &[
    ("owner", "Business Owner", "Full access to everything", &["*"]),
    ("admin", "Administrator", "Users, roles, settings and master data", &["admin.*", "catalog.*", "*.read"]),
    ("manager", "Manager", "Sees everything, approves documents", &["*.read", "approvals.decide", "purchase.write", "sales.write", "catalog.write"]),
    ("accountant", "Accountant", "Ledger, payments, VAT and reports", &["finance.*", "tax.*", "payroll.read", "purchase.read", "purchase.bill", "sales.read", "sales.invoice", "import.read", "import.pay", "inventory.read", "catalog.read", "hr.read", "approvals.read"]),
    ("import_officer", "Import Officer", "Purchase orders, LC, shipments, customs, landed cost", &["import.*", "purchase.read", "purchase.write", "catalog.read", "inventory.read", "finance.rates", "approvals.read"]),
    ("warehouse", "Warehouse Staff", "Receiving, deliveries, transfers and stock counts", &["inventory.*", "catalog.read", "purchase.read", "sales.read", "import.read"]),
    ("sales", "Sales Staff", "Customers, orders and invoices", &["sales.read", "sales.write", "sales.invoice", "catalog.read", "inventory.read", "approvals.read"]),
    ("hr", "HR Officer", "Employees, attendance and payroll", &["hr.*", "payroll.read", "payroll.write", "approvals.read"]),
    ("viewer", "Viewer", "Read-only access", &["*.read"]),
];

/// Every permission the API checks, for the role editor.
pub const PERMISSIONS: &[(&str, &str)] = &[
    ("admin.read", "View users, roles, settings, audit log"), ("admin.write", "Manage users, roles, branches and settings"),
    ("catalog.read", "View products, categories, units, HS codes"), ("catalog.write", "Manage products, categories, units, HS codes"),
    ("purchase.read", "View suppliers, requisitions, purchase orders, bills"), ("purchase.write", "Create and edit suppliers, requisitions and purchase orders"), ("purchase.bill", "Record supplier bills"),
    ("import.read", "View LCs, shipments, customs and landed cost"), ("import.write", "Manage LCs, shipments, tracking, customs and landed cost"), ("import.pay", "Post LC and TT payments and customs duty payments"),
    ("inventory.read", "View warehouses and stock"), ("inventory.write", "Receive goods, deliver, transfer and adjust stock"),
    ("sales.read", "View customers, orders, invoices"), ("sales.write", "Create customers, orders and returns"), ("sales.invoice", "Issue invoices (Mushak 6.3) and record receipts"),
    ("finance.read", "View accounts, ledger, banks and financial reports"), ("finance.write", "Manage accounts, banks, transfers, supplier payments"), ("finance.post", "Post manual journals, reverse entries, close periods"), ("finance.rates", "Maintain exchange rates"),
    ("tax.read", "View VAT registers and returns"), ("tax.write", "Prepare and submit VAT returns"),
    ("hr.read", "View employees, departments, attendance"), ("hr.write", "Manage employees, departments, attendance, leave"),
    ("payroll.read", "View payroll"), ("payroll.write", "Prepare, post and pay payroll"),
    ("approvals.read", "View approval requests"), ("approvals.decide", "Approve or reject (when holding the approver role)"),
    ("reports.read", "View dashboards and operational reports"),
];

/// Chart of accounts for a Bangladesh trading/importing company:
/// (code, name, Bangla name, type, parent code, is_group, system key).
pub const COA: &[(&str, &str, &str, &str, Option<&str>, bool, Option<&str>)] = &[
    ("1000", "Assets", "সম্পদ", "asset", None, true, None),
    ("1010", "Cash in Hand", "হাতে নগদ", "asset", Some("1000"), false, Some("cash")),
    ("1020", "Bank Accounts", "ব্যাংক হিসাব", "asset", Some("1000"), true, Some("bank_parent")),
    ("1100", "Accounts Receivable", "প্রাপ্য হিসাব", "asset", Some("1000"), false, Some("ar")),
    ("1150", "VDS Receivable (VAT deducted by customers)", "উৎসে কর্তিত মূসক প্রাপ্য", "asset", Some("1000"), false, Some("vds_receivable")),
    ("1200", "Inventory", "মজুদ পণ্য", "asset", Some("1000"), false, Some("inventory")),
    ("1210", "Goods in Transit", "পথে থাকা পণ্য", "asset", Some("1000"), false, Some("git")),
    ("1220", "LC Margin Deposits", "এলসি মার্জিন", "asset", Some("1000"), false, Some("lc_margin")),
    ("1230", "Advance to Suppliers", "সরবরাহকারীকে অগ্রিম", "asset", Some("1000"), false, Some("supplier_advance")),
    ("1300", "Input VAT (VAT Current Account)", "উপকরণ কর", "asset", Some("1000"), false, Some("vat_input")),
    ("1310", "Advance Tax (AT) on Imports", "আগাম কর", "asset", Some("1000"), false, Some("at_paid")),
    ("1320", "Advance Income Tax (AIT)", "অগ্রিম আয়কর", "asset", Some("1000"), false, Some("ait_paid")),
    ("1500", "Fixed Assets", "স্থায়ী সম্পদ", "asset", Some("1000"), false, None),
    ("2000", "Liabilities", "দায়", "liability", None, true, None),
    ("2010", "Accounts Payable - Local", "প্রদেয় হিসাব - স্থানীয়", "liability", Some("2000"), false, Some("ap_local")),
    ("2020", "Accounts Payable - Foreign", "প্রদেয় হিসাব - বৈদেশিক", "liability", Some("2000"), false, Some("ap_foreign")),
    ("2030", "Goods Received Not Invoiced", "বিলবিহীন প্রাপ্ত পণ্য", "liability", Some("2000"), false, Some("grni")),
    ("2040", "Accrued Expenses", "বকেয়া খরচ", "liability", Some("2000"), false, Some("accrued_expenses")),
    ("2100", "Output VAT", "উৎপাদ কর", "liability", Some("2000"), false, Some("vat_output")),
    ("2110", "Supplementary Duty Payable", "সম্পূরক শুল্ক প্রদেয়", "liability", Some("2000"), false, Some("sd_payable")),
    ("2120", "VAT Payable to Treasury", "সরকারি কোষাগারে প্রদেয় মূসক", "liability", Some("2000"), false, Some("vat_payable")),
    ("2130", "VDS Payable (VAT deducted from suppliers)", "উৎসে কর্তিত মূসক প্রদেয়", "liability", Some("2000"), false, Some("vds_payable")),
    ("2140", "TDS Payable (tax deducted at source)", "উৎসে কর প্রদেয়", "liability", Some("2000"), false, Some("tds_payable")),
    ("2200", "Salary Payable", "বেতন প্রদেয়", "liability", Some("2000"), false, Some("salary_payable")),
    ("2210", "Provident Fund Payable", "ভবিষ্য তহবিল প্রদেয়", "liability", Some("2000"), false, Some("pf_payable")),
    ("2300", "Bank Loans (LTR / UPAS)", "ব্যাংক ঋণ", "liability", Some("2000"), false, Some("bank_loans")),
    ("3000", "Equity", "মালিকানা স্বত্ব", "equity", None, true, None),
    ("3010", "Share Capital", "শেয়ার মূলধন", "equity", Some("3000"), false, Some("capital")),
    ("3020", "Retained Earnings", "সংরক্ষিত আয়", "equity", Some("3000"), false, Some("retained_earnings")),
    ("3030", "Opening Balance Equity", "প্রারম্ভিক জের", "equity", Some("3000"), false, Some("opening_equity")),
    ("4000", "Income", "আয়", "income", None, true, None),
    ("4010", "Sales Revenue", "বিক্রয় আয়", "income", Some("4000"), false, Some("sales")),
    ("4020", "Sales Returns", "বিক্রয় ফেরত", "income", Some("4000"), false, Some("sales_returns")),
    ("4090", "Other Income", "অন্যান্য আয়", "income", Some("4000"), false, Some("other_income")),
    ("5000", "Expenses", "ব্যয়", "expense", None, true, None),
    ("5010", "Cost of Goods Sold", "বিক্রীত পণ্যের ব্যয়", "expense", Some("5000"), false, Some("cogs")),
    ("5020", "Inventory Adjustments", "মজুদ সমন্বয়", "expense", Some("5000"), false, Some("inventory_adjustment")),
    ("5100", "Salaries and Allowances", "বেতন ও ভাতা", "expense", Some("5000"), false, Some("salary_expense")),
    ("5110", "Employer PF Contribution", "নিয়োগকর্তার ভবিষ্য তহবিল", "expense", Some("5000"), false, Some("pf_expense")),
    ("5200", "Bank Charges", "ব্যাংক চার্জ", "expense", Some("5000"), false, Some("bank_charges")),
    ("5210", "Foreign Exchange Loss / (Gain)", "বৈদেশিক মুদ্রা ক্ষতি/(লাভ)", "expense", Some("5000"), false, Some("fx_gain_loss")),
    ("5300", "Office and Administrative Expenses", "অফিস ও প্রশাসনিক ব্যয়", "expense", Some("5000"), false, Some("admin_expense")),
    ("5310", "Rent", "ভাড়া", "expense", Some("5000"), false, None),
    ("5320", "Utilities", "ইউটিলিটি", "expense", Some("5000"), false, None),
    ("5330", "Carriage Outward", "পরিবহন ব্যয়", "expense", Some("5000"), false, None),
];

pub const SEQUENCES: &[(&str, &str)] = &[
    ("purchase_order", "PO"), ("requisition", "PR"), ("goods_receipt", "GRN"), ("supplier_bill", "BILL"), ("supplier_payment", "PAY"),
    ("lc", "LC"), ("shipment", "SHP"), ("foreign_payment", "FP"), ("landed_cost", "LCS"),
    ("sales_order", "SO"), ("delivery", "DC"), ("sales_invoice", "INV"), ("mushak_63", "M63"), ("customer_receipt", "RCT"), ("sales_return", "CN"),
    ("journal", "JV"), ("bank_transfer", "TRF"), ("stock_transfer", "ST"), ("stock_adjustment", "ADJ"), ("payroll", "PAYROLL"),
    ("customer", "CUS"), ("supplier", "SUP"), ("employee", "EMP"),
];

/// Policy registry: (key, kind, default, description).
pub fn policies() -> Vec<(&'static str, &'static str, Value, &'static str)> {
    vec![
        ("sales.enforce_credit_limit", "bool", json!(true), "Stop new orders when a customer would exceed their credit limit"),
        ("sales.block_below_min_price", "bool", json!(true), "Stop order lines priced below the product's minimum sale price"),
        ("inventory.allow_negative_stock", "bool", json!(false), "Allow stock to go below zero"),
        ("customs.landing_charge_pct", "number", json!(1), "Landing charge added to CIF to get the assessable value (%)"),
        ("customs.default_insurance_pct", "number", json!(1), "Insurance assumed for customs when not declared (% of C&F)"),
        ("tax.standard_vat_rate", "number", json!(15), "Standard VAT rate (%)"),
        ("tax.mushak_610_threshold", "number", json!(200000), "Mushak 6.10 lists invoices above this amount (BDT)"),
        ("auth.max_failed_logins", "number", json!(5), "Failed sign-ins before the account is locked"),
        ("auth.lockout_minutes", "number", json!(15), "Minutes an account stays locked"),
        ("import.default_free_days", "number", json!(4), "Free days at port before demurrage starts"),
        ("replenishment.safety_days", "number", json!(10), "Extra days of stock to keep on top of supplier lead time"),
        ("hr.working_days_per_month", "number", json!(26), "Working days used for payroll when attendance is incomplete"),
    ]
}

fn fiscal_year_start(today: NaiveDate, start_month: u32) -> NaiveDate {
    let year = if today.month() >= start_month { today.year() } else { today.year() - 1 };
    NaiveDate::from_ymd_opt(year, start_month, 1).unwrap()
}

/// Create a company with all defaults and make `owner` its Business Owner.
pub async fn create_company(conn: &mut PgConnection, c: NewCompany<'_>, owner: Uuid) -> AppResult<Uuid> {
    if c.code.trim().is_empty() || c.name.trim().is_empty() {
        return Err(AppError::field("name", "Company code and name are required"));
    }
    let company: Uuid = sqlx::query_scalar("INSERT INTO companies (code, name, name_bn, legal_name, bin, tin, address) VALUES ($1,$2,$3,$2,$4,$5,$6) RETURNING id")
        .bind(c.code.trim().to_uppercase()).bind(c.name.trim()).bind(c.name_bn).bind(c.bin).bind(c.tin).bind(c.address)
        .fetch_one(&mut *conn).await?;
    let branch: Uuid = sqlx::query_scalar("INSERT INTO branches (company_id, code, name, name_bn, is_head_office) VALUES ($1,'HO','Head Office','প্রধান কার্যালয়',true) RETURNING id")
        .bind(company).fetch_one(&mut *conn).await?;

    let mut owner_role = None;
    for (code, name, desc, perms) in ROLES {
        let id: Uuid = sqlx::query_scalar("INSERT INTO roles (company_id, code, name, description, is_system, permissions) VALUES ($1,$2,$3,$4,true,$5) RETURNING id")
            .bind(company).bind(code).bind(name).bind(desc).bind(perms.iter().map(|s| s.to_string()).collect::<Vec<_>>())
            .fetch_one(&mut *conn).await?;
        if *code == "owner" { owner_role = Some(id); }
    }
    let first: bool = sqlx::query_scalar("SELECT NOT EXISTS (SELECT 1 FROM user_companies WHERE user_id = $1)").bind(owner).fetch_one(&mut *conn).await?;
    sqlx::query("INSERT INTO user_companies (user_id, company_id, role_id, is_default) VALUES ($1,$2,$3,$4)")
        .bind(owner).bind(company).bind(owner_role.unwrap()).bind(first).execute(&mut *conn).await?;

    for (code, name, name_bn, kind, _, is_group, key) in COA {
        sqlx::query("INSERT INTO accounts (company_id, code, name, name_bn, account_type, is_group, system_key) VALUES ($1,$2,$3,$4,$5,$6,$7)")
            .bind(company).bind(code).bind(name).bind(name_bn).bind(kind).bind(is_group).bind(key).execute(&mut *conn).await?;
    }
    sqlx::query("UPDATE accounts a SET parent_id = p.id FROM accounts p, (SELECT * FROM unnest($2::text[], $3::text[]) AS m(code, parent)) m WHERE a.company_id = $1 AND p.company_id = $1 AND a.code = m.code AND p.code = m.parent")
        .bind(company)
        .bind(COA.iter().filter(|a| a.4.is_some()).map(|a| a.0.to_string()).collect::<Vec<_>>())
        .bind(COA.iter().filter(|a| a.4.is_some()).map(|a| a.4.unwrap().to_string()).collect::<Vec<_>>())
        .execute(&mut *conn).await?;

    for (key, prefix) in SEQUENCES {
        sqlx::query("INSERT INTO sequences (company_id, key, prefix) VALUES ($1,$2,$3)").bind(company).bind(key).bind(prefix).execute(&mut *conn).await?;
    }
    for (code, name, bn) in [("PCS", "Piece", "পিস"), ("KG", "Kilogram", "কেজি"), ("MT", "Metric Ton", "মেট্রিক টন"), ("LTR", "Litre", "লিটার"), ("MTR", "Metre", "মিটার"), ("BOX", "Box", "বক্স"), ("CTN", "Carton", "কার্টন"), ("SET", "Set", "সেট"), ("DOZ", "Dozen", "ডজন"), ("BAG", "Bag", "ব্যাগ")] {
        sqlx::query("INSERT INTO units (company_id, code, name, name_bn) VALUES ($1,$2,$3,$4)").bind(company).bind(code).bind(name).bind(bn).execute(&mut *conn).await?;
    }
    for (key, _, default, _) in policies() {
        sqlx::query("INSERT INTO settings (company_id, key, value) VALUES ($1,$2,$3)").bind(company).bind(key).bind(default).execute(&mut *conn).await?;
    }
    sqlx::query("INSERT INTO warehouses (company_id, branch_id, code, name, name_bn) VALUES ($1,$2,'MAIN','Main Warehouse','প্রধান গুদাম')").bind(company).bind(branch).execute(&mut *conn).await?;
    sqlx::query("INSERT INTO bank_accounts (company_id, branch_id, account_id, code, name, kind, currency) SELECT $1, $2, id, 'CASH', 'Cash in Hand', 'cash', 'BDT' FROM accounts WHERE company_id = $1 AND system_key = 'cash'")
        .bind(company).bind(branch).execute(&mut *conn).await?;

    // Default approval ladder; amounts are BDT and fully editable.
    for (entity, min, level, role, desc) in [
        ("purchase_order", 500_000i64, 1, "manager", "Purchase orders of Tk 5 lakh or more need a manager"),
        ("purchase_order", 5_000_000, 2, "owner", "Purchase orders of Tk 50 lakh or more also need the owner"),
        ("supplier_payment", 1_000_000, 1, "owner", "Supplier payments of Tk 10 lakh or more need the owner"),
        ("foreign_payment", 2_000_000, 1, "owner", "Foreign payments of Tk 20 lakh or more need the owner"),
        ("payroll_run", 0, 1, "owner", "Every payroll needs the owner's approval"),
        ("stock_adjustment", 100_000, 1, "manager", "Stock write-offs of Tk 1 lakh or more need a manager"),
    ] {
        sqlx::query("INSERT INTO approval_rules (company_id, entity_type, min_amount, level, approver_role, description) VALUES ($1,$2,$3,$4,$5,$6)")
            .bind(company).bind(entity).bind(rust_decimal::Decimal::from(min)).bind(level).bind(role).bind(desc).execute(&mut *conn).await?;
    }

    // Monthly periods for the current fiscal year (July to June by default).
    let today: NaiveDate = sqlx::query_scalar("SELECT (now() AT TIME ZONE 'Asia/Dhaka')::date").fetch_one(&mut *conn).await?;
    let mut start = fiscal_year_start(today, 7);
    for _ in 0..12 {
        let next = if start.month() == 12 { NaiveDate::from_ymd_opt(start.year() + 1, 1, 1) } else { NaiveDate::from_ymd_opt(start.year(), start.month() + 1, 1) }.unwrap();
        sqlx::query("INSERT INTO fiscal_periods (company_id, name, start_date, end_date) VALUES ($1,$2,$3,$4)")
            .bind(company).bind(start.format("%b %Y").to_string()).bind(start).bind(next.pred_opt().unwrap()).execute(&mut *conn).await?;
        start = next;
    }
    sqlx::query("INSERT INTO audit_logs (company_id, user_id, action, entity, entity_id, detail) VALUES ($1,$2,'create','companies',$1::text,$3)")
        .bind(company).bind(owner).bind(json!({"code": c.code, "name": c.name})).execute(&mut *conn).await?;
    Ok(company)
}

pub async fn create_user(conn: &mut PgConnection, email: &str, password: &str, full_name: &str, superadmin: bool) -> AppResult<Uuid> {
    let email = email.trim().to_lowercase();
    if !email.contains('@') { return Err(AppError::field("email", "Must be a valid email address")); }
    if full_name.trim().is_empty() { return Err(AppError::field("full_name", "This field is required")); }
    auth::validate_password(password)?;
    let hash = auth::hash_password(password)?;
    Ok(sqlx::query_scalar("INSERT INTO users (email, password_hash, full_name, is_superadmin) VALUES ($1,$2,$3,$4) RETURNING id")
        .bind(email).bind(hash).bind(full_name.trim()).bind(superadmin).fetch_one(&mut *conn).await?)
}
