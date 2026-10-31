-- MF-63: orders from shops (issues #1 and #2). The customer pays MakerFoundry; MakerFoundry keeps
-- 10% and the card fee comes out of the shop's share (the owner's decisions of 2026-10-31). The
-- amounts are worked out from the shop's catalogue when the order is made and never change; each
-- share of a paid order is a row in mf_shop_ledger. `token` is the order's secret, which the shop
-- uses to fetch the art. Sales tax (MF-66) is MakerFoundry's to collect as the marketplace: it's
-- outside the shop's share.
CREATE TABLE mf_shop_orders (
    id BIGSERIAL PRIMARY KEY,
    ref TEXT NOT NULL UNIQUE,
    token TEXT NOT NULL,
    buyer_account BIGINT NOT NULL,
    shop_party TEXT NOT NULL,
    shop_account BIGINT NOT NULL,
    lines JSONB NOT NULL,
    art JSONB NOT NULL,
    customer_name TEXT NOT NULL,
    ship_to JSONB,
    note TEXT NOT NULL DEFAULT '',
    goods_cents BIGINT NOT NULL CHECK (goods_cents >= 0),
    setup_cents BIGINT NOT NULL CHECK (setup_cents >= 0),
    shipping_cents BIGINT NOT NULL CHECK (shipping_cents >= 0),
    tax_cents BIGINT NOT NULL CHECK (tax_cents >= 0),
    paid_cents BIGINT NOT NULL CHECK (paid_cents > 0),
    fee_cents BIGINT NOT NULL CHECK (fee_cents >= 0),
    card_fee_cents BIGINT NOT NULL CHECK (card_fee_cents >= 0),
    shop_cents BIGINT NOT NULL CHECK (shop_cents > 0),
    CHECK (paid_cents = goods_cents + setup_cents + shipping_cents + tax_cents),
    CHECK (paid_cents = tax_cents + fee_cents + card_fee_cents + shop_cents),
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'paid', 'sent', 'received',
        'proof_sent', 'approved', 'in_production', 'ready', 'shipped', 'collected', 'cancelled',
        'refunded', 'expired', 'abandoned')),
    stripe_session TEXT,
    stripe_payment_intent TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    paid_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX mf_shop_orders_buyer ON mf_shop_orders (buyer_account, created_at DESC);
CREATE INDEX mf_shop_orders_intent ON mf_shop_orders (stripe_payment_intent) WHERE stripe_payment_intent IS NOT NULL;

CREATE TABLE mf_shop_ledger (
    id BIGSERIAL PRIMARY KEY,
    order_id BIGINT NOT NULL REFERENCES mf_shop_orders (id),
    account_id BIGINT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('shop', 'fee', 'processing', 'tax')),
    cents BIGINT NOT NULL,
    reason TEXT NOT NULL CHECK (reason IN ('sale', 'refund')),
    at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (order_id, kind, reason)
);
