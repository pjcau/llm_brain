//! Client keys: `brain_<profile>_<random>`, stored as sha256, bound to a
//! profile, with optional expiry and IP allowlist. Issued only by the CLI on
//! the server (no HTTP endpoint), see docs/architecture/auth-flow.md.

use anyhow::{Result, bail};
use base64::Engine;
use chrono::{DateTime, Utc};
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::net::IpAddr;

pub const PREFIX: &str = "brain_";

#[derive(Debug, Clone, PartialEq)]
pub struct ApiKey {
    pub id: i64,
    pub key_hash: String,
    /// `brain_car_…a1b2`: safe to log.
    pub prefix: String,
    pub profile: String,
    pub name: String,
    /// CSV of IPs or CIDRs; empty = any.
    pub ip_allow: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

/// Generates a new key for `profile`: 32 random bytes, base64url, no padding.
pub fn generate(profile: &str) -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!(
        "{PREFIX}{profile}_{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    )
}

pub fn hash(key: &str) -> String {
    let mut h = Sha256::new();
    h.update(key.as_bytes());
    format!("{:x}", h.finalize())
}

/// `brain_car_abcdef…` → `brain_car_…cdef` (last 4 chars) for logs. The
/// token itself is base64url and may contain `_`, so the head is taken from
/// the `brain_<profile>_` structure, never by splitting on the last `_`.
pub fn display_prefix(key: &str) -> String {
    let tail: String = key
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let head = key
        .strip_prefix(PREFIX)
        .and_then(|rest| rest.split_once('_'))
        .map(|(profile, _)| format!("{PREFIX}{profile}"))
        .unwrap_or_else(|| "brain".into());
    format!("{head}_…{tail}")
}

/// Cheap syntactic check before touching the database: `brain_<profile>_<token>`.
pub fn looks_like_key(key: &str) -> Option<&str> {
    let rest = key.strip_prefix(PREFIX)?;
    let (profile, token) = rest.split_once('_')?;
    if profile.is_empty() || !profile.chars().all(|c| c.is_ascii_lowercase()) {
        return None;
    }
    if token.len() < 32
        || !token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return None;
    }
    Some(profile)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejection {
    Malformed,
    Unknown,
    Revoked,
    Expired,
    IpNotAllowed,
}

impl Rejection {
    pub fn as_str(self) -> &'static str {
        match self {
            Rejection::Malformed => "malformed",
            Rejection::Unknown => "unknown",
            Rejection::Revoked => "revoked",
            Rejection::Expired => "expired",
            Rejection::IpNotAllowed => "ip_not_allowed",
        }
    }
}

/// Validates a stored key record against `now` and the caller's IP.
pub fn check(record: &ApiKey, now: DateTime<Utc>, ip: Option<IpAddr>) -> Result<(), Rejection> {
    if record.revoked_at.is_some() {
        return Err(Rejection::Revoked);
    }
    if record.expires_at.is_some_and(|e| e <= now) {
        return Err(Rejection::Expired);
    }
    if !record.ip_allow.trim().is_empty() {
        let Some(ip) = ip else {
            return Err(Rejection::IpNotAllowed);
        };
        if !ip_allowed(&record.ip_allow, ip) {
            return Err(Rejection::IpNotAllowed);
        }
    }
    Ok(())
}

/// `1.2.3.4, 10.0.0.0/8, 2001:db8::/32`
pub fn ip_allowed(allow: &str, ip: IpAddr) -> bool {
    allow
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .any(|entry| match entry.split_once('/') {
            None => entry.parse::<IpAddr>().is_ok_and(|a| a == ip),
            Some((net, bits)) => match (net.parse::<IpAddr>(), bits.parse::<u32>()) {
                (Ok(IpAddr::V4(n)), Ok(b)) if b <= 32 => match ip {
                    IpAddr::V4(i) => {
                        let mask = if b == 0 { 0 } else { u32::MAX << (32 - b) };
                        (u32::from(n) & mask) == (u32::from(i) & mask)
                    }
                    _ => false,
                },
                (Ok(IpAddr::V6(n)), Ok(b)) if b <= 128 => match ip {
                    IpAddr::V6(i) => {
                        let mask = if b == 0 { 0 } else { u128::MAX << (128 - b) };
                        (u128::from(n) & mask) == (u128::from(i) & mask)
                    }
                    _ => false,
                },
                _ => false,
            },
        })
}

