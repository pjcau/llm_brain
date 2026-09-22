//! Phase 0 observability without a proxy: ingest what the tools leave on disk
//! — aider's chat history and Claude Code's session transcripts — into an
//! `events` table, then report requests, errors, error rate, tokens, cache hit
//! and anomalies per day × tool × model. Formats were captured from real
//! files on 2026-09-20 (aider 0.86.2, Claude Code session JSONL).

use crate::config::Config;
use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDateTime, Utc};
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub ts: DateTime<Utc>,
    /// "aider" | "claude"
    pub source: String,
    pub session: String,
    pub model: String,
    /// "request" | "error" | "edit_failed" | "retry"
    pub kind: String,
    pub input_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub output_tokens: i64,
    pub detail: String,
    /// Claude Code only: assistant timestamp minus the previous entry's (request → response).
    pub latency_ms: Option<i64>,
}

// ---------------------------------------------------------------------------
// aider: .aider.chat.history.md
// ---------------------------------------------------------------------------

/// Parses aider's chat history. Sessions start with
/// `# aider chat started at 2026-09-20 08:52:31`; the model line is
/// `> Model: openai/<id> with <fmt> edit format`; every LLM round trip ends
/// with `> Tokens: N sent, M received.`; errors are `> ...` lines.
/// `model_hint` is the model of the file's previous session (state for
/// incremental reads).
pub fn parse_aider_chat(text: &str, model_hint: &str) -> (Vec<Event>, String) {
    let mut events = Vec::new();
    let mut ts = Utc::now();
    let mut session = String::from("unknown");
    let mut model = model_hint.to_string();
    for line in text.lines() {
        let l = line.trim_end();
        if let Some(rest) = l.strip_prefix("# aider chat started at ") {
            if let Ok(t) = NaiveDateTime::parse_from_str(rest.trim(), "%Y-%m-%d %H:%M:%S") {
                ts = t.and_utc();
                session = rest.trim().to_string();
            }
            continue;
        }
        let Some(note) = l.strip_prefix("> ") else {
            continue;
        };
        let note = note.trim();
        if let Some(m) = note.strip_prefix("Model: ") {
            model = m.split(" with ").next().unwrap_or(m).trim().to_string();
            continue;
        }
        if let Some(t) = note.strip_prefix("Tokens: ") {
            // "860 sent, 38 received." — units may be "k" (e.g. "12k sent")
            let mut sent = 0;
            let mut received = 0;
            for part in t.trim_end_matches('.').split(',') {
                let mut it = part.split_whitespace();
                let (Some(n), Some(what)) = (it.next(), it.next()) else {
                    continue;
                };
                let n = parse_tokens(n);
                match what {
                    "sent" => sent = n,
                    "received" => received = n,
                    _ => {}
                }
            }
            events.push(Event {
                ts,
                source: "aider".into(),
                session: session.clone(),
                model: model.clone(),
                kind: "request".into(),
                input_tokens: sent,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                output_tokens: received,
                detail: String::new(),
                latency_ms: None,
            });
            continue;
        }
        let lower = note.to_ascii_lowercase();
        let kind = if lower.contains("did not conform")
            || lower.contains("failed to apply")
            || lower.contains("malformed")
        {
            Some("edit_failed")
        } else if lower.starts_with("retrying") {
            Some("retry")
        } else if lower.contains("error") || lower.contains("exception") {
            Some("error")
        } else {
            None
        };
        if let Some(kind) = kind {
            events.push(Event {
                ts,
                source: "aider".into(),
                session: session.clone(),
                model: model.clone(),
                kind: kind.into(),
                input_tokens: 0,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                output_tokens: 0,
                detail: note.chars().take(200).collect(),
                latency_ms: None,
            });
        }
    }
    (events, model)
}

