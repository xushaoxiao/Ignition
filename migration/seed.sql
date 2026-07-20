-- 演示数据：一个租户 + KOL + 活动 + 投放位 + 一个待核销的领奖码。
-- 用于本地跑通全链路。
--
-- 这里**不包含** Bot token 与 API Key：它们是密文，明文写进版本库就失去了
-- 加密存储的意义。用 `make secrets` 生成并写入（见 Makefile）。
\if :{?schema}
\else
  \set schema ignition
\endif
SET search_path TO :"schema", public;

BEGIN;

-- 演示数据全部属于租户 1。启用了 FORCE ROW LEVEL SECURITY 之后，策略对表
-- owner 同样生效，而只有 USING 没有 WITH CHECK 的策略在 INSERT 时会拿 USING
-- 当校验用 —— 不设这个上下文，灌数据会被自己的租户隔离策略挡下来。
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

-- 全局默认定价：activation 每笔 $2.00，月度封顶 $500
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

INSERT INTO link (id, tenant_id, campaign_id, channel_id, kol_id, tracking_id)
VALUES (1, 1, 1, 1, 1, 'aB3xY9zK1m') ON CONFLICT (id) DO NOTHING;

-- 一个已完成游戏、等待核销的用户。
-- first_seen 设为 2 分钟前，避免触发 too_fast 风控规则。
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
