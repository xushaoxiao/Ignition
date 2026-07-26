-- 0004 — Game templates catalog
--
-- A "game" is a client animation skin over the same server-authoritative draw. play() returns a
-- won prize (game::draw + atomic stock); the template only decides how the TMA animates the reveal
-- — the outcome is game-agnostic, so no billing/attribution code changes with a new game. The
-- catalog is platform reference data (not tenant-scoped), so it lives with the schema, not in seed.
--
-- lucky_wheel is (re)asserted here with its seed id so the migration is the single source of truth
-- and is safe whether it runs before or after db/seed.sql on a fresh database.

-- Target schema, same as 0001/0002/0003. Override: psql -v schema=xxx
\if :{?schema}
\else
  \set schema ignition
\endif
SET search_path TO :"schema", public;

BEGIN;

INSERT INTO template (id, code, name, config_schema) VALUES
  (1, 'lucky_wheel',  '幸运大转盘', '{"type":"object","properties":{"segments":{"type":"integer","minimum":4}}}'),
  (2, 'scratch_card', '刮刮卡',     '{"type":"object","properties":{}}'),
  (3, 'slot_machine', '老虎机',     '{"type":"object","properties":{"reels":{"type":"integer","minimum":3}}}'),
  (4, 'blind_box',    '幸运盲盒',   '{"type":"object","properties":{}}'),
  (5, 'flip_card',    '翻牌有礼',   '{"type":"object","properties":{"cards":{"type":"integer","minimum":3}}}')
ON CONFLICT (id) DO NOTHING;

-- Keep the sequence ahead of the explicit ids above.
SELECT setval('template_id_seq', (SELECT max(id) FROM template));

COMMIT;
