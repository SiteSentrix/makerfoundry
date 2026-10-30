//! MakerFoundry 3D (MF-35…): printable models, their categories, and (in the modules beside this
//! one) the readers for the files makers upload. See docs/3d-hub-plan.md.

pub mod files;
pub mod import;
pub mod mesh;
pub mod obj;
pub mod render;
pub mod stl;
pub mod threemf;
pub mod upload;
pub mod zip;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::{Html, IntoResponse, Response};

use crate::AppState;
use crate::accounts::{User, current_user, private};
use crate::pages::{escape, page};

/// A 3D category: `id` is kept in the database, `name` is shown.
#[derive(Debug, PartialEq, Eq)]
pub struct Category {
    pub id: &'static str,
    pub name: &'static str,
}

/// The 14 categories (owner's decision, docs/3d-hub-plan.md § 14).
pub const CATEGORIES: &[Category] = &[
    Category {
        id: "household",
        name: "Household",
    },
    Category {
        id: "tools-workshop",
        name: "Tools & workshop",
    },
    Category {
        id: "organisers-storage",
        name: "Organisers & storage",
    },
    Category {
        id: "toys-games",
        name: "Toys & games",
    },
    Category {
        id: "cosplay-props",
        name: "Cosplay & props",
    },
    Category {
        id: "miniatures-tabletop",
        name: "Miniatures & tabletop",
    },
    Category {
        id: "art-decor",
        name: "Art & decor",
    },
    Category {
        id: "fashion-jewellery",
        name: "Fashion & jewellery",
    },
    Category {
        id: "gadgets-tech-mounts",
        name: "Gadgets & tech mounts",
    },
    Category {
        id: "education",
        name: "Education",
    },
    Category {
        id: "garden-outdoor",
        name: "Garden & outdoor",
    },
    Category {
        id: "automotive",
        name: "Automotive",
    },
    Category {
        id: "printer-parts",
        name: "3D printer parts & upgrades",
    },
    Category {
        id: "other",
        name: "Other",
    },
];

/// A category by id.
pub fn category(id: &str) -> Option<&'static Category> {
    CATEGORIES.iter().find(|c| c.id == id)
}

/// Whether `viewer` may see MakerFoundry 3D: everyone once it's switched on (`MF_3D=yes`), admins
/// before.
pub fn open_to(state: &AppState, viewer: Option<&User>) -> bool {
    state.three_d || viewer.is_some_and(|u| state.admins.contains(&u.account_id))
}

/// `/3d`: the 3D section's front page.
pub async fn landing(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let viewer = current_user(&state, &headers).await;
    if !open_to(&state, viewer.as_ref()) {
        return crate::not_found().await.into_response();
    }
    let categories: String = CATEGORIES
        .iter()
        .map(|c| format!("<li>{}</li>", escape(c.name)))
        .collect();
    let preview = if state.three_d {
        ""
    } else {
        r#"<p class="note">Only admins see this until MakerFoundry 3D opens.</p>"#
    };
    let body = format!(
        r#"<section class="hero">
  <h1>MakerFoundry 3D</h1>
  <p class="lead">Printable models shared and sold by the makers who designed them: STL, 3MF, OBJ and STEP, with the print settings that worked.</p>
  {preview}
</section>
<section>
  <h2>Categories</h2>
  <ul class="categories">{categories}</ul>
</section>"#
    );
    let title = "MakerFoundry 3D · MakerFoundry";
    match viewer {
        // (the admins' look before opening isn't for caches)
        Some(_) if !state.three_d => private(title, &body, true),
        Some(_) => Html(page(title, &body, true)).into_response(),
        None => Html(page(title, &body, false)).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_category_is_accepted_and_nothing_else() {
        assert_eq!(CATEGORIES.len(), 14);
        for c in CATEGORIES {
            assert_eq!(category(c.id).map(|f| f.name), Some(c.name));
        }
        assert!(category("nope").is_none());
        assert!(category("").is_none());
        let mut ids: Vec<_> = CATEGORIES.iter().map(|c| c.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), CATEGORIES.len());
    }
}
