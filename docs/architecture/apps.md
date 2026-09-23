---
title: App integration
---

# Integration with the existing apps

Every client is a **profile** in `config/profiles.yaml` with its own
client key, budget and default tier; the key decides the profile. No
custom SDK: the official OpenAI or Anthropic SDK (or the `claude` CLI)
with `base_url` = llm_brain and the app's key ([Compatibility](./client-compatibility.md)).

| App | Profile | Status | Budget (day / month soft) |
|-----|---------|--------|---------------------------|
| **find-a-car** (second-hand cars: valuation, extraction from listings) | `car` | **integrated** (2026-09-20) | 0.50 $ / 15 $ |
| **assistant** (local chat, RAG) | `assistant` | client key issued, own `brain/auto` router configured | 0.50 $ / 15 $ |
| agent-orchestrator | `ago` | profile and upstream key only, not wired | 1 $ / 5 $ |
| dev tooling (Claude Code, aider) | `dev` | daily use through `px-claude` / `px-aider` | 5 $ / 60 $ |
| second-hand market | — | later | — |

{/* diagram: 04-app-integration */}
```mermaid
flowchart LR
    subgraph APPS["Clients"]
        FAC["find-a-car (integrated)<br/>skills run claude via the proxy"]
        AST["assistant (key issued)<br/>chat, RAG"]
        SHM["second-hand market (later)"]
        AGO["agent-orchestrator (not yet wired)"]
        DEV["Dev tooling<br/>Claude Code · aider · OpenCode"]
    end

    subgraph SDK["How they integrate"]
        S1["Official SDK<br/>base_url = llm_brain<br/>api_key = brain_&lt;profile&gt;_…"]
        S2["user = opaque end-user id<br/>x-brain-session = conversation (brain/auto)"]
    end

    subgraph BRAIN["llm_brain profiles (config/profiles.yaml)"]
        P1["car<br/>fast · 0.50 $/day · 15 $/m soft"]
        P2["assistant<br/>own brain/auto ladder · 0.50 $/day · 15 $/m soft"]
        P3["dev<br/>brain/auto via px-claude · 5 $/day · 60 $/m soft"]
        P4["ago<br/>1 $/day · 5 $/m soft"]
        P5["benchmark<br/>0.50 $/day · 5 $/m soft"]
        USG["requests table → board"]
    end

    FAC & AST & SHM & AGO & DEV --> S1 --> S2
    S2 --> P1 & P2 & P3 & P4 & P5
    P1 & P2 & P3 & P4 & P5 --> USG
```

## find-a-car

The app's skills run the `claude` CLI as a subprocess. `claude_env()` in
the app sets `ANTHROPIC_BASE_URL` / `ANTHROPIC_AUTH_TOKEN` for that
subprocess from `BRAIN_BASE_URL` / `BRAIN_CAR_KEY` in the app's
git-ignored `.env`, so every skill run is a `car` request through the
proxy (verified live: one skill run from the Docker app = 4 proxied
requests). The change itself was made by aider through the proxy for
0.002 $.

## assistant

Its own router in `profiles.yaml` replaces the global coding-agent ladder
for [`brain/auto`](./auto-routing.md): `fast` for greetings and lookups,
`medium` for summaries and drafting, `agent` for long analysis; fallback
`fast`. The app should name each conversation with the `x-brain-session`
header so the tier is decided once per conversation; the app-side wiring
is not recorded here yet.

## What is not built for apps

- **Server-side, versioned system prompts** per profile
  ([why](./cache-logic.md)): designed, not built (`config/prompts/` is
  empty).
- **L2 response cache**: `l2_cache: exact` is set for `assistant` and
  `car` but is only a config field today; nothing is cached server-side.
