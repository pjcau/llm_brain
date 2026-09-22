//! `brain/auto`: pick a tier per *session*, never per turn.
//!
//! The first user message of a conversation goes to a decision model (Jev,
//! through OpenRouter's `/alpha/decisions` endpoint: typed answers with
//! probabilities, no generated text, ~0.5 s, ~0.00002 $). Its choice is one
//! rung of the ladder in `tiers.yaml` and is remembered for the session, so
//! every later turn of the same agent loop lands on the same model: the
//! prompt cache stays warm and no "light-looking" turn ends up on a model
//! that breaks the tool call. Details: docs/architecture/auto-routing.md.

use crate::config::Router;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The alias clients send to opt in.
pub const ALIAS: &str = "brain/auto";

/// The decision model reads at most this much of the message.
const MAX_STATE_CHARS: usize = 6000;

/// Session → tier, with a sliding TTL.
pub struct SessionCache {
    map: Mutex<HashMap<String, (String, Instant)>>,
    ttl: Duration,
}

impl SessionCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            map: Mutex::new(HashMap::new()),
            ttl,
        }
    }

    /// The tier of a live session; touching it extends its life.
    pub fn get(&self, key: &str) -> Option<String> {
        let mut map = self.map.lock().unwrap();
        let (tier, seen) = map.get_mut(key)?;
        if seen.elapsed() > self.ttl {
            map.remove(key);
            return None;
        }
        *seen = Instant::now();
        Some(tier.clone())
    }

    pub fn put(&self, key: String, tier: String) {
        let mut map = self.map.lock().unwrap();
        if map.len() >= 4096 {
            let ttl = self.ttl;
            map.retain(|_, (_, seen)| seen.elapsed() <= ttl);
        }
        map.insert(key, (tier, Instant::now()));
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.map.lock().unwrap().len()
    }
}

/// Headers a client may use to name its conversation, in order of precedence:
/// ours for apps, Claude Code's own.
pub const SESSION_HEADERS: [&str; 2] = ["x-brain-session", "x-claude-code-session-id"];

/// What identifies a conversation: a session header when present, else a
/// hash of the first user message (OpenRouter's own fingerprint), scoped to
/// the profile. `None` when there is no user text at all (nothing to
/// classify either).
pub fn session_key(profile: &str, session_header: Option<&str>, body: &Value) -> Option<String> {
    if let Some(sid) = session_header.map(str::trim).filter(|s| !s.is_empty()) {
        return Some(format!("{profile}:h:{sid}"));
    }
    let text = first_user_text(body)?;
    let mut h = DefaultHasher::new();
    text.hash(&mut h);
    Some(format!("{profile}:m:{:016x}", h.finish()))
}

