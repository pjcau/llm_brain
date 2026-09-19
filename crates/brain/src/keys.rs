//! `brain keys provision|list`: one OpenRouter key per profile, each with a
//! daily limit taken from `config/profiles.yaml`. The secret is printed once
//! as an `.env` line and never stored by us.

use crate::config::Config;
use crate::openrouter::{Client, CreateKey, KeyData};
use anyhow::Result;
use std::collections::HashMap;

/// Key name on the OpenRouter side, so they are recognizable in the dashboard.
pub fn key_name(profile: &str) -> String {
    format!("llm_brain/{profile}")
}

pub struct Provisioned {
    pub profile: String,
    pub env_line: String,
    pub limit: f64,
}

/// Creates a key for every profile that has no key in `env` (or all of them
/// with `force`). Returns `.env` lines to paste; skipped profiles are listed.
pub async fn provision(
    cfg: &Config,
    env: &HashMap<String, String>,
    client: &Client,
    management_key: &str,
    force: bool,
    only: Option<&[String]>,
) -> Result<(Vec<Provisioned>, Vec<String>)> {
    let mut created = Vec::new();
    let mut skipped = Vec::new();
    for p in &cfg.profiles {
        if let Some(only) = only
            && !only.iter().any(|o| o == &p.name)
        {
            continue;
        }
        let has_key = env.get(&p.key_env()).is_some_and(|k| !k.is_empty());
        if has_key && !force {
            skipped.push(p.name.clone());
            continue;
        }
        let name = key_name(&p.name);
        let req = CreateKey {
            name: &name,
            limit: p.daily_limit_usd,
            limit_reset: "daily",
            include_byok_in_limit: false,
        };
        let k = client.create_key(management_key, &req).await?;
        created.push(Provisioned {
            profile: p.name.clone(),
            env_line: format!("{}={}", p.key_env(), k.key),
            limit: k.data.limit.unwrap_or(p.daily_limit_usd),
        });
    }
    Ok((created, skipped))
}

pub fn render_list(keys: &[KeyData]) -> String {
    let mut out = format!(
        "{:<28} {:<10} {:>7} {:>7} {:>9} {}\n",
        "name", "reset", "limit$", "today$", "month$", "state"
    );
    for k in keys {
        out.push_str(&format!(
            "{:<28} {:<10} {:>7} {:>7.3} {:>9.3} {}\n",
            k.name,
            k.limit_reset.as_deref().unwrap_or("-"),
            k.limit
                .map(|l| format!("{l:.2}"))
                .unwrap_or_else(|| "-".into()),
            k.usage_daily,
            k.usage_monthly,
            if k.disabled { "disabled" } else { "active" }
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn cfg() -> Config {
        Config::from_yaml(
            "profiles:\n  - {name: dev, tier: fast, daily_limit_usd: 3.0, monthly_soft_usd: 30.0}\n  - {name: benchmark, tier: fast, daily_limit_usd: 0.5, monthly_soft_usd: 5.0}\n",
            "tiers:\n  fast: {model: m}\n",
        )
        .unwrap()
    }

    #[tokio::test]
    async fn provisions_missing_profiles_with_daily_limits() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/keys"))
            .and(body_partial_json(serde_json::json!({"name": "llm_brain/benchmark", "limit": 0.5, "limit_reset": "daily"})))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "key": "sk-or-v1-bench", "data": {"hash": "h", "name": "llm_brain/benchmark", "limit": 0.5, "limit_reset": "daily"}
            })))
            .expect(1)
            .mount(&server)
            .await;
        // dev already has a key → skipped, no POST for it
        let env = HashMap::from([("OPENROUTER_KEY_DEV".to_string(), "sk-existing".to_string())]);
        let (created, skipped) = provision(
            &cfg(),
            &env,
            &Client::new(server.uri()),
            "mgmt",
            false,
            None,
        )
        .await
        .unwrap();
        assert_eq!(skipped, vec!["dev".to_string()]);
        assert_eq!(created.len(), 1);
        assert_eq!(
            created[0].env_line,
            "OPENROUTER_KEY_BENCHMARK=sk-or-v1-bench"
        );
        assert_eq!(created[0].limit, 0.5);
    }

    #[tokio::test]
    async fn only_filter_restricts_profiles() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/keys"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "key": "sk-new", "data": {"name": "llm_brain/dev", "limit": 3.0}
            })))
            .expect(1)
            .mount(&server)
            .await;
        let only = vec!["dev".to_string()];
        let (created, skipped) = provision(
            &cfg(),
            &HashMap::new(),
            &Client::new(server.uri()),
            "mgmt",
            false,
            Some(&only),
        )
        .await
        .unwrap();
        assert_eq!(created.len(), 1);
        assert_eq!(created[0].profile, "dev");
        assert!(skipped.is_empty());
    }

    #[test]
    fn list_renders_usage_and_state() {
        let k = KeyData {
            hash: "h".into(),
            name: "llm_brain/dev".into(),
            label: "sk-or-v1-abc...".into(),
            disabled: false,
            limit: Some(3.0),
            limit_remaining: Some(2.0),
            limit_reset: Some("daily".into()),
            usage: 9.0,
            usage_daily: 1.0,
            usage_weekly: 4.0,
            usage_monthly: 9.0,
        };
        let out = render_list(&[k]);
        assert!(out.contains("llm_brain/dev"));
        assert!(out.contains("daily"));
        assert!(out.contains("active"));
    }
}
