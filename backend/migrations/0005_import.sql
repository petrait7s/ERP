-- Import: LC, foreign payments, shipments, containers, customs, landed cost
CREATE TABLE letters_of_credit (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  company_id   uuid NOT NULL REFERENCES companies(id),
  lc_no        text NOT NULL,
  bank_lc_no   text,
  po_id        uuid NOT NULL REFERENCES purchase_orders(id),
  supplier_id  uuid NOT NULL REFERENCES suppliers(id),
  bank_account_id uuid NOT NULL REFERENCES bank_accounts(id),   -- issuing bank account (BDT)
  advising_bank text,
  lc_type      text NOT NULL DEFAULT 'sight' CHECK (lc_type IN ('sight','usance','deferred','upas')),
  currency     text NOT NULL REFERENCES currencies(code),
  amount       numeric(18,2) NOT NULL CHECK (amount > 0),
  tolerance_pct numeric(9,4) NOT NULL DEFAULT 0,
  margin_pct   numeric(9,4) NOT NULL DEFAULT 0 CHECK (margin_pct BETWEEN 0 AND 100),
  margin_bdt   numeric(18,2) NOT NULL DEFAULT 0,      -- margin currently held by bank
  utilized_amount numeric(18,2) NOT NULL DEFAULT 0,   -- FC value of documents negotiated
  settled_amount  numeric(18,2) NOT NULL DEFAULT 0,   -- FC paid to supplier via bank
  open_date    date,
  expiry_date  date,
  latest_shipment_date date,
  usance_days  int NOT NULL DEFAULT 0,
  insurance_cover_note text,
  status       text NOT NULL DEFAULT 'draft' CHECK (status IN ('draft','opened','settled','expired','cancelled')),
  notes        text,
  created_by   uuid REFERENCES users(id),
  created_at   timestamptz NOT NULL DEFAULT now(),
  UNIQUE (company_id, lc_no)
);
CREATE TABLE lc_amendments (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  lc_id        uuid NOT NULL REFERENCES letters_of_credit(id) ON DELETE CASCADE,
  amendment_no int NOT NULL,
  amendment_date date NOT NULL,
  changes      jsonb NOT NULL,
  reason       text,
  created_by   uuid REFERENCES users(id),
  created_at   timestamptz NOT NULL DEFAULT now(),
  UNIQUE (lc_id, amendment_no)
);

CREATE TABLE shipments (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  company_id   uuid NOT NULL REFERENCES companies(id),
  branch_id    uuid REFERENCES branches(id),
  shipment_no  text NOT NULL,
  po_id        uuid NOT NULL REFERENCES purchase_orders(id),
  lc_id        uuid REFERENCES letters_of_credit(id),
  supplier_id  uuid NOT NULL REFERENCES suppliers(id),
  mode         text NOT NULL DEFAULT 'sea' CHECK (mode IN ('sea','air','land')),
  bl_no        text,                 -- bill of lading / airway bill / truck receipt
  bl_date      date,
  vessel_name  text,
  voyage_no    text,
  carrier      text,
  port_of_loading text,
  port_of_discharge text NOT NULL DEFAULT 'Chattogram',
  etd          date,
  eta          date,
  departed_at  date,
  arrived_at   date,
  commercial_invoice_no text,
  commercial_invoice_date date,
  currency     text NOT NULL REFERENCES currencies(code),
  fx_rate      numeric(18,6) NOT NULL CHECK (fx_rate > 0),    -- booking rate (BDT per FC)
  goods_value  numeric(18,2) NOT NULL DEFAULT 0,              -- FC, sum of lines
  freight      numeric(18,2) NOT NULL DEFAULT 0,              -- FC, payable to supplier (CFR/CIF)
  insurance    numeric(18,2) NOT NULL DEFAULT 0,              -- FC, payable to supplier (CIF)
  invoice_value numeric(18,2) NOT NULL DEFAULT 0,             -- FC payable to supplier
  invoice_bdt  numeric(18,2) NOT NULL DEFAULT 0,
  free_days    int NOT NULL DEFAULT 4,
  demurrage_per_day numeric(18,2) NOT NULL DEFAULT 0,         -- BDT per container per day
  status       text NOT NULL DEFAULT 'booked' CHECK (status IN ('booked','in_transit','arrived','customs','cleared','received','cancelled')),
  notes        text,
  booking_journal_id uuid REFERENCES journal_entries(id),
  created_by   uuid REFERENCES users(id),
  created_at   timestamptz NOT NULL DEFAULT now(),
  UNIQUE (company_id, shipment_no)
);
CREATE TABLE shipment_lines (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  shipment_id  uuid NOT NULL REFERENCES shipments(id) ON DELETE CASCADE,
  po_line_id   uuid NOT NULL REFERENCES purchase_order_lines(id),
  product_id   uuid NOT NULL REFERENCES products(id),
  hs_code_id   uuid REFERENCES hs_codes(id),
  qty          numeric(18,4) NOT NULL CHECK (qty > 0),
  unit_price   numeric(18,6) NOT NULL,                  -- FC
  line_value   numeric(18,2) NOT NULL,                  -- FC
  received_qty numeric(18,4) NOT NULL DEFAULT 0,
  received_value numeric(18,2) NOT NULL DEFAULT 0       -- BDT landed value already received
);
ALTER TABLE goods_receipts ADD CONSTRAINT goods_receipts_shipment_fk FOREIGN KEY (shipment_id) REFERENCES shipments(id);

