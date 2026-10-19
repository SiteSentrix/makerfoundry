//! Stripe (MF-4): MakerFoundry's calls to its API (https://docs.stripe.com/api), and checking that
//! a webhook came from Stripe (https://docs.stripe.com/webhooks#verify-manually).
//!
//! Sellers have Stripe Express accounts (https://docs.stripe.com/connect/express-accounts); a sale
//! is a Checkout Session with a destination charge
//! (https://docs.stripe.com/connect/destination-charges): the buyer pays MakerFoundry's Stripe
//! account, Stripe moves the price less the application fee to the seller's, and pays the seller
//! out on their schedule (monthly). Card details only ever reach Stripe's own pages.
//!
//! The secret key and the webhook signing secrets come from Railway's variables and are never
//! logged or shown.

use std::time::Duration;

use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::Sha256;

/// Stripe's API, unless `STRIPE_API_URL` says otherwise (tests use a stand-in).
pub const DEFAULT_API: &str = "https://api.stripe.com";
/// How old a webhook's signature may be: Stripe's own libraries' default.
pub const TOLERANCE_SECS: i64 = 300;

#[derive(Clone)]
pub struct Stripe {
    api: String,
    key: String,
    /// Webhook signing secrets: the platform endpoint's, and the connected accounts' endpoint's.
    webhook_secrets: Vec<String>,
    /// MakerFoundry's address, for the links Stripe sends people back to.
    pub public_url: String,
    http: reqwest::Client,
}

impl std::fmt::Debug for Stripe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Stripe")
    }
}

/// Stripe said no, or couldn't be reached.
#[derive(Debug)]
pub struct StripeError(pub String);

impl std::fmt::Display for StripeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl Stripe {
    pub fn new(api: &str, key: &str, webhook_secrets: Vec<String>, public_url: &str) -> Result<Stripe, String> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .no_proxy()
            .build()
            .map_err(|e| format!("can't make an HTTP client: {e}"))?;
        Ok(Stripe {
            api: api.trim_end_matches('/').to_string(),
            key: key.to_string(),
            webhook_secrets,
            public_url: public_url.trim_end_matches('/').to_string(),
            http,
        })
    }

    /// From `STRIPE_SECRET_KEY`, `STRIPE_WEBHOOK_SECRET` (and `STRIPE_CONNECT_WEBHOOK_SECRET`),
    /// `STRIPE_API_URL` and `MF_PUBLIC_URL`: `None` without a key (nothing can be sold).
    pub fn from_env(var: impl Fn(&str) -> Option<String>) -> Result<Option<Stripe>, String> {
        let Some(key) = var("STRIPE_SECRET_KEY").filter(|k| !k.trim().is_empty()) else {
            return Ok(None);
        };
        let secrets: Vec<String> = ["STRIPE_WEBHOOK_SECRET", "STRIPE_CONNECT_WEBHOOK_SECRET"]
            .iter()
            .filter_map(|k| var(k))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if secrets.is_empty() {
            return Err("STRIPE_SECRET_KEY is set but STRIPE_WEBHOOK_SECRET isn't: sales couldn't be confirmed".into());
        }
        let api = var("STRIPE_API_URL").unwrap_or_else(|| DEFAULT_API.to_string());
        let public = var("MF_PUBLIC_URL").unwrap_or_else(|| "https://makerfoundry.app".to_string());
        Stripe::new(&api, key.trim(), secrets, &public).map(Some)
    }

    /// Whether the key is a test-mode key (`sk_test_…`).
    pub fn test_mode(&self) -> bool {
        self.key.starts_with("sk_test_") || self.key.starts_with("rk_test_")
    }

    /// `POST {path}` with a form; `idempotency` makes a repeat return the first answer
    /// (https://docs.stripe.com/api/idempotent_requests).
    pub async fn post(
        &self,
        path: &str,
        form: &[(&str, String)],
        idempotency: Option<&str>,
    ) -> Result<Value, StripeError> {
        let mut req = self
            .http
            .post(format!("{}{path}", self.api))
            .bearer_auth(&self.key)
            .form(form);
        if let Some(k) = idempotency {
            req = req.header("Idempotency-Key", k);
        }
        answer(req.send().await).await
    }

    pub async fn get(&self, path: &str) -> Result<Value, StripeError> {
        answer(
            self.http
                .get(format!("{}{path}", self.api))
                .bearer_auth(&self.key)
                .send()
                .await,
        )
        .await
    }

    /// Whether `header` (`Stripe-Signature`) signs `payload` with one of our secrets, at most
    /// `TOLERANCE_SECS` from `now` (Unix seconds).
    pub fn verify(&self, header: &str, payload: &[u8], now: i64) -> bool {
        let mut t: Option<i64> = None;
        let mut signatures: Vec<Vec<u8>> = Vec::new();
        for item in header.split(',') {
            match item.trim().split_once('=') {
                Some(("t", v)) => t = v.parse().ok(),
                Some(("v1", v)) => {
                    if let Some(bytes) = unhex(v) {
                        signatures.push(bytes);
                    }
                }
                _ => {}
            }
        }
        let Some(t) = t else { return false };
        if (now - t).abs() > TOLERANCE_SECS || signatures.len() > 8 {
            return false;
        }
        self.webhook_secrets.iter().any(|secret| {
            signatures.iter().any(|sig| {
                let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) else {
                    return false;
                };
                mac.update(format!("{t}.").as_bytes());
                mac.update(payload);
                // (compares in constant time)
                mac.verify_slice(sig).is_ok()
            })
        })
    }
}

