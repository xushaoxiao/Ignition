-- Growth 初始化迁移
-- 对应设计文档 §3 领域模型 / §5 计费系统 / §8 多租户
--
-- 约定：
--   1. 所有租户数据表带 tenant_id，并启用 RLS（应用以 growth_app 角色连接，受策略约束）
--   2. ledger_entry 表对应用角色只授予 SELECT/INSERT —— 账本不可改（设计约束 C3）
--   3. 金额一律用 BIGINT 存最小货币单位（cent），禁止浮点

BEGIN;

-- ---------------------------------------------------------------- 应用角色
DO $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'growth_app') THEN
    CREATE ROLE growth_app LOGIN PASSWORD 'growth_app';
  END IF;
END
$$;

-- ---------------------------------------------------------------- 枚举
CREATE TYPE attribution_method AS ENUM (
  'deterministic_code',   -- 领奖码核销         confidence=100  billable
  'install_referrer',     -- Play Install Referrer  100         billable
  'universal_link',       -- 已安装用户直接唤起      100         billable
  'clipboard_match',      -- 剪贴板匹配             60          NOT billable
  'probabilistic'         -- 指纹/时间窗匹配        30          NOT billable
);

CREATE TYPE billable_status AS ENUM (
  'pending',   -- 已接收，在 hold 冷静期内
  'held',      -- 风控暂缓，等待人工复核
  'cleared',   -- 已放行，可计入账单
  'billed',    -- 已开票
  'reversed',  -- 已冲正（退款 / 事后判定作弊）
  'rejected'   -- 判定无效，不计费
);

CREATE TYPE claim_status AS ENUM ('issued', 'redeemed', 'expired', 'voided');

CREATE TYPE subscription_status AS ENUM (
  'trialing', 'active', 'past_due', 'paused', 'canceled'
);

-- ---------------------------------------------------------------- 租户与订阅
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
  postback_secret_enc        BYTEA       NOT NULL, -- KMS 信封加密，禁止明文
  attribution_policy_version TEXT        NOT NULL DEFAULT 'v1',
  created_at                 TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ON app (tenant_id);

CREATE TABLE bot (
  id            BIGSERIAL PRIMARY KEY,
  tenant_id     BIGINT      NOT NULL REFERENCES tenant(id),
  username      TEXT        NOT NULL,
  token_enc     BYTEA       NOT NULL,  -- KMS 信封加密，禁止进日志
  created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (tenant_id, username)
);

