-- Inventory (weighted average cost per warehouse) and procurement
CREATE TABLE stock_balances (
  company_id   uuid NOT NULL REFERENCES companies(id),
  warehouse_id uuid NOT NULL REFERENCES warehouses(id),
  product_id   uuid NOT NULL REFERENCES products(id),
  qty          numeric(18,4) NOT NULL DEFAULT 0,
  value        numeric(18,2) NOT NULL DEFAULT 0,
  avg_cost     numeric(18,6) NOT NULL DEFAULT 0,
  updated_at   timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (company_id, warehouse_id, product_id)
);

CREATE TABLE stock_moves (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  company_id   uuid NOT NULL REFERENCES companies(id),
  warehouse_id uuid NOT NULL REFERENCES warehouses(id),
  product_id   uuid NOT NULL REFERENCES products(id),
  move_date    date NOT NULL,
  move_type    text NOT NULL CHECK (move_type IN ('receipt','issue','transfer_in','transfer_out','adjustment_in','adjustment_out','sales_return','purchase_return','opening')),
  qty          numeric(18,4) NOT NULL CHECK (qty <> 0),   -- signed: + in, - out
  unit_cost    numeric(18,6) NOT NULL,
  value        numeric(18,2) NOT NULL,                    -- signed
  balance_qty  numeric(18,4) NOT NULL,
  balance_value numeric(18,2) NOT NULL,
  source_type  text NOT NULL,
  source_id    uuid,
  batch_no     text,
  created_by   uuid REFERENCES users(id),
  created_at   timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX stock_moves_product_idx ON stock_moves (company_id, product_id, move_date);
CREATE INDEX stock_moves_source_idx ON stock_moves (company_id, source_type, source_id);
CREATE TRIGGER stock_moves_immutable BEFORE UPDATE OR DELETE ON stock_moves FOR EACH ROW EXECUTE FUNCTION forbid_change();

CREATE TABLE stock_transfers (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  company_id   uuid NOT NULL REFERENCES companies(id),
  transfer_no  text NOT NULL,
  transfer_date date NOT NULL,
  from_warehouse_id uuid NOT NULL REFERENCES warehouses(id),
  to_warehouse_id   uuid NOT NULL REFERENCES warehouses(id),
  status       text NOT NULL DEFAULT 'posted' CHECK (status IN ('posted')),
  notes        text,
  created_by   uuid REFERENCES users(id),
  created_at   timestamptz NOT NULL DEFAULT now(),
  UNIQUE (company_id, transfer_no),
  CHECK (from_warehouse_id <> to_warehouse_id)
);
CREATE TABLE stock_transfer_lines (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  transfer_id  uuid NOT NULL REFERENCES stock_transfers(id) ON DELETE CASCADE,
  product_id   uuid NOT NULL REFERENCES products(id),
  qty          numeric(18,4) NOT NULL CHECK (qty > 0),
  value        numeric(18,2) NOT NULL DEFAULT 0
);

CREATE TABLE stock_adjustments (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  company_id   uuid NOT NULL REFERENCES companies(id),
  adjustment_no text NOT NULL,
  adjustment_date date NOT NULL,
  warehouse_id uuid NOT NULL REFERENCES warehouses(id),
  reason       text NOT NULL,
  is_opening   boolean NOT NULL DEFAULT false,   -- opening stock: contra account is Opening Balance Equity
  total_value  numeric(18,2) NOT NULL DEFAULT 0,
  status       text NOT NULL DEFAULT 'posted' CHECK (status IN ('pending_approval','posted','rejected')),
  journal_entry_id uuid REFERENCES journal_entries(id),
  created_by   uuid REFERENCES users(id),
  created_at   timestamptz NOT NULL DEFAULT now(),
  UNIQUE (company_id, adjustment_no)
);
CREATE TABLE stock_adjustment_lines (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  adjustment_id uuid NOT NULL REFERENCES stock_adjustments(id) ON DELETE CASCADE,
  product_id   uuid NOT NULL REFERENCES products(id),
  qty_change   numeric(18,4) NOT NULL CHECK (qty_change <> 0),
  unit_cost    numeric(18,6),          -- required for positive adjustments of items with no cost
  value        numeric(18,2) NOT NULL DEFAULT 0
);

CREATE TABLE purchase_requisitions (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  company_id   uuid NOT NULL REFERENCES companies(id),
  branch_id    uuid REFERENCES branches(id),
  requisition_no text NOT NULL,
  request_date date NOT NULL,
  required_date date,
  status       text NOT NULL DEFAULT 'open' CHECK (status IN ('open','ordered','cancelled')),
  notes        text,
  created_by   uuid REFERENCES users(id),
  created_at   timestamptz NOT NULL DEFAULT now(),
  UNIQUE (company_id, requisition_no)
);
CREATE TABLE purchase_requisition_lines (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  requisition_id uuid NOT NULL REFERENCES purchase_requisitions(id) ON DELETE CASCADE,
  product_id   uuid NOT NULL REFERENCES products(id),
  qty          numeric(18,4) NOT NULL CHECK (qty > 0),
  notes        text
);

CREATE TABLE purchase_orders (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  company_id   uuid NOT NULL REFERENCES companies(id),
  branch_id    uuid REFERENCES branches(id),
  po_no        text NOT NULL,
  po_type      text NOT NULL DEFAULT 'local' CHECK (po_type IN ('local','import')),
  supplier_id  uuid NOT NULL REFERENCES suppliers(id),
  requisition_id uuid REFERENCES purchase_requisitions(id),
  order_date   date NOT NULL,
  expected_date date,
  currency     text NOT NULL DEFAULT 'BDT' REFERENCES currencies(code),
  fx_rate      numeric(18,6) NOT NULL DEFAULT 1 CHECK (fx_rate > 0),
  incoterm     text CHECK (incoterm IN ('EXW','FOB','CFR','CIF','CPT','CIP','DAP','DDP')),
  payment_mode text NOT NULL DEFAULT 'credit' CHECK (payment_mode IN ('credit','cash','lc','tt')),
  port_of_loading text,
  port_of_discharge text,
  pi_no        text,                 -- supplier proforma invoice
  pi_date      date,
  subtotal     numeric(18,2) NOT NULL DEFAULT 0,
  discount     numeric(18,2) NOT NULL DEFAULT 0,
  vat_amount   numeric(18,2) NOT NULL DEFAULT 0,
  total        numeric(18,2) NOT NULL DEFAULT 0,       -- in PO currency
  total_bdt    numeric(18,2) NOT NULL DEFAULT 0,
  status       text NOT NULL DEFAULT 'draft' CHECK (status IN ('draft','pending_approval','approved','partially_received','received','closed','cancelled','rejected')),
  notes        text,
  created_by   uuid REFERENCES users(id),
  approved_by  uuid REFERENCES users(id),
  approved_at  timestamptz,
  created_at   timestamptz NOT NULL DEFAULT now(),
  UNIQUE (company_id, po_no)
);
CREATE TABLE purchase_order_lines (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  po_id        uuid NOT NULL REFERENCES purchase_orders(id) ON DELETE CASCADE,
  line_no      int NOT NULL,
  product_id   uuid NOT NULL REFERENCES products(id),
  description  text,
  qty          numeric(18,4) NOT NULL CHECK (qty > 0),
  unit_price   numeric(18,6) NOT NULL CHECK (unit_price >= 0),
  vat_rate     numeric(9,4) NOT NULL DEFAULT 0,
  line_total   numeric(18,2) NOT NULL,       -- qty * unit_price (excl. VAT), PO currency
  shipped_qty  numeric(18,4) NOT NULL DEFAULT 0,
  received_qty numeric(18,4) NOT NULL DEFAULT 0,
  billed_qty   numeric(18,4) NOT NULL DEFAULT 0
);
CREATE INDEX purchase_order_lines_po_idx ON purchase_order_lines (po_id);

CREATE TABLE goods_receipts (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  company_id   uuid NOT NULL REFERENCES companies(id),
  branch_id    uuid REFERENCES branches(id),
  grn_no       text NOT NULL,
  po_id        uuid NOT NULL REFERENCES purchase_orders(id),
  shipment_id  uuid,                 -- set for import receipts (FK added in import migration)
  warehouse_id uuid NOT NULL REFERENCES warehouses(id),
  receipt_date date NOT NULL,
  total_value  numeric(18,2) NOT NULL DEFAULT 0,
  billed       boolean NOT NULL DEFAULT false,
  notes        text,
  journal_entry_id uuid REFERENCES journal_entries(id),
  created_by   uuid REFERENCES users(id),
  created_at   timestamptz NOT NULL DEFAULT now(),
  UNIQUE (company_id, grn_no)
);
CREATE TABLE goods_receipt_lines (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  grn_id       uuid NOT NULL REFERENCES goods_receipts(id) ON DELETE CASCADE,
  po_line_id   uuid NOT NULL REFERENCES purchase_order_lines(id),
  product_id   uuid NOT NULL REFERENCES products(id),
  qty          numeric(18,4) NOT NULL CHECK (qty > 0),
  unit_cost    numeric(18,6) NOT NULL,      -- BDT
  value        numeric(18,2) NOT NULL,
  batch_no     text
);

CREATE TABLE supplier_bills (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  company_id   uuid NOT NULL REFERENCES companies(id),
  branch_id    uuid REFERENCES branches(id),
  bill_no      text NOT NULL,
  supplier_id  uuid NOT NULL REFERENCES suppliers(id),
  po_id        uuid REFERENCES purchase_orders(id),
  grn_id       uuid REFERENCES goods_receipts(id),
  supplier_invoice_no text,
  mushak_no    text,                 -- supplier's Mushak 6.3 number (input VAT evidence)
  bill_date    date NOT NULL,
  due_date     date NOT NULL,
  subtotal     numeric(18,2) NOT NULL,
  vat_amount   numeric(18,2) NOT NULL DEFAULT 0,
  total        numeric(18,2) NOT NULL,
  paid_amount  numeric(18,2) NOT NULL DEFAULT 0,
  status       text NOT NULL DEFAULT 'open' CHECK (status IN ('open','partial','paid','cancelled')),
  expense_account_id uuid REFERENCES accounts(id),  -- for service/expense bills without a GRN
  notes        text,
  journal_entry_id uuid REFERENCES journal_entries(id),
  created_by   uuid REFERENCES users(id),
  created_at   timestamptz NOT NULL DEFAULT now(),
  UNIQUE (company_id, bill_no),
  CHECK (paid_amount <= total)
);

CREATE TABLE supplier_payments (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  company_id   uuid NOT NULL REFERENCES companies(id),
  payment_no   text NOT NULL,
  supplier_id  uuid NOT NULL REFERENCES suppliers(id),
  bank_account_id uuid NOT NULL REFERENCES bank_accounts(id),
  payment_date date NOT NULL,
  gross_amount numeric(18,2) NOT NULL CHECK (gross_amount > 0),   -- settles payables
  tds_amount   numeric(18,2) NOT NULL DEFAULT 0 CHECK (tds_amount >= 0),
  vds_amount   numeric(18,2) NOT NULL DEFAULT 0 CHECK (vds_amount >= 0),
  net_amount   numeric(18,2) NOT NULL,                            -- paid from bank
  method       text NOT NULL DEFAULT 'bank' CHECK (method IN ('bank','cash','cheque','bkash','other')),
  reference    text,
  status       text NOT NULL DEFAULT 'posted' CHECK (status IN ('pending_approval','posted','rejected')),
  journal_entry_id uuid REFERENCES journal_entries(id),
  created_by   uuid REFERENCES users(id),
  created_at   timestamptz NOT NULL DEFAULT now(),
  UNIQUE (company_id, payment_no)
);
CREATE TABLE supplier_payment_allocations (
  payment_id   uuid NOT NULL REFERENCES supplier_payments(id) ON DELETE CASCADE,
  bill_id      uuid NOT NULL REFERENCES supplier_bills(id),
  amount       numeric(18,2) NOT NULL CHECK (amount > 0),
  PRIMARY KEY (payment_id, bill_id)
);
