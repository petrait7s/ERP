-- Protect system accounts and the meaning of accounts that already carry postings.
CREATE FUNCTION accounts_guard() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF TG_OP = 'DELETE' THEN
    IF OLD.system_key IS NOT NULL THEN
      RAISE EXCEPTION 'system account % cannot be deleted', OLD.code USING ERRCODE = 'P0001';
    END IF;
    RETURN OLD;
  END IF;
  IF NEW.account_type <> OLD.account_type AND EXISTS (SELECT 1 FROM journal_lines l WHERE l.account_id = OLD.id) THEN
    RAISE EXCEPTION 'account % already has postings; its type cannot change', OLD.code USING ERRCODE = 'P0001';
  END IF;
  IF NEW.is_group AND NOT OLD.is_group AND EXISTS (SELECT 1 FROM journal_lines l WHERE l.account_id = OLD.id) THEN
    RAISE EXCEPTION 'account % already has postings; it cannot become a group', OLD.code USING ERRCODE = 'P0001';
  END IF;
  NEW.system_key := OLD.system_key;
  RETURN NEW;
END $$;
CREATE TRIGGER accounts_guard BEFORE UPDATE OR DELETE ON accounts FOR EACH ROW EXECUTE FUNCTION accounts_guard();

-- Journal lines may only use active, non-group accounts of the same company.
CREATE FUNCTION journal_line_account_check() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE a accounts%ROWTYPE;
BEGIN
  SELECT * INTO a FROM accounts WHERE id = NEW.account_id;
  IF a.company_id <> NEW.company_id THEN RAISE EXCEPTION 'account belongs to another company' USING ERRCODE = 'P0001'; END IF;
  IF a.is_group THEN RAISE EXCEPTION 'account % is a group; post to one of its sub-accounts', a.code USING ERRCODE = 'P0001'; END IF;
  IF NOT a.is_active THEN RAISE EXCEPTION 'account % is inactive', a.code USING ERRCODE = 'P0001'; END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER journal_line_account_check BEFORE INSERT ON journal_lines FOR EACH ROW EXECUTE FUNCTION journal_line_account_check();
