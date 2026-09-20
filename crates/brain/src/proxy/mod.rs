//! Phase 1: the aware reverse proxy. Every `/v1/*` request must carry a
//! client key; the middleware chain is the one documented in
//! docs/architecture/auth-flow.md: header → syntax → hash lookup → checks →
//! rate limits → budget → forward with the profile's upstream key → record.
//! Bodies pass through as bytes (streaming untouched, never buffered) except
//! for the JSON shaping in `sanitize`.

pub mod sanitize;
pub mod tap;
pub mod usage_parse;

use crate::auth::{self, ApiKey, Rejection};
use crate::budget::{self, Action};
use crate::config::Config;
use crate::db::{Db, RequestRow};
use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use chrono::Utc;
use governor::{Quota, RateLimiter};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use usage_parse::{Dialect, Usage};

type KeyedLimiter = RateLimiter<
    String,
    governor::state::keyed::DefaultKeyedStateStore<String>,
    governor::clock::DefaultClock,
>;

#[derive(Debug, Clone)]
pub struct Limits {
    pub key_per_minute: u32,
    pub user_per_minute: u32,
    /// Auth failures from one IP within `ip_window` before it is blocked.
    pub ip_fail_max: u32,
    pub ip_window: Duration,
    pub key_cache_ttl: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            key_per_minute: 60,
            user_per_minute: 10,
            ip_fail_max: 20,
            ip_window: Duration::from_secs(600),
            key_cache_ttl: Duration::from_secs(60),
        }
    }
}

pub struct ProxyState {
    pub cfg: Arc<Config>,
    pub db_path: PathBuf,
    pub http: reqwest::Client,
    /// OpenRouter API base, e.g. `https://openrouter.ai/api/v1` (tests: a mock).
    pub upstream_base: String,
    /// profile → upstream key, from the environment (`OPENROUTER_KEY_<PROFILE>`).
    pub upstream_keys: HashMap<String, String>,
    pub limits: Limits,
    key_cache: Mutex<HashMap<String, (ApiKey, Instant)>>,
    per_key: KeyedLimiter,
    per_user: KeyedLimiter,
    ip_fail: Mutex<HashMap<IpAddr, (u32, Instant)>>,
}

impl ProxyState {
    pub fn new(
        cfg: Arc<Config>,
        db_path: PathBuf,
        upstream_base: String,
        env: &HashMap<String, String>,
        limits: Limits,
    ) -> Arc<Self> {
        let upstream_keys = cfg
            .profiles
            .iter()
            .filter_map(|p| {
                env.get(&p.key_env())
                    .filter(|k| !k.is_empty())
                    .map(|k| (p.name.clone(), k.clone()))
            })
            .collect();
        let per_min = |n: u32| Quota::per_minute(NonZeroU32::new(n.max(1)).unwrap());
        Arc::new(Self {
            cfg,
            db_path,
            http: reqwest::Client::builder()
                .user_agent(concat!("llm_brain/", env!("CARGO_PKG_VERSION")))
                .build()
                .expect("reqwest"),
            upstream_base: upstream_base.trim_end_matches('/').to_string(),
            upstream_keys,
            per_key: RateLimiter::keyed(per_min(limits.key_per_minute)),
            per_user: RateLimiter::keyed(per_min(limits.user_per_minute)),
            limits,
            key_cache: Mutex::new(HashMap::new()),
            ip_fail: Mutex::new(HashMap::new()),
        })
    }
}

pub fn router(state: Arc<ProxyState>) -> Router {
    Router::new()
        .route("/v1/chat/completions", post(openai_chat))
        .route("/v1/models", get(models))
        .route("/v1/messages", post(anthropic_messages))
        .route("/v1/messages/count_tokens", post(anthropic_count_tokens))
        .route("/api/hello", any(hello))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// errors in the client's dialect
// ---------------------------------------------------------------------------

fn error_response(
    dialect: Dialect,
    status: StatusCode,
    kind: &str,
    message: &str,
    retry_after: Option<u64>,
    should_retry: Option<bool>,
) -> Response {
    let body = match dialect {
        Dialect::OpenAi => {
            json!({"error": {"message": message, "type": kind, "code": status.as_u16()}})
        }
        Dialect::Anthropic => json!({"type": "error", "error": {"type": kind, "message": message}}),
    };
    let mut resp = (status, axum::Json(body)).into_response();
    if let Some(s) = retry_after {
        resp.headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from(s));
    }
    if let Some(b) = should_retry {
        resp.headers_mut().insert(
            "x-should-retry",
            HeaderValue::from_static(if b { "true" } else { "false" }),
        );
    }
    resp
}

