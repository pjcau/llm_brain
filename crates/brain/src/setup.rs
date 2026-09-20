//! `brain setup claude-code|aider`: print the client configuration for a
//! profile, pointing the tool at OpenRouter directly (Phase 0). Nothing is
//! written to disk; the user pastes what they need.

use crate::config::Config;
use anyhow::Result;
use serde_json::json;

pub const OPENROUTER_ANTHROPIC_BASE: &str = "https://openrouter.ai/api";
pub const OPENROUTER_OPENAI_BASE: &str = "https://openrouter.ai/api/v1";

/// Where the tools' logs go so `brain events ingest` finds them.
pub fn data_dir() -> String {
    std::env::var("BRAIN_DATA").unwrap_or_else(|_| "$HOME/.local/share/llm_brain".into())
}

/// Env block for Claude Code. See docs/architecture/client-compatibility.md
/// for why each variable is there.
pub fn claude_code(cfg: &Config, profile: &str) -> Result<String> {
    let p = cfg.profile(profile)?;
    let fast = cfg.model_for_tier("fast")?;
    let key_env = p.key_env();
    Ok(format!(
        "# Claude Code → OpenRouter (profile `{profile}`, Phase 0). Add to your shell or ~/.claude/settings.json → env\n\
         export ANTHROPIC_BASE_URL={OPENROUTER_ANTHROPIC_BASE}\n\
         export ANTHROPIC_AUTH_TOKEN=\"${key_env}\"\n\
         export ANTHROPIC_MODEL={fast}\n\
         export ANTHROPIC_DEFAULT_HAIKU_MODEL={fast}\n\
         export CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS=1\n\
         export CLAUDE_CODE_DISABLE_ADAPTIVE_THINKING=1\n\
         # switch tier in-session: /model {reasoning}\n",
        reasoning = cfg
            .model_for_tier("reasoning")
            .unwrap_or("<no reasoning tier>"),
    ))
}

/// Env block + `.aider.model.metadata.json` for aider (architect = reasoning, editor = fast).
pub fn aider(cfg: &Config, profile: &str) -> Result<(String, String)> {
    let p = cfg.profile(profile)?;
    let fast = cfg.model_for_tier("fast")?;
    let reasoning = cfg.model_for_tier("reasoning").unwrap_or(fast);
    let key_env = p.key_env();
    let env = format!(
        "# aider → OpenRouter (profile `{profile}`, Phase 0)\n\
         export OPENAI_API_BASE={OPENROUTER_OPENAI_BASE}\n\
         export OPENAI_API_KEY=\"${key_env}\"\n\
         # logs that `brain events ingest` reads (BRAIN_DATA)\n\
         mkdir -p {data}\n\
         # 90/10: editor = fast, architect = reasoning\n\
         aider --architect --model openai/{reasoning} --editor-model openai/{fast} --auto-test \\\n\
               --chat-history-file {data}/aider-chat.md --llm-history-file {data}/aider-llm.history\n",
        data = data_dir(),
    );
    let mut meta = serde_json::Map::new();
    for tier in ["fast", "reasoning"] {
        if let (Some(t), Ok(model)) = (cfg.tiers.get(tier), cfg.model_for_tier(tier)) {
            meta.insert(
                format!("openai/{model}"),
                json!({
                    "max_input_tokens": t.context,
                    "max_output_tokens": 32768,
                    "input_cost_per_token": t.input_usd_per_m / 1_000_000.0,
                    "output_cost_per_token": t.output_usd_per_m / 1_000_000.0,
                    "litellm_provider": "openai",
                    "mode": "chat"
                }),
            );
        }
    }
    Ok((
        env,
        serde_json::to_string_pretty(&serde_json::Value::Object(meta))?,
    ))
}

