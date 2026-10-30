//! Posting showcase items (issue #30): when the owner connects MakerFoundry's Facebook Page (and
//! its Instagram account), admins can post an item from the showcase queue with one button
//! instead of by hand, through Meta's Graph API: a Page photo
//! (<https://developers.facebook.com/docs/graph-api/reference/page/photos/>) and Instagram content
//! publishing, a media container then its publishing
//! (<https://developers.facebook.com/docs/instagram-platform/content-publishing>). Only active
//! consents post, and the caption credits only the creator's display name (`showcase::caption`).
//! When a creator withdraws, their Facebook posts are deleted through the API; Instagram's API
//! can't delete, so those stay flagged for the admins to take down by hand. YouTube takes video,
//! and showcase items are pictures, so YouTube stays by hand.
//!
//! Off until `MF_FACEBOOK_PAGE_ID` and `MF_FACEBOOK_PAGE_TOKEN` are set (and
//! `MF_INSTAGRAM_USER_ID` for Instagram). The token is a secret: never logged or shown.

use std::time::Duration;

/// The Graph API version used (to be kept current when the owner connects the Page).
const VERSION: &str = "v21.0";

/// MakerFoundry's Page (and Instagram account) on Meta.
#[derive(Clone)]
pub struct Social {
    page: String,
    token: String,
    instagram: Option<String>,
    graph: String,
    http: reqwest::Client,
}

impl std::fmt::Debug for Social {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // (never the token)
        f.write_str("Social")
    }
}

/// A post made: its platform's id for it, and its address.
#[derive(Debug, PartialEq, Eq)]
pub struct Posted {
    pub remote_id: String,
    pub url: String,
}

impl Social {
    pub fn new(page: &str, token: &str, instagram: &str, graph: Option<&str>) -> Option<Self> {
        let (page, token) = (page.trim(), token.trim());
        if page.is_empty() || token.is_empty() || !page.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let instagram = instagram.trim();
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .ok()?;
        Some(Social {
            page: page.to_string(),
            token: token.to_string(),
            instagram: (!instagram.is_empty() && instagram.bytes().all(|b| b.is_ascii_digit()))
                .then(|| instagram.to_string()),
            graph: graph
                .unwrap_or("https://graph.facebook.com")
                .trim_end_matches('/')
                .to_string(),
            http,
        })
    }

    /// The platforms this can post to.
    pub fn platforms(&self) -> Vec<&'static str> {
        let mut p = vec!["facebook"];
        if self.instagram.is_some() {
            p.push("instagram");
        }
        p
    }

    async fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        form: &[(&str, &str)],
    ) -> Result<serde_json::Value, String> {
        let mut fields: Vec<(&str, &str)> = form.to_vec();
        fields.push(("access_token", &self.token));
        let answer = self
            .http
            .request(method, format!("{}/{VERSION}/{path}", self.graph))
            .form(&fields)
            .send()
            .await
            .map_err(|e| format!("graph: {}", e.without_url()))?;
        let status = answer.status();
        let bytes = answer
            .bytes()
            .await
            .map_err(|e| format!("graph: {}", e.without_url()))?;
        if bytes.len() > 64 * 1024 {
            return Err("graph: an answer too large".into());
        }
        let v: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| "graph: not JSON".to_string())?;
        if !status.is_success() {
            let why = v
                .pointer("/error/message")
                .and_then(|m| m.as_str())
                .unwrap_or("an error");
            return Err(format!("graph {status}: {}", why.chars().take(200).collect::<String>()));
        }
        Ok(v)
    }

    /// Posts a picture (at a public `image` address) with `caption` to `platform`.
    pub async fn post(&self, platform: &str, image: &str, caption: &str) -> Result<Posted, String> {
        match platform {
            "facebook" => {
                let v = self
                    .call(
                        reqwest::Method::POST,
                        &format!("{}/photos", self.page),
                        &[("url", image), ("caption", caption)],
                    )
                    .await?;
                let id = v
                    .get("post_id")
                    .or_else(|| v.get("id"))
                    .and_then(|x| x.as_str())
                    .filter(|id| id.bytes().all(|b| b.is_ascii_digit() || b == b'_'))
                    .ok_or("graph: no post id")?;
                Ok(Posted {
                    remote_id: id.to_string(),
                    url: format!("https://www.facebook.com/{id}"),
                })
            }
            "instagram" => {
                let ig = self.instagram.as_deref().ok_or("Instagram isn't connected")?;
                let made = self
                    .call(
                        reqwest::Method::POST,
                        &format!("{ig}/media"),
                        &[("image_url", image), ("caption", caption)],
                    )
                    .await?;
                let container = made
                    .get("id")
                    .and_then(|x| x.as_str())
                    .ok_or("graph: no media container")?;
                let done = self
                    .call(
                        reqwest::Method::POST,
                        &format!("{ig}/media_publish"),
                        &[("creation_id", container)],
                    )
                    .await?;
                let id = done
                    .get("id")
                    .and_then(|x| x.as_str())
                    .filter(|id| id.bytes().all(|b| b.is_ascii_digit()))
                    .ok_or("graph: no media id")?;
                let permalink = self
                    .call(reqwest::Method::GET, &format!("{id}?fields=permalink"), &[])
                    .await
                    .ok()
                    .and_then(|v| v.get("permalink")?.as_str().map(str::to_string))
                    .filter(|p| p.starts_with("https://www.instagram.com/"))
                    .unwrap_or_else(|| "https://www.instagram.com/".to_string());
                Ok(Posted {
                    remote_id: id.to_string(),
                    url: permalink,
                })
            }
            _ => Err("posting there isn't connected".into()),
        }
    }

    /// Deletes a post where the platform allows it (Facebook): `Ok(true)` deleted, `Ok(false)` it
    /// must be taken down by hand (Instagram).
    pub async fn delete(&self, platform: &str, remote_id: &str) -> Result<bool, String> {
        match platform {
            "facebook" if remote_id.bytes().all(|b| b.is_ascii_digit() || b == b'_') => {
                self.call(reqwest::Method::DELETE, remote_id, &[]).await.map(|_| true)
            }
            _ => Ok(false),
        }
    }
}

