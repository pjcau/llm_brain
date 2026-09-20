//! `brain` — llm_brain CLI. Phase 0 surface:
//!   brain upstream provision|sync|list  OpenRouter keys, one per profile, daily limits
//!   brain keys create|list|revoke  client keys (Phase 1): brain_<profile>_…
//!   brain usage snapshot|report    spend per profile from GET /key, stored in SQLite
//!   brain setup claude-code|aider  client configuration for a profile
//!   brain bench run|report         the real-bug suite

mod auth;
mod bench;
mod budget;
mod catalog;
mod config;
mod dashboard;
mod db;
mod events;
mod keys;
mod openrouter;
mod proxy;
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
    /// OpenRouter (upstream) keys per profile
    Upstream {
        #[command(subcommand)]
        cmd: UpstreamCmd,
    },
    /// Client keys for the proxy (brain_<profile>_…)
    Keys {
        #[command(subcommand)]
        cmd: KeysCmd,
    },
    /// Spend per profile
    Usage {
        #[command(subcommand)]
        cmd: UsageCmd,
    },
    /// Tier models with the facts OpenRouter publishes about them (context, max output, prices)
    Models,
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
    /// Issue a client key bound to a profile; printed once, stored hashed
    Create {
        #[arg(long)]
        profile: String,
        /// Who holds it: prod, staging, laptop…
        #[arg(long)]
        name: String,
        /// YYYY-MM-DD or RFC 3339
        #[arg(long)]
        expires: Option<String>,
        /// Comma-separated IPs/CIDRs allowed to use it
        #[arg(long)]
        ip: Option<String>,
    },
    /// Client keys (prefix only, never the secret)
    List {
        #[arg(long)]
        profile: Option<String>,
    },
    /// Revoke the active key `profile`/`name`
    Revoke {
        #[arg(long)]
        profile: String,
        #[arg(long)]
        name: String,
    },
}

