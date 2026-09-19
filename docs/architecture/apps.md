---
title: App integration
---

# Integration with the existing apps

Order decided (2026-09-19): **assistant and find-a-car first**, then the
second-hand market. agent-orchestrator integrates the same way, with the
`ago` profile. The repos are not public: **name and stack to be
confirmed** ([open decision](../decisions.md)).

For each of the first two apps three things are needed: the profile, the
**versioned system prompt** kept by the layer ([why](./cache-logic.md)),
and the list of LLM calls the app makes today (to estimate the budget).

The integration model is the same for all and needs no custom SDK.

{/* diagram: 04-app-integration */}
```mermaid
flowchart LR
    subgraph APPS["Existing apps (GitHub)"]
        SHM["Second-hand market (later)<br/>listing classification, descriptions, moderation"]
        FAC["Find-a-car second hand (first)<br/>valuation, comparison, extraction from listings"]
        AST["Assistant (first)<br/>conversational chat, RAG"]
        AGO["agent-orchestrator<br/>openai-compat provider"]
        DEV["Dev tooling<br/>Claude Code · aider"]
    end

    subgraph SDK["How they integrate"]
        S1["Official OpenAI SDK<br/>base_url = llm_brain<br/>api_key = per-app key"]
        S2["X-Brain-Profile header<br/>or api_key → profile"]
    end

    subgraph BRAIN["llm_brain"]
        PRF["Per-client profiles"]
        P1["profile: market<br/>fast tier · L2 semantic ON · budget 5€/m"]
        P2["profile: car<br/>fast + reasoning for valuations · budget 10€/m"]
        P3["profile: assistant<br/>fast tier · L2 exact ON · streaming"]
        P4["profile: dev<br/>fast tier, reasoning escalation · L2 OFF · 3€/day, 30€/m"]
        P5["profile: ago<br/>like dev, own budget"]
        USG["Usage per profile → llm_brain dashboard"]
    end

    SHM --> S1
    FAC --> S1
    AST --> S1
    DEV --> S1
    AGO --> S1
    S1 --> S2 --> PRF
    PRF --> P1 & P2 & P3 & P4 & P5
    P1 & P2 & P3 & P4 & P5 --> USG
```

- Every app uses **the official OpenAI SDK of its language** with
  `base_url = llm_brain` and its own `api_key`. Zero dependencies on
  llm_brain in the app's code.
- The `api_key` identifies the **profile**: default tier, monthly budget,
  cache policy, streaming. Changing an app's model = changing the profile.
- Usage per profile lands in llm_brain's dashboard: one place to see how
  much each app and the dev tooling spend.

## Plausible tiers per app

| App | Tier | L2 cache |
|-----|------|----------|
| assistant (chat, RAG) — **first** | `fast` | exact |
| find-a-car (valuation, comparison, extraction from listings) — **first** | `fast` + `reasoning` for valuations | exact |
| second-hand market (listing classification, descriptions, moderation) — later | `fast` | semantic |
| agent-orchestrator (`ago`) | like `dev` | OFF |
| dev tooling (Claude Code + aider) | `fast` with escalation | OFF |