#[doc(hidden)]
/// A stand-in Graph API for tests: records each call (method, path, form) and answers as Meta's
/// API does, with made-up ids.
pub mod stand_in {
    use std::sync::{Arc, Mutex};

    use axum::Router;
    use axum::extract::{Path, State};
    use axum::http::Method;
    use axum::routing::any;

    pub type Calls = Arc<Mutex<Vec<(String, String, String)>>>;

    pub async fn start() -> std::io::Result<(String, Calls)> {
        let calls: Calls = Arc::default();
        async fn answer(State(calls): State<Calls>, method: Method, Path(path): Path<String>, body: String) -> String {
            let mut log = calls.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            log.push((method.to_string(), path.clone(), body));
            let n = log.len();
            if path.ends_with("/photos") {
                format!(r#"{{"id": "{n}", "post_id": "111_{n}"}}"#)
            } else if path.ends_with("/media") {
                format!(r#"{{"id": "c{n}"}}"#)
            } else if path.ends_with("/media_publish") {
                format!(r#"{{"id": "99{n}"}}"#)
            } else if method == Method::DELETE {
                r#"{"success": true}"#.to_string()
            } else {
                r#"{"permalink": "https://www.instagram.com/p/test/"}"#.to_string()
            }
        }
        let app = Router::new().route("/{*path}", any(answer)).with_state(calls.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}", listener.local_addr()?);
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Ok((base, calls))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn posts_and_deletes_through_the_graph_api() {
        let (base, calls) = stand_in::start().await.unwrap();
        let s = Social::new("123", "page-token", "456", Some(&base)).unwrap();
        assert_eq!(s.platforms(), ["facebook", "instagram"]);
        let fb = s
            .post("facebook", "https://makerfoundry.test/a.png", "Dragon by Bea")
            .await
            .unwrap();
        assert!(fb.url.starts_with("https://www.facebook.com/111_"));
        let ig = s
            .post("instagram", "https://makerfoundry.test/a.png", "Dragon by Bea")
            .await
            .unwrap();
        assert_eq!(ig.url, "https://www.instagram.com/p/test/");
        assert!(s.delete("facebook", &fb.remote_id).await.unwrap());
        assert!(
            !s.delete("instagram", &ig.remote_id).await.unwrap(),
            "Instagram: by hand"
        );
        let log = calls.lock().unwrap();
        assert_eq!(log[0].1, "v21.0/123/photos");
        assert!(log[0].2.contains("caption=Dragon+by+Bea") && log[0].2.contains("access_token=page-token"));
        assert_eq!(log[1].1, "v21.0/456/media");
        assert_eq!(log.last().unwrap().0, "DELETE");
        assert!(Social::new("", "t", "", None).is_none() && Social::new("12a", "t", "", None).is_none());
        assert_eq!(Social::new("1", "t", "", None).unwrap().platforms(), ["facebook"]);
        assert_eq!(format!("{:?}", s), "Social");
    }
}
