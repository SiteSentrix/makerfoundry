-- MF-84: what a shop reports on an order, its proofs, cancelling and refunds. A refund that's due
-- (`refund_due`: the order was cancelled before production, or the shop never took it) is made
-- by Stripe, recorded once (`refunded_at`), and then MakerFoundry's 10% goes back to the shop
-- (`fee_returned_at`), as for designs: the shop is out only the card fee.
ALTER TABLE mf_shop_orders
    ADD COLUMN tracking TEXT NOT NULL DEFAULT '',
    ADD COLUMN shop_note TEXT NOT NULL DEFAULT '',
    ADD COLUMN cancelled_by TEXT CHECK (cancelled_by IN ('shop', 'customer')),
    ADD COLUMN cancel_why TEXT NOT NULL DEFAULT '',
    ADD COLUMN refund_due BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN refunded_at TIMESTAMPTZ,
    ADD COLUMN fee_returned_at TIMESTAMPTZ;
CREATE INDEX mf_shop_orders_refunds ON mf_shop_orders (id) WHERE refund_due AND fee_returned_at IS NULL;

ALTER TABLE mf_shop_ledger DROP CONSTRAINT mf_shop_ledger_reason_check;
ALTER TABLE mf_shop_ledger ADD CONSTRAINT mf_shop_ledger_reason_check CHECK (reason IN ('sale', 'refund', 'fee_return'));

-- A shop's proofs (mock-ups), each re-drawn from the checked image before it's kept.
CREATE TABLE mf_shop_proofs (
    order_id BIGINT NOT NULL REFERENCES mf_shop_orders (id) ON DELETE CASCADE,
    proof INT NOT NULL,
    image_key TEXT NOT NULL,
    image_type TEXT NOT NULL,
    note TEXT NOT NULL DEFAULT '',
    answer TEXT CHECK (answer IN ('approved', 'changes')),
    answer_note TEXT NOT NULL DEFAULT '',
    sent_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    answered_at TIMESTAMPTZ,
    PRIMARY KEY (order_id, proof)
);
