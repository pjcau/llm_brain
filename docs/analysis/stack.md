---
title: Stack
---

# Stack

Criterion: the minimum that supports budget, benchmark and the two
dialects. No pieces "for the future".

:::note Decided on 2026-09-19: everything in Rust
**All code in llm_brain is Rust** — the proxy, the CLI, the benchmark
runner, the dashboard endpoints, even the docs tooling
(`tools/sync-diagrams`). The first draft of this page proposed
Python/FastAPI to copy agent-orchestrator's modules verbatim; those
modules are small (a few hundred lines each) and get **ported** instead.
What Rust buys for a streaming proxy that must run for months on a €4
VPS: a single static binary, ~20–40 MB of RAM, no runtime to patch, and
back-pressure-correct streaming for free with tokio.
:::

| Layer | Choice | Why |
|-------|--------|-----|
| Repo | **`llm_brain`, standalone**, one Cargo workspace | single backend; agent-orchestrator and the apps are clients |
| Language | **Rust (stable), edition 2024** — no other language in the repo | single binary, low memory, safe concurrency; agent-orchestrator already ships a Rust/PyO3 component, so the toolchain is familiar |
| HTTP server | **axum** on **tokio** + **tower-http** (timeouts, request ids; compression off for SSE) | the de-facto standard, first-class streaming bodies |
| HTTP client | **reqwest** with `stream` feature (rustls, HTTP/2) | SSE pass-through chunk by chunk, no buffering |
| JSON | **serde / serde_json** with `Value` for pass-through bodies | the sanitizer edits a `Value` tree; typed structs only where the layer must read fields (model, `user`, `max_tokens`, usage) |
| Persistence | **SQLite via `rusqlite`** (bundled, WAL) | single file: usage, budget counters, keys (hashed), L2 cache, eval reports |
| Config | **YAML** (`serde_yaml`): `tiers.yaml`, `profiles.yaml`; secrets only as env var names | one source |
| CLI (`brain serve | keys | bench`) | **clap** (derive) | one binary, subcommands |
| Secrets at runtime | env vars (from `.env` 0600 via `dotenvy`, or systemd `LoadCredential`) | [Secrets](../architecture/secrets.md) |
| Hashing / randomness | `sha2`, `rand` (keys), `subtle` for constant-time compare | [Authentication flow](../architecture/auth-flow.md) |
| Rate limiting | in-memory token buckets (`governor`) per key and per (key, user) | no external service |
| Primary provider | **OpenRouter, both dialects native** | it exposes both `/api/v1/chat/completions` and `/api/v1/messages` (Anthropic format, verified): Claude Code and aider talk to it **directly** |
| The layer in Phase 1 | **an aware reverse proxy**, not a translator | auth → profile → OpenRouter key, alias → model, budget, usage, logs. It does not convert formats |
| Format translator | **deferred** | only for providers that don't speak the two dialects (local) |
| Sanitizer for Claude Code | **yes, small** | strips `thinking: adaptive`, `context_management`, `output_config`, beta tool fields on a `serde_json::Value`; maintained at every release ([details](../architecture/client-compatibility.md)) |
| Benchmark runner | `brain bench` (drives `aider`/`claude` headless, runs `verify`, reads usage from SQLite); git mining via `git2` | one toolchain |
| Scheduler | system cron / systemd timer → `brain bench run` | zero dependencies |
| Dashboard | **minimal, served by axum**: a static page + JSON endpoints (budget/usage/cache/benchmark); Mermaid/Chart.js from a CDN | no frontend build step |
| Skills, agents, hooks for the dev workflow | [**claude-kit**](https://github.com/pjcau/claude-kit) as a git submodule (`.claude-kit/`) | portable Claude Code skills/agents/hooks, reused instead of re-written; they are client-side and unaffected by the proxy ([why](../architecture/claude-code-aider.md)) |
| Tests | `cargo test` + a mock upstream (`wiremock`) | protocol tests for both dialects, sanitizer fixtures per Claude Code release |
| Observability | `tracing` + `tracing-subscriber` (JSON logs), keys redacted | |
| TLS / edge | **Caddy** in front (automatic HTTPS), llm_brain on `127.0.0.1:8080` | no TLS code in the binary |
| Docs tooling | `tools/sync-diagrams` (Rust, std only) copies `diagrams/*.mmd` into the pages | the "all Rust" rule applies to helpers too |
| Containers | not needed: one binary + systemd unit | |

## Why OpenRouter directly in Phase 0 (no LiteLLM)

- Budget: OpenRouter keys have a `limit` with daily reset; LiteLLM enforces
  budgets **only with Postgres**. Fewer pieces, same control.
- Dialects: OpenRouter already speaks Anthropic and OpenAI; nothing to
  translate.
- What you lose: fallback and routing in Phase 0 → done by hand (`/model`
  in aider, aliases in Claude Code) for a week. Acceptable.

## What "porting from agent-orchestrator" means now

| Python module | Rust module | Notes |
|---------------|-------------|-------|
| `core/usage.py` (287 lines) | `budget.rs` + `usage.rs` | `BudgetConfig`, daily/monthly counters, degradation thresholds |
| `core/cache.py` (327 lines) | `cache_l2.rs` | trait `Cache`, SQLite backend, key = hash of the normalized request |
| `providers/openrouter.py` | `providers/openrouter.rs` | `cache_control` injection, `session_id`, usage parsing (`cached_tokens`, `cache_discount`) |
| `core/evaluator.py` + `evals_routes.py` | `bench/` (suite, runner, compare) | test-based verifier instead of an LLM judge |

## Repo structure (updated for Rust)

```
llm_brain/
  Cargo.toml                 workspace
  crates/
    brain/                   the binary: `brain serve | keys | bench`
      src/
        main.rs
        api/                 openai_compat.rs · anthropic_compat.rs · models.rs · health.rs
        auth/                middleware.rs · keys.rs (hash, lookup cache, audit)
        core/                profiles.rs · tiers.rs · budget.rs · usage.rs · cache_l2.rs · escalation.rs · sanitizer.rs
        providers/           openrouter.rs · openai_compat.rs · (local.rs, later)
        db/                  sqlite.rs · migrations/
        bench/               suite.rs · runner.rs · compare.rs
        dashboard/           routes.rs · static/
  tools/
    sync-diagrams/           docs helper (Rust)
  config/                    tiers.yaml · profiles.yaml · prompts/<profile>@<version>.md  (no secrets)
  bench/tasks/               task YAMLs
  deploy/                    Caddyfile · brain.service · litestream.yml
  .claude-kit/               git submodule: skills, agents, hooks for Claude Code
  website/                   this Docusaurus site (today at the root)
  .env                       per-profile OpenRouter keys, 0600, not committed
```

## What Rust costs, honestly

- **Iteration speed on the sanitizer**: Claude Code adds fields every
  release; editing a `serde_json::Value` is easy, but each change ships
  as a rebuild + restart (seconds, not a problem on a single VPS).
- **Ecosystem for LLM plumbing**: thinner than Python's (no LiteLLM to
  borrow from). Irrelevant while the layer is a pass-through proxy;
  relevant only if the format translator is ever needed.
- **Semantic L2 cache**: local embeddings would mean `candle`/`ort`;
  simpler to call an embedding model via OpenRouter. Same conclusion as
  before.

## On Phase 3 (GPU)

It is a **wish, not a plan**. The only cost it brings today is a
zero-cost rule: *no code outside `providers/` knows the provider*.
Nothing else is designed for the GPU.
