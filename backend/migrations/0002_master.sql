-- Master data
CREATE TABLE exchange_rates (
  id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  company_id  uuid NOT NULL REFERENCES companies(id),
  currency    text NOT NULL REFERENCES currencies(code),
  rate_date   date NOT NULL,
  rate_type   text NOT NULL DEFAULT 'mid' CHECK (rate_type IN ('mid','bc_selling','tt_buying','customs')),
  rate        numeric(18,6) NOT NULL CHECK (rate > 0),   -- BDT (base) per 1 unit of currency
  source      text,
  created_at  timestamptz NOT NULL DEFAULT now(),
  UNIQUE (company_id, currency, rate_date, rate_type)
);

CREATE TABLE units (
  id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  company_id  uuid NOT NULL REFERENCES companies(id),
  code        text NOT NULL,
  name        text NOT NULL,
  name_bn     text,
  UNIQUE (company_id, code)
);

CREATE TABLE categories (
  id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  company_id  uuid NOT NULL REFERENCES companies(id),
  parent_id   uuid REFERENCES categories(id),
  code        text NOT NULL,
  name        text NOT NULL,
  name_bn     text,
  is_active   boolean NOT NULL DEFAULT true,
  UNIQUE (company_id, code)
);

-- Bangladesh customs tariff lines. Rates are percentages and fully configurable.
CREATE TABLE hs_codes (
  id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  company_id  uuid NOT NULL REFERENCES companies(id),
  code        text NOT NULL,
  description text NOT NULL,
  cd_rate     numeric(9,4) NOT NULL DEFAULT 0 CHECK (cd_rate >= 0),   -- Customs Duty
  rd_rate     numeric(9,4) NOT NULL DEFAULT 0 CHECK (rd_rate >= 0),   -- Regulatory Duty
  sd_rate     numeric(9,4) NOT NULL DEFAULT 0 CHECK (sd_rate >= 0),   -- Supplementary Duty
  vat_rate    numeric(9,4) NOT NULL DEFAULT 15 CHECK (vat_rate >= 0), -- import-stage VAT
  ait_rate    numeric(9,4) NOT NULL DEFAULT 5 CHECK (ait_rate >= 0),  -- Advance Income Tax
  at_rate     numeric(9,4) NOT NULL DEFAULT 5 CHECK (at_rate >= 0),   -- Advance Tax (VAT Act)
  is_active   boolean NOT NULL DEFAULT true,
  UNIQUE (company_id, code)
);

CREATE TABLE products (
  id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  company_id  uuid NOT NULL REFERENCES companies(id),
  sku         text NOT NULL,
  name        text NOT NULL,
  name_bn     text,
  description text,
  category_id uuid REFERENCES categories(id),
  unit_id     uuid REFERENCES units(id),
  hs_code_id  uuid REFERENCES hs_codes(id),
  product_type text NOT NULL DEFAULT 'goods' CHECK (product_type IN ('goods','service')),
  brand       text,
  origin_country text,
  barcode     text,
  sale_price  numeric(18,2) NOT NULL DEFAULT 0 CHECK (sale_price >= 0),
  min_sale_price numeric(18,2) NOT NULL DEFAULT 0 CHECK (min_sale_price >= 0),
  vat_rate    numeric(9,4) NOT NULL DEFAULT 15 CHECK (vat_rate >= 0),
  sd_rate     numeric(9,4) NOT NULL DEFAULT 0 CHECK (sd_rate >= 0),
  reorder_level numeric(18,4) NOT NULL DEFAULT 0,
  reorder_qty numeric(18,4) NOT NULL DEFAULT 0,
  lead_time_days int NOT NULL DEFAULT 0,
  weight_kg   numeric(18,4) NOT NULL DEFAULT 0,
  volume_cbm  numeric(18,6) NOT NULL DEFAULT 0,
  is_active   boolean NOT NULL DEFAULT true,
  created_at  timestamptz NOT NULL DEFAULT now(),
  UNIQUE (company_id, sku)
);

CREATE TABLE suppliers (
  id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  company_id  uuid NOT NULL REFERENCES companies(id),
  code        text NOT NULL,
  name        text NOT NULL,
  supplier_type text NOT NULL DEFAULT 'local' CHECK (supplier_type IN ('local','foreign','service')),
  country     text NOT NULL DEFAULT 'BD',
  currency    text NOT NULL DEFAULT 'BDT' REFERENCES currencies(code),
  bin         text,
  tin         text,
  address     text,
  contact_person text,
  phone       text,
  email       text,
  bank_details text,
  payment_terms_days int NOT NULL DEFAULT 0,
  tds_rate    numeric(9,4) NOT NULL DEFAULT 0 CHECK (tds_rate >= 0),  -- tax deducted at source on payments
  vds_rate    numeric(9,4) NOT NULL DEFAULT 0 CHECK (vds_rate >= 0),  -- VAT deducted at source
  is_active   boolean NOT NULL DEFAULT true,
  created_at  timestamptz NOT NULL DEFAULT now(),
  UNIQUE (company_id, code)
);

CREATE TABLE customers (
  id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  company_id  uuid NOT NULL REFERENCES companies(id),
  code        text NOT NULL,
  name        text NOT NULL,
  name_bn     text,
  customer_type text NOT NULL DEFAULT 'dealer' CHECK (customer_type IN ('dealer','retail','corporate','government','distributor')),
  bin         text,
  tin         text,
  address     text,
  district    text,
  contact_person text,
  phone       text,
  email       text,
  credit_limit numeric(18,2) NOT NULL DEFAULT 0 CHECK (credit_limit >= 0),
  payment_terms_days int NOT NULL DEFAULT 0,
  is_active   boolean NOT NULL DEFAULT true,
  created_at  timestamptz NOT NULL DEFAULT now(),
  UNIQUE (company_id, code)
);

CREATE TABLE warehouses (
  id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  company_id  uuid NOT NULL REFERENCES companies(id),
  branch_id   uuid REFERENCES branches(id),
  code        text NOT NULL,
  name        text NOT NULL,
  name_bn     text,
  address     text,
  is_active   boolean NOT NULL DEFAULT true,
  UNIQUE (company_id, code)
);
