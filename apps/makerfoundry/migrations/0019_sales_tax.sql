-- MF-24: sales tax with Stripe Tax (on with MF_STRIPE_TAX). MakerFoundry, as the marketplace, is
-- liable for the tax (https://docs.stripe.com/tax/tax-for-marketplaces): the buyer pays the price
-- plus the tax, the tax is MakerFoundry's ('tax' rows of account 0), never the seller's.
-- A destination charge moves the whole payment to the seller's Stripe account, so the tax is
-- taken back from it with a transfer reversal once the sale is paid (`tax_withheld_at`). Refunding
-- a taxed sale takes the price (and the tax, if it's still there) back from the seller first
-- (`refund_reversed_at`), then refunds the buyer everything they paid.
ALTER TABLE mf_orders ADD COLUMN tax_cents BIGINT NOT NULL DEFAULT 0 CHECK (tax_cents >= 0);
ALTER TABLE mf_orders ADD COLUMN tax_withheld_at TIMESTAMPTZ;
ALTER TABLE mf_orders ADD COLUMN refund_reversed_at TIMESTAMPTZ;

ALTER TABLE mf_ledger DROP CONSTRAINT mf_ledger_kind_check;
ALTER TABLE mf_ledger ADD CONSTRAINT mf_ledger_kind_check CHECK (kind IN ('seller', 'fee', 'processing', 'tax'));