fn parse_tokens(s: &str) -> i64 {
    let s = s.trim();
    if let Some(k) = s.strip_suffix('k') {
        return (k.parse::<f64>().unwrap_or(0.0) * 1000.0) as i64;
    }
    s.parse().unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Claude Code: ~/.claude/projects/<project>/<session>.jsonl
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ClaudeLine {
    #[serde(rename = "type")]
    kind: Option<String>,
    timestamp: Option<String>,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    error: Option<String>,
    #[serde(rename = "isApiErrorMessage")]
    is_api_error: Option<bool>,
    message: Option<ClaudeMessage>,
}

#[derive(Deserialize)]
struct ClaudeMessage {
    model: Option<String>,
    usage: Option<ClaudeUsage>,
    content: Option<serde_json::Value>,
}

#[derive(Deserialize, Default)]
struct ClaudeUsage {
    #[serde(default)]
    input_tokens: i64,
    #[serde(default)]
    cache_creation_input_tokens: i64,
    #[serde(default)]
    cache_read_input_tokens: i64,
    #[serde(default)]
    output_tokens: i64,
}

/// One event per assistant message: a request with usage, or an API error
/// (`isApiErrorMessage`). Unparseable lines are skipped.
pub fn parse_claude_session(text: &str) -> Vec<Event> {
    let mut events = Vec::new();
    let mut prev_ts: Option<DateTime<Utc>> = None;
    for line in text.lines() {
        let Ok(l) = serde_json::from_str::<ClaudeLine>(line) else {
            continue;
        };
        let line_ts = l
            .timestamp
            .as_deref()
            .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
            .map(|t| t.with_timezone(&Utc));
        if l.kind.as_deref() != Some("assistant") {
            if line_ts.is_some() {
                prev_ts = line_ts;
            }
            continue;
        }
        let Some(msg) = l.message else { continue };
        let ts = l
            .timestamp
            .as_deref()
            .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
            .map(|t| t.with_timezone(&Utc))
            .unwrap_or_else(Utc::now);
        let session = l.session_id.unwrap_or_default();
        if l.is_api_error == Some(true) || l.error.is_some() {
            let text = msg
                .content
                .as_ref()
                .and_then(|c| c.get(0))
                .and_then(|b| b.get("text"))
                .and_then(|t| t.as_str())
                .unwrap_or_default();
            events.push(Event {
                ts,
                source: "claude".into(),
                session,
                model: msg.model.unwrap_or_default(),
                kind: "error".into(),
                input_tokens: 0,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                output_tokens: 0,
                detail: format!("{}: {}", l.error.unwrap_or_default(), text)
                    .chars()
                    .take(200)
                    .collect(),
                latency_ms: None,
            });
            continue;
        }
        let latency_ms = match (prev_ts, line_ts) {
            (Some(p), Some(t)) => Some((t - p).num_milliseconds().max(0)),
            _ => None,
        };
        prev_ts = line_ts;
        let u = msg.usage.unwrap_or_default();
        events.push(Event {
            ts,
            source: "claude".into(),
            session,
            model: msg.model.unwrap_or_default(),
            kind: "request".into(),
            input_tokens: u.input_tokens,
            cache_read_tokens: u.cache_read_input_tokens,
            cache_write_tokens: u.cache_creation_input_tokens,
            output_tokens: u.output_tokens,
            detail: String::new(),
            latency_ms,
        });
    }
    events
}

// ---------------------------------------------------------------------------
// Report
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Default)]
pub struct DailyStat {
    pub day: String,
    pub source: String,
    pub model: String,
    pub requests: i64,
    pub errors: i64,
    pub edit_failed: i64,
    pub retries: i64,
    pub input_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub output_tokens: i64,
    /// Mean request → response latency where known (Claude Code).
    pub avg_latency_ms: Option<f64>,
}

