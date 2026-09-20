//! Request shaping before forwarding. Two jobs:
//! 1. **Model resolution**: `brain/<tier>` aliases and Claude Code's
//!    `claude-*` ids become the tier's OpenRouter model; explicit OpenRouter
//!    ids pass through.
//! 2. **Sanitizer** (Anthropic dialect): drop the fields Claude Code sends
//!    that non-Claude models reject with 400 — see
//!    docs/architecture/client-compatibility.md. Everything else is forwarded
//!    unchanged (open lists, no allowlisting).

use crate::config::Config;
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    pub tier: String,
    pub model: String,
    /// OpenRouter `models[]` fallback chain (primary first), when the tier has a fallback.
    pub chain: Vec<String>,
}

/// Maps the requested model id to a tier and a concrete model.
/// - `brain/fast` … → that tier
/// - `claude-*`, `sonnet`, `opus`, `haiku` → the profile's default tier
/// - an id that is one of the configured tier models → that tier
/// - any other id containing `/` → passed through as-is (tier "custom")
/// - anything else → the profile's default tier
pub fn resolve_model(
    cfg: &Config,
    requested: Option<&str>,
    default_tier: &str,
) -> Option<Resolved> {
    let req = requested.unwrap_or("").trim();
    let tier_of = |name: &str| {
        cfg.tiers
            .get(name)
            .and_then(|t| t.model.as_ref())
            .map(|m| (name.to_string(), m.clone()))
    };
    let picked: Option<(String, String)> = if let Some(t) = req.strip_prefix("brain/") {
        tier_of(t)
    } else if req.is_empty()
        || req.starts_with("claude-")
        || matches!(req, "sonnet" | "opus" | "haiku")
    {
        tier_of(default_tier)
    } else if let Some((name, _)) = cfg
        .tiers
        .iter()
        .find(|(_, t)| t.model.as_deref() == Some(req))
    {
        tier_of(name)
    } else if req.contains('/') {
        Some(("custom".into(), req.to_string()))
    } else {
        tier_of(default_tier)
    };
    let (tier, model) = picked?;
    let mut chain = vec![model.clone()];
    if let Some(fb) = cfg.tiers.get(&tier).and_then(|t| t.fallback.clone()) {
        chain.push(fb);
    }
    Some(Resolved { tier, model, chain })
}

/// Anthropic-dialect body: remove what non-Claude models reject.
/// Returns the names of the fields removed (for the request log).
pub fn sanitize_anthropic(body: &mut Value) -> Vec<&'static str> {
    let mut removed = Vec::new();
    let Some(obj) = body.as_object_mut() else {
        return removed;
    };
    if obj.remove("context_management").is_some() {
        removed.push("context_management");
    }
    if obj.remove("output_config").is_some() {
        removed.push("output_config");
    }
    if obj
        .get("thinking")
        .and_then(|t| t.get("type"))
        .and_then(Value::as_str)
        == Some("adaptive")
    {
        obj.remove("thinking");
        removed.push("thinking.adaptive");
    }
    if let Some(tools) = obj.get_mut("tools").and_then(Value::as_array_mut) {
        let mut hit = false;
        for t in tools.iter_mut() {
            if let Some(t) = t.as_object_mut() {
                hit |= t.remove("strict").is_some();
                hit |= t.remove("defer_loading").is_some();
            }
        }
        if hit {
            removed.push("tools.beta_fields");
        }
    }
    removed
}

/// OpenAI-dialect body: add the fallback chain and ask OpenRouter to report
/// cost in `usage` (both are OpenRouter extensions; harmless elsewhere).
pub fn shape_openai(body: &mut Value, chain: &[String]) {
    let Some(obj) = body.as_object_mut() else {
        return;
    };
    if chain.len() > 1 && !obj.contains_key("models") {
        obj.insert("models".into(), json!(chain));
    }
    if !obj.contains_key("usage") {
        obj.insert("usage".into(), json!({"include": true}));
    }
    // streaming: make sure the final chunk carries usage
    if obj.get("stream").and_then(Value::as_bool) == Some(true)
        && !obj.contains_key("stream_options")
    {
        obj.insert("stream_options".into(), json!({"include_usage": true}));
    }
}

/// Caps `max_tokens` (both dialects use the same field name).
pub fn cap_max_tokens(body: &mut Value, cap: u64) -> bool {
    let Some(obj) = body.as_object_mut() else {
        return false;
    };
    match obj.get("max_tokens").and_then(Value::as_u64) {
        Some(v) if v <= cap => false,
        _ => {
            obj.insert("max_tokens".into(), json!(cap));
            true
        }
    }
}

