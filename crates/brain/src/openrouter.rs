//! Minimal OpenRouter client for Phase 0: key provisioning (management key)
//! and per-key usage (`GET /api/v1/key`). Endpoints and fields verified on
//! 2026-09-19 against the official docs.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

pub const DEFAULT_BASE_URL: &str = "https://openrouter.ai/api/v1";

#[derive(Clone)]
pub struct Client {
    base_url: String,
    http: reqwest::Client,
}

#[derive(Debug, Serialize)]
pub struct CreateKey<'a> {
    pub name: &'a str,
    pub limit: f64,
    pub limit_reset: &'a str,
    pub include_byok_in_limit: bool,
}

/// `data` object of the keys endpoints and of `GET /api/v1/key`.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct KeyData {
    #[serde(default)]
    pub hash: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub disabled: bool,
    pub limit: Option<f64>,
    pub limit_remaining: Option<f64>,
    pub limit_reset: Option<String>,
    #[serde(default)]
    pub usage: f64,
    #[serde(default)]
    pub usage_daily: f64,
    #[serde(default)]
    pub usage_weekly: f64,
    #[serde(default)]
    pub usage_monthly: f64,
}

#[derive(Debug, Deserialize)]
struct Created {
    key: String,
    data: KeyData,
}

#[derive(Debug, Deserialize)]
struct DataList {
    data: Vec<KeyData>,
}

#[derive(Debug, Deserialize)]
struct DataOne {
    data: KeyData,
}

pub struct CreatedKey {
    /// The secret. Shown once by OpenRouter; we print it once and never store it.
    pub key: String,
    pub data: KeyData,
}

impl Client {
    pub fn new(base_url: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(concat!("llm_brain/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("reqwest client");
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            http,
        }
    }

    /// `POST /keys` with the management key.
    pub async fn create_key(
        &self,
        management_key: &str,
        req: &CreateKey<'_>,
    ) -> Result<CreatedKey> {
        let resp = self
            .http
            .post(format!("{}/keys", self.base_url))
            .bearer_auth(management_key)
            .json(req)
            .send()
            .await
            .context("POST /keys")?;
        let resp = check(resp).await?;
        let created: Created = resp.json().await.context("decoding created key")?;
        Ok(CreatedKey {
            key: created.key,
            data: created.data,
        })
    }

    /// `GET /keys` with the management key.
    pub async fn list_keys(&self, management_key: &str) -> Result<Vec<KeyData>> {
        let resp = self
            .http
            .get(format!("{}/keys", self.base_url))
            .bearer_auth(management_key)
            .send()
            .await
            .context("GET /keys")?;
        let resp = check(resp).await?;
        Ok(resp
            .json::<DataList>()
            .await
            .context("decoding key list")?
            .data)
    }

    /// `GET /key`: usage and limits of the key used for auth.
    pub async fn key_info(&self, api_key: &str) -> Result<KeyData> {
        let resp = self
            .http
            .get(format!("{}/key", self.base_url))
            .bearer_auth(api_key)
            .send()
            .await
            .context("GET /key")?;
        let resp = check(resp).await?;
        Ok(resp
            .json::<DataOne>()
            .await
            .context("decoding key info")?
            .data)
    }
}

async fn check(resp: reqwest::Response) -> Result<reqwest::Response> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let body = resp.text().await.unwrap_or_default();
    bail!(
        "OpenRouter returned {status}: {}",
        body.chars().take(300).collect::<String>()
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{bearer_token, body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn key_json(name: &str, usage_daily: f64) -> serde_json::Value {
        serde_json::json!({
            "hash": "h1", "name": name, "label": "sk-or-v1-abc...123", "disabled": false,
            "limit": 3.0, "limit_remaining": 3.0 - usage_daily, "limit_reset": "daily",
            "usage": 12.5, "usage_daily": usage_daily, "usage_weekly": 4.0, "usage_monthly": 12.5
        })
    }

    #[tokio::test]
    async fn create_key_sends_limits_and_returns_secret_once() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/keys"))
            .and(bearer_token("mgmt"))
            .and(body_json(serde_json::json!({
                "name": "llm_brain/dev", "limit": 3.0, "limit_reset": "daily", "include_byok_in_limit": false
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "key": "sk-or-v1-secret", "data": key_json("llm_brain/dev", 0.0)
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = Client::new(server.uri());
        let req = CreateKey {
            name: "llm_brain/dev",
            limit: 3.0,
            limit_reset: "daily",
            include_byok_in_limit: false,
        };
        let created = client.create_key("mgmt", &req).await.unwrap();
        assert_eq!(created.key, "sk-or-v1-secret");
        assert_eq!(created.data.limit_reset.as_deref(), Some("daily"));
    }

    #[tokio::test]
    async fn key_info_reads_usage_fields() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/key"))
            .and(bearer_token("sk-or-v1-dev"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"data": key_json("dev", 0.8)})),
            )
            .mount(&server)
            .await;
        let info = Client::new(server.uri())
            .key_info("sk-or-v1-dev")
            .await
            .unwrap();
        assert_eq!(info.usage_daily, 0.8);
        assert_eq!(info.limit_remaining, Some(2.2));
        assert_eq!(info.usage_monthly, 12.5);
    }

    #[tokio::test]
    async fn non_2xx_is_an_error_with_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/key"))
            .respond_with(
                ResponseTemplate::new(401).set_body_string(r#"{"error":{"message":"bad key"}}"#),
            )
            .mount(&server)
            .await;
        let err = Client::new(server.uri())
            .key_info("nope")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("401"), "{err}");
    }
}
