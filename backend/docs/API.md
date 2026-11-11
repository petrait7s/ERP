# WEFT ERP API reference

Base URL: `/api/v1`. Send `Authorization: Bearer <access_token>` on every call except those marked public.

## Conventions

- Lists: `GET` returns `{"data": [...], "total": n, "page": 1, "per_page": 50}`. Query: `page`, `per_page` (max 500), `q` (search), plus the filters listed per route.
- Single record: `{"data": {...}}`. Create returns HTTP 201.
- Errors: `{"error": {"code", "message", "details": [{"field", "message"}]}}` with status 400, 401, 403, 404, 409 or 422.
- Money is BDT with 2 decimals unless a `currency` field is present. Quantities have up to 4 decimals. Requests accept numbers or numeric strings.
- Dates are `YYYY-MM-DD` (Bangladesh Standard Time, UTC+6). Timestamps are ISO-8601 UTC.
- Permissions are `module.action`. A role may hold exact permissions, `module.*`, `*.read` or `*`.

## Authentication

| Method | Path | Permission | Purpose |
|---|---|---|---|
| POST | `/setup` | `public` | First-run setup: create the first company and its owner |
| POST | `/auth/login` | `public` | Sign in |
| POST | `/auth/refresh` | `public` | Exchange a refresh token for a new session (single use) |
| POST | `/auth/logout` | `auth` | Sign out and revoke the refresh token |
| POST | `/auth/switch-company` | `auth` | Start a session in another company you belong to |
| GET | `/auth/me` | `auth` | Current user, role, permissions and company |
| POST | `/auth/change-password` | `auth` | Change your own password |
| GET | `/meta` | `auth` | Lookups for forms: currencies, enums, permission catalogue |

**POST `/setup`** example body:

```json
{"company_code":"PTC","company_name":"PETRA Trade Corporation","company_name_bn":"পেট্রা ট্রেড কর্পোরেশন","bin":"000123456-0101","tin":"123456789012","address":"Motijheel C/A, Dhaka","admin_email":"owner@example.com","admin_password":"Str0ngPassw0rd","admin_name":"Business Owner"}
```

**POST `/auth/login`** example body:

```json
{"email":"owner@example.com","password":"Str0ngPassw0rd","company_code":"PTC"}
```

**POST `/auth/refresh`** example body:

```json
{"refresh_token":"<token>"}
```

**POST `/auth/logout`** example body:

```json
{"refresh_token":"<token>"}
```

**POST `/auth/switch-company`** example body:

```json
{"company_id":"<uuid>"}
```

**POST `/auth/change-password`** example body:

```json
{"current_password":"old","new_password":"NewStr0ngPass"}
```

## Administration

| Method | Path | Permission | Purpose |
|---|---|---|---|
| GET | `/companies` | `auth` | Companies you belong to |
| POST | `/companies` | `auth` | Add a company (system administrator only) |
| GET | `/company` | `auth` | Current company profile |
| PUT | `/company` | `admin.write` | Update company profile |
| GET | `/users` | `admin.read` | Users of this company Query: `page,per_page,q,role`. |
| POST | `/users` | `admin.write` | Add a user to this company |
| PUT | `/users/:id` | `admin.write` | Update a user's name, role, branch or status |
| POST | `/users/:id/reset-password` | `admin.write` | Set a new password for a user |
| GET | `/roles` | `admin.read` | Roles with their permissions |
| POST | `/roles` | `admin.write` | Create a custom role |
| PUT | `/roles/:id` | `admin.write` | Update a role |
| DELETE | `/roles/:id` | `admin.write` | Delete a custom role |
| GET | `/settings` | `admin.read` | Business policies and their current values |
| PUT | `/settings` | `admin.write` | Change business policies |
| GET | `/sequences` | `admin.read` | Document numbering |
| PUT | `/sequences/:key` | `admin.write` | Change a document prefix or padding |
| GET | `/audit-logs` | `admin.read` | Who did what and when Query: `page,per_page,entity,entity_id,action,user_id,from,to`. |
| GET | `/branches` | `auth` | List branch records Query: `page,per_page,q,sort,order,is_active`. |
| POST | `/branches` | `admin.write` | Create a branch |
| GET | `/branches/:id` | `auth` | Get one branch |
| PUT | `/branches/:id` | `admin.write` | Update a branch (send only the fields to change) |
| DELETE | `/branches/:id` | `admin.write` | Delete a branch (refused when already used) |

**POST `/companies`** example body:

```json
{"code":"PTC2","name":"PETRA Distribution Ltd","bin":"000987654-0101"}
```

**PUT `/company`** example body:

```json
{"address":"Level 8, Motijheel C/A, Dhaka","irc_no":"BA-0123456"}
```

**POST `/users`** example body:

```json
{"email":"accounts@example.com","full_name":"Rahim Uddin","password":"Str0ngPassw0rd","role":"accountant","branch_id":null}
```

**PUT `/users/:id`** example body:

```json
{"role":"manager","is_active":true}
```

**POST `/users/:id/reset-password`** example body:

```json
{"new_password":"NewStr0ngPass1"}
```

**POST `/roles`** example body:

