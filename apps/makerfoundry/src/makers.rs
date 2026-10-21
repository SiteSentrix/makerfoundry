//! Makers' public profiles (MF-3): the name and address a maker is shown by, since an account's
//! email is never shown to anyone. A maker sets them once before publishing; `/makers/<handle>`
//! lists their published designs.

use axum::Form;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use serde::Deserialize;
use sqlx::PgPool;

use crate::AppState;
use crate::accounts::{current_user, forbidden, private, same_origin};
use crate::listings::{cards, paragraphs};
use crate::pages::{escape, page};

/// A maker as others see them.
#[derive(Debug, Clone)]
pub struct Maker {
    pub handle: String,
    pub name: String,
    pub bio: String,
}

pub async fn profile(pool: &PgPool, account_id: i64) -> Option<Maker> {
    sqlx::query_as::<_, (String, String, String)>("SELECT handle, name, bio FROM mf_makers WHERE account_id = $1")
        .bind(account_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .map(|(handle, name, bio)| Maker { handle, name, bio })
}

/// Why `handle` can't be an address, if it can't: 3 to 30 lower-case letters, digits and hyphens,
/// starting with a letter or digit.
pub fn check_handle(handle: &str) -> Result<(), String> {
    let ok_chars = handle
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    let first_ok = handle.bytes().next().is_some_and(|b| b != b'-');
    if (3..=30).contains(&handle.len()) && ok_chars && first_ok {
        Ok(())
    } else {
        Err("The address needs 3 to 30 lower-case letters, digits or hyphens, and can't start with a hyphen.".into())
    }
}

fn form(name: &str, handle: &str, bio: &str, problem: &str) -> String {
    let problem = if problem.is_empty() {
        String::new()
    } else {
        format!(r#"<p class="problem" role="alert">{}</p>"#, escape(problem))
    };
    format!(
        r#"<section class="form">
<h1>Your maker profile</h1>
<p class="lead">How people see you on MakerFoundry. Your email is never shown.</p>
{problem}
<form method="post" action="/account/maker">
<label>Name <small>(your name or your shop's)</small><input name="name" value="{name}" required maxlength="60"></label>
<label>Address <small>(makerfoundry.app/makers/<em>this</em>: lower-case letters, digits, hyphens)</small><input name="handle" value="{handle}" required minlength="3" maxlength="30" pattern="[a-z0-9][a-z0-9\-]{{2,29}}"></label>
<label>About you <small>(optional)</small><textarea name="bio" rows="4" maxlength="1000">{bio}</textarea></label>
<button type="submit">Save</button>
</form>
</section>"#,
        name = escape(name),
        handle = escape(handle),
        bio = escape(bio),
    )
}

pub async fn edit_page(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let (Some(user), Some(pool)) = (current_user(&state, &headers).await, state.db.as_ref()) else {
        return Redirect::to("/signin").into_response();
    };
    let m = profile(pool, user.account_id).await;
    let (name, handle, bio) = m.map(|m| (m.name, m.handle, m.bio)).unwrap_or_default();
    private(
        "Your maker profile · MakerFoundry",
        &form(&name, &handle, &bio, ""),
        true,
    )
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct ProfileForm {
    name: String,
    handle: String,
    bio: String,
}

pub async fn save(State(state): State<AppState>, headers: HeaderMap, Form(f): Form<ProfileForm>) -> Response {
    if !same_origin(&headers) {
        return forbidden();
    }
    let (Some(user), Some(pool)) = (current_user(&state, &headers).await, state.db.as_ref()) else {
        return Redirect::to("/signin").into_response();
    };
    let name = f.name.trim().to_string();
    let handle = f.handle.trim().to_lowercase();
    let bio = f.bio.trim().replace("\r\n", "\n");
    let again = |why: &str| {
        private(
            "Your maker profile · MakerFoundry",
            &form(&name, &handle, &bio, why),
            true,
        )
    };
    if name.is_empty() || name.chars().count() > 60 || name.chars().any(char::is_control) {
        return again("Give a name of up to 60 characters.");
    }
    if let Err(why) = check_handle(&handle) {
        return again(&why);
    }
    if bio.chars().count() > 1000 {
        return again("About you can be up to 1,000 characters.");
    }
    let saved = sqlx::query(
        "INSERT INTO mf_makers (account_id, handle, name, bio) VALUES ($1, $2, $3, $4)
         ON CONFLICT (account_id) DO UPDATE SET handle = EXCLUDED.handle, name = EXCLUDED.name, bio = EXCLUDED.bio",
    )
    .bind(user.account_id)
    .bind(&handle)
    .bind(&name)
    .bind(&bio)
    .execute(pool)
    .await;
    match saved {
        Ok(_) => Redirect::to("/account").into_response(),
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => again("That address is taken: try another."),
        Err(e) => {
            eprintln!("makers: {e}");
            again("Your profile couldn't be saved just now. Please try again.")
        }
    }
}

pub async fn show(State(state): State<AppState>, headers: HeaderMap, Path(handle): Path<String>) -> Response {
    let viewer = current_user(&state, &headers).await;
    let signed_in = viewer.is_some();
    let not_found = || {
        (
            StatusCode::NOT_FOUND,
            Html(page(
                "Not found · MakerFoundry",
                r#"<section class="hero"><h1>Maker not found</h1></section>"#,
                signed_in,
            )),
        )
            .into_response()
    };
    let Some(pool) = state.db.as_ref() else {
        return not_found();
    };
    if check_handle(&handle).is_err() {
        return not_found();
    }
    let row: Option<(i64, String, String)> =
        sqlx::query_as("SELECT account_id, name, bio FROM mf_makers WHERE handle = $1")
            .bind(&handle)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    let Some((account_id, name, bio)) = row else {
        return not_found();
    };
    let designs = cards(pool, account_id, 200).await;
    let collections = crate::collections::public_html(pool, account_id).await;
    let follow = crate::follows::button_html(pool, viewer.as_ref(), account_id, &handle).await;
    let list = if designs.is_empty() {
        r#"<p class="muted">No designs published yet.</p>"#.to_string()
    } else {
        designs
    };
    let url = format!("{}/makers/{handle}", state.public_url);
    let description = if bio.trim().is_empty() {
        format!("Embroidery and laser designs by {name} on MakerFoundry")
    } else {
        crate::pages::summary(&bio, 200)
    };
    let preview = crate::pages::Preview {
        title: &name,
        description: &description,
        url: &url,
        image: None,
    };
    Html(crate::pages::page_with(
        &format!("{} · MakerFoundry", escape(&name)),
        &format!(
            r#"<section class="design"><h1>{}</h1>{follow}{}<h2>Designs</h2>{list}{collections}</section>"#,
            escape(&name),
            paragraphs(&bio)
        ),
        signed_in,
        Some(&preview),
    ))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_are_short_lower_case_words() {
        for ok in ["ann", "ann-s-stitches", "shop42", "a1b"] {
            assert!(check_handle(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "ab",
            "-ann",
            "Ann",
            "ann s",
            "ann/../x",
            "ann_s",
            &"a".repeat(31),
            "ännie",
        ] {
            assert!(check_handle(bad).is_err(), "{bad}");
        }
    }
}
