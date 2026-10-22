-- MF-23: on a refund the seller bears the card fee Stripe keeps (the owner's decision, 2026-10-22;
-- Seller Terms § 7). A refund takes the whole price back from the seller ('refund': the seller
-- -price), then MakerFoundry's 10% goes back to them ('fee_return': the seller +fee, MakerFoundry's
-- fee -fee), so the seller is left out only the card fee, and the card fee row stays as Stripe
-- keeps it. The second step is its own Stripe call: until it's done, `fee_returned_at` is empty and
-- the admins' orders page offers it again.
ALTER TABLE mf_orders ADD COLUMN fee_returned_at TIMESTAMPTZ;
-- refunds made before this (the whole application fee given back with the refund) are complete
UPDATE mf_orders SET fee_returned_at = refunded_at WHERE status = 'refunded';

ALTER TABLE mf_ledger DROP CONSTRAINT mf_ledger_reason_check;
ALTER TABLE mf_ledger ADD CONSTRAINT mf_ledger_reason_check CHECK (reason IN ('sale', 'refund', 'fee_return'));