```json
{"code":"store_keeper","name":"Store Keeper","permissions":["inventory.read","inventory.write","catalog.read"]}
```

**PUT `/roles/:id`** example body:

```json
{"permissions":["inventory.*","catalog.read"]}
```

**PUT `/settings`** example body:

```json
{"sales.enforce_credit_limit":true,"customs.landing_charge_pct":1}
```

**PUT `/sequences/:key`** example body:

```json
{"prefix":"PO/25-26","padding":5}
```

**POST `/branches`** example body:

```json
{"code":"CTG","name":"Chattogram Branch","name_bn":"চট্টগ্রাম শাখা","address":"Agrabad C/A, Chattogram"}
```

## Catalogue

| Method | Path | Permission | Purpose |
|---|---|---|---|
| GET | `/units` | `catalog.read` | List unit records Query: `page,per_page,q,sort,order`. |
| POST | `/units` | `catalog.write` | Create a unit |
| GET | `/units/:id` | `catalog.read` | Get one unit |
| PUT | `/units/:id` | `catalog.write` | Update a unit (send only the fields to change) |
| DELETE | `/units/:id` | `catalog.write` | Delete a unit (refused when already used) |
| GET | `/categories` | `catalog.read` | List category records Query: `page,per_page,q,sort,order,parent_id,is_active`. |
| POST | `/categories` | `catalog.write` | Create a category |
| GET | `/categories/:id` | `catalog.read` | Get one category |
| PUT | `/categories/:id` | `catalog.write` | Update a category (send only the fields to change) |
| DELETE | `/categories/:id` | `catalog.write` | Delete a category (refused when already used) |
| GET | `/hs-codes` | `catalog.read` | List HS code records Query: `page,per_page,q,sort,order,is_active`. |
| POST | `/hs-codes` | `catalog.write` | Create a HS code |
| GET | `/hs-codes/:id` | `catalog.read` | Get one HS code |
| PUT | `/hs-codes/:id` | `catalog.write` | Update a HS code (send only the fields to change) |
| DELETE | `/hs-codes/:id` | `catalog.write` | Delete a HS code (refused when already used) |
| GET | `/products` | `catalog.read` | List product records Query: `page,per_page,q,sort,order,category_id,is_active,product_type,hs_code_id`. |
| POST | `/products` | `catalog.write` | Create a product |
| GET | `/products/:id` | `catalog.read` | Get one product |
| PUT | `/products/:id` | `catalog.write` | Update a product (send only the fields to change) |
| DELETE | `/products/:id` | `catalog.write` | Delete a product (refused when already used) |

**POST `/units`** example body:

```json
{"code":"ROLL","name":"Roll","name_bn":"রোল"}
```

**POST `/categories`** example body:

```json
{"code":"LED","name":"LED Lighting","name_bn":"এলইডি লাইট"}
```

**POST `/hs-codes`** example body:

```json
{"code":"9405.42.00","description":"LED luminaires and lighting fittings","cd_rate":25,"rd_rate":3,"sd_rate":0,"vat_rate":15,"ait_rate":5,"at_rate":5}
```

**POST `/products`** example body:

```json
{"sku":"LED-60W","name":"LED Panel Light 60W","name_bn":"এলইডি প্যানেল লাইট ৬০ ওয়াট","category_id":"<uuid>","unit_id":"<uuid>","hs_code_id":"<uuid>","sale_price":4200,"min_sale_price":3600,"vat_rate":15,"reorder_level":300,"reorder_qty":1200,"lead_time_days":45,"weight_kg":2.1,"volume_cbm":0.008}
```

## Procurement

| Method | Path | Permission | Purpose |
|---|---|---|---|
| GET | `/suppliers` | `purchase.read` | List supplier records Query: `page,per_page,q,sort,order,supplier_type,is_active,currency`. |
| POST | `/suppliers` | `purchase.write` | Create a supplier |
| GET | `/suppliers/:id` | `purchase.read` | Get one supplier |
| PUT | `/suppliers/:id` | `purchase.write` | Update a supplier (send only the fields to change) |
| DELETE | `/suppliers/:id` | `purchase.write` | Delete a supplier (refused when already used) |
| GET | `/requisitions` | `purchase.read` | Purchase requisitions Query: `page,per_page,status`. |
| POST | `/requisitions` | `purchase.write` | Ask for items to be bought |
| GET | `/requisitions/:id` | `purchase.read` | One requisition with lines |
| POST | `/requisitions/:id/cancel` | `purchase.write` | Cancel an open requisition |
| GET | `/purchase-orders` | `purchase.read` | Purchase orders Query: `page,per_page,q,status,po_type,supplier_id,from,to`. |
| POST | `/purchase-orders` | `purchase.write` | Create a draft purchase order (local or import) |
| GET | `/purchase-orders/:id` | `purchase.read` | One purchase order with lines and approvals |
| PUT | `/purchase-orders/:id` | `purchase.write` | Replace a draft purchase order |
| POST | `/purchase-orders/:id/submit` | `purchase.write` | Submit for approval (auto-approves when no rule applies) |
| POST | `/purchase-orders/:id/cancel` | `purchase.write` | Cancel an order that has no shipments, receipts or payments |
| POST | `/purchase-orders/:id/close` | `purchase.write` | Close an order; nothing more will be received |
| GET | `/goods-receipts` | `purchase.read` | Goods receipts (GRN) Query: `page,per_page,po_id,supplier_id,unbilled,from,to`. |
| GET | `/goods-receipts/:id` | `purchase.read` | One goods receipt with lines |
| POST | `/goods-receipts` | `inventory.write` | Receive a local purchase order into a warehouse |
| GET | `/supplier-bills` | `purchase.read` | Supplier bills Query: `page,per_page,q,status,supplier_id,unpaid,from,to`. |
| GET | `/supplier-bills/:id` | `purchase.read` | One supplier bill |
| POST | `/supplier-bills` | `purchase.bill` | Record a local supplier's bill against a goods receipt, or a service bill |

