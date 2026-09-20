//! `brain serve`: the Phase 0 board. One axum server with the HTML page on
//! `/`, JSON on `/api/summary`, `/health`, and a background loop that runs
//! `usage snapshot` + `events ingest` every `refresh` seconds so the page
//! is current without cron.

use crate::config::Config;
use crate::db::{BenchRun, DailyUsage, Db};
use crate::events::{DailyStat, anomalies};
use axum::{Router, extract::State, response::Html, routing::get};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<Config>,
    pub db_path: PathBuf,
    pub days: u32,
}

#[derive(Serialize)]
pub struct Summary {
    pub generated_at: String,
    pub usage: Vec<UsageRow>,
    pub events: Vec<EventRow>,
    pub bench: Vec<BenchRow>,
    pub anomalies: Vec<String>,
}

#[derive(Serialize)]
pub struct UsageRow {
    pub day: String,
    pub profile: String,
    pub spent_usd: f64,
    pub limit_usd: Option<f64>,
    pub pct: f64,
    pub state: &'static str,
}

#[derive(Serialize)]
pub struct EventRow {
    pub day: String,
    pub tool: String,
    pub model: String,
    pub requests: i64,
    pub errors: i64,
    pub edit_failed: i64,
    pub retries: i64,
    pub prompt_tokens: i64,
    pub output_tokens: i64,
    pub cache_hit: Option<f64>,
    pub est_cost_usd: Option<f64>,
    pub avg_latency_s: Option<f64>,
}

#[derive(Serialize)]
pub struct BenchRow {
    pub ts: String,
    pub run_id: String,
    pub task_id: String,
    pub tool: String,
    pub tier: String,
    pub model: String,
    pub passed: bool,
    pub seconds: f64,
    pub cost_usd: Option<f64>,
    pub notes: String,
}

pub fn usage_state(pct: f64) -> &'static str {
    match pct {
        p if p >= 100.0 => "exhausted",
        p if p >= 85.0 => "degrade-tokens",
        p if p >= 70.0 => "degrade-fast",
        _ => "ok",
    }
}

pub fn summary(
    cfg: &Config,
    usage: &[DailyUsage],
    stats: &[DailyStat],
    bench: &[BenchRun],
) -> Summary {
    let usage = usage
        .iter()
        .map(|u| {
            let limit = u.limit.unwrap_or(0.0);
            let pct = if limit > 0.0 {
                u.usage_daily / limit * 100.0
            } else {
                0.0
            };
            UsageRow {
                day: u.day.clone(),
                profile: u.profile.clone(),
                spent_usd: u.usage_daily,
                limit_usd: u.limit,
                pct,
                state: usage_state(pct),
            }
        })
        .collect();
    let events = stats
        .iter()
        .map(|s| EventRow {
            day: s.day.clone(),
            tool: s.source.clone(),
            model: s.model.clone(),
            requests: s.requests,
            errors: s.errors,
            edit_failed: s.edit_failed,
            retries: s.retries,
            prompt_tokens: s.input_tokens + s.cache_read_tokens + s.cache_write_tokens,
            output_tokens: s.output_tokens,
            cache_hit: s.cache_hit(),
            est_cost_usd: s.est_cost(cfg),
            avg_latency_s: s.avg_latency_ms.map(|l| l / 1000.0),
        })
        .collect();
    let bench = bench
        .iter()
        .rev()
        .map(|b| BenchRow {
            ts: b.ts.format("%Y-%m-%d %H:%M").to_string(),
            run_id: b.run_id.clone(),
            task_id: b.task_id.clone(),
            tool: b.tool.clone(),
            tier: b.tier.clone(),
            model: b.model.clone(),
            passed: b.passed,
            seconds: b.seconds,
            cost_usd: b.cost_usd,
            notes: b.notes.clone(),
        })
        .collect();
    Summary {
        generated_at: chrono::Utc::now().to_rfc3339(),
        usage,
        events,
        bench,
        anomalies: anomalies(stats),
    }
}

fn load(state: &AppState) -> anyhow::Result<Summary> {
    let db = Db::open(&state.db_path)?;
    Ok(summary(
        &state.cfg,
        &db.daily_usage(state.days)?,
        &db.daily_stats(state.days)?,
        &db.bench_runs(None)?,
    ))
}

