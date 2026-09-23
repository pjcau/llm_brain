---
title: Stack
---

# Stack

Criterion: the minimum that supports budget, benchmark and the two
dialects. No pieces "for the future".

:::note Decided on 2026-09-19: everything in Rust
**All code in llm_brain is Rust** — the proxy, the CLI, the benchmark
runner, the board, the docs tooling (`tools/sync-diagrams`). The useful
agent-orchestrator modules were small and got **ported** in spirit, not
imported ([what was reused](./agent-orchestrator.md)). What Rust buys for a
streaming proxy that runs for months on a small VPS: a single static
binary, ~20–40 MB of RAM, no runtime to patch, back-pressure-correct
streaming with tokio.
:::

| Layer | Choice | Why |
|-------|--------|-----|
| Repo | **`llm_brain`, standalone**, one Cargo workspace (edition 2024) | single backend; agent-orchestrator and the apps are clients |
| HTTP server | **axum** on **tokio** | first-class streaming bodies |
| HTTP client | **reqwest** with `stream` (rustls) | SSE pass-through chunk by chunk, no buffering |
| JSON | **serde / serde_json**, `Value` for pass-through bodies | the sanitizer edits a `Value` tree; typed structs only where fields are read (model, `max_tokens`, usage) |
| Persistence | **SQLite via `rusqlite`** (bundled, WAL) | one file: usage snapshots, requests, events, keys (hashed), bench runs |
| Config | **YAML** (`serde_yaml`): `config/tiers.yaml`, `config/profiles.yaml`; secrets only as env var names | [Configuration](../configuration.md) |
| CLI | **clap** (derive), one binary `brain` | subcommands below |
| Secrets at runtime | env vars from `.env` (0600, `dotenvy`) | [Secrets](../architecture/secrets.md) |
| Hashing / compare | `sha2`, `rand` (keys), `subtle` (constant-time) | [Authentication flow](../architecture/auth-flow.md) |
| Rate limiting | in-memory token buckets (`governor`) per key | no external service |
| Upstream | **OpenRouter, both dialects native** (`/chat/completions` and `/messages`) | nothing to translate |
| The layer | **an aware reverse proxy**, not a translator | auth → profile → upstream key, alias → model, budget, usage. No format conversion; a translator is deferred until a provider that speaks neither dialect (local GPU) |
| Sanitizer for Claude Code | small, on a `serde_json::Value` | strips fields other models reject ([details](../architecture/client-compatibility.md)) |
| Routing | `brain/auto`: a decision model picks the tier once per session | [Auto-routing](../architecture/auto-routing.md) |
| Benchmark | `brain bench run --tool aider\|claude\|opencode`, tools headless, `verify` = the repo's tests | [Benchmark](../architecture/benchmark.md) |
| Board | HTML page + JSON (`/api/summary`) served by `brain serve` | no frontend build step |
| Tests | `cargo test` + `wiremock`; real tools in Docker via `testcontainers` (feature `docker-tests`) | protocol tests for both dialects, sanitizer fixtures |
| TLS / edge | **Caddy** in front, `brain` on `127.0.0.1:8080`, systemd unit | no TLS code in the binary ([Deploy](../deploy-vps.md)) |
| Dev-workflow skills | [**claude-kit**](https://github.com/pjcau/claude-kit) submodule (`.claude-kit/`) | client-side, unaffected by the proxy ([why](../architecture/claude-code-aider.md)) |
| Docs tooling | `tools/sync-diagrams` copies `diagrams/*.mmd` into the pages | "all Rust" applies to helpers too |

## Why OpenRouter directly (no LiteLLM)

- Budget: OpenRouter keys have a `limit` with daily reset (ring 1 of the
  [budget](../architecture/budget.md)); LiteLLM enforces budgets only with
  Postgres.
- Dialects: OpenRouter already speaks Anthropic and OpenAI.

## Repo layout

```
llm_brain/
  Cargo.toml                 workspace: crates/brain, tools/sync-diagrams
  crates/brain/
    src/
      main.rs                CLI (clap)
      config.rs              profiles.yaml + tiers.yaml
      auth.rs                client keys: brain_<profile>_…, sha256, expiry, IP allowlist
      keys.rs                OpenRouter key per profile (provision, daily limit)
      openrouter.rs          OpenRouter client: key provisioning, GET /key
      catalog.rs             OpenRouter model catalog: max_tokens caps, prices
      budget.rs              ring 2: daily/monthly spend, degradation
      usage.rs               usage snapshots and reports
      events.rs              ingest aider / Claude Code logs into `events`
      db.rs                  SQLite (WAL)
      dashboard.rs           `brain serve`: the board + background refresh
      setup.rs               `brain setup claude-code|aider`
      proxy/                 mod.rs (middleware chain) · route.rs (brain/auto)
                             sanitize.rs · tap.rs (SSE usage) · usage_parse.rs
      bench/                 mod.rs · runner.rs · task.rs · tool.rs
    tests/                   docker-tests (testcontainers)
  tools/sync-diagrams/       docs helper
  config/                    tiers.yaml · profiles.yaml (no secrets)
  bench/tasks/               task YAMLs
  deploy/                    Caddyfile · brain.service · cloud-init.yaml · litestream.yml (not installed yet)
  docker/                    brain.Dockerfile · aider/claude/opencode test images
  diagrams/                  *.mmd, the source of the page diagrams
  docs/ · src/ · static/     this Docusaurus site (package.json at the root)
  .claude-kit/               git submodule
  .env                       per-profile keys, 0600, not committed
```

CLI: `brain upstream provision|sync|list` · `keys create|list|revoke` ·
`usage snapshot|report` · `models` · `setup claude-code|aider` · `serve` ·
`events ingest|report` · `bench run|report`.

## What Rust costs, honestly

- **Sanitizer iteration**: Claude Code adds fields every release; each fix
  ships as rebuild + restart (seconds on a single VPS).
- **LLM plumbing ecosystem**: thinner than Python's. Irrelevant while the
  layer is a pass-through; relevant only if a format translator is needed.
- **Semantic L2 cache** (not built): local embeddings would mean
  `candle`/`ort`; simpler to call an embedding model via OpenRouter.

## On Phase 3 (GPU)

A **wish, not a plan** ([GPU](../architecture/gpu.md)). Its only cost today
is a rule: the upstream is chosen per tier in config, so adding a local
provider changes config and one client, not the callers.