**POST `/suppliers`** example body:

```json
{"name":"Shenzhen Lumina Optoelectronics Co. Ltd","supplier_type":"foreign","country":"CN","currency":"USD","contact_person":"Ms. Li","email":"sales@lumina.example","payment_terms_days":0}
```

**POST `/requisitions`** example body:

```json
{"required_date":"2026-12-15","notes":"Stock for winter season","lines":[{"product_id":"<uuid>","qty":1200}]}
```

**POST `/purchase-orders`** example body:

```json
{"po_type":"import","supplier_id":"<uuid>","currency":"USD","fx_rate":121.50,"incoterm":"CFR","payment_mode":"lc","port_of_loading":"Shenzhen","port_of_discharge":"Chattogram","pi_no":"PI-554","pi_date":"2026-11-01","expected_date":"2026-12-28","lines":[{"product_id":"<uuid>","qty":2400,"unit_price":18.5}]}
```

**POST `/goods-receipts`** example body:

```json
{"po_id":"<uuid>","warehouse_id":"<uuid>","receipt_date":"2026-11-12","lines":[{"po_line_id":"<uuid>","qty":100,"batch_no":"B-2231"}]}
```

**POST `/supplier-bills`** example body:

```json
{"supplier_id":"<uuid>","grn_id":"<uuid>","supplier_invoice_no":"INV-889","mushak_no":"6.3-000451","bill_date":"2026-11-12","subtotal":250000,"vat_amount":37500}
```

## Sales

| Method | Path | Permission | Purpose |
|---|---|---|---|
| GET | `/customers` | `sales.read` | List customer records Query: `page,per_page,q,sort,order,customer_type,is_active,district`. |
| POST | `/customers` | `sales.write` | Create a customer |
| GET | `/customers/:id` | `sales.read` | Get one customer |
| PUT | `/customers/:id` | `sales.write` | Update a customer (send only the fields to change) |
| DELETE | `/customers/:id` | `sales.write` | Delete a customer (refused when already used) |
| GET | `/sales-orders` | `sales.read` | Sales orders Query: `page,per_page,q,status,customer_id,from,to`. |
| POST | `/sales-orders` | `sales.write` | Create a draft sales order (prices default from the product) |
| GET | `/sales-orders/:id` | `sales.read` | One order with lines, delivery and invoice progress |
| PUT | `/sales-orders/:id` | `sales.write` | Replace a draft order |
| POST | `/sales-orders/:id/submit` | `sales.write` | Confirm the order (checks credit limit; may need approval) |
| POST | `/sales-orders/:id/cancel` | `sales.write` | Cancel an order with no deliveries |
| POST | `/sales-orders/:id/close` | `sales.write` | Close a delivered or part-delivered order |
| GET | `/deliveries` | `sales.read` | Delivery challans Query: `page,per_page,so_id,customer_id,uninvoiced,from,to`. |
| GET | `/deliveries/:id` | `sales.read` | One delivery |
| POST | `/deliveries` | `inventory.write` | Deliver goods: stock leaves, cost of goods is booked. Set invoice=true to invoice at once |
| GET | `/sales-invoices` | `sales.read` | Sales invoices Query: `page,per_page,q,status,customer_id,unpaid,overdue,from,to`. |
| GET | `/sales-invoices/:id` | `sales.read` | One invoice with lines, receipts and returns |
| GET | `/sales-invoices/:id/mushak-6-3` | `sales.read` | Mushak 6.3 print data |
| POST | `/sales-invoices` | `sales.invoice` | Invoice a delivery (Mushak 6.3 number issued automatically) |
| GET | `/customer-receipts` | `sales.read` | Customer receipts Query: `page,per_page,customer_id,unallocated,from,to`. |
| GET | `/customer-receipts/:id` | `sales.read` | One receipt with allocations |
| POST | `/customer-receipts` | `sales.invoice` | Record money received; VDS and TDS withheld by the customer are booked as assets |
| POST | `/customer-receipts/:id/allocate` | `sales.invoice` | Apply an advance or unallocated receipt to invoices |
| GET | `/sales-returns` | `sales.read` | Credit notes (Mushak 6.7) Query: `page,per_page,invoice_id,from,to`. |
| GET | `/sales-returns/:id` | `sales.read` | One credit note |
| POST | `/sales-returns` | `sales.write` | Return goods against an invoice: credit note, stock back at original cost |
| GET | `/customers/:id/statement` | `sales.read` | Customer ledger statement with running balance Query: `page,per_page,from,to`. |