fn auth_error(dialect: Dialect, status: StatusCode, message: &str) -> Response {
    let kind = match (dialect, status) {
        (Dialect::OpenAi, StatusCode::FORBIDDEN) => "permission_error",
        (Dialect::OpenAi, _) => "invalid_api_key",
        (Dialect::Anthropic, StatusCode::FORBIDDEN) => "permission_error",
        (Dialect::Anthropic, _) => "authentication_error",
    };
    error_response(dialect, status, kind, message, None, Some(false))
}

fn rate_error(dialect: Dialect, message: &str, retry_after: u64, should_retry: bool) -> Response {
    let kind = match dialect {
        Dialect::OpenAi => "rate_limit_exceeded",
        Dialect::Anthropic => "rate_limit_error",
    };
    error_response(
        dialect,
        StatusCode::TOO_MANY_REQUESTS,
        kind,
        message,
        Some(retry_after),
        Some(should_retry),
    )
}

// ---------------------------------------------------------------------------
// authentication
// ---------------------------------------------------------------------------

/// The caller's IP: `X-Forwarded-For` (we sit behind Caddy on localhost), else the socket.
pub fn client_ip(headers: &HeaderMap, addr: SocketAddr) -> IpAddr {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(addr.ip())
}

/// `Authorization: Bearer …` (OpenAI SDK, aider) or `x-api-key` (Anthropic SDK, Claude Code).
pub fn presented_key(headers: &HeaderMap) -> Option<String> {
    if let Some(v) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        && let Some(t) = v.strip_prefix("Bearer ")
        && !t.trim().is_empty()
    {
        return Some(t.trim().to_string());
    }
    headers
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub struct Authed {
    pub key: ApiKey,
}

impl ProxyState {
    fn ip_blocked(&self, ip: IpAddr) -> bool {
        let mut m = self.ip_fail.lock().unwrap();
        match m.get(&ip) {
            Some((n, since)) if since.elapsed() < self.limits.ip_window => {
                *n >= self.limits.ip_fail_max
            }
            Some(_) => {
                m.remove(&ip);
                false
            }
            None => false,
        }
    }

    fn note_failure(&self, ip: IpAddr) {
        let mut m = self.ip_fail.lock().unwrap();
        let e = m.entry(ip).or_insert((0, Instant::now()));
        if e.1.elapsed() >= self.limits.ip_window {
            *e = (0, Instant::now());
        }
        e.0 += 1;
    }

    fn cached_key(&self, hash: &str) -> Option<ApiKey> {
        let m = self.key_cache.lock().unwrap();
        m.get(hash)
            .filter(|(_, at)| at.elapsed() < self.limits.key_cache_ttl)
            .map(|(k, _)| k.clone())
    }

    fn cache_key(&self, hash: &str, key: &ApiKey) {
        self.key_cache
            .lock()
            .unwrap()
            .insert(hash.to_string(), (key.clone(), Instant::now()));
    }

    /// The full auth chain. `Err` is a ready-to-send response.
    async fn authenticate(
        &self,
        dialect: Dialect,
        headers: &HeaderMap,
        ip: IpAddr,
    ) -> Result<Authed, Response> {
        if self.ip_blocked(ip) {
            return Err(rate_error(
                dialect,
                "too many failed authentications from this address",
                600,
                false,
            ));
        }
        let Some(presented) = presented_key(headers) else {
            self.note_failure(ip);
            self.audit(ip, "-", "missing").await;
            return Err(auth_error(
                dialect,
                StatusCode::UNAUTHORIZED,
                "missing API key: use Authorization: Bearer brain_… or x-api-key",
            ));
        };
        let shown = auth::display_prefix(&presented);
        if auth::looks_like_key(&presented).is_none() {
            self.note_failure(ip);
            self.audit(ip, &shown, Rejection::Malformed.as_str()).await;
            return Err(auth_error(
                dialect,
                StatusCode::UNAUTHORIZED,
                "invalid API key",
            ));
        }
        let hash = auth::hash(&presented);
        let record = match self.cached_key(&hash) {
            Some(k) => Some(k),
            None => {
                let db_path = self.db_path.clone();
                let h = hash.clone();
                let found = tokio::task::spawn_blocking(move || {
                    Db::open(&db_path).and_then(|db| db.api_key_by_hash(&h))
                })
                .await
                .ok()
                .and_then(Result::ok)
                .flatten();
                if let Some(k) = &found {
                    self.cache_key(&hash, k);
                    let (db_path, id) = (self.db_path.clone(), k.id);
                    tokio::task::spawn_blocking(move || {
                        Db::open(&db_path).and_then(|db| db.touch_api_key(id))
                    });
                }
                found
            }
        };
        let Some(record) = record else {
            self.note_failure(ip);
            self.audit(ip, &shown, Rejection::Unknown.as_str()).await;
            return Err(auth_error(
                dialect,
                StatusCode::UNAUTHORIZED,
                "invalid API key",
            ));
        };
        if let Err(rej) = auth::check(&record, Utc::now(), Some(ip)) {
            self.note_failure(ip);
            self.audit(ip, &record.prefix, rej.as_str()).await;
            let (status, msg) = match rej {
                Rejection::IpNotAllowed => (
                    StatusCode::FORBIDDEN,
                    "this key is not allowed from your address",
                ),
                Rejection::Revoked => (StatusCode::UNAUTHORIZED, "API key revoked"),
                Rejection::Expired => (StatusCode::UNAUTHORIZED, "API key expired"),
                _ => (StatusCode::UNAUTHORIZED, "invalid API key"),
            };
            return Err(auth_error(dialect, status, msg));
        }
        Ok(Authed { key: record })
    }

    async fn audit(&self, ip: IpAddr, prefix: &str, outcome: &str) {
        let (db_path, ip, prefix, outcome) = (
            self.db_path.clone(),
            ip.to_string(),
            prefix.to_string(),
            outcome.to_string(),
        );
        let _ = tokio::task::spawn_blocking(move || {
            Db::open(&db_path).and_then(|db| db.insert_audit(&ip, &prefix, &outcome))
        })
        .await;
    }

    fn rate_limited(&self, key: &ApiKey, user: &str) -> bool {
        if self.per_key.check_key(&key.key_hash).is_err() {
            return true;
        }
        if !user.is_empty()
            && self
                .per_user
                .check_key(&format!("{}:{user}", key.key_hash))
                .is_err()
        {
            return true;
        }
        false
    }

    async fn budget_decision(&self, profile: &crate::config::Profile) -> budget::Decision {
        let now = Utc::now();
        let (db_path, name) = (self.db_path.clone(), profile.name.clone());
        let spent = tokio::task::spawn_blocking(move || {
            let db = Db::open(&db_path)?;
            Ok::<_, anyhow::Error>((
                db.spent_since(&name, budget::day_start(now))?,
                db.spent_since(&name, budget::month_start(now))?,
            ))
        })
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or((0.0, 0.0));
        budget::decide(profile, spent.0, spent.1, now)
    }

    fn record(&self, row: RequestRow) {
        let db_path = self.db_path.clone();
        tokio::task::spawn_blocking(move || {
            if let Err(e) = Db::open(&db_path).and_then(|db| db.insert_request(&row)) {
                eprintln!("record request failed: {e:#}");
            }
        });
    }
}

// ---------------------------------------------------------------------------
// handlers
// ---------------------------------------------------------------------------

async fn hello() -> StatusCode {
    StatusCode::NO_CONTENT
}

async fn models(
    State(st): State<Arc<ProxyState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    let ip = client_ip(&headers, addr);
    if let Err(r) = st.authenticate(Dialect::OpenAi, &headers, ip).await {
        return r;
    }
    let mut data = Vec::new();
    for (name, t) in &st.cfg.tiers {
        if let Some(m) = &t.model {
            data.push(json!({"id": format!("brain/{name}"), "object": "model", "owned_by": "llm_brain", "display_name": format!("brain {name} → {m}"), "description": format!("tier {name}")}));
        }
    }
    for (name, t) in &st.cfg.tiers {
        if let Some(m) = &t.model {
            data.push(json!({"id": m, "object": "model", "owned_by": "openrouter", "description": format!("tier {name}, direct")}));
        }
    }
    axum::Json(json!({"object": "list", "data": data})).into_response()
}

async fn openai_chat(
    State(st): State<Arc<ProxyState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    proxy(
        st,
        Dialect::OpenAi,
        "/chat/completions",
        headers,
        addr,
        body,
    )
    .await
}

async fn anthropic_messages(
    State(st): State<Arc<ProxyState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    proxy(st, Dialect::Anthropic, "/messages", headers, addr, body).await
}

async fn anthropic_count_tokens(
    State(st): State<Arc<ProxyState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    proxy(
        st,
        Dialect::Anthropic,
        "/messages/count_tokens",
        headers,
        addr,
        body,
    )
    .await
}

async fn proxy(
    st: Arc<ProxyState>,
    dialect: Dialect,
    upstream_path: &str,
    headers: HeaderMap,
    addr: SocketAddr,
    body: Bytes,
) -> Response {
    let started = Instant::now();
    let ip = client_ip(&headers, addr);
    let authed = match st.authenticate(dialect, &headers, ip).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let key = authed.key;
    let Ok(profile) = st.cfg.profile(&key.profile).cloned() else {
        return error_response(
            dialect,
            StatusCode::FORBIDDEN,
            "permission_error",
            "this key's profile no longer exists",
            None,
            Some(false),
        );
    };
    let Some(upstream_key) = st.upstream_keys.get(&profile.name).cloned() else {
        return error_response(
            dialect,
            StatusCode::SERVICE_UNAVAILABLE,
            "api_error",
            "no upstream key configured for this profile",
            Some(60),
            Some(true),
        );
    };

    // body
    let mut json: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return error_response(
                dialect,
                StatusCode::BAD_REQUEST,
                "invalid_request_error",
                &format!("body is not JSON: {e}"),
                None,
                Some(false),
            );
        }
    };
    let mut user = sanitize::end_user(&json);
    if user.is_empty()
        && let Some(sid) = headers
            .get("x-claude-code-session-id")
            .and_then(|v| v.to_str().ok())
    {
        user = format!("cc:{}", sid.chars().take(12).collect::<String>());
    }

    // rate limits
    if st.rate_limited(&key, &user) {
        return rate_error(
            dialect,
            "rate limit: slow down (per key / per user)",
            5,
            true,
        );
    }

    // count_tokens: forward without budget accounting (it costs nothing)
    let is_count = upstream_path.ends_with("count_tokens");

    // model + budget
    let requested = json
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_string);
    let Some(mut resolved) = sanitize::resolve_model(&st.cfg, requested.as_deref(), &profile.tier)
    else {
        return error_response(
            dialect,
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            "unknown model or tier",
            None,
            Some(false),
        );
    };
    let decision = if is_count {
        budget::decide(&profile, 0.0, 0.0, Utc::now())
    } else {
        st.budget_decision(&profile).await
    };
    match decision.action {
        Action::Block => {
            return rate_error(
                dialect,
                &format!(
                    "budget exhausted for profile `{}` (day {:.0}%, month {:.0}%); resets in {}s",
                    profile.name, decision.day_pct, decision.month_pct, decision.retry_after_s
                ),
                decision.retry_after_s.max(3600),
                false,
            );
        }
        Action::FastOnly | Action::CapTokens(_) => {
            let tier = budget::effective_tier(&st.cfg, &resolved.tier, decision.action).to_string();
            if tier != resolved.tier
                && let Some(r) =
                    sanitize::resolve_model(&st.cfg, Some(&format!("brain/{tier}")), &profile.tier)
            {
                resolved = r;
            }
            if let Action::CapTokens(cap) = decision.action {
                sanitize::cap_max_tokens(&mut json, cap);
            }
        }
        Action::Allow => {}
    }
    let mut degraded = decision.degraded_label().to_string();

    // shaping
    if let Some(obj) = json.as_object_mut() {
        obj.insert("model".into(), json!(resolved.model));
    }
    match dialect {
        Dialect::OpenAi => sanitize::shape_openai(&mut json, &resolved.chain),
        Dialect::Anthropic => {
            let removed = sanitize::sanitize_anthropic(&mut json);
            if !removed.is_empty() {
                if !degraded.is_empty() {
                    degraded.push(' ');
                }
                degraded.push_str(&format!("sanitized:{}", removed.join(",")));
            }
        }
    }
    let stream = json.get("stream").and_then(Value::as_bool).unwrap_or(false);

    // forward
    let mut req = st
        .http
        .post(format!("{}{}", st.upstream_base, upstream_path))
        .bearer_auth(&upstream_key)
        .header(header::CONTENT_TYPE, "application/json")
        .header("HTTP-Referer", "https://github.com/pjcau/llm_brain")
        .header("X-Title", "llm_brain");
    for name in ["anthropic-version", "anthropic-beta", "accept"] {
        if let Some(v) = headers.get(name) {
            req = req.header(name, v.clone());
        }
    }
    let upstream = match req
        .body(serde_json::to_vec(&json).unwrap_or_default())
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            st.record(row(
                &profile,
                &key,
                &user,
                dialect,
                &resolved,
                502,
                &Usage::default(),
                started,
                stream,
                &degraded,
                &st.cfg,
            ));
            return error_response(
                dialect,
                StatusCode::BAD_GATEWAY,
                "api_error",
                &format!("upstream unreachable: {e}"),
                Some(10),
                Some(true),
            );
        }
    };
    let status = upstream.status();
    let mut out_headers = HeaderMap::new();
    for name in [
        header::CONTENT_TYPE,
        header::CACHE_CONTROL,
        header::RETRY_AFTER,
    ] {
        if let Some(v) = upstream.headers().get(&name) {
            out_headers.insert(name, v.clone());
        }
    }
    for name in ["x-should-retry", "x-request-id"] {
        if let Some(v) = upstream.headers().get(name) {
            out_headers.insert(name, v.clone());
        }
    }

    let is_sse = upstream
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|c| c.starts_with("text/event-stream"));
    if is_sse && status.is_success() {
        let (st2, profile2, key2, user2, resolved2, degraded2) = (
            st.clone(),
            profile.clone(),
            key.clone(),
            user.clone(),
            resolved.clone(),
            degraded.clone(),
        );
        let recorder: tap::Recorder = Box::new(move |usage, latency_ms, completed| {
            let mut d = degraded2;
            if !completed {
                if !d.is_empty() {
                    d.push(' ');
                }
                d.push_str("client-disconnected");
            }
            let mut r = row(
                &profile2,
                &key2,
                &user2,
                dialect,
                &resolved2,
                status.as_u16() as i64,
                &usage,
                started,
                true,
                &d,
                &st2.cfg,
            );
            r.latency_ms = latency_ms;
            st2.record(r);
        });
        let tapped = tap::Tap::new(upstream.bytes_stream(), dialect, started, recorder);
        let mut resp = Response::new(Body::from_stream(tapped));
        *resp.status_mut() = status;
        *resp.headers_mut() = out_headers;
        return resp;
    }

    let bytes = match upstream.bytes().await {
        Ok(b) => b,
        Err(e) => {
            return error_response(
                dialect,
                StatusCode::BAD_GATEWAY,
                "api_error",
                &format!("upstream read failed: {e}"),
                Some(10),
                Some(true),
            );
        }
    };
    let usage = if status.is_success() && !is_count {
        serde_json::from_slice::<Value>(&bytes)
            .map(|v| Usage::from_body(dialect, &v))
            .unwrap_or_default()
    } else {
        Usage::default()
    };
    if !is_count {
        st.record(row(
            &profile,
            &key,
            &user,
            dialect,
            &resolved,
            status.as_u16() as i64,
            &usage,
            started,
            false,
            &degraded,
            &st.cfg,
        ));
    }
    let mut resp = Response::new(Body::from(bytes));
    *resp.status_mut() = status;
    *resp.headers_mut() = out_headers;
    resp
}

