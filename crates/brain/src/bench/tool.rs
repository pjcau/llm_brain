//! How each tool is invoked headless. Pure functions, so the exact env and
//! arguments are unit-tested; the container test in `tests/` checks them
//! against the real `aider`.

use clap::ValueEnum;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

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

/// aider-specific knobs mirrored from `brain setup aider`.
#[derive(Debug, Clone, Default)]
pub struct AiderOptions {
    /// `--architect --model <model> --editor-model <editor>`: the reasoning
    /// tier proposes, the fast tier applies the edits.
    pub editor_model: Option<String>,
    /// `--model-settings-file`: OpenRouter fallback chains per model.
    pub settings_file: Option<PathBuf>,
}

/// Builds the headless invocation for `tool` with `model`, working in the
/// task's worktree (the caller sets the cwd).
pub fn invocation(
    tool: Tool,
    endpoint: &Endpoint,
    model: &str,
    prompt: &str,
    run_id: &str,
    aider: &AiderOptions,
) -> ToolInvocation {
    let mut env = BTreeMap::new();
    match tool {
        Tool::Aider => {
            env.insert("OPENAI_API_BASE".into(), endpoint.openai_base.clone());
            env.insert("OPENAI_API_KEY".into(), endpoint.api_key.clone());
            // aider reads OPENAI_API_BASE via LiteLLM; the model needs the openai/ prefix
            let mut args: Vec<String> = Vec::new();
            if let Some(editor) = aider.editor_model.as_deref() {
                args.extend([
                    "--architect".into(),
                    "--editor-model".into(),
                    format!("openai/{editor}"),
                ]);
            }
            if let Some(f) = aider.settings_file.as_deref() {
                args.extend(["--model-settings-file".into(), f.display().to_string()]);
            }
            args.extend(vec![
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
            ]);
            ToolInvocation {
                program: "aider".into(),
                args,
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

/// Wraps an invocation in `docker run` so a tool that is not installed on the
/// host (aider) runs from an image. The worktree and the repo cache are
/// mounted at their host paths, so the worktree's `.git` pointer still
/// resolves; the container runs as the host user so files stay writable.
pub fn dockerize(
    inv: &ToolInvocation,
    image: &str,
    worktree: &Path,
    cache: &Path,
    uid_gid: &str,
    extra_ro: &[PathBuf],
) -> ToolInvocation {
    let mut args: Vec<String> = vec![
        "run".into(),
        "--rm".into(),
        "--user".into(),
        uid_gid.into(),
        "-e".into(),
        "HOME=/tmp".into(),
        "-v".into(),
        format!("{0}:{0}", worktree.display()),
        "-v".into(),
        format!("{0}:{0}", cache.display()),
        "-w".into(),
        worktree.display().to_string(),
    ];
    for p in extra_ro {
        args.push("-v".into());
        args.push(format!("{0}:{0}:ro", p.display()));
    }
    // `-e NAME` without a value: docker copies it from the calling process's
    // environment, so the key never appears on the command line (`ps`).
    for k in inv.env.keys() {
        args.push("-e".into());
        args.push(k.clone());
    }
    args.push(image.into());
    args.push(inv.program.clone());
    args.extend(inv.args.iter().cloned());
    ToolInvocation {
        program: "docker".into(),
        args,
        env: inv.env.clone(),
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
            &AiderOptions::default(),
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
    fn dockerize_mounts_paths_passes_env_and_keeps_the_command() {
        let inv = invocation(
            Tool::Aider,
            &Endpoint::openrouter("sk-b"),
            "m",
            "fix it",
            "r1",
            &AiderOptions::default(),
        );
        let d = dockerize(
            &inv,
            "llm-brain-aider-test:latest",
            Path::new("/tmp/wt-1"),
            Path::new("/repo/cache"),
            "1000:1000",
            &[PathBuf::from("/data/llm_brain")],
        );
        assert_eq!(d.program, "docker");
        assert_eq!(
            d.env, inv.env,
            "env stays on the docker process, inherited by the container"
        );
        let a = d.args.join(" ");
        assert!(
            a.starts_with("run --rm --user 1000:1000 -e HOME=/tmp"),
            "{a}"
        );
        assert!(
            a.contains("-v /tmp/wt-1:/tmp/wt-1 -v /repo/cache:/repo/cache -w /tmp/wt-1 -v /data/llm_brain:/data/llm_brain:ro"),
            "{a}"
        );
        assert!(a.contains("-e OPENAI_API_BASE -e OPENAI_API_KEY"), "{a}");
        assert!(
            !a.contains("sk-b"),
            "the key must never be on the docker command line: {a}"
        );
        let i = d
            .args
            .iter()
            .position(|x| x == "llm-brain-aider-test:latest")
            .unwrap();
        assert_eq!(d.args[i + 1], "aider");
        assert_eq!(&d.args[i + 2..i + 4], ["--model", "openai/m"]);
    }

    #[test]
    fn aider_architect_mode_and_settings_file_are_passed_through() {
        let opts = AiderOptions {
            editor_model: Some("deepseek/deepseek-v4-flash".into()),
            settings_file: Some(PathBuf::from("/data/s.yml")),
        };
        let inv = invocation(
            Tool::Aider,
            &Endpoint::openrouter("k"),
            "prism-ml/ternary-bonsai-2-27b",
            "p",
            "r",
            &opts,
        );
        let a = inv.args.join(" ");
        assert!(
            a.starts_with("--architect --editor-model openai/deepseek/deepseek-v4-flash --model-settings-file /data/s.yml --model openai/prism-ml/ternary-bonsai-2-27b"),
            "{a}"
        );
    }

    #[test]
    fn claude_gets_anthropic_env_sanitizer_flags_and_run_header() {
        let inv = invocation(
            Tool::Claude,
            &Endpoint::openrouter("sk-b"),
            "deepseek/deepseek-v4-pro",
            "fix it",
            "run-42",
            &AiderOptions::default(),
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
