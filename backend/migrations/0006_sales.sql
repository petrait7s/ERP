-- Sales, delivery, invoicing (Mushak 6.3), receipts, returns (Mushak 6.7)
CREATE TABLE sales_orders (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  company_id   uuid NOT NULL REFERENCES companies(id),
  branch_id    uuid REFERENCES branches(id),
  so_no        text NOT NULL,
  customer_id  uuid NOT NULL REFERENCES customers(id),
  warehouse_id uuid NOT NULL REFERENCES warehouses(id),
  order_date   date NOT NULL,
  delivery_date date,
  salesperson_id uuid,
  subtotal     numeric(18,2) NOT NULL DEFAULT 0,
  discount     numeric(18,2) NOT NULL DEFAULT 0,
  sd_amount    numeric(18,2) NOT NULL DEFAULT 0,
  vat_amount   numeric(18,2) NOT NULL DEFAULT 0,
  total        numeric(18,2) NOT NULL DEFAULT 0,
  status       text NOT NULL DEFAULT 'draft' CHECK (status IN ('draft','pending_approval','confirmed','partially_delivered','delivered','closed','cancelled','rejected')),
  notes        text,
  created_by   uuid REFERENCES users(id),
  created_at   timestamptz NOT NULL DEFAULT now(),
  UNIQUE (company_id, so_no)
);
CREATE TABLE sales_order_lines (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  so_id        uuid NOT NULL REFERENCES sales_orders(id) ON DELETE CASCADE,
  line_no      int NOT NULL,
  product_id   uuid NOT NULL REFERENCES products(id),
  qty          numeric(18,4) NOT NULL CHECK (qty > 0),
  unit_price   numeric(18,6) NOT NULL CHECK (unit_price >= 0),
  discount_pct numeric(9,4) NOT NULL DEFAULT 0 CHECK (discount_pct BETWEEN 0 AND 100),
  sd_rate      numeric(9,4) NOT NULL DEFAULT 0,
  vat_rate     numeric(9,4) NOT NULL DEFAULT 0,
  net_amount   numeric(18,2) NOT NULL,     -- after discount, before SD and VAT
  sd_amount    numeric(18,2) NOT NULL DEFAULT 0,
  vat_amount   numeric(18,2) NOT NULL DEFAULT 0,
  line_total   numeric(18,2) NOT NULL,
  delivered_qty numeric(18,4) NOT NULL DEFAULT 0,
  invoiced_qty numeric(18,4) NOT NULL DEFAULT 0
);
CREATE INDEX sales_order_lines_so_idx ON sales_order_lines (so_id);

CREATE TABLE deliveries (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  company_id   uuid NOT NULL REFERENCES companies(id),
  branch_id    uuid REFERENCES branches(id),
  delivery_no  text NOT NULL,              -- delivery challan
  so_id        uuid NOT NULL REFERENCES sales_orders(id),
  customer_id  uuid NOT NULL REFERENCES customers(id),
  warehouse_id uuid NOT NULL REFERENCES warehouses(id),
  delivery_date date NOT NULL,
  vehicle_no   text,
  driver_name  text,
  cogs_value   numeric(18,2) NOT NULL DEFAULT 0,
  invoiced     boolean NOT NULL DEFAULT false,
  notes        text,
  journal_entry_id uuid REFERENCES journal_entries(id),
  created_by   uuid REFERENCES users(id),
  created_at   timestamptz NOT NULL DEFAULT now(),
  UNIQUE (company_id, delivery_no)
);
CREATE TABLE delivery_lines (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  delivery_id  uuid NOT NULL REFERENCES deliveries(id) ON DELETE CASCADE,
  so_line_id   uuid NOT NULL REFERENCES sales_order_lines(id),
  product_id   uuid NOT NULL REFERENCES products(id),
  qty          numeric(18,4) NOT NULL CHECK (qty > 0),
  cost_value   numeric(18,2) NOT NULL DEFAULT 0
);

