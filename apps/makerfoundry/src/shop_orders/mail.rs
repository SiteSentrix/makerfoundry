//! Emailing the customer when the shop reports a status on their order (issue #82).
//!
//! Applying an `order.status` notes one email by the hub message's id (`note`), so a message
//! applied again is mailed once; the sweep sends what's noted (`send_due`) through the same
//! sender as the email notices (Resend, or a staging deploy's test mailbox), with a link to the
//! order on this deploy's own address (`MF_PUBLIC_URL`). An email that can't be sent is tried at
//! the next few sweeps, then left; nothing about it holds up the order's messages.

use sqlx::PgPool;

use super::orders::status_name;
use crate::AppState;

/// How many times an email is tried.
const TRIES: i32 = 5;
/// The most emails sent in one sweep.
const BATCH: i64 = 100;

/// Notes the email for `order.status` message `message_id`, which moved order `order_id` to
/// `status` (once per message).
pub async fn note(pool: &PgPool, order_id: i64, message_id: &str, status: &str) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO mf_shop_order_mails (message_id, order_id, status) VALUES ($1, $2, $3)
         ON CONFLICT (message_id) DO NOTHING",
    )
    .bind(message_id)
    .bind(order_id)
    .bind(status)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// The subject and text of the email telling the customer that `shop` moved order `reference` to
/// `status`, with a link to the order at `public_url`.
pub fn status_mail(public_url: &str, shop: &str, reference: &str, status: &str) -> (String, String) {
    let name = status_name(status);
    let link = format!("{}/shop-orders/{reference}", public_url.trim_end_matches('/'));
    (
        format!("Your order from {shop}: {name}"),
        format!(
            "Hello,\n\n{shop} has an update on your order {reference}:\n\n{name}\n\n\
             The shop's note, any tracking number and what happens next are on your order's page:\n{link}\n\n\
             You get this email because you ordered from a shop on MakerFoundry.\n"
        ),
    )
}

/// One noted email: the message id, the status, the order's reference, the customer's email and
/// the shop's name.
type Due = (String, String, String, String, Option<String>);

/// Sends the noted emails (the sweep, after the hub's messages are applied).
pub async fn send_due(state: &AppState, pool: &PgPool) -> Result<(), String> {
    let Some(notices) = state.notices.as_ref() else {
        return Ok(());
    };
    let due: Vec<Due> = sqlx::query_as(
        "SELECT m.message_id, m.status, o.ref, a.email, s.name FROM mf_shop_order_mails m
         JOIN mf_shop_orders o ON o.id = m.order_id
         JOIN mf_accounts a ON a.account_id = o.buyer_account
         LEFT JOIN mf_shops s ON s.party = o.shop_party
         WHERE m.sent_at IS NULL AND m.tries < $1 AND m.created_at > now() - interval '2 days'
         ORDER BY m.created_at, m.message_id LIMIT $2",
    )
    .bind(TRIES)
    .bind(BATCH)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    for (id, status, reference, email, shop) in due {
        let sent = if email.trim().is_empty() {
            // (no address to send to: nothing more to do)
            Ok(())
        } else {
            let shop = shop.unwrap_or_else(|| "The shop".into());
            let (subject, text) = status_mail(&state.public_url, &shop, &reference, &status);
            notices.send_about_order(&email, &subject, &text).await
        };
        let done = match sent {
            Ok(()) => "UPDATE mf_shop_order_mails SET sent_at = now(), tries = tries + 1 WHERE message_id = $1",
            Err(e) => {
                // (by the message's id only: never the address)
                eprintln!("shop orders: email for {id}: {e}");
                "UPDATE mf_shop_order_mails SET tries = tries + 1 WHERE message_id = $1"
            }
        };
        sqlx::query(done)
            .bind(&id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    }
    sqlx::query("DELETE FROM mf_shop_order_mails WHERE created_at < now() - interval '30 days'")
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_email_names_the_status_and_links_to_this_deploys_order_page() {
        let (subject, text) = status_mail(
            "https://staging.makerfoundry.example/",
            "Main Street Threadworks",
            "mf-abc123",
            "in_production",
        );
        assert_eq!(
            subject,
            "Your order from Main Street Threadworks: Being made (in production)"
        );
        assert!(
            text.contains("https://staging.makerfoundry.example/shop-orders/mf-abc123\n"),
            "{text}"
        );
        assert!(text.contains("Being made (in production)"), "{text}");
        let (subject, _) = status_mail("https://makerfoundry.app", "A shop", "mf-1", "received");
        assert_eq!(subject, "Your order from A shop: Received by the shop");
    }

    #[test]
    fn every_status_a_shop_sends_has_words_for_the_customer() {
        for s in super::super::contract::STATUSES {
            assert_ne!(status_name(s), "Unknown", "{s}");
        }
        // the page and the email say "production" for in_production, as the shop does
        assert!(status_name("in_production").contains("production"));
    }
}
