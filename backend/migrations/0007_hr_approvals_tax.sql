-- HR and payroll
CREATE TABLE departments (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  company_id   uuid NOT NULL REFERENCES companies(id),
  parent_id    uuid REFERENCES departments(id),
  code         text NOT NULL,
  name         text NOT NULL,
  name_bn      text,
  is_active    boolean NOT NULL DEFAULT true,
  UNIQUE (company_id, code)
);

CREATE TABLE employees (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  company_id   uuid NOT NULL REFERENCES companies(id),
  branch_id    uuid REFERENCES branches(id),
  department_id uuid REFERENCES departments(id),
  user_id      uuid REFERENCES users(id),
  code         text NOT NULL,
  name         text NOT NULL,
  name_bn      text,
  designation  text,
  nid          text,
  tin          text,
  phone        text,
  email        text,
  address      text,
  join_date    date NOT NULL,
  exit_date    date,
  employment_type text NOT NULL DEFAULT 'permanent' CHECK (employment_type IN ('permanent','probation','contract','intern')),
  status       text NOT NULL DEFAULT 'active' CHECK (status IN ('active','on_leave','resigned','terminated')),
  basic_salary numeric(18,2) NOT NULL DEFAULT 0 CHECK (basic_salary >= 0),
  house_rent   numeric(18,2) NOT NULL DEFAULT 0 CHECK (house_rent >= 0),
  medical      numeric(18,2) NOT NULL DEFAULT 0 CHECK (medical >= 0),
  conveyance   numeric(18,2) NOT NULL DEFAULT 0 CHECK (conveyance >= 0),
  other_allowance numeric(18,2) NOT NULL DEFAULT 0 CHECK (other_allowance >= 0),
  pf_rate      numeric(9,4) NOT NULL DEFAULT 0 CHECK (pf_rate BETWEEN 0 AND 100),   -- % of basic, matched by employer
  monthly_tax  numeric(18,2) NOT NULL DEFAULT 0 CHECK (monthly_tax >= 0),           -- salary TDS
  bank_account_no text,
  payment_method text NOT NULL DEFAULT 'bank' CHECK (payment_method IN ('bank','cash','bkash')),
  created_at   timestamptz NOT NULL DEFAULT now(),
  UNIQUE (company_id, code)
);
ALTER TABLE sales_orders ADD CONSTRAINT sales_orders_salesperson_fk FOREIGN KEY (salesperson_id) REFERENCES employees(id);

CREATE TABLE attendance (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  company_id   uuid NOT NULL REFERENCES companies(id),
  employee_id  uuid NOT NULL REFERENCES employees(id),
  work_date    date NOT NULL,
  check_in     time,
  check_out    time,
  status       text NOT NULL DEFAULT 'present' CHECK (status IN ('present','late','absent','leave','holiday','weekend','half_day')),
  notes        text,
  UNIQUE (employee_id, work_date)
);

CREATE TABLE leave_requests (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  company_id   uuid NOT NULL REFERENCES companies(id),
  employee_id  uuid NOT NULL REFERENCES employees(id),
  leave_type   text NOT NULL CHECK (leave_type IN ('casual','sick','earned','maternity','unpaid','other')),
  from_date    date NOT NULL,
  to_date      date NOT NULL,
  days         numeric(6,1) NOT NULL CHECK (days > 0),
  reason       text,
  status       text NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','approved','rejected','cancelled')),
  decided_by   uuid REFERENCES users(id),
  decided_at   timestamptz,
  created_at   timestamptz NOT NULL DEFAULT now(),
  CHECK (to_date >= from_date)
);

