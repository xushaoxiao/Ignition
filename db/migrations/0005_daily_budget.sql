-- 0005 — Daily budget decision game (`daily_budget`)
--
-- The five existing games are animation skins over one server-decided prize draw. This one is
-- **not** a skin: the player makes a decision, the server scores it, and the score accumulates
-- across days into a streak and a "虚拟信用分". It is an engagement layer that sits *in front of*
-- the prize draw — the billable path (play → claim code → redeem) is untouched, so nothing here
-- can move an invoice.
--
-- Two tables:
--   · daily_scenario  platform reference catalog (like `template`) — no tenant_id, no RLS
--   · daily_round     one player's decision for one day — tenant-scoped, RLS, one row per day
--
-- Scoring lives in `options`, server-side only. The API projects key + label and never the
-- delta — same rule as the prize pool, where `Segment` exposes id + label but never weight or
-- stock: publishing the score table turns a decision game into a lookup table.

-- Target schema, same as 0001..0004. Override: psql -v schema=xxx
\if :{?schema}
\else
  \set schema ignition
\endif
SET search_path TO :"schema", public;

BEGIN;

-- ---------------------------------------------------------------- Scenario catalog
-- Platform reference data, not tenant data: every tenant running `daily_budget` gets the same
-- catalog, the same way every tenant gets the same `template` rows. A tenant-authored scenario
-- library is a console feature, not a schema change — it would add a nullable tenant_id here.
CREATE TABLE daily_scenario (
  id          BIGSERIAL   PRIMARY KEY,
  code        TEXT        NOT NULL UNIQUE,
  locale      TEXT        NOT NULL DEFAULT 'zh-CN',
  title       TEXT        NOT NULL,   -- 场景标题
  prompt      TEXT        NOT NULL,   -- 场景描述
  -- [{ "key", "label", "delta", "verdict", "tip" }]
  --   delta   信用分变化，**永不下发给客户端**
  --   verdict 即时反馈
  --   tip     科普（利息成本 / 信用影响）
  options     JSONB       NOT NULL,
  created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- ---------------------------------------------------------------- Rounds
-- `credit_after` / `streak_after` are stored, not recomputed, even though both are derivable from
-- the round history. Same reason `attribution.is_billable` is stored redundantly: these are the
-- numbers the player was actually shown and ranked by. Rebalancing a scenario's `delta` later must
-- not silently rewrite yesterday's leaderboard.
CREATE TABLE daily_round (
  id           BIGSERIAL   PRIMARY KEY,
  tenant_id    BIGINT      NOT NULL REFERENCES tenant(id),
  player_id    BIGINT      NOT NULL REFERENCES player(id),
  campaign_id  BIGINT      NOT NULL REFERENCES campaign(id),
  -- UTC date computed by the API, not date_trunc(now()) in SQL: "which day is it" decides whether
  -- a streak survives, and that must not depend on the database server's timezone setting.
  play_date    DATE        NOT NULL,
  scenario_id  BIGINT      NOT NULL REFERENCES daily_scenario(id),
  choice_key   TEXT        NOT NULL,
  -- Kept apart so the result screen can show them apart ("决策 +12 / 连续打卡 +5"):
  -- credit_after = previous credit + score_delta + streak_bonus, clamped.
  score_delta  INT         NOT NULL,   -- the chosen option's delta
  streak_bonus INT         NOT NULL DEFAULT 0,
  credit_after INT         NOT NULL,
  streak_after INT         NOT NULL,
  created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
  -- One decision per player per campaign per day, enforced by the database.
  -- The daily challenge *is* the scarcity: without this, a player re-answers until the score is
  -- maximal and both the leaderboard and the streak become meaningless. The API turns a conflict
  -- into an idempotent replay of the stored outcome, never an error.
  UNIQUE (tenant_id, player_id, campaign_id, play_date)
);
-- Leaderboard reads the latest row per player (DISTINCT ON player_id ORDER BY play_date DESC).
CREATE INDEX ON daily_round (tenant_id, campaign_id, player_id, play_date DESC);

-- ---------------------------------------------------------------- Template row
INSERT INTO template (id, code, name, config_schema) VALUES
  (6, 'daily_budget', '每日理财决策',
   '{"type":"object","properties":{"promo":{"type":"object","properties":{"text":{"type":"string"},"url":{"type":"string"},"min_credit":{"type":"integer"}}}}}')
ON CONFLICT (id) DO NOTHING;
SELECT setval('template_id_seq', (SELECT max(id) FROM template));

-- ---------------------------------------------------------------- Scenario content
-- Rotation is by index into `ORDER BY id`, so ids must stay stable — append new scenarios, never
-- renumber. Content is deliberately neutral: it explains what a choice costs (interest, credit
-- impact, opportunity cost) and never recommends a product. The soft promo is campaign config
-- (`campaign.config.promo`), not scenario copy, so a customer's marketing claim can never be
-- mistaken for the game's own advice.
INSERT INTO daily_scenario (id, code, title, prompt, options) VALUES
  (1, 'payday', '发工资了', '本月工资 8,000 元刚刚到账。房租水电 3,000 元已经扣掉，手里还有 5,000 元。你第一件事做什么？',
   '[
     {"key":"save_first","label":"先转 1,500 元进货币基金","delta":12,"verdict":"稳。先支付给自己，剩下的才是可花的钱。","tip":"“先存后花”把储蓄变成一笔固定支出。每月存 1,500 元，一年就是 1.8 万元的安全垫。"},
     {"key":"budget","label":"列一份本月开支计划","delta":8,"verdict":"不错。计划本身不省钱，但它让你知道钱去哪了。","tip":"记账的价值在于发现“漏水点”——多数人低估自己的餐饮和订阅支出约 20%。"},
     {"key":"spend_free","label":"先放着，想花就花","delta":-6,"verdict":"月底大概率会紧张。没有分配的钱最容易消失。","tip":"没有预算时，支出会自然膨胀到收入的上限，这被称为“生活方式膨胀”。"},
     {"key":"prepay_card","label":"提前把信用卡欠款全部还清","delta":10,"verdict":"很好。已有欠款的利息通常远高于理财收益。","tip":"信用卡循环利息按日计息（常见日利率 0.05%，年化约 18%），先还债几乎总是优于先理财。"}
   ]'),

  (2, 'new_phone', '想换新手机', '用了三年的手机开始卡顿。你看中一台 6,000 元的新机，目前存款 9,000 元，其中 6,000 元是应急储备。',
   '[
     {"key":"wait_save","label":"再存两个月，用额外攒的钱买","delta":12,"verdict":"最稳。应急储备不该为一次升级清零。","tip":"应急储备的作用是让你在失业或生病时不必借钱，动用它买消费品等于把风险转嫁给未来的自己。"},
     {"key":"installment_0","label":"用商家的 12 期免息分期","delta":4,"verdict":"可以，但要确认“免息”是不是真的没有手续费。","tip":"很多“免息”分期收每期手续费，例如每期 0.6%，12 期合计 7.2%，实际年化接近 13%。"},
     {"key":"use_savings","label":"直接花应急储备买","delta":-8,"verdict":"风险变高了。手机新了，抗风险能力没了。","tip":"通常建议应急储备覆盖 3–6 个月的必要开支；清空后重建平均需要半年以上。"},
     {"key":"cash_loan","label":"借一笔短期现金贷","delta":-14,"verdict":"最贵的选项。为一件非必需品支付高息。","tip":"短期现金贷年化常在 24%–36%，6,000 元借一年利息可达 1,400–2,100 元。"}
   ]'),

  (3, 'phone_broken', '突发支出', '通勤路上手机摔坏，维修报价 800 元。这个月的可支配预算只剩 500 元。',
   '[
     {"key":"emergency_fund","label":"动用应急储备支付","delta":12,"verdict":"正确。这正是应急储备存在的意义。","tip":"应急储备就是为了这种“必须花、又没预算”的支出，用完后下个月优先补回。"},
     {"key":"cut_budget","label":"砍掉本月娱乐和外食，凑齐 800","delta":10,"verdict":"很好。用现金流解决问题，不产生任何利息。","tip":"可变支出（外食、娱乐、打车）通常占月支出的 20%–30%，是压缩空间最大的部分。"},
     {"key":"credit_min","label":"刷信用卡，下月只还最低还款","delta":-10,"verdict":"最低还款会让 800 元滚很久。","tip":"最低还款只免除违约金，不免息：全部欠款从消费日起按日计息，且失去免息期。"},
     {"key":"delay","label":"先不修，凑合用","delta":2,"verdict":"省钱，但要留意拖延的隐性成本。","tip":"延后维修有时更贵（屏碎导致主板进灰）。判断标准是“拖延成本是否高于利息成本”。"}
   ]'),

  (4, 'card_bill', '信用卡账单来了', '本期账单 5,200 元，最低还款额 520 元。你账户里有 5,600 元，其中 3,000 元是下周的房租。',
   '[
     {"key":"pay_full","label":"全额还清 5,200 元，房租另想办法","delta":6,"verdict":"避开了利息，但房租缺口成了新问题。","tip":"全额还款保留免息期；但用一个刚性支出去补另一个，只是把问题往后挪一周。"},
     {"key":"pay_partial","label":"还 2,600 元，剩余下月还","delta":-4,"verdict":"部分还款不能停息。","tip":"未还清时，利息通常按**全额账单**从消费日算起，而不是按剩余部分计算。"},
     {"key":"pay_min","label":"只还最低还款额 520 元","delta":-12,"verdict":"最贵的拖延方式。","tip":"以年化约 18% 计，5,200 元只还最低，一年利息接近 900 元，本金几乎没减少。"},
     {"key":"cut_and_full","label":"全额还清，同时砍掉本月所有非必要开支补房租","delta":12,"verdict":"最优。先止息，再用预算把缺口补上。","tip":"处理顺序：先停掉利率最高的负债，再压缩可变支出，最后才考虑借款。"}
   ]'),

  (5, 'year_bonus', '年终奖到账', '年终奖 20,000 元到账。你有 8,000 元信用卡分期未还（年化约 15%），也想投资。',
   '[
     {"key":"repay_debt","label":"先还清 8,000 元分期，剩下的存起来","delta":14,"verdict":"最优解。还债是一笔确定的“收益”。","tip":"还清年化 15% 的债 = 获得 15% 的无风险回报。市场上没有同等确定性的投资。"},
     {"key":"invest_all","label":"全部买入股票基金","delta":-6,"verdict":"一边付 15% 的利息，一边赌一个不确定的收益。","tip":"投资收益不确定，利息支出确定。负债利率高于预期收益率时，先还债。"},
     {"key":"half","label":"一半还债，一半理财","delta":6,"verdict":"折中，但仍在为剩下的欠款付高息。","tip":"分散在这里没有意义：债务利率是已知的，先消灭确定的成本。"},
     {"key":"spend","label":"先犒劳自己一次长途旅行","delta":-10,"verdict":"欠款还在滚利息。","tip":"奖金是一次性收入，用来消除负债对全年现金流的改善远大于一次消费。"}
   ]'),

  (6, 'rent_hike', '房租上涨', '续租时房东把月租从 3,000 元涨到 3,450 元（涨幅 15%），你的月收入没有变化。',
   '[
     {"key":"negotiate","label":"先和房东谈，或比较周边房源再决定","delta":12,"verdict":"对。搬家有成本，但信息不对称的成本更高。","tip":"住房支出通常建议控制在税后收入的 30% 以内，超过后其他所有目标都会被挤压。"},
     {"key":"cut_other","label":"接受涨价，从其他预算里砍出 450 元","delta":8,"verdict":"可行，但要确认砍的是可变支出而不是储蓄。","tip":"当刚性支出上升时，最先被牺牲的往往是储蓄——那正是最不该动的一项。"},
     {"key":"reduce_saving","label":"接受涨价，把每月储蓄减少 450 元","delta":-6,"verdict":"短期无感，长期代价大。","tip":"每月少存 450 元，十年下来（按年化 3% 计）约少 63,000 元。"},
     {"key":"borrow","label":"用消费贷补差额","delta":-14,"verdict":"用负债覆盖长期固定支出，会持续放大。","tip":"借款只能解决一次性缺口；用它填补每月都会发生的支出，债务会按月累积。"}
   ]'),

  (7, 'flash_sale', '直播间秒杀', '大促直播间里，一台原价 2,400 元的扫地机器人现在 1,299 元，倒计时 3 分钟。你没有计划买它。',
   '[
     {"key":"skip","label":"不买，本来就没这个需求","delta":10,"verdict":"对。折扣不会让不需要的东西变成需要。","tip":"“省下 1,101 元”是错觉，实际发生的是支出 1,299 元。"},
     {"key":"wishlist","label":"加入清单，24 小时后再决定","delta":12,"verdict":"最好。冷静期是最便宜的省钱工具。","tip":"限时倒计时的作用是制造稀缺感、抑制比较；延迟 24 小时能过滤掉大部分冲动购买。"},
     {"key":"buy_cash","label":"用本月预算里的余钱买","delta":0,"verdict":"至少没有借钱，但预算被挤占了。","tip":"计划外支出会挤压当月储蓄，可以问自己：这笔钱原本要去哪？"},
     {"key":"buy_credit","label":"信用卡分 12 期买下","delta":-10,"verdict":"为一个临时决定背上一年的还款。","tip":"分期把一次冲动摊成 12 个月的现金流占用，还会占用你的信用卡额度和征信中的负债率。"}
   ]'),

  (8, 'side_income', '副业收入', '这个月副业多赚了 2,000 元。这是计划外的收入。',
   '[
     {"key":"save_all","label":"全部存入应急储备","delta":12,"verdict":"很好。计划外收入是最容易存下来的钱。","tip":"意外之财在心理上属于“额外”，此时储蓄阻力最小——这是提高储蓄率最有效的时机。"},
     {"key":"split","label":"70% 存起来，30% 用来奖励自己","delta":10,"verdict":"可持续。留一点正反馈，计划才不会崩。","tip":"完全禁止奖励的预算通常撑不过三个月，留出小额“可自由支配”额度反而更容易坚持。"},
     {"key":"upgrade","label":"顺势升级一下日常消费","delta":-8,"verdict":"当心生活方式膨胀。","tip":"收入上升时同步提高消费，储蓄率不变甚至下降，收入增长的好处会被完全吃掉。"},
     {"key":"invest_risky","label":"全部投入朋友推荐的高收益产品","delta":-6,"verdict":"先问清楚风险和资金去向。","tip":"收益与风险成正比。承诺高收益又强调“稳赚”的产品，风险通常在你看不到的地方。"}
   ]'),

  (9, 'loan_sms', '一条借款短信', '你收到短信：“您已获得 50,000 元低息额度，日息万三，随借随还。”你目前没有资金缺口。',
   '[
     {"key":"ignore","label":"忽略，没有缺口就不借","delta":12,"verdict":"对。额度不是收入。","tip":"日息万三 = 年化约 10.95%。“低息”是相对说法，关键永远看年化利率（APR）。"},
     {"key":"check_only","label":"点进去看看能批多少额度","delta":-4,"verdict":"看一眼也可能留下记录。","tip":"多数借贷产品的额度测算会发起一次征信查询；短期内频繁的贷款审批查询会影响信用评估。"},
     {"key":"borrow_invest","label":"借出来买理财赚利差","delta":-14,"verdict":"用确定的利息成本去赌不确定的收益。","tip":"借贷投资会同时放大收益和亏损；利率 10.95% 意味着你必须先跑赢 10.95% 才开始赚钱。"},
     {"key":"borrow_backup","label":"先借 10,000 元放着备用","delta":-10,"verdict":"从取现那一刻起就开始计息。","tip":"“随借随还”通常按日计息，闲置资金放在账户里不会产生足以抵消利息的收益。"}
   ]'),

  (10, 'insurance', '要不要买保险', '同事住院自费花了 3 万元。你今年 28 岁，没有任何商业保险，公司只交了社保。',
   '[
     {"key":"medical","label":"先配一份百万医疗险","delta":12,"verdict":"顺序对。先覆盖会击穿储蓄的大额风险。","tip":"保险的作用是转移“低概率、高损失”的风险；医疗险保费低、杠杆高，通常是第一顺位。"},
     {"key":"nothing","label":"暂时不买，年轻风险低","delta":-6,"verdict":"概率低不等于损失小。","tip":"社保对自费药、进口器材和特需病房的覆盖有限，这部分缺口正是商业医疗险的作用范围。"},
     {"key":"savings_policy","label":"买一份带返还的储蓄型保险","delta":0,"verdict":"保障和理财混在一起，两边都不突出。","tip":"返还型产品的保障成本被摊在保费里，同等保额的保费通常是纯保障型的数倍。"},
     {"key":"all_in","label":"一次性把重疾、寿险、年金都买齐","delta":-4,"verdict":"保额够了，但保费可能挤爆现金流。","tip":"常见经验是年保费控制在年收入的 5%–10%；保费过高导致中途退保，损失往往更大。"}
   ]'),

  (11, 'guaranteed_return', '“稳赚不赔”的机会', '熟人推荐一个项目：月息 3%，承诺保本，只要拉人进来还能拿返佣。',
   '[
     {"key":"refuse","label":"拒绝，并提醒身边的人","delta":14,"verdict":"正确。保本高息 + 拉人返佣是典型的危险组合。","tip":"月息 3% 即年化约 42.6%；持牌机构不允许承诺保本收益，靠拉新支付老客收益的结构无法持续。"},
     {"key":"small_test","label":"先投 5,000 元试试水","delta":-10,"verdict":"“小额试水”通常是入口，不是风控。","tip":"这类结构早期确实会按时兑付——本金来自后加入的人，兑付本身不构成安全证明。"},
     {"key":"ask_license","label":"要求出示牌照和合同再说","delta":8,"verdict":"该问的都问了，但对方大概率给不出。","tip":"可以核验机构是否持牌、产品是否登记备案；无法核验的“合同”不提供任何保护。"},
     {"key":"all_in_loan","label":"借钱加大投入，赚快钱","delta":-15,"verdict":"最坏的组合：高杠杆押注不可核验的承诺。","tip":"借贷参与此类项目时，本金损失和债务同时存在，损失不会因为项目失败而免除。"}
   ]'),

  (12, 'job_gap', '收入中断', '公司调整，你被通知一个月后离职。目前存款 12,000 元，月必要开支 4,500 元。',
   '[
     {"key":"cut_now","label":"立刻把开支压到最低，延长储备时间","delta":12,"verdict":"对。收入不确定时，先延长生存时间。","tip":"12,000 元 ÷ 4,500 元 ≈ 2.7 个月；把开支压到 3,500 元，就变成 3.4 个月的缓冲。"},
     {"key":"keep","label":"保持现有生活水平，边找工作边说","delta":-6,"verdict":"缓冲期会明显变短。","tip":"求职周期常见为 1–3 个月；储备月数低于求职周期时，往往被迫接受更差的条件。"},
     {"key":"borrow_now","label":"趁还在职先申请一笔信用贷","delta":0,"verdict":"在职时更容易获批，但要算清成本。","tip":"贷款审批看收入证明，离职后额度会收紧；不过借来的钱是负债，只在确有缺口时才动用。"},
     {"key":"invest_gap","label":"把存款投入股市，争取快速回本","delta":-14,"verdict":"应急资金不能承担波动。","tip":"应急储备的第一属性是流动性和确定性，通常放在活期或货币基金，而不是波动性资产。"}
   ]')
ON CONFLICT (id) DO NOTHING;

SELECT setval('daily_scenario_id_seq', (SELECT max(id) FROM daily_scenario));

-- ---------------------------------------------------------------- Permissions
ALTER TABLE daily_round ENABLE ROW LEVEL SECURITY;
ALTER TABLE daily_round FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON daily_round
  USING (tenant_id = current_setting('app.tenant_id', true)::bigint);

DO $$
DECLARE s TEXT := current_schema();
BEGIN
  EXECUTE 'GRANT SELECT, INSERT, UPDATE, DELETE ON daily_round TO ignition_app';
  -- Catalog is reference data: read-only for the app, same as `template`.
  EXECUTE 'GRANT SELECT ON daily_scenario TO ignition_app';
  EXECUTE format('GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA %I TO ignition_app', s);
EXCEPTION WHEN undefined_object THEN
  RAISE WARNING 'ignition_app role does not exist, skipping grants';
END
$$;

COMMIT;
