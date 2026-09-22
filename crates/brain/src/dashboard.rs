//! `brain serve`: the Phase 0 board. One axum server with the HTML page on
//! `/`, JSON on `/api/summary`, `/health`, and a background loop that runs
//! `usage snapshot` + `events ingest` every `refresh` seconds so the page
//! is current without cron.

use crate::config::Config;
use crate::db::{
    AuditRow, AutoTier, BenchRun, DailyRequests, DailyUsage, DaySpend, Db, ModelStat, RecentRow,
    SessionRow, TodayProfile,
};
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
    /// Shared with the refresh loop: reference prices and catalog age for the board.
    pub facts: Arc<std::sync::Mutex<Facts>>,
}

/// Facts the board shows that come from outside the database.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Facts {
    /// Claude Sonnet prices from the OpenRouter catalog, USD per M (input, output), for the savings tile.
    pub claude_ref: Option<(String, f64, f64)>,
    pub catalog_fetched_at: Option<String>,
    pub catalog_models: usize,
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
    /// Today per profile from the proxy itself (ring 2): live spend, rejections, errors, degradations.
    pub today: Vec<TodayProfile>,
    /// Last authentication failures / revocations.
    pub audit: Vec<AuditRow>,
    /// Anomalies computed from the proxy's own rows (runaway outputs, very slow requests, rejections).
    pub proxy_anomalies: Vec<String>,
    /// Spend per day × profile, oldest first (chart).
    pub spend: Vec<DaySpend>,
    /// Per-model quality over the window (table).
    pub models: Vec<ModelStat>,
    /// Month-to-date figures and the pace projection.
    pub month: Month,
    pub facts: Facts,
    pub version: &'static str,
    /// Authentication failures in the last hour (health strip).
    pub auth_failures_last_hour: i64,
    /// `brain/auto` over the window: per tier, what the decision model sent there
    /// and what the same tokens would have cost on the baseline tier.
    pub auto: Vec<AutoRow>,
    /// The tier `auto.baseline_cost_usd` is computed against.
    pub auto_baseline: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AutoRow {
    pub tier: String,
    pub model: String,
    pub sessions: i64,
    pub requests: i64,
    pub cost_usd: f64,
    /// Estimate from the baseline tier's prices (cache reads at 0.1× input).
    pub baseline_cost_usd: Option<f64>,
}

