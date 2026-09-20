//! `brain` — llm_brain CLI. Phase 0 surface:
//!   brain keys provision|list      OpenRouter keys, one per profile, daily limits
//!   brain usage snapshot|report    spend per profile from GET /key, stored in SQLite
//!   brain setup claude-code|aider  client configuration for a profile
//!   brain bench run|report         the real-bug suite

mod bench;
mod config;
mod dashboard;
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
    /// The board: HTML on /, JSON on /api/summary; refreshes usage + events itself
    Serve {
        #[arg(long, default_value = "127.0.0.1:8080")]
        bind: String,
        /// Seconds between automatic `usage snapshot` + `events ingest`
        #[arg(long, default_value_t = 600)]
        refresh: u64,
        #[arg(long, default_value_t = 7)]
        days: u32,
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
        /// Print a shell function that runs Claude Code from this Docker image instead
        #[arg(long, value_name = "IMAGE")]
        docker: Option<String>,
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
        /// Where each task's tool output is saved
        #[arg(long, default_value = "bench/.runs")]
        logs: PathBuf,
    },
    /// Rows of a run (or all runs)
    Report {
        #[arg(long)]
        run: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    // .env from the cwd, else from BRAIN_HOME (the shell usually sources it anyway)
    if dotenvy::dotenv().is_err()
        && let Ok(home) = std::env::var("BRAIN_HOME")
    {
        let _ = dotenvy::from_path(std::path::Path::new(&home).join(".env"));
    }
    let cli = Cli::parse();
    let env: HashMap<String, String> = std::env::vars().collect();
    let cfg_dir = config::find_config_dir(cli.config.as_deref())?;
    let cfg = config::Config::load(&cfg_dir)?;
    let client = openrouter::Client::new(&cli.openrouter_base);
    let db_path = config::anchored(
        cli.db
            .clone()
            .or_else(|| env.get("BRAIN_DB").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("brain.db")),
    );

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
            SetupCmd::ClaudeCode {
                profile,
                docker: Some(image),
            } => {
                print!("{}", setup::claude_code_docker(&cfg, &profile, &image)?)
            }
            SetupCmd::ClaudeCode {
                profile,
                docker: None,
            } => print!("{}", setup::claude_code(&cfg, &profile)?),
            SetupCmd::Aider {
                profile,
                docker: Some(image),
            } => print!("{}", setup::aider_docker(&cfg, &profile, &image)?),
            SetupCmd::Aider {
                profile,
                docker: None,
            } => {
                let (env_block, meta) = setup::aider(&cfg, &profile)?;
                print!(
                    "{env_block}\n# .aider.model.metadata.json\n{meta}\n\n# .aider.model.settings.yml\n{}",
                    setup::aider_model_settings(&cfg)
                );
            }
        },
        Cmd::Serve {
            bind,
            refresh,
            days,
        } => {
            let state = dashboard::AppState {
                cfg: std::sync::Arc::new(cfg.clone()),
                db_path: db_path.clone(),
                days,
            };
            // refresh loop: snapshot (if keys) + ingest (if logs), errors logged, never fatal
            let (cfg_bg, env_bg, client_bg, db_bg) =
                (cfg.clone(), env.clone(), client.clone(), db_path.clone());
            tokio::spawn(async move {
                let paths = events::IngestPaths::from_env(&env_bg, None, None);
                loop {
                    // network first (no db borrow across awaits), then a short sync write
                    let fetched = usage::fetch(&cfg_bg, &env_bg, &client_bg).await;
                    match db::Db::open(&db_bg) {
                        Ok(db) => {
                            match fetched {
                                Ok((taken, _)) => {
                                    let n = taken.len();
                                    if let Err(e) =
                                        taken.iter().try_for_each(|s| db.insert_snapshot(s))
                                    {
                                        eprintln!("refresh: storing snapshots failed: {e:#}");
                                    } else {
                                        eprintln!("refresh: {n} usage snapshot(s)");
                                    }
                                }
                                Err(e) => eprintln!("refresh: usage snapshot failed: {e:#}"),
                            }
                            match events::ingest(&db, &paths) {
                                Ok(n) => eprintln!("refresh: {n} event(s) ingested"),
                                Err(e) => eprintln!("refresh: ingest failed: {e:#}"),
                            }
                        }
                        Err(e) => eprintln!("refresh: cannot open db: {e:#}"),
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(refresh.max(30))).await;
                }
            });
            let listener = tokio::net::TcpListener::bind(&bind)
                .await
                .with_context(|| format!("binding {bind}"))?;
            eprintln!("llm_brain board on http://{bind}/  (refresh every {refresh}s)");
            axum::serve(listener, dashboard::router(state)).await?;
        }
        Cmd::Events { cmd } => {
            let db = db::Db::open(&db_path)?;
            match cmd {
                EventsCmd::Ingest {
                    aider_chat,
                    claude_projects,
                } => {
                    let paths = events::IngestPaths::from_env(&env, aider_chat, claude_projects);
                    let total = events::ingest(&db, &paths)?;
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
                    logs,
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
                    let (tasks, cache, logs) = (
                        config::anchored(tasks),
                        config::anchored(cache),
                        config::anchored(logs),
                    );
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
                        log_dir: logs,
                        aider: {
                            // mirror `brain setup aider`: architect mode when running the reasoning tier,
                            // fallback chains if the settings file exists
                            let editor_model = (tier == "reasoning")
                                .then(|| cfg.model_for_tier("fast").ok().map(String::from))
                                .flatten();
                            let home = env.get("HOME").cloned().unwrap_or_default();
                            let data = env
                                .get("BRAIN_DATA")
                                .cloned()
                                .unwrap_or_else(|| format!("{home}/.local/share/llm_brain"));
                            let settings =
                                PathBuf::from(format!("{data}/aider-model-settings.yml"));
                            bench::tool::AiderOptions {
                                editor_model,
                                settings_file: settings.is_file().then_some(settings),
                            }
                        },
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
