-- Demo data: one tenant + KOL + campaign + link + one pending claim code.
-- For running the full local path end-to-end.
--
-- Does **not** include bot token or API key: they are ciphertext; plaintext in the repo
-- defeats encrypted storage. Use `make secrets` to generate and insert (see Makefile).
\if :{?schema}
\else
  \set schema ignition
\endif
SET search_path TO :"schema", public;

BEGIN;

-- All demo rows belong to tenant 1. With FORCE ROW LEVEL SECURITY, policies apply to the
-- table owner too; policies with USING but no WITH CHECK use USING on INSERT — without
-- this context, seed data is blocked by tenant isolation.
SELECT set_config('app.tenant_id', '1', true);

INSERT INTO tenant (id, name, slug) VALUES (1, 'Demo Tenant', 'demo')
  ON CONFLICT (id) DO NOTHING;

INSERT INTO app (id, tenant_id, name, bundle_id, package_name,
                 store_url_ios, store_url_android)
VALUES (1, 1, 'Demo App', 'com.demo.app', 'com.demo.app',
        'https://apps.apple.com/app/id000000000',
        'https://play.google.com/store/apps/details?id=com.demo.app')
  ON CONFLICT (id) DO NOTHING;

INSERT INTO plan (id, code, name) VALUES
  (1, 'starter', 'Starter'), (2, 'pro', 'Pro')
  ON CONFLICT (id) DO NOTHING;

INSERT INTO plan_entitlement (plan_id, key, value) VALUES
  (1, 'channel.count',        '{"limit": 1}'),
  (1, 'analytics.basic',      'true'),
  (1, 'billing.performance',  'true'),
  (2, 'channel.count',        '{"limit": 50}'),
  (2, 'channel.discord',      'true'),
  (2, 'analytics.basic',      'true'),
  (2, 'analytics.cohort',     'true'),
  (2, 'branding.whitelabel',  'true'),
  (2, 'billing.performance',  'true')
  ON CONFLICT DO NOTHING;

INSERT INTO subscription
  (id, tenant_id, plan_id, status, current_period_start, current_period_end)
VALUES (1, 1, 1, 'active', date_trunc('month', now()), date_trunc('month', now()) + interval '1 month')
  ON CONFLICT (id) DO NOTHING;

-- Global default pricing: $2.00 per activation, $500 monthly cap
INSERT INTO pricing_config
  (id, tenant_id, effective_from, platform_fee_cents, cpa_rates, monthly_cap_cents)
VALUES (1, NULL, now() - interval '1 day', 9900, '{"activation": 200}', 50000)
  ON CONFLICT (id) DO NOTHING;

INSERT INTO kol (id, tenant_id, display_name, tg_user_id)
VALUES (1, 1, 'Demo KOL', 555001) ON CONFLICT (id) DO NOTHING;

INSERT INTO channel (id, tenant_id, kol_id, platform, external_id, name)
VALUES (1, 1, 1, 'telegram', '-1001234567890', 'Demo Group')
  ON CONFLICT (id) DO NOTHING;

INSERT INTO template (id, code, name, config_schema)
VALUES (1, 'lucky_wheel', '每日签到大转盘',
        '{"type":"object","properties":{"segments":{"type":"integer","minimum":4}}}')
  ON CONFLICT (id) DO NOTHING;

INSERT INTO campaign (id, tenant_id, app_id, template_id, name, config, status, starts_at)
VALUES (1, 1, 1, 1, 'Demo Wheel', '{"segments": 8}', 'active', now() - interval '1 day')
  ON CONFLICT (id) DO NOTHING;

INSERT INTO reward_item (id, tenant_id, campaign_id, label, weight, remaining) VALUES
  (1, 1, 1, '100 金币', 70, 100000),
  (2, 1, 1, '500 金币', 25, 10000),
  (3, 1, 1, '限定皮肤',  5, 100)
  ON CONFLICT (id) DO NOTHING;

-- Second demo campaign: the daily budget-decision game (template 6, see migration 0005).
-- Same prize/claim/redeem path as the wheel — the decision round only sits in front of it.
-- `promo` is the soft prompt shown to high scorers; it lives in campaign config, never in the
-- scenario catalog, so the customer owns that claim and the game's own copy stays neutral.
INSERT INTO campaign (id, tenant_id, app_id, template_id, name, config, status, starts_at)
VALUES (2, 1, 1, 6, 'Demo Daily Budget',
        '{"promo": {"text": "想了解真实的借款与理财工具？关注官方频道", "url": "https://t.me/demo_official", "min_credit": 700}}',
        'active', now() - interval '1 day')
  ON CONFLICT (id) DO NOTHING;

INSERT INTO reward_item (id, tenant_id, campaign_id, label, weight, remaining) VALUES
  (4, 1, 2, '记账本周边', 60, 100000),
  (5, 1, 2, '理财课程券', 35, 5000),
  (6, 1, 2, '年度会员',   5, 100)
  ON CONFLICT (id) DO NOTHING;

INSERT INTO link (id, tenant_id, campaign_id, channel_id, kol_id, tracking_id) VALUES
  (1, 1, 1, 1, 1, 'aB3xY9zK1m'),
  (2, 1, 2, 1, 1, 'dQ7wN2pR5t')
  ON CONFLICT (id) DO NOTHING;

-- One user who finished the game and awaits redemption.
-- first_seen 2 minutes ago to avoid triggering the too_fast risk rule.
INSERT INTO player (id, tenant_id, tg_user_id, tg_username, first_seen_at)
VALUES (1, 1, 123456789, 'demo_user', now() - interval '2 minutes')
  ON CONFLICT (id) DO NOTHING;

INSERT INTO claim_code
  (id, tenant_id, code, player_id, campaign_id, link_id, kol_id, status, issued_at, expires_at)
VALUES (1, 1, 'DEMA2345', 1, 1, 1, 1, 'issued',
        now() - interval '2 minutes', now() + interval '24 hours')
  ON CONFLICT (id) DO NOTHING;

SELECT setval('tenant_id_seq',      (SELECT max(id) FROM tenant));
SELECT setval('app_id_seq',         (SELECT max(id) FROM app));
SELECT setval('plan_id_seq',        (SELECT max(id) FROM plan));
SELECT setval('subscription_id_seq',(SELECT max(id) FROM subscription));
SELECT setval('pricing_config_id_seq',(SELECT max(id) FROM pricing_config));
SELECT setval('kol_id_seq',         (SELECT max(id) FROM kol));
SELECT setval('channel_id_seq',     (SELECT max(id) FROM channel));
SELECT setval('template_id_seq',    (SELECT max(id) FROM template));
SELECT setval('campaign_id_seq',    (SELECT max(id) FROM campaign));
SELECT setval('reward_item_id_seq', (SELECT max(id) FROM reward_item));
SELECT setval('link_id_seq',        (SELECT max(id) FROM link));
SELECT setval('player_id_seq',      (SELECT max(id) FROM player));
SELECT setval('claim_code_id_seq',  (SELECT max(id) FROM claim_code));

COMMIT;
