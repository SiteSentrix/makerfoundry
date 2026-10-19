//! Money (MF-4), always whole US cents in an `i64`, never a float.
//!
//! The owner's decision (2026-10-19): MakerFoundry keeps 10% of each sale, the card fee (2.9% +
//! 30¢, Stripe's standard US card price, https://stripe.com/pricing) also comes out of the seller's
//! share, and sellers are paid monthly.

/// The lowest price of a paid design: $1.
pub const MIN_PRICE: i64 = 100;
/// The highest: $200.
pub const MAX_PRICE: i64 = 20_000;

/// A sale's price, split three ways.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Split {
    pub price: i64,
    /// MakerFoundry's 10%.
    pub fee: i64,
    /// The card fee: 2.9% + 30¢.
    pub processing: i64,
    /// What the seller is paid.
    pub seller: i64,
}

impl Split {
    /// What Stripe sends MakerFoundry out of the sale (the rest goes to the seller's account).
    pub fn application_fee(&self) -> i64 {
        self.fee + self.processing
    }
}

/// `price` (cents, `MIN_PRICE..=MAX_PRICE`) split: each part rounded to the nearest cent (halves
/// up), the seller getting what's left, so the parts always add up to the price.
pub fn split(price: i64) -> Option<Split> {
    if !(MIN_PRICE..=MAX_PRICE).contains(&price) {
        return None;
    }
    let fee = (price * 10 + 50) / 100;
    let processing = (price * 29 + 500) / 1000 + 30;
    let seller = price - fee - processing;
    (seller > 0).then_some(Split {
        price,
        fee,
        processing,
        seller,
    })
}

/// Cents as dollars: `$4.50`, `$12`, `-$0.33`.
pub fn dollars(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let c = cents.unsigned_abs();
    if c.is_multiple_of(100) {
        format!("{sign}${}", c / 100)
    } else {
        format!("{sign}${}.{:02}", c / 100, c % 100)
    }
}

/// Dollars as typed (`4`, `4.5`, `$4.50`, `1,000`) → cents; `None` if it isn't an amount with at
/// most two decimals. Empty is 0.
pub fn parse_dollars(text: &str) -> Option<i64> {
    let t: String = text
        .trim()
        .trim_start_matches('$')
        .chars()
        .filter(|c| *c != ',')
        .collect();
    if t.is_empty() {
        return Some(0);
    }
    let (whole, part) = t.split_once('.').unwrap_or((&t, ""));
    if whole.len() > 7 || part.len() > 2 || !(whole.bytes().chain(part.bytes())).all(|b| b.is_ascii_digit()) {
        return None;
    }
    if whole.is_empty() && part.is_empty() {
        return None;
    }
    let whole: i64 = if whole.is_empty() { 0 } else { whole.parse().ok()? };
    let part: i64 = match part.len() {
        0 => 0,
        1 => part.parse::<i64>().ok()? * 10,
        _ => part.parse().ok()?,
    };
    Some(whole * 100 + part)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sale_is_split_to_the_cent() {
        assert_eq!(
            split(500),
            Some(Split {
                price: 500,
                fee: 50,
                processing: 45,
                seller: 405
            })
        );
        assert_eq!(
            split(100),
            Some(Split {
                price: 100,
                fee: 10,
                processing: 33,
                seller: 57
            })
        );
        assert_eq!(
            split(20_000).map(|s| (s.fee, s.processing, s.seller)),
            Some((2000, 610, 17_390))
        );
        assert_eq!(split(99), None);
        assert_eq!(split(0), None);
        assert_eq!(split(20_001), None);
    }

    #[test]
    fn every_price_splits_into_parts_that_add_up() {
        for price in MIN_PRICE..=MAX_PRICE {
            let s = split(price).unwrap();
            assert_eq!(s.fee + s.processing + s.seller, price);
            assert!(s.seller > 0 && s.fee >= 0 && s.processing >= 30);
            // within half a cent of the exact shares
            assert!((s.fee * 100 - price * 10).abs() <= 50, "{price}");
            assert!(((s.processing - 30) * 1000 - price * 29).abs() <= 500, "{price}");
            assert_eq!(s.application_fee(), s.fee + s.processing);
        }
    }

    #[test]
    fn dollars_are_written_and_read() {
        assert_eq!(dollars(450), "$4.50");
        assert_eq!(dollars(1200), "$12");
        assert_eq!(dollars(5), "$0.05");
        assert_eq!(dollars(-33), "-$0.33");
        for (text, cents) in [
            ("4", 400),
            ("4.5", 450),
            ("$4.50", 450),
            (" 12.05 ", 1205),
            ("1,000", 100_000),
            (".99", 99),
            ("", 0),
            ("0", 0),
        ] {
            assert_eq!(parse_dollars(text), Some(cents), "{text}");
        }
        for bad in ["4.505", "-4", "four", "4.5.0", ".", "1e3", "99999999", "4 50"] {
            assert_eq!(parse_dollars(bad), None, "{bad}");
        }
    }
}
