---
title: Phase 1 runbook
sidebar_position: 4
---

# Phase 1 runbook: the proxy

Built on 2026-09-20. From now on every LLM request from the CLIs and the
apps goes through `brain serve` on the VPS, and **no `/v1/*` route answers
without a client key**.

## What exists

| Piece | Where | Tests |
|-------|-------|-------|
| Client keys `brain_<profile>_<random>`: sha256 only, name, optional expiry and IP/CIDR allowlist, revoke, audit of failures | `crates/brain/src/auth.rs`, `db.rs` | generation/shape, log prefix never leaks the token, revocation/expiry/IP checks, unique active name per profile |
| Endpoints `/v1/chat/completions`, `/v1/models` (OpenAI); `/v1/messages`, `/v1/messages/count_tokens` (Anthropic); `HEAD /api/hello` | `proxy/mod.rs` | end-to-end against a mock OpenRouter |
| Middleware chain: header → syntax → hash lookup (60 s cache) → revoked/expired/IP → per-key and per-user rate limit → budget → forward → record | `proxy/mod.rs` | 401/403 in the client's dialect, `x-should-retry: false`, X-Forwarded-For, IP block after repeated failures, upstream key never sent to the client |
| Budget ring 2: daily and monthly spend from `requests`; 70% → fast only, 85% → `max_tokens` cap, 100% → 429 with `retry-after ≥ 3600` + `x-should-retry: false` | `budget.rs` | thresholds, retry-after at the window reset, effective tier |
| Streaming pass-through with a tap that records usage at end-of-stream or client disconnect | `proxy/tap.rs` | complete and interrupted streams |
| Usage in both dialects, streamed or not; OpenRouter's `cost` when present, tier-price estimate otherwise | `proxy/usage_parse.rs` | OpenAI/Anthropic bodies and SSE, OpenRouter's `message_delta` shape |
| Model resolution (`brain/<tier>`, `claude-*` → profile tier, explicit ids), sanitizer for Claude Code fields, `models[]` fallbacks, `max_tokens` cap, end-user id (Claude Code's metadata reduced to its session id) | `proxy/sanitize.rs` | exact field lists |
| `brain keys create\|list\|revoke`; OpenRouter provisioning renamed to `brain upstream provision\|list` | `main.rs` | — |
| Board section "Proxy · day × profile × model" | `dashboard.rs` | summary shape |

Verified live on 2026-09-20: curl in both dialects and **Claude Code
headless through the proxy** (`pong`, 2 turns, 6.2 s, 20.8k input tokens,
0.00075 $ per request, sanitizer removed `output_config` and
`thinking.adaptive` even with the client flags set).

## Issue a key (on the server, admin only)

```bash
ssh root@<vps> 'cd /opt/llm_brain && sudo -u brain ./brain keys create --profile dev --name laptop [--expires 2027-01-01] [--ip 1.2.3.4/32]'
```
Printed once. `brain keys list` shows prefixes only; `brain keys revoke
--profile dev --name laptop` cuts it (the proxy forgets cached keys within
60 s).

## Point the clients at the proxy

Same variables as Phase 0, different base URL and key:

```bash
# Claude Code
ANTHROPIC_BASE_URL=https://brain.<host>/  ANTHROPIC_AUTH_TOKEN=brain_dev_…  ANTHROPIC_MODEL=brain/fast  ANTHROPIC_DEFAULT_HAIKU_MODEL=brain/fast
# aider
OPENAI_API_BASE=https://brain.<host>/v1   OPENAI_API_KEY=brain_dev_…   aider --architect --model openai/brain/reasoning --editor-model openai/brain/fast
# apps (OpenAI SDK)
OpenAI(base_url="https://brain.<host>/v1", api_key="brain_car_…").chat.completions.create(model="brain/fast", user=user_id, …)
```
`brain/fast` and `brain/reasoning` are stable aliases: the model behind
them is `config/tiers.yaml` on the server. Fallback chains and edit
formats for aider come from `brain setup aider` as before; the `models[]`
fallback is added by the proxy for the OpenAI dialect.

## What the VPS exposes

| Route | Auth |
|-------|------|
| `/v1/*`, `/api/hello` | client key (Bearer or `x-api-key`) — no exceptions |
| `/`, `/api/summary` (board) | Caddy basic auth |
| `/health` | open, returns `ok` |

Rate limits: 60 req/min per key, 10 req/min per (key, end user); 20 auth
failures in 10 minutes block the IP for the window. Ring 1 (the
OpenRouter key's own daily limit) still holds if all of this fails.

## Agents compared through the proxy

2026-09-20, task `ago-0001`, model deepseek-v4-pro via `brain/agent`:

| Tool | Result | Time | Turns / tool calls | Cost (proxy) |
|------|--------|------|--------------------|--------------|
| Claude Code (`px-claude`) | **PASS** | 120 s | 14 turns | ~0.03 $ |
| OpenCode (`brain bench --tool opencode`) | **PASS** | 41 s | 3 tool calls (grep, read, edit) | ~0.005 $ |
| OpenCode on `fast` (deepseek-v4-flash, direct) | PASS | 58 s | 10 tool calls | 0.001 $ |
| aider, architect bonsai + editor deepseek-v4-flash (direct) | PASS | 74 s | — (no tools) | < 0.001 $ |

Before the system-turn fix, Claude Code on non-Claude models failed this
task with an empty reply; through the proxy it now completes it. OpenCode
is the open-source agent that matches Claude Code's autonomy at a fraction
of the tokens (its system prompt is far smaller). Both are usable through
the proxy today; `brain bench run --via-proxy <url>` runs the suite the way
the clients actually use it (client key of the `benchmark` profile).

### `:exacto` A/B (2026-09-22, Claude Code through the proxy, two runs each)

| Model | Run 1 | Run 2 | Cost of both runs (proxy, real) |
|-------|-------|-------|---------------------------------|
| `deepseek/deepseek-v4-pro` | PASS, 72 s, 9 turns | PASS, 345 s, 17 requests | 0.22 $ |
| `deepseek/deepseek-v4-pro:exacto` | PASS, 123 s, 20 turns | PASS, 154 s | 0.14 $ |

All four pass; the spread between two runs of the *same* model (72 s vs
345 s) is larger than any difference between the two sides, and every
request landed on the same provider (StreamLake) either way. For this
model Auto Exacto is already what the default route does, so the suffix
changes nothing today; it stays available as a config switch
([Provider routing](./architecture/api-layer.md#provider-routing-exacto)).
Per-task cost is now ≈ 0.07–0.12 $, up from 0.03 $ on 2026-09-20: the
catalog price of `deepseek-v4-pro` doubled in between (`tiers.yaml`
updated).

## Known gaps

- No `fallbacks` on the Anthropic dialect yet (OpenRouter uses a different
  parameter there); the `dev` tier fallback applies to aider/OpenAI calls.
- Semantic L2 cache and escalation on failure are Phase 2.
- `count_tokens` is forwarded as-is; OpenRouter may not implement it, and
  Claude Code then estimates locally.