**POST `/customers`** example body:

```json
{"name":"Apex Electric Store","name_bn":"এপেক্স ইলেকট্রিক স্টোর","customer_type":"dealer","bin":"001234567-0203","address":"Nawabpur Road, Dhaka","district":"Dhaka","phone":"01711000000","credit_limit":1500000,"payment_terms_days":30}
```

**POST `/sales-orders`** example body:

```json
{"customer_id":"<uuid>","warehouse_id":"<uuid>","delivery_date":"2026-12-05","lines":[{"product_id":"<uuid>","qty":120,"unit_price":4200,"discount_pct":2}]}
```

**POST `/deliveries`** example body:

```json
{"so_id":"<uuid>","vehicle_no":"Dhaka Metro-Ta 11-2345","invoice":true,"lines":[{"so_line_id":"<uuid>","qty":120}]}
```

**POST `/sales-invoices`** example body:

```json
{"delivery_id":"<uuid>","invoice_date":"2026-12-02"}
```

**POST `/customer-receipts`** example body:

```json
{"customer_id":"<uuid>","bank_account_id":"<uuid>","gross_amount":500000,"vds_amount":30000,"tds_amount":5000,"method":"bank","reference":"EFT 88231","allocations":[{"invoice_id":"<uuid>","amount":500000}]}
```

**POST `/customer-receipts/:id/allocate`** example body:

```json
{"allocations":[{"invoice_id":"<uuid>","amount":120000}]}
```

**POST `/sales-returns`** example body:

```json
{"invoice_id":"<uuid>","reason":"Damaged in transit","lines":[{"invoice_line_id":"<uuid>","qty":5}]}
```

## Inventory

| Method | Path | Permission | Purpose |
|---|---|---|---|
| GET | `/warehouses` | `inventory.read` | List warehouse records Query: `page,per_page,q,sort,order,branch_id,is_active`. |
| POST | `/warehouses` | `admin.write` | Create a warehouse |
| GET | `/warehouses/:id` | `inventory.read` | Get one warehouse |
| PUT | `/warehouses/:id` | `admin.write` | Update a warehouse (send only the fields to change) |
| DELETE | `/warehouses/:id` | `admin.write` | Delete a warehouse (refused when already used) |
| GET | `/stock` | `inventory.read` | Stock on hand by warehouse and product Query: `page,per_page,q,warehouse_id,product_id,category_id,in_stock,low`. |
| GET | `/stock/moves` | `inventory.read` | Stock ledger: every movement with running balance Query: `page,per_page,warehouse_id,product_id,move_type,source_id,from,to`. |
| GET | `/stock/transfers` | `inventory.read` | Transfers between warehouses Query: `page,per_page,from,to`. |
| GET | `/stock/transfers/:id` | `inventory.read` | One transfer with lines |
| POST | `/stock/transfers` | `inventory.write` | Move stock between warehouses at cost |
| GET | `/stock/adjustments` | `inventory.read` | Stock adjustments Query: `page,per_page,status,from,to`. |
| GET | `/stock/adjustments/:id` | `inventory.read` | One adjustment with lines |
| POST | `/stock/adjustments` | `inventory.write` | Correct stock after a count, damage or loss (may need approval) |
| POST | `/stock/opening` | `finance.write` | Enter opening stock with its cost |

**POST `/warehouses`** example body:

```json
{"code":"CTG-01","name":"Chattogram Depot","branch_id":"<uuid>","address":"Sagorika Road, Chattogram"}
```

**POST `/stock/transfers`** example body:

```json
{"from_warehouse_id":"<uuid>","to_warehouse_id":"<uuid>","transfer_date":"2026-11-12","lines":[{"product_id":"<uuid>","qty":200}]}
```

**POST `/stock/adjustments`** example body:

```json
{"warehouse_id":"<uuid>","reason":"Damaged in handling","lines":[{"product_id":"<uuid>","qty_change":-5}]}
```

**POST `/stock/opening`** example body:

```json
{"warehouse_id":"<uuid>","adjustment_date":"2026-07-01","reason":"Opening stock","lines":[{"product_id":"<uuid>","qty_change":500,"unit_cost":2850.75}]}
```

## Accounting

| Method | Path | Permission | Purpose |
|---|---|---|---|
| GET | `/accounts` | `finance.read` | List account records Query: `page,per_page,q,sort,order,account_type,is_group,is_active,parent_id`. |
| POST | `/accounts` | `finance.write` | Create a account |
| GET | `/accounts/:id` | `finance.read` | Get one account |
| PUT | `/accounts/:id` | `finance.write` | Update a account (send only the fields to change) |
| DELETE | `/accounts/:id` | `finance.write` | Delete a account (refused when already used) |
| GET | `/journals` | `finance.read` | List journal entries Query: `page,per_page,q,source_type,source_id,status,account_id,from,to`. |
| GET | `/journals/:id` | `finance.read` | Journal entry with its lines |
| POST | `/journals` | `finance.post` | Post a manual journal (must balance) |
| POST | `/journals/:id/reverse` | `finance.post` | Reverse a manual journal with a mirror entry |
| GET | `/fiscal-periods` | `finance.read` | Accounting periods |
| POST | `/fiscal-periods/generate` | `finance.post` | Create the monthly periods of a fiscal year |
| POST | `/fiscal-periods/:id/close` | `finance.post` | Close a period (no more postings) |
| POST | `/fiscal-periods/:id/reopen` | `finance.post` | Reopen the latest closed period |

