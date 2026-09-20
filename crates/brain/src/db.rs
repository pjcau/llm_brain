//! SQLite persistence for Phase 0: usage snapshots per profile and benchmark
//! runs. One file, WAL mode; `Db::memory()` for tests.

use crate::auth::ApiKey;
use crate::events::{DailyStat, Event};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};
use std::path::Path;

pub struct Db {
    conn: Connection,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub ts: DateTime<Utc>,
    pub profile: String,
    pub usage_total: f64,
    pub usage_daily: f64,
    pub usage_weekly: f64,
    pub usage_monthly: f64,
    pub limit: Option<f64>,
    pub limit_remaining: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BenchRun {
    pub ts: DateTime<Utc>,
    pub run_id: String,
    pub task_id: String,
    pub tool: String,
    pub tier: String,
    pub model: String,
    pub passed: bool,
    pub cost_usd: Option<f64>,
    pub seconds: f64,
    pub exit_code: Option<i32>,
    pub notes: String,
}

/// One proxied request, as recorded by the Phase 1 proxy.
#[derive(Debug, Clone, PartialEq)]
pub struct RequestRow {
    pub ts: DateTime<Utc>,
    pub profile: String,
    pub key_prefix: String,
    pub user: String,
    /// "openai" | "anthropic"
    pub dialect: String,
    pub tier: String,
    pub model: String,
    pub status: i64,
    pub input_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub output_tokens: i64,
    pub cost_usd: f64,
    pub latency_ms: i64,
    pub stream: bool,
    /// "" | "fast-only" | "max-tokens" | "blocked"
    pub degraded: String,
}

/// One end-user session as seen by the proxy: the `user` field (an app's
/// opaque id, or Claude Code's `cc:<session>`), grouped per profile.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
pub struct SessionRow {
    pub profile: String,
    pub user: String,
    pub first_ts: String,
    pub last_ts: String,
    pub requests: i64,
    pub errors: i64,
    pub input_tokens: i64,
    pub cache_read_tokens: i64,
    pub output_tokens: i64,
    pub cost_usd: f64,
    pub avg_latency_ms: Option<f64>,
    pub models: String,
    pub degraded: i64,
}

/// Today's proxy-side figures per profile (ring 2, live).
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
pub struct TodayProfile {
    pub profile: String,
    pub requests: i64,
    pub spent_usd: f64,
    pub rejected: i64,
    pub errors: i64,
    pub degraded: i64,
}

#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
pub struct AuditRow {
    pub ts: String,
    pub ip: String,
    pub prefix: String,
    pub outcome: String,
}

/// One proxied request for the live feed.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
pub struct RecentRow {
    pub ts: String,
    pub profile: String,
    pub user: String,
    pub dialect: String,
    pub model: String,
    pub status: i64,
    pub input_tokens: i64,
    pub cache_read_tokens: i64,
    pub output_tokens: i64,
    pub cost_usd: f64,
    pub latency_ms: i64,
    pub stream: bool,
    pub degraded: String,
}

/// Aggregated proxied traffic for the board.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
pub struct DailyRequests {
    pub day: String,
    pub profile: String,
    pub model: String,
    pub requests: i64,
    pub errors: i64,
    pub input_tokens: i64,
    pub cache_read_tokens: i64,
    pub output_tokens: i64,
    pub cost_usd: f64,
    pub avg_latency_ms: Option<f64>,
    pub degraded: i64,
    pub streamed: i64,
}

/// One row of `brain usage report`: the max daily usage seen per (day, profile).
#[derive(Debug, Clone, PartialEq)]
pub struct DailyUsage {
    pub day: String,
    pub profile: String,
    pub usage_daily: f64,
    pub limit: Option<f64>,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS usage_snapshots (
  id INTEGER PRIMARY KEY,
  ts TEXT NOT NULL,
  profile TEXT NOT NULL,
  usage_total REAL NOT NULL,
  usage_daily REAL NOT NULL,
  usage_weekly REAL NOT NULL,
  usage_monthly REAL NOT NULL,
  "limit" REAL,
  limit_remaining REAL
);
CREATE INDEX IF NOT EXISTS usage_snapshots_profile_ts ON usage_snapshots(profile, ts);

CREATE TABLE IF NOT EXISTS bench_runs (
  id INTEGER PRIMARY KEY,
  ts TEXT NOT NULL,
  run_id TEXT NOT NULL,
  task_id TEXT NOT NULL,
  tool TEXT NOT NULL,
  tier TEXT NOT NULL,
  model TEXT NOT NULL,
  passed INTEGER NOT NULL,
  cost_usd REAL,
  seconds REAL NOT NULL,
  exit_code INTEGER,
  notes TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS bench_runs_run ON bench_runs(run_id);

CREATE TABLE IF NOT EXISTS events (
  id INTEGER PRIMARY KEY,
  ts TEXT NOT NULL,
  source TEXT NOT NULL,
  session TEXT NOT NULL,
  model TEXT NOT NULL,
  kind TEXT NOT NULL,
  input_tokens INTEGER NOT NULL,
  cache_read_tokens INTEGER NOT NULL,
  cache_write_tokens INTEGER NOT NULL,
  output_tokens INTEGER NOT NULL,
  detail TEXT NOT NULL DEFAULT '',
  latency_ms INTEGER
);
CREATE INDEX IF NOT EXISTS events_ts ON events(ts);

-- append-only files already read up to `offset`
CREATE TABLE IF NOT EXISTS ingest_files (
  path TEXT PRIMARY KEY,
  offset INTEGER NOT NULL,
  model_hint TEXT NOT NULL DEFAULT ''
);

-- client keys (Phase 1): sha256 only, never the key
CREATE TABLE IF NOT EXISTS api_keys (
  id INTEGER PRIMARY KEY,
  key_hash TEXT UNIQUE NOT NULL,
  prefix TEXT NOT NULL,
  profile TEXT NOT NULL,
  name TEXT NOT NULL,
  ip_allow TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL,
  expires_at TEXT,
  last_used_at TEXT,
  revoked_at TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS api_keys_profile_name_active ON api_keys(profile, name) WHERE revoked_at IS NULL;

-- authentication failures and revocations
CREATE TABLE IF NOT EXISTS auth_audit (
  id INTEGER PRIMARY KEY,
  ts TEXT NOT NULL,
  ip TEXT NOT NULL,
  prefix TEXT NOT NULL,
  outcome TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS auth_audit_ts ON auth_audit(ts);

-- one row per proxied request (Phase 1): the source of budget and the board
CREATE TABLE IF NOT EXISTS requests (
  id INTEGER PRIMARY KEY,
  ts TEXT NOT NULL,
  profile TEXT NOT NULL,
  key_prefix TEXT NOT NULL,
  user TEXT NOT NULL DEFAULT '',
  dialect TEXT NOT NULL,
  tier TEXT NOT NULL,
  model TEXT NOT NULL,
  status INTEGER NOT NULL,
  input_tokens INTEGER NOT NULL DEFAULT 0,
  cache_read_tokens INTEGER NOT NULL DEFAULT 0,
  cache_write_tokens INTEGER NOT NULL DEFAULT 0,
  output_tokens INTEGER NOT NULL DEFAULT 0,
  cost_usd REAL NOT NULL DEFAULT 0,
  latency_ms INTEGER NOT NULL DEFAULT 0,
  stream INTEGER NOT NULL DEFAULT 0,
  degraded TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS requests_profile_ts ON requests(profile, ts);
"#;

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Self::init(conn)
    }

    #[cfg(test)]
    pub fn memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch(SCHEMA).context("applying schema")?;
        // migration for databases created before latency_ms existed
        let has_latency: bool = conn
            .prepare("SELECT 1 FROM pragma_table_info('events') WHERE name = 'latency_ms'")?
            .exists([])?;
        if !has_latency {
            conn.execute_batch("ALTER TABLE events ADD COLUMN latency_ms INTEGER")?;
        }
        Ok(Self { conn })
    }

    pub fn insert_snapshot(&self, s: &Snapshot) -> Result<()> {
        self.conn.execute(
            r#"INSERT INTO usage_snapshots (ts, profile, usage_total, usage_daily, usage_weekly, usage_monthly, "limit", limit_remaining)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"#,
            params![
                s.ts.to_rfc3339(),
                s.profile,
                s.usage_total,
                s.usage_daily,
                s.usage_weekly,
                s.usage_monthly,
                s.limit,
                s.limit_remaining
            ],
        )?;
        Ok(())
    }

    /// Per (UTC day, profile): the highest `usage_daily` recorded that day.
    /// OpenRouter resets `usage_daily` at midnight UTC, so the max is the day's spend.
    pub fn daily_usage(&self, days: u32) -> Result<Vec<DailyUsage>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT substr(ts, 1, 10) AS day, profile, MAX(usage_daily), MAX("limit")
               FROM usage_snapshots
               WHERE ts >= datetime('now', ?1)
               GROUP BY day, profile
               ORDER BY day DESC, profile"#,
        )?;
        let rows = stmt.query_map(params![format!("-{days} days")], |r| {
            Ok(DailyUsage {
                day: r.get(0)?,
                profile: r.get(1)?,
                usage_daily: r.get(2)?,
                limit: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn insert_events(&self, events: &[Event]) -> Result<usize> {
        let mut stmt = self.conn.prepare_cached(
            "INSERT INTO events (ts, source, session, model, kind, input_tokens, cache_read_tokens, cache_write_tokens, output_tokens, detail, latency_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        )?;
        for e in events {
            stmt.execute(params![
                e.ts.to_rfc3339(),
                e.source,
                e.session,
                e.model,
                e.kind,
                e.input_tokens,
                e.cache_read_tokens,
                e.cache_write_tokens,
                e.output_tokens,
                e.detail,
                e.latency_ms
            ])?;
        }
        Ok(events.len())
    }

    /// Per UTC day × source × model over the last `days`.
    pub fn daily_stats(&self, days: u32) -> Result<Vec<DailyStat>> {
        let mut stmt = self.conn.prepare(
            "SELECT substr(ts, 1, 10) AS day, source, model,
                    SUM(kind = 'request'), SUM(kind = 'error'), SUM(kind = 'edit_failed'), SUM(kind = 'retry'),
                    SUM(input_tokens), SUM(cache_read_tokens), SUM(cache_write_tokens), SUM(output_tokens),
                    AVG(CASE WHEN kind = 'request' THEN latency_ms END)
             FROM events WHERE ts >= datetime('now', ?1)
             GROUP BY day, source, model ORDER BY day DESC, source, model",
        )?;
        let rows = stmt.query_map(params![format!("-{days} days")], |r| {
            Ok(DailyStat {
                day: r.get(0)?,
                source: r.get(1)?,
                model: r.get(2)?,
                requests: r.get(3)?,
                errors: r.get(4)?,
                edit_failed: r.get(5)?,
                retries: r.get(6)?,
                input_tokens: r.get(7)?,
                cache_read_tokens: r.get(8)?,
                cache_write_tokens: r.get(9)?,
                output_tokens: r.get(10)?,
                avg_latency_ms: r.get(11)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// (offset, model_hint) of an ingested file; (0, "") if never seen.
    pub fn ingest_state(&self, path: &str) -> Result<(u64, String)> {
        let mut stmt = self
            .conn
            .prepare("SELECT offset, model_hint FROM ingest_files WHERE path = ?1")?;
        let mut rows = stmt.query(params![path])?;
        match rows.next()? {
            Some(r) => Ok((r.get::<_, i64>(0)? as u64, r.get(1)?)),
            None => Ok((0, String::new())),
        }
    }

    pub fn set_ingest_state(&self, path: &str, offset: u64, model_hint: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO ingest_files (path, offset, model_hint) VALUES (?1, ?2, ?3)
             ON CONFLICT(path) DO UPDATE SET offset = excluded.offset, model_hint = excluded.model_hint",
            params![path, offset as i64, model_hint],
        )?;
        Ok(())
    }

    // ----- client keys -----------------------------------------------------

    pub fn insert_api_key(&self, k: &ApiKey) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO api_keys (key_hash, prefix, profile, name, ip_allow, created_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                k.key_hash,
                k.prefix,
                k.profile,
                k.name,
                k.ip_allow,
                k.created_at.to_rfc3339(),
                k.expires_at.map(|d| d.to_rfc3339())
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    fn row_to_key(r: &rusqlite::Row<'_>) -> rusqlite::Result<ApiKey> {
        let ts = |s: Option<String>| {
            s.and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                .map(|d| d.with_timezone(&Utc))
        };
        Ok(ApiKey {
            id: r.get(0)?,
            key_hash: r.get(1)?,
            prefix: r.get(2)?,
            profile: r.get(3)?,
            name: r.get(4)?,
            ip_allow: r.get(5)?,
            created_at: ts(Some(r.get::<_, String>(6)?)).unwrap_or_else(Utc::now),
            expires_at: ts(r.get(7)?),
            last_used_at: ts(r.get(8)?),
            revoked_at: ts(r.get(9)?),
        })
    }

    const KEY_COLS: &'static str = "id, key_hash, prefix, profile, name, ip_allow, created_at, expires_at, last_used_at, revoked_at";

    pub fn api_key_by_hash(&self, key_hash: &str) -> Result<Option<ApiKey>> {
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT {} FROM api_keys WHERE key_hash = ?1",
            Self::KEY_COLS
        ))?;
        let mut rows = stmt.query(params![key_hash])?;
        match rows.next()? {
            Some(r) => Ok(Some(Self::row_to_key(r)?)),
            None => Ok(None),
        }
    }

    pub fn api_keys(&self, profile: Option<&str>) -> Result<Vec<ApiKey>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {} FROM api_keys WHERE (?1 IS NULL OR profile = ?1) ORDER BY profile, created_at",
            Self::KEY_COLS
        ))?;
        let rows = stmt.query_map(params![profile], Self::row_to_key)?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// Marks the active key `profile`/`name` revoked; false if none was active.
    pub fn revoke_api_key(&self, profile: &str, name: &str) -> Result<bool> {
        let n = self.conn.execute(
            "UPDATE api_keys SET revoked_at = ?1 WHERE profile = ?2 AND name = ?3 AND revoked_at IS NULL",
            params![Utc::now().to_rfc3339(), profile, name],
        )?;
        Ok(n > 0)
    }

    pub fn touch_api_key(&self, id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE api_keys SET last_used_at = ?1 WHERE id = ?2",
            params![Utc::now().to_rfc3339(), id],
        )?;
        Ok(())
    }

    pub fn insert_audit(&self, ip: &str, prefix: &str, outcome: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO auth_audit (ts, ip, prefix, outcome) VALUES (?1, ?2, ?3, ?4)",
            params![Utc::now().to_rfc3339(), ip, prefix, outcome],
        )?;
        Ok(())
    }

    // ----- proxied requests --------------------------------------------------

    pub fn insert_request(&self, r: &RequestRow) -> Result<()> {
        self.conn.execute(
            "INSERT INTO requests (ts, profile, key_prefix, user, dialect, tier, model, status, input_tokens, cache_read_tokens,
                                   cache_write_tokens, output_tokens, cost_usd, latency_ms, stream, degraded)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![
                r.ts.to_rfc3339(),
                r.profile,
                r.key_prefix,
                r.user,
                r.dialect,
                r.tier,
                r.model,
                r.status,
                r.input_tokens,
                r.cache_read_tokens,
                r.cache_write_tokens,
                r.output_tokens,
                r.cost_usd,
                r.latency_ms,
                r.stream as i32,
                r.degraded
            ],
        )?;
        Ok(())
    }

    /// Proxied traffic per UTC day × profile × model over the last `days`.
    pub fn daily_requests(&self, days: u32) -> Result<Vec<DailyRequests>> {
        let mut stmt = self.conn.prepare(
            "SELECT substr(ts, 1, 10) AS day, profile, model,
                    COUNT(*), SUM(status >= 400), SUM(input_tokens + cache_write_tokens), SUM(cache_read_tokens), SUM(output_tokens),
                    SUM(cost_usd), AVG(latency_ms), SUM(degraded != ''), SUM(stream)
             FROM requests WHERE ts >= datetime('now', ?1)
             GROUP BY day, profile, model ORDER BY day DESC, profile, model",
        )?;
        let rows = stmt.query_map(params![format!("-{days} days")], |r| {
            Ok(DailyRequests {
                day: r.get(0)?,
                profile: r.get(1)?,
                model: r.get(2)?,
                requests: r.get(3)?,
                errors: r.get(4)?,
                input_tokens: r.get(5)?,
                cache_read_tokens: r.get(6)?,
                output_tokens: r.get(7)?,
                cost_usd: r.get(8)?,
                avg_latency_ms: r.get(9)?,
                degraded: r.get(10)?,
                streamed: r.get(11)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// Ring 2 view of today (UTC): per profile, live spend and rejections from the proxy.
    pub fn today_by_profile(&self) -> Result<Vec<TodayProfile>> {
        let mut stmt = self.conn.prepare(
            "SELECT profile, COUNT(*), SUM(cost_usd), SUM(status = 429), SUM(status >= 400 AND status != 429), SUM(degraded IN ('fast-only','max-tokens'))
             FROM requests WHERE ts >= date('now') GROUP BY profile",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(TodayProfile {
                profile: r.get(0)?,
                requests: r.get(1)?,
                spent_usd: r.get(2)?,
                rejected: r.get(3)?,
                errors: r.get(4)?,
                degraded: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// Last `limit` authentication failures / revocations.
    pub fn recent_audit(&self, limit: u32) -> Result<Vec<AuditRow>> {
        let mut stmt = self
            .conn
            .prepare("SELECT ts, ip, prefix, outcome FROM auth_audit ORDER BY id DESC LIMIT ?1")?;
        let rows = stmt.query_map(params![limit], |r| {
            Ok(AuditRow {
                ts: r.get(0)?,
                ip: r.get(1)?,
                prefix: r.get(2)?,
                outcome: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// Sessions (profile × user) active in the last `days`, most recent first.
    pub fn sessions(&self, days: u32, limit: u32) -> Result<Vec<SessionRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT profile, user, MIN(ts), MAX(ts), COUNT(*), SUM(status >= 400),
                    SUM(input_tokens + cache_write_tokens), SUM(cache_read_tokens), SUM(output_tokens),
                    SUM(cost_usd), AVG(latency_ms), GROUP_CONCAT(DISTINCT model), SUM(degraded != '')
             FROM requests WHERE ts >= datetime('now', ?1) AND user != ''
             GROUP BY profile, user ORDER BY MAX(ts) DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![format!("-{days} days"), limit], |r| {
            Ok(SessionRow {
                profile: r.get(0)?,
                user: r.get(1)?,
                first_ts: r.get(2)?,
                last_ts: r.get(3)?,
                requests: r.get(4)?,
                errors: r.get(5)?,
                input_tokens: r.get(6)?,
                cache_read_tokens: r.get(7)?,
                output_tokens: r.get(8)?,
                cost_usd: r.get(9)?,
                avg_latency_ms: r.get(10)?,
                models: r.get::<_, Option<String>>(11)?.unwrap_or_default(),
                degraded: r.get(12)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// The last `limit` proxied requests, newest first.
    pub fn recent_requests(&self, limit: u32) -> Result<Vec<RecentRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT ts, profile, user, dialect, model, status, input_tokens + cache_write_tokens, cache_read_tokens, output_tokens,
                    cost_usd, latency_ms, stream, degraded
             FROM requests ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], |r| {
            Ok(RecentRow {
                ts: r.get(0)?,
                profile: r.get(1)?,
                user: r.get(2)?,
                dialect: r.get(3)?,
                model: r.get(4)?,
                status: r.get(5)?,
                input_tokens: r.get(6)?,
                cache_read_tokens: r.get(7)?,
                output_tokens: r.get(8)?,
                cost_usd: r.get(9)?,
                latency_ms: r.get(10)?,
                stream: r.get::<_, i32>(11)? != 0,
                degraded: r.get(12)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// USD spent by `profile` since `since` (UTC, RFC 3339 prefix compare works on the stored format).
    pub fn spent_since(&self, profile: &str, since: DateTime<Utc>) -> Result<f64> {
        Ok(self.conn.query_row(
            "SELECT COALESCE(SUM(cost_usd), 0) FROM requests WHERE profile = ?1 AND ts >= ?2",
            params![profile, since.to_rfc3339()],
            |r| r.get(0),
        )?)
    }

    #[cfg(test)]
    pub fn conn_for_tests(&self) -> &Connection {
        &self.conn
    }

    pub fn insert_bench_run(&self, b: &BenchRun) -> Result<()> {
        self.conn.execute(
            "INSERT INTO bench_runs (ts, run_id, task_id, tool, tier, model, passed, cost_usd, seconds, exit_code, notes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                b.ts.to_rfc3339(),
                b.run_id,
                b.task_id,
                b.tool,
                b.tier,
                b.model,
                b.passed as i32,
                b.cost_usd,
                b.seconds,
                b.exit_code,
                b.notes
            ],
        )?;
        Ok(())
    }

    pub fn bench_runs(&self, run_id: Option<&str>) -> Result<Vec<BenchRun>> {
        let sql = "SELECT ts, run_id, task_id, tool, tier, model, passed, cost_usd, seconds, exit_code, notes
                   FROM bench_runs WHERE (?1 IS NULL OR run_id = ?1) ORDER BY ts";
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(params![run_id], |r| {
            Ok(BenchRun {
                ts: DateTime::parse_from_rfc3339(&r.get::<_, String>(0)?)
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                run_id: r.get(1)?,
                task_id: r.get(2)?,
                tool: r.get(3)?,
                tier: r.get(4)?,
                model: r.get(5)?,
                passed: r.get::<_, i32>(6)? != 0,
                cost_usd: r.get(7)?,
                seconds: r.get(8)?,
                exit_code: r.get(9)?,
                notes: r.get(10)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn snap(ts: &str, profile: &str, daily: f64) -> Snapshot {
        Snapshot {
            ts: DateTime::parse_from_rfc3339(ts)
                .unwrap()
                .with_timezone(&Utc),
            profile: profile.into(),
            usage_total: 10.0,
            usage_daily: daily,
            usage_weekly: 3.0,
            usage_monthly: 10.0,
            limit: Some(3.0),
            limit_remaining: Some(3.0 - daily),
        }
    }

    #[test]
    fn daily_report_takes_the_max_per_day_and_profile() {
        let db = Db::memory().unwrap();
        let today = Utc::now().format("%Y-%m-%d").to_string();
        db.insert_snapshot(&snap(&format!("{today}T08:00:00Z"), "dev", 0.4))
            .unwrap();
        db.insert_snapshot(&snap(&format!("{today}T18:00:00Z"), "dev", 1.1))
            .unwrap();
        db.insert_snapshot(&snap(&format!("{today}T18:00:00Z"), "car", 0.05))
            .unwrap();
        let rows = db.daily_usage(7).unwrap();
        assert_eq!(rows.len(), 2);
        let dev = rows.iter().find(|r| r.profile == "dev").unwrap();
        assert_eq!(dev.usage_daily, 1.1);
        assert_eq!(dev.limit, Some(3.0));
    }

    #[test]
    fn events_aggregate_per_day_and_ingest_state_round_trips() {
        let db = Db::memory().unwrap();
        let now = Utc::now();
        let mk = |kind: &str, inp: i64, out: i64| Event {
            ts: now,
            source: "aider".into(),
            session: "s".into(),
            model: "m".into(),
            kind: kind.into(),
            input_tokens: inp,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            output_tokens: out,
            detail: String::new(),
            latency_ms: Some(1500),
        };
        db.insert_events(&[
            mk("request", 100, 10),
            mk("request", 200, 20),
            mk("error", 0, 0),
            mk("retry", 0, 0),
        ])
        .unwrap();
        let s = db.daily_stats(1).unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(
            (
                s[0].requests,
                s[0].errors,
                s[0].retries,
                s[0].input_tokens,
                s[0].output_tokens
            ),
            (2, 1, 1, 300, 30)
        );
        assert_eq!(s[0].avg_latency_ms, Some(1500.0));
        assert_eq!(db.ingest_state("/x").unwrap(), (0, String::new()));
        db.set_ingest_state("/x", 42, "m").unwrap();
        db.set_ingest_state("/x", 84, "m2").unwrap();
        assert_eq!(db.ingest_state("/x").unwrap(), (84, "m2".to_string()));
    }

    #[test]
    fn api_keys_insert_lookup_revoke_and_audit() {
        let db = Db::memory().unwrap();
        let key = ApiKey {
            id: 0,
            key_hash: "abc".into(),
            prefix: "brain_car_…1234".into(),
            profile: "car".into(),
            name: "prod".into(),
            ip_allow: String::new(),
            created_at: Utc::now(),
            expires_at: None,
            last_used_at: None,
            revoked_at: None,
        };
        let id = db.insert_api_key(&key).unwrap();
        let found = db.api_key_by_hash("abc").unwrap().unwrap();
        assert_eq!(
            (found.id, found.profile.as_str(), found.name.as_str()),
            (id, "car", "prod")
        );
        assert!(db.api_key_by_hash("nope").unwrap().is_none());
        // same active name in the same profile is refused
        assert!(
            db.insert_api_key(&ApiKey {
                key_hash: "other".into(),
                ..key.clone()
            })
            .is_err()
        );
        db.touch_api_key(id).unwrap();
        assert!(
            db.api_key_by_hash("abc")
                .unwrap()
                .unwrap()
                .last_used_at
                .is_some()
        );
        assert!(db.revoke_api_key("car", "prod").unwrap());
        assert!(
            !db.revoke_api_key("car", "prod").unwrap(),
            "already revoked"
        );
        assert!(
            db.api_key_by_hash("abc")
                .unwrap()
                .unwrap()
                .revoked_at
                .is_some()
        );
        // after revocation the name can be reused
        assert!(
            db.insert_api_key(&ApiKey {
                key_hash: "other".into(),
                ..key.clone()
            })
            .is_ok()
        );
        assert_eq!(db.api_keys(Some("car")).unwrap().len(), 2);
        assert!(db.api_keys(Some("dev")).unwrap().is_empty());
        db.insert_audit("1.2.3.4", "brain_car_…", "unknown")
            .unwrap();
        db.insert_audit("1.2.3.4", "brain_car_…", "unknown")
            .unwrap();
    }

    #[test]
    fn requests_sum_spend_per_profile_since() {
        let db = Db::memory().unwrap();
        let row = |profile: &str, cost: f64| RequestRow {
            ts: Utc::now(),
            profile: profile.into(),
            key_prefix: "p".into(),
            user: String::new(),
            dialect: "openai".into(),
            tier: "fast".into(),
            model: "m".into(),
            status: 200,
            input_tokens: 10,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            output_tokens: 5,
            cost_usd: cost,
            latency_ms: 100,
            stream: false,
            degraded: String::new(),
        };
        db.insert_request(&row("dev", 0.5)).unwrap();
        db.insert_request(&row("dev", 0.25)).unwrap();
        db.insert_request(&row("car", 0.1)).unwrap();
        let since = Utc::now() - chrono::Duration::hours(1);
        assert!((db.spent_since("dev", since).unwrap() - 0.75).abs() < 1e-9);
        assert!((db.spent_since("car", since).unwrap() - 0.1).abs() < 1e-9);
        assert_eq!(
            db.spent_since("dev", Utc::now() + chrono::Duration::hours(1))
                .unwrap(),
            0.0
        );
        let agg = db.daily_requests(1).unwrap();
        let dev = agg.iter().find(|a| a.profile == "dev").unwrap();
        assert_eq!(
            (
                dev.requests,
                dev.errors,
                dev.input_tokens,
                dev.output_tokens
            ),
            (2, 0, 20, 10)
        );
        assert!((dev.cost_usd - 0.75).abs() < 1e-9);
        assert_eq!(dev.avg_latency_ms, Some(100.0));
        // sessions group by (profile, user) and skip anonymous requests
        let mut s1 = row("dev", 0.01);
        s1.user = "cc:abc".into();
        s1.model = "m2".into();
        db.insert_request(&s1).unwrap();
        db.insert_request(&s1).unwrap();
        let sessions = db.sessions(1, 10).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(
            (
                sessions[0].user.as_str(),
                sessions[0].requests,
                sessions[0].models.as_str()
            ),
            ("cc:abc", 2, "m2")
        );
        assert!((sessions[0].cost_usd - 0.02).abs() < 1e-9);
        let today = db.today_by_profile().unwrap();
        let d = today.iter().find(|t| t.profile == "dev").unwrap();
        assert_eq!((d.requests, d.rejected, d.errors), (4, 0, 0));
        assert!((d.spent_usd - 0.77).abs() < 1e-9);
        db.insert_audit("1.2.3.4", "brain_x_…", "unknown").unwrap();
        assert_eq!(db.recent_audit(5).unwrap()[0].outcome, "unknown");
        let recent = db.recent_requests(3).unwrap();
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].user, "cc:abc", "newest first");
    }

    #[test]
    fn bench_runs_round_trip() {
        let db = Db::memory().unwrap();
        let run = BenchRun {
            ts: Utc.with_ymd_and_hms(2026, 9, 19, 1, 0, 0).unwrap(),
            run_id: "r1".into(),
            task_id: "ago-0001".into(),
            tool: "aider".into(),
            tier: "fast".into(),
            model: "prism-ml/ternary-bonsai-2-27b".into(),
            passed: true,
            cost_usd: Some(0.021),
            seconds: 42.5,
            exit_code: Some(0),
            notes: String::new(),
        };
        db.insert_bench_run(&run).unwrap();
        assert_eq!(db.bench_runs(Some("r1")).unwrap(), vec![run.clone()]);
        assert_eq!(db.bench_runs(None).unwrap().len(), 1);
        assert!(db.bench_runs(Some("other")).unwrap().is_empty());
    }
}