impl DailyStat {
    pub fn error_rate(&self) -> f64 {
        if self.requests == 0 {
            return 0.0;
        }
        (self.errors + self.edit_failed) as f64 / self.requests as f64
    }
    /// Share of the prompt served from the provider cache (Claude Code reports it; aider doesn't).
    pub fn cache_hit(&self) -> Option<f64> {
        let total = self.input_tokens + self.cache_read_tokens + self.cache_write_tokens;
        (self.cache_read_tokens + self.cache_write_tokens > 0)
            .then(|| self.cache_read_tokens as f64 / total as f64)
    }
    /// Estimated cost from the tier prices when the model is a configured tier model.
    pub fn est_cost(&self, cfg: &Config) -> Option<f64> {
        let tier = cfg.tiers.values().find(|t| {
            t.model
                .as_deref()
                .is_some_and(|m| crate::catalog::base_id(&self.model).ends_with(m))
        })?;
        let input = (self.input_tokens + self.cache_write_tokens) as f64
            + self.cache_read_tokens as f64 * 0.25;
        Some(
            input / 1e6 * tier.input_usd_per_m
                + self.output_tokens as f64 / 1e6 * tier.output_usd_per_m,
        )
    }
}

/// Traffic that never touches OpenRouter: Claude Code on the claude.ai
/// subscription (`claude-*` models) and its synthetic error entries. Shown
/// in the tables, excluded from the anomaly rules, which are about the
/// cheap-model setup.
pub fn is_subscription(model: &str) -> bool {
    model.starts_with("claude-") || model == "<synthetic>"
}

/// Anomaly rules for Phase 0. Thresholds are deliberately simple; they are
/// the questions the week must answer.
pub fn anomalies(stats: &[DailyStat]) -> Vec<String> {
    let mut out = Vec::new();
    for s in stats.iter().filter(|s| !is_subscription(&s.model)) {
        let who = format!("{} {} {}", s.day, s.source, s.model);
        if s.requests >= 5 && s.error_rate() > 0.10 {
            out.push(format!(
                "{who}: error rate {:.0}% ({} errors, {} bad edits on {} requests)",
                s.error_rate() * 100.0,
                s.errors,
                s.edit_failed,
                s.requests
            ));
        }
        if s.retries >= 3 {
            out.push(format!("{who}: {} retries", s.retries));
        }
        if let Some(h) = s.cache_hit()
            && s.requests >= 10
            && h < 0.30
        {
            out.push(format!(
                "{who}: cache hit {:.0}% (< 30%): prompt prefix unstable or provider without cache",
                h * 100.0
            ));
        }
        if s.requests >= 5 && s.output_tokens > 0 && s.input_tokens + s.cache_read_tokens > 0 {
            let ratio = (s.input_tokens + s.cache_read_tokens + s.cache_write_tokens) as f64
                / s.requests as f64;
            if ratio > 150_000.0 {
                out.push(format!(
                    "{who}: {:.0}k prompt tokens per request: context not trimmed",
                    ratio / 1000.0
                ));
            }
        }
    }
    out
}

pub fn render_report(cfg: &Config, stats: &[DailyStat]) -> String {
    let mut out = format!(
        "{:<10} {:<7} {:<34} {:>5} {:>4} {:>4} {:>4} {:>8} {:>8} {:>6} {:>7} {:>7}\n",
        "day",
        "tool",
        "model",
        "req",
        "err",
        "bad",
        "rtry",
        "in_tok",
        "out_tok",
        "cache",
        "est$",
        "lat_s"
    );
    for s in stats {
        out.push_str(&format!(
            "{:<10} {:<7} {:<34} {:>5} {:>4} {:>4} {:>4} {:>8} {:>8} {:>6} {:>7} {:>7}\n",
            s.day,
            s.source,
            s.model.chars().take(34).collect::<String>(),
            s.requests,
            s.errors,
            s.edit_failed,
            s.retries,
            s.input_tokens + s.cache_read_tokens + s.cache_write_tokens,
            s.output_tokens,
            s.cache_hit()
                .map(|h| format!("{:.0}%", h * 100.0))
                .unwrap_or_else(|| "-".into()),
            s.est_cost(cfg)
                .map(|c| format!("{c:.3}"))
                .unwrap_or_else(|| "-".into()),
            s.avg_latency_ms
                .map(|l| format!("{:.1}", l / 1000.0))
                .unwrap_or_else(|| "-".into()),
        ));
    }
    if stats.is_empty() {
        out.push_str("(no events yet — run `brain events ingest`)\n");
    }
    let a = anomalies(stats);
    if !a.is_empty() {
        out.push_str("\nanomalies:\n");
        for line in a {
            out.push_str(&format!("  ! {line}\n"));
        }
    }
    out
}

