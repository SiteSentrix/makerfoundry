//! StitchFoundry's account service (`apps/stitchfoundry-cloud` in the StitchFoundry repository,
//! `docs/cloud.md` there): MakerFoundry's accounts are Foundry accounts, kept there (owner's decision,
//! 2026-10-19), so signing up, signing in and resetting a password go through its API. Its
//! session tokens can spend the account's AI credits: they stay on the server, never in a page.

use std::time::Duration;

use serde_json::{Value, json};

/// The account service MakerFoundry uses unless `STITCHFOUNDRY_CLOUD_URL` says otherwise.
pub const DEFAULT_URL: &str = "https://cloud.stitchfoundry.app";

/// An account as the service describes it.
#[derive(Clone, Debug, PartialEq)]
pub struct Account {
    /// The account's lasting number, which MakerFoundry keeps its records under.
    pub id: i64,
    pub email: String,
}

#[derive(Debug, PartialEq)]
pub enum CloudError {
    /// The service answered with an error: its status and its message for people.
    Refused { status: u16, message: String },
    /// The service couldn't be reached, or its answer couldn't be read.
    Unreachable,
}

impl CloudError {
    /// Whether the token used is no good any more (signed out, or a password reset).
    pub fn is_signed_out(&self) -> bool {
        matches!(self, CloudError::Refused { status: 401, .. })
    }
}

#[derive(Clone)]
pub struct Cloud {
    base: String,
    http: reqwest::Client,
}

impl Cloud {
    /// The service at `base` (`https://cloud.stitchfoundry.app`, or a test server).
    pub fn new(base: &str) -> Result<Cloud, String> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            // Straight to the service: MakerFoundry's host has no proxy, and a test's local server
            // must not go through one.
            .no_proxy()
            .build()
            .map_err(|e| format!("can't make an HTTP client: {e}"))?;
        Ok(Cloud {
            base: base.trim_end_matches('/').to_string(),
            http,
        })
    }

    /// A new account → its session token and the account.
    pub async fn signup(&self, email: &str, password: &str) -> Result<(String, Account), CloudError> {
        let v = self
            .post("/v1/signup", None, json!({ "email": email, "password": password }))
            .await?;
        signed_in(&v)
    }

    /// Signs in → a session token and the account.
    pub async fn signin(&self, email: &str, password: &str) -> Result<(String, Account), CloudError> {
        let v = self
            .post("/v1/signin", None, json!({ "email": email, "password": password }))
            .await?;
        signed_in(&v)
    }

    /// Ends the session `token` stands for.
    pub async fn signout(&self, token: &str) -> Result<(), CloudError> {
        self.post("/v1/signout", Some(token), json!({})).await.map(|_| ())
    }

    /// The account `token` is signed in to, while it is.
    pub async fn account(&self, token: &str) -> Result<Account, CloudError> {
        let v = self.post("/v1/account", Some(token), json!({})).await?;
        account(&v)
    }

    /// Asks for a password-reset email → the service's message for people (the same whether or
    /// not the email has an account).
    pub async fn forgot(&self, email: &str) -> Result<String, CloudError> {
        let v = self
            .post("/v1/password/forgot", None, json!({ "email": email }))
            .await?;
        Ok(v.get("message")
            .and_then(Value::as_str)
            .unwrap_or("A link to reset the password is on its way.")
            .to_string())
    }

    async fn post(&self, path: &str, token: Option<&str>, body: Value) -> Result<Value, CloudError> {
        let mut req = self.http.post(format!("{}{path}", self.base)).json(&body);
        if let Some(t) = token {
            req = req.bearer_auth(t);
        }
        let resp = req.send().await.map_err(|_| CloudError::Unreachable)?;
        let status = resp.status().as_u16();
        let v: Value = resp.json().await.map_err(|_| CloudError::Unreachable)?;
        if (200..300).contains(&status) {
            return Ok(v);
        }
        let message = v
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or("something went wrong")
            .to_string();
        Err(CloudError::Refused { status, message })
    }
}

fn account(v: &Value) -> Result<Account, CloudError> {
    let id = v.get("id").and_then(Value::as_i64).ok_or(CloudError::Unreachable)?;
    let email = v.get("email").and_then(Value::as_str).ok_or(CloudError::Unreachable)?;
    Ok(Account {
        id,
        email: email.to_string(),
    })
}

fn signed_in(v: &Value) -> Result<(String, Account), CloudError> {
    let token = v.get("token").and_then(Value::as_str).ok_or(CloudError::Unreachable)?;
    let account = account(v.get("account").ok_or(CloudError::Unreachable)?)?;
    Ok((token.to_string(), account))
}