/// The board's per-tier view of `brain/auto`.
pub fn auto_rows(cfg: &Config, auto: &[AutoTier]) -> (Vec<AutoRow>, Option<String>) {
    let baseline = cfg
        .router
        .as_ref()
        .and_then(|r| r.baseline.clone().or_else(|| Some(r.fallback.clone())));
    let prices = baseline
        .as_deref()
        .and_then(|b| cfg.tiers.get(b))
        .map(|t| (t.input_usd_per_m, t.output_usd_per_m));
    let rows = auto
        .iter()
        .map(|a| AutoRow {
            tier: a.tier.clone(),
            model: cfg.model_for_tier(&a.tier).unwrap_or("").to_string(),
            sessions: a.sessions,
            requests: a.requests,
            cost_usd: a.cost_usd,
            baseline_cost_usd: prices.filter(|(i, o)| *i > 0.0 || *o > 0.0).map(|(i, o)| {
                (a.input_tokens as f64 + a.cache_read_tokens as f64 * 0.1) / 1e6 * i
                    + a.output_tokens as f64 / 1e6 * o
            }),
        })
        .collect();
    (rows, baseline)
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Month {
    pub spent_usd: f64,
    pub projected_usd: f64,
    pub day_of_month: u32,
    pub days_in_month: u32,
    /// Sum of the profiles' monthly soft caps.
    pub soft_cap_usd: f64,
    /// What the month's tokens would have cost on the reference Claude model, if known.
    pub claude_would_cost_usd: Option<f64>,
    pub today_usd: f64,
    pub today_claude_would_cost_usd: Option<f64>,
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

#[allow(clippy::too_many_arguments)]
pub fn summary(
    cfg: &Config,
    usage: &[DailyUsage],
    stats: &[DailyStat],
    bench: &[BenchRun],
    proxied: Vec<DailyRequests>,
    sessions: Vec<SessionRow>,
    recent: Vec<RecentRow>,
    today: Vec<TodayProfile>,
    audit: Vec<AuditRow>,
    spend: Vec<DaySpend>,
    models: Vec<ModelStat>,
    month: Month,
    facts: Facts,
    auto: Vec<AutoTier>,
) -> Summary {
    let (auto, auto_baseline) = auto_rows(cfg, &auto);
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
        proxy_anomalies: proxy_anomalies(&recent, &today),
        proxied,
        sessions,
        recent,
        today,
        audit,
        spend,
        models,
        month,
        facts,
        version: env!("CARGO_PKG_VERSION"),
        auth_failures_last_hour: 0,
        auto,
        auto_baseline,
    }
}

/// Month-to-date spend, linear projection to month end, and the Claude reference cost.
pub fn month_figures(
    cfg: &Config,
    now: chrono::DateTime<chrono::Utc>,
    month_tokens: (i64, i64, i64, f64),
    today_tokens: (i64, i64, i64, f64),
    claude_ref: Option<(f64, f64)>,
) -> Month {
    use chrono::Datelike;
    let day = now.day();
    let days_in_month = {
        let (y, m) = if now.month() == 12 {
            (now.year() + 1, 1)
        } else {
            (now.year(), now.month() + 1)
        };
        (chrono::NaiveDate::from_ymd_opt(y, m, 1).unwrap() - chrono::Duration::days(1)).day()
    };
    let would = |t: (i64, i64, i64, f64)| {
        claude_ref.map(|(i, o)| ((t.0 as f64 + t.1 as f64 * 0.1) / 1e6) * i + t.2 as f64 / 1e6 * o)
    };
    Month {
        spent_usd: month_tokens.3,
        projected_usd: month_tokens.3 / day.max(1) as f64 * days_in_month as f64,
        day_of_month: day,
        days_in_month,
        soft_cap_usd: cfg.profiles.iter().map(|p| p.monthly_soft_usd).sum(),
        claude_would_cost_usd: would(month_tokens),
        today_usd: today_tokens.3,
        today_claude_would_cost_usd: would(today_tokens),
    }
}

/// Rules on the proxy's own data. Simple on purpose, like the Phase 0 ones.
pub fn proxy_anomalies(recent: &[RecentRow], today: &[TodayProfile]) -> Vec<String> {
    let mut out = Vec::new();
    for r in recent {
        if r.output_tokens >= 20_000 {
            out.push(format!(
                "{} {} {}: {} output tokens in one request (runaway generation?)",
                &r.ts[..16],
                r.profile,
                r.model,
                r.output_tokens
            ));
        } else if r.latency_ms >= 600_000 {
            out.push(format!(
                "{} {} {}: request took {} min",
                &r.ts[..16],
                r.profile,
                r.model,
                r.latency_ms / 60_000
            ));
        }
    }
    for t in today {
        if t.rejected > 0 {
            out.push(format!(
                "today {}: {} request(s) refused by the proxy (rate limit or budget)",
                t.profile, t.rejected
            ));
        }
        if t.errors > 0 {
            out.push(format!(
                "today {}: {} upstream error(s)",
                t.profile, t.errors
            ));
        }
    }
    out
}

fn load(state: &AppState) -> anyhow::Result<Summary> {
    let db = Db::open(&state.db_path)?;
    let failures = db.auth_failures_since_minutes(60)?;
    let mut s = summary(
        &state.cfg,
        &db.daily_usage(state.days)?,
        &db.daily_stats(state.days)?,
        &db.bench_runs(None)?,
        db.daily_requests(state.days)?,
        db.sessions(state.days, 100)?,
        db.recent_requests(50)?,
        db.today_by_profile()?,
        db.recent_audit(20)?,
        db.daily_spend(30)?,
        db.model_stats(state.days)?,
        {
            let now = chrono::Utc::now();
            let facts = state.facts.lock().unwrap().clone();
            month_figures(
                &state.cfg,
                now,
                db.totals_since(crate::budget::month_start(now))?,
                db.totals_since(crate::budget::day_start(now))?,
                facts.claude_ref.as_ref().map(|(_, i, o)| (*i, *o)),
            )
        },
        state.facts.lock().unwrap().clone(),
        db.auto_routed(state.days)?,
    );
    s.auth_failures_last_hour = failures;
    Ok(s)
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
<meta name="viewport" content="width=device-width,initial-scale=1"><meta name="color-scheme" content="dark light">
<style>
:root{--bg:#0b1120;--card:#0f172a;--line:#1e293b;--fg:#e2e8f0;--mut:#94a3b8;--ok:#22c55e;--warn:#f59e0b;--bad:#ef4444;--acc:#22d3ee;--acc2:#6366f1}
@media (prefers-color-scheme: light){:root:not([data-theme=dark]){--bg:#f8fafc;--card:#ffffff;--line:#e2e8f0;--fg:#0f172a;--mut:#64748b}}
:root[data-theme=light]{--bg:#f8fafc;--card:#ffffff;--line:#e2e8f0;--fg:#0f172a;--mut:#64748b}
.kpis{display:grid;grid-template-columns:repeat(auto-fit,minmax(150px,1fr));gap:10px}
.kpi{border:1px solid var(--line);border-radius:8px;padding:10px;background:var(--bg)}.kpi .l{color:var(--mut);font-size:11px;text-transform:uppercase;letter-spacing:.04em}.kpi .v{font-size:22px;font-variant-numeric:tabular-nums;margin:2px 0}.kpi .s{color:var(--mut);font-size:12px}
.meter{height:6px;background:var(--line);border-radius:3px;overflow:hidden;margin-top:6px}.meter i{display:block;height:100%;background:var(--ok)}.meter.warn i{background:var(--warn)}.meter.bad i{background:var(--bad)}
.viz{--s1:#2a78d6;--s2:#eb6834;--s3:#1baf7a;--s4:#eda100;--s5:#e87ba4;--err:#e34948;--ref:#e34948}
@media (prefers-color-scheme: dark){:root:not([data-theme=light]) .viz{--s1:#3987e5;--s2:#d95926;--s3:#199e70;--s4:#c98500;--s5:#d55181;--err:#e66767;--ref:#e66767}}
:root[data-theme=dark] .viz{--s1:#3987e5;--s2:#d95926;--s3:#199e70;--s4:#c98500;--s5:#d55181;--err:#e66767;--ref:#e66767}
.chart{width:100%;height:220px;display:block}.chart text{font:11px system-ui,sans-serif;fill:var(--mut)}.chart .grid{stroke:var(--line);stroke-width:1}.chart .axis{stroke:var(--line)}
.chart rect{shape-rendering:crispEdges}.chart .lbl{fill:var(--fg);font-weight:600}
.legend{display:flex;gap:12px;flex-wrap:wrap;font-size:12px;color:var(--mut);margin-top:4px}.legend i{display:inline-block;width:10px;height:10px;border-radius:2px;margin-right:4px;vertical-align:-1px}
.tip{position:fixed;pointer-events:none;background:var(--card);color:var(--fg);border:1px solid var(--line);border-radius:6px;padding:6px 8px;font-size:12px;box-shadow:0 4px 16px rgba(0,0,0,.25);display:none;z-index:9;max-width:260px}
.health{display:flex;gap:14px;flex-wrap:wrap;font-size:12px;color:var(--mut)}.health b{color:var(--fg);font-weight:600}
.seg{display:inline-flex;border:1px solid var(--line);border-radius:999px;overflow:hidden}.seg button{background:none;border:0;color:var(--mut);font:inherit;font-size:11px;padding:2px 8px;cursor:pointer}.seg button.on{background:var(--line);color:var(--fg)}
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
<span class="seg" id="theme" title="theme"><button data-t="auto">auto</button><button data-t="light">light</button><button data-t="dark">dark</button></span>
<nav><a href="#kpi">month</a><a href="#budget">budget</a><a href="#spend">spend</a><a href="#models">models</a><a href="#sessions">sessions</a><a href="#proxy">proxy</a><a href="#recent">live</a><a href="#anom">anomalies</a><a href="#audit">auth</a><a href="#bench">bench</a></nav></header>
<main>
<section id="kpi"><h2>Month <small id="kpi-sub"></small></h2><div class="kpis" id="kpis"></div></section>
<section id="budget"><h2>Budget · today per profile <small id="budget-sub"></small></h2><div class="cards" id="usage"></div></section>
<section id="spend"><h2>Spend per day · by profile <small>last 30 days, USD, from the proxy</small></h2><div class="viz"><svg class="chart" id="spend-chart" role="img" aria-label="Daily spend stacked by profile"></svg><div class="legend" id="spend-legend"></div></div></section>
<section id="reqs"><h2>Requests per day · ok / upstream errors / refused</h2><div class="viz"><svg class="chart" id="req-chart" role="img" aria-label="Daily requests by outcome"></svg><div class="legend" id="req-legend"></div></div></section>
<section id="models"><h2>Models · quality and cost <small>last 7 days</small></h2><div class="tw"><table id="models-t"></table></div></section>
<section id="auto"><h2>Auto routing · brain/auto <small id="auto-sub">per-session decision, last 7 days</small></h2><div class="tw"><table id="auto-t"></table></div></section>
<section id="health"><h2>Health</h2><div class="health" id="health-l"></div></section>
<section id="sessions"><h2>Sessions · profile × user <small>last 7 days, proxy only</small></h2><div class="tw"><table id="sessions-t"></table></div></section>
<section id="proxy"><h2>Proxy · day × profile × model <small>exact, from the proxy</small></h2><div class="tw"><table id="proxied"></table></div></section>
<section id="recent"><h2>Live · last requests</h2><div class="tw"><table id="recent-t"></table></div></section>
<section id="anom"><h2>Anomalies</h2><ul id="anom-l"></ul></section>
<section id="audit"><h2>Auth failures <small>last 20 · IPs get blocked after 20 in 10 min</small></h2><div class="tw"><table id="audit-t"></table></div></section>
<section id="tools"><h2>Tool logs · day × tool × model <small>Phase 0, from aider / Claude Code files</small></h2><div class="tw"><table id="events"></table></div></section>
<section id="bench"><h2>Benchmark runs</h2><div class="tw"><table id="bench-t"></table></div></section>
</main>
<div class="tip" id="tip"></div>
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
  // --- month KPIs ---
  const m=s.month||{}, fr=s.facts||{};
  const pct=m.soft_cap_usd?Math.min(100,m.projected_usd/m.soft_cap_usd*100):0;
  const saved=m.claude_would_cost_usd!=null?m.claude_would_cost_usd-m.spent_usd:null;
  document.getElementById('kpi-sub').textContent=`day ${m.day_of_month} of ${m.days_in_month}`;
  document.getElementById('kpis').innerHTML=[
    `<div class="kpi"><div class="l">spent this month</div><div class="v">$${f(m.spent_usd,2)}</div><div class="s">soft cap $${f(m.soft_cap_usd,0)} across profiles</div></div>`,
    `<div class="kpi"><div class="l">projected month end</div><div class="v">$${f(m.projected_usd,2)}</div><div class="s">${pct.toFixed(0)}% of the soft cap at this pace</div><div class="meter ${pct>=100?'bad':pct>=70?'warn':''}"><i style="width:${pct}%"></i></div></div>`,
    `<div class="kpi"><div class="l">today</div><div class="v">$${f(m.today_usd,3)}</div><div class="s">${m.today_claude_would_cost_usd!=null?'on Claude: $'+f(m.today_claude_would_cost_usd,2):''}</div></div>`,
    `<div class="kpi"><div class="l">saved vs Claude · month</div><div class="v">${saved==null?'-':'$'+f(saved,2)}</div><div class="s">${fr.claude_ref?'same tokens on '+esc(fr.claude_ref[0].replace('anthropic/','')):'reference price unknown'}</div></div>`
  ].join('');
  // --- charts (inline SVG) ---
  const tip=document.getElementById('tip');
  const showTip=(e,html)=>{tip.innerHTML=html;tip.style.display='block';tip.style.left=Math.min(e.clientX+12,window.innerWidth-280)+'px';tip.style.top=(e.clientY+12)+'px'};
  const hideTip=()=>{tip.style.display='none'};
  const PROF=['dev','ago','benchmark','assistant','car'];
  const colorOf=p=>`var(--s${Math.max(1,PROF.indexOf(p)+1)})`;
  function stackedBars(svgId,legendId,days,series,valueOf,fmt){
    const svg=document.getElementById(svgId);const W=svg.clientWidth||800,H=220,pad={l:44,r:8,t:22,b:24};svg.setAttribute('viewBox',`0 0 ${W} ${H}`);
    const totals=days.map(d=>series.reduce((a,k)=>a+valueOf(d,k),0));const max=Math.max(1e-9,...totals);
    const iw=(W-pad.l-pad.r)/Math.max(1,days.length);const bw=Math.max(2,Math.min(28,iw-2));
    const y=v=>pad.t+(H-pad.t-pad.b)*(1-v/max);let out='';
    for(let g=0;g<=4;g++){const v=max*g/4,yy=y(v);out+=`<line class="grid" x1="${pad.l}" x2="${W-pad.r}" y1="${yy}" y2="${yy}"/><text x="${pad.l-6}" y="${yy+4}" text-anchor="end">${fmt(v)}</text>`}
    days.forEach((d,i)=>{const x=pad.l+i*iw+(iw-bw)/2;let acc=0;series.forEach((k,si)=>{const v=valueOf(d,k);if(v<=0)return;const y1=y(acc+v),y0=y(acc);const h=Math.max(0,y0-y1-2);
      out+=`<rect x="${x}" y="${y1}" width="${bw}" height="${h}" rx="${si===series.length-1?3:0}" fill="${k.color}" data-tip="${esc(d.day)} · ${esc(k.label)}: ${fmt(v)}"/>`;acc+=v});
      if((days.length<=14)||i%Math.ceil(days.length/10)===0)out+=`<text x="${x+bw/2}" y="${H-6}" text-anchor="middle">${d.day.slice(5)}</text>`;
      if(totals[i]>0&&days.length<=31)out+=`<text class="lbl" x="${x+bw/2}" y="${y(totals[i])-4}" text-anchor="middle">${days.length<=16?fmt(totals[i]):''}</text>`});
    svg.innerHTML=out;
    svg.querySelectorAll('rect').forEach(r=>{r.addEventListener('mousemove',e=>showTip(e,r.dataset.tip));r.addEventListener('mouseleave',hideTip)});
    document.getElementById(legendId).innerHTML=series.map(k=>`<span><i style="background:${k.color}"></i>${esc(k.label)}</span>`).join('');
  }
  const byDay={};(s.spend||[]).forEach(r=>{(byDay[r.day]??={});byDay[r.day][r.profile]=r});
  const days=Object.keys(byDay).sort().map(d=>({day:d,rows:byDay[d]}));
  const profs=PROF.filter(p=>days.some(d=>d.rows[p])).concat(Object.keys(Object.assign({},...days.map(d=>d.rows))).filter(p=>!PROF.includes(p)));
  if(days.length){stackedBars('spend-chart','spend-legend',days,profs.map(p=>({key:p,label:p,color:colorOf(p)})),(d,k)=>(d.rows[k.key]||{}).cost_usd||0,v=>'$'+(v<0.01?v.toFixed(4):v.toFixed(2)));
    const outcome=[{key:'ok',label:'ok',color:'var(--s1)'},{key:'errors',label:'upstream errors',color:'var(--err)'},{key:'refused',label:'refused (budget / rate)',color:'var(--s4)'}];
    stackedBars('req-chart','req-legend',days,outcome,(d,k)=>Object.values(d.rows).reduce((a,r)=>a+(k.key==='ok'?r.requests-r.errors-r.refused:r[k.key]),0),v=>Math.round(v));}
  else{document.getElementById('spend-chart').innerHTML='<text x="20" y="40">no proxied requests yet</text>';document.getElementById('req-chart').innerHTML='<text x="20" y="40">no proxied requests yet</text>'}
  // --- models ---
  document.getElementById('models-t').innerHTML='<tr><th>model</th><th class=n>req</th><th class=n>err %</th><th class=n>empty %</th><th class=n>prompt tok</th><th class=n>cache %</th><th class=n>out tok</th><th class=n>cost $</th><th class=n>$/req</th><th class=n>lat s</th><th class=n>max s</th></tr>'+
    ((s.models||[]).map(x=>{const er=x.requests?100*x.errors/x.requests:0,em=x.requests?100*x.empty_replies/x.requests:0,ch=(x.input_tokens+x.cache_read_tokens)?100*x.cache_read_tokens/(x.input_tokens+x.cache_read_tokens):0;
      return `<tr><td>${esc(x.model)}</td><td class=n>${x.requests}</td><td class="n ${er>10?'fail':''}">${er.toFixed(0)}</td><td class="n ${em>10?'warn':''}">${em.toFixed(0)}</td><td class=n>${x.input_tokens}</td><td class=n>${ch.toFixed(0)}</td><td class=n>${x.output_tokens}</td><td class=n>${f(x.cost_usd,4)}</td><td class=n>${f(x.cost_usd/Math.max(1,x.requests),5)}</td><td class=n>${x.avg_latency_ms==null?'-':(x.avg_latency_ms/1000).toFixed(1)}</td><td class=n>${x.max_latency_ms==null?'-':(x.max_latency_ms/1000).toFixed(0)}</td></tr>`}).join('')||'<tr><td class=empty colspan=11>no data yet</td></tr>');
  // --- health ---
  const age=fr.catalog_fetched_at?Math.round((Date.now()-new Date(fr.catalog_fetched_at))/60000):null;
  document.getElementById('health-l').innerHTML=[`proxy <b>v${esc(s.version)}</b>`,`catalog <b>${fr.catalog_models||0} models</b>${age==null?' (not loaded)':', '+age+' min old'}`,`auth failures last hour <b class="${s.auth_failures_last_hour?'fail':''}">${s.auth_failures_last_hour}</b>`,`refused today <b>${(s.today||[]).reduce((a,t)=>a+t.rejected,0)}</b>`,`upstream errors today <b>${(s.today||[]).reduce((a,t)=>a+t.errors,0)}</b>`].map(x=>`<span>${x}</span>`).join('');
  const todayMap=Object.fromEntries((s.today||[]).map(t=>[t.profile,t]));
  document.getElementById('usage').innerHTML=todayRows.map(u=>{const t=todayMap[u.profile]||{};const live=t.spent_usd??0;const pct=u.limit_usd?Math.max(u.pct,live/u.limit_usd*100):u.pct;return `<div class="card"><div class="p">${esc(u.profile)}</div><div class="v">$${f(live,4)} <span class="l">/ $${f(u.limit_usd,2)}</span></div><div class="l">proxy live · OpenRouter $${f(u.spent_usd)}</div><div class="l ${stateCls(u.state)}">${pct.toFixed(0)}% · ${esc(u.state)}${t.rejected?` · <span class=fail>${t.rejected} refused</span>`:''}${t.errors?` · <span class=fail>${t.errors} err</span>`:''}${t.degraded?` · <span class=warn>${t.degraded} degraded</span>`:''}</div><div class="bar ${u.state}"><i style="width:${Math.min(100,pct)}%"></i></div></div>`}).join('')||'<div class="empty">no snapshots yet</div>';
  document.getElementById('sessions-t').innerHTML='<tr><th>last</th><th>profile</th><th>user / session</th><th class=n>req</th><th class=n>err</th><th class=n>degr</th><th class=n>prompt tok</th><th class=n>cache</th><th class=n>out tok</th><th class=n>cost $</th><th class=n>lat s</th><th class="hide-sm">models</th><th class="hide-sm">first</th></tr>'+
    (s.sessions.map(x=>`<tr><td title="${esc(x.last_ts)}">${ago(x.last_ts)}</td><td>${esc(x.profile)}</td><td><span class="pill">${esc(x.user)}</span></td><td class=n>${x.requests}</td><td class="n ${x.errors?'fail':''}">${x.errors}</td><td class="n ${x.degraded?'warn':''}">${x.degraded}</td><td class=n>${x.input_tokens}</td><td class=n>${x.input_tokens?Math.round(100*x.cache_read_tokens/(x.input_tokens+x.cache_read_tokens))+'%':'-'}</td><td class=n>${x.output_tokens}</td><td class=n>${f(x.cost_usd,4)}</td><td class=n>${x.avg_latency_ms==null?'-':(x.avg_latency_ms/1000).toFixed(1)}</td><td class="hide-sm tag">${esc(x.models)}</td><td class="hide-sm">${t(x.first_ts)}</td></tr>`).join('')||'<tr><td class=empty colspan=13>no sessions yet — route a client through the proxy</td></tr>');
  document.getElementById('proxied').innerHTML='<tr><th>day</th><th>profile</th><th>model</th><th class=n>req</th><th class=n>err</th><th class=n>degr</th><th class=n>stream</th><th class=n>prompt tok</th><th class=n>cache read</th><th class=n>out tok</th><th class=n>cost $</th><th class=n>lat s</th></tr>'+
    (s.proxied.map(p=>`<tr><td>${p.day}</td><td>${esc(p.profile)}</td><td>${esc(p.model)}</td><td class=n>${p.requests}</td><td class="n ${p.errors?'fail':''}">${p.errors}</td><td class="n ${p.degraded?'warn':''}">${p.degraded}</td><td class=n>${p.streamed}</td><td class=n>${p.input_tokens}</td><td class=n>${p.cache_read_tokens}</td><td class=n>${p.output_tokens}</td><td class=n>${f(p.cost_usd,4)}</td><td class=n>${p.avg_latency_ms==null?'-':(p.avg_latency_ms/1000).toFixed(1)}</td></tr>`).join('')||'<tr><td class=empty colspan=12>no proxied requests yet</td></tr>');
  document.getElementById('recent-t').innerHTML='<tr><th>when</th><th>profile</th><th>user</th><th class="hide-sm">dialect</th><th>model</th><th class=n>status</th><th class=n>in</th><th class=n>cache</th><th class=n>out</th><th class=n>cost $</th><th class=n>lat s</th><th>notes (cap applied / fields removed / degradation)</th></tr>'+
    (s.recent.map(x=>`<tr class="${x.output_tokens>=20000||x.latency_ms>=600000?'anom':''}"><td title="${esc(x.ts)}">${t(x.ts)}</td><td>${esc(x.profile)}</td><td class="tag">${esc(x.user)}</td><td class="hide-sm tag">${esc(x.dialect)}${x.stream?' · sse':''}</td><td>${esc(x.model)}</td><td class="n ${x.status>=400?'fail':'pass'}">${x.status}</td><td class=n>${x.input_tokens}</td><td class=n>${x.cache_read_tokens}</td><td class=n>${x.output_tokens}</td><td class=n>${f(x.cost_usd,5)}</td><td class=n>${(x.latency_ms/1000).toFixed(1)}</td><td class="tag">${esc(x.degraded)}</td></tr>`).join('')||'<tr><td class=empty colspan=12>nothing yet</td></tr>');
  const anoms=[...(s.proxy_anomalies||[]).map(a=>'proxy · '+a),...s.anomalies.map(a=>'tool logs · '+a)];
  const base=s.auto_baseline;const aTot=(s.auto||[]).reduce((t,a)=>({c:t.c+a.cost_usd,b:t.b+(a.baseline_cost_usd||0),n:t.n+a.sessions}),{c:0,b:0,n:0});
  document.getElementById('auto-sub').textContent=base?`per-session decision by the decision model · last 7 days · ${aTot.n} sessions · $${f(aTot.c,3)} vs ≈ $${f(aTot.b,3)} had they all gone to ${base}`:'no router configured';
  document.getElementById('auto-t').innerHTML=`<tr><th>tier</th><th>model</th><th class=n>sessions</th><th class=n>req</th><th class=n>cost $</th><th class=n>on ${esc(base||'baseline')} ≈ $</th></tr>`+
    ((s.auto||[]).map(a=>`<tr><td>${esc(a.tier)}</td><td class="tag">${esc(a.model)}</td><td class=n>${a.sessions}</td><td class=n>${a.requests}</td><td class=n>${f(a.cost_usd,4)}</td><td class=n>${a.baseline_cost_usd==null?'-':f(a.baseline_cost_usd,4)}</td></tr>`).join('')||'<tr><td class=empty colspan=6>no brain/auto traffic yet — run px-claude</td></tr>');
  document.getElementById('anom-l').innerHTML=anoms.length?anoms.map(a=>`<li class=anom>${esc(a)}</li>`).join(''):'<li class=empty>none</li>';
  document.getElementById('audit-t').innerHTML='<tr><th>when</th><th>ip</th><th>key</th><th>outcome</th></tr>'+((s.audit||[]).map(a=>`<tr><td>${t(a.ts)}</td><td>${esc(a.ip)}</td><td class="tag">${esc(a.prefix)}</td><td class="${a.outcome==='revoked'?'warn':'fail'}">${esc(a.outcome)}</td></tr>`).join('')||'<tr><td class=empty colspan=4>no failed authentications</td></tr>');
  document.getElementById('events').innerHTML='<tr><th>day</th><th>tool</th><th>model</th><th class=n>req</th><th class=n>err</th><th class=n>bad edits</th><th class=n>retries</th><th class=n>prompt tok</th><th class=n>out tok</th><th class=n>cache</th><th class=n>est $</th><th class=n>lat s</th></tr>'+
    (s.events.map(e=>`<tr><td>${e.day}</td><td>${esc(e.tool)}</td><td>${esc(e.model)}</td><td class=n>${e.requests}</td><td class="n ${e.errors?'fail':''}">${e.errors}</td><td class=n>${e.edit_failed}</td><td class=n>${e.retries}</td><td class=n>${e.prompt_tokens}</td><td class=n>${e.output_tokens}</td><td class=n>${e.cache_hit==null?'-':(e.cache_hit*100).toFixed(0)+'%'}</td><td class=n>${f(e.est_cost_usd)}</td><td class=n>${f(e.avg_latency_s,1)}</td></tr>`).join('')||'<tr><td class=empty colspan=12>no events yet</td></tr>');
  document.getElementById('bench-t').innerHTML='<tr><th>when</th><th class="hide-sm">run</th><th>task</th><th>tool</th><th>tier</th><th>model</th><th>result</th><th class=n>s</th><th class=n>$</th><th class="hide-sm">notes</th></tr>'+
    (s.bench.map(b=>`<tr><td>${b.ts}</td><td class="hide-sm tag">${esc(b.run_id)}</td><td>${esc(b.task_id)}</td><td>${esc(b.tool)}</td><td>${esc(b.tier)}</td><td>${esc(b.model)}</td><td class="${b.passed?'pass':'fail'}">${b.passed?'PASS':'FAIL'}</td><td class=n>${b.seconds.toFixed(0)}</td><td class=n>${f(b.cost_usd)}</td><td class="hide-sm tag">${esc(b.notes)}</td></tr>`).join('')||'<tr><td class=empty colspan=10>no runs yet</td></tr>');
}
// theme: auto (system) / light / dark, remembered per browser
const applyTheme=t=>{if(t==='auto')document.documentElement.removeAttribute('data-theme');else document.documentElement.setAttribute('data-theme',t);document.querySelectorAll('#theme button').forEach(b=>b.classList.toggle('on',b.dataset.t===t));};
let theme='auto';try{theme=localStorage.getItem('brain-theme')||'auto'}catch(e){}
applyTheme(theme);
document.querySelectorAll('#theme button').forEach(b=>b.onclick=()=>{applyTheme(b.dataset.t);try{localStorage.setItem('brain-theme',b.dataset.t)}catch(e){}});
load();setInterval(load,30000);
</script></body></html>"##;

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn cfg() -> Config {
        Config::from_yaml(
            "profiles:\n  - {name: dev, tier: fast, daily_limit_usd: 3.0, monthly_soft_usd: 30.0}\n",
            "tiers:\n  fast: {model: m, input_usd_per_m: 0.04, output_usd_per_m: 0.08}\n",
        )
        .unwrap()
    }

    #[test]
    fn auto_rows_price_the_same_tokens_on_the_baseline_tier() {
        let no_router = auto_rows(&cfg(), &[]);
        assert!(no_router.0.is_empty() && no_router.1.is_none());
        let cfg = Config::from_yaml(
            "profiles:\n  - {name: dev, tier: fast, daily_limit_usd: 3.0, monthly_soft_usd: 30.0}\n",
            "tiers:\n  fast: {model: f, input_usd_per_m: 0.1, output_usd_per_m: 0.2}\n  agent: {model: a, input_usd_per_m: 1.0, output_usd_per_m: 2.0}\nrouter:\n  model: j\n  fallback: agent\n  ladder: [{tier: fast, when: x}, {tier: agent, when: y}]\n",
        )
        .unwrap();
        let rows = vec![AutoTier {
            tier: "fast".into(),
            sessions: 2,
            requests: 10,
            input_tokens: 1_000_000,
            cache_read_tokens: 10_000_000,
            output_tokens: 500_000,
            cost_usd: 0.4,
        }];
        let (out, baseline) = auto_rows(&cfg, &rows);
        assert_eq!(
            baseline.as_deref(),
            Some("agent"),
            "fallback when no baseline is set"
        );
        assert_eq!((out[0].model.as_str(), out[0].sessions), ("f", 2));
        // (1M + 10M × 0.1) × 1.0 + 0.5M × 2.0 = 2 + 1
        assert!((out[0].baseline_cost_usd.unwrap() - 3.0).abs() < 1e-9);
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
        let s = summary(
            &cfg(),
            &usage,
            &stats,
            &bench,
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            Month::default(),
            Facts::default(),
            vec![],
        );
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

    #[test]
    fn proxy_anomalies_flag_runaways_slow_requests_and_refusals() {
        let mk = |out: i64, lat: i64| RecentRow {
            ts: "2026-09-20T14:30:12+00:00".into(),
            profile: "dev".into(),
            model: "m".into(),
            output_tokens: out,
            latency_ms: lat,
            ..Default::default()
        };
        let today = vec![TodayProfile {
            profile: "car".into(),
            requests: 3,
            spent_usd: 0.0,
            rejected: 2,
            errors: 1,
            degraded: 0,
        }];
        let a = proxy_anomalies(
            &[mk(89_474, 2_137_046), mk(10, 500), mk(100, 700_000)],
            &today,
        );
        assert!(a.iter().any(|x| x.contains("89474 output tokens")), "{a:?}");
        assert!(a.iter().any(|x| x.contains("took 11 min")), "{a:?}");
        assert!(
            a.iter().any(|x| x.contains("car: 2 request(s) refused")),
            "{a:?}"
        );
        assert!(
            a.iter().any(|x| x.contains("car: 1 upstream error")),
            "{a:?}"
        );
        assert_eq!(a.len(), 4);
    }

    #[test]
    fn month_projection_and_claude_reference() {
        let now = chrono::Utc.with_ymd_and_hms(2026, 9, 20, 12, 0, 0).unwrap();
        let m = month_figures(
            &cfg(),
            now,
            (1_000_000, 500_000, 100_000, 2.0),
            (100_000, 0, 10_000, 0.2),
            Some((3.0, 15.0)),
        );
        assert_eq!((m.day_of_month, m.days_in_month), (20, 30));
        assert!(
            (m.projected_usd - 3.0).abs() < 1e-9,
            "2 $ in 20 days → 3 $ over 30"
        );
        assert_eq!(m.soft_cap_usd, 30.0);
        // Claude reference: input + 10% of cached reads at input price, output at output price
        let expected = ((1_000_000.0 + 50_000.0) / 1e6) * 3.0 + 100_000.0 / 1e6 * 15.0;
        assert!((m.claude_would_cost_usd.unwrap() - expected).abs() < 1e-9);
        assert!(
            month_figures(&cfg(), now, (0, 0, 0, 0.0), (0, 0, 0, 0.0), None)
                .claude_would_cost_usd
                .is_none()
        );
        let dec = chrono::Utc.with_ymd_and_hms(2026, 12, 5, 0, 0, 0).unwrap();
        assert_eq!(
            month_figures(&cfg(), dec, (0, 0, 0, 1.0), (0, 0, 0, 0.0), None).days_in_month,
            31
        );
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
            facts: Arc::new(std::sync::Mutex::new(Facts::default())),
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
