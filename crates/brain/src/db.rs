//! SQLite persistence for Phase 0: usage snapshots per profile and benchmark
//! runs. One file, WAL mode; `Db::memory()` for tests.

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