#[allow(clippy::too_many_arguments)]
fn row(
    profile: &crate::config::Profile,
    key: &ApiKey,
    user: &str,
    dialect: Dialect,
    resolved: &sanitize::Resolved,
    status: i64,
    usage: &Usage,
    started: Instant,
    stream: bool,
    degraded: &str,
    cfg: &Config,
) -> RequestRow {
    let cost = usage.cost_usd.unwrap_or_else(|| {
        let t = cfg.tiers.get(&resolved.tier);
        usage.estimate_cost(
            t.map(|t| t.input_usd_per_m).unwrap_or(0.0),
            t.map(|t| t.output_usd_per_m).unwrap_or(0.0),
        )
    });
    RequestRow {
        ts: Utc::now(),
        profile: profile.name.clone(),
        key_prefix: key.prefix.clone(),
        user: user.to_string(),
        dialect: dialect.as_str().into(),
        tier: resolved.tier.clone(),
        model: resolved.model.clone(),
        status,
        input_tokens: usage.input_tokens,
        cache_read_tokens: usage.cache_read_tokens,
        cache_write_tokens: usage.cache_write_tokens,
        output_tokens: usage.output_tokens,
        cost_usd: cost,
        latency_ms: started.elapsed().as_millis() as i64,
        stream,
        degraded: degraded.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth;
    use wiremock::matchers::{bearer_token, body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    struct Harness {
        base: String,
        db_path: PathBuf,
        upstream: MockServer,
        http: reqwest::Client,
        _dir: tempfile::TempDir,
    }

    async fn harness(limits: Limits) -> Harness {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("b.db");
        Db::open(&db_path).unwrap();
        let cfg = Arc::new(
            Config::from_yaml(
                "profiles:\n  - {name: dev, tier: fast, daily_limit_usd: 1.0, monthly_soft_usd: 10.0}\n  - {name: car, tier: reasoning, daily_limit_usd: 0.2, monthly_soft_usd: 3.0}\n",
                "tiers:\n  fast: {model: deepseek/deepseek-v4-flash, fallback: qwen/qwen3.7-flash, input_usd_per_m: 0.04, output_usd_per_m: 0.08}\n  reasoning: {model: prism-ml/ternary-bonsai-2-27b, fallback: deepseek/deepseek-v4-pro, input_usd_per_m: 0.075, output_usd_per_m: 0.5}\n",
            )
            .unwrap(),
        );
        let upstream = MockServer::start().await;
        let env = HashMap::from([("OPENROUTER_KEY_DEV".to_string(), "sk-or-dev".to_string())]); // car has no upstream key on purpose
        let state = ProxyState::new(cfg, db_path.clone(), upstream.uri(), &env, limits);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(
                listener,
                router(state).into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap()
        });
        Harness {
            base: format!("http://{addr}"),
            db_path,
            upstream,
            http: reqwest::Client::new(),
            _dir: dir,
        }
    }

    fn issue(db_path: &std::path::Path, profile: &str, name: &str, ip: &str) -> String {
        let secret = auth::generate(profile);
        let db = Db::open(db_path).unwrap();
        db.insert_api_key(&ApiKey {
            id: 0,
            key_hash: auth::hash(&secret),
            prefix: auth::display_prefix(&secret),
            profile: profile.into(),
            name: name.into(),
            ip_allow: ip.into(),
            created_at: Utc::now(),
            expires_at: None,
            last_used_at: None,
            revoked_at: None,
        })
        .unwrap();
        secret
    }

    async fn wait_rows(db_path: &std::path::Path, n: usize) -> Vec<RequestRow> {
        for _ in 0..50 {
            let rows: Vec<RequestRow> = {
                let db = Db::open(db_path).unwrap();
                let mut stmt = db.conn_for_tests().prepare("SELECT profile, model, status, input_tokens, output_tokens, cost_usd, stream, degraded, user, tier FROM requests ORDER BY id").unwrap();
                stmt.query_map([], |r| {
                    Ok(RequestRow {
                        ts: Utc::now(),
                        profile: r.get(0)?,
                        key_prefix: String::new(),
                        user: r.get(8)?,
                        dialect: String::new(),
                        tier: r.get(9)?,
                        model: r.get(1)?,
                        status: r.get(2)?,
                        input_tokens: r.get(3)?,
                        cache_read_tokens: 0,
                        cache_write_tokens: 0,
                        output_tokens: r.get(4)?,
                        cost_usd: r.get(5)?,
                        latency_ms: 0,
                        stream: r.get::<_, i32>(6)? != 0,
                        degraded: r.get(7)?,
                    })
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
            };
            if rows.len() >= n {
                return rows;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("expected {n} request rows");
    }

    #[tokio::test]
    async fn no_key_bad_key_and_wrong_ip_are_rejected_in_the_clients_dialect() {
        let h = harness(Limits::default()).await;
        // no key, OpenAI dialect
        let r = h
            .http
            .post(format!("{}/v1/chat/completions", h.base))
            .json(&json!({"model": "brain/fast", "messages": []}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 401);
        let body: Value = r.json().await.unwrap();
        assert_eq!(body["error"]["type"], "invalid_api_key");
        // garbage key, Anthropic dialect
        let r = h
            .http
            .post(format!("{}/v1/messages", h.base))
            .header("x-api-key", "sk-ant-nope")
            .json(&json!({"model": "claude-x", "messages": []}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 401);
        assert_eq!(
            r.headers()
                .get("x-should-retry")
                .map(|v| v.to_str().unwrap()),
            Some("false")
        );
        let body: Value = r.json().await.unwrap();
        assert_eq!(body["type"], "error");
        assert_eq!(body["error"]["type"], "authentication_error");
        // well-formed but unknown
        let r = h
            .http
            .post(format!("{}/v1/chat/completions", h.base))
            .bearer_auth(auth::generate("dev"))
            .json(&json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 401);
        // right key, wrong IP
        let pinned = issue(&h.db_path, "dev", "pinned", "203.0.113.0/24");
        let r = h
            .http
            .post(format!("{}/v1/chat/completions", h.base))
            .bearer_auth(&pinned)
            .json(&json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 403);
        // …unless X-Forwarded-For says otherwise (Caddy in front)
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"choices": [], "usage": {"prompt_tokens": 1, "completion_tokens": 1}}),
            ))
            .mount(&h.upstream)
            .await;
        let r = h
            .http
            .post(format!("{}/v1/chat/completions", h.base))
            .bearer_auth(&pinned)
            .header("x-forwarded-for", "203.0.113.9, 10.0.0.1")
            .json(&json!({"model": "brain/fast"}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        // the upstream never saw a client key
        for req in h.upstream.received_requests().await.unwrap() {
            let a = req.headers.get("authorization").unwrap().to_str().unwrap();
            assert_eq!(a, "Bearer sk-or-dev");
        }
        // failures were audited
        let db = Db::open(&h.db_path).unwrap();
        let n: i64 = db
            .conn_for_tests()
            .query_row("SELECT COUNT(*) FROM auth_audit", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 4);
    }

    #[tokio::test]
    async fn forwards_openai_with_upstream_key_alias_resolution_fallbacks_and_records_usage() {
        let h = harness(Limits::default()).await;
        let key = issue(&h.db_path, "dev", "laptop", "");
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(bearer_token("sk-or-dev"))
            .and(body_partial_json(json!({"model": "deepseek/deepseek-v4-flash", "models": ["deepseek/deepseek-v4-flash", "qwen/qwen3.7-flash"], "usage": {"include": true}, "user": "u_7"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "gen-1", "model": "deepseek/deepseek-v4-flash", "choices": [{"message": {"role": "assistant", "content": "pong"}}], "usage": {"prompt_tokens": 100, "completion_tokens": 10, "cost": 0.0005}})))
            .expect(1)
            .mount(&h.upstream)
            .await;
        let r = h.http.post(format!("{}/v1/chat/completions", h.base)).bearer_auth(&key).json(&json!({"model": "brain/fast", "user": "u_7", "messages": [{"role": "user", "content": "ping"}]})).send().await.unwrap();
        assert_eq!(r.status(), 200);
        let body: Value = r.json().await.unwrap();
        assert_eq!(body["choices"][0]["message"]["content"], "pong");
        let rows = wait_rows(&h.db_path, 1).await;
        assert_eq!(
            (
                rows[0].profile.as_str(),
                rows[0].tier.as_str(),
                rows[0].status,
                rows[0].input_tokens,
                rows[0].output_tokens
            ),
            ("dev", "fast", 200, 100, 10)
        );
        assert!(
            (rows[0].cost_usd - 0.0005).abs() < 1e-12,
            "OpenRouter's cost wins over the estimate"
        );
        assert_eq!(rows[0].user, "u_7");
        assert!(!rows[0].stream);
    }

    #[tokio::test]
    async fn anthropic_stream_is_passed_through_sanitized_and_usage_recorded_at_the_end() {
        let h = harness(Limits::default()).await;
        let key = issue(&h.db_path, "dev", "cc", "");
        let sse = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":40,\"cache_read_input_tokens\":30,\"output_tokens\":1}}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"pong\"}}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":17}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
        Mock::given(method("POST"))
            .and(path("/messages"))
            .and(header("anthropic-version", "2023-06-01"))
            .and(body_partial_json(
                json!({"model": "deepseek/deepseek-v4-flash", "stream": true}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_raw(sse, "text/event-stream"))
            .expect(1)
            .mount(&h.upstream)
            .await;
        let r = h
            .http
            .post(format!("{}/v1/messages", h.base))
            .header("x-api-key", &key)
            .header("anthropic-version", "2023-06-01")
            .header("x-claude-code-session-id", "sess-1234567890abc")
            .json(&json!({"model": "claude-sonnet-5", "stream": true, "max_tokens": 100, "thinking": {"type": "adaptive"}, "context_management": {}, "messages": [{"role": "user", "content": "ping"}]}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        assert!(
            r.headers()
                .get("content-type")
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("text/event-stream")
        );
        let text = r.text().await.unwrap();
        assert_eq!(text, sse, "bytes pass through unchanged");
        // the sanitizer removed the fields before forwarding
        let sent: Value =
            serde_json::from_slice(&h.upstream.received_requests().await.unwrap()[0].body).unwrap();
        assert!(sent.get("thinking").is_none() && sent.get("context_management").is_none());
        assert_eq!(sent["max_tokens"], 100);
        let rows = wait_rows(&h.db_path, 1).await;
        assert_eq!(
            (rows[0].input_tokens, rows[0].output_tokens, rows[0].stream),
            (40, 17, true)
        );
        assert!(
            rows[0]
                .degraded
                .contains("sanitized:context_management,thinking.adaptive"),
            "{}",
            rows[0].degraded
        );
        assert_eq!(rows[0].user, "cc:sess-1234567");
        let expected = ((40.0 + 30.0 * 0.25) / 1e6) * 0.04 + 17.0 / 1e6 * 0.08;
        assert!(
            (rows[0].cost_usd - expected).abs() < 1e-15,
            "estimated from tier prices"
        );
    }

    #[tokio::test]
    async fn budget_degrades_then_blocks_with_the_documented_headers() {
        let h = harness(Limits::default()).await;
        let key = issue(&h.db_path, "dev", "k", "");
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"choices": [], "usage": {"prompt_tokens": 1, "completion_tokens": 1}}),
            ))
            .mount(&h.upstream)
            .await;
        let spend = |usd: f64| {
            let db = Db::open(&h.db_path).unwrap();
            db.insert_request(&RequestRow {
                ts: Utc::now(),
                profile: "dev".into(),
                key_prefix: "p".into(),
                user: String::new(),
                dialect: "openai".into(),
                tier: "fast".into(),
                model: "m".into(),
                status: 200,
                input_tokens: 0,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                output_tokens: 0,
                cost_usd: usd,
                latency_ms: 0,
                stream: false,
                degraded: String::new(),
            })
            .unwrap();
        };
        // 75% of the 1 $/day limit: reasoning requests are forced to fast, max_tokens untouched
        spend(0.75);
        let r = h
            .http
            .post(format!("{}/v1/chat/completions", h.base))
            .bearer_auth(&key)
            .json(&json!({"model": "brain/reasoning", "max_tokens": 9000}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let sent: Value = serde_json::from_slice(
            &h.upstream
                .received_requests()
                .await
                .unwrap()
                .last()
                .unwrap()
                .body,
        )
        .unwrap();
        assert_eq!(sent["model"], "deepseek/deepseek-v4-flash");
        assert_eq!(sent["max_tokens"], 9000);
        // 90%: capped
        spend(0.15);
        let r = h
            .http
            .post(format!("{}/v1/chat/completions", h.base))
            .bearer_auth(&key)
            .json(&json!({"model": "brain/reasoning", "max_tokens": 9000}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let sent: Value = serde_json::from_slice(
            &h.upstream
                .received_requests()
                .await
                .unwrap()
                .last()
                .unwrap()
                .body,
        )
        .unwrap();
        assert_eq!(sent["max_tokens"], budget::CAP_TOKENS);
        // 100%: blocked, retry-after ≥ 3600 and x-should-retry false so Claude Code shows it at once
        spend(0.10);
        let r = h
            .http
            .post(format!("{}/v1/chat/completions", h.base))
            .bearer_auth(&key)
            .json(&json!({"model": "brain/fast"}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 429);
        let ra: u64 = r
            .headers()
            .get("retry-after")
            .unwrap()
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        assert!(ra >= 3600, "{ra}");
        assert_eq!(r.headers().get("x-should-retry").unwrap(), "false");
        let body: Value = r.json().await.unwrap();
        assert!(
            body["error"]["message"]
                .as_str()
                .unwrap()
                .contains("budget exhausted")
        );
        let rows = wait_rows(&h.db_path, 5).await;
        assert_eq!(rows.iter().filter(|r| r.degraded == "fast-only").count(), 1);
        assert_eq!(
            rows.iter().filter(|r| r.degraded == "max-tokens").count(),
            1
        );
    }

    #[tokio::test]
    async fn per_key_rate_limit_and_ip_block_after_repeated_failures() {
        let h = harness(Limits {
            key_per_minute: 2,
            user_per_minute: 10,
            ip_fail_max: 3,
            ip_window: Duration::from_secs(600),
            key_cache_ttl: Duration::from_secs(60),
        })
        .await;
        let key = issue(&h.db_path, "dev", "k", "");
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"choices": [], "usage": {}})),
            )
            .mount(&h.upstream)
            .await;
        let call = || {
            h.http
                .post(format!("{}/v1/chat/completions", h.base))
                .bearer_auth(&key)
                .json(&json!({"model": "brain/fast"}))
                .send()
        };
        assert_eq!(call().await.unwrap().status(), 200);
        assert_eq!(call().await.unwrap().status(), 200);
        let r = call().await.unwrap();
        assert_eq!(r.status(), 429);
        assert_eq!(r.headers().get("x-should-retry").unwrap(), "true");
        // three bad keys from the same IP → blocked even with a good key
        for _ in 0..3 {
            h.http
                .post(format!("{}/v1/chat/completions", h.base))
                .bearer_auth("brain_dev_wrongwrongwrongwrongwrongwrongwrong")
                .json(&json!({}))
                .send()
                .await
                .unwrap();
        }
        let r = h
            .http
            .get(format!("{}/v1/models", h.base))
            .bearer_auth(&key)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 429);
        assert!(
            r.text()
                .await
                .unwrap()
                .contains("too many failed authentications")
        );
    }

    #[tokio::test]
    async fn models_lists_aliases_and_profile_without_upstream_key_gets_503() {
        let h = harness(Limits::default()).await;
        let dev = issue(&h.db_path, "dev", "k", "");
        let r = h
            .http
            .get(format!("{}/v1/models", h.base))
            .bearer_auth(&dev)
            .send()
            .await
            .unwrap();
        let body: Value = r.json().await.unwrap();
        let ids: Vec<&str> = body["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["id"].as_str().unwrap())
            .collect();
        assert!(
            ids.contains(&"brain/fast")
                && ids.contains(&"brain/reasoning")
                && ids.contains(&"deepseek/deepseek-v4-flash")
        );
        let car = issue(&h.db_path, "car", "prod", "");
        let r = h
            .http
            .post(format!("{}/v1/chat/completions", h.base))
            .bearer_auth(&car)
            .json(&json!({"model": "brain/fast"}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 503);
        // upstream errors pass through unmodified
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(402)
                    .set_body_string(r#"{"error":{"message":"Insufficient credits","code":402}}"#),
            )
            .mount(&h.upstream)
            .await;
        let r = h
            .http
            .post(format!("{}/v1/chat/completions", h.base))
            .bearer_auth(&dev)
            .json(&json!({"model": "brain/fast"}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 402);
        assert!(r.text().await.unwrap().contains("Insufficient credits"));
        assert_eq!(
            h.http
                .head(format!("{}/api/hello", h.base))
                .send()
                .await
                .unwrap()
                .status(),
            204
        );
    }
}
