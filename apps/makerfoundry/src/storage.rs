//! Where MakerFoundry keeps files (MF-2): designs, photos and their thumbnails, in Cloudflare R2
//! through its S3 API (https://developers.cloudflare.com/r2/api/s3/api/). Requests are signed
//! (AWS Signature Version 4, by rusty-s3) as URLs that last a few minutes.
//!
//! Keys are made here from random bytes, never from what an uploader named a file, so a name
//! can't reach another file. Settings: `R2_ACCOUNT_ID`, `R2_ACCESS_KEY_ID`, `R2_SECRET_ACCESS_KEY`
//! and `R2_BUCKET` (or `STORAGE_ENDPOINT` in place of the account, for a local or test server).
//! Without them, files aren't kept and the pages say so.

use std::time::Duration;

use rusty_s3::actions::S3Action;
use rusty_s3::{Bucket, Credentials, UrlStyle};
use url::Url;

/// How long a signed request lasts.
const SIGNED_FOR: Duration = Duration::from_secs(5 * 60);
/// The largest file read back through the server (photos and thumbnails; designs are
/// downloaded straight from storage).
const MAX_GET_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone)]
pub struct Storage {
    bucket: Bucket,
    credentials: Credentials,
    http: reqwest::Client,
}

impl std::fmt::Debug for Storage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // (never the keys)
        f.debug_struct("Storage").field("bucket", &self.bucket.name()).finish()
    }
}

/// A new key for a file of the given kind (`design`, `photo`, `thumb`): the kind and 32 random
/// hex digits.
pub fn new_key(kind: &str) -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| format!("no randomness: {e}"))?;
    Ok(format!(
        "{kind}/{}",
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
    ))
}

impl Storage {
    /// The storage the environment describes (`env` gives a setting's value), if it describes one.
    pub fn from_env(env: impl Fn(&str) -> Option<String>) -> Result<Option<Storage>, String> {
        let get = |name: &str| env(name).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        let (Some(key), Some(secret), Some(bucket)) =
            (get("R2_ACCESS_KEY_ID"), get("R2_SECRET_ACCESS_KEY"), get("R2_BUCKET"))
        else {
            return Ok(None);
        };
        let endpoint = match (get("STORAGE_ENDPOINT"), get("R2_ACCOUNT_ID")) {
            (Some(e), _) => e,
            (None, Some(account)) => format!("https://{account}.r2.cloudflarestorage.com"),
            (None, None) => return Err("R2_ACCOUNT_ID (or STORAGE_ENDPOINT) is missing".into()),
        };
        Storage::new(&endpoint, &bucket, &key, &secret).map(Some)
    }

    pub fn new(endpoint: &str, bucket: &str, key: &str, secret: &str) -> Result<Storage, String> {
        let endpoint: Url = endpoint
            .parse()
            .map_err(|_| "the storage address isn't a URL".to_string())?;
        // R2 takes "auto" for the region
        let bucket =
            Bucket::new(endpoint, UrlStyle::Path, bucket.to_string(), "auto").map_err(|e| format!("storage: {e}"))?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .no_proxy()
            .build()
            .map_err(|e| format!("can't make an HTTP client: {e}"))?;
        Ok(Storage {
            bucket,
            credentials: Credentials::new(key, secret),
            http,
        })
    }

    /// Keeps `bytes` under `key`.
    pub async fn put(&self, key: &str, bytes: Vec<u8>, content_type: &str) -> Result<(), String> {
        let url = self.bucket.put_object(Some(&self.credentials), key).sign(SIGNED_FOR);
        let resp = self
            .http
            .put(url)
            .header(reqwest::header::CONTENT_TYPE, content_type)
            .body(bytes)
            .send()
            .await
            .map_err(|_| "file storage can't be reached".to_string())?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(format!("file storage refused the file ({})", resp.status().as_u16()))
        }
    }

    /// The bytes kept under `key`, if there are any.
    pub async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
        let url = self.bucket.get_object(Some(&self.credentials), key).sign(SIGNED_FOR);
        let resp = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|_| "file storage can't be reached".to_string())?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(format!("file storage refused ({})", resp.status().as_u16()));
        }
        if resp.content_length().is_some_and(|n| n > MAX_GET_BYTES as u64) {
            return Err("the stored file is too large to read here".into());
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|_| "the stored file couldn't be read".to_string())?;
        if bytes.len() > MAX_GET_BYTES {
            return Err("the stored file is too large to read here".into());
        }
        Ok(Some(bytes.to_vec()))
    }

    /// Removes the file under `key` (removing one that isn't there is fine).
    pub async fn delete(&self, key: &str) -> Result<(), String> {
        let url = self.bucket.delete_object(Some(&self.credentials), key).sign(SIGNED_FOR);
        let resp = self
            .http
            .delete(url)
            .send()
            .await
            .map_err(|_| "file storage can't be reached".to_string())?;
        if resp.status().is_success() || resp.status() == reqwest::StatusCode::NOT_FOUND {
            Ok(())
        } else {
            Err(format!(
                "file storage refused to remove a file ({})",
                resp.status().as_u16()
            ))
        }
    }

    /// A link that downloads the file under `key` as `file_name`, for a few minutes.
    pub fn download_url(&self, key: &str, file_name: &str) -> Url {
        let mut action = self.bucket.get_object(Some(&self.credentials), key);
        // only safe characters in the name the browser saves it as
        let safe: String = file_name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || "._- ".contains(c) {
                    c
                } else {
                    '_'
                }
            })
            .take(100)
            .collect();
        action.query_mut().insert(
            "response-content-disposition",
            format!("attachment; filename=\"{safe}\""),
        );
        action.sign(SIGNED_FOR)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_random_and_carry_only_their_kind() {
        let a = new_key("photo").unwrap();
        let b = new_key("photo").unwrap();
        assert!(a.starts_with("photo/") && a.len() == "photo/".len() + 32);
        assert_ne!(a, b);
    }

    #[test]
    fn settings_come_from_the_environment() {
        let none = Storage::from_env(|_| None).unwrap();
        assert!(none.is_none(), "no settings, no storage");
        let env = |name: &str| match name {
            "R2_ACCOUNT_ID" => Some("abc123".to_string()),
            "R2_ACCESS_KEY_ID" => Some("key".to_string()),
            "R2_SECRET_ACCESS_KEY" => Some("secret".to_string()),
            "R2_BUCKET" => Some("makerfoundry-files".to_string()),
            _ => None,
        };
        let s = Storage::from_env(env).unwrap().unwrap();
        let url = s.download_url("design/0011", "rose \"<x>\".pes");
        assert_eq!(url.host_str(), Some("abc123.r2.cloudflarestorage.com"));
        assert!(url.path().ends_with("/makerfoundry-files/design/0011"), "{url}");
        let query = url.query().unwrap_or_default();
        assert!(
            query.contains("X-Amz-Signature=") && query.contains("rose%20_"),
            "{query}"
        );
        assert!(!format!("{s:?}").contains("secret"));
        let missing = |name: &str| if name == "R2_ACCOUNT_ID" { None } else { env(name) };
        assert!(Storage::from_env(missing).is_err());
    }
}
