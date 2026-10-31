//! Sales tax on orders from shops (MF-86; the owner's decision of 2026-10-31): on with Stripe Tax
//! (`MF_STRIPE_TAX`), as for designs. MakerFoundry takes the payment, so it's the marketplace
//! that collects the tax, never the shop: the tax goes to MakerFoundry inside the application fee
//! and is outside the shop's share.
//!
//! Unlike designs (taxed by Stripe Checkout and taken back from the seller afterwards), an
//! order's tax is worked out before checkout, from where it ships to (or the shop's address, for
//! pick-up), so the customer sees it on the order before paying
//! (https://docs.stripe.com/api/tax/calculations/create). Once paid, the calculation is recorded
//! as a tax transaction for Stripe's tax reports
//! (https://docs.stripe.com/api/tax/transactions/create_from_calculation), and a refund reverses
//! it in full (https://docs.stripe.com/api/tax/transactions/create_reversal). Stripe charges tax
//! only where MakerFoundry has registered to collect it: elsewhere it's 0.

use serde_json::Value;
use sqlx::PgPool;

use super::contract::Address;
use crate::stripe::Stripe;

fn stripe_id(id: &str, prefix: &str) -> bool {
    id.starts_with(prefix) && id.len() <= 255 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// The tax on goods and set-up of `items` cents, and shipping of `shipping`, sent to (or picked
/// up at) `to` → the tax in cents and Stripe's calculation. `Err` is a sentence for the customer.
pub async fn calculate(
    stripe: &Stripe,
    sku: &str,
    items: i64,
    shipping: i64,
    to: &Address,
) -> Result<(i64, String), String> {
    let mut form: Vec<(&str, String)> = vec![
        ("currency", "usd".into()),
        ("line_items[0][amount]", items.to_string()),
        ("line_items[0][reference]", sku.chars().take(40).collect()),
        ("line_items[0][tax_code]", stripe.goods_tax_code.clone()),
        ("line_items[0][tax_behavior]", "exclusive".into()),
        ("customer_details[address][line1]", to.line1.clone()),
        ("customer_details[address][city]", to.city.clone()),
        ("customer_details[address][state]", to.region.clone()),
        ("customer_details[address][postal_code]", to.postcode.clone()),
        ("customer_details[address][country]", to.country.clone()),
        ("customer_details[address_source]", "shipping".into()),
    ];
    if !to.line2.is_empty() {
        form.push(("customer_details[address][line2]", to.line2.clone()));
    }
    if shipping > 0 {
        form.push(("shipping_cost[amount]", shipping.to_string()));
    }
    let calc = stripe.post("/v1/tax/calculations", &form, None).await.map_err(|e| {
        eprintln!("shop orders: tax calculation: {e}");
        "The sales tax couldn't be worked out for that address. Please check it, or try again in a few minutes."
            .to_string()
    })?;
    let id = calc
        .get("id")
        .and_then(Value::as_str)
        .filter(|i| stripe_id(i, "taxcalc_"))
        .ok_or("The sales tax couldn't be worked out just now. Please try again.")?;
    let tax = calc
        .get("tax_amount_exclusive")
        .and_then(Value::as_i64)
        .filter(|t| (0..=items.saturating_add(shipping)).contains(t))
        .ok_or("The sales tax couldn't be worked out just now. Please try again.")?;
    Ok((tax, id.to_string()))
}

/// A paid order's tax recorded with Stripe, once (nothing for an untaxed or refunded order).
pub async fn record(stripe: &Stripe, pool: &PgPool, id: i64) -> Result<(), String> {
    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT ref, tax_calculation FROM mf_shop_orders
         WHERE id = $1 AND paid_at IS NOT NULL AND refunded_at IS NULL AND tax_cents > 0
           AND tax_calculation IS NOT NULL AND tax_transaction IS NULL",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    let Some((reference, calculation)) = row else {
        return Ok(());
    };
    let t = stripe
        .post(
            "/v1/tax/transactions/create_from_calculation",
            &[("calculation", calculation), ("reference", reference.clone())],
            Some(&format!("mf-shop-tax-{reference}")),
        )
        .await
        .map_err(|e| e.to_string())?;
    let transaction = t
        .get("id")
        .and_then(Value::as_str)
        .filter(|i| stripe_id(i, "tax_"))
        .ok_or("Stripe gave no tax transaction")?;
    sqlx::query("UPDATE mf_shop_orders SET tax_transaction = $2 WHERE id = $1 AND tax_transaction IS NULL")
        .bind(id)
        .bind(transaction)
        .execute(pool)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// A refunded order's tax transaction reversed in full, once.
pub async fn reverse(stripe: &Stripe, pool: &PgPool, id: i64) -> Result<(), String> {
    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT ref, tax_transaction FROM mf_shop_orders
         WHERE id = $1 AND refunded_at IS NOT NULL AND tax_transaction IS NOT NULL AND tax_reversed_at IS NULL",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    let Some((reference, transaction)) = row else {
        return Ok(());
    };
    stripe
        .post(
            "/v1/tax/transactions/create_reversal",
            &[
                ("mode", "full".into()),
                ("original_transaction", transaction),
                ("reference", format!("{reference}-refund")),
            ],
            Some(&format!("mf-shop-tax-back-{reference}")),
        )
        .await
        .map_err(|e| e.to_string())?;
    sqlx::query("UPDATE mf_shop_orders SET tax_reversed_at = now() WHERE id = $1 AND tax_reversed_at IS NULL")
        .bind(id)
        .execute(pool)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Both, for every order that needs one (the sweep, and after a payment or a refund).
pub async fn settle_all(stripe: &Stripe, pool: &PgPool) -> Result<(), String> {
    let due: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM mf_shop_orders
         WHERE tax_cents > 0 AND paid_at IS NOT NULL
           AND ((tax_transaction IS NULL AND refunded_at IS NULL)
             OR (tax_transaction IS NOT NULL AND refunded_at IS NOT NULL AND tax_reversed_at IS NULL))
         ORDER BY id LIMIT 50",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    for id in due {
        let done = async {
            record(stripe, pool, id).await?;
            reverse(stripe, pool, id).await
        };
        if let Err(e) = done.await {
            eprintln!("shop orders: tax of order {id}: {e}");
        }
    }
    Ok(())
}