async fn api_summary(State(state): State<AppState>) -> axum::response::Response {
    match load(&state) {
        Ok(s) => axum::Json(s).into_response(),
        Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
use axum::response::IntoResponse;

async fn health() -> &'static str {
    "ok"
}

async fn index() -> Html<&'static str> {
    Html(PAGE)
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/api/summary", get(api_summary))
        .route("/health", get(health))
        .with_state(state)
}

/// The page is static; it fetches `/api/summary` and re-renders every 60 s.
const PAGE: &str = r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><title>llm_brain board</title>
<meta name="viewport" content="width=device-width,initial-scale=1">
<style>
:root{--bg:#0b1120;--card:#0f172a;--line:#1e293b;--fg:#e2e8f0;--mut:#94a3b8;--ok:#22c55e;--warn:#f59e0b;--bad:#ef4444;--acc:#22d3ee}
*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--fg);font:14px/1.45 system-ui,sans-serif}
header{padding:14px 20px;border-bottom:1px solid var(--line);display:flex;gap:16px;align-items:baseline}
header h1{margin:0;font-size:18px;background:linear-gradient(90deg,#6366f1,var(--acc));-webkit-background-clip:text;background-clip:text;color:transparent}
header span{color:var(--mut);font-size:12px}
main{padding:16px 20px;display:grid;gap:16px;max-width:1400px}
section{background:var(--card);border:1px solid var(--line);border-radius:10px;padding:12px 14px}
h2{margin:0 0 8px;font-size:14px;color:var(--mut);text-transform:uppercase;letter-spacing:.04em}
table{width:100%;border-collapse:collapse;font-variant-numeric:tabular-nums}th,td{text-align:left;padding:5px 8px;border-bottom:1px solid var(--line);white-space:nowrap}
th{color:var(--mut);font-weight:600}td.n,th.n{text-align:right}
.bar{height:8px;background:var(--line);border-radius:4px;min-width:120px;overflow:hidden}.bar i{display:block;height:100%;background:var(--ok)}
.degrade-fast i{background:var(--warn)}.degrade-tokens i{background:var(--warn)}.exhausted i{background:var(--bad)}
.pass{color:var(--ok)}.fail{color:var(--bad)}.anom{color:var(--warn)}.empty{color:var(--mut)}
</style></head><body>
<header><h1>llm_brain</h1><span id="ts">loading…</span><span>auto-refresh 60 s</span></header>
<main>
<section><h2>Budget · today per profile</h2><table id="usage"></table></section>
<section><h2>Anomalies</h2><ul id="anom"></ul></section>
<section><h2>Requests · day × tool × model</h2><table id="events"></table></section>
<section><h2>Benchmark runs</h2><table id="bench"></table></section>
</main>
<script>
const f=(n,d=3)=>n==null?'-':Number(n).toFixed(d);
const esc=s=>String(s??'').replace(/[&<>]/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;'}[c]));
async function load(){
  const r=await fetch('api/summary');const s=await r.json();
  document.getElementById('ts').textContent='updated '+new Date(s.generated_at).toLocaleString();
  const today=(s.usage[0]||{}).day;
  document.getElementById('usage').innerHTML='<tr><th>profile</th><th class=n>spent $</th><th class=n>limit $</th><th>use</th><th>state</th></tr>'+
    (s.usage.filter(u=>u.day===today).map(u=>`<tr><td>${esc(u.profile)}</td><td class=n>${f(u.spent_usd)}</td><td class=n>${f(u.limit_usd,2)}</td><td><div class="bar ${u.state}"><i style="width:${Math.min(100,u.pct)}%"></i></div></td><td>${u.pct.toFixed(0)}% ${esc(u.state)}</td></tr>`).join('')||'<tr><td class=empty colspan=5>no snapshots yet</td></tr>');
  document.getElementById('anom').innerHTML=s.anomalies.length?s.anomalies.map(a=>`<li class=anom>${esc(a)}</li>`).join(''):'<li class=empty>none</li>';
  document.getElementById('events').innerHTML='<tr><th>day</th><th>tool</th><th>model</th><th class=n>req</th><th class=n>err</th><th class=n>bad edits</th><th class=n>retries</th><th class=n>prompt tok</th><th class=n>out tok</th><th class=n>cache</th><th class=n>est $</th><th class=n>lat s</th></tr>'+
    (s.events.map(e=>`<tr><td>${e.day}</td><td>${esc(e.tool)}</td><td>${esc(e.model)}</td><td class=n>${e.requests}</td><td class="n ${e.errors?'fail':''}">${e.errors}</td><td class=n>${e.edit_failed}</td><td class=n>${e.retries}</td><td class=n>${e.prompt_tokens}</td><td class=n>${e.output_tokens}</td><td class=n>${e.cache_hit==null?'-':(e.cache_hit*100).toFixed(0)+'%'}</td><td class=n>${f(e.est_cost_usd)}</td><td class=n>${f(e.avg_latency_s,1)}</td></tr>`).join('')||'<tr><td class=empty colspan=12>no events yet</td></tr>');
  document.getElementById('bench').innerHTML='<tr><th>when</th><th>run</th><th>task</th><th>tool</th><th>tier</th><th>model</th><th>result</th><th class=n>s</th><th class=n>$</th><th>notes</th></tr>'+
    (s.bench.map(b=>`<tr><td>${b.ts}</td><td>${esc(b.run_id)}</td><td>${esc(b.task_id)}</td><td>${esc(b.tool)}</td><td>${esc(b.tier)}</td><td>${esc(b.model)}</td><td class="${b.passed?'pass':'fail'}">${b.passed?'PASS':'FAIL'}</td><td class=n>${b.seconds.toFixed(0)}</td><td class=n>${f(b.cost_usd)}</td><td>${esc(b.notes)}</td></tr>`).join('')||'<tr><td class=empty colspan=10>no runs yet</td></tr>');
}
load();setInterval(load,60000);
</script></body></html>"#;

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn cfg() -> Config {
        Config::from_yaml(
            "profiles:\n  - {name: dev, tier: fast, daily_limit_usd: 3.0, monthly_soft_usd: 30.0}\n",
            "tiers:\n  fast: {model: m, input_usd_per_m: 0.04, output_usd_per_m: 0.08}\n",
        )
        .unwrap()
    }

    #[test]
    fn summary_maps_usage_states_events_and_bench() {
        let usage = vec![DailyUsage {
            day: "2026-09-20".into(),
            profile: "dev".into(),
            usage_daily: 2.4,
            limit: Some(3.0),
        }];
        let stats = vec![DailyStat {
            day: "2026-09-20".into(),
            source: "aider".into(),
            model: "m".into(),
            requests: 10,
            errors: 2,
            input_tokens: 1000,
            output_tokens: 100,
            avg_latency_ms: Some(2500.0),
            ..Default::default()
        }];
        let bench = vec![BenchRun {
            ts: Utc::now(),
            run_id: "r".into(),
            task_id: "t".into(),
            tool: "aider".into(),
            tier: "fast".into(),
            model: "m".into(),
            passed: true,
            cost_usd: Some(0.004),
            seconds: 260.0,
            exit_code: Some(0),
            notes: String::new(),
        }];
        let s = summary(&cfg(), &usage, &stats, &bench);
        assert_eq!(s.usage[0].state, "degrade-fast");
        assert!((s.usage[0].pct - 80.0).abs() < 1e-9);
        assert_eq!(s.events[0].requests, 10);
        assert_eq!(s.events[0].avg_latency_s, Some(2.5));
        assert!(s.events[0].est_cost_usd.is_some());
        assert!(s.bench[0].passed);
        assert!(
            s.anomalies.iter().any(|a| a.contains("error rate 20%")),
            "{:?}",
            s.anomalies
        );
        assert_eq!(usage_state(100.0), "exhausted");
        assert_eq!(usage_state(90.0), "degrade-tokens");
        assert_eq!(usage_state(10.0), "ok");
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"spent_usd\":2.4"));
    }

    #[tokio::test]
    async fn server_serves_page_health_and_json() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("b.db");
        Db::open(&db_path).unwrap();
        let state = AppState {
            cfg: Arc::new(cfg()),
            db_path,
            days: 7,
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router(state)).await.unwrap() });
        let base = format!("http://{addr}");
        let http = reqwest::Client::new();
        assert_eq!(
            http.get(format!("{base}/health"))
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
            "ok"
        );
        let page = http
            .get(format!("{base}/"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(page.contains("llm_brain board"));
        let s: serde_json::Value = http
            .get(format!("{base}/api/summary"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(s["usage"].as_array().unwrap().is_empty());
        assert!(s["anomalies"].as_array().unwrap().is_empty());
    }
}