**POST `/accounts`** example body:

```json
{"code":"5340","name":"Marketing Expenses","name_bn":"বিপণন ব্যয়","account_type":"expense","parent_id":"<uuid of 5000>"}
```

**POST `/journals`** example body:

```json
{"entry_date":"2026-07-01","memo":"Owner capital introduced","lines":[{"account_id":"<bank GL uuid>","debit":5000000},{"account_id":"<share capital uuid>","credit":5000000}]}
```

**POST `/journals/:id/reverse`** example body:

```json
{"reason":"Posted to the wrong account"}
```

**POST `/fiscal-periods/generate`** example body:

```json
{"start_year":2027}
```

## Cash & FX

| Method | Path | Permission | Purpose |
|---|---|---|---|
| GET | `/exchange-rates` | `finance.read` | List exchange rate records Query: `page,per_page,q,sort,order,currency,rate_type,rate_date`. |
| POST | `/exchange-rates` | `finance.rates` | Create a exchange rate |
| GET | `/exchange-rates/:id` | `finance.read` | Get one exchange rate |
| PUT | `/exchange-rates/:id` | `finance.rates` | Update a exchange rate (send only the fields to change) |
| DELETE | `/exchange-rates/:id` | `finance.rates` | Delete a exchange rate (refused when already used) |
| GET | `/exchange-rates-latest` | `finance.read` | Latest rate per currency and type |
| GET | `/bank-accounts` | `finance.read` | Bank, cash and mobile accounts with balances Query: `page,per_page,kind,currency`. |
| POST | `/bank-accounts` | `finance.write` | Add a bank or cash account (creates its ledger account) |
| GET | `/bank-accounts/:id` | `finance.read` | One bank or cash account with balance |
| PUT | `/bank-accounts/:id` | `finance.write` | Update bank account details |
| GET | `/bank-accounts/:id/statement` | `finance.read` | Account statement with running balance Query: `page,per_page,from,to`. |
| GET | `/bank-transfers` | `finance.read` | Transfers between own accounts Query: `page,per_page,from,to`. |
| POST | `/bank-transfers` | `finance.write` | Move money between own accounts (any currency) |
| GET | `/supplier-payments` | `finance.read` | Payments to local suppliers Query: `page,per_page,supplier_id,status,from,to`. |
| GET | `/supplier-payments/:id` | `finance.read` | One supplier payment |
| POST | `/supplier-payments` | `finance.write` | Pay supplier bills; TDS and VDS are withheld automatically (may need approval) |
| GET | `/fx/exposure` | `finance.read` | Open foreign-currency exposure by currency, with open LCs |
| POST | `/fx/revalue` | `finance.post` | Revalue foreign payables and FC bank balances at the latest rate |

**POST `/exchange-rates`** example body:

```json
{"currency":"USD","rate_date":"2026-11-10","rate":121.5,"rate_type":"mid","source":"Bangladesh Bank"}
```

**POST `/bank-accounts`** example body:

```json
{"code":"EBL-CA","name":"Eastern Bank Current A/C","kind":"bank","bank_name":"Eastern Bank PLC","branch_name":"Motijheel","account_no":"1011220001234","routing_no":"095274241","currency":"BDT"}
```

**POST `/bank-transfers`** example body:

```json
{"from_bank_account_id":"<uuid>","to_bank_account_id":"<uuid>","transfer_date":"2026-11-12","amount":500000,"charges":115,"reference":"Cash deposit"}
```

**POST `/supplier-payments`** example body:

```json
{"supplier_id":"<uuid>","bank_account_id":"<uuid>","payment_date":"2026-11-20","method":"cheque","reference":"CHQ 004512","allocations":[{"bill_id":"<uuid>","amount":287500}]}
```

**POST `/fx/revalue`** example body:

```json
{"date":"2026-11-30"}
```

## Import: LC & payments

| Method | Path | Permission | Purpose |
|---|---|---|---|
| GET | `/lcs` | `import.read` | Letters of credit Query: `page,per_page,q,status,po_id`. |
| POST | `/lcs` | `import.write` | Draft an LC for an approved import order |
| GET | `/lcs/:id` | `import.read` | One LC with amendments, payments and shipments |
| POST | `/lcs/:id/open` | `import.pay` | Open the LC: bank takes margin and charges |
| POST | `/lcs/:id/amend` | `import.write` | Amend amount, dates or tolerance |
| POST | `/lcs/:id/close` | `import.pay` | Close the LC and release remaining margin |
| GET | `/foreign-payments` | `import.read` | TT and LC payments Query: `page,per_page,kind,status,po_id,shipment_id,from,to`. |
| GET | `/foreign-payments/:id` | `import.read` | One foreign payment |
| POST | `/foreign-payments` | `import.pay` | Pay a foreign supplier: TT advance, TT payment or LC settlement (may need approval) |