/// Where the tools' logs live.
pub struct IngestPaths {
    /// The global aider history (Phase 0 setup), if any.
    pub aider_chat: std::path::PathBuf,
    /// Directories scanned (3 levels deep) for per-repo `.aider.chat.history.md`
    /// files, the default aider location that `--restore-chat-history` uses.
    pub aider_roots: Vec<std::path::PathBuf>,
    /// Roots that contain `<project>/<session>.jsonl` (host and Docker HOMEs).
    pub claude_roots: Vec<std::path::PathBuf>,
}

impl IngestPaths {
    pub fn from_env(
        env: &std::collections::HashMap<String, String>,
        aider_chat: Option<std::path::PathBuf>,
        claude_projects: Option<std::path::PathBuf>,
    ) -> Self {
        let home = env.get("HOME").cloned().unwrap_or_default();
        let data = env
            .get("BRAIN_DATA")
            .cloned()
            .unwrap_or_else(|| format!("{home}/.local/share/llm_brain"));
        let aider_chat =
            aider_chat.unwrap_or_else(|| std::path::PathBuf::from(format!("{data}/aider-chat.md")));
        let host_projects = claude_projects
            .unwrap_or_else(|| std::path::PathBuf::from(format!("{home}/.claude/projects")));
        let docker_projects =
            std::path::PathBuf::from(format!("{data}/claude-home/.claude/projects"));
        let mut claude_roots = vec![host_projects];
        if let Some(extra) = env.get("BRAIN_CLAUDE_PROJECTS") {
            claude_roots.push(std::path::PathBuf::from(extra));
        }
        claude_roots.push(docker_projects);
        let aider_roots = env
            .get("BRAIN_AIDER_ROOTS")
            .map(|v| {
                v.split(':')
                    .filter(|s| !s.is_empty())
                    .map(std::path::PathBuf::from)
                    .collect()
            })
            .unwrap_or_else(|| {
                vec![std::path::PathBuf::from(format!(
                    "{home}/Documents/myProjects"
                ))]
            });
        Self {
            aider_chat,
            claude_roots,
            aider_roots,
        }
    }
}

/// Incremental ingest of every known log into `db`; returns the number of
/// events added. Missing or unreadable files are skipped with a note.
pub fn ingest(db: &crate::db::Db, paths: &IngestPaths) -> Result<usize> {
    let mut total = 0;
    let mut aider_files = vec![paths.aider_chat.clone()];
    for root in &paths.aider_roots {
        find_aider_histories(root, 3, &mut aider_files);
    }
    for file in aider_files.iter().filter(|f| f.is_file()) {
        let key = file.to_string_lossy().to_string();
        let (offset, hint) = db.ingest_state(&key)?;
        let (text, new_offset) = match read_from(file, offset) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("skipping {}: {e:#}", file.display());
                continue;
            }
        };
        if new_offset == offset {
            continue;
        }
        let (ev, model) = parse_aider_chat(&text, &hint);
        total += db.insert_events(&ev)?;
        db.set_ingest_state(&key, new_offset, &model)?;
    }
    let mut sessions = Vec::new();
    for root in &paths.claude_roots {
        let Ok(projects) = std::fs::read_dir(root) else {
            continue;
        };
        for project in projects.flatten() {
            if let Ok(files) = std::fs::read_dir(project.path()) {
                sessions.extend(
                    files
                        .flatten()
                        .map(|f| f.path())
                        .filter(|p| p.extension().is_some_and(|e| e == "jsonl")),
                );
            }
        }
    }
    for path in sessions {
        let key = path.to_string_lossy().to_string();
        let (offset, _) = db.ingest_state(&key)?;
        let (text, new_offset) = match read_from(&path, offset) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("skipping {}: {e:#}", path.display());
                continue;
            }
        };
        if new_offset == offset {
            continue;
        }
        total += db.insert_events(&parse_claude_session(&text))?;
        db.set_ingest_state(&key, new_offset, "")?;
    }
    Ok(total)
}

