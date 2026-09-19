//! Task format: one YAML per task in `bench/tasks/`.

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Task {
    pub id: String,
    /// Git URL or local path, cloned into the bench cache.
    pub repo: String,
    /// Commit checked out before the tool runs.
    pub commit_before: String,
    /// The human fix, reference only.
    #[serde(default)]
    pub commit_fix: Option<String>,
    pub prompt: String,
    /// Optional shell command run in the worktree before the tool (deps, venv).
    #[serde(default)]
    pub setup: Option<String>,
    /// Files taken from `commit_fix` after the tool ran and before `verify`:
    /// the tests that prove the fix usually arrive with the fix itself.
    #[serde(default)]
    pub test_files_from_fix: Vec<String>,
    /// Shell command run in the worktree; exit 0 = pass.
    pub verify: String,
    #[serde(default = "default_timeout")]
    pub timeout_s: u64,
    #[serde(default = "default_max_cost")]
    pub max_cost_usd: f64,
    #[serde(default)]
    pub tags: Vec<String>,
}

fn default_timeout() -> u64 {
    600
}
fn default_max_cost() -> f64 {
    0.30
}

impl Task {
    pub fn parse(yaml: &str) -> Result<Self> {
        let t: Task = serde_yaml::from_str(yaml)?;
        if t.id.trim().is_empty() || t.repo.trim().is_empty() || t.commit_before.trim().is_empty() {
            bail!("task needs id, repo and commit_before");
        }
        if t.verify.trim().is_empty() {
            bail!("task `{}` needs a verify command", t.id);
        }
        if !t.test_files_from_fix.is_empty() && t.commit_fix.is_none() {
            bail!(
                "task `{}` lists test_files_from_fix but has no commit_fix",
                t.id
            );
        }
        Ok(t)
    }
}

/// Loads every `*.yaml` in `dir`, sorted by file name; `only` filters by id.
pub fn load_tasks(dir: &Path, only: Option<&[String]>) -> Result<Vec<Task>> {
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "yaml" || e == "yml"))
        .collect();
    paths.sort();
    let mut tasks = Vec::new();
    for p in paths {
        let text = std::fs::read_to_string(&p)?;
        let t = Task::parse(&text).with_context(|| format!("in {}", p.display()))?;
        if only.is_none_or(|ids| ids.iter().any(|i| i == &t.id)) {
            tasks.push(t);
        }
    }
    Ok(tasks)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TASK: &str = r#"
id: ago-0001
repo: https://github.com/pjcau/agent-orchestrator
commit_before: abc123
commit_fix: def456
prompt: |
  The test tests/core/test_usage.py::test_daily_budget fails.
setup: python -m venv .venv && .venv/bin/pip install -q -e .
test_files_from_fix: [tests/core/test_usage.py]
verify: .venv/bin/pytest tests/core/test_usage.py -q
tags: [python, small]
"#;

    #[test]
    fn parses_with_defaults() {
        let t = Task::parse(TASK).unwrap();
        assert_eq!(t.id, "ago-0001");
        assert_eq!(t.timeout_s, 600);
        assert_eq!(t.max_cost_usd, 0.30);
        assert_eq!(t.tags, vec!["python", "small"]);
        assert!(t.prompt.contains("test_daily_budget"));
        assert_eq!(t.test_files_from_fix, vec!["tests/core/test_usage.py"]);
        assert!(t.setup.as_deref().unwrap().starts_with("python -m venv"));
    }

    #[test]
    fn test_files_require_commit_fix() {
        assert!(Task::parse(&TASK.replace("commit_fix: def456\n", "")).is_err());
    }

    #[test]
    fn rejects_missing_verify() {
        assert!(
            Task::parse(&TASK.replace(
                "verify: .venv/bin/pytest tests/core/test_usage.py -q",
                "verify: ''"
            ))
            .is_err()
        );
    }

    #[test]
    fn loads_sorted_and_filters_by_id() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("b.yaml"),
            TASK.replace("ago-0001", "b-task"),
        )
        .unwrap();
        std::fs::write(dir.path().join("a.yaml"), TASK).unwrap();
        std::fs::write(dir.path().join("notes.txt"), "ignored").unwrap();
        let all = load_tasks(dir.path(), None).unwrap();
        assert_eq!(
            all.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            vec!["ago-0001", "b-task"]
        );
        let only = load_tasks(dir.path(), Some(&["b-task".to_string()])).unwrap();
        assert_eq!(only.len(), 1);
        assert_eq!(only[0].id, "b-task");
    }
}
