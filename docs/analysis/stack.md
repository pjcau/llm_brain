---
title: Recommended stack
---

# Recommended stack

Criterion: the minimum that supports budget, benchmark and the two
dialects, reusing agent-orchestrator. No pieces "for the future".

| Layer | Choice | Why |
|-------|--------|-----|
| Repo | **`llm_brain`, standalone** | single backend; agent-orchestrator and the apps are clients |
| Language / server | **Python 3.12 + FastAPI** | same stack as agent-orchestrator: copied modules come in without rewriting |
| HTTP client | async `httpx` with streaming | SSE pass-through without buffering |
| Primary provider | **OpenRouter, both dialects native** | it exposes both `/api/v1/chat/completions` and `/api/v1/messages` (Anthropic format, verified): Claude Code and aider talk to it **directly** |
| The layer in Phase 1 | **an aware reverse proxy**, not a translator | it does: auth → profile → OpenRouter key, alias → model, budget, usage, logs. It does not convert formats |
| Format translator | **deferred** | needed only for providers that don't speak the two dialects (local). No longer the main risk |
| Sanitizer for Claude Code | **yes, small** | strips `thinking: adaptive`, `context_management`, `output_config`, beta tool fields that non-Claude models reject; maintained at every release ([details](../architecture/client-compatibility.md)) |
| Persistence | **SQLite** (usage, budget counters, eval reports) | single user; `usage_db.py` is Postgres with fallback: add a SQLite backend. Postgres only if already in docker |
| Config | existing YAML (`orchestrator.yaml`) + `tiers:` and `profiles:` sections | one source |
| Benchmark scheduler | system cron / systemd timer → `POST /api/evals/run` | zero dependencies |
| CLI | aider (pip) **and** Claude Code with `ANTHROPIC_BASE_URL` | together in Phase 0; the benchmark measures who burns less and gets there first |
| Dashboard | **minimal, in llm_brain**: budget/usage/cache/benchmark page. agent-orchestrator's dashboard stays its own | don't make llm_brain depend on a client's frontend |
| Tests | pytest + `providers/mock.py` | already there |
| Secrets | `.env` 0600 locally; sops+age if llm_brain goes to another machine; client keys hashed in SQLite | [Secrets](../architecture/secrets.md) |
| Exposure | `127.0.0.1` until the location is decided; then Tailscale or Cloudflare Tunnel | [decision](../decisions.md) |
| Containers | not needed locally | the layer is a Python process |

## Why OpenRouter directly in Phase 0 (no LiteLLM)

- Budget: OpenRouter keys have a `limit` with daily reset; LiteLLM enforces
  budgets **only with Postgres**. Fewer pieces, same control.
- Dialects: OpenRouter already speaks Anthropic and OpenAI; LiteLLM isn't
  needed to translate.
- What you lose: fallback and routing in Phase 0 → done by hand (`/model`
  in aider, aliases in Claude Code) for a week. Acceptable.

LiteLLM remains useful later as a library, if and when a translator for
non-compatible providers is needed.

## Repo structure (decided on 2026-09-19)

```
llm_brain/
  src/llm_brain/
    api/         openai_compat.py · anthropic_compat.py
    core/        profiles.py · tiers.py · budget.py · usage.py · cache_l2.py · escalation.py
    providers/   openrouter.py · openai_compat.py · (local.py, later)
    db/          sqlite.py (usage · budget · cache · eval)
  bench/         JSON suite + test verifier + cron
  config/        tiers.yaml · profiles.yaml · prompts/<profile>@<version>.md  (no secrets: only env var names)
  .env           per-profile OpenRouter keys, 0600, not committed  → [Secrets](../architecture/secrets.md)
  website/       this Docusaurus site (today at the root)
  tests/
```

## On Phase 3 (GPU)

It is a **wish, not a plan**. The only cost it brings today is a
zero-cost rule: *no code outside `providers/` knows the provider*.
Nothing else is designed for the GPU.