**POST `/lcs`** example body:

```json
{"po_id":"<uuid>","bank_account_id":"<uuid>","lc_type":"sight","margin_pct":20,"tolerance_pct":5,"advising_bank":"Bank of China, Shenzhen","expiry_date":"2027-02-28","latest_shipment_date":"2027-01-31"}
```

**POST `/lcs/:id/open`** example body:

```json
{"open_date":"2026-11-15","bank_lc_no":"0000123426010045","fx_rate":121.5,"commission":18500,"swift_charges":3500}
```

**POST `/lcs/:id/amend`** example body:

```json
{"latest_shipment_date":"2027-02-15","expiry_date":"2027-03-15","reason":"Factory delay","charges":2500}
```

**POST `/lcs/:id/close`** example body:

```json
{"close_date":"2027-03-01"}
```

**POST `/foreign-payments`** example body:

```json
{"kind":"lc_settlement","shipment_id":"<uuid>","bank_account_id":"<uuid>","payment_date":"2027-01-20","fc_amount":44400,"fx_rate":122.1,"bank_charges":4200,"reference":"PAD 2027/118"}
```

## Import: shipments

| Method | Path | Permission | Purpose |
|---|---|---|---|
| GET | `/shipments` | `import.read` | Shipments with ETA, progress and missing documents Query: `page,per_page,q,status,po_id,supplier_id,active`. |
| POST | `/shipments` | `import.write` | Book a shipment against an import order |
| GET | `/shipments/:id` | `import.read` | Shipment with lines, containers, tracking, documents, payments and costing |
| PUT | `/shipments/:id` | `import.write` | Update vessel, dates and references |
| POST | `/shipments/:id/depart` | `import.write` | Goods on board: books Goods in Transit and the supplier payable |
| POST | `/shipments/:id/arrive` | `import.write` | Vessel arrived: starts the port free-time clock |
| POST | `/shipments/:id/cancel` | `import.write` | Cancel a booked shipment |
| POST | `/shipments/:id/containers` | `import.write` | Add a container |
| PUT | `/containers/:id` | `import.write` | Update container status and dates |
| POST | `/shipments/:id/events` | `import.write` | Add a tracking event |
| PUT | `/shipments/:id/documents/:doc_type` | `import.write` | Update one document on the checklist |

**POST `/shipments`** example body:

```json
{"po_id":"<uuid>","lc_id":"<uuid>","mode":"sea","vessel_name":"MV Maersk Chattogram","voyage_no":"246E","carrier":"Maersk","etd":"2026-12-05","eta":"2026-12-24","commercial_invoice_no":"LM-CI-2231","freight":1800,"insurance":0,"free_days":4,"demurrage_per_day":2400,"lines":[{"po_line_id":"<uuid>","qty":2400}],"containers":[{"container_no":"MSKU7731450","size":"40HC","seal_no":"ML-889123"}]}
```

**PUT `/shipments/:id`** example body:

```json
{"eta":"2026-12-27","vessel_name":"MV Maersk Chattogram"}
```

**POST `/shipments/:id/depart`** example body:

```json
{"departed_at":"2026-12-05","bl_no":"MAEU226884512","fx_rate":121.8}
```

**POST `/shipments/:id/arrive`** example body:

```json
{"arrived_at":"2026-12-24"}
```

**POST `/shipments/:id/containers`** example body:

```json
{"container_no":"TGHU2208113","size":"20GP"}
```

**PUT `/containers/:id`** example body:

```json
{"delivered_at":"2026-12-29"}
```

**POST `/shipments/:id/events`** example body:

```json
{"event_type":"transshipment","event_time":"2026-12-14T08:00:00Z","location":"Colombo","description":"Transshipped to feeder vessel"}
```

**PUT `/shipments/:id/documents/:doc_type`** example body:

```json
{"doc_no":"COO-88123","doc_date":"2026-12-03","status":"received"}
```

## Import: customs

| Method | Path | Permission | Purpose |
|---|---|---|---|
| POST | `/customs/duty-calculator` | `import.read` | Estimate duties for a CIF value and HS code (nothing is saved) |
| GET | `/bills-of-entry` | `import.read` | Bills of Entry Query: `page,per_page,q,status,shipment_id,from,to`. |
| POST | `/bills-of-entry` | `import.write` | Lodge a Bill of Entry: duties are computed per HS code |
| GET | `/bills-of-entry/:id` | `import.read` | One Bill of Entry with duty per line |
| DELETE | `/bills-of-entry/:id` | `import.write` | Remove an unpaid assessment to lodge it again |
| POST | `/bills-of-entry/:id/pay` | `import.pay` | Pay assessed duties from a bank account |
| POST | `/bills-of-entry/:id/release` | `import.write` | Customs released the goods |

**POST `/customs/duty-calculator`** example body:

```json
{"cif_value":46200,"fx_rate":122.5,"hs_code_id":"<uuid>"}
```

**POST `/bills-of-entry`** example body:

