-- Double-entry accounting
CREATE TABLE accounts (
  id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  company_id  uuid NOT NULL REFERENCES companies(id),
  code        text NOT NULL,
  name        text NOT NULL,
  name_bn     text,
  account_type text NOT NULL CHECK (account_type IN ('asset','liability','equity','income','expense')),
  parent_id   uuid REFERENCES accounts(id),
  is_group    boolean NOT NULL DEFAULT false,
  system_key  text,
  is_active   boolean NOT NULL DEFAULT true,
  created_at  timestamptz NOT NULL DEFAULT now(),
  UNIQUE (company_id, code)
);
CREATE UNIQUE INDEX accounts_system_key_uq ON accounts (company_id, system_key) WHERE system_key IS NOT NULL;

CREATE TABLE fiscal_periods (
  id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  company_id  uuid NOT NULL REFERENCES companies(id),
  name        text NOT NULL,
  start_date  date NOT NULL,
  end_date    date NOT NULL,
  status      text NOT NULL DEFAULT 'open' CHECK (status IN ('open','closed')),
  closed_by   uuid REFERENCES users(id),
  closed_at   timestamptz,
  CHECK (end_date >= start_date),
  UNIQUE (company_id, start_date)
);

CREATE TABLE journal_entries (
  id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  company_id  uuid NOT NULL REFERENCES companies(id),
  branch_id   uuid REFERENCES branches(id),
  entry_no    text NOT NULL,
  entry_date  date NOT NULL,
  source_type text NOT NULL,          -- manual, sales_invoice, goods_receipt, ...
  source_id   uuid,
  memo        text,
  status      text NOT NULL DEFAULT 'posted' CHECK (status IN ('posted','reversed')),
  reversal_of uuid REFERENCES journal_entries(id),
  created_by  uuid REFERENCES users(id),
  created_at  timestamptz NOT NULL DEFAULT now(),
  UNIQUE (company_id, entry_no)
);
CREATE INDEX journal_entries_source_idx ON journal_entries (company_id, source_type, source_id);
CREATE INDEX journal_entries_date_idx ON journal_entries (company_id, entry_date);

CREATE TABLE journal_lines (
  id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  entry_id    uuid NOT NULL REFERENCES journal_entries(id),
  company_id  uuid NOT NULL REFERENCES companies(id),
  line_no     int NOT NULL,
  account_id  uuid NOT NULL REFERENCES accounts(id),
  debit       numeric(18,2) NOT NULL DEFAULT 0 CHECK (debit >= 0),
  credit      numeric(18,2) NOT NULL DEFAULT 0 CHECK (credit >= 0),
  party_type  text CHECK (party_type IN ('customer','supplier','employee')),
  party_id    uuid,
  currency    text REFERENCES currencies(code),
  fc_amount   numeric(18,2),          -- signed amount in foreign currency (debit positive)
  ref_type    text,                   -- sub-ledger tag, e.g. shipment
  ref_id      uuid,
  memo        text,
  CHECK (NOT (debit > 0 AND credit > 0)),
  CHECK (debit > 0 OR credit > 0)
);
CREATE INDEX journal_lines_entry_idx ON journal_lines (entry_id);
CREATE INDEX journal_lines_account_idx ON journal_lines (company_id, account_id);
CREATE INDEX journal_lines_party_idx ON journal_lines (company_id, party_type, party_id);
CREATE INDEX journal_lines_ref_idx ON journal_lines (company_id, ref_type, ref_id);

-- Every entry must balance; checked at commit time.
CREATE FUNCTION check_entry_balanced() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE d numeric; c numeric; n int;
BEGIN
  SELECT coalesce(sum(debit),0), coalesce(sum(credit),0), count(*) INTO d, c, n FROM journal_lines WHERE entry_id = NEW.entry_id;
  IF d <> c THEN
    RAISE EXCEPTION 'journal entry % is not balanced: debit % <> credit %', NEW.entry_id, d, c USING ERRCODE = 'P0001';
  END IF;
  IF n < 2 THEN
    RAISE EXCEPTION 'journal entry % must have at least two lines', NEW.entry_id USING ERRCODE = 'P0001';
  END IF;
  RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER journal_lines_balanced AFTER INSERT ON journal_lines
  DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION check_entry_balanced();

-- Posted lines are immutable; corrections are made by reversal entries.
CREATE TRIGGER journal_lines_immutable BEFORE UPDATE OR DELETE ON journal_lines FOR EACH ROW EXECUTE FUNCTION forbid_change();
CREATE FUNCTION journal_entry_guard() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF TG_OP = 'DELETE' THEN
    RAISE EXCEPTION 'journal entries cannot be deleted' USING ERRCODE = 'P0001';
  END IF;
  IF NEW.entry_date <> OLD.entry_date OR NEW.entry_no <> OLD.entry_no OR NEW.company_id <> OLD.company_id
     OR NEW.source_type <> OLD.source_type OR NEW.source_id IS DISTINCT FROM OLD.source_id THEN
    RAISE EXCEPTION 'posted journal entries are immutable' USING ERRCODE = 'P0001';
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER journal_entries_guard BEFORE UPDATE OR DELETE ON journal_entries FOR EACH ROW EXECUTE FUNCTION journal_entry_guard();

-- No posting into closed periods.
CREATE FUNCTION check_period_open() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF EXISTS (SELECT 1 FROM fiscal_periods p WHERE p.company_id = NEW.company_id AND p.status = 'closed'
             AND NEW.entry_date BETWEEN p.start_date AND p.end_date) THEN
    RAISE EXCEPTION 'accounting period for % is closed', NEW.entry_date USING ERRCODE = 'P0001';
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER journal_entries_period BEFORE INSERT ON journal_entries FOR EACH ROW EXECUTE FUNCTION check_period_open();

CREATE TABLE bank_accounts (
  id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  company_id  uuid NOT NULL REFERENCES companies(id),
  branch_id   uuid REFERENCES branches(id),
  account_id  uuid NOT NULL REFERENCES accounts(id),   -- GL account
  code        text NOT NULL,
  name        text NOT NULL,
  kind        text NOT NULL DEFAULT 'bank' CHECK (kind IN ('bank','cash','mobile')),
  bank_name   text,
  branch_name text,
  account_no  text,
  routing_no  text,
  swift_code  text,
  currency    text NOT NULL DEFAULT 'BDT' REFERENCES currencies(code),
  is_active   boolean NOT NULL DEFAULT true,
  created_at  timestamptz NOT NULL DEFAULT now(),
  UNIQUE (company_id, code)
);

CREATE TABLE bank_transfers (
  id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  company_id  uuid NOT NULL REFERENCES companies(id),
  transfer_no text NOT NULL,
  transfer_date date NOT NULL,
  from_bank_account_id uuid NOT NULL REFERENCES bank_accounts(id),
  to_bank_account_id   uuid NOT NULL REFERENCES bank_accounts(id),
  amount      numeric(18,2) NOT NULL CHECK (amount > 0),        -- in source account currency
  to_amount   numeric(18,2) NOT NULL CHECK (to_amount > 0),     -- in destination account currency
  bdt_amount  numeric(18,2) NOT NULL,
  charges     numeric(18,2) NOT NULL DEFAULT 0,
  reference   text,
  journal_entry_id uuid REFERENCES journal_entries(id),
  created_by  uuid REFERENCES users(id),
  created_at  timestamptz NOT NULL DEFAULT now(),
  UNIQUE (company_id, transfer_no),
  CHECK (from_bank_account_id <> to_bank_account_id)
);
