-- Ignition initial migration
-- Maps to design doc §3 domain model / §5 billing / §8 multi-tenancy
--
-- Conventions:
--   1. All tenant tables carry tenant_id and RLS (app connects as ignition_app, policies apply)
--   2. ledger_entry grants SELECT/INSERT only to the app role — ledger is append-only (constraint C3)
--   3. Amounts stored as BIGINT in smallest currency unit (cents); no floating point

-- Target schema. psql variable, default ignition: isolates this project on a shared DB
-- (same convention as growing-tales). Override: psql -v schema=xxx
\if :{?schema}
\else
  \set schema ignition
\endif
CREATE SCHEMA IF NOT EXISTS :"schema";
SET search_path TO :"schema", public;

BEGIN;

-- ---------------------------------------------------------------- Application role
-- App must connect as this non-privileged role or RLS policies do not apply.
--
-- **Deliberately NOLOGIN** — permission container only: migrations run on hosted DBs
-- (Supabase) that are internet-reachable. A login role with a default password is a
-- guessable entry point. Enabling login is a deploy-time step with a real random password:
--
--   ALTER ROLE ignition_app LOGIN PASSWORD '<random password>';
--
-- Local docker: `make migrate` enables login; see Makefile.
--
-- Current role may lack CREATEROLE; failure to create must not block migration, but warn —
-- without this role, tenant isolation falls back to application-layer WHERE clauses only.
DO $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'ignition_app') THEN
    BEGIN
      CREATE ROLE ignition_app NOLOGIN;
    EXCEPTION WHEN insufficient_privilege THEN
      RAISE WARNING 'insufficient privilege to create ignition_app role; DBA must create and grant manually or RLS will not apply';
    END;
  END IF;
END
$$;

-- ---------------------------------------------------------------- Enums
CREATE TYPE attribution_method AS ENUM (
  'deterministic_code',   -- claim code redemption    confidence=100  billable
  'install_referrer',     -- Play Install Referrer    100         billable
  'universal_link',       -- direct open (installed)  100         billable
  'clipboard_match',      -- clipboard match          60          NOT billable
  'probabilistic'         -- fingerprint/time window  30          NOT billable
);

CREATE TYPE billable_status AS ENUM (
  'pending',   -- received, within hold period
  'held',      -- risk hold, awaiting manual review
  'cleared',   -- released, billable
  'billed',    -- invoiced
  'reversed',  -- reversed (refund / post-hoc fraud)
  'rejected'   -- invalid, not billable
);

CREATE TYPE claim_status AS ENUM ('issued', 'redeemed', 'expired', 'voided');

CREATE TYPE subscription_status AS ENUM (
  'trialing', 'active', 'past_due', 'paused', 'canceled'
);