CREATE TABLE plan (
  id          BIGSERIAL PRIMARY KEY,
  code        TEXT        NOT NULL UNIQUE,   -- 'free' | 'starter' | 'pro'
  name        TEXT        NOT NULL,
  created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- 能力门控（设计约束 C4）：禁止 if plan == 'pro' 硬编码
CREATE TABLE plan_entitlement (
  plan_id BIGINT NOT NULL REFERENCES plan(id),
  key     TEXT   NOT NULL,   -- 'channel.discord' | 'analytics.cohort' | ...
  value   JSONB  NOT NULL,   -- true | {"limit": 5}
  PRIMARY KEY (plan_id, key)
);

-- 销售谈判用的例外，没有这张表承诺就会变成硬编码
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
  grace_until          TIMESTAMPTZ,        -- past_due 宽限，期内不断服
  created_at           TIMESTAMPTZ         NOT NULL DEFAULT now(),
  updated_at           TIMESTAMPTZ         NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX ON subscription (tenant_id) WHERE status <> 'canceled';

-- 定价配置：必须带生效区间，涨价是加记录不是改记录（设计文档 §5.3）
CREATE TABLE pricing_config (
  id                 BIGSERIAL   PRIMARY KEY,
  tenant_id          BIGINT      REFERENCES tenant(id),  -- NULL = 全局默认
  effective_from     TIMESTAMPTZ NOT NULL,
  effective_to       TIMESTAMPTZ,                        -- NULL = 当前生效
  platform_fee_cents BIGINT      NOT NULL DEFAULT 0,
  cpa_rates          JSONB       NOT NULL DEFAULT '{}',  -- {"activation": 200}
  monthly_cap_cents  BIGINT,                             -- NULL = 无封顶
  currency           CHAR(3)     NOT NULL DEFAULT 'USD',
  created_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ON pricing_config (tenant_id, effective_from DESC);

-- ---------------------------------------------------------------- 渠道侧
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
  config_schema JSONB       NOT NULL,          -- JSON Schema，保存 campaign 时校验
  created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE campaign (
  id            BIGSERIAL PRIMARY KEY,
  tenant_id     BIGINT      NOT NULL REFERENCES tenant(id),
  app_id        BIGINT      NOT NULL REFERENCES app(id),
  template_id   BIGINT      NOT NULL REFERENCES template(id),
  name          TEXT        NOT NULL,
  config        JSONB       NOT NULL DEFAULT '{}',
  daily_play_limit INT      NOT NULL DEFAULT 3,   -- 风控 L1
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
  version       BIGINT      NOT NULL DEFAULT 0,  -- 乐观锁
  payload       JSONB       NOT NULL DEFAULT '{}'
);
CREATE INDEX ON reward_item (tenant_id, campaign_id);

-- 投放位：tracking_id 必须不可枚举，否则 KOL 之间可互相猜链接刷量
CREATE TABLE link (
  id            BIGSERIAL PRIMARY KEY,
  tenant_id     BIGINT      NOT NULL REFERENCES tenant(id),
  campaign_id   BIGINT      NOT NULL REFERENCES campaign(id),
  channel_id    BIGINT      NOT NULL REFERENCES channel(id),
  kol_id        BIGINT      NOT NULL REFERENCES kol(id),
  tracking_id   TEXT        NOT NULL UNIQUE,   -- 10 位 base62 随机
  created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ON link (tenant_id, campaign_id);

-- ---------------------------------------------------------------- 用户与归因
CREATE TABLE player (
  id            BIGSERIAL   PRIMARY KEY,
  tenant_id     BIGINT      NOT NULL REFERENCES tenant(id),
  tg_user_id    BIGINT      NOT NULL,
  app_user_id   TEXT,                            -- 核销时绑定，全链路唯一缝合点
  device_ids    TEXT[]      NOT NULL DEFAULT '{}',
  tg_is_premium BOOLEAN     NOT NULL DEFAULT false,
  tg_username   TEXT,
  first_seen_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (tenant_id, tg_user_id)
);
CREATE UNIQUE INDEX ON player (tenant_id, app_user_id) WHERE app_user_id IS NOT NULL;

-- 归因记录 —— 系统的信任基石（设计文档 §3）
CREATE TABLE attribution (
  id             BIGSERIAL          PRIMARY KEY,
  tenant_id      BIGINT             NOT NULL REFERENCES tenant(id),
  player_id      BIGINT             NOT NULL REFERENCES player(id),
  kol_id         BIGINT             NOT NULL REFERENCES kol(id),
  campaign_id    BIGINT             NOT NULL REFERENCES campaign(id),
  link_id        BIGINT             NOT NULL REFERENCES link(id),

  method         attribution_method NOT NULL,
  confidence     SMALLINT           NOT NULL CHECK (confidence BETWEEN 0 AND 100),
  -- 冗余字段，但必须存：计费规则会变，已开账单的判定依据必须冻结
  is_billable    BOOLEAN            NOT NULL,

  policy_version TEXT               NOT NULL,  -- 对应公开文档，用于申诉复算
  touch_at       TIMESTAMPTZ        NOT NULL,  -- 首次触点
  attributed_at  TIMESTAMPTZ        NOT NULL,  -- 归因成立时刻
  locked_until   TIMESTAMPTZ        NOT NULL,  -- 归因锁定期
  evidence       JSONB              NOT NULL,  -- 判定依据快照，只增不改

  -- 单归因：一个 Player 只归属一个 KOL，MVP 不做多归因拆分
  UNIQUE (tenant_id, player_id)
);
CREATE INDEX ON attribution (tenant_id, kol_id, attributed_at);

CREATE TABLE claim_code (
  id            BIGSERIAL   PRIMARY KEY,
  tenant_id     BIGINT      NOT NULL REFERENCES tenant(id),
  code          TEXT        NOT NULL,   -- 8 位，排除 0/O/1/I/l
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

-- ---------------------------------------------------------------- 计费
-- 可计费事件 —— 收入的原子（设计文档 §3）
CREATE TABLE billable_event (
  id             BIGSERIAL       PRIMARY KEY,
  tenant_id      BIGINT          NOT NULL REFERENCES tenant(id),
  attribution_id BIGINT          NOT NULL REFERENCES attribution(id),
  event_type     TEXT            NOT NULL,   -- 'activation' | 'iap_purchase'
  external_id    TEXT            NOT NULL,   -- 主 App 侧唯一 ID，幂等键

  status         billable_status NOT NULL DEFAULT 'pending',
  amount_cents   BIGINT          NOT NULL CHECK (amount_cents >= 0),
  currency       CHAR(3)         NOT NULL DEFAULT 'USD',
  over_cap       BOOLEAN         NOT NULL DEFAULT false,

  occurred_at    TIMESTAMPTZ     NOT NULL,   -- 业务发生时间（主 App 报的）
  received_at    TIMESTAMPTZ     NOT NULL DEFAULT now(),
  hold_until     TIMESTAMPTZ     NOT NULL,   -- 冷静期结束
  cleared_at     TIMESTAMPTZ,
  billed_at      TIMESTAMPTZ,
  invoice_id     BIGINT,
  status_reason  TEXT,

  -- 幂等的物理保证（设计约束 C6）
  UNIQUE (tenant_id, event_type, external_id)
);
CREATE INDEX ON billable_event (tenant_id, status, hold_until);
CREATE INDEX ON billable_event (tenant_id, cleared_at) WHERE status = 'cleared';

-- 复式记账（设计约束 C3）：只可追加，不可 UPDATE / DELETE
CREATE TABLE ledger_entry (
  id           BIGSERIAL   PRIMARY KEY,
  tenant_id    BIGINT      NOT NULL REFERENCES tenant(id),
  txn_id       UUID        NOT NULL,          -- 同一笔交易的分录共享
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

-- ---------------------------------------------------------------- 风控
-- L2 信号：第一天就存，这些数据事后无法补采
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
  latency_ms  BIGINT,                 -- 上一阶段到本阶段耗时
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

-- ---------------------------------------------------------------- 幂等
CREATE TABLE idempotency_key (
  id          BIGSERIAL   PRIMARY KEY,
  tenant_id   BIGINT      NOT NULL REFERENCES tenant(id),
  scope       TEXT        NOT NULL,   -- 接口标识
  key         TEXT        NOT NULL,
  response    JSONB,                  -- 首次结果，重放时原样返回
  created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (tenant_id, scope, key)
);

-- ---------------------------------------------------------------- RLS
-- 应用以 growth_app 连接（非表 owner），策略生效。
-- 用 current_setting(..., true)：未设置时返回 NULL → 比较为 false → fail-closed
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
    EXECUTE format(
      'CREATE POLICY tenant_isolation ON %I USING (tenant_id = current_setting(''app.tenant_id'', true)::bigint)',
      t);
    EXECUTE format('GRANT SELECT, INSERT, UPDATE, DELETE ON %I TO growth_app', t);
  END LOOP;
END
$$;

-- pricing_config 需要单独的策略：tenant_id IS NULL 表示全局默认定价，
-- 通用策略会把它挡在外面，导致计价查不到行、金额静默算成 0 —— 那比报错更糟。
ALTER TABLE pricing_config ENABLE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON pricing_config
  USING (tenant_id IS NULL
         OR tenant_id = current_setting('app.tenant_id', true)::bigint);
GRANT SELECT, INSERT, UPDATE, DELETE ON pricing_config TO growth_app;

GRANT SELECT ON tenant, plan, plan_entitlement, template TO growth_app;
GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO growth_app;

-- 账本不可改：应用角色只能读和追加（设计约束 C3 的数据库层强制）
REVOKE UPDATE, DELETE ON ledger_entry FROM growth_app;

COMMIT;
