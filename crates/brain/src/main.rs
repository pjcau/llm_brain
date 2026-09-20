//! `brain` — llm_brain CLI. Phase 0 surface:
//!   brain keys provision|list      OpenRouter keys, one per profile, daily limits
//!   brain usage snapshot|report    spend per profile from GET /key, stored in SQLite
//!   brain setup claude-code|aider  client configuration for a profile
//!   brain bench run|report         the real-bug suite

mod bench;
mod config;
mod db;
mod events;
mod keys;
mod openrouter;
mod setup;
mod usage;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "brain", version, about = "llm_brain CLI (Phase 0)")]
struct Cli {
    /// Directory with profiles.yaml and tiers.yaml (default: nearest ./config)
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    /// SQLite file (default: $BRAIN_DB or ./brain.db)
    #[arg(long, global = true)]
    db: Option<PathBuf>,
    /// OpenRouter API base (tests point it at a mock)
    #[arg(long, global = true, default_value = openrouter::DEFAULT_BASE_URL, hide = true)]
    openrouter_base: String,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// OpenRouter keys per profile
    Keys {
        #[command(subcommand)]
        cmd: KeysCmd,
    },
    /// Spend per profile
    Usage {
        #[command(subcommand)]
        cmd: UsageCmd,
    },
    /// Print client configuration
    Setup {
        #[command(subcommand)]
        cmd: SetupCmd,
    },
    /// Errors, tokens and anomalies from the tools' own logs
    Events {
        #[command(subcommand)]
        cmd: EventsCmd,
    },
    /// Benchmark suite
    Bench {
        #[command(subcommand)]
        cmd: BenchCmd,
    },
}

#[derive(Subcommand)]
enum KeysCmd {
    /// Create one key per profile with its daily limit (needs OPENROUTER_MANAGEMENT_KEY)
    Provision {
        /// Recreate keys even for profiles that already have one in the env
        #[arg(long)]
        force: bool,
        /// Only these profiles
        #[arg(long, value_delimiter = ',')]
        only: Option<Vec<String>>,
    },
    /// List keys of the account (needs OPENROUTER_MANAGEMENT_KEY)
    List,
}

#[derive(Subcommand)]
enum UsageCmd {
    /// Read GET /key for every profile with a key and store a snapshot
    Snapshot,
    /// Day × profile spend against limits
    Report {
        #[arg(long, default_value_t = 7)]
        days: u32,
    },
}

#[derive(Subcommand)]
enum SetupCmd {
    /// Env block for Claude Code
    ClaudeCode {
        #[arg(long, default_value = "dev")]
        profile: String,
    },
    /// Env block + .aider.model.metadata.json for aider
    Aider {
        #[arg(long, default_value = "dev")]
        profile: String,
        /// Print a shell function that runs aider from this Docker image instead
        #[arg(long, value_name = "IMAGE")]
        docker: Option<String>,
    },
}

#[derive(Subcommand)]
enum EventsCmd {
    /// Read new lines from aider's chat history and Claude Code's session files
    Ingest {
        /// aider chat history (default: $BRAIN_DATA/aider-chat.md)
        #[arg(long)]
        aider_chat: Option<PathBuf>,
        /// Claude Code projects dir (default: ~/.claude/projects)
        #[arg(long)]
        claude_projects: Option<PathBuf>,
    },
    /// Day × tool × model: requests, errors, tokens, cache hit, anomalies
    Report {
        #[arg(long, default_value_t = 7)]
        days: u32,
    },
}