CREATE TABLE payroll_runs (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  company_id   uuid NOT NULL REFERENCES companies(id),
  run_no       text NOT NULL,
  period_year  int NOT NULL,
  period_month int NOT NULL CHECK (period_month BETWEEN 1 AND 12),
  working_days int NOT NULL,
  total_gross  numeric(18,2) NOT NULL DEFAULT 0,
  total_deductions numeric(18,2) NOT NULL DEFAULT 0,
  total_net    numeric(18,2) NOT NULL DEFAULT 0,
  total_employer_pf numeric(18,2) NOT NULL DEFAULT 0,
  status       text NOT NULL DEFAULT 'draft' CHECK (status IN ('draft','pending_approval','approved','posted','paid','rejected')),
  journal_entry_id uuid REFERENCES journal_entries(id),
  payment_journal_id uuid REFERENCES journal_entries(id),
  created_by   uuid REFERENCES users(id),
  created_at   timestamptz NOT NULL DEFAULT now(),
  UNIQUE (company_id, period_year, period_month),
  UNIQUE (company_id, run_no)
);
CREATE TABLE payslips (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  run_id       uuid NOT NULL REFERENCES payroll_runs(id) ON DELETE CASCADE,
  employee_id  uuid NOT NULL REFERENCES employees(id),
  working_days int NOT NULL,
  payable_days numeric(6,1) NOT NULL,
  absent_days  numeric(6,1) NOT NULL DEFAULT 0,
  basic        numeric(18,2) NOT NULL,
  house_rent   numeric(18,2) NOT NULL,
  medical      numeric(18,2) NOT NULL,
  conveyance   numeric(18,2) NOT NULL,
  other_allowance numeric(18,2) NOT NULL,
  gross        numeric(18,2) NOT NULL,
  absent_deduction numeric(18,2) NOT NULL DEFAULT 0,
  pf_employee  numeric(18,2) NOT NULL DEFAULT 0,
  pf_employer  numeric(18,2) NOT NULL DEFAULT 0,
  tax          numeric(18,2) NOT NULL DEFAULT 0,
  net_pay      numeric(18,2) NOT NULL,
  UNIQUE (run_id, employee_id)
);

-- Approvals
CREATE TABLE approval_rules (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  company_id   uuid NOT NULL REFERENCES companies(id),
  entity_type  text NOT NULL CHECK (entity_type IN ('purchase_order','sales_order','supplier_payment','foreign_payment','payroll_run','stock_adjustment')),
  min_amount   numeric(18,2) NOT NULL DEFAULT 0 CHECK (min_amount >= 0),   -- BDT; rule applies when amount >= min_amount
  level        int NOT NULL DEFAULT 1 CHECK (level BETWEEN 1 AND 5),
  approver_role text NOT NULL,          -- role code
  description  text,
  is_active    boolean NOT NULL DEFAULT true,
  UNIQUE (company_id, entity_type, min_amount, level)
);
CREATE TABLE approval_requests (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  company_id   uuid NOT NULL REFERENCES companies(id),
  entity_type  text NOT NULL,
  entity_id    uuid NOT NULL,
  entity_no    text NOT NULL,
  amount       numeric(18,2) NOT NULL DEFAULT 0,
  level        int NOT NULL,
  approver_role text NOT NULL,
  status       text NOT NULL DEFAULT 'pending' CHECK (status IN ('waiting','pending','approved','rejected','cancelled')),
  requested_by uuid REFERENCES users(id),
  decided_by   uuid REFERENCES users(id),
  decided_at   timestamptz,
  comment      text,
  created_at   timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX approval_requests_entity_idx ON approval_requests (company_id, entity_type, entity_id);
CREATE INDEX approval_requests_pending_idx ON approval_requests (company_id, approver_role) WHERE status = 'pending';

-- Bangladesh VAT returns (Mushak 9.1 snapshot per tax period)
CREATE TABLE vat_returns (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  company_id   uuid NOT NULL REFERENCES companies(id),
  period_year  int NOT NULL,
  period_month int NOT NULL CHECK (period_month BETWEEN 1 AND 12),
  output_vat   numeric(18,2) NOT NULL DEFAULT 0,
  output_sd    numeric(18,2) NOT NULL DEFAULT 0,
  input_vat_local  numeric(18,2) NOT NULL DEFAULT 0,
  input_vat_import numeric(18,2) NOT NULL DEFAULT 0,
  advance_tax  numeric(18,2) NOT NULL DEFAULT 0,       -- AT paid at import stage
  vds_by_customers numeric(18,2) NOT NULL DEFAULT 0,   -- decreasing adjustment
  vds_on_suppliers numeric(18,2) NOT NULL DEFAULT 0,   -- increasing adjustment (payable to treasury)
  credit_notes_vat numeric(18,2) NOT NULL DEFAULT 0,
  net_payable  numeric(18,2) NOT NULL DEFAULT 0,       -- negative = carry forward
  status       text NOT NULL DEFAULT 'draft' CHECK (status IN ('draft','submitted')),
  journal_entry_id uuid REFERENCES journal_entries(id),
  submitted_by uuid REFERENCES users(id),
  submitted_at timestamptz,
  created_at   timestamptz NOT NULL DEFAULT now(),
  UNIQUE (company_id, period_year, period_month)
);
