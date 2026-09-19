//! How each tool is invoked headless. Pure functions, so the exact env and
//! arguments are unit-tested; the container test in `tests/` checks them
//! against the real `aider`.

use clap::ValueEnum;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Tool {
    Aider,
    Claude,
}

impl Tool {
    pub fn as_str(self) -> &'static str {
        match self {
            Tool::Aider => "aider",
            Tool::Claude => "claude",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolInvocation {
    pub program: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
}

/// Where the tool sends requests. Phase 0: OpenRouter; later: llm_brain.
#[derive(Debug, Clone)]
pub struct Endpoint {
    /// OpenAI-dialect base (…/v1) for aider.
    pub openai_base: String,
    /// Anthropic-dialect base (no /v1) for Claude Code.
    pub anthropic_base: String,
    pub api_key: String,
}

impl Endpoint {
    pub fn openrouter(api_key: impl Into<String>) -> Self {
        Self {
            openai_base: "https://openrouter.ai/api/v1".into(),
            anthropic_base: "https://openrouter.ai/api".into(),
            api_key: api_key.into(),
        }
    }
}

/// Builds the headless invocation for `tool` with `model`, working in the
/// task's worktree (the caller sets the cwd).
pub fn invocation(
    tool: Tool,
    endpoint: &Endpoint,
    model: &str,
    prompt: &str,
    run_id: &str,
) -> ToolInvocation {
    let mut env = BTreeMap::new();
    match tool {
        Tool::Aider => {
            env.insert("OPENAI_API_BASE".into(), endpoint.openai_base.clone());
            env.insert("OPENAI_API_KEY".into(), endpoint.api_key.clone());
            // aider reads OPENAI_API_BASE via LiteLLM; the model needs the openai/ prefix
            ToolInvocation {
                program: "aider".into(),
                args: vec![
                    "--model".into(),
                    format!("openai/{model}"),
                    "--message".into(),
                    prompt.to_string(),
                    "--yes-always".into(),
                    "--no-show-model-warnings".into(),
                    "--no-check-update".into(),
                    "--no-analytics".into(),
                    "--no-auto-commits".into(),
                    "--no-stream".into(),
                ],
                env,
            }
        }
        Tool::Claude => {
            env.insert("ANTHROPIC_BASE_URL".into(), endpoint.anthropic_base.clone());
            env.insert("ANTHROPIC_AUTH_TOKEN".into(), endpoint.api_key.clone());
            env.insert("ANTHROPIC_MODEL".into(), model.to_string());
            env.insert("ANTHROPIC_DEFAULT_HAIKU_MODEL".into(), model.to_string());
            env.insert("CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS".into(), "1".into());
            env.insert("CLAUDE_CODE_DISABLE_ADAPTIVE_THINKING".into(), "1".into());
            env.insert(
                "ANTHROPIC_CUSTOM_HEADERS".into(),
                format!("X-Brain-Run: {run_id}"),
            );
            ToolInvocation {
                program: "claude".into(),
                args: vec![
                    "-p".into(),
                    prompt.to_string(),
                    "--model".into(),
                    model.to_string(),
                    "--output-format".into(),
                    "json".into(),
                    "--permission-mode".into(),
                    "acceptEdits".into(),
                ],
                env,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aider_gets_openai_env_and_prefixed_model() {
        let inv = invocation(
            Tool::Aider,
            &Endpoint::openrouter("sk-b"),
            "prism-ml/ternary-bonsai-2-27b",
            "fix it",
            "r1",
        );
        assert_eq!(inv.program, "aider");
        assert_eq!(inv.env["OPENAI_API_BASE"], "https://openrouter.ai/api/v1");
        assert_eq!(inv.env["OPENAI_API_KEY"], "sk-b");
        let i = inv.args.iter().position(|a| a == "--model").unwrap();
        assert_eq!(inv.args[i + 1], "openai/prism-ml/ternary-bonsai-2-27b");
        assert!(inv.args.contains(&"--yes-always".to_string()));
        assert!(inv.args.contains(&"--no-auto-commits".to_string()));
        assert!(!inv.env.contains_key("ANTHROPIC_BASE_URL"));
    }

    #[test]
    fn claude_gets_anthropic_env_sanitizer_flags_and_run_header() {
        let inv = invocation(
            Tool::Claude,
            &Endpoint::openrouter("sk-b"),
            "deepseek/deepseek-v4-pro",
            "fix it",
            "run-42",
        );
        assert_eq!(inv.program, "claude");
        assert_eq!(inv.env["ANTHROPIC_BASE_URL"], "https://openrouter.ai/api");
        assert_eq!(inv.env["ANTHROPIC_AUTH_TOKEN"], "sk-b");
        assert_eq!(inv.env["ANTHROPIC_MODEL"], "deepseek/deepseek-v4-pro");
        assert_eq!(inv.env["CLAUDE_CODE_DISABLE_ADAPTIVE_THINKING"], "1");
        assert_eq!(inv.env["ANTHROPIC_CUSTOM_HEADERS"], "X-Brain-Run: run-42");
        assert_eq!(inv.args[0..2], ["-p".to_string(), "fix it".to_string()]);
        assert!(
            inv.args
                .windows(2)
                .any(|w| w == ["--output-format", "json"])
        );
    }
}
