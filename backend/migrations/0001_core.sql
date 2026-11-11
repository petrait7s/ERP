-- Core: tenancy, identity, access control, audit, settings
CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE TABLE currencies (
  code        text PRIMARY KEY,
  name        text NOT NULL,
  symbol      text NOT NULL,
  decimals    int  NOT NULL DEFAULT 2
);

CREATE TABLE companies (
  id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  code        text NOT NULL UNIQUE,
  name        text NOT NULL,
  name_bn     text,
  legal_name  text,
  bin         text,            -- Business Identification Number (VAT registration)
  tin         text,            -- Taxpayer Identification Number
  irc_no      text,            -- Import Registration Certificate
  trade_license text,
  address     text,
  phone       text,
  email       text,
  base_currency text NOT NULL DEFAULT 'BDT' REFERENCES currencies(code),
  timezone    text NOT NULL DEFAULT 'Asia/Dhaka',
  locale      text NOT NULL DEFAULT 'en' CHECK (locale IN ('en','bn')),
  fiscal_year_start_month int NOT NULL DEFAULT 7 CHECK (fiscal_year_start_month BETWEEN 1 AND 12),
  is_active   boolean NOT NULL DEFAULT true,
  created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE branches (
  id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  company_id  uuid NOT NULL REFERENCES companies(id),
  code        text NOT NULL,
  name        text NOT NULL,
  name_bn     text,
  address     text,
  phone       text,
  is_head_office boolean NOT NULL DEFAULT false,
  is_active   boolean NOT NULL DEFAULT true,
  created_at  timestamptz NOT NULL DEFAULT now(),
  UNIQUE (company_id, code)
);

CREATE TABLE users (
  id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  email         text NOT NULL,
  password_hash text NOT NULL,
  full_name     text NOT NULL,
  phone         text,
  locale        text NOT NULL DEFAULT 'en' CHECK (locale IN ('en','bn')),
  is_active     boolean NOT NULL DEFAULT true,
  is_superadmin boolean NOT NULL DEFAULT false,
  failed_logins int NOT NULL DEFAULT 0,
  locked_until  timestamptz,
  last_login_at timestamptz,
  created_at    timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX users_email_uq ON users (lower(email));

CREATE TABLE roles (
  id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  company_id  uuid NOT NULL REFERENCES companies(id),
  code        text NOT NULL,
  name        text NOT NULL,
  description text,
  is_system   boolean NOT NULL DEFAULT false,
  permissions text[] NOT NULL DEFAULT '{}',
  created_at  timestamptz NOT NULL DEFAULT now(),
  UNIQUE (company_id, code)
);

CREATE TABLE user_companies (
  user_id     uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  company_id  uuid NOT NULL REFERENCES companies(id),
  role_id     uuid NOT NULL REFERENCES roles(id),
  branch_id   uuid REFERENCES branches(id),   -- NULL = all branches
  is_default  boolean NOT NULL DEFAULT false,
  PRIMARY KEY (user_id, company_id)
);

CREATE TABLE refresh_tokens (
  id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id     uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  company_id  uuid NOT NULL REFERENCES companies(id),
  token_hash  text NOT NULL UNIQUE,
  expires_at  timestamptz NOT NULL,
  revoked_at  timestamptz,
  created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE audit_logs (
  id          bigserial PRIMARY KEY,
  company_id  uuid,
  user_id     uuid,
  action      text NOT NULL,
  entity      text NOT NULL,
  entity_id   text,
  detail      jsonb,
  created_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX audit_logs_company_idx ON audit_logs (company_id, created_at DESC);
CREATE INDEX audit_logs_entity_idx ON audit_logs (entity, entity_id);

CREATE TABLE settings (
  company_id  uuid NOT NULL REFERENCES companies(id),
  key         text NOT NULL,
  value       jsonb NOT NULL,
  updated_at  timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (company_id, key)
);

CREATE TABLE sequences (
  company_id  uuid NOT NULL REFERENCES companies(id),
  key         text NOT NULL,
  prefix      text NOT NULL,
  next_no     bigint NOT NULL DEFAULT 1,
  padding     int NOT NULL DEFAULT 5,
  PRIMARY KEY (company_id, key)
);

CREATE TABLE notifications (
  id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  company_id  uuid NOT NULL REFERENCES companies(id),
  user_id     uuid REFERENCES users(id) ON DELETE CASCADE,  -- NULL with role_code = broadcast to role
  role_code   text,
  severity    text NOT NULL DEFAULT 'info' CHECK (severity IN ('info','success','warning','critical')),
  title       text NOT NULL,
  title_bn    text,
  body        text,
  entity      text,
  entity_id   uuid,
  dedupe_key  text,
  read_at     timestamptz,
  created_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX notifications_user_idx ON notifications (company_id, user_id, created_at DESC);
CREATE UNIQUE INDEX notifications_dedupe_uq ON notifications (company_id, dedupe_key) WHERE dedupe_key IS NOT NULL;

-- audit logs are append-only
CREATE FUNCTION forbid_change() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  RAISE EXCEPTION '% on % is not allowed: records are immutable', TG_OP, TG_TABLE_NAME USING ERRCODE = 'P0001';
END $$;
CREATE TRIGGER audit_logs_immutable BEFORE UPDATE OR DELETE ON audit_logs FOR EACH ROW EXECUTE FUNCTION forbid_change();

INSERT INTO currencies (code, name, symbol, decimals) VALUES
 ('BDT','Bangladeshi Taka','৳',2),('USD','US Dollar','$',2),('EUR','Euro','€',2),('GBP','Pound Sterling','£',2),
 ('CNY','Chinese Yuan','¥',2),('INR','Indian Rupee','₹',2),('JPY','Japanese Yen','¥',0),('AED','UAE Dirham','AED',2),
 ('SGD','Singapore Dollar','S$',2),('SAR','Saudi Riyal','SAR',2),('MYR','Malaysian Ringgit','RM',2),('THB','Thai Baht','฿',2),
 ('TRY','Turkish Lira','₺',2),('KRW','South Korean Won','₩',0);
