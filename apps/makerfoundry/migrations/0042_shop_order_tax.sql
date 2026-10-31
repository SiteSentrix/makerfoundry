-- MF-66: sales tax on orders from shops, worked out by Stripe Tax before checkout (the
-- calculation), recorded with Stripe once paid (the transaction) and reversed on a refund.
ALTER TABLE mf_shop_orders
    ADD COLUMN tax_calculation TEXT,
    ADD COLUMN tax_transaction TEXT,
    ADD COLUMN tax_reversed_at TIMESTAMPTZ;
CREATE INDEX mf_shop_orders_tax_due ON mf_shop_orders (id)
    WHERE tax_cents > 0 AND (tax_transaction IS NULL OR tax_reversed_at IS NULL);