#[derive(Subcommand)]
enum UpstreamCmd {
    /// Create one OpenRouter key per profile with its daily limit (needs OPENROUTER_MANAGEMENT_KEY)
    Provision {
        /// Recreate keys even for profiles that already have one in the env
        #[arg(long)]
        force: bool,
        /// Only these profiles
        #[arg(long, value_delimiter = ',')]
        only: Option<Vec<String>>,
    },
    /// Align the daily limits of existing keys with profiles.yaml, secrets unchanged (needs OPENROUTER_MANAGEMENT_KEY)
    Sync {
        /// Only these profiles
        #[arg(long, value_delimiter = ',')]
        only: Option<Vec<String>>,
    },
    /// List OpenRouter keys of the account (needs OPENROUTER_MANAGEMENT_KEY)
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
        /// Print the `px-claude` function (through the llm_brain proxy)
        #[arg(long)]
        proxy: bool,
    },
    /// Env block + .aider.model.metadata.json for aider
    Aider {
        #[arg(long, default_value = "dev")]
        profile: String,
        /// Print a shell function that runs aider from this Docker image instead
        #[arg(long, value_name = "IMAGE")]
        docker: Option<String>,
        /// Print the `px-aider` function (through the llm_brain proxy) and the conventions file
        #[arg(long)]
        proxy: bool,
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
        /// Go through the llm_brain proxy at this URL instead of OpenRouter directly
        /// (needs BRAIN_BENCH_KEY, a client key of the `benchmark` profile; `--model` may be brain/<tier>)
        #[arg(long, value_name = "URL")]
        via_proxy: Option<String>,
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
        Cmd::Upstream { cmd } => {
            let mgmt = env.get("OPENROUTER_MANAGEMENT_KEY").filter(|k| !k.is_empty()).context(
                "OPENROUTER_MANAGEMENT_KEY is not set (create a management key on openrouter.ai, put it in .env for this command only)",
            )?;
            match cmd {
                UpstreamCmd::Provision { force, only } => {
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
                UpstreamCmd::Sync { only } => {
                    let (changed, missing) =
                        keys::sync_limits(&cfg, &client, mgmt, only.as_deref()).await?;
                    if !missing.is_empty() {
                        eprintln!(
                            "no key on OpenRouter for: {} (run `brain upstream provision`)",
                            missing.join(", ")
                        );
                    }
                    if changed.is_empty() {
                        println!("all limits already match profiles.yaml");
                    }
                    for c in &changed {
                        println!(
                            "{}: daily limit {} → ${:.2}",
                            c.profile,
                            c.from
                                .map(|f| format!("${f:.2}"))
                                .unwrap_or_else(|| "-".into()),
                            c.to
                        );
                    }
                    eprintln!("# then remove OPENROUTER_MANAGEMENT_KEY from .env");
                }
                UpstreamCmd::List => {
                    let list = client.list_keys(mgmt).await?;
                    print!("{}", keys::render_list(&list));
                }
            }
        }
        Cmd::Keys { cmd } => {
            let db = db::Db::open(&db_path)?;
            match cmd {
                KeysCmd::Create {
                    profile,
                    name,
                    expires,
                    ip,
                } => {
                    cfg.profile(&profile)?;
                    let secret = auth::generate(&profile);
                    let key = auth::ApiKey {
                        id: 0,
                        key_hash: auth::hash(&secret),
                        prefix: auth::display_prefix(&secret),
                        profile: profile.clone(),
                        name: name.clone(),
                        ip_allow: ip.unwrap_or_default(),
                        created_at: chrono::Utc::now(),
                        expires_at: expires.as_deref().map(auth::parse_expiry).transpose()?,
                        last_used_at: None,
                        revoked_at: None,
                    };
                    db.insert_api_key(&key).with_context(|| {
                        format!("an active key named `{name}` already exists for `{profile}`")
                    })?;
                    eprintln!(
                        "# client key for profile `{profile}` ({name}). Shown once, stored hashed. Give it to that app only."
                    );
                    println!("{secret}");
                }
                KeysCmd::List { profile } => {
                    println!(
                        "{:<22} {:<10} {:<12} {:<17} {:<17} {:<17} state",
                        "prefix", "profile", "name", "created", "expires", "last used"
                    );
                    for k in db.api_keys(profile.as_deref())? {
                        let f = |d: Option<chrono::DateTime<chrono::Utc>>| {
                            d.map(|d| d.format("%Y-%m-%d %H:%M").to_string())
                                .unwrap_or_else(|| "-".into())
                        };
                        let state = if k.revoked_at.is_some() {
                            "revoked"
                        } else if k.expires_at.is_some_and(|e| e <= chrono::Utc::now()) {
                            "expired"
                        } else {
                            "active"
                        };
                        let ip = if k.ip_allow.is_empty() {
                            String::new()
                        } else {
                            format!(" ip={}", k.ip_allow)
                        };
                        println!(
                            "{:<22} {:<10} {:<12} {:<17} {:<17} {:<17} {state}{ip}",
                            k.prefix,
                            k.profile,
                            k.name,
                            f(Some(k.created_at)),
                            f(k.expires_at),
                            f(k.last_used_at)
                        );
                    }
                }
                KeysCmd::Revoke { profile, name } => {
                    if db.revoke_api_key(&profile, &name)? {
                        db.insert_audit("cli", &format!("{profile}/{name}"), "revoked")?;
                        println!(
                            "revoked {profile}/{name} (the proxy forgets cached keys within 60 s)"
                        );
                    } else {
                        bail!("no active key `{name}` for profile `{profile}`");
                    }
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
                            "no key in env for: {} (run `brain upstream provision`)",
                            missing.join(", ")
                        );
                    }
                }
                UsageCmd::Report { days } => {
                    print!("{}", usage::render_report(&cfg, &db.daily_usage(days)?))
                }
            }
        }
        Cmd::Models => {
            let cat =
                catalog::Catalog::fetch(&reqwest::Client::new(), &cli.openrouter_base).await?;
            println!(
                "{:<10} {:<36} {:>9} {:>9} {:>9} {:>8} {:>8} tools/reason",
                "tier", "model", "context", "max_out", "cap", "in$/M", "out$/M"
            );
            for (name, t) in &cfg.tiers {
                for (role, id) in [
                    ("", t.model.as_deref()),
                    ("  fallback", t.fallback.as_deref()),
                ] {
                    let Some(id) = id else { continue };
                    match cat.get(id) {
                        Some(i) => println!(
                            "{:<10} {:<36} {:>9} {:>9} {:>9} {:>8.3} {:>8.3} {}/{}",
                            format!("{name}{role}"),
                            id,
                            i.context_length
                                .map(|v| v.to_string())
                                .unwrap_or_else(|| "-".into()),
                            i.max_completion_tokens
                                .map(|v| v.to_string())
                                .unwrap_or_else(|| "-".into()),
                            catalog::output_cap(Some(i), Some(t.max_output_tokens))
                                .map(|v| v.to_string())
                                .unwrap_or_else(|| "-".into()),
                            i.prompt_usd_per_m,
                            i.completion_usd_per_m,
                            i.supports_tools,
                            i.supports_reasoning
                        ),
                        None => println!(
                            "{:<10} {:<36} not in the OpenRouter catalog",
                            format!("{name}{role}"),
                            id
                        ),
                    }
                }
            }
        }
        Cmd::Setup { cmd } => match cmd {
            SetupCmd::ClaudeCode {
                profile,
                docker: Some(image),
                ..
            } => {
                print!("{}", setup::claude_code_docker(&cfg, &profile, &image)?)
            }
            SetupCmd::ClaudeCode { proxy: true, .. } => {
                print!("{}", setup::claude_code_proxy(&cfg)?)
            }
            SetupCmd::ClaudeCode {
                profile,
                docker: None,
                ..
            } => print!("{}", setup::claude_code(&cfg, &profile)?),
            SetupCmd::Aider {
                profile,
                docker: Some(image),
                ..
            } => print!("{}", setup::aider_docker(&cfg, &profile, &image)?),
            SetupCmd::Aider { proxy: true, .. } => {
                let data = setup::data_dir();
                print!(
                    "{}\n# {data}/aider-model-settings.yml\n{}\n# {data}/aider-conventions.md\n{}",
                    setup::aider_proxy(&cfg)?,
                    setup::aider_model_settings(&cfg),
                    setup::AIDER_CONVENTIONS
                );
            }
            SetupCmd::Aider {
                profile,
                docker: None,
                ..
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
            let facts = std::sync::Arc::new(std::sync::Mutex::new(dashboard::Facts::default()));
            let state = dashboard::AppState {
                cfg: std::sync::Arc::new(cfg.clone()),
                db_path: db_path.clone(),
                days,
                facts: facts.clone(),
            };
            // refresh loop: snapshot (if keys) + ingest (if logs), errors logged, never fatal
            let (cfg_bg, env_bg, client_bg, db_bg) =
                (cfg.clone(), env.clone(), client.clone(), db_path.clone());
            let (facts_bg, base_bg) = (facts.clone(), cli.openrouter_base.clone());
            let http_bg = reqwest::Client::new();
            tokio::spawn(async move {
                let paths = events::IngestPaths::from_env(&env_bg, None, None);
                loop {
                    // catalog facts for the board: Claude reference prices and catalog age
                    match catalog::Catalog::fetch(&http_bg, &base_bg).await {
                        Ok(cat) => {
                            let claude_ref = [
                                "anthropic/claude-sonnet-4.6",
                                "anthropic/claude-sonnet-4.5",
                                "anthropic/claude-sonnet-4",
                            ]
                            .iter()
                            .find_map(|id| {
                                cat.get(id).map(|m| {
                                    (id.to_string(), m.prompt_usd_per_m, m.completion_usd_per_m)
                                })
                            })
                            .or_else(|| {
                                cat.models
                                    .values()
                                    .filter(|m| m.id.starts_with("anthropic/claude-sonnet"))
                                    .map(|m| {
                                        (m.id.clone(), m.prompt_usd_per_m, m.completion_usd_per_m)
                                    })
                                    .next()
                            });
                            let mut f = facts_bg.lock().unwrap();
                            f.claude_ref = claude_ref;
                            f.catalog_fetched_at = Some(chrono::Utc::now().to_rfc3339());
                            f.catalog_models = cat.models.len();
                        }
                        Err(e) => eprintln!("refresh: catalog failed: {e:#}"),
                    }
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
            let proxy_state = proxy::ProxyState::new(
                std::sync::Arc::new(cfg.clone()),
                db_path.clone(),
                cli.openrouter_base.clone(),
                &env,
                proxy::Limits::default(),
            );
            let app = proxy::router(proxy_state).merge(dashboard::router(state));
            let listener = tokio::net::TcpListener::bind(&bind)
                .await
                .with_context(|| format!("binding {bind}"))?;
            eprintln!(
                "llm_brain proxy + board on http://{bind}/  (refresh every {refresh}s; /v1/* needs a client key)"
            );
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .await?;
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
                    via_proxy,
                } => {
                    let p = cfg.profile(&profile)?;
                    // direct: the profile's OpenRouter key; via proxy: a client key of the benchmark profile
                    let endpoint = match via_proxy.as_deref() {
                        Some(url) => {
                            let k = env.get("BRAIN_BENCH_KEY").filter(|k| !k.is_empty()).context("BRAIN_BENCH_KEY not set (brain keys create --profile benchmark --name bench on the server)")?;
                            bench::tool::Endpoint::brain(url, k.clone())
                        }
                        None => {
                            let key = env
                                .get(&p.key_env())
                                .filter(|k| !k.is_empty())
                                .with_context(|| format!("{} not set", p.key_env()))?;
                            bench::tool::Endpoint::openrouter(key.clone())
                        }
                    };
                    let model = match (model, via_proxy.is_some()) {
                        (Some(m), _) => m,
                        (None, true) => format!("brain/{tier}"),
                        (None, false) => cfg.model_for_tier(&tier)?.to_string(),
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
                        endpoint,
                        cache_dir: cache,
                        run_id: run_id.clone(),
                        cost_probe: if via_proxy.is_some() {
                            None
                        } else {
                            Some(client.clone())
                        },
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