CREATE TABLE containers (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  shipment_id  uuid NOT NULL REFERENCES shipments(id) ON DELETE CASCADE,
  container_no text NOT NULL,
  size         text NOT NULL DEFAULT '40HC' CHECK (size IN ('20GP','40GP','40HC','45HC','LCL','AIR','TRUCK')),
  seal_no      text,
  gross_weight_kg numeric(18,2),
  packages     int,
  status       text NOT NULL DEFAULT 'loaded' CHECK (status IN ('loaded','at_sea','discharged','at_depot','delivered','returned')),
  discharged_at date,
  free_until   date,
  delivered_at date,
  returned_at  date,
  UNIQUE (shipment_id, container_no)
);

CREATE TABLE tracking_events (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  shipment_id  uuid NOT NULL REFERENCES shipments(id) ON DELETE CASCADE,
  container_id uuid REFERENCES containers(id) ON DELETE CASCADE,
  event_time   timestamptz NOT NULL,
  event_type   text NOT NULL CHECK (event_type IN ('booked','gate_in','loaded','departed','transshipment','arrived','discharged','customs_hold','customs_released','gate_out','delivered','empty_returned','delay','note')),
  location     text,
  description  text,
  source       text NOT NULL DEFAULT 'manual',
  created_by   uuid REFERENCES users(id),
  created_at   timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX tracking_events_shipment_idx ON tracking_events (shipment_id, event_time);

CREATE TABLE import_documents (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  shipment_id  uuid NOT NULL REFERENCES shipments(id) ON DELETE CASCADE,
  doc_type     text NOT NULL CHECK (doc_type IN ('PI','CI','PL','BL','COO','LC','LCA','INS','IP','BSTI','PHYTO','TEST','BE','DO','OTHER')),
  doc_no       text,
  doc_date     date,
  status       text NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','requested','received','verified','submitted')),
  is_required  boolean NOT NULL DEFAULT true,
  file_url     text,
  notes        text,
  updated_by   uuid REFERENCES users(id),
  updated_at   timestamptz NOT NULL DEFAULT now(),
  UNIQUE (shipment_id, doc_type)
);

CREATE TABLE foreign_payments (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  company_id   uuid NOT NULL REFERENCES companies(id),
  payment_no   text NOT NULL,
  kind         text NOT NULL CHECK (kind IN ('tt_advance','tt_payment','lc_margin','lc_settlement','lc_charges')),
  po_id        uuid REFERENCES purchase_orders(id),
  lc_id        uuid REFERENCES letters_of_credit(id),
  shipment_id  uuid REFERENCES shipments(id),
  supplier_id  uuid REFERENCES suppliers(id),
  bank_account_id uuid NOT NULL REFERENCES bank_accounts(id),
  payment_date date NOT NULL,
  currency     text NOT NULL REFERENCES currencies(code),
  fc_amount    numeric(18,2) NOT NULL DEFAULT 0 CHECK (fc_amount >= 0),
  fx_rate      numeric(18,6) NOT NULL DEFAULT 1 CHECK (fx_rate > 0),
  bdt_amount   numeric(18,2) NOT NULL CHECK (bdt_amount >= 0),        -- value of FC at fx_rate, or BDT margin/charges
  margin_applied numeric(18,2) NOT NULL DEFAULT 0,                    -- LC margin released at settlement
  bank_charges numeric(18,2) NOT NULL DEFAULT 0,
  fx_gain_loss numeric(18,2) NOT NULL DEFAULT 0,                      -- + loss, - gain
  reference    text,
  status       text NOT NULL DEFAULT 'posted' CHECK (status IN ('pending_approval','posted','rejected')),
  journal_entry_id uuid REFERENCES journal_entries(id),
  created_by   uuid REFERENCES users(id),
  created_at   timestamptz NOT NULL DEFAULT now(),
  UNIQUE (company_id, payment_no)
);

CREATE TABLE bills_of_entry (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  company_id   uuid NOT NULL REFERENCES companies(id),
  shipment_id  uuid NOT NULL REFERENCES shipments(id),
  be_no        text NOT NULL,
  be_date      date NOT NULL,
  customs_station text NOT NULL DEFAULT 'Custom House, Chattogram',
  cnf_agent    text,
  customs_rate numeric(18,6) NOT NULL CHECK (customs_rate > 0),   -- customs-declared exchange rate
  landing_charge_pct numeric(9,4) NOT NULL DEFAULT 1,
  cif_value    numeric(18,2) NOT NULL DEFAULT 0,       -- FC
  assessable_value numeric(18,2) NOT NULL DEFAULT 0,   -- BDT
  cd           numeric(18,2) NOT NULL DEFAULT 0,
  rd           numeric(18,2) NOT NULL DEFAULT 0,
  sd           numeric(18,2) NOT NULL DEFAULT 0,
  vat          numeric(18,2) NOT NULL DEFAULT 0,
  ait          numeric(18,2) NOT NULL DEFAULT 0,
  at           numeric(18,2) NOT NULL DEFAULT 0,
  other_fees   numeric(18,2) NOT NULL DEFAULT 0,       -- document/scanning/processing fees
  total_payable numeric(18,2) NOT NULL DEFAULT 0,
  status       text NOT NULL DEFAULT 'assessed' CHECK (status IN ('assessed','paid','released')),
  bank_account_id uuid REFERENCES bank_accounts(id),
  paid_date    date,
  released_date date,
  journal_entry_id uuid REFERENCES journal_entries(id),
  created_by   uuid REFERENCES users(id),
  created_at   timestamptz NOT NULL DEFAULT now(),
  UNIQUE (company_id, be_no),
  UNIQUE (shipment_id)
);
CREATE TABLE bill_of_entry_lines (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  be_id        uuid NOT NULL REFERENCES bills_of_entry(id) ON DELETE CASCADE,
  shipment_line_id uuid NOT NULL REFERENCES shipment_lines(id),
  product_id   uuid NOT NULL REFERENCES products(id),
  hs_code      text NOT NULL,
  cif_value    numeric(18,2) NOT NULL,                 -- FC
  assessable_value numeric(18,2) NOT NULL,             -- BDT
  cd_rate numeric(9,4) NOT NULL, rd_rate numeric(9,4) NOT NULL, sd_rate numeric(9,4) NOT NULL,
  vat_rate numeric(9,4) NOT NULL, ait_rate numeric(9,4) NOT NULL, at_rate numeric(9,4) NOT NULL,
  cd numeric(18,2) NOT NULL, rd numeric(18,2) NOT NULL, sd numeric(18,2) NOT NULL,
  vat numeric(18,2) NOT NULL, ait numeric(18,2) NOT NULL, at numeric(18,2) NOT NULL
);

CREATE TABLE landed_costs (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  company_id   uuid NOT NULL REFERENCES companies(id),
  shipment_id  uuid NOT NULL REFERENCES shipments(id),
  costing_no   text NOT NULL,
  costing_date date NOT NULL,
  allocation_method text NOT NULL DEFAULT 'value' CHECK (allocation_method IN ('value','quantity','weight','volume')),
  goods_bdt    numeric(18,2) NOT NULL DEFAULT 0,
  duties_bdt   numeric(18,2) NOT NULL DEFAULT 0,
  charges_bdt  numeric(18,2) NOT NULL DEFAULT 0,
  total_landed numeric(18,2) NOT NULL DEFAULT 0,
  status       text NOT NULL DEFAULT 'draft' CHECK (status IN ('draft','finalized')),
  journal_entry_id uuid REFERENCES journal_entries(id),
  finalized_by uuid REFERENCES users(id),
  finalized_at timestamptz,
  created_by   uuid REFERENCES users(id),
  created_at   timestamptz NOT NULL DEFAULT now(),
  UNIQUE (company_id, costing_no),
  UNIQUE (shipment_id)
);
CREATE TABLE landed_cost_charges (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  landed_cost_id uuid NOT NULL REFERENCES landed_costs(id) ON DELETE CASCADE,
  charge_type  text NOT NULL CHECK (charge_type IN ('freight','insurance','lc_commission','bank_charges','cnf_commission','port_charges','shipping_agent','transport','labour','demurrage','testing','other')),
  description  text,
  amount       numeric(18,2) NOT NULL CHECK (amount > 0),     -- BDT, excluding VAT
  vat_amount   numeric(18,2) NOT NULL DEFAULT 0 CHECK (vat_amount >= 0),  -- recoverable input VAT on the service
  allocation_method text CHECK (allocation_method IN ('value','quantity','weight','volume')),
  supplier_id  uuid REFERENCES suppliers(id),          -- credit AP if set
  bank_account_id uuid REFERENCES bank_accounts(id),   -- or paid from bank/cash
  source       text NOT NULL DEFAULT 'manual' CHECK (source IN ('manual','foreign_payment')),
  source_id    uuid
);
CREATE TABLE landed_cost_lines (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  landed_cost_id uuid NOT NULL REFERENCES landed_costs(id) ON DELETE CASCADE,
  shipment_line_id uuid NOT NULL REFERENCES shipment_lines(id),
  product_id   uuid NOT NULL REFERENCES products(id),
  qty          numeric(18,4) NOT NULL,
  goods_bdt    numeric(18,2) NOT NULL,
  duties_bdt   numeric(18,2) NOT NULL,
  charges_bdt  numeric(18,2) NOT NULL,
  landed_total numeric(18,2) NOT NULL,
  unit_landed_cost numeric(18,6) NOT NULL
);
