-- 0002 — API Key auth, game play records, monetisation postback
--
-- Covers README "not yet implemented" items 1/3/4/5/6/7/8:
--   · api_key           replaces X-Tenant-ID placeholder auth
--   · game_play         server-authoritative play + idempotency
--   · purchase_event    monetisation postback analytics landing table
--   · two SECURITY DEFINER functions  solve "auth before tenant context exists"

-- Target schema, same as 0001. Override: psql -v schema=xxx
\if :{?schema}
\else
  \set schema ignition
\endif
SET search_path TO :"schema", public;

BEGIN;

-- ---------------------------------------------------------------- API Key
-- Long-lived main-app server credential. secret stored encrypted; format in src/secrets.rs.
CREATE TABLE api_key (
  id           BIGSERIAL   PRIMARY KEY,
  tenant_id    BIGINT      NOT NULL REFERENCES tenant(id),
  key_id       TEXT        NOT NULL UNIQUE,   -- plaintext identifier sent with requests
  secret_enc   BYTEA       NOT NULL,          -- signing secret; never log
  label        TEXT        NOT NULL,
  -- Empty scopes by default: no grant unless explicit. Main-app keys usually need {redeem} only —
  -- even if leaked, monetisation postback cannot be forged.
  scopes       TEXT[]      NOT NULL DEFAULT '{}',
  revoked_at   TIMESTAMPTZ,
  last_used_at TIMESTAMPTZ,
  created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ON api_key (tenant_id);

-- Postback and redeem both use api_key + scope; no separate postback secret table.
-- Two secret systems means two rotation flows and two expiry surfaces; scope already expresses
-- "this key can redeem but not post revenue".
ALTER TABLE app DROP COLUMN postback_secret_enc;

-- ---------------------------------------------------------------- Game play
CREATE TABLE game_play (
  id              BIGSERIAL   PRIMARY KEY,
  tenant_id       BIGINT      NOT NULL REFERENCES tenant(id),
  player_id       BIGINT      NOT NULL REFERENCES player(id),
  campaign_id     BIGINT      NOT NULL REFERENCES campaign(id),
  -- Client-generated. Offline retries and rapid taps resubmit; without this, prize pool debits repeat.
  idempotency_key TEXT        NOT NULL,
  reward_item_id  BIGINT      NOT NULL REFERENCES reward_item(id),
  segment_index   INT         NOT NULL,   -- wheel segment index; frontend stops pointer here
  result          JSONB       NOT NULL DEFAULT '{}',
  created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (tenant_id, player_id, campaign_id, idempotency_key)
);
CREATE INDEX ON game_play (tenant_id, player_id, campaign_id, created_at);

-- One claim code per play. Each code is a pending attribution carrier;
-- re-issuing turns one play into multiple billable redemptions.
ALTER TABLE claim_code ADD COLUMN game_play_id BIGINT REFERENCES game_play(id);
CREATE UNIQUE INDEX ON claim_code (tenant_id, game_play_id) WHERE game_play_id IS NOT NULL;

-- ---------------------------------------------------------------- Monetisation postback
-- Analytics and billing streams are physically separate: postbacks land here; a billable_event
-- is created only when attribution is billable **and** iap_purchase rate is configured.
CREATE TABLE purchase_event (
  id                BIGSERIAL   PRIMARY KEY,
  tenant_id         BIGINT      NOT NULL REFERENCES tenant(id),
  attribution_id    BIGINT      REFERENCES attribution(id),  -- NULL = not from our channel
  app_user_id       TEXT        NOT NULL,
  transaction_id    TEXT        NOT NULL,   -- main-app unique id; idempotency key
  amount_cents      BIGINT      NOT NULL CHECK (amount_cents >= 0),
  currency          CHAR(3)     NOT NULL DEFAULT 'USD',
  occurred_at       TIMESTAMPTZ NOT NULL,
  received_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
  billable_event_id BIGINT      REFERENCES billable_event(id),
  UNIQUE (tenant_id, transaction_id)
);
CREATE INDEX ON purchase_event (tenant_id, attribution_id);

-- ---------------------------------------------------------------- Settlement
-- Reversal credit can be applied only once. Without this field, each period re-subtracts the same reversal.
ALTER TABLE billable_event ADD COLUMN credited_invoice_id BIGINT REFERENCES invoice(id);
CREATE INDEX ON billable_event (tenant_id, status)
  WHERE status = 'reversed' AND credited_invoice_id IS NULL;

-- ---------------------------------------------------------------- Bootstrap queries
-- Auth and tenant resolution happen before tenant context exists — chicken and egg: RLS needs
-- app.tenant_id, but tenant_id is exactly what these queries must resolve.
--
-- The fix is not table-level read grants for the app role (that defeats RLS as a backstop), but
-- two SECURITY DEFINER functions: inputs are non-enumerable random IDs; outputs are only fields
-- required for resolution, none of which are tenant business data.
-- SECURITY DEFINER functions must pin search_path or callers can prepend their schema and make
-- `api_key` in the function body resolve to a forged same-named table — handing "run as definer" to the caller.
--
-- Pin the **schema where the function lives** (current_schema()), not hard-coded public:
-- tables live in the ignition schema; pinning public makes functions fail at runtime.
DO $$
DECLARE s TEXT := current_schema();
BEGIN
  EXECUTE format($fn$
    CREATE FUNCTION auth_resolve_api_key(p_key_id TEXT)
    RETURNS TABLE (id BIGINT, tenant_id BIGINT, secret_enc BYTEA, scopes TEXT[])
    LANGUAGE sql SECURITY DEFINER SET search_path = %I, public AS $body$
      SELECT k.id, k.tenant_id, k.secret_enc, k.scopes
        FROM api_key k
       WHERE k.key_id = p_key_id AND k.revoked_at IS NULL;
    $body$;
  $fn$, s);

  EXECUTE format($fn$
    CREATE FUNCTION auth_resolve_tracking(p_tracking_id TEXT)
    RETURNS TABLE (
      tenant_id BIGINT, campaign_id BIGINT, link_id BIGINT, kol_id BIGINT,
      bot_token_enc BYTEA, sub_status subscription_status, grace_until TIMESTAMPTZ
    )
    LANGUAGE sql SECURITY DEFINER SET search_path = %I, public AS $body$
      SELECT l.tenant_id, l.campaign_id, l.id, l.kol_id,
             b.token_enc, s.status, s.grace_until
        FROM link l
        JOIN campaign c ON c.id = l.campaign_id
        LEFT JOIN bot b ON b.tenant_id = l.tenant_id
        LEFT JOIN subscription s ON s.tenant_id = l.tenant_id AND s.status <> 'canceled'
       WHERE l.tracking_id = p_tracking_id AND c.status = 'active'
       LIMIT 1;
    $body$;
  $fn$, s);
END
$$;

-- ---------------------------------------------------------------- Permissions
DO $$
DECLARE t TEXT;
BEGIN
  FOREACH t IN ARRAY ARRAY['game_play','purchase_event'] LOOP
    EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
    EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', t);
    EXECUTE format(
      'CREATE POLICY tenant_isolation ON %I USING (tenant_id = current_setting(''app.tenant_id'', true)::bigint)',
      t);
    BEGIN
      EXECUTE format('GRANT SELECT, INSERT, UPDATE, DELETE ON %I TO ignition_app', t);
    EXCEPTION WHEN undefined_object THEN NULL;
    END;
  END LOOP;
END
$$;

-- api_key has no table-level grant: app resolves one row by key_id via the function above —
-- cannot list all keys.
ALTER TABLE api_key ENABLE ROW LEVEL SECURITY;
ALTER TABLE api_key FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON api_key
  USING (tenant_id = current_setting('app.tenant_id', true)::bigint);

DO $$
DECLARE s TEXT := current_schema();
BEGIN
  EXECUTE 'GRANT SELECT, UPDATE ON api_key TO ignition_app';
  EXECUTE format('GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA %I TO ignition_app', s);
  EXECUTE 'GRANT EXECUTE ON FUNCTION auth_resolve_api_key(TEXT)  TO ignition_app';
  EXECUTE 'GRANT EXECUTE ON FUNCTION auth_resolve_tracking(TEXT) TO ignition_app';
EXCEPTION WHEN undefined_object THEN
  RAISE WARNING 'ignition_app role does not exist, skipping grants';
END
$$;

COMMIT;
