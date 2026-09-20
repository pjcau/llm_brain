//! `brain usage snapshot|report`: read `GET /key` for every profile whose key
//! is in the environment and store it; report the day's spend per profile
//! against its limit.

use crate::config::Config;
use crate::db::{DailyUsage, Db, Snapshot};
use crate::openrouter::Client;
use anyhow::Result;
use chrono::Utc;
use std::collections::HashMap;

/// Takes one snapshot per profile that has a key in `env` and stores them.
/// Profiles without a key are reported in the returned `missing` list, not
/// treated as errors.
pub async fn snapshot(
    cfg: &Config,
    env: &HashMap<String, String>,
    client: &Client,
    db: &Db,
) -> Result<(Vec<Snapshot>, Vec<String>)> {
    let (taken, missing) = fetch(cfg, env, client).await?;
    for s in &taken {
        db.insert_snapshot(s)?;
    }
    Ok((taken, missing))
}

/// Network half of [`snapshot`]: no database borrow across awaits, so it can
/// run inside a spawned task.
pub async fn fetch(
    cfg: &Config,
    env: &HashMap<String, String>,
    client: &Client,
) -> Result<(Vec<Snapshot>, Vec<String>)> {
    let mut taken = Vec::new();
    let mut missing = Vec::new();
    for p in &cfg.profiles {
        let Some(key) = env.get(&p.key_env()).filter(|k| !k.is_empty()) else {
            missing.push(p.name.clone());
            continue;
        };
        let info = client.key_info(key).await?;
        let s = Snapshot {
            ts: Utc::now(),
            profile: p.name.clone(),
            usage_total: info.usage,
            usage_daily: info.usage_daily,
            usage_weekly: info.usage_weekly,
            usage_monthly: info.usage_monthly,
            limit: info.limit,
            limit_remaining: info.limit_remaining,
        };
        taken.push(s);
    }
    Ok((taken, missing))
}

/// Text table: day · profile · spent · limit · % · monthly soft cap status.
pub fn render_report(cfg: &Config, rows: &[DailyUsage]) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{:<10} {:<10} {:>8} {:>8} {:>5}  {}\n",
        "day", "profile", "spent$", "limit$", "%", "state"
    ));
    for r in rows {
        let limit = r.limit.unwrap_or(0.0);
        let pct = if limit > 0.0 {
            r.usage_daily / limit * 100.0
        } else {
            0.0
        };
        let state = match pct {
            p if p >= 100.0 => "EXHAUSTED",
            p if p >= 85.0 => "degrade: max_tokens",
            p if p >= 70.0 => "degrade: fast only",
            _ => "ok",
        };
        let _ = cfg.profile(&r.profile); // profiles removed from config still show up
        out.push_str(&format!(
            "{:<10} {:<10} {:>8.3} {:>8.2} {:>4.0}%  {}\n",
            r.day, r.profile, r.usage_daily, limit, pct, state
        ));
    }
    if rows.is_empty() {
        out.push_str("(no snapshots yet — run `brain usage snapshot`)\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{bearer_token, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn cfg() -> Config {
        Config::from_yaml(
            "profiles:\n  - {name: dev, tier: fast, daily_limit_usd: 3.0, monthly_soft_usd: 30.0}\n  - {name: car, tier: fast, daily_limit_usd: 0.2, monthly_soft_usd: 3.0}\n",
            "tiers:\n  fast: {model: m}\n",
        )
        .unwrap()
    }

    #[tokio::test]
    async fn snapshots_only_profiles_with_a_key() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/key"))
            .and(bearer_token("sk-dev"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data": {
                "usage": 5.0, "usage_daily": 2.4, "usage_weekly": 5.0, "usage_monthly": 5.0, "limit": 3.0, "limit_remaining": 0.6
            }})))
            .expect(1)
            .mount(&server)
            .await;
        let env = HashMap::from([("OPENROUTER_KEY_DEV".to_string(), "sk-dev".to_string())]);
        let db = Db::memory().unwrap();
        let (taken, missing) = snapshot(&cfg(), &env, &Client::new(server.uri()), &db)
            .await
            .unwrap();
        assert_eq!(taken.len(), 1);
        assert_eq!(taken[0].profile, "dev");
        assert_eq!(missing, vec!["car".to_string()]);
        let report = render_report(&cfg(), &db.daily_usage(1).unwrap());
        assert!(report.contains("dev"));
        assert!(report.contains("80%"), "{report}");
        assert!(report.contains("degrade: fast only"), "{report}");
    }

    #[test]
    fn report_states_follow_the_degradation_thresholds() {
        let mk = |u: f64| DailyUsage {
            day: "2026-09-19".into(),
            profile: "dev".into(),
            usage_daily: u,
            limit: Some(1.0),
        };
        let r = render_report(&cfg(), &[mk(0.1), mk(0.7), mk(0.9), mk(1.0)]);
        assert!(r.contains(" ok"));
        assert!(r.contains("degrade: fast only"));
        assert!(r.contains("degrade: max_tokens"));
        assert!(r.contains("EXHAUSTED"));
        assert!(render_report(&cfg(), &[]).contains("no snapshots"));
    }
}