/// Collects `.aider.chat.history.md` files under `dir`, `depth` levels deep,
/// skipping hidden and dependency directories.
fn find_aider_histories(dir: &std::path::Path, depth: u32, out: &mut Vec<std::path::PathBuf>) {
    let candidate = dir.join(".aider.chat.history.md");
    if candidate.is_file() {
        out.push(candidate);
    }
    if depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        let name = e.file_name();
        let name = name.to_string_lossy();
        if p.is_dir()
            && !name.starts_with('.')
            && name != "node_modules"
            && name != "target"
            && name != "venv"
        {
            find_aider_histories(&p, depth - 1, out);
        }
    }
}

/// Reads the part of `path` after `offset` (append-only files).
pub fn read_from(path: &Path, offset: u64) -> Result<(String, u64)> {
    let data = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let start = (offset as usize).min(data.len());
    let text = String::from_utf8_lossy(&data[start..]).to_string();
    Ok((text, data.len() as u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHAT: &str = "\
# aider chat started at 2026-09-20 08:52:31

> Aider v0.86.2  
> Model: openai/prism-ml/ternary-bonsai-2-27b with whole edit format  
> Git repo: .git with 1 files  

#### Reply with exactly one word: pong  

pong

> Tokens: 860 sent, 38 received.  

#### fix the bug  

> litellm.BadRequestError: OpenAIException - Provider returned error  
> Retrying in 0.2 seconds...  
> Tokens: 12k sent, 1.2k received.  
> The LLM did not conform to the edit format.  
";

    #[test]
    fn aider_chat_yields_requests_errors_and_model_state() {
        let (ev, model) = parse_aider_chat(CHAT, "");
        assert_eq!(model, "openai/prism-ml/ternary-bonsai-2-27b");
        let kinds: Vec<&str> = ev.iter().map(|e| e.kind.as_str()).collect();
        assert_eq!(
            kinds,
            vec!["request", "error", "retry", "request", "edit_failed"]
        );
        assert_eq!(ev[0].input_tokens, 860);
        assert_eq!(ev[0].output_tokens, 38);
        assert_eq!(ev[3].input_tokens, 12000);
        assert_eq!(ev[3].output_tokens, 1200);
        assert!(ev.iter().all(|e| e.model == model && e.source == "aider"));
        assert_eq!(
            ev[0].ts.format("%Y-%m-%d %H:%M").to_string(),
            "2026-09-20 08:52"
        );
        // incremental read without the Model line keeps the hint
        let (ev2, m2) = parse_aider_chat("> Tokens: 5 sent, 1 received.\n", &model);
        assert_eq!(ev2[0].model, model);
        assert_eq!(m2, model);
    }

    #[test]
    fn claude_session_yields_usage_and_api_errors() {
        let user = r#"{"type":"user","timestamp":"2026-09-19T09:13:30.000Z","sessionId":"s1","message":{"role":"user","content":"hi"}}"#;
        let ok = r#"{"type":"assistant","timestamp":"2026-09-19T09:13:34.802Z","sessionId":"s1","message":{"model":"prism-ml/ternary-bonsai-2-27b","usage":{"input_tokens":2,"cache_creation_input_tokens":14259,"cache_read_input_tokens":24641,"output_tokens":526}}}"#;
        let err = r#"{"type":"assistant","timestamp":"2026-09-19T09:20:00.000Z","sessionId":"s1","error":"authentication_failed","isApiErrorMessage":true,"message":{"model":"<synthetic>","content":[{"type":"text","text":"Login expired"}]}}"#;
        let junk = "not json\n{\"type\":\"user\"}\n";
        let ev = parse_claude_session(&format!("{user}\n{ok}\n{err}\n{junk}"));
        assert_eq!(ev.len(), 2);
        assert_eq!(ev[0].kind, "request");
        assert_eq!(ev[0].latency_ms, Some(4802));
        assert_eq!(ev[0].cache_read_tokens, 24641);
        assert_eq!(ev[0].cache_write_tokens, 14259);
        assert_eq!(ev[0].output_tokens, 526);
        assert_eq!(ev[0].model, "prism-ml/ternary-bonsai-2-27b");
        assert_eq!(ev[1].kind, "error");
        assert!(
            ev[1]
                .detail
                .starts_with("authentication_failed: Login expired")
        );
    }

    fn cfg() -> Config {
        Config::from_yaml(
            "profiles:\n  - {name: dev, tier: fast, daily_limit_usd: 3.0, monthly_soft_usd: 30.0}\n",
            "tiers:\n  fast: {model: prism-ml/ternary-bonsai-2-27b, input_usd_per_m: 0.075, output_usd_per_m: 0.5}\n",
        )
        .unwrap()
    }

    #[test]
    fn report_flags_error_rate_low_cache_and_fat_prompts() {
        let base = DailyStat {
            day: "2026-09-20".into(),
            source: "claude".into(),
            model: "prism-ml/ternary-bonsai-2-27b".into(),
            ..Default::default()
        };
        let bad = DailyStat {
            requests: 20,
            errors: 3,
            edit_failed: 0,
            retries: 4,
            input_tokens: 4_000_000,
            cache_read_tokens: 100_000,
            cache_write_tokens: 200_000,
            output_tokens: 50_000,
            ..base.clone()
        };
        let good = DailyStat {
            requests: 20,
            input_tokens: 10_000,
            cache_read_tokens: 900_000,
            cache_write_tokens: 50_000,
            output_tokens: 5_000,
            ..base.clone()
        };
        let a = anomalies(std::slice::from_ref(&bad));
        assert!(a.iter().any(|l| l.contains("error rate 15%")), "{a:?}");
        assert!(a.iter().any(|l| l.contains("4 retries")), "{a:?}");
        assert!(a.iter().any(|l| l.contains("cache hit 2%")), "{a:?}");
        assert!(
            a.iter().any(|l| l.contains("prompt tokens per request")),
            "{a:?}"
        );
        assert!(anomalies(std::slice::from_ref(&good)).is_empty());
        // subscription traffic is never an anomaly of the cheap-model setup
        let sub = DailyStat {
            model: "claude-opus-5".into(),
            ..bad.clone()
        };
        assert!(anomalies(std::slice::from_ref(&sub)).is_empty());
        assert!(is_subscription("<synthetic>") && !is_subscription("deepseek/deepseek-v4-flash"));
        // estimated cost from tier prices: cached reads at 0.25×
        let c = good.est_cost(&cfg()).unwrap();
        let expected =
            ((10_000.0 + 50_000.0) + 900_000.0 * 0.25) / 1e6 * 0.075 + 5_000.0 / 1e6 * 0.5;
        assert!((c - expected).abs() < 1e-9);
        // a routing variant of a tier model is priced as that tier
        let exacto = DailyStat {
            model: format!("{}:exacto", good.model),
            ..good.clone()
        };
        assert_eq!(exacto.est_cost(&cfg()), Some(c));
        let r = render_report(&cfg(), &[good]);
        assert!(r.contains("94%"), "{r}");
        assert!(render_report(&cfg(), &[]).contains("no events yet"));
    }

    #[test]
    fn ingest_is_incremental_across_aider_and_claude_logs() {
        let dir = tempfile::tempdir().unwrap();
        let aider = dir.path().join("aider-chat.md");
        std::fs::write(&aider, CHAT).unwrap();
        let proj = dir.path().join("projects/p1");
        std::fs::create_dir_all(&proj).unwrap();
        let sess = proj.join("s.jsonl");
        std::fs::write(&sess, r#"{"type":"assistant","timestamp":"2026-09-19T09:13:34.802Z","sessionId":"s1","message":{"model":"m","usage":{"input_tokens":2,"output_tokens":5}}}"#).unwrap();
        let db = crate::db::Db::memory().unwrap();
        let paths = IngestPaths {
            aider_chat: aider.clone(),
            claude_roots: vec![dir.path().join("projects"), dir.path().join("missing")],
            aider_roots: vec![],
        };
        assert_eq!(ingest(&db, &paths).unwrap(), 6, "5 aider events + 1 claude");
        assert_eq!(ingest(&db, &paths).unwrap(), 0, "nothing new");
        // append to both files: only the new lines are read, model hint kept
        // (a new session without a Model line; dated so the stat lands on the
        // same day as the fixture instead of on "today")
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&aider)
            .unwrap();
        std::io::Write::write_all(
            &mut f,
            b"\n# aider chat started at 2026-09-20 10:00:00\n\n> Tokens: 5 sent, 1 received.\n",
        )
        .unwrap();
        let mut g = std::fs::OpenOptions::new()
            .append(true)
            .open(&sess)
            .unwrap();
        std::io::Write::write_all(&mut g, b"\n{\"type\":\"assistant\",\"timestamp\":\"2026-09-19T09:14:00.000Z\",\"sessionId\":\"s1\",\"message\":{\"model\":\"m\",\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}").unwrap();
        assert_eq!(ingest(&db, &paths).unwrap(), 2);
        let stats = db.daily_stats(3650).unwrap();
        let aider_stat = stats.iter().find(|s| s.source == "aider").unwrap();
        assert_eq!(aider_stat.model, "openai/prism-ml/ternary-bonsai-2-27b");
        assert_eq!(aider_stat.requests, 3);
    }

    #[test]
    fn paths_from_env_default_to_data_dir_and_both_claude_homes() {
        let env = std::collections::HashMap::from([("HOME".to_string(), "/h".to_string())]);
        let p = IngestPaths::from_env(&env, None, None);
        assert_eq!(
            p.aider_chat,
            std::path::PathBuf::from("/h/.local/share/llm_brain/aider-chat.md")
        );
        assert_eq!(
            p.claude_roots,
            vec![
                std::path::PathBuf::from("/h/.claude/projects"),
                std::path::PathBuf::from("/h/.local/share/llm_brain/claude-home/.claude/projects")
            ]
        );
        assert_eq!(
            p.aider_roots,
            vec![std::path::PathBuf::from("/h/Documents/myProjects")]
        );
        let env3 = std::collections::HashMap::from([
            ("HOME".to_string(), "/h".to_string()),
            ("BRAIN_AIDER_ROOTS".to_string(), "/a:/b".to_string()),
        ]);
        assert_eq!(
            IngestPaths::from_env(&env3, None, None).aider_roots.len(),
            2
        );
        let env2 = std::collections::HashMap::from([
            ("HOME".to_string(), "/h".to_string()),
            ("BRAIN_DATA".to_string(), "/d".to_string()),
            ("BRAIN_CLAUDE_PROJECTS".to_string(), "/x".to_string()),
        ]);
        let p2 = IngestPaths::from_env(&env2, Some("/a.md".into()), None);
        assert_eq!(p2.aider_chat, std::path::PathBuf::from("/a.md"));
        assert_eq!(p2.claude_roots.len(), 3);
        assert_eq!(p2.claude_roots[1], std::path::PathBuf::from("/x"));
    }

    #[test]
    fn per_repo_aider_histories_are_discovered_under_the_roots() {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("projects/app");
        std::fs::create_dir_all(repo.join("node_modules/x")).unwrap();
        std::fs::write(repo.join(".aider.chat.history.md"), CHAT).unwrap();
        std::fs::write(repo.join("node_modules/x/.aider.chat.history.md"), CHAT).unwrap(); // ignored
        let db = crate::db::Db::memory().unwrap();
        let paths = IngestPaths {
            aider_chat: root.path().join("none.md"),
            claude_roots: vec![],
            aider_roots: vec![root.path().join("projects")],
        };
        assert_eq!(
            ingest(&db, &paths).unwrap(),
            5,
            "the repo history, not the one under node_modules"
        );
        assert_eq!(ingest(&db, &paths).unwrap(), 0);
    }

    #[test]
    fn read_from_returns_only_the_tail() {
        let f = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(f.path(), "abc\ndef\n").unwrap();
        let (t, n) = read_from(f.path(), 4).unwrap();
        assert_eq!(t, "def\n");
        assert_eq!(n, 8);
        assert_eq!(read_from(f.path(), 100).unwrap().0, "");
    }
}
