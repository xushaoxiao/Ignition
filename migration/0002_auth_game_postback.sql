-- 0002 —— API Key 认证、抽奖记录、变现回传
--
-- 对应 README「未实现」清单的 1/3/4/5/6/7/8 项：
--   · api_key           替换掉 X-Tenant-ID 占位认证
--   · game_play         服务端权威抽奖 + 幂等
--   · purchase_event    变现回传的分析流落地
--   · 两个 SECURITY DEFINER 函数  解决「认证发生在租户上下文建立之前」

-- 目标 schema，与 0001 一致。覆盖方式：psql -v schema=xxx
\if :{?schema}
\else
  \set schema ignition
\endif
SET search_path TO :"schema", public;

BEGIN;

-- ---------------------------------------------------------------- API Key
-- 主 App 服务端的长期凭据。secret 加密存储，格式见 src/secrets.rs。
CREATE TABLE api_key (
  id           BIGSERIAL   PRIMARY KEY,
  tenant_id    BIGINT      NOT NULL REFERENCES tenant(id),
  key_id       TEXT        NOT NULL UNIQUE,   -- 明文标识，随请求发送
  secret_enc   BYTEA       NOT NULL,          -- 签名密钥，禁止进日志
  label        TEXT        NOT NULL,
  -- 权限缺省为空：不显式授予就没有。给主 App 的密钥通常只需要 {redeem}，
  -- 即便泄漏也伪造不了变现回传。
  scopes       TEXT[]      NOT NULL DEFAULT '{}',
  revoked_at   TIMESTAMPTZ,
  last_used_at TIMESTAMPTZ,
  created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ON api_key (tenant_id);

-- postback 与 redeem 统一走 api_key + scope，不再单独维护一份 postback secret。
-- 两套密钥意味着两套轮换流程和两个可能过期的地方，而 scope 已经能表达
-- 「这把钥匙只能核销，不能报账」。
ALTER TABLE app DROP COLUMN postback_secret_enc;

-- ---------------------------------------------------------------- 抽奖
CREATE TABLE game_play (
  id              BIGSERIAL   PRIMARY KEY,
  tenant_id       BIGINT      NOT NULL REFERENCES tenant(id),
  player_id       BIGINT      NOT NULL REFERENCES player(id),
  campaign_id     BIGINT      NOT NULL REFERENCES campaign(id),
  -- 客户端生成。断网重试与用户狂点都会重复提交，没有它就会重复扣奖池。
  idempotency_key TEXT        NOT NULL,
  reward_item_id  BIGINT      NOT NULL REFERENCES reward_item(id),
  segment_index   INT         NOT NULL,   -- 转盘扇区下标，前端据此停针
  result          JSONB       NOT NULL DEFAULT '{}',
  created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (tenant_id, player_id, campaign_id, idempotency_key)
);
CREATE INDEX ON game_play (tenant_id, player_id, campaign_id, created_at);

-- 一次抽奖只签发一个领奖码。每个码都是一个待核销的归因载体，
-- 重复签发等于让一次抽奖换来多次可计费的核销。
ALTER TABLE claim_code ADD COLUMN game_play_id BIGINT REFERENCES game_play(id);
CREATE UNIQUE INDEX ON claim_code (tenant_id, game_play_id) WHERE game_play_id IS NOT NULL;

-- ---------------------------------------------------------------- 变现回传
-- 分析流与计费流物理分离：回传一律落这张表，只有在归因可计费 **且** 配了
-- iap_purchase 单价时，才额外产生一条 billable_event。
CREATE TABLE purchase_event (
  id                BIGSERIAL   PRIMARY KEY,
  tenant_id         BIGINT      NOT NULL REFERENCES tenant(id),
  attribution_id    BIGINT      REFERENCES attribution(id),  -- NULL = 非我方渠道用户
  app_user_id       TEXT        NOT NULL,
  transaction_id    TEXT        NOT NULL,   -- 主 App 侧唯一 ID，幂等键
  amount_cents      BIGINT      NOT NULL CHECK (amount_cents >= 0),
  currency          CHAR(3)     NOT NULL DEFAULT 'USD',
  occurred_at       TIMESTAMPTZ NOT NULL,
  received_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
  billable_event_id BIGINT      REFERENCES billable_event(id),
  UNIQUE (tenant_id, transaction_id)
);
CREATE INDEX ON purchase_event (tenant_id, attribution_id);

-- ---------------------------------------------------------------- 结算
-- 冲正额只能被抵扣一次。没有这个字段，每期结算都会把同一笔冲正再减一遍。
ALTER TABLE billable_event ADD COLUMN credited_invoice_id BIGINT REFERENCES invoice(id);
CREATE INDEX ON billable_event (tenant_id, status)
  WHERE status = 'reversed' AND credited_invoice_id IS NULL;

-- ---------------------------------------------------------------- 引导期查询
-- 认证与租户解析发生在租户上下文建立之前 —— 这是个鸡生蛋问题：RLS 需要
-- app.tenant_id，而 tenant_id 正是这两次查询要解出来的东西。
--
-- 解法不是给应用角色开表级读权限（那等于把 RLS 的兜底作用让掉），
-- 而是开两个 SECURITY DEFINER 函数：输入是不可枚举的随机 ID，
-- 输出只有解析所必需的字段，且都不是租户业务数据。
-- SECURITY DEFINER 函数必须钉死 search_path，否则调用方可以把自己的 schema
-- 插到前面，让函数体里的 `api_key` 解析到一张伪造的同名表 —— 那等于把
-- 「以定义者身份执行」直接送给调用方。
--
-- 钉的是**函数实际所在的 schema**（current_schema()），不是写死的 public：
-- 这些表落在 ignition schema 里，钉 public 会让函数在运行时找不到表。
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

-- ---------------------------------------------------------------- 权限
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

-- api_key 不开表级权限：应用只能经上面的函数按 key_id 精确解析一行，
-- 拿不到「列出所有密钥」这个能力。
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
  RAISE WARNING 'ignition_app 角色不存在，已跳过授权';
END
$$;

COMMIT;