-- ---------------------------------------------------------------- Tenant and subscription
CREATE TABLE tenant (
  id          BIGSERIAL PRIMARY KEY,
  name        TEXT        NOT NULL,
  slug        TEXT        NOT NULL UNIQUE,
  created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE app (
  id                         BIGSERIAL PRIMARY KEY,
  tenant_id                  BIGINT      NOT NULL REFERENCES tenant(id),
  name                       TEXT        NOT NULL,
  bundle_id                  TEXT,                 -- iOS
  package_name               TEXT,                 -- Android
  store_url_ios              TEXT,
  store_url_android          TEXT,
  postback_secret_enc        BYTEA       NOT NULL, -- KMS envelope encryption; no plaintext
  attribution_policy_version TEXT        NOT NULL DEFAULT 'v1',
  created_at                 TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ON app (tenant_id);

CREATE TABLE bot (
  id            BIGSERIAL PRIMARY KEY,
  tenant_id     BIGINT      NOT NULL REFERENCES tenant(id),
  username      TEXT        NOT NULL,
  token_enc     BYTEA       NOT NULL,  -- KMS envelope encryption; never log
  created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (tenant_id, username)
);

CREATE TABLE plan (
  id          BIGSERIAL PRIMARY KEY,
  code        TEXT        NOT NULL UNIQUE,   -- 'free' | 'starter' | 'pro'
  name        TEXT        NOT NULL,
  created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Entitlement gating (constraint C4): no hard-coded if plan == 'pro'
CREATE TABLE plan_entitlement (
  plan_id BIGINT NOT NULL REFERENCES plan(id),
  key     TEXT   NOT NULL,   -- 'channel.discord' | 'analytics.cohort' | ...
  value   JSONB  NOT NULL,   -- true | {"limit": 5}
  PRIMARY KEY (plan_id, key)
);

-- Sales-negotiated exceptions; without this table, promises become hard-coded
CREATE TABLE tenant_entitlement_override (
  tenant_id  BIGINT      NOT NULL REFERENCES tenant(id),
  key        TEXT        NOT NULL,
  value      JSONB       NOT NULL,
  note       TEXT,
  expires_at TIMESTAMPTZ,
  PRIMARY KEY (tenant_id, key)
);

CREATE TABLE subscription (
  id                   BIGSERIAL PRIMARY KEY,
  tenant_id            BIGINT              NOT NULL REFERENCES tenant(id),
  plan_id              BIGINT              NOT NULL REFERENCES plan(id),
  status               subscription_status NOT NULL,
  stripe_subscription_id TEXT,
  trial_ends_at        TIMESTAMPTZ,
  current_period_start TIMESTAMPTZ         NOT NULL,
  current_period_end   TIMESTAMPTZ         NOT NULL,
  grace_until          TIMESTAMPTZ,        -- past_due grace; service continues within window
  created_at           TIMESTAMPTZ         NOT NULL DEFAULT now(),
  updated_at           TIMESTAMPTZ         NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX ON subscription (tenant_id) WHERE status <> 'canceled';

-- Pricing config: effective intervals required; price increases are new rows, not updates (design doc §5.3)
CREATE TABLE pricing_config (
  id                 BIGSERIAL   PRIMARY KEY,
  tenant_id          BIGINT      REFERENCES tenant(id),  -- NULL = global default
  effective_from     TIMESTAMPTZ NOT NULL,
  effective_to       TIMESTAMPTZ,                        -- NULL = currently effective
  platform_fee_cents BIGINT      NOT NULL DEFAULT 0,
  cpa_rates          JSONB       NOT NULL DEFAULT '{}',  -- {"activation": 200}
  monthly_cap_cents  BIGINT,                             -- NULL = no cap
  currency           CHAR(3)     NOT NULL DEFAULT 'USD',
  created_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ON pricing_config (tenant_id, effective_from DESC);

-- ---------------------------------------------------------------- Channel side
CREATE TABLE kol (
  id            BIGSERIAL PRIMARY KEY,
  tenant_id     BIGINT      NOT NULL REFERENCES tenant(id),
  display_name  TEXT        NOT NULL,
  tg_user_id    BIGINT,
  payout_email  TEXT,
  created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ON kol (tenant_id);

CREATE TABLE channel (
  id            BIGSERIAL PRIMARY KEY,
  tenant_id     BIGINT      NOT NULL REFERENCES tenant(id),
  kol_id        BIGINT      NOT NULL REFERENCES kol(id),
  platform      TEXT        NOT NULL DEFAULT 'telegram',
  external_id   TEXT,       -- TG chat id / Discord guild id
  name          TEXT        NOT NULL,
  created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ON channel (tenant_id, kol_id);

CREATE TABLE template (
  id            BIGSERIAL PRIMARY KEY,
  code          TEXT        NOT NULL UNIQUE,   -- 'lucky_wheel'
  name          TEXT        NOT NULL,
  config_schema JSONB       NOT NULL,          -- JSON Schema; validated on campaign save
  created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE campaign (
  id            BIGSERIAL PRIMARY KEY,
  tenant_id     BIGINT      NOT NULL REFERENCES tenant(id),
  app_id        BIGINT      NOT NULL REFERENCES app(id),
  template_id   BIGINT      NOT NULL REFERENCES template(id),
  name          TEXT        NOT NULL,
  config        JSONB       NOT NULL DEFAULT '{}',
  daily_play_limit INT      NOT NULL DEFAULT 3,   -- risk L1
  status        TEXT        NOT NULL DEFAULT 'draft',
  starts_at     TIMESTAMPTZ,
  ends_at       TIMESTAMPTZ,
  created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ON campaign (tenant_id);

CREATE TABLE reward_item (
  id            BIGSERIAL PRIMARY KEY,
  tenant_id     BIGINT      NOT NULL REFERENCES tenant(id),
  campaign_id   BIGINT      NOT NULL REFERENCES campaign(id),
  label         TEXT        NOT NULL,
  weight        INT         NOT NULL CHECK (weight >= 0),
  remaining     BIGINT      NOT NULL CHECK (remaining >= 0),
  version       BIGINT      NOT NULL DEFAULT 0,  -- optimistic lock
  payload       JSONB       NOT NULL DEFAULT '{}'
);
CREATE INDEX ON reward_item (tenant_id, campaign_id);

-- Link / placement: tracking_id must be non-enumerable or KOLs can guess each other's links
CREATE TABLE link (
  id            BIGSERIAL PRIMARY KEY,
  tenant_id     BIGINT      NOT NULL REFERENCES tenant(id),
  campaign_id   BIGINT      NOT NULL REFERENCES campaign(id),
  channel_id    BIGINT      NOT NULL REFERENCES channel(id),
  kol_id        BIGINT      NOT NULL REFERENCES kol(id),
  tracking_id   TEXT        NOT NULL UNIQUE,   -- 10-char base62 random
  created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ON link (tenant_id, campaign_id);

-- ---------------------------------------------------------------- Users and attribution
CREATE TABLE player (
  id            BIGSERIAL   PRIMARY KEY,
  tenant_id     BIGINT      NOT NULL REFERENCES tenant(id),
  tg_user_id    BIGINT      NOT NULL,
  app_user_id   TEXT,                            -- bound on redeem; single stitch point end-to-end
  device_ids    TEXT[]      NOT NULL DEFAULT '{}',
  tg_is_premium BOOLEAN     NOT NULL DEFAULT false,
  tg_username   TEXT,
  first_seen_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (tenant_id, tg_user_id)
);
CREATE UNIQUE INDEX ON player (tenant_id, app_user_id) WHERE app_user_id IS NOT NULL;

-- Attribution record — trust foundation of the system (design doc §3)
CREATE TABLE attribution (
  id             BIGSERIAL          PRIMARY KEY,
  tenant_id      BIGINT             NOT NULL REFERENCES tenant(id),
  player_id      BIGINT             NOT NULL REFERENCES player(id),
  kol_id         BIGINT             NOT NULL REFERENCES kol(id),
  campaign_id    BIGINT             NOT NULL REFERENCES campaign(id),
  link_id        BIGINT             NOT NULL REFERENCES link(id),

  method         attribution_method NOT NULL,
  confidence     SMALLINT           NOT NULL CHECK (confidence BETWEEN 0 AND 100),
  -- Denormalised but required: billing rules change; invoiced decisions must be frozen
  is_billable    BOOLEAN            NOT NULL,

  policy_version TEXT               NOT NULL,  -- public doc version; used for dispute replay
  touch_at       TIMESTAMPTZ        NOT NULL,  -- first touch
  attributed_at  TIMESTAMPTZ        NOT NULL,  -- attribution established
  locked_until   TIMESTAMPTZ        NOT NULL,  -- attribution lock period
  evidence       JSONB              NOT NULL,  -- decision snapshot; append-only semantics

  -- Single attribution: one player belongs to one KOL; MVP does not split multi-touch
  UNIQUE (tenant_id, player_id)
);
CREATE INDEX ON attribution (tenant_id, kol_id, attributed_at);

CREATE TABLE claim_code (
  id            BIGSERIAL   PRIMARY KEY,
  tenant_id     BIGINT      NOT NULL REFERENCES tenant(id),
  code          TEXT        NOT NULL,   -- 8 chars; excludes 0/O/1/I/l
  player_id     BIGINT      NOT NULL REFERENCES player(id),
  campaign_id   BIGINT      NOT NULL REFERENCES campaign(id),
  link_id       BIGINT      NOT NULL REFERENCES link(id),
  kol_id        BIGINT      NOT NULL REFERENCES kol(id),
  status        claim_status NOT NULL DEFAULT 'issued',
  issued_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
  expires_at    TIMESTAMPTZ NOT NULL,
  redeemed_at   TIMESTAMPTZ,
  UNIQUE (tenant_id, code)
);
CREATE INDEX ON claim_code (tenant_id, player_id, status);

-- ---------------------------------------------------------------- Billing
-- Billable event — atomic unit of revenue (design doc §3)
CREATE TABLE billable_event (
  id             BIGSERIAL       PRIMARY KEY,
  tenant_id      BIGINT          NOT NULL REFERENCES tenant(id),
  attribution_id BIGINT          NOT NULL REFERENCES attribution(id),
  event_type     TEXT            NOT NULL,   -- 'activation' | 'iap_purchase'
  external_id    TEXT            NOT NULL,   -- main-app unique id; idempotency key

  status         billable_status NOT NULL DEFAULT 'pending',
  amount_cents   BIGINT          NOT NULL CHECK (amount_cents >= 0),
  currency       CHAR(3)         NOT NULL DEFAULT 'USD',
  over_cap       BOOLEAN         NOT NULL DEFAULT false,

  occurred_at    TIMESTAMPTZ     NOT NULL,   -- business time (reported by main app)
  received_at    TIMESTAMPTZ     NOT NULL DEFAULT now(),
  hold_until     TIMESTAMPTZ     NOT NULL,   -- hold period end
  cleared_at     TIMESTAMPTZ,
  billed_at      TIMESTAMPTZ,
  invoice_id     BIGINT,
  status_reason  TEXT,

  -- Physical idempotency guarantee (constraint C6)
  UNIQUE (tenant_id, event_type, external_id)
);
CREATE INDEX ON billable_event (tenant_id, status, hold_until);
CREATE INDEX ON billable_event (tenant_id, cleared_at) WHERE status = 'cleared';

-- Double-entry ledger (constraint C3): append-only; no UPDATE / DELETE
CREATE TABLE ledger_entry (
  id           BIGSERIAL   PRIMARY KEY,
  tenant_id    BIGINT      NOT NULL REFERENCES tenant(id),
  txn_id       UUID        NOT NULL,          -- entries in one txn share txn_id
  account      TEXT        NOT NULL,          -- tenant_receivable | platform_revenue | ...
  direction    CHAR(1)     NOT NULL CHECK (direction IN ('D','C')),
  amount_cents BIGINT      NOT NULL CHECK (amount_cents > 0),
  currency     CHAR(3)     NOT NULL DEFAULT 'USD',
  ref_type     TEXT        NOT NULL,          -- billable_event | subscription | reversal
  ref_id       BIGINT      NOT NULL,
  created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ON ledger_entry (tenant_id, txn_id);
CREATE INDEX ON ledger_entry (tenant_id, account);
CREATE INDEX ON ledger_entry (ref_type, ref_id);

CREATE TABLE invoice (
  id            BIGSERIAL   PRIMARY KEY,
  tenant_id     BIGINT      NOT NULL REFERENCES tenant(id),
  period_start  DATE        NOT NULL,
  period_end    DATE        NOT NULL,
  subtotal_cents BIGINT     NOT NULL DEFAULT 0,
  credit_cents  BIGINT      NOT NULL DEFAULT 0,
  total_cents   BIGINT      NOT NULL DEFAULT 0,
  currency      CHAR(3)     NOT NULL DEFAULT 'USD',
  status        TEXT        NOT NULL DEFAULT 'draft',
  created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (tenant_id, period_start)
);

CREATE TABLE invoice_line (
  id           BIGSERIAL PRIMARY KEY,
  tenant_id    BIGINT    NOT NULL REFERENCES tenant(id),
  invoice_id   BIGINT    NOT NULL REFERENCES invoice(id),
  kind         TEXT      NOT NULL,   -- platform_fee | cpa | credit
  description  TEXT      NOT NULL,
  quantity     BIGINT    NOT NULL DEFAULT 1,
  unit_cents   BIGINT    NOT NULL,
  amount_cents BIGINT    NOT NULL
);
CREATE INDEX ON invoice_line (tenant_id, invoice_id);

-- ---------------------------------------------------------------- Risk
-- L2 signals: capture from day one; this data cannot be backfilled
CREATE TABLE risk_signal (
  id          BIGSERIAL   PRIMARY KEY,
  tenant_id   BIGINT      NOT NULL REFERENCES tenant(id),
  player_id   BIGINT      REFERENCES player(id),
  stage       TEXT        NOT NULL,   -- open | play | claim | redeem
  ip          INET,
  asn         TEXT,
  country     CHAR(2),
  device_id   TEXT,
  user_agent  TEXT,
  latency_ms  BIGINT,                 -- time since previous stage
  attrs       JSONB       NOT NULL DEFAULT '{}',
  created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ON risk_signal (tenant_id, player_id, created_at);

CREATE TABLE risk_verdict (
  id               BIGSERIAL   PRIMARY KEY,
  tenant_id        BIGINT      NOT NULL REFERENCES tenant(id),
  billable_event_id BIGINT     REFERENCES billable_event(id),
  kol_id           BIGINT      REFERENCES kol(id),
  level            TEXT        NOT NULL,   -- pass | suspect | block
  rule             TEXT        NOT NULL,
  detail           JSONB       NOT NULL DEFAULT '{}',
  created_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ON risk_verdict (tenant_id, kol_id, created_at);

-- ---------------------------------------------------------------- Idempotency
CREATE TABLE idempotency_key (
  id          BIGSERIAL   PRIMARY KEY,
  tenant_id   BIGINT      NOT NULL REFERENCES tenant(id),
  scope       TEXT        NOT NULL,   -- endpoint identifier
  key         TEXT        NOT NULL,
  response    JSONB,                  -- first response; replays return verbatim
  created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (tenant_id, scope, key)
);

-- ---------------------------------------------------------------- RLS
-- App connects as ignition_app (not table owner); policies apply.
-- current_setting(..., true): unset → NULL → comparison false → fail-closed
DO $$
DECLARE t TEXT;
BEGIN
  FOREACH t IN ARRAY ARRAY[
    'app','bot','tenant_entitlement_override','subscription',
    'kol','channel','campaign','reward_item','link','player','attribution',
    'claim_code','billable_event','ledger_entry','invoice','invoice_line',
    'risk_signal','risk_verdict','idempotency_key'
  ] LOOP
    EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
    -- FORCE is required: by default table owners bypass their own policies, and the
    -- migration runner is usually owner. Without FORCE, owner queries see all tenants.
    -- (FORCE does not stop BYPASSRLS roles — only role choice fixes that.)
    EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', t);
    EXECUTE format(
      'CREATE POLICY tenant_isolation ON %I USING (tenant_id = current_setting(''app.tenant_id'', true)::bigint)',
      t);
    EXECUTE format('GRANT SELECT, INSERT, UPDATE, DELETE ON %I TO ignition_app', t);
  END LOOP;
END
$$;

-- pricing_config needs its own policy: tenant_id IS NULL is global default pricing;
-- the generic policy hides those rows and pricing silently returns 0 — worse than an error.
ALTER TABLE pricing_config ENABLE ROW LEVEL SECURITY;
ALTER TABLE pricing_config FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON pricing_config
  USING (tenant_id IS NULL
         OR tenant_id = current_setting('app.tenant_id', true)::bigint);
DO $$
BEGIN
  EXECUTE 'GRANT SELECT, INSERT, UPDATE, DELETE ON pricing_config TO ignition_app';
EXCEPTION WHEN undefined_object THEN NULL;
END
$$;

DO $$
DECLARE s TEXT := current_schema();
BEGIN
  EXECUTE format('GRANT USAGE ON SCHEMA %I TO ignition_app', s);
  EXECUTE 'GRANT SELECT ON tenant, plan, plan_entitlement, template TO ignition_app';
  EXECUTE format('GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA %I TO ignition_app', s);
EXCEPTION WHEN undefined_object THEN
  RAISE WARNING 'ignition_app role does not exist, skipping grants';
END
$$;

-- Ledger append-only: app role may read and insert only (constraint C3 at DB layer)
DO $$
BEGIN
  EXECUTE 'REVOKE UPDATE, DELETE ON ledger_entry FROM ignition_app';
EXCEPTION WHEN undefined_object THEN NULL;
END
$$;

-- Second lock on append-only ledger: REVOKE affects only the revoked role; table owner is unaffected.
-- Triggers apply to all roles, including owner and BYPASSRLS — on hosted DBs where the app often
-- connects as owner, this is the only DB-layer guarantee left for C3.
CREATE OR REPLACE FUNCTION ledger_is_append_only() RETURNS TRIGGER
LANGUAGE plpgsql AS $$
BEGIN
  RAISE EXCEPTION 'ledger is append-only: ledger_entry does not allow % — reverse with a compensating entry (ledger::Txn::reverse)', TG_OP;
END
$$;

CREATE TRIGGER ledger_entry_append_only
  BEFORE UPDATE OR DELETE ON ledger_entry
  FOR EACH ROW EXECUTE FUNCTION ledger_is_append_only();

COMMIT;