#[derive(Subcommand)]
enum BenchCmd {
    /// Run tasks with a tool against a tier
    Run {
        #[arg(long, value_enum)]
        tool: bench::Tool,
        #[arg(long, default_value = "fast")]
        tier: String,
        /// Override the tier's model
        #[arg(long)]
        model: Option<String>,
        #[arg(long, default_value = "bench/tasks")]
        tasks: PathBuf,
        #[arg(long, default_value = "bench/.cache")]
        cache: PathBuf,
        /// Only these task ids
        #[arg(long, value_delimiter = ',')]
        only: Option<Vec<String>>,
        /// Profile whose key pays (and whose usage delta is the cost)
        #[arg(long, default_value = "benchmark")]
        profile: String,
        /// Run the tool inside this Docker image (e.g. llm-brain-aider-test:latest)
        #[arg(long)]
        docker: Option<String>,
    },
    /// Rows of a run (or all runs)
    Report {
        #[arg(long)]
        run: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();
    let cli = Cli::parse();
    let env: HashMap<String, String> = std::env::vars().collect();
    let cfg_dir = config::find_config_dir(cli.config.as_deref())?;
    let cfg = config::Config::load(&cfg_dir)?;
    let client = openrouter::Client::new(&cli.openrouter_base);
    let db_path = cli
        .db
        .clone()
        .or_else(|| env.get("BRAIN_DB").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("brain.db"));

    match cli.cmd {
        Cmd::Keys { cmd } => {
            let mgmt = env.get("OPENROUTER_MANAGEMENT_KEY").filter(|k| !k.is_empty()).context(
                "OPENROUTER_MANAGEMENT_KEY is not set (create a management key on openrouter.ai, put it in .env for this command only)",
            )?;
            match cmd {
                KeysCmd::Provision { force, only } => {
                    let (created, skipped) =
                        keys::provision(&cfg, &env, &client, mgmt, force, only.as_deref()).await?;
                    if !skipped.is_empty() {
                        eprintln!(
                            "skipped (key already in env, use --force to recreate): {}",
                            skipped.join(", ")
                        );
                    }
                    if created.is_empty() {
                        eprintln!("nothing to provision");
                    } else {
                        eprintln!(
                            "# paste into .env (chmod 600). Shown once, not stored anywhere else."
                        );
                        for c in &created {
                            eprintln!("# {} — daily limit ${:.2}", c.profile, c.limit);
                            println!("{}", c.env_line);
                        }
                        eprintln!("# then remove OPENROUTER_MANAGEMENT_KEY from .env");
                    }
                }
                KeysCmd::List => {
                    let list = client.list_keys(mgmt).await?;
                    print!("{}", keys::render_list(&list));
                }
            }
        }
        Cmd::Usage { cmd } => {
            let db = db::Db::open(&db_path)?;
            match cmd {
                UsageCmd::Snapshot => {
                    let (taken, missing) = usage::snapshot(&cfg, &env, &client, &db).await?;
                    for s in &taken {
                        println!(
                            "{:<10} today ${:.3} / ${:.2}  month ${:.3}",
                            s.profile,
                            s.usage_daily,
                            s.limit.unwrap_or(0.0),
                            s.usage_monthly
                        );
                    }
                    if !missing.is_empty() {
                        eprintln!(
                            "no key in env for: {} (run `brain keys provision`)",
                            missing.join(", ")
                        );
                    }
                }
                UsageCmd::Report { days } => {
                    print!("{}", usage::render_report(&cfg, &db.daily_usage(days)?))
                }
            }
        }
        Cmd::Setup { cmd } => match cmd {
            SetupCmd::ClaudeCode { profile } => print!("{}", setup::claude_code(&cfg, &profile)?),
            SetupCmd::Aider {
                profile,
                docker: Some(image),
            } => print!("{}", setup::aider_docker(&cfg, &profile, &image)?),
            SetupCmd::Aider {
                profile,
                docker: None,
            } => {
                let (env_block, meta) = setup::aider(&cfg, &profile)?;
                print!("{env_block}\n# .aider.model.metadata.json\n{meta}\n");
            }
        },
        Cmd::Events { cmd } => {
            let db = db::Db::open(&db_path)?;
            match cmd {
                EventsCmd::Ingest {
                    aider_chat,
                    claude_projects,
                } => {
                    let home = env.get("HOME").cloned().unwrap_or_default();
                    let data = env
                        .get("BRAIN_DATA")
                        .cloned()
                        .unwrap_or_else(|| format!("{home}/.local/share/llm_brain"));
                    let aider_chat = aider_chat
                        .unwrap_or_else(|| PathBuf::from(format!("{data}/aider-chat.md")));
                    let claude_projects = claude_projects
                        .unwrap_or_else(|| PathBuf::from(format!("{home}/.claude/projects")));
                    let mut total = 0;
                    if aider_chat.is_file() {
                        let key = aider_chat.to_string_lossy().to_string();
                        let (offset, hint) = db.ingest_state(&key)?;
                        let (text, new_offset) = events::read_from(&aider_chat, offset)?;
                        let (ev, model) = events::parse_aider_chat(&text, &hint);
                        total += db.insert_events(&ev)?;
                        db.set_ingest_state(&key, new_offset, &model)?;
                    } else {
                        eprintln!(
                            "no aider chat history at {} (use `brain setup aider`)",
                            aider_chat.display()
                        );
                    }
                    let mut sessions = Vec::new();
                    if claude_projects.is_dir() {
                        for project in std::fs::read_dir(&claude_projects)?.flatten() {
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
                        let (text, new_offset) = events::read_from(&path, offset)?;
                        if new_offset == offset {
                            continue;
                        }
                        total += db.insert_events(&events::parse_claude_session(&text))?;
                        db.set_ingest_state(&key, new_offset, "")?;
                    }
                    println!("ingested {total} event(s)");
                }
                EventsCmd::Report { days } => {
                    print!("{}", events::render_report(&cfg, &db.daily_stats(days)?))
                }
            }
        }
        Cmd::Bench { cmd } => {
            let db = db::Db::open(&db_path)?;
            match cmd {
                BenchCmd::Run {
                    tool,
                    tier,
                    model,
                    tasks,
                    cache,
                    only,
                    profile,
                    docker,
                } => {
                    let p = cfg.profile(&profile)?;
                    let key = env
                        .get(&p.key_env())
                        .filter(|k| !k.is_empty())
                        .with_context(|| format!("{} not set", p.key_env()))?;
                    let model = match model {
                        Some(m) => m,
                        None => cfg.model_for_tier(&tier)?.to_string(),
                    };
                    let list = bench::load_tasks(&tasks, only.as_deref())?;
                    if list.is_empty() {
                        bail!("no tasks in {}", tasks.display());
                    }
                    let run_id = format!(
                        "{}-{}-{}",
                        chrono::Utc::now().format("%Y%m%dT%H%M%S"),
                        tool.as_str(),
                        tier
                    );
                    let opts = bench::RunOptions {
                        tool,
                        tier: tier.clone(),
                        model: model.clone(),
                        endpoint: bench::tool::Endpoint::openrouter(key.clone()),
                        cache_dir: cache,
                        run_id: run_id.clone(),
                        cost_probe: Some(client.clone()),
                        program_override: None,
                        docker_image: docker,
                    };
                    eprintln!(
                        "run {run_id}: {} task(s), {} on {}",
                        list.len(),
                        tool.as_str(),
                        model
                    );
                    let outcomes = bench::run_suite(&opts, &list, &db).await?;
                    let passed = outcomes.iter().filter(|o| o.passed).count();
                    for o in &outcomes {
                        println!(
                            "{:<12} {} {:>6.1}s {} {}",
                            o.task_id,
                            if o.passed { "PASS" } else { "FAIL" },
                            o.seconds,
                            o.cost_usd
                                .map(|c| format!("${c:.3}"))
                                .unwrap_or_else(|| "$?".into()),
                            o.notes
                        );
                    }
                    println!("{passed}/{} passed — run {run_id}", outcomes.len());
                }
                BenchCmd::Report { run } => {
                    for r in db.bench_runs(run.as_deref())? {
                        println!(
                            "{} {:<28} {:<12} {:<6} {:<9} {} {:>6.1}s {}",
                            r.ts.format("%Y-%m-%d %H:%M"),
                            r.run_id,
                            r.task_id,
                            r.tool,
                            r.tier,
                            if r.passed { "PASS" } else { "FAIL" },
                            r.seconds,
                            r.cost_usd
                                .map(|c| format!("${c:.3}"))
                                .unwrap_or_else(|| "$?".into())
                        );
                    }
                }
            }
        }
    }
    Ok(())
}
