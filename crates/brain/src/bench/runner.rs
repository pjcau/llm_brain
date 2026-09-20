//! Runs tasks: clone/fetch the repo into a cache, add a detached worktree at
//! `commit_before`, run the tool headless in it, run `verify`, measure cost as
//! the delta of the benchmark key's `usage` (OpenRouter reports USD), record.

use super::task::Task;
use super::tool::{Endpoint, Tool, ToolInvocation, dockerize, invocation};
use crate::db::{BenchRun, Db};
use crate::openrouter::Client;
use anyhow::{Context, Result, bail};
use chrono::Utc;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::process::Command;

pub struct RunOptions {
    pub tool: Tool,
    pub tier: String,
    pub model: String,
    pub endpoint: Endpoint,
    pub cache_dir: PathBuf,
    pub run_id: String,
    /// Reads `GET /key` before and after each task to attribute cost.
    pub cost_probe: Option<Client>,
    /// Tests only: run this command instead of the tool (same cwd and env).
    pub program_override: Option<Vec<String>>,
    /// Run the tool inside this Docker image instead of on the host.
    pub docker_image: Option<String>,
}

#[derive(Debug)]
pub struct Outcome {
    pub task_id: String,
    pub passed: bool,
    pub cost_usd: Option<f64>,
    pub seconds: f64,
    pub exit_code: Option<i32>,
    pub notes: String,
}

pub async fn run_suite(opts: &RunOptions, tasks: &[Task], db: &Db) -> Result<Vec<Outcome>> {
    std::fs::create_dir_all(&opts.cache_dir)?;
    let mut outcomes = Vec::new();
    for task in tasks {
        let outcome = run_task(opts, task).await?;
        db.insert_bench_run(&BenchRun {
            ts: Utc::now(),
            run_id: opts.run_id.clone(),
            task_id: outcome.task_id.clone(),
            tool: opts.tool.as_str().into(),
            tier: opts.tier.clone(),
            model: opts.model.clone(),
            passed: outcome.passed,
            cost_usd: outcome.cost_usd,
            seconds: outcome.seconds,
            exit_code: outcome.exit_code,
            notes: outcome.notes.clone(),
        })?;
        outcomes.push(outcome);
    }
    Ok(outcomes)
}

async fn run_task(opts: &RunOptions, task: &Task) -> Result<Outcome> {
    let repo_dir = ensure_repo(&opts.cache_dir, &task.repo).await?;
    let work = tempfile::Builder::new()
        .prefix(&format!("brain-{}-", task.id))
        .tempdir()?;
    git(
        &repo_dir,
        &[
            "worktree",
            "add",
            "--detach",
            &work.path().to_string_lossy(),
            &task.commit_before,
        ],
    )
    .await?;

    if let Some(setup) = task.setup.as_deref()
        && let Some(err) = shell_err(setup, work.path(), task.timeout_s).await?
    {
        let _ = git(
            &repo_dir,
            &[
                "worktree",
                "remove",
                "--force",
                &work.path().to_string_lossy(),
            ],
        )
        .await;
        return Ok(Outcome {
            task_id: task.id.clone(),
            passed: false,
            cost_usd: None,
            seconds: 0.0,
            exit_code: None,
            notes: format!("setup failed: {err};"),
        });
    }

    let usage_before = probe(opts).await;
    let started = Instant::now();
    let mut inv = invocation(
        opts.tool,
        &opts.endpoint,
        &opts.model,
        &task.prompt,
        &opts.run_id,
    );
    if let Some(image) = opts.docker_image.as_deref() {
        let cache = std::fs::canonicalize(&opts.cache_dir)?;
        inv = dockerize(&inv, image, work.path(), &cache, &host_uid_gid());
    }
    let (exit_code, mut notes) = run_tool(
        &inv,
        opts.program_override.as_deref(),
        work.path(),
        task.timeout_s,
    )
    .await;
    let passed = match exit_code {
        Some(_) => {
            bring_test_files(work.path(), task).await?;
            shell(&task.verify, work.path(), task.timeout_s).await?
        }
        None => false,
    };
    let seconds = started.elapsed().as_secs_f64();
    let usage_after = probe(opts).await;
    let cost_usd = match (usage_before, usage_after) {
        (Some(b), Some(a)) => Some((a - b).max(0.0)),
        _ => None,
    };
    if let Some(c) = cost_usd
        && c > task.max_cost_usd
    {
        notes.push_str(&format!(" cost {c:.3} > max {:.3};", task.max_cost_usd));
    }

    // best effort cleanup; the tempdir is removed on drop anyway
    let _ = git(
        &repo_dir,
        &[
            "worktree",
            "remove",
            "--force",
            &work.path().to_string_lossy(),
        ],
    )
    .await;
    Ok(Outcome {
        task_id: task.id.clone(),
        passed,
        cost_usd,
        seconds,
        exit_code,
        notes: notes.trim().to_string(),
    })
}

