#!/usr/bin/env python3
"""Load a realistic Bangladesh importer demo through the public API (so it also exercises it).

Usage: python3 scripts/seed_demo.py [base_url]    default http://127.0.0.1:8080
Creates the company on first run, then master data, one imported shipment (cleared and in stock),
local sales, a receipt, employees and attendance. Safe to run once on an empty system.
"""
import json, sys, urllib.request, urllib.error

BASE = (sys.argv[1] if len(sys.argv) > 1 else "http://127.0.0.1:8080").rstrip("/") + "/api/v1"
TOKEN = None

def call(method, path, body=None, expect=(200, 201)):
    req = urllib.request.Request(BASE + path, method=method, data=json.dumps(body).encode() if body is not None else None)
    req.add_header("Content-Type", "application/json")
    if TOKEN: req.add_header("Authorization", "Bearer " + TOKEN)
    try:
        with urllib.request.urlopen(req) as r: out = json.load(r)
    except urllib.error.HTTPError as e:
        out = json.load(e)
        if e.code not in expect: raise SystemExit(f"{method} {path} -> {e.code}: {json.dumps(out)}")
    return out

def post(path, body): return call("POST", path, body)["data"]

def approve_all():
    # multi-level rules: approving one level opens the next, so loop until nothing is pending
    while True:
        pending = call("GET", "/approvals?status=pending")["data"]
        if not pending: return
        for a in pending: post(f"/approvals/{a['id']}/approve", {"comment": "Approved"})

# --- company and owner ---
try:
    call("POST", "/setup", {"company_code": "MTC", "company_name": "Meghna Trade Corporation", "company_name_bn": "মেঘনা ট্রেড কর্পোরেশন",
        "bin": "000123456-0101", "tin": "123456789012", "address": "Motijheel C/A, Dhaka-1000",
        "admin_email": "owner@meghna.example", "admin_password": "Demo@12345", "admin_name": "Tanvir Ahmed"})
except SystemExit as e:
    print("setup skipped:", str(e)[:80])
TOKEN = call("POST", "/auth/login", {"email": "owner@meghna.example", "password": "Demo@12345"})["data"]["access_token"]
call("PUT", "/company", {"irc_no": "BA-0123456", "trade_license": "TRAD/DNCC/045678", "phone": "+880 2 9555000", "email": "info@meghna.example"})

# --- users for each role ---
for email, name, role in [("manager@meghna.example", "Farhana Rahman", "manager"), ("accounts@meghna.example", "Rahim Uddin", "accountant"),
                          ("import@meghna.example", "Nusrat Jahan", "import_officer"), ("store@meghna.example", "Kamal Hossain", "warehouse"),
                          ("sales@meghna.example", "Sabbir Khan", "sales"), ("hr@meghna.example", "Mitu Akter", "hr")]:
    call("POST", "/users", {"email": email, "full_name": name, "password": "Demo@12345", "role": role}, expect=(200, 201, 409))

# --- banks and capital ---
bank = post("/bank-accounts", {"code": "EBL", "name": "Eastern Bank - Motijheel CA", "bank_name": "Eastern Bank PLC", "branch_name": "Motijheel", "account_no": "1011220001234", "currency": "BDT"})
bkash = post("/bank-accounts", {"code": "BKASH", "name": "bKash Merchant", "kind": "mobile", "currency": "BDT"})
gl = call("GET", f"/bank-accounts/{bank['id']}")["data"]["account_id"]
capital = next(a["id"] for a in call("GET", "/accounts?q=Share%20Capital")["data"])
post("/journals", {"entry_date": "2026-07-01", "memo": "Share capital introduced", "lines": [{"account_id": gl, "debit": 30000000}, {"account_id": capital, "credit": 30000000}]})
for cur, rate in [("USD", "122.00"), ("CNY", "16.80"), ("EUR", "132.50")]:
    post("/exchange-rates", {"currency": cur, "rate_date": "2026-11-01", "rate": rate, "source": "Bangladesh Bank"})

# --- catalogue ---
hs = {c["code"]: post("/hs-codes", c)["id"] for c in [
    {"code": "9405.42.00", "description": "LED luminaires and lighting fittings", "cd_rate": 25, "rd_rate": 3, "sd_rate": 0, "vat_rate": 15, "ait_rate": 5, "at_rate": 5},
    {"code": "8516.10.00", "description": "Electric water heaters and kettles", "cd_rate": 25, "rd_rate": 3, "sd_rate": 0, "vat_rate": 15, "ait_rate": 5, "at_rate": 5},
    {"code": "7318.15.00", "description": "Steel screws and bolts", "cd_rate": 10, "rd_rate": 0, "sd_rate": 0, "vat_rate": 15, "ait_rate": 5, "at_rate": 5}]}
