-- MF-4: selling. Sellers' Stripe Express accounts, prices, orders, the money ledger (every
-- charge, fee and refund a row, in whole cents), and the Stripe events already handled.

CREATE TABLE mf_sellers (
    account_id BIGINT PRIMARY KEY REFERENCES mf_accounts (account_id),
    stripe_account TEXT NOT NULL UNIQUE,
    -- as Stripe last told us (account.updated)
    details_submitted BOOLEAN NOT NULL DEFAULT false,
    charges_enabled BOOLEAN NOT NULL DEFAULT false,
    payouts_enabled BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- 0 is free; a paid design costs whole cents
ALTER TABLE mf_designs ADD COLUMN price_cents BIGINT NOT NULL DEFAULT 0 CHECK (price_cents >= 0);

CREATE TABLE mf_orders (
    id BIGSERIAL PRIMARY KEY,
    buyer_account BIGINT NOT NULL REFERENCES mf_accounts (account_id),
    design_id BIGINT REFERENCES mf_designs (id) ON DELETE SET NULL,
    seller_account BIGINT NOT NULL REFERENCES mf_accounts (account_id),
    -- the amounts, fixed when the order is made
    price_cents BIGINT NOT NULL CHECK (price_cents > 0),
    fee_cents BIGINT NOT NULL CHECK (fee_cents >= 0),
    processing_cents BIGINT NOT NULL CHECK (processing_cents >= 0),
    seller_cents BIGINT NOT NULL CHECK (seller_cents > 0),
    CHECK (fee_cents + processing_cents + seller_cents = price_cents),
    currency TEXT NOT NULL DEFAULT 'usd',
    title TEXT NOT NULL,
    stripe_session TEXT UNIQUE,
    stripe_payment_intent TEXT,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'paid', 'refunded')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    paid_at TIMESTAMPTZ,
    refunded_at TIMESTAMPTZ
);
CREATE INDEX mf_orders_buyer ON mf_orders (buyer_account, created_at DESC);
CREATE INDEX mf_orders_seller ON mf_orders (seller_account, created_at DESC);

-- Every movement of money, never changed or deleted: a sale's three parts when it's paid, and the
-- same with the opposite sign when it's refunded. account 0 is MakerFoundry.
CREATE TABLE mf_ledger (
    id BIGSERIAL PRIMARY KEY,
    order_id BIGINT NOT NULL REFERENCES mf_orders (id),
    account_id BIGINT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('seller', 'fee', 'processing')),
    cents BIGINT NOT NULL,
    reason TEXT NOT NULL CHECK (reason IN ('sale', 'refund')),
    at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (order_id, kind, reason)
);

-- Stripe sends an event again until it's acknowledged: each is handled once
CREATE TABLE mf_stripe_events (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    at TIMESTAMPTZ NOT NULL DEFAULT now()
);