CREATE TABLE sales_invoices (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  company_id   uuid NOT NULL REFERENCES companies(id),
  branch_id    uuid REFERENCES branches(id),
  invoice_no   text NOT NULL,
  mushak_no    text NOT NULL,              -- Mushak 6.3 serial
  so_id        uuid REFERENCES sales_orders(id),
  delivery_id  uuid REFERENCES deliveries(id),
  customer_id  uuid NOT NULL REFERENCES customers(id),
  invoice_date date NOT NULL,
  due_date     date NOT NULL,
  subtotal     numeric(18,2) NOT NULL,     -- net of discount, before SD and VAT
  sd_amount    numeric(18,2) NOT NULL DEFAULT 0,
  vat_amount   numeric(18,2) NOT NULL DEFAULT 0,
  total        numeric(18,2) NOT NULL,
  paid_amount  numeric(18,2) NOT NULL DEFAULT 0,
  returned_amount numeric(18,2) NOT NULL DEFAULT 0,
  status       text NOT NULL DEFAULT 'open' CHECK (status IN ('open','partial','paid','cancelled')),
  notes        text,
  journal_entry_id uuid REFERENCES journal_entries(id),
  created_by   uuid REFERENCES users(id),
  created_at   timestamptz NOT NULL DEFAULT now(),
  UNIQUE (company_id, invoice_no),
  UNIQUE (company_id, mushak_no),
  CHECK (paid_amount + returned_amount <= total)
);
CREATE TABLE sales_invoice_lines (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  invoice_id   uuid NOT NULL REFERENCES sales_invoices(id) ON DELETE CASCADE,
  so_line_id   uuid REFERENCES sales_order_lines(id),
  product_id   uuid NOT NULL REFERENCES products(id),
  qty          numeric(18,4) NOT NULL CHECK (qty > 0),
  unit_price   numeric(18,6) NOT NULL,
  discount_pct numeric(9,4) NOT NULL DEFAULT 0,
  sd_rate      numeric(9,4) NOT NULL DEFAULT 0,
  vat_rate     numeric(9,4) NOT NULL DEFAULT 0,
  net_amount   numeric(18,2) NOT NULL,
  sd_amount    numeric(18,2) NOT NULL DEFAULT 0,
  vat_amount   numeric(18,2) NOT NULL DEFAULT 0,
  line_total   numeric(18,2) NOT NULL,
  cost_value   numeric(18,2) NOT NULL DEFAULT 0,
  returned_qty numeric(18,4) NOT NULL DEFAULT 0
);

CREATE TABLE customer_receipts (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  company_id   uuid NOT NULL REFERENCES companies(id),
  receipt_no   text NOT NULL,
  customer_id  uuid NOT NULL REFERENCES customers(id),
  bank_account_id uuid NOT NULL REFERENCES bank_accounts(id),
  receipt_date date NOT NULL,
  gross_amount numeric(18,2) NOT NULL CHECK (gross_amount > 0),  -- settles receivables
  vds_amount   numeric(18,2) NOT NULL DEFAULT 0 CHECK (vds_amount >= 0),   -- VAT withheld by customer (Mushak 6.6 received)
  tds_amount   numeric(18,2) NOT NULL DEFAULT 0 CHECK (tds_amount >= 0),   -- income tax withheld by customer
  net_amount   numeric(18,2) NOT NULL,
  method       text NOT NULL DEFAULT 'bank' CHECK (method IN ('bank','cash','cheque','bkash','other')),
  reference    text,
  unallocated  numeric(18,2) NOT NULL DEFAULT 0,
  journal_entry_id uuid REFERENCES journal_entries(id),
  created_by   uuid REFERENCES users(id),
  created_at   timestamptz NOT NULL DEFAULT now(),
  UNIQUE (company_id, receipt_no)
);
CREATE TABLE customer_receipt_allocations (
  receipt_id   uuid NOT NULL REFERENCES customer_receipts(id) ON DELETE CASCADE,
  invoice_id   uuid NOT NULL REFERENCES sales_invoices(id),
  amount       numeric(18,2) NOT NULL CHECK (amount > 0),
  PRIMARY KEY (receipt_id, invoice_id)
);

CREATE TABLE sales_returns (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  company_id   uuid NOT NULL REFERENCES companies(id),
  return_no    text NOT NULL,              -- credit note, Mushak 6.7
  invoice_id   uuid NOT NULL REFERENCES sales_invoices(id),
  customer_id  uuid NOT NULL REFERENCES customers(id),
  warehouse_id uuid NOT NULL REFERENCES warehouses(id),
  return_date  date NOT NULL,
  reason       text NOT NULL,
  subtotal     numeric(18,2) NOT NULL,
  sd_amount    numeric(18,2) NOT NULL DEFAULT 0,
  vat_amount   numeric(18,2) NOT NULL DEFAULT 0,
  total        numeric(18,2) NOT NULL,
  cost_value   numeric(18,2) NOT NULL DEFAULT 0,
  journal_entry_id uuid REFERENCES journal_entries(id),
  created_by   uuid REFERENCES users(id),
  created_at   timestamptz NOT NULL DEFAULT now(),
  UNIQUE (company_id, return_no)
);
CREATE TABLE sales_return_lines (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  return_id    uuid NOT NULL REFERENCES sales_returns(id) ON DELETE CASCADE,
  invoice_line_id uuid NOT NULL REFERENCES sales_invoice_lines(id),
  product_id   uuid NOT NULL REFERENCES products(id),
  qty          numeric(18,4) NOT NULL CHECK (qty > 0),
  net_amount   numeric(18,2) NOT NULL,
  sd_amount    numeric(18,2) NOT NULL DEFAULT 0,
  vat_amount   numeric(18,2) NOT NULL DEFAULT 0,
  cost_value   numeric(18,2) NOT NULL DEFAULT 0
);
