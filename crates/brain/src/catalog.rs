//! OpenRouter's model catalog (`GET /api/v1/models`): context length, the
//! provider's max completion tokens, prices, supported parameters. Fetched
//! once an hour and used as the source of facts for capping `max_tokens`,
//! estimating cost and describing tiers. Config stays the source of policy.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ModelInfo {
    pub id: String,
    pub context_length: Option<u64>,
    pub max_completion_tokens: Option<u64>,
    pub prompt_usd_per_m: f64,
    pub completion_usd_per_m: f64,
    pub supports_tools: bool,
    pub supports_reasoning: bool,
}

#[derive(Deserialize)]
struct Raw {
    id: String,
    context_length: Option<u64>,
    #[serde(default)]
    top_provider: Option<RawTop>,
    #[serde(default)]
    pricing: Option<RawPricing>,
    #[serde(default)]
    supported_parameters: Option<Vec<String>>,
}
#[derive(Deserialize, Default)]
struct RawTop {
    max_completion_tokens: Option<u64>,
    context_length: Option<u64>,
}
#[derive(Deserialize, Default)]
struct RawPricing {
    prompt: Option<String>,
    completion: Option<String>,
}
#[derive(Deserialize)]
struct RawList {
    data: Vec<Raw>,
}

pub const TTL: Duration = Duration::from_secs(3600);

#[derive(Debug, Clone)]
pub struct Catalog {
    pub fetched_at: Instant,
    pub models: HashMap<String, ModelInfo>,
}

impl Catalog {
    pub fn parse(json: &str) -> Result<Self> {
        let list: RawList = serde_json::from_str(json).context("decoding /models")?;
        let per_m = |s: &Option<String>| {
            s.as_deref()
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(0.0)
                * 1e6
        };
        let models = list
            .data
            .into_iter()
            .map(|r| {
                let sp = r.supported_parameters.unwrap_or_default();
                let top = r.top_provider.unwrap_or_default();
                let pricing = r.pricing.unwrap_or_default();
                let info = ModelInfo {
                    id: r.id.clone(),
                    context_length: r.context_length.or(top.context_length),
                    max_completion_tokens: top.max_completion_tokens,
                    prompt_usd_per_m: per_m(&pricing.prompt),
                    completion_usd_per_m: per_m(&pricing.completion),
                    supports_tools: sp.iter().any(|p| p == "tools"),
                    supports_reasoning: sp.iter().any(|p| p == "reasoning"),
                };
                (r.id, info)
            })
            .collect();
        Ok(Catalog {
            fetched_at: Instant::now(),
            models,
        })
    }

    pub async fn fetch(http: &reqwest::Client, base: &str) -> Result<Self> {
        let text = http
            .get(format!("{}/models", base.trim_end_matches('/')))
            .send()
            .await
            .context("GET /models")?
            .error_for_status()?
            .text()
            .await?;
        Self::parse(&text)
    }

    pub fn is_stale(&self) -> bool {
        self.fetched_at.elapsed() > TTL
    }

    /// Facts for `id`. A routing variant (`model:exacto`, `:nitro`, `:floor`)
    /// is not a catalog entry — it selects providers of the base model — so
    /// it falls back to the base id. `:free` and `:batch` are real entries
    /// with their own prices and are matched exactly first.
    pub fn get(&self, id: &str) -> Option<&ModelInfo> {
        self.models.get(id).or_else(|| self.models.get(base_id(id)))
    }
}

/// `deepseek/deepseek-v4-pro:exacto` → `deepseek/deepseek-v4-pro`.
pub fn base_id(id: &str) -> &str {
    match id.rsplit_once(':') {
        Some((base, variant)) if base.contains('/') && !variant.is_empty() => base,
        _ => id,
    }
}

/// The `max_tokens` a request may use for `model`: the smaller of the
/// provider's maximum (catalog) and the tier policy (config), if either is known.
pub fn output_cap(info: Option<&ModelInfo>, policy: Option<u64>) -> Option<u64> {
    match (info.and_then(|i| i.max_completion_tokens), policy) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const JSON: &str = r#"{"data":[
      {"id":"deepseek/deepseek-v4-flash","context_length":1048576,"pricing":{"prompt":"0.000000036","completion":"0.000000073"},"top_provider":{"context_length":1048576,"max_completion_tokens":131072},"supported_parameters":["tools","reasoning","max_tokens"]},
      {"id":"prism-ml/ternary-bonsai-2-27b","context_length":262144,"pricing":{"prompt":"0.000000075","completion":"0.0000005"},"top_provider":{"max_completion_tokens":32768},"supported_parameters":["tools","reasoning"]},
      {"id":"deepseek/deepseek-v4-flash:free","context_length":65536,"pricing":{"prompt":"0","completion":"0"},"top_provider":{"max_completion_tokens":8192},"supported_parameters":["tools"]},
      {"id":"some/odd-model","pricing":{"prompt":"0","completion":"0"}}
    ]}"#;

    #[test]
    fn parses_the_openrouter_shape() {
        let c = Catalog::parse(JSON).unwrap();
        let d = c.get("deepseek/deepseek-v4-flash").unwrap();
        assert_eq!(
            (d.context_length, d.max_completion_tokens),
            (Some(1048576), Some(131072))
        );
        assert!(
            (d.prompt_usd_per_m - 0.036).abs() < 1e-9
                && (d.completion_usd_per_m - 0.073).abs() < 1e-9
        );
        assert!(d.supports_tools && d.supports_reasoning);
        let odd = c.get("some/odd-model").unwrap();
        assert_eq!(
            (
                odd.context_length,
                odd.max_completion_tokens,
                odd.supports_tools
            ),
            (None, None, false)
        );
        assert!(!c.is_stale());
    }

    #[test]
    fn routing_variants_fall_back_to_the_base_model() {
        let c = Catalog::parse(JSON).unwrap();
        assert_eq!(
            c.get("deepseek/deepseek-v4-flash:exacto")
                .map(|m| m.id.as_str()),
            Some("deepseek/deepseek-v4-flash")
        );
        assert_eq!(
            c.get("deepseek/deepseek-v4-flash:free")
                .map(|m| m.id.as_str()),
            Some("deepseek/deepseek-v4-flash:free"),
            "a real variant entry wins over the base"
        );
        assert!(c.get("nobody/here:exacto").is_none());
        assert_eq!(base_id("a/b:exacto"), "a/b");
        assert_eq!(base_id("a/b"), "a/b");
        assert_eq!(base_id("a/b:"), "a/b:");
        assert_eq!(base_id("noslash:exacto"), "noslash:exacto");
    }

    #[test]
    fn output_cap_takes_the_smaller_known_bound() {
        let c = Catalog::parse(JSON).unwrap();
        assert_eq!(
            output_cap(c.get("deepseek/deepseek-v4-flash"), Some(16384)),
            Some(16384)
        );
        assert_eq!(
            output_cap(c.get("prism-ml/ternary-bonsai-2-27b"), Some(65536)),
            Some(32768)
        );
        assert_eq!(output_cap(c.get("some/odd-model"), None), None);
        assert_eq!(output_cap(None, Some(4096)), Some(4096));
    }

    #[tokio::test]
    async fn fetches_from_the_api() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(ResponseTemplate::new(200).set_body_string(JSON))
            .mount(&server)
            .await;
        let c = Catalog::fetch(&reqwest::Client::new(), &server.uri())
            .await
            .unwrap();
        assert_eq!(c.models.len(), 4);
    }
}