pub fn parse_expiry(s: &str) -> Result<DateTime<Utc>> {
    if let Ok(d) = DateTime::parse_from_rfc3339(s) {
        return Ok(d.with_timezone(&Utc));
    }
    if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Ok(d.and_hms_opt(0, 0, 0).unwrap().and_utc());
    }
    bail!("expiry must be YYYY-MM-DD or RFC 3339, got `{s}`")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn record(profile: &str) -> ApiKey {
        ApiKey {
            id: 1,
            key_hash: "h".into(),
            prefix: format!("brain_{profile}_…abcd"),
            profile: profile.into(),
            name: "prod".into(),
            ip_allow: String::new(),
            created_at: Utc::now(),
            expires_at: None,
            last_used_at: None,
            revoked_at: None,
        }
    }

    #[test]
    fn generated_keys_have_the_shape_and_are_unique() {
        let a = generate("car");
        let b = generate("car");
        assert!(a.starts_with("brain_car_"));
        assert_ne!(a, b);
        assert_eq!(looks_like_key(&a), Some("car"));
        assert_eq!(hash(&a).len(), 64);
        assert_ne!(hash(&a), hash(&b));
        assert!(display_prefix(&a).starts_with("brain_car_…"));
        let shown = display_prefix(&a);
        assert_eq!(shown.len(), "brain_car_…".len() + 4, "{shown}");
        assert!(
            !shown.contains(&a[10..20]),
            "prefix must not leak the token: {shown}"
        );
        // a token containing underscores must not move the split point
        assert_eq!(
            display_prefix("brain_dev_ab_cd_efghijklmnopqrstuvwxyz012345"),
            "brain_dev_…2345"
        );
    }

    #[test]
    fn malformed_keys_are_rejected_syntactically() {
        assert_eq!(looks_like_key("sk-or-v1-abc"), None);
        assert_eq!(looks_like_key("brain_car_short"), None);
        assert_eq!(
            looks_like_key("brain_Car_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            None
        );
        assert_eq!(
            looks_like_key("brain__aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            None
        );
    }

    #[test]
    fn check_enforces_revocation_expiry_and_ip_allowlist() {
        let now = Utc::now();
        let ok = record("car");
        assert_eq!(check(&ok, now, None), Ok(()));
        let revoked = ApiKey {
            revoked_at: Some(now),
            ..record("car")
        };
        assert_eq!(check(&revoked, now, None), Err(Rejection::Revoked));
        let expired = ApiKey {
            expires_at: Some(now - Duration::seconds(1)),
            ..record("car")
        };
        assert_eq!(check(&expired, now, None), Err(Rejection::Expired));
        let future = ApiKey {
            expires_at: Some(now + Duration::days(1)),
            ..record("car")
        };
        assert_eq!(check(&future, now, None), Ok(()));
        let pinned = ApiKey {
            ip_allow: "10.0.0.0/8, 203.0.113.7".into(),
            ..record("dev")
        };
        assert_eq!(
            check(&pinned, now, Some("10.20.30.40".parse().unwrap())),
            Ok(())
        );
        assert_eq!(
            check(&pinned, now, Some("203.0.113.7".parse().unwrap())),
            Ok(())
        );
        assert_eq!(
            check(&pinned, now, Some("203.0.113.8".parse().unwrap())),
            Err(Rejection::IpNotAllowed)
        );
        assert_eq!(check(&pinned, now, None), Err(Rejection::IpNotAllowed));
        assert!(ip_allowed("2001:db8::/32", "2001:db8::1".parse().unwrap()));
        assert!(!ip_allowed("2001:db8::/32", "2001:db9::1".parse().unwrap()));
        assert!(!ip_allowed("garbage", "1.2.3.4".parse().unwrap()));
    }

    #[test]
    fn expiry_parses_date_or_rfc3339() {
        assert_eq!(
            parse_expiry("2027-01-01").unwrap().to_rfc3339(),
            "2027-01-01T00:00:00+00:00"
        );
        assert!(parse_expiry("2027-01-01T10:00:00Z").is_ok());
        assert!(parse_expiry("tomorrow").is_err());
    }
}
