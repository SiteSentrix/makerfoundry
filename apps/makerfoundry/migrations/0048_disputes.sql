-- Issue #90: chargebacks. A sale is a destination charge with no `on_behalf_of`, so MakerFoundry is
-- the merchant of record and Stripe takes a disputed payment from MakerFoundry's balance. When a
-- buyer disputes a sale (`charge.dispute.created`) the order is `disputed` (no more downloads) and
-- the disputed price is taken back from the seller's transfer, then MakerFoundry's 10% of it goes
-- back to them (as on a refund, Seller Terms § 7). When the dispute closes
-- (`charge.dispute.closed`): won (or an inquiry closed), the order is paid again and the seller
-- gets back what was taken; lost, the order is a `chargeback` and stays blocked.
-- The Stripe calls are made after the event is recorded and retried by the sweep until done
-- (`*_at` below), each checked with Stripe first so none is made twice.
ALTER TABLE mf_orders DROP CONSTRAINT mf_orders_status_check;
ALTER TABLE mf_orders ADD CONSTRAINT mf_orders_status_check
    CHECK (status IN ('pending', 'paid', 'refunded', 'expired', 'disputed', 'chargeback'));

-- Stripe's dispute (`dp_…`), what it disputes (cents, of the price and tax the buyer paid), and how
-- it closed (`won`, `lost`, `warning_closed`; empty while open)
ALTER TABLE mf_orders ADD COLUMN dispute_id TEXT;
ALTER TABLE mf_orders ADD COLUMN dispute_cents BIGINT NOT NULL DEFAULT 0 CHECK (dispute_cents >= 0);
ALTER TABLE mf_orders ADD COLUMN dispute_outcome TEXT NOT NULL DEFAULT '';
ALTER TABLE mf_orders ADD COLUMN disputed_at TIMESTAMPTZ;
ALTER TABLE mf_orders ADD COLUMN dispute_closed_at TIMESTAMPTZ;
-- the price taken back from the seller's transfer; MakerFoundry's 10% of it given back to them;
-- (won) what was taken given back again, or nothing to give back
ALTER TABLE mf_orders ADD COLUMN dispute_reversed_at TIMESTAMPTZ;
ALTER TABLE mf_orders ADD COLUMN dispute_fee_returned_at TIMESTAMPTZ;
ALTER TABLE mf_orders ADD COLUMN dispute_restored_at TIMESTAMPTZ;
-- the last state the seller was emailed about (`disputed`, `won`, `lost`), and when the admins
-- were alerted that a Stripe step couldn't be done
ALTER TABLE mf_orders ADD COLUMN dispute_mailed TEXT NOT NULL DEFAULT '';
ALTER TABLE mf_orders ADD COLUMN dispute_alerted_at TIMESTAMPTZ;
CREATE INDEX mf_orders_disputes ON mf_orders (id) WHERE dispute_id IS NOT NULL;

ALTER TABLE mf_ledger DROP CONSTRAINT mf_ledger_reason_check;
ALTER TABLE mf_ledger ADD CONSTRAINT mf_ledger_reason_check
    CHECK (reason IN ('sale', 'refund', 'fee_return', 'dispute', 'dispute_fee', 'dispute_won'));