/// The end-user id a client attached: OpenAI `user` or Anthropic
/// `metadata.user_id`. Claude Code puts a JSON blob there (`device_id`,
/// `account_uuid`, `session_id`): only its `session_id` is kept, as
/// `cc:<12 chars>`, never the device or account identifiers.
pub fn end_user(body: &Value) -> String {
    let raw = body
        .get("user")
        .and_then(Value::as_str)
        .or_else(|| {
            body.get("metadata")
                .and_then(|m| m.get("user_id"))
                .and_then(Value::as_str)
        })
        .unwrap_or("")
        .trim();
    if raw.starts_with('{') {
        return serde_json::from_str::<Value>(raw)
            .ok()
            .and_then(|v| {
                v.get("session_id")
                    .and_then(Value::as_str)
                    .map(|s| format!("cc:{}", s.chars().take(12).collect::<String>()))
            })
            .unwrap_or_default();
    }
    raw.chars().take(64).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Config {
        Config::from_yaml(
            "profiles:\n  - {name: dev, tier: fast, daily_limit_usd: 3.0, monthly_soft_usd: 30.0}\n",
            "tiers:\n  fast: {model: deepseek/deepseek-v4-flash, fallback: qwen/qwen3.7-flash}\n  reasoning: {model: prism-ml/ternary-bonsai-2-27b, fallback: deepseek/deepseek-v4-pro}\n  premium: {model: null}\n",
        )
        .unwrap()
    }

    #[test]
    fn aliases_claude_ids_and_explicit_models_resolve() {
        let c = cfg();
        let r = resolve_model(&c, Some("brain/reasoning"), "fast").unwrap();
        assert_eq!(
            (r.tier.as_str(), r.model.as_str()),
            ("reasoning", "prism-ml/ternary-bonsai-2-27b")
        );
        assert_eq!(
            r.chain,
            vec!["prism-ml/ternary-bonsai-2-27b", "deepseek/deepseek-v4-pro"]
        );
        let r = resolve_model(&c, Some("claude-sonnet-5"), "fast").unwrap();
        assert_eq!(r.tier, "fast");
        assert_eq!(
            resolve_model(&c, Some("opus"), "reasoning").unwrap().tier,
            "reasoning"
        );
        assert_eq!(
            resolve_model(&c, None, "fast").unwrap().model,
            "deepseek/deepseek-v4-flash"
        );
        let r = resolve_model(&c, Some("prism-ml/ternary-bonsai-2-27b"), "fast").unwrap();
        assert_eq!(
            r.tier, "reasoning",
            "a configured model id maps back to its tier"
        );
        let r = resolve_model(&c, Some("google/gemini-2.5-flash-lite"), "fast").unwrap();
        assert_eq!((r.tier.as_str(), r.chain.len()), ("custom", 1));
        assert!(
            resolve_model(&c, Some("brain/premium"), "fast").is_none(),
            "tier without a model"
        );
        assert!(resolve_model(&c, Some("brain/nope"), "fast").is_none());
    }

    #[test]
    fn anthropic_sanitizer_drops_exactly_the_documented_fields() {
        let mut body = json!({
            "model": "claude-x", "max_tokens": 10,
            "system": [{"type": "text", "text": "s", "cache_control": {"type": "ephemeral"}}],
            "thinking": {"type": "adaptive"},
            "context_management": {"edits": []},
            "output_config": {"effort": "high"},
            "tools": [{"name": "t", "input_schema": {}, "strict": true, "defer_loading": true}],
            "metadata": {"user_id": "u1"},
            "messages": [{"role": "user", "content": "hi"}]
        });
        let removed = sanitize_anthropic(&mut body);
        assert_eq!(
            removed,
            vec![
                "context_management",
                "output_config",
                "thinking.adaptive",
                "tools.beta_fields"
            ]
        );
        assert!(body.get("thinking").is_none());
        assert!(body["tools"][0].get("strict").is_none());
        assert_eq!(
            body["system"][0]["cache_control"]["type"], "ephemeral",
            "cache_control untouched"
        );
        assert_eq!(body["metadata"]["user_id"], "u1");
        // enabled (non-adaptive) thinking is left alone
        let mut b2 = json!({"thinking": {"type": "enabled", "budget_tokens": 1024}});
        assert!(sanitize_anthropic(&mut b2).is_empty());
        assert!(b2.get("thinking").is_some());
    }

    #[test]
    fn openai_shaping_adds_fallbacks_usage_and_stream_options_without_overriding() {
        let mut body = json!({"model": "x", "stream": true, "messages": []});
        shape_openai(&mut body, &["a".into(), "b".into()]);
        assert_eq!(body["models"], json!(["a", "b"]));
        assert_eq!(body["usage"]["include"], true);
        assert_eq!(body["stream_options"]["include_usage"], true);
        let mut b2 = json!({"model": "x", "models": ["mine"], "usage": {"include": false}});
        shape_openai(&mut b2, &["a".into(), "b".into()]);
        assert_eq!(b2["models"], json!(["mine"]), "client's own chain wins");
        assert_eq!(b2["usage"]["include"], false);
        let mut b3 = json!({"model": "x"});
        shape_openai(&mut b3, &["a".into()]);
        assert!(b3.get("models").is_none(), "no chain without a fallback");
    }

    #[test]
    fn max_tokens_cap_and_end_user_extraction() {
        let mut b = json!({"max_tokens": 8000});
        assert!(cap_max_tokens(&mut b, 2048));
        assert_eq!(b["max_tokens"], 2048);
        let mut b = json!({"max_tokens": 100});
        assert!(!cap_max_tokens(&mut b, 2048));
        let mut b = json!({});
        assert!(cap_max_tokens(&mut b, 2048));
        assert_eq!(end_user(&json!({"user": "u_1"})), "u_1");
        assert_eq!(end_user(&json!({"metadata": {"user_id": "u_2"}})), "u_2");
        assert_eq!(end_user(&json!({})), "");
        assert_eq!(end_user(&json!({"user": "x".repeat(100)})).len(), 64);
        // Claude Code's metadata blob: keep the session id only
        let cc = json!({"metadata": {"user_id": "{\"device_id\":\"605a480a96e7\",\"account_uuid\":\"acc-1\",\"session_id\":\"1234567890abcdef\"}"}});
        assert_eq!(end_user(&cc), "cc:1234567890ab");
        assert_eq!(
            end_user(&json!({"metadata": {"user_id": "{\"device_id\":\"x\"}"}})),
            ""
        );
        assert_eq!(end_user(&json!({"user": "{not json"})), "");
    }
}
