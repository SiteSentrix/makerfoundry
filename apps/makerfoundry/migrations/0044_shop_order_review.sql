-- Fixes from the code review of 2026-11-01 (shop orders, still switched off on production).

-- A shop's Foundry Account, as its catalogue names it, is only paid once that account's holder
-- has confirmed the shop on MakerFoundry; a catalogue naming another account resets it.
ALTER TABLE mf_shops ADD COLUMN account_confirmed BOOLEAN NOT NULL DEFAULT false;

-- When a due refund was last tried, so refunds that keep failing don't hold up the rest.
ALTER TABLE mf_shop_orders ADD COLUMN refund_tried_at TIMESTAMPTZ;

-- A payment Stripe took for an order that couldn't take it (deleted, already paid through another
-- checkout, a different amount): recorded here and refunded in full by the sweep.
CREATE TABLE mf_shop_stray_payments (
    payment_intent TEXT PRIMARY KEY,
    session TEXT NOT NULL,
    ref TEXT NOT NULL,
    cents BIGINT NOT NULL,
    currency TEXT NOT NULL,
    at TIMESTAMPTZ NOT NULL DEFAULT now(),
    tried_at TIMESTAMPTZ,
    refunded_at TIMESTAMPTZ
);
