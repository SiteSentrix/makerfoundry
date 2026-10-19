//! The licences a listing can carry (docs/plan.md › Licences), in plain words. The wording is a
//! draft: the owner decides the final text (legal wording is the owner's, docs/start-here.md).

/// What one licence lets people do.
#[derive(Debug, PartialEq)]
pub struct Licence {
    /// Kept in the database.
    pub id: &'static str,
    pub name: &'static str,
    /// What it allows, in a sentence or two.
    pub summary: &'static str,
    /// The licence's own text, for the Creative Commons ones.
    pub url: Option<&'static str>,
}

/// How many items a small business may sell from a design (draft: the owner decides).
pub const SMALL_BUSINESS_ITEMS: u32 = 200;

/// For the designs MakerFoundry's own licences cover: the files themselves are never shared.
const NO_SHARING: &str = "Don't share, sell or give away the design file itself.";

pub const LICENCES: &[Licence] = &[
    Licence {
        id: "personal",
        name: "Personal use",
        summary: "Make things from this design for yourself and as gifts. Don't sell what you make.",
        url: None,
    },
    Licence {
        id: "small-business",
        name: "Small business",
        summary: "Make things from this design and sell up to 200 of them.",
        url: None,
    },
    Licence {
        id: "commercial",
        name: "Commercial",
        summary: "Make things from this design and sell as many as you like.",
        url: None,
    },
    Licence {
        id: "cc-by",
        name: "Creative Commons Attribution (CC BY 4.0)",
        summary: "Anyone may use, change and share this design, for any purpose, as long as they credit its maker.",
        url: Some("https://creativecommons.org/licenses/by/4.0/"),
    },
    Licence {
        id: "cc-by-nc",
        name: "Creative Commons Attribution-NonCommercial (CC BY-NC 4.0)",
        summary: "Anyone may use, change and share this design, crediting its maker, but not to make money.",
        url: Some("https://creativecommons.org/licenses/by-nc/4.0/"),
    },
    Licence {
        id: "cc0",
        name: "Public domain (CC0 1.0)",
        summary: "No rights kept: anyone may use this design for anything.",
        url: Some("https://creativecommons.org/publicdomain/zero/1.0/"),
    },
];

pub fn find(id: &str) -> Option<&'static Licence> {
    LICENCES.iter().find(|l| l.id == id)
}

impl Licence {
    /// The full plain-words text shown on a design's page.
    pub fn plain(&self) -> String {
        match self.url {
            Some(_) => self.summary.to_string(),
            None => format!("{} {NO_SHARING}", self.summary),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn licences_are_found_by_id_and_mf_licences_keep_files_private() {
        assert_eq!(find("commercial").map(|l| l.name), Some("Commercial"));
        assert!(find("nope").is_none());
        assert!(find("personal").unwrap().plain().contains("Don't share"));
        assert!(!find("cc0").unwrap().plain().contains("Don't share"));
        assert!(
            find("small-business")
                .unwrap()
                .summary
                .contains(&SMALL_BUSINESS_ITEMS.to_string())
        );
        let mut ids: Vec<_> = LICENCES.iter().map(|l| l.id).collect();
        ids.dedup();
        assert_eq!(ids.len(), LICENCES.len());
    }
}