```json
{"shipment_id":"<uuid>","be_no":"C-1284551","be_date":"2026-12-25","cnf_agent":"Karnaphuli C&F Ltd","customs_rate":122.5,"other_fees":3500}
```

**POST `/bills-of-entry/:id/pay`** example body:

```json
{"bank_account_id":"<uuid>","paid_date":"2026-12-26"}
```

**POST `/bills-of-entry/:id/release`** example body:

```json
{"released_date":"2026-12-27"}
```

## Import: landed cost

| Method | Path | Permission | Purpose |
|---|---|---|---|
| GET | `/landed-costs` | `import.read` | Landed cost sheets Query: `page,per_page,status,shipment_id`. |
| POST | `/landed-costs` | `import.write` | Start a landed cost sheet for a shipment |
| GET | `/landed-costs/:id` | `import.read` | Cost sheet: charges, duty, landed unit cost and margin per product |
| PUT | `/landed-costs/:id/method` | `import.write` | Change how shared charges are spread |
| POST | `/landed-costs/:id/charges` | `import.write` | Add a charge (freight, C&F, port, transport...) |
| DELETE | `/landed-costs/:id/charges/:charge_id` | `import.write` | Remove a manually added charge |
| POST | `/landed-costs/:id/finalize` | `import.write` | Lock the costing and post the charges |
| POST | `/shipments/:id/receive` | `inventory.write` | Receive a cleared shipment into a warehouse at landed cost |

**POST `/landed-costs`** example body:

```json
{"shipment_id":"<uuid>","allocation_method":"value"}
```

**PUT `/landed-costs/:id/method`** example body:

```json
{"allocation_method":"weight"}
```

**POST `/landed-costs/:id/charges`** example body:

```json
{"charge_type":"cnf_commission","description":"C&F agent bill 2231","amount":45000,"vat_amount":6750,"supplier_id":"<uuid>"}
```

**POST `/shipments/:id/receive`** example body:

```json
{"warehouse_id":"<uuid>","receipt_date":"2026-12-29","lines":[{"shipment_line_id":"<uuid>","qty":2400,"batch_no":"LM-2231"}]}
```

## HR

| Method | Path | Permission | Purpose |
|---|---|---|---|
| GET | `/departments` | `hr.read` | List department records Query: `page,per_page,q,sort,order,is_active,parent_id`. |
| POST | `/departments` | `hr.write` | Create a department |
| GET | `/departments/:id` | `hr.read` | Get one department |
| PUT | `/departments/:id` | `hr.write` | Update a department (send only the fields to change) |
| DELETE | `/departments/:id` | `hr.write` | Delete a department (refused when already used) |
| GET | `/employees` | `hr.read` | List employee records Query: `page,per_page,q,sort,order,branch_id,department_id,status,employment_type`. |
| POST | `/employees` | `hr.write` | Create a employee |
| GET | `/employees/:id` | `hr.read` | Get one employee |
| PUT | `/employees/:id` | `hr.write` | Update a employee (send only the fields to change) |
| DELETE | `/employees/:id` | `hr.write` | Delete a employee (refused when already used) |
| GET | `/attendance` | `hr.read` | Attendance records Query: `page,per_page,employee_id,status,from,to`. |
| GET | `/attendance/summary` | `hr.read` | Monthly attendance summary per employee Query: `year,month`. |
| POST | `/attendance` | `hr.write` | Mark a day's attendance for many employees |
| GET | `/leave-requests` | `hr.read` | Leave requests Query: `page,per_page,status,employee_id,from`. |
| POST | `/leave-requests` | `hr.write` | Request leave |
| POST | `/leave-requests/:id/decide` | `hr.write` | Approve or reject leave (approved leave is marked in attendance) |

**POST `/departments`** example body:

```json
{"code":"IMP","name":"Import & Logistics","name_bn":"আমদানি ও লজিস্টিকস"}
```

**POST `/employees`** example body:

```json
{"name":"Nusrat Jahan","name_bn":"নুসরাত জাহান","designation":"Import Officer","department_id":"<uuid>","join_date":"2025-07-01","basic_salary":32000,"house_rent":16000,"medical":4000,"conveyance":3000,"pf_rate":10,"monthly_tax":0,"payment_method":"bank","bank_account_no":"1011220009911"}
```

**POST `/attendance`** example body:

```json
{"work_date":"2026-11-10","rows":[{"employee_id":"<uuid>","status":"present","check_in":"09:02:00","check_out":"18:05:00"},{"employee_id":"<uuid>","status":"absent"}]}
```

**POST `/leave-requests`** example body:

```json
{"employee_id":"<uuid>","leave_type":"casual","from_date":"2026-11-15","to_date":"2026-11-17","reason":"Family event"}
```

**POST `/leave-requests/:id/decide`** example body:

```json
{"approve":true}
```

## Payroll