async fn probe(opts: &RunOptions) -> Option<f64> {
    let client = opts.cost_probe.as_ref()?;
    client
        .key_info(&opts.endpoint.api_key)
        .await
        .ok()
        .map(|k| k.usage)
}

/// Returns (exit code or None on timeout, notes).
async fn run_tool(
    inv: &ToolInvocation,
    override_cmd: Option<&[String]>,
    cwd: &Path,
    timeout_s: u64,
) -> (Option<i32>, String) {
    let (program, args): (String, Vec<String>) = match override_cmd {
        Some(cmd) if !cmd.is_empty() => (cmd[0].clone(), cmd[1..].to_vec()),
        _ => (inv.program.clone(), inv.args.clone()),
    };
    let mut cmd = Command::new(&program);
    cmd.args(&args)
        .current_dir(cwd)
        .envs(&inv.env)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return (Some(127), format!("cannot start {program}: {e};")),
    };
    match tokio::time::timeout(Duration::from_secs(timeout_s), child.wait_with_output()).await {
        Ok(Ok(out)) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let tail: String = stderr
                .lines()
                .rev()
                .take(3)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join(" | ");
            let notes = if out.status.success() {
                String::new()
            } else {
                format!("tool stderr: {tail};")
            };
            (out.status.code(), notes)
        }
        Ok(Err(e)) => (Some(-1), format!("tool failed: {e};")),
        Err(_) => (None, format!("tool timeout after {timeout_s}s;")),
    }
}

/// `git checkout <commit_fix> -- <files>`: the fix's own tests, applied after
/// the tool ran so the model cannot read them.
async fn bring_test_files(work: &Path, task: &Task) -> Result<()> {
    let Some(fix) = task.commit_fix.as_deref() else {
        return Ok(());
    };
    if task.test_files_from_fix.is_empty() {
        return Ok(());
    }
    let mut args = vec!["checkout", fix, "--"];
    args.extend(task.test_files_from_fix.iter().map(String::as_str));
    git(work, &args)
        .await
        .context("bringing test files from commit_fix")
}

/// Runs a shell command in `cwd`; `None` on success, otherwise the reason
/// (last stderr lines, or timeout).
async fn shell_err(command: &str, cwd: &Path, timeout_s: u64) -> Result<Option<String>> {
    let fut = Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .output();
    match tokio::time::timeout(Duration::from_secs(timeout_s), fut).await {
        Ok(out) => {
            let out = out?;
            if out.status.success() {
                return Ok(None);
            }
            let stderr = String::from_utf8_lossy(&out.stderr);
            let tail = stderr
                .lines()
                .rev()
                .take(3)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join(" | ");
            Ok(Some(if tail.is_empty() {
                format!("exit {}", out.status.code().unwrap_or(-1))
            } else {
                tail
            }))
        }
        Err(_) => Ok(Some(format!("timeout after {timeout_s}s"))),
    }
}

/// Runs a shell command in `cwd`; false on non-zero exit or timeout.
async fn shell(command: &str, cwd: &Path, timeout_s: u64) -> Result<bool> {
    let fut = Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .status();
    match tokio::time::timeout(Duration::from_secs(timeout_s), fut).await {
        Ok(status) => Ok(status?.success()),
        Err(_) => Ok(false),
    }
}

/// `uid:gid` of the current user, so files the container writes stay ours.
fn host_uid_gid() -> String {
    let id = |flag: &str| {
        std::process::Command::new("id")
            .arg(flag)
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "1000".into())
    };
    format!("{}:{}", id("-u"), id("-g"))
}

/// Clone once into `cache/<slug>`, fetch afterwards.
async fn ensure_repo(cache: &Path, repo: &str) -> Result<PathBuf> {
    let slug: String = repo
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let dir = cache.join(slug);
    if dir.join(".git").exists() || dir.join("HEAD").exists() {
        git(&dir, &["fetch", "--quiet", "--all"]).await.ok();
    } else {
        let status = Command::new("git")
            .args(["clone", "--quiet", repo])
            .arg(&dir)
            .status()
            .await
            .context("git clone")?;
        if !status.success() {
            bail!("git clone of {repo} failed");
        }
    }
    Ok(dir)
}