/// Text of the first `user` message, both dialects (string content or
/// `[{type: text, text}]` blocks). Tool results are skipped: the task is
/// what the human typed.
pub fn first_user_text(body: &Value) -> Option<String> {
    let msgs = body.get("messages")?.as_array()?;
    let m = msgs
        .iter()
        .find(|m| m.get("role").and_then(Value::as_str) == Some("user"))?;
    let text = match m.get("content")? {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => return None,
    };
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// Ask the decision model which rung the task belongs to. Returns the tier
/// and the confidence; the caller applies `min_confidence` and the fallback.
pub async fn classify(
    http: &reqwest::Client,
    decisions_url: &str,
    upstream_key: &str,
    router: &Router,
    task: &str,
) -> Result<(String, f64)> {
    let state: String = task.chars().take(MAX_STATE_CHARS).collect();
    let criteria: serde_json::Map<String, Value> = router
        .ladder
        .iter()
        .map(|r| (r.tier.clone(), json!(r.when)))
        .collect();
    let body = json!({
        "model": router.model,
        "state": format!("{}: {state}", router.context),
        "questions": {
            "tier": {
                "type": "choice",
                "instructions": "How much model capability does this task need? Pick the lightest rung that will get it done reliably.",
                "criteria": criteria,
            }
        }
    });
    let resp = http
        .post(decisions_url)
        .bearer_auth(upstream_key)
        .json(&body)
        .timeout(Duration::from_secs(8))
        .send()
        .await
        .context("decisions request")?;
    let status = resp.status();
    let v: Value = resp.json().await.context("decisions body")?;
    if !status.is_success() {
        bail!(
            "decisions {status}: {}",
            v.get("error").cloned().unwrap_or(v)
        );
    }
    let answer = v
        .pointer("/answers/tier")
        .context("decisions answer missing")?;
    let choice = answer
        .get("choice")
        .and_then(Value::as_str)
        .context("decisions choice missing")?;
    if !router.ladder.iter().any(|r| r.tier == choice) {
        bail!("decisions chose `{choice}`, not on the ladder");
    }
    let confidence = answer
        .get("confidence")
        .and_then(Value::as_f64)
        .unwrap_or(1.0);
    Ok((choice.to_string(), confidence))
}

/// `https://openrouter.ai/api/v1` → `https://openrouter.ai/api/alpha/decisions`.
pub fn decisions_url(upstream_base: &str) -> String {
    let base = upstream_base.trim_end_matches('/');
    let base = base.strip_suffix("/v1").unwrap_or(base);
    format!("{base}/alpha/decisions")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Rung;
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn router() -> Router {
        Router {
            model: "typesafe/jev-1.13".into(),
            ladder: vec![
                Rung {
                    tier: "fast".into(),
                    when: "trivial".into(),
                },
                Rung {
                    tier: "agent".into(),
                    when: "hard".into(),
                },
            ],
            fallback: "agent".into(),
            min_confidence: 0.5,
            session_ttl_s: 60,
            baseline: None,
            context: "Task given to an autonomous coding agent".into(),
        }
    }

    #[test]
    fn session_key_prefers_the_header_then_hashes_the_first_user_message() {
        let body = json!({"messages": [
            {"role": "system", "content": "x"},
            {"role": "user", "content": [{"type": "text", "text": "fix the typo"}, {"type": "text", "text": "in README"}]},
            {"role": "user", "content": "later"}
        ]});
        assert_eq!(
            session_key("dev", Some(" abc "), &body).unwrap(),
            "dev:h:abc"
        );
        let k1 = session_key("dev", None, &body).unwrap();
        let k2 = session_key("dev", Some(""), &body).unwrap();
        assert_eq!(k1, k2);
        assert!(k1.starts_with("dev:m:"));
        assert_ne!(k1, session_key("car", None, &body).unwrap());
        assert_eq!(
            first_user_text(&body).unwrap(),
            "fix the typo\nin README",
            "text blocks joined, later turns ignored"
        );
        // no user text: nothing to key on
        let none = json!({"messages": [{"role": "user", "content": [{"type": "tool_result", "content": "ok"}]}]});
        assert!(session_key("dev", None, &none).is_none());
        assert!(session_key("dev", None, &json!({})).is_none());
    }

    #[test]
    fn session_cache_expires_and_refreshes_on_touch() {
        let c = SessionCache::new(Duration::from_millis(30));
        c.put("s1".into(), "fast".into());
        assert_eq!(c.get("s1").as_deref(), Some("fast"));
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(c.get("s1").as_deref(), Some("fast"), "touched: still alive");
        std::thread::sleep(Duration::from_millis(40));
        assert!(c.get("s1").is_none(), "expired");
        assert_eq!(c.len(), 0);
    }

    #[test]
    fn decisions_url_replaces_the_v1_segment() {
        assert_eq!(
            decisions_url("https://openrouter.ai/api/v1"),
            "https://openrouter.ai/api/alpha/decisions"
        );
        assert_eq!(
            decisions_url("http://127.0.0.1:9/"),
            "http://127.0.0.1:9/alpha/decisions"
        );
    }

    #[tokio::test]
    async fn classify_sends_the_ladder_as_criteria_and_reads_choice_and_confidence() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/alpha/decisions"))
            .and(header("authorization", "Bearer sk-up"))
            .and(body_partial_json(json!({
                "model": "typesafe/jev-1.13",
                "state": "Task given to an autonomous coding agent: fix a typo",
                "questions": {"tier": {"type": "choice", "criteria": {"fast": "trivial", "agent": "hard"}}}
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "model": "jev-1.13.0",
                "answers": {"tier": {"choice": "fast", "probabilities": {"fast": 0.9, "agent": 0.1}, "confidence": 0.85}},
                "usage": {"input_tokens": 40, "cost": 0.0000017}
            })))
            .expect(1)
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let url = decisions_url(&server.uri());
        let (tier, conf) = classify(&http, &url, "sk-up", &router(), "fix a typo")
            .await
            .unwrap();
        assert_eq!((tier.as_str(), conf), ("fast", 0.85));
    }

    #[tokio::test]
    async fn classify_rejects_errors_and_choices_off_the_ladder() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/alpha/decisions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "answers": {"tier": {"choice": "premium", "confidence": 1.0}}
            })))
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let url = decisions_url(&server.uri());
        let err = classify(&http, &url, "k", &router(), "x")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not on the ladder"), "{err}");

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(402).set_body_json(json!({"error": {"message": "credits"}})),
            )
            .mount(&server)
            .await;
        let url = decisions_url(&server.uri());
        let err = classify(&http, &url, "k", &router(), "x")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("402"), "{err}");
    }
}