| Method | Path | Permission | Purpose |
|---|---|---|---|
| GET | `/payroll-runs` | `payroll.read` | Payroll runs Query: `page,per_page,status`. |
| POST | `/payroll-runs` | `payroll.write` | Prepare payroll for a month from salaries and attendance |
| GET | `/payroll-runs/:id` | `payroll.read` | Payroll run with payslips |
| DELETE | `/payroll-runs/:id` | `payroll.write` | Delete an unposted payroll run |
| POST | `/payroll-runs/:id/submit` | `payroll.write` | Submit for approval |
| POST | `/payroll-runs/:id/post` | `payroll.write` | Book the salary expense and liabilities |
| POST | `/payroll-runs/:id/pay` | `finance.write` | Pay net salaries from a bank account (finance only) |

**POST `/payroll-runs`** example body:

```json
{"year":2026,"month":11}
```

**POST `/payroll-runs/:id/post`** example body:

```json
{"posting_date":"2026-11-30"}
```

**POST `/payroll-runs/:id/pay`** example body:

```json
{"bank_account_id":"<uuid>","payment_date":"2026-12-01"}
```

## Tax (Bangladesh)

| Method | Path | Permission | Purpose |
|---|---|---|---|
| GET | `/tax/vat-return` | `tax.read` | Live Mushak 9.1 figures for a month Query: `year,month`. |
| GET | `/tax/vat-returns` | `tax.read` | Saved VAT returns |
| POST | `/tax/vat-returns` | `tax.write` | Save the month's VAT return as a draft (re-run to refresh) Query: `year,month`. |
| POST | `/tax/vat-returns/:id/submit` | `tax.write` | Submit the return and book the VAT payable |
| GET | `/tax/mushak-6-1` | `tax.read` | Purchase register Query: `page,per_page,from,to`. |
| GET | `/tax/mushak-6-2` | `tax.read` | Sales register Query: `page,per_page,from,to`. |
| GET | `/tax/mushak-6-10` | `tax.read` | Transactions above the reporting threshold Query: `page,per_page,from,to`. |
| GET | `/tax/liabilities` | `tax.read` | Amounts owed for VAT, SD, VDS, TDS and provident fund |
| POST | `/tax/deposits` | `tax.write` | Deposit VAT, SD, VDS, TDS or PF to the treasury |

**POST `/tax/vat-returns/:id/submit`** example body:

```json
{"submission_date":"2026-12-15"}
```

**POST `/tax/deposits`** example body:

```json
{"kind":"vds","amount":18000,"bank_account_id":"<uuid>","challan_no":"TR-6 0012345"}
```

## Approvals

| Method | Path | Permission | Purpose |
|---|---|---|---|
| GET | `/approvals` | `approvals.read` | Approval requests Query: `page,per_page,status,entity_type,entity_id,mine`. |
| POST | `/approvals/:id/approve` | `approvals.decide` | Approve a pending request |
| POST | `/approvals/:id/reject` | `approvals.decide` | Reject a pending request (comment required) |
| GET | `/approval-rules` | `approvals.read` | Approval rules |
| POST | `/approval-rules` | `admin.write` | Add an approval rule |
| PUT | `/approval-rules/:id` | `admin.write` | Change an approval rule |
| DELETE | `/approval-rules/:id` | `admin.write` | Remove an approval rule |

**POST `/approvals/:id/approve`** example body:

```json
{"comment":"Price matches the proforma"}
```

**POST `/approvals/:id/reject`** example body:

```json
{"comment":"Negotiate a better price first"}
```

**POST `/approval-rules`** example body:

```json
{"entity_type":"purchase_order","min_amount":1000000,"level":1,"approver_role":"manager","description":"POs of Tk 10 lakh or more need a manager"}
```

## Notifications

| Method | Path | Permission | Purpose |
|---|---|---|---|
| GET | `/notifications` | `auth` | Your notifications, newest first Query: `page,per_page,unread,severity`. |
| GET | `/notifications/unread-count` | `auth` | Number of unread notifications |
| POST | `/notifications/:id/read` | `auth` | Mark one notification as read |
| POST | `/notifications/read-all` | `auth` | Mark everything as read |

## Reports & dashboards

| Method | Path | Permission | Purpose |
|---|---|---|---|
| GET | `/dashboard` | `auth` | Role-aware dashboard: only blocks the user may see |
| GET | `/search` | `auth` | Global search across records the user may open Query: `q`. |
| GET | `/reports/trial-balance` | `finance.read` | Trial balance Query: `as_of`. |
| GET | `/reports/income-statement` | `finance.read` | Profit and loss for a period Query: `from,to`. |
| GET | `/reports/balance-sheet` | `finance.read` | Balance sheet Query: `as_of`. |
| GET | `/reports/general-ledger` | `finance.read` | Ledger of one account with running balance Query: `account_id,page,per_page,from,to`. |
| GET | `/reports/receivables-ageing` | `finance.read` | Customer ageing Query: `as_of`. |
| GET | `/reports/payables-ageing` | `finance.read` | Supplier ageing Query: `as_of`. |
| GET | `/reports/stock-valuation` | `inventory.read` | Stock value per product, reconciled to the ledger |
| GET | `/reports/sales-analysis` | `sales.read` | Sales by customer, product (with margin) and day Query: `from,to`. |
| GET | `/reports/import-pipeline` | `import.read` | Shipments in flight, demurrage and LC expiry |
| GET | `/reports/replenishment` | `inventory.read` | Reorder suggestions from cover, lead time and demand |