cat = post("/categories", {"code": "LIGHT", "name": "Lighting", "name_bn": "আলোকসজ্জা"})["id"]
cat2 = post("/categories", {"code": "HOME", "name": "Home Appliances", "name_bn": "গৃহস্থালী যন্ত্রপাতি"})["id"]
units = {u["code"]: u["id"] for u in call("GET", "/units?per_page=100")["data"]}
products = {}
for sku, name, bn, c, h, price, floor, lead, wt, vol in [
    ("LED-60W", "LED Panel Light 60W", "এলইডি প্যানেল লাইট ৬০ ওয়াট", cat, "9405.42.00", 4200, 3600, 45, 2.1, 0.008),
    ("LED-STR5", "LED Strip Light 5m", "এলইডি স্ট্রিপ লাইট ৫ মি.", cat, "9405.42.00", 780, 650, 45, 0.3, 0.0012),
    ("KTL-17", "Electric Kettle 1.7L", "ইলেকট্রিক কেটলি ১.৭ লি.", cat2, "8516.10.00", 1950, 1700, 38, 1.1, 0.012),
    ("BOLT-M8", "Steel Bolt M8 (box of 100)", "স্টিল বোল্ট এম৮ (১০০ পিস)", cat2, "7318.15.00", 1450, 1250, 50, 9.0, 0.02)]:
    products[sku] = post("/products", {"sku": sku, "name": name, "name_bn": bn, "category_id": c, "unit_id": units["PCS"], "hs_code_id": hs[h], "sale_price": price, "min_sale_price": floor,
        "vat_rate": 15, "reorder_level": 200, "reorder_qty": 500, "lead_time_days": lead, "weight_kg": wt, "volume_cbm": vol, "origin_country": "CN"})["id"]

sup = post("/suppliers", {"name": "Shenzhen Lumina Optoelectronics Co. Ltd", "supplier_type": "foreign", "country": "CN", "currency": "USD", "contact_person": "Ms. Li Na", "email": "sales@lumina.example"})["id"]
post("/suppliers", {"name": "Ningbo Fasteners Co.", "supplier_type": "foreign", "country": "CN", "currency": "USD"})
cnf = post("/suppliers", {"name": "Karnaphuli C&F Agency", "supplier_type": "service", "payment_terms_days": 7, "tds_rate": 10, "vds_rate": 0})["id"]
post("/suppliers", {"name": "Dhaka Packaging Industries", "supplier_type": "local", "payment_terms_days": 15, "tds_rate": 5, "vds_rate": 15})
customers = [post("/customers", c)["id"] for c in [
    {"name": "Apex Electric Store", "name_bn": "এপেক্স ইলেকট্রিক স্টোর", "customer_type": "dealer", "district": "Dhaka", "phone": "01711000001", "credit_limit": 3000000, "payment_terms_days": 30, "bin": "001234567-0203"},
    {"name": "Chattogram Lighting House", "customer_type": "dealer", "district": "Chattogram", "phone": "01811000002", "credit_limit": 2000000, "payment_terms_days": 21},
    {"name": "Sylhet Home Needs", "customer_type": "retail", "district": "Sylhet", "phone": "01911000003", "credit_limit": 500000, "payment_terms_days": 7}]]
wh = call("GET", "/warehouses")["data"][0]["id"]

# --- import: PO -> approval -> LC -> shipment -> customs -> landed cost -> stock ---
po = post("/purchase-orders", {"supplier_id": sup, "po_type": "import", "currency": "USD", "incoterm": "CFR", "payment_mode": "lc", "order_date": "2026-11-02", "port_of_loading": "Shenzhen", "port_of_discharge": "Chattogram", "pi_no": "PI-554",
    "lines": [{"product_id": products["LED-60W"], "qty": 2400, "unit_price": 18.5}, {"product_id": products["LED-STR5"], "qty": 5000, "unit_price": 3.2}]})
post(f"/purchase-orders/{po['id']}/submit", {})
approve_all()
lc = post("/lcs", {"po_id": po["id"], "bank_account_id": bank["id"], "margin_pct": 20, "tolerance_pct": 5, "expiry_date": "2027-02-28", "latest_shipment_date": "2027-01-31", "advising_bank": "Bank of China, Shenzhen"})
post(f"/lcs/{lc['id']}/open", {"open_date": "2026-11-05", "commission": 38000, "swift_charges": 3500})
lines = call("GET", f"/purchase-orders/{po['id']}")["data"]["lines"]
sh = post("/shipments", {"po_id": po["id"], "lc_id": lc["id"], "vessel_name": "MV Maersk Chattogram", "voyage_no": "246E", "carrier": "Maersk", "etd": "2026-11-10", "eta": "2026-12-01", "freight": 2600, "free_days": 4, "demurrage_per_day": 2400,
    "lines": [{"po_line_id": l["id"], "qty": l["qty"]} for l in lines], "containers": [{"container_no": "MSKU7731450", "size": "40HC", "seal_no": "ML-889123"}]})
