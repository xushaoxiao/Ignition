-- 0003 — Invoice payment push (Stripe)
--
-- Month-end settlement (jobs/settle.rs) creates invoices in status 'draft'. The push job
-- (jobs/push.rs) hands each finalized invoice to the payment gateway and records the external
-- reference here, moving the invoice to 'open'. Kept separate from settlement so the push is
-- independently re-runnable and a gateway outage never blocks invoice generation.

-- Target schema, same as 0001/0002. Override: psql -v schema=xxx
\if :{?schema}
\else
  \set schema ignition
\endif
SET search_path TO :"schema", public;

BEGIN;

-- External payment reference (e.g. a Stripe invoice id). NULL until the push succeeds — the push
-- job treats NULL as "not yet sent", so setting it is what makes the send idempotent across reruns.
ALTER TABLE invoice ADD COLUMN IF NOT EXISTS stripe_invoice_id TEXT;
ALTER TABLE invoice ADD COLUMN IF NOT EXISTS pushed_at TIMESTAMPTZ;

-- The push job scans only unpushed invoices; a partial index keeps that scan cheap as invoices
-- accumulate month over month.
CREATE INDEX IF NOT EXISTS invoice_unpushed
  ON invoice (tenant_id)
  WHERE stripe_invoice_id IS NULL;

COMMIT;
