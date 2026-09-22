//! Ring 2 of the budget design (docs/architecture/budget.md): per-profile
//! daily and monthly spend from the `requests` table, degradation before the
//! wall. Ring 1 (the OpenRouter key's own daily limit) still holds if this
//! code is wrong.

use crate::config::{Config, Profile};
use chrono::{DateTime, Datelike, TimeZone, Utc};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    Allow,
    /// ≥ 70%: the reasoning tier is off, everything goes to `fast`.
    FastOnly,
    /// ≥ 85%: like FastOnly, plus `max_tokens` capped to this value.
    CapTokens(u64),
    /// ≥ 100%: refuse with 429 until the window resets.
    Block,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    pub action: Action,
    pub day_pct: f64,
    pub month_pct: f64,
    /// Seconds until the binding window (day or month) resets, for `retry-after`.
    pub retry_after_s: u64,
}

impl Decision {
    pub fn degraded_label(&self) -> &'static str {
        match self.action {
            Action::Allow => "",
            Action::FastOnly => "fast-only",
            Action::CapTokens(_) => "max-tokens",
            Action::Block => "blocked",
        }
    }
}

pub const CAP_TOKENS: u64 = 2048;

pub fn day_start(now: DateTime<Utc>) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(now.year(), now.month(), now.day(), 0, 0, 0)
        .unwrap()
}

pub fn month_start(now: DateTime<Utc>) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
        .unwrap()
}

fn next_month_start(now: DateTime<Utc>) -> DateTime<Utc> {
    let (y, m) = if now.month() == 12 {
        (now.year() + 1, 1)
    } else {
        (now.year(), now.month() + 1)
    };
    Utc.with_ymd_and_hms(y, m, 1, 0, 0, 0).unwrap()
}

/// The decision for `profile` given what it spent today and this month.
pub fn decide(
    profile: &Profile,
    spent_today: f64,
    spent_month: f64,
    now: DateTime<Utc>,
) -> Decision {
    let day_pct = if profile.daily_limit_usd > 0.0 {
        spent_today / profile.daily_limit_usd * 100.0
    } else {
        0.0
    };
    let month_pct = if profile.monthly_soft_usd > 0.0 {
        spent_month / profile.monthly_soft_usd * 100.0
    } else {
        0.0
    };
    let pct = day_pct.max(month_pct);
    let action = match pct {
        p if p >= 100.0 => Action::Block,
        p if p >= 85.0 => Action::CapTokens(CAP_TOKENS),
        p if p >= 70.0 => Action::FastOnly,
        _ => Action::Allow,
    };
    // whichever window is the binding one decides when to retry
    let reset = if day_pct >= month_pct {
        day_start(now) + chrono::Duration::days(1)
    } else {
        next_month_start(now)
    };
    let retry_after_s = (reset - now).num_seconds().max(60) as u64;
    Decision {
        action,
        day_pct,
        month_pct,
        retry_after_s,
    }
}

/// Applies the decision to the resolved tier: `FastOnly`/`CapTokens` force `fast`.
pub fn effective_tier<'a>(cfg: &'a Config, requested: &'a str, action: Action) -> &'a str {
    match action {
        Action::FastOnly | Action::CapTokens(_) if cfg.tiers.contains_key("fast") => "fast",
        _ => requested,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> Profile {
        Profile {
            name: "dev".into(),
            description: String::new(),
            tier: "fast".into(),
            daily_limit_usd: 3.0,
            monthly_soft_usd: 30.0,
            l2_cache: "off".into(),
            router: None,
        }
    }

    #[test]
    fn thresholds_follow_the_documented_rings() {
        let now = Utc.with_ymd_and_hms(2026, 9, 20, 10, 0, 0).unwrap();
        let p = profile();
        assert_eq!(decide(&p, 0.5, 5.0, now).action, Action::Allow);
        assert_eq!(decide(&p, 2.1, 5.0, now).action, Action::FastOnly);
        assert_eq!(
            decide(&p, 2.6, 5.0, now).action,
            Action::CapTokens(CAP_TOKENS)
        );
        assert_eq!(decide(&p, 3.0, 5.0, now).action, Action::Block);
        // the monthly window binds too
        assert_eq!(decide(&p, 0.1, 21.0, now).action, Action::FastOnly);
        assert_eq!(decide(&p, 0.1, 30.0, now).action, Action::Block);
    }

    #[test]
    fn retry_after_points_at_the_binding_window_reset() {
        let now = Utc.with_ymd_and_hms(2026, 9, 20, 23, 0, 0).unwrap();
        let p = profile();
        let d = decide(&p, 3.0, 5.0, now);
        assert_eq!(d.retry_after_s, 3600, "daily window resets at midnight UTC");
        let d = decide(&p, 0.0, 30.0, now);
        assert_eq!(
            d.retry_after_s,
            (Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap() - now).num_seconds() as u64
        );
        let dec = Utc.with_ymd_and_hms(2026, 12, 31, 12, 0, 0).unwrap();
        assert_eq!(decide(&p, 0.0, 30.0, dec).retry_after_s, 12 * 3600);
        assert_eq!(decide(&p, 0.0, 0.0, now).degraded_label(), "");
        assert_eq!(decide(&p, 3.0, 0.0, now).degraded_label(), "blocked");
    }

    #[test]
    fn effective_tier_forces_fast_when_degraded() {
        let cfg = Config::from_yaml(
            "profiles:\n  - {name: dev, tier: fast, daily_limit_usd: 3.0, monthly_soft_usd: 30.0}\n",
            "tiers:\n  fast: {model: f}\n  reasoning: {model: r}\n",
        )
        .unwrap();
        assert_eq!(
            effective_tier(&cfg, "reasoning", Action::Allow),
            "reasoning"
        );
        assert_eq!(effective_tier(&cfg, "reasoning", Action::FastOnly), "fast");
        assert_eq!(
            effective_tier(&cfg, "reasoning", Action::CapTokens(1)),
            "fast"
        );
        assert_eq!(effective_tier(&cfg, "fast", Action::Block), "fast");
    }
}