/// The `v1` signature of `payload` at time `t` with `secret`, as Stripe makes it (hex).
pub fn signature(secret: &str, t: i64, payload: &[u8]) -> String {
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) else {
        return String::new();
    };
    mac.update(format!("{t}.").as_bytes());
    mac.update(payload);
    mac.finalize().into_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if text.len() != 64 {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| text.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok()))
        .collect()
}

async fn answer(sent: Result<reqwest::Response, reqwest::Error>) -> Result<Value, StripeError> {
    let resp = sent.map_err(|_| StripeError("Stripe can't be reached".into()))?;
    let ok = resp.status().is_success();
    let status = resp.status().as_u16();
    let v: Value = resp
        .json()
        .await
        .map_err(|_| StripeError(format!("Stripe's answer ({status}) can't be read")))?;
    if ok {
        Ok(v)
    } else {
        let message = v
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or("no message");
        Err(StripeError(format!("Stripe refused ({status}): {message}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stripe() -> Stripe {
        Stripe::new(
            DEFAULT_API,
            "sk_test_x",
            vec!["whsec_one".into(), "whsec_two".into()],
            "https://makerfoundry.app/",
        )
        .unwrap()
    }

    #[test]
    fn webhooks_signed_by_stripe_are_believed_and_nothing_else() {
        let s = stripe();
        let body = br#"{"id":"evt_1"}"#;
        let now = 1_700_000_000;
        let good = format!("t={now},v1={}", signature("whsec_one", now, body));
        assert!(s.verify(&good, body, now));
        assert!(s.verify(&good, body, now + TOLERANCE_SECS));
        // the connected accounts' endpoint has its own secret
        let other = format!("t={now},v1={}", signature("whsec_two", now, body));
        assert!(s.verify(&other, body, now));
        // several signatures (a secret being rolled): any good one will do
        assert!(s.verify(
            &format!("t={now},v1={},v1={}", "0".repeat(64), signature("whsec_one", now, body)),
            body,
            now
        ));
        for bad in [
            format!("t={now},v1={}", signature("whsec_wrong", now, body)),
            format!("t={},v1={}", now + 1, signature("whsec_one", now, body)),
            format!("v1={}", signature("whsec_one", now, body)),
            format!("t={now}"),
            format!("t={now},v0={}", signature("whsec_one", now, body)),
            String::new(),
            "t=x,v1=zz".into(),
        ] {
            assert!(!s.verify(&bad, body, now), "{bad}");
        }
        // changed payload, or too old a signature
        assert!(!s.verify(&good, br#"{"id":"evt_2"}"#, now));
        assert!(!s.verify(&good, body, now + TOLERANCE_SECS + 1));
        assert!(!s.verify(&good, body, now - TOLERANCE_SECS - 1));
    }

    #[test]
    fn settings_come_from_the_environment() {
        let none = |_: &str| None;
        assert!(Stripe::from_env(none).unwrap().is_none());
        let key_only = |k: &str| (k == "STRIPE_SECRET_KEY").then(|| "sk_test_1".to_string());
        assert!(Stripe::from_env(key_only).is_err());
        let both = |k: &str| match k {
            "STRIPE_SECRET_KEY" => Some("sk_test_1".to_string()),
            "STRIPE_WEBHOOK_SECRET" => Some("whsec_1".to_string()),
            _ => None,
        };
        let s = Stripe::from_env(both).unwrap().unwrap();
        assert!(s.test_mode());
        assert_eq!(s.public_url, "https://makerfoundry.app");
        assert_eq!(format!("{s:?}"), "Stripe");
    }
}
