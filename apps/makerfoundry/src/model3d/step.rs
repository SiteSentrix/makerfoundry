//! STEP previews (issue #32): there's no STEP reader in Rust we could use, so a STEP file is sent
//! to a converter service of the owner's, a separate program (one built on OpenCASCADE, which is
//! LGPL, runs there as its own program and is never linked into this one), which answers a
//! binary STL of it. The STL is untrusted like any upload: read by our own STL reader with its
//! limits (`stl::read`), then drawn like any mesh. The STEP file itself is still kept and offered
//! as it came, as a download only.
//!
//! The service: `POST <MF_STEP_CONVERTER>/convert`, the STEP file as the body
//! (`application/step`), `Authorization: Bearer <MF_STEP_CONVERTER_KEY>` when a key is set →
//! `200` with a binary STL (`model/stl`), anything else meaning no preview. Off until
//! `MF_STEP_CONVERTER` is set; a STEP file then simply has no preview, as before. A preview that
//! can't be made never fails an upload.

use std::time::Duration;

use super::mesh::MAX_FILE_BYTES;

/// The converter service.
#[derive(Clone)]
pub struct Converter {
    url: String,
    key: Option<String>,
    http: reqwest::Client,
}

impl std::fmt::Debug for Converter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // (never the key)
        f.write_str("Converter")
    }
}

impl Converter {
    pub fn new(url: &str, key: Option<&str>) -> Option<Self> {
        let url = url.trim().trim_end_matches('/');
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            return None;
        }
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .ok()?;
        Some(Converter {
            url: url.to_string(),
            key: key.map(str::trim).filter(|k| !k.is_empty()).map(str::to_string),
            http,
        })
    }

    /// A STEP file as a binary STL, at most `MAX_FILE_BYTES`.
    pub async fn to_stl(&self, step: &[u8]) -> Result<Vec<u8>, String> {
        let mut request = self
            .http
            .post(format!("{}/convert", self.url))
            .header(reqwest::header::CONTENT_TYPE, "application/step")
            .header(reqwest::header::ACCEPT, "model/stl")
            .body(step.to_vec());
        if let Some(key) = &self.key {
            request = request.bearer_auth(key);
        }
        let mut answer = request
            .send()
            .await
            .map_err(|e| format!("converter: {}", e.without_url()))?;
        if !answer.status().is_success() {
            return Err(format!("converter: {}", answer.status()));
        }
        if answer.content_length().is_some_and(|n| n > MAX_FILE_BYTES as u64) {
            return Err("converter: the STL is too large".into());
        }
        // read in pieces, so an answer without a length can't grow past the limit
        let mut stl = Vec::new();
        while let Some(chunk) = answer
            .chunk()
            .await
            .map_err(|e| format!("converter: {}", e.without_url()))?
        {
            if stl.len() + chunk.len() > MAX_FILE_BYTES {
                return Err("converter: the STL is too large".into());
            }
            stl.extend_from_slice(&chunk);
        }
        Ok(stl)
    }
}

#[doc(hidden)]
/// A stand-in converter for tests: answers a 10 mm cube's binary STL for anything that looks like
/// STEP and carries the key `test-key`; `ISO-10303-21; BAD` gets an error, and `ISO-10303-21; HUGE`
/// an answer too large to read as an STL.
pub mod stand_in {
    use std::sync::{Arc, Mutex};

    use axum::Router;
    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::post;

    pub type Calls = Arc<Mutex<usize>>;

    /// A binary STL of an `s` mm cube.
    pub fn cube(s: f32) -> Vec<u8> {
        let v = |x: f32, y: f32, z: f32| [x * s, y * s, z * s];
        let faces: [[[f32; 3]; 3]; 12] = [
            [v(0., 0., 0.), v(1., 1., 0.), v(1., 0., 0.)],
            [v(0., 0., 0.), v(0., 1., 0.), v(1., 1., 0.)],
            [v(0., 0., 1.), v(1., 0., 1.), v(1., 1., 1.)],
            [v(0., 0., 1.), v(1., 1., 1.), v(0., 1., 1.)],
            [v(0., 0., 0.), v(1., 0., 0.), v(1., 0., 1.)],
            [v(0., 0., 0.), v(1., 0., 1.), v(0., 0., 1.)],
            [v(0., 1., 0.), v(1., 1., 1.), v(1., 1., 0.)],
            [v(0., 1., 0.), v(0., 1., 1.), v(1., 1., 1.)],
            [v(0., 0., 0.), v(0., 0., 1.), v(0., 1., 1.)],
            [v(0., 0., 0.), v(0., 1., 1.), v(0., 1., 0.)],
            [v(1., 0., 0.), v(1., 1., 0.), v(1., 1., 1.)],
            [v(1., 0., 0.), v(1., 1., 1.), v(1., 0., 1.)],
        ];
        let mut out = vec![0u8; 80];
        out.extend_from_slice(&12u32.to_le_bytes());
        for f in faces {
            out.extend_from_slice(&[0u8; 12]);
            for p in f {
                for c in p {
                    out.extend_from_slice(&c.to_le_bytes());
                }
            }
            out.extend_from_slice(&[0, 0]);
        }
        out
    }

    pub async fn start() -> std::io::Result<(String, Calls)> {
        let calls: Calls = Arc::default();
        async fn convert(
            State(calls): State<Calls>,
            headers: HeaderMap,
            body: axum::body::Bytes,
        ) -> (StatusCode, Vec<u8>) {
            *calls.lock().unwrap_or_else(std::sync::PoisonError::into_inner) += 1;
            let keyed = headers
                .get("authorization")
                .is_some_and(|v| v.as_bytes() == b"Bearer test-key");
            if !keyed {
                return (StatusCode::UNAUTHORIZED, Vec::new());
            }
            if !body.starts_with(b"ISO-10303-21;") || body.windows(3).any(|w| w == b"BAD") {
                return (StatusCode::UNPROCESSABLE_ENTITY, Vec::new());
            }
            if body.windows(4).any(|w| w == b"HUGE") {
                // a header claiming far more triangles than are there
                let mut stl = vec![0u8; 80];
                stl.extend_from_slice(&u32::MAX.to_le_bytes());
                return (StatusCode::OK, stl);
            }
            (StatusCode::OK, cube(10.0))
        }
        let app = Router::new()
            .route("/convert", post(convert))
            .layer(axum::extract::DefaultBodyLimit::max(64 * 1024 * 1024))
            .with_state(calls.clone());
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
    async fn the_converters_stl_is_read_by_our_own_reader() {
        let (base, calls) = stand_in::start().await.unwrap();
        let c = Converter::new(&base, Some("test-key")).unwrap();
        let stl = c.to_stl(b"ISO-10303-21; a part").await.unwrap();
        let mesh = super::super::stl::read(&stl).unwrap();
        assert_eq!(mesh.total, 12);
        assert!(c.to_stl(b"ISO-10303-21; BAD").await.is_err());
        let huge = c.to_stl(b"ISO-10303-21; HUGE").await.unwrap();
        assert!(super::super::stl::read(&huge).is_err(), "a lying STL is refused");
        let unkeyed = Converter::new(&base, None).unwrap();
        assert!(unkeyed.to_stl(b"ISO-10303-21; a part").await.is_err());
        assert_eq!(*calls.lock().unwrap(), 4);
        assert!(Converter::new("", None).is_none() && Converter::new("ftp://x", None).is_none());
        assert_eq!(format!("{c:?}"), "Converter");
    }
}
