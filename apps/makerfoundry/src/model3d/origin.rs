//! "Originally published on" (MF-53): where an imported model was first posted, the creator's own
//! page there and the date, as the creator gives them. The address must be on the site named
//! (https, its own host or `www.`); MakerFoundry shows it as a link and never fetches it
//! (docs/3d-hub-plan.md § 8).

use crate::pages::escape;

/// The sites a model may say it came from: id, name, host.
pub const SITES: &[(&str, &str, &str)] = &[
    ("makerworld", "MakerWorld", "makerworld.com"),
    ("printables", "Printables", "printables.com"),
    ("thingiverse", "Thingiverse", "thingiverse.com"),
];

/// Where a model was first published, checked.
#[derive(Debug, PartialEq, Eq)]
pub struct Origin {
    pub site: &'static str,
    pub url: Option<String>,
    /// `YYYY-MM-DD`.
    pub date: Option<String>,
}

/// A civil date from days since 1970-01-01 (Howard Hinnant's `civil_from_days`,
/// <https://howardhinnant.github.io/date_algorithms.html>).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// Today (UTC) as `YYYY-MM-DD`.
pub fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (y, m, d) = civil(i64::try_from(secs / 86_400).unwrap_or(0));
    format!("{y:04}-{m:02}-{d:02}")
}

/// A `YYYY-MM-DD` date that exists → (year, month, day).
fn parse_date(text: &str) -> Option<(i64, u32, u32)> {
    let mut parts = text.split('-');
    let (y, m, d) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || y.len() != 4 || m.len() != 2 || d.len() != 2 {
        return None;
    }
    let (y, m, d): (i64, u32, u32) = (y.parse().ok()?, m.parse().ok()?, d.parse().ok()?);
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    (1..=days).contains(&d).then_some((y, m, d))
}

/// Checks what the creator gave (`today` as `YYYY-MM-DD`): nothing at all is fine; an address or a
/// date needs the site; the address must be on that site; the date must exist, from 2000 to today.
pub fn check(site: &str, url: &str, date: &str, today: &str) -> Result<Option<Origin>, String> {
    let (site, url, date) = (site.trim(), url.trim(), date.trim());
    if site.is_empty() && url.is_empty() && date.is_empty() {
        return Ok(None);
    }
    let Some(&(id, name, host)) = SITES.iter().find(|(id, _, _)| *id == site) else {
        return Err("Choose where it was first published (MakerWorld, Printables or Thingiverse).".into());
    };
    let url = if url.is_empty() {
        None
    } else {
        let parsed = url::Url::parse(url).ok();
        let on_site = parsed.as_ref().is_some_and(|u| {
            u.scheme() == "https"
                && u.username().is_empty()
                && u.password().is_none()
                && u.port().is_none()
                && u.host_str()
                    .is_some_and(|h| h == host || h.strip_prefix("www.") == Some(host))
        });
        if !on_site || url.len() > 300 {
            return Err(format!(
                "That address isn't on {name}: give your own page there (https://{host}/…)."
            ));
        }
        parsed.map(|u| u.to_string())
    };
    let date = if date.is_empty() {
        None
    } else {
        let ok = parse_date(date).is_some_and(|(y, _, _)| y >= 2000) && date <= today;
        if !ok {
            return Err("Give the date it was first published as YYYY-MM-DD, not after today.".into());
        }
        Some(date.to_string())
    };
    Ok(Some(Origin { site: id, url, date }))
}

/// "Originally published on MakerWorld, 12 March 2025", the site's name linking to the creator's
/// page there (HTML).
pub fn line(site: &str, url: Option<&str>, date: Option<&str>) -> String {
    let Some(&(_, name, _)) = SITES.iter().find(|(id, _, _)| *id == site) else {
        return String::new();
    };
    let named = match url {
        Some(u) => format!(r#"<a href="{}" rel="nofollow noopener">{name}</a>"#, escape(u)),
        None => name.to_string(),
    };
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let when = date
        .and_then(parse_date)
        .and_then(|(y, m, d)| Some(format!(", {d} {} {y}", MONTHS.get(m as usize - 1)?)))
        .unwrap_or_default();
    format!("Originally published on {named}{when}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_become_dates() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(20_525), (2026, 3, 13));
        assert_eq!(civil(11_016), (2000, 2, 29));
        assert_eq!(today().len(), 10);
    }

    #[test]
    fn origins_are_checked() {
        let t = "2026-10-30";
        assert_eq!(check("", "", "", t), Ok(None));
        let ok = check("makerworld", "https://makerworld.com/en/models/12345", "2025-03-12", t)
            .unwrap()
            .unwrap();
        assert_eq!(
            (ok.site, ok.url.as_deref(), ok.date.as_deref()),
            (
                "makerworld",
                Some("https://makerworld.com/en/models/12345"),
                Some("2025-03-12")
            )
        );
        assert!(check("thingiverse", "https://www.thingiverse.com/thing:1", "", t).is_ok());
        assert!(check("printables", "", "", t).is_ok(), "the site alone");
        for (site, url, date) in [
            ("", "https://www.printables.com/model/1", ""),
            ("myspace", "", ""),
            ("makerworld", "https://evil.example/makerworld.com", ""),
            ("makerworld", "http://makerworld.com/en/models/1", ""),
            ("makerworld", "https://makerworld.com.evil.example/", ""),
            ("makerworld", "https://user@makerworld.com/", ""),
            ("makerworld", "https://makerworld.com:8443/", ""),
            ("makerworld", "javascript:alert(1)", ""),
            ("printables", "https://makerworld.com/x", ""),
            ("printables", "", "2025-02-29"),
            ("printables", "", "2024-13-01"),
            ("printables", "", "1999-12-31"),
            ("printables", "", "2026-10-31"),
            ("printables", "", "12/03/2025"),
        ] {
            assert!(check(site, url, date, t).is_err(), "{site} {url} {date}");
        }
        assert!(check("printables", "", "2024-02-29", t).is_ok(), "a leap day");
        assert!(check("printables", "", "2026-10-30", t).is_ok(), "today");
    }

    #[test]
    fn the_line_reads_well() {
        assert_eq!(
            line(
                "makerworld",
                Some("https://makerworld.com/en/models/1"),
                Some("2025-03-12")
            ),
            r#"Originally published on <a href="https://makerworld.com/en/models/1" rel="nofollow noopener">MakerWorld</a>, 12 March 2025"#
        );
        assert_eq!(line("thingiverse", None, None), "Originally published on Thingiverse");
        assert_eq!(line("nope", None, None), "");
    }
}
