//! English and Bangla UI text. Unknown keys fall back to English, so a missing translation never breaks a screen.
use crate::state::is_bn;

pub fn t(key: &str) -> String {
    if is_bn() { if let Some(v) = bn(key) { return v.to_string(); } }
    key.to_string()
}

fn bn(key: &str) -> Option<&'static str> {
    Some(match key {
        // navigation
        "Dashboard" => "ড্যাশবোর্ড", "Imports & Voyages" => "আমদানি ও জাহাজ", "Shipments" => "চালান", "Letters of Credit" => "ঋণপত্র (এলসি)", "Foreign Payments" => "বৈদেশিক পরিশোধ",
        "Landed Cost" => "ল্যান্ডেড কস্ট", "Customs" => "শুল্ক", "Bills of Entry" => "বিল অব এন্ট্রি", "HS Codes" => "এইচএস কোড", "Replenishment" => "পুনঃসরবরাহ",
        "Procurement" => "ক্রয়", "Purchase Orders" => "ক্রয় আদেশ", "Requisitions" => "ক্রয় চাহিদা", "Goods Receipts" => "পণ্য গ্রহণ", "Supplier Bills" => "সরবরাহকারী বিল", "Suppliers" => "সরবরাহকারী",
        "Order Book" => "অর্ডার বুক", "Inventory & Warehouse" => "মজুদ ও গুদাম", "Stock" => "মজুদ", "Stock Ledger" => "মজুদ খতিয়ান", "Transfers" => "স্থানান্তর", "Adjustments" => "সমন্বয়", "Warehouses" => "গুদাম",
        "Sales & Customers" => "বিক্রয় ও ক্রেতা", "Customers" => "ক্রেতা", "Sales Orders" => "বিক্রয় আদেশ", "Deliveries" => "ডেলিভারি", "Invoices" => "চালান (ইনভয়েস)", "Receipts" => "প্রাপ্তি", "Returns" => "ফেরত",
        "Ledger Studio" => "লেজার স্টুডিও", "Accounting & Finance" => "হিসাব ও অর্থ", "Chart of Accounts" => "হিসাবের তালিকা", "Bank & Cash" => "ব্যাংক ও নগদ", "Supplier Payments" => "সরবরাহকারী পরিশোধ", "Periods" => "হিসাব মাস",
        "Cash & FX" => "নগদ ও বিদেশি মুদ্রা", "Exchange Rates" => "বিনিময় হার", "HR & Payroll" => "মানবসম্পদ ও বেতন", "Employees" => "কর্মী", "Attendance" => "উপস্থিতি", "Leave" => "ছুটি", "Payroll" => "বেতন", "Departments" => "বিভাগ",
        "Policy & Approvals" => "নীতি ও অনুমোদন", "Approvals" => "অনুমোদন", "Approval Rules" => "অনুমোদনের নিয়ম", "Business Policies" => "ব্যবসায়িক নীতি",
        "Reports & Analytics" => "প্রতিবেদন ও বিশ্লেষণ", "VAT & Tax" => "মূসক ও কর", "Administration" => "প্রশাসন", "Users" => "ব্যবহারকারী", "Roles" => "ভূমিকা", "Branches" => "শাখা", "Company" => "প্রতিষ্ঠান", "Audit Log" => "অডিট লগ",
        "Products" => "পণ্য", "Categories" => "বিভাগ", "Units" => "একক", "Operations" => "কার্যক্রম", "Finance" => "অর্থ", "People" => "মানুষ", "System" => "সিস্টেম", "Overview" => "সারসংক্ষেপ",
        // common
        "Search" => "খুঁজুন", "Save" => "সংরক্ষণ", "Cancel" => "বাতিল", "Create" => "তৈরি করুন", "New" => "নতুন", "Edit" => "সম্পাদনা", "Delete" => "মুছুন", "Back" => "পিছনে", "Status" => "অবস্থা", "Total" => "মোট",
        "Date" => "তারিখ", "Amount" => "পরিমাণ", "Name" => "নাম", "Code" => "কোড", "Actions" => "কার্যক্রম", "Sign in" => "প্রবেশ করুন", "Sign out" => "বের হন", "Email" => "ইমেইল", "Password" => "পাসওয়ার্ড",
        "Notifications" => "বিজ্ঞপ্তি", "Mark all as read" => "সব পড়া হয়েছে", "Loading" => "লোড হচ্ছে", "Nothing here yet" => "এখনও কিছু নেই", "Confirm" => "নিশ্চিত করুন", "Approve" => "অনুমোদন", "Reject" => "প্রত্যাখ্যান",
        "Good morning" => "শুভ সকাল", "Good afternoon" => "শুভ অপরাহ্ণ", "Good evening" => "শুভ সন্ধ্যা", "Cash & bank" => "নগদ ও ব্যাংক", "Receivables" => "প্রাপ্য", "Payables" => "প্রদেয়",
        "Sales this month" => "এ মাসের বিক্রয়", "Gross margin" => "মোট মুনাফার হার", "Goods in transit" => "পথে থাকা পণ্য", "Stock value" => "মজুদের মূল্য", "Needs your attention" => "আপনার মনোযোগ প্রয়োজন",
        "Open orders" => "চলমান অর্ডার", "Low stock items" => "কম মজুদের পণ্য", "Waiting for approval" => "অনুমোদনের অপেক্ষায়", "Overdue" => "মেয়াদোত্তীর্ণ", "Arriving soon" => "শীঘ্রই আসছে",
        "Search everything…" => "সবকিছু খুঁজুন…", "Customer" => "ক্রেতা", "Supplier" => "সরবরাহকারী", "Product" => "পণ্য", "Quantity" => "পরিমাণ", "Price" => "দাম", "Balance" => "অবশিষ্ট", "Description" => "বিবরণ",
        _ => return None,
    })
}