async fn git(dir: &Path, args: &[&str]) -> Result<()> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .await
        .context("running git")?;
    if !out.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command as Std;

    /// A local repo with one commit; `verify` passes only if `fixed.txt` exists.
    fn fixture_repo() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        let run = |args: &[&str]| {
            let st = Std::new("git")
                .args(args)
                .current_dir(p)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .output()
                .unwrap();
            assert!(
                st.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&st.stderr)
            );
            String::from_utf8_lossy(&st.stdout).trim().to_string()
        };
        run(&["init", "-q", "-b", "main"]);
        std::fs::write(p.join("README.md"), "bug here\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "before"]);
        let sha = run(&["rev-parse", "HEAD"]);
        (dir, sha)
    }

    fn task(repo: &str, sha: &str, timeout_s: u64) -> Task {
        Task {
            id: "t1".into(),
            repo: repo.into(),
            commit_before: sha.into(),
            commit_fix: None,
            prompt: "add fixed.txt".into(),
            setup: None,
            test_files_from_fix: vec![],
            verify: "test -f fixed.txt".into(),
            timeout_s,
            max_cost_usd: 0.3,
            tags: vec![],
        }
    }

    fn opts(cache: &Path, program: Vec<&str>) -> RunOptions {
        RunOptions {
            tool: Tool::Aider,
            tier: "fast".into(),
            model: "m".into(),
            endpoint: Endpoint::openrouter("sk-bench"),
            cache_dir: cache.to_path_buf(),
            run_id: "run-1".into(),
            cost_probe: None,
            program_override: Some(program.into_iter().map(String::from).collect()),
            docker_image: None,
        }
    }

    #[tokio::test]
    async fn passing_tool_yields_pass_and_is_recorded() {
        let (repo, sha) = fixture_repo();
        let cache = tempfile::tempdir().unwrap();
        let db = Db::memory().unwrap();
        // the "tool" sees the tool env and creates the file the verifier wants
        let o = opts(
            cache.path(),
            vec![
                "sh",
                "-c",
                "test \"$OPENAI_API_KEY\" = sk-bench && echo done > fixed.txt",
            ],
        );
        let out = run_suite(&o, &[task(&repo.path().to_string_lossy(), &sha, 30)], &db)
            .await
            .unwrap();
        assert!(out[0].passed, "{:?}", out[0]);
        assert_eq!(out[0].exit_code, Some(0));
        let rows = db.bench_runs(Some("run-1")).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].passed);
        assert_eq!(rows[0].tool, "aider");
        // worktree was removed, cache repo remains
        assert!(cache.path().read_dir().unwrap().count() == 1);
    }

    #[tokio::test]
    async fn failing_verify_and_timeout_are_reported() {
        let (repo, sha) = fixture_repo();
        let cache = tempfile::tempdir().unwrap();
        let db = Db::memory().unwrap();
        let repo_s = repo.path().to_string_lossy().to_string();

        let noop = opts(cache.path(), vec!["sh", "-c", "true"]);
        let out = run_suite(&noop, &[task(&repo_s, &sha, 30)], &db)
            .await
            .unwrap();
        assert!(!out[0].passed);
        assert_eq!(out[0].exit_code, Some(0));

        let slow = opts(cache.path(), vec!["sh", "-c", "sleep 5"]);
        let out = run_suite(&slow, &[task(&repo_s, &sha, 1)], &db)
            .await
            .unwrap();
        assert!(!out[0].passed);
        assert_eq!(out[0].exit_code, None);
        assert!(out[0].notes.contains("timeout"), "{}", out[0].notes);

        let missing = opts(cache.path(), vec!["definitely-not-a-program-xyz"]);
        let out = run_suite(&missing, &[task(&repo_s, &sha, 5)], &db)
            .await
            .unwrap();
        assert_eq!(out[0].exit_code, Some(127));
        assert_eq!(db.bench_runs(None).unwrap().len(), 3);
    }

    #[tokio::test]
    async fn setup_runs_first_and_fix_tests_arrive_after_the_tool() {
        let (repo, sha_before) = fixture_repo();
        let p = repo.path();
        // second commit = the human fix, which also adds the test file
        std::fs::write(
            p.join("check.sh"),
            "test -f fixed.txt && test -f setup.marker\n",
        )
        .unwrap();
        let run = |args: &[&str]| {
            let st = Std::new("git")
                .args(args)
                .current_dir(p)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .output()
                .unwrap();
            assert!(
                st.status.success(),
                "{}",
                String::from_utf8_lossy(&st.stderr)
            );
            String::from_utf8_lossy(&st.stdout).trim().to_string()
        };
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "fix"]);
        let sha_fix = run(&["rev-parse", "HEAD"]);

        let cache = tempfile::tempdir().unwrap();
        let db = Db::memory().unwrap();
        let mut t = task(&p.to_string_lossy(), &sha_before, 30);
        t.commit_fix = Some(sha_fix);
        t.setup = Some("touch setup.marker".into());
        t.test_files_from_fix = vec!["check.sh".into()];
        t.verify = "sh check.sh".into();
        // the tool must NOT see check.sh (it arrives after), and must see setup.marker
        let o = opts(
            cache.path(),
            vec![
                "sh",
                "-c",
                "test ! -f check.sh && test -f setup.marker && echo done > fixed.txt",
            ],
        );
        let out = run_suite(&o, &[t.clone()], &db).await.unwrap();
        assert!(out[0].passed, "{:?}", out[0]);

        t.setup = Some("echo 'no module named venv' >&2; exit 3".into());
        let out = run_suite(&o, &[t], &db).await.unwrap();
        assert!(!out[0].passed);
        assert!(
            out[0].notes.contains("setup failed: no module named venv"),
            "{}",
            out[0].notes
        );
    }
}
