---
title: agent-orchestrator — what to reuse
---

# agent-orchestrator: from backend to client, and what to reuse

Decision (2026-09-19): **llm_brain is a single, standalone backend**;
agent-orchestrator does not contain it, **it uses it**, like the apps.
Reason: centralize every LLM request in one place (budget, cache, usage),
independent of who makes it.

Repo: [pjcau/agent-orchestrator](https://github.com/pjcau/agent-orchestrator),
verified on the code (clone of 2026-09-19).

## How agent-orchestrator becomes a client

`providers/openai.py` with `base_url = llm_brain` and the `ago` profile's
`api_key`. Everything else (dashboard, graph, agent runtime) is unchanged.
Its `providers/openrouter.py` becomes redundant: only llm_brain sees
OpenRouter.

## What to copy into llm_brain (don't depend on it)

Standalone means standalone: the useful modules are copied and adapted,
the package is not imported. They are small.

| Module | Lines | What it gives | What's missing |
|--------|-------|---------------|----------------|
| `core/usage.py` | 287 | `UsageRecord`, `BudgetConfig.max_per_day`, `UsageTracker.check_budget` | per-profile, monthly, SQLite persistence, degradation |
| `core/cache.py` | 327 | `BaseCache`, `CachePolicy`, `CacheStats.hit_rate`, `InMemoryCache` | SQLite backend, key from the normalized request |
| `providers/openrouter.py` | — | `cache_control` injection for cacheable prefixes | reading `cached_tokens`, `session_id` |
| `core/evaluator.py` + `dashboard/evals_routes.py` | 584 + 267 | `EvalCase`/`EvalSuite`, `POST /api/evals/run`, `compare` | "run the tests" verifier, report persistence, cron |
| `core/router.py` | — | regex classifier | not needed in Phase 1 (routing by alias + escalation) |

## What exists nowhere (grep over all of `src/`: zero matches)

- `POST /v1/chat/completions`, `POST /v1/messages`: the two dialects. They
  are the heart of llm_brain and must be written.

{/* diagram: 05-agent-orchestrator-modules */}
```mermaid
flowchart LR
    subgraph AO["agent-orchestrator (unchanged, becomes a client)"]
        A1["providers/openai.py<br/>base_url = llm_brain, api_key = 'ago' profile"]
        A2["dashboard, graph, agent runtime…<br/>unchanged"]
    end

    subgraph REUSE["From agent-orchestrator: copy and adapt into llm_brain"]
        R1["core/usage.py<br/>UsageRecord · BudgetConfig.max_per_day · UsageTracker"]
        R2["core/cache.py<br/>BaseCache · CachePolicy · CacheStats → + SQLite backend"]
        R3["providers/openrouter.py<br/>cache_control injection"]
        R4["core/evaluator.py + evals_routes.py<br/>EvalCase · EvalSuite · compare"]
    end

    subgraph NEW["llm_brain — its own repo"]
        N1["api/openai_compat.py<br/>POST /v1/chat/completions · GET /v1/models"]
        N2["api/anthropic_compat.py<br/>POST /v1/messages · count_tokens"]
        N3["core/profiles.py<br/>api_key → profile: tier, daily/monthly budget, cache policy, versioned system prompt"]
        N4["core/tiers.py<br/>brain/* aliases → (provider, model); claude-* → tier"]
        N5["core/budget.py<br/>daily + monthly, 70/85/100 degradation, pre-call estimate"]
        N6["core/cache_l2.py<br/>exact SQLite (semantic later)"]
        N7["core/escalation.py<br/>failure → higher tier (Phase 2)"]
        N8["bench/<br/>real-bug suite + cron + promotion"]
        DB[("SQLite<br/>usage · budget · cache · eval")]
    end

    A1 --> N1
    N1 & N2 --> N3 --> N5 --> N4 --> N6
    R1 -.-> N5
    R2 -.-> N6
    R3 -.-> N4
    R4 -.-> N8
    N5 & N6 & N8 --> DB
```