post(f"/shipments/{sh['id']}/depart", {"departed_at": "2026-11-10", "bl_no": "MAEU226884512"})
post(f"/shipments/{sh['id']}/events", {"event_type": "transshipment", "location": "Colombo", "description": "Transshipped to feeder vessel"})
post(f"/shipments/{sh['id']}/arrive", {"arrived_at": "2026-12-01"})
be = post("/bills-of-entry", {"shipment_id": sh["id"], "be_no": "C-1284551", "be_date": "2026-12-02", "customs_rate": "122.50", "cnf_agent": "Karnaphuli C&F Agency", "other_fees": 3500})
post(f"/bills-of-entry/{be['id']}/pay", {"bank_account_id": bank["id"], "paid_date": "2026-12-02"})
post(f"/bills-of-entry/{be['id']}/release", {"released_date": "2026-12-03"})
lcs = post("/landed-costs", {"shipment_id": sh["id"], "costing_date": "2026-12-03"})
post(f"/landed-costs/{lcs['id']}/charges", {"charge_type": "cnf_commission", "description": "C&F agent bill", "amount": 85000, "vat_amount": 12750, "supplier_id": cnf})
post(f"/landed-costs/{lcs['id']}/charges", {"charge_type": "port_charges", "description": "Port and THC", "amount": 42000, "bank_account_id": bank["id"]})
post(f"/landed-costs/{lcs['id']}/finalize", {})
sl = call("GET", f"/shipments/{sh['id']}")["data"]["lines"]
post(f"/shipments/{sh['id']}/receive", {"warehouse_id": wh, "receipt_date": "2026-12-04", "lines": [{"shipment_line_id": l["id"], "qty": l["qty"], "batch_no": "LM-2231"} for l in sl]})
approve_all()

# a second shipment still at sea, so tracking screens have something live
po2 = post("/purchase-orders", {"supplier_id": sup, "po_type": "import", "currency": "USD", "incoterm": "FOB", "payment_mode": "tt", "order_date": "2026-11-08",
    "lines": [{"product_id": products["KTL-17"], "qty": 1200, "unit_price": 9.4}]})
post(f"/purchase-orders/{po2['id']}/submit", {})
approve_all()
l2 = call("GET", f"/purchase-orders/{po2['id']}")["data"]["lines"][0]
sh2 = post("/shipments", {"po_id": po2["id"], "vessel_name": "MV Evergreen Hope", "etd": "2026-11-15", "eta": "2026-12-12", "lines": [{"po_line_id": l2["id"], "qty": 1200}], "containers": [{"container_no": "CMAU9904123", "size": "40HC"}]})

# --- sales ---
so = post("/sales-orders", {"customer_id": customers[0], "warehouse_id": wh, "order_date": "2026-12-08", "lines": [{"product_id": products["LED-60W"], "qty": 400}, {"product_id": products["LED-STR5"], "qty": 600}]})
post(f"/sales-orders/{so['id']}/submit", {})
sol = call("GET", f"/sales-orders/{so['id']}")["data"]["lines"]
dl = post("/deliveries", {"so_id": so["id"], "delivery_date": "2026-12-09", "invoice": True, "vehicle_no": "Dhaka Metro-Ta 11-2345", "lines": [{"so_line_id": l["id"], "qty": l["qty"]} for l in sol]})
inv = call("GET", f"/sales-invoices/{dl['invoice_id']}")["data"]
post("/customer-receipts", {"customer_id": customers[0], "bank_account_id": bank["id"], "receipt_date": "2026-12-15", "gross_amount": 1000000, "vds_amount": 60000, "allocations": [{"invoice_id": inv["id"], "amount": 1000000}]})
so2 = post("/sales-orders", {"customer_id": customers[1], "warehouse_id": wh, "order_date": "2026-12-10", "lines": [{"product_id": products["LED-60W"], "qty": 250}]})
post(f"/sales-orders/{so2['id']}/submit", {})

# --- people ---
dep = post("/departments", {"code": "IMP", "name": "Import & Logistics", "name_bn": "আমদানি ও লজিস্টিকস"})["id"]
for name, bn, desig, basic in [("Nusrat Jahan", "নুসরাত জাহান", "Import Officer", 32000), ("Kamal Hossain", "কামাল হোসেন", "Store Keeper", 22000), ("Sabbir Khan", "সাব্বির খান", "Sales Executive", 28000), ("Rahim Uddin", "রহিম উদ্দিন", "Accountant", 35000)]:
    post("/employees", {"name": name, "name_bn": bn, "designation": desig, "department_id": dep, "join_date": "2025-07-01", "basic_salary": basic, "house_rent": basic // 2, "medical": 3000, "conveyance": 2500, "pf_rate": 10})
emps = call("GET", "/employees")["data"]
post("/attendance", {"work_date": "2026-12-09", "rows": [{"employee_id": e["id"], "status": "present", "check_in": "09:05:00", "check_out": "18:00:00"} for e in emps]})

print("Demo data loaded. Sign in as owner@meghna.example / Demo@12345 (company MTC).")
