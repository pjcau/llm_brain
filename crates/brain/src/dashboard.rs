//! `brain serve`: the Phase 0 board. One axum server with the HTML page on
//! `/`, JSON on `/api/summary`, `/health`, and a background loop that runs
//! `usage snapshot` + `events ingest` every `refresh` seconds so the page
//! is current without cron.

use crate::config::Config;
use crate::db::{BenchRun, DailyRequests, DailyUsage, Db, RecentRow, SessionRow};
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
    /// Proxied traffic (Phase 1): the precise source, from the `requests` table.
    pub proxied: Vec<DailyRequests>,
    /// profile × end-user sessions seen by the proxy, most recent first.
    pub sessions: Vec<SessionRow>,
    /// The last proxied requests, newest first.
    pub recent: Vec<RecentRow>,
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
    proxied: Vec<DailyRequests>,
    sessions: Vec<SessionRow>,
    recent: Vec<RecentRow>,
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
        proxied,
        sessions,
        recent,
    }
}

fn load(state: &AppState) -> anyhow::Result<Summary> {
    let db = Db::open(&state.db_path)?;
    Ok(summary(
        &state.cfg,
        &db.daily_usage(state.days)?,
        &db.daily_stats(state.days)?,
        &db.bench_runs(None)?,
        db.daily_requests(state.days)?,
        db.sessions(state.days, 100)?,
        db.recent_requests(50)?,
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

/// The page is static; it fetches `/api/summary` and re-renders every 30 s.
/// Responsive: cards and tables reflow below 720 px, wide tables scroll.
const PAGE: &str = r##"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><title>llm_brain board</title>
<meta name="viewport" content="width=device-width,initial-scale=1">
<style>
:root{--bg:#0b1120;--card:#0f172a;--line:#1e293b;--fg:#e2e8f0;--mut:#94a3b8;--ok:#22c55e;--warn:#f59e0b;--bad:#ef4444;--acc:#22d3ee;--acc2:#6366f1}
@media (prefers-color-scheme: light){:root{--bg:#f8fafc;--card:#ffffff;--line:#e2e8f0;--fg:#0f172a;--mut:#64748b}}
*{box-sizing:border-box}html{-webkit-text-size-adjust:100%}
body{margin:0;background:var(--bg);color:var(--fg);font:14px/1.45 system-ui,-apple-system,"Segoe UI",Roboto,sans-serif}
header{position:sticky;top:0;z-index:2;padding:10px 16px;border-bottom:1px solid var(--line);background:var(--card);display:flex;gap:12px;align-items:baseline;flex-wrap:wrap}
header h1{margin:0;font-size:17px;background:linear-gradient(90deg,var(--acc2),var(--acc));-webkit-background-clip:text;background-clip:text;color:transparent}
header span{color:var(--mut);font-size:12px}
header nav{margin-left:auto;display:flex;gap:10px;flex-wrap:wrap}header nav a{color:var(--mut);text-decoration:none;font-size:12px}header nav a:hover{color:var(--acc)}
main{padding:12px 16px 40px;display:grid;gap:14px;max-width:1500px;margin:0 auto}
section{background:var(--card);border:1px solid var(--line);border-radius:10px;padding:10px 12px;min-width:0}
h2{margin:0 0 8px;font-size:12px;color:var(--mut);text-transform:uppercase;letter-spacing:.05em;display:flex;justify-content:space-between;align-items:baseline}
h2 small{font-weight:400;text-transform:none;letter-spacing:0}
.cards{display:grid;grid-template-columns:repeat(auto-fit,minmax(170px,1fr));gap:10px}
.card{border:1px solid var(--line);border-radius:8px;padding:10px;background:var(--bg)}
.card .p{font-weight:600}.card .v{font-size:20px;font-variant-numeric:tabular-nums;margin:4px 0}.card .l{color:var(--mut);font-size:12px}
.bar{height:6px;background:var(--line);border-radius:3px;overflow:hidden;margin-top:6px}.bar i{display:block;height:100%;background:var(--ok)}
.degrade-fast i,.degrade-tokens i{background:var(--warn)}.exhausted i{background:var(--bad)}
.tw{overflow-x:auto;-webkit-overflow-scrolling:touch}
table{width:100%;border-collapse:collapse;font-variant-numeric:tabular-nums;font-size:13px;min-width:600px}
th,td{text-align:left;padding:5px 7px;border-bottom:1px solid var(--line);white-space:nowrap}th{color:var(--mut);font-weight:600;position:sticky;top:0;background:var(--card)}
td.n,th.n{text-align:right}.pass{color:var(--ok)}.fail{color:var(--bad)}.warn{color:var(--warn)}.anom{color:var(--warn)}.empty{color:var(--mut)}
.pill{display:inline-block;padding:1px 6px;border-radius:999px;font-size:11px;border:1px solid var(--line);color:var(--mut)}
.tag{font-size:11px;color:var(--mut)}
ul{margin:0;padding-left:18px}
@media (max-width:720px){main{padding:10px 10px 40px;gap:10px}section{padding:8px 10px}table{font-size:12px;min-width:560px}th,td{padding:4px 5px}.cards{grid-template-columns:repeat(2,1fr);gap:8px}.card{padding:8px}.card .v{font-size:17px}header{padding:8px 10px}.hide-sm{display:none}}
</style></head><body>
<header><h1>llm_brain</h1><span id="ts">loading…</span><span>auto-refresh 30 s</span>
<nav><a href="#budget">budget</a><a href="#sessions">sessions</a><a href="#proxy">proxy</a><a href="#recent">live</a><a href="#anom">anomalies</a><a href="#bench">bench</a></nav></header>
<main>
<section id="budget"><h2>Budget · today per profile <small id="budget-sub"></small></h2><div class="cards" id="usage"></div></section>
<section id="sessions"><h2>Sessions · profile × user <small>last 7 days, proxy only</small></h2><div class="tw"><table id="sessions-t"></table></div></section>
<section id="proxy"><h2>Proxy · day × profile × model <small>exact, from the proxy</small></h2><div class="tw"><table id="proxied"></table></div></section>
<section id="recent"><h2>Live · last requests</h2><div class="tw"><table id="recent-t"></table></div></section>
<section id="anom"><h2>Anomalies</h2><ul id="anom-l"></ul></section>
<section id="tools"><h2>Tool logs · day × tool × model <small>Phase 0, from aider / Claude Code files</small></h2><div class="tw"><table id="events"></table></div></section>
<section id="bench"><h2>Benchmark runs</h2><div class="tw"><table id="bench-t"></table></div></section>
</main>
<script>
const f=(n,d=3)=>n==null?'-':Number(n).toFixed(d);
const esc=s=>String(s??'').replace(/[&<>]/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;'}[c]));
const t=s=>s?new Date(s).toLocaleString([], {month:'2-digit',day:'2-digit',hour:'2-digit',minute:'2-digit'}):'-';
const ago=s=>{if(!s)return'-';const m=Math.round((Date.now()-new Date(s))/60000);return m<1?'now':m<60?m+' min':m<1440?Math.round(m/60)+' h':Math.round(m/1440)+' d'};
const stateCls=s=>s==='ok'?'pass':s==='exhausted'?'fail':'warn';
async function load(){
  const r=await fetch('api/summary');if(!r.ok){document.getElementById('ts').textContent='error '+r.status;return}
  const s=await r.json();
  document.getElementById('ts').textContent='updated '+new Date(s.generated_at).toLocaleTimeString();
  const today=(s.usage[0]||{}).day; document.getElementById('budget-sub').textContent=today||'';
  const todayRows=s.usage.filter(u=>u.day===today);
  document.getElementById('usage').innerHTML=todayRows.map(u=>`<div class="card"><div class="p">${esc(u.profile)}</div><div class="v">$${f(u.spent_usd)} <span class="l">/ $${f(u.limit_usd,2)}</span></div><div class="l ${stateCls(u.state)}">${u.pct.toFixed(0)}% · ${esc(u.state)}</div><div class="bar ${u.state}"><i style="width:${Math.min(100,u.pct)}%"></i></div></div>`).join('')||'<div class="empty">no snapshots yet</div>';
  document.getElementById('sessions-t').innerHTML='<tr><th>last</th><th>profile</th><th>user / session</th><th class=n>req</th><th class=n>err</th><th class=n>degr</th><th class=n>prompt tok</th><th class=n>cache</th><th class=n>out tok</th><th class=n>cost $</th><th class=n>lat s</th><th class="hide-sm">models</th><th class="hide-sm">first</th></tr>'+
    (s.sessions.map(x=>`<tr><td title="${esc(x.last_ts)}">${ago(x.last_ts)}</td><td>${esc(x.profile)}</td><td><span class="pill">${esc(x.user)}</span></td><td class=n>${x.requests}</td><td class="n ${x.errors?'fail':''}">${x.errors}</td><td class="n ${x.degraded?'warn':''}">${x.degraded}</td><td class=n>${x.input_tokens}</td><td class=n>${x.input_tokens?Math.round(100*x.cache_read_tokens/(x.input_tokens+x.cache_read_tokens))+'%':'-'}</td><td class=n>${x.output_tokens}</td><td class=n>${f(x.cost_usd,4)}</td><td class=n>${x.avg_latency_ms==null?'-':(x.avg_latency_ms/1000).toFixed(1)}</td><td class="hide-sm tag">${esc(x.models)}</td><td class="hide-sm">${t(x.first_ts)}</td></tr>`).join('')||'<tr><td class=empty colspan=13>no sessions yet — route a client through the proxy</td></tr>');
  document.getElementById('proxied').innerHTML='<tr><th>day</th><th>profile</th><th>model</th><th class=n>req</th><th class=n>err</th><th class=n>degr</th><th class=n>stream</th><th class=n>prompt tok</th><th class=n>cache read</th><th class=n>out tok</th><th class=n>cost $</th><th class=n>lat s</th></tr>'+
    (s.proxied.map(p=>`<tr><td>${p.day}</td><td>${esc(p.profile)}</td><td>${esc(p.model)}</td><td class=n>${p.requests}</td><td class="n ${p.errors?'fail':''}">${p.errors}</td><td class="n ${p.degraded?'warn':''}">${p.degraded}</td><td class=n>${p.streamed}</td><td class=n>${p.input_tokens}</td><td class=n>${p.cache_read_tokens}</td><td class=n>${p.output_tokens}</td><td class=n>${f(p.cost_usd,4)}</td><td class=n>${p.avg_latency_ms==null?'-':(p.avg_latency_ms/1000).toFixed(1)}</td></tr>`).join('')||'<tr><td class=empty colspan=12>no proxied requests yet</td></tr>');
  document.getElementById('recent-t').innerHTML='<tr><th>when</th><th>profile</th><th>user</th><th class="hide-sm">dialect</th><th>model</th><th class=n>status</th><th class=n>in</th><th class=n>cache</th><th class=n>out</th><th class=n>cost $</th><th class=n>lat s</th><th>notes</th></tr>'+
    (s.recent.map(x=>`<tr><td title="${esc(x.ts)}">${t(x.ts)}</td><td>${esc(x.profile)}</td><td class="tag">${esc(x.user)}</td><td class="hide-sm tag">${esc(x.dialect)}${x.stream?' · sse':''}</td><td>${esc(x.model)}</td><td class="n ${x.status>=400?'fail':'pass'}">${x.status}</td><td class=n>${x.input_tokens}</td><td class=n>${x.cache_read_tokens}</td><td class=n>${x.output_tokens}</td><td class=n>${f(x.cost_usd,5)}</td><td class=n>${(x.latency_ms/1000).toFixed(1)}</td><td class="tag">${esc(x.degraded)}</td></tr>`).join('')||'<tr><td class=empty colspan=12>nothing yet</td></tr>');
  document.getElementById('anom-l').innerHTML=s.anomalies.length?s.anomalies.map(a=>`<li class=anom>${esc(a)}</li>`).join(''):'<li class=empty>none</li>';
  document.getElementById('events').innerHTML='<tr><th>day</th><th>tool</th><th>model</th><th class=n>req</th><th class=n>err</th><th class=n>bad edits</th><th class=n>retries</th><th class=n>prompt tok</th><th class=n>out tok</th><th class=n>cache</th><th class=n>est $</th><th class=n>lat s</th></tr>'+
    (s.events.map(e=>`<tr><td>${e.day}</td><td>${esc(e.tool)}</td><td>${esc(e.model)}</td><td class=n>${e.requests}</td><td class="n ${e.errors?'fail':''}">${e.errors}</td><td class=n>${e.edit_failed}</td><td class=n>${e.retries}</td><td class=n>${e.prompt_tokens}</td><td class=n>${e.output_tokens}</td><td class=n>${e.cache_hit==null?'-':(e.cache_hit*100).toFixed(0)+'%'}</td><td class=n>${f(e.est_cost_usd)}</td><td class=n>${f(e.avg_latency_s,1)}</td></tr>`).join('')||'<tr><td class=empty colspan=12>no events yet</td></tr>');
  document.getElementById('bench-t').innerHTML='<tr><th>when</th><th class="hide-sm">run</th><th>task</th><th>tool</th><th>tier</th><th>model</th><th>result</th><th class=n>s</th><th class=n>$</th><th class="hide-sm">notes</th></tr>'+
    (s.bench.map(b=>`<tr><td>${b.ts}</td><td class="hide-sm tag">${esc(b.run_id)}</td><td>${esc(b.task_id)}</td><td>${esc(b.tool)}</td><td>${esc(b.tier)}</td><td>${esc(b.model)}</td><td class="${b.passed?'pass':'fail'}">${b.passed?'PASS':'FAIL'}</td><td class=n>${b.seconds.toFixed(0)}</td><td class=n>${f(b.cost_usd)}</td><td class="hide-sm tag">${esc(b.notes)}</td></tr>`).join('')||'<tr><td class=empty colspan=10>no runs yet</td></tr>');
}
load();setInterval(load,30000);
</script></body></html>"##;

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
        let s = summary(&cfg(), &usage, &stats, &bench, vec![], vec![], vec![]);
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
