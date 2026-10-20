//! Rate limits (MF-5): how often one address or one account may do something, counted in memory
//! over a sliding window. One server runs the site, so memory is enough; a restart forgets.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::http::HeaderMap;

/// What's limited, how many times, in how long.
#[derive(Clone, Copy, Debug)]
pub struct Rule {
    pub name: &'static str,
    pub max: usize,
    pub per: Duration,
}

const fn rule(name: &'static str, max: usize, minutes: u64) -> Rule {
    Rule {
        name,
        max,
        per: Duration::from_secs(minutes * 60),
    }
}

/// Accounts made from one address an hour.
pub const SIGNUP: Rule = rule("signup", 10, 60);
/// Sign-in attempts from one address in 15 minutes (the account service also limits each email).
pub const SIGNIN: Rule = rule("signin", 30, 15);
/// Password-reset emails asked for from one address an hour.
pub const FORGOT: Rule = rule("forgot", 10, 60);
/// Files previewed by one account an hour.
pub const PREVIEW: Rule = rule("preview", 60, 60);
/// Designs added by one account a day.
pub const UPLOAD: Rule = rule("upload", 50, 24 * 60);
/// Reports and notices from one address an hour.
pub const REPORT: Rule = rule("report", 10, 60);
/// Converted downloads (MF-9) by one address an hour: each is made on the spot.
pub const CONVERT: Rule = rule("convert", 120, 60);
/// Comments by one account an hour.
pub const COMMENT: Rule = rule("comment", 20, 60);
/// Purchases started (Stripe checkout pages made) by one account an hour.
pub const CHECKOUT: Rule = rule("checkout", 30, 60);

/// The most designs one account may keep.
pub const MAX_DESIGNS: i64 = 500;

#[derive(Default)]
pub struct Limits {
    hits: Mutex<HashMap<(&'static str, String), VecDeque<Instant>>>,
}

impl std::fmt::Debug for Limits {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Limits")
    }
}

impl Limits {
    /// Counts one more for `key` under `rule`: `true` if it's allowed, `false` if over the limit
    /// (and then it isn't counted).
    pub fn allow(&self, rule: Rule, key: &str) -> bool {
        self.allow_at(rule, key, Instant::now())
    }

    /// Forgets every key not used within the longest window (a day): what the privacy policy
    /// promises about network addresses. Run every few minutes.
    pub fn sweep(&self) {
        self.sweep_at(Instant::now());
    }

    fn sweep_at(&self, now: Instant) {
        let mut hits = self.hits.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        hits.retain(|_, times| times.back().is_some_and(|t| now.duration_since(*t) < UPLOAD.per));
    }

    fn allow_at(&self, rule: Rule, key: &str, now: Instant) -> bool {
        // (a panic elsewhere while holding the lock leaves only counts behind: carry on)
        let mut hits = self.hits.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if hits.len() > 100_000 {
            hits.retain(|(name, _), times| {
                let per = match *name {
                    "upload" => UPLOAD.per,
                    _ => Duration::from_secs(3600),
                };
                times.back().is_some_and(|t| now.duration_since(*t) < per)
            });
        }
        let times = hits.entry((rule.name, key.to_string())).or_default();
        while times.front().is_some_and(|t| now.duration_since(*t) >= rule.per) {
            times.pop_front();
        }
        if times.len() >= rule.max {
            return false;
        }
        times.push_back(now);
        true
    }
}

/// The address a request came from: Railway's proxy gives it in `X-Real-IP`, or first in
/// `X-Forwarded-For`; "unknown" without either (local runs, tests).
pub fn client(headers: &HeaderMap) -> String {
    let real = headers
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let forwarded = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(str::trim)
        .filter(|v| !v.is_empty());
    real.or(forwarded).unwrap_or("unknown").chars().take(64).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_limit_holds_for_its_window_then_lets_go() {
        let limits = Limits::default();
        let r = Rule {
            name: "t",
            max: 3,
            per: Duration::from_secs(60),
        };
        let t0 = Instant::now();
        for i in 0..3 {
            assert!(limits.allow_at(r, "a", t0 + Duration::from_secs(i)));
        }
        assert!(
            !limits.allow_at(r, "a", t0 + Duration::from_secs(10)),
            "a fourth within the minute"
        );
        assert!(
            limits.allow_at(r, "b", t0 + Duration::from_secs(10)),
            "another key has its own count"
        );
        assert!(
            limits.allow_at(r, "a", t0 + Duration::from_secs(61)),
            "the first has left the window"
        );
        // a day later, nothing of either address is kept
        limits.sweep_at(t0 + Duration::from_secs(61) + UPLOAD.per);
        assert!(limits.hits.lock().unwrap().is_empty());
    }

    #[test]
    fn the_client_address_comes_from_the_proxy() {
        let mut h = HeaderMap::new();
        assert_eq!(client(&h), "unknown");
        h.insert("x-forwarded-for", "203.0.113.9, 10.0.0.1".parse().unwrap());
        assert_eq!(client(&h), "203.0.113.9");
        h.insert("x-real-ip", "198.51.100.7".parse().unwrap());
        assert_eq!(client(&h), "198.51.100.7");
    }
}