/// Shell function that runs aider from the Docker image with the current
/// repo mounted, for machines where aider is not installed. Same model
/// split and log files as [`aider`], so `brain events ingest` sees it.
pub fn aider_docker(cfg: &Config, profile: &str, image: &str) -> Result<String> {
    let p = cfg.profile(profile)?;
    let fast = cfg.model_for_tier("fast")?;
    let reasoning = cfg.model_for_tier("reasoning").unwrap_or(fast);
    let key_env = p.key_env();
    let data = data_dir();
    Ok(format!(
        "# aider from Docker → OpenRouter (profile `{profile}`). Paste into ~/.bashrc, then run `or-aider` inside a git repo.\n\
         # Save the metadata JSON from `brain setup aider` as {data}/aider-model-metadata.json for cost display.\n\
         or-aider() {{\n\
           mkdir -p {data}\n\
           docker run --rm -it --user \"$(id -u):$(id -g)\" -e HOME=/tmp \\\n\
             -v \"$PWD:$PWD\" -w \"$PWD\" -v \"{data}:{data}\" \\\n\
             -e OPENAI_API_BASE={OPENROUTER_OPENAI_BASE} -e OPENAI_API_KEY=\"${key_env}\" \\\n\
             {image} aider --architect --model openai/{reasoning} --editor-model openai/{fast} \\\n\
               --no-check-update --no-analytics \\\n\
               --model-metadata-file {data}/aider-model-metadata.json \\\n\
               --chat-history-file {data}/aider-chat.md --llm-history-file {data}/aider-llm.history \"$@\"\n\
         }}\n",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Config {
        Config::from_yaml(
            "profiles:\n  - {name: dev, tier: fast, daily_limit_usd: 3.0, monthly_soft_usd: 30.0}\n",
            "tiers:\n  fast: {model: prism-ml/ternary-bonsai-2-27b, context: 262144, input_usd_per_m: 0.075, output_usd_per_m: 0.5}\n  reasoning: {model: deepseek/deepseek-v4-pro, context: 1048576, input_usd_per_m: 0.42, output_usd_per_m: 0.84}\n",
        )
        .unwrap()
    }

    #[test]
    fn claude_code_block_points_at_openrouter_with_the_profile_key() {
        let out = claude_code(&cfg(), "dev").unwrap();
        assert!(out.contains("ANTHROPIC_BASE_URL=https://openrouter.ai/api\n"));
        assert!(out.contains("ANTHROPIC_AUTH_TOKEN=\"$OPENROUTER_KEY_DEV\""));
        assert!(out.contains("ANTHROPIC_MODEL=prism-ml/ternary-bonsai-2-27b"));
        assert!(out.contains("CLAUDE_CODE_DISABLE_ADAPTIVE_THINKING=1"));
        assert!(out.contains("/model deepseek/deepseek-v4-pro"));
        assert!(claude_code(&cfg(), "nope").is_err());
    }

    #[test]
    fn aider_docker_function_mounts_repo_and_logs_with_the_dev_key() {
        let out = aider_docker(&cfg(), "dev", "llm-brain-aider-test:latest").unwrap();
        assert!(out.contains("or-aider() {"));
        assert!(out.contains("-v \"$PWD:$PWD\" -w \"$PWD\""), "{out}");
        assert!(
            out.contains("-e OPENAI_API_KEY=\"$OPENROUTER_KEY_DEV\""),
            "{out}"
        );
        assert!(out.contains("llm-brain-aider-test:latest aider --architect --model openai/deepseek/deepseek-v4-pro --editor-model openai/prism-ml/ternary-bonsai-2-27b"), "{out}");
        assert!(
            out.contains("--chat-history-file $HOME/.local/share/llm_brain/aider-chat.md"),
            "{out}"
        );
        assert!(
            out.contains(
                "--model-metadata-file $HOME/.local/share/llm_brain/aider-model-metadata.json"
            ),
            "{out}"
        );
    }

    #[test]
    fn aider_block_uses_architect_editor_split_and_prices_metadata() {
        let (env, meta) = aider(&cfg(), "dev").unwrap();
        assert!(env.contains("OPENAI_API_BASE=https://openrouter.ai/api/v1"));
        assert!(env.contains("--model openai/deepseek/deepseek-v4-pro --editor-model openai/prism-ml/ternary-bonsai-2-27b"));
        assert!(
            env.contains("--chat-history-file $HOME/.local/share/llm_brain/aider-chat.md"),
            "{env}"
        );
        let v: serde_json::Value = serde_json::from_str(&meta).unwrap();
        let fast = &v["openai/prism-ml/ternary-bonsai-2-27b"];
        assert_eq!(fast["max_input_tokens"], 262144);
        assert!((fast["input_cost_per_token"].as_f64().unwrap() - 0.075e-6).abs() < 1e-12);
        assert!(v.get("openai/deepseek/deepseek-v4-pro").is_some());
    }
}
