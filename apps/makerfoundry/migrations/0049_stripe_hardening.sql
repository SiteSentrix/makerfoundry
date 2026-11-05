-- Issue #91: Stripe hardening (the Stripe Connect audit's fixes 3 to 6).

-- Refunds made outside MakerFoundry's admin (the Stripe Dashboard): a partial refund is recorded
-- (`partial_refund` ledger rows, one per total refunded so far, by `stripe_ref`), and the
-- seller's transfer is reversed for any refund Stripe didn't reverse (`refund_check_due`, until
-- that's been checked with Stripe).
ALTER TABLE mf_orders ADD COLUMN refunded_cents BIGINT NOT NULL DEFAULT 0 CHECK (refunded_cents >= 0);
ALTER TABLE mf_orders ADD COLUMN refund_check_due BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE mf_orders ADD COLUMN refund_alerted_at TIMESTAMPTZ;
CREATE INDEX mf_orders_refund_check ON mf_orders (id) WHERE refund_check_due;

-- a ledger row is once per order, kind, reason and Stripe reference (empty for the one-off rows)
ALTER TABLE mf_ledger ADD COLUMN stripe_ref TEXT NOT NULL DEFAULT '';
ALTER TABLE mf_ledger DROP CONSTRAINT mf_ledger_order_id_kind_reason_key;
ALTER TABLE mf_ledger ADD CONSTRAINT mf_ledger_entry_key UNIQUE (order_id, kind, reason, stripe_ref);
ALTER TABLE mf_ledger DROP CONSTRAINT mf_ledger_reason_check;
ALTER TABLE mf_ledger ADD CONSTRAINT mf_ledger_reason_check
    CHECK (reason IN ('sale', 'refund', 'fee_return', 'dispute', 'dispute_fee', 'dispute_won', 'partial_refund'));

-- account.updated: applied only if newer than the last one applied (`event.created`, Unix
-- seconds), and why Stripe has paused the account (`requirements.disabled_reason`)
ALTER TABLE mf_sellers ADD COLUMN stripe_event_at BIGINT;
ALTER TABLE mf_sellers ADD COLUMN disabled_reason TEXT NOT NULL DEFAULT '';
-- the payout delay (days) last set on the seller's Stripe account; NULL: Stripe's default
ALTER TABLE mf_sellers ADD COLUMN payout_delay_days INT;
