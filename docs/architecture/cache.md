---
title: Cache layers
---

# The cache layers: what they are and who owns them

Key point: **not all caches are ours**, and they must be treated
differently. This page says *what* the layers are; the decision flow, the
case of clients with no cache, and the software options are in
[Cache logic and software](./cache-logic.md).

{/* diagram: 03-cache-layers */}
```mermaid
flowchart TB
    subgraph L0["L0 — Client (not ours)"]
        L0a["Claude Code: session context, files read"]
        L0b["aider: repo map, chat history"]
    end

    subgraph L2["L2 — Response cache (gateway, llm_brain)"]
        L2a["Exact match: hash(normalized messages + model + tools) → response, TTL"]
        L2b["Semantic (optional, apps only): query embedding → similar response"]
        L2n["⚠ OFF for coding agents: files change, response goes stale"]
    end

    subgraph L1["L1 — Provider prompt cache (translated by llm_brain)"]
        L1a["Anthropic: explicit cache_control on blocks"]
        L1b["OpenAI: automatic prefix cache"]
        L1c["Gemini: explicit context caching, with TTL and storage cost"]
        L1d["DeepSeek: automatic on-disk context cache"]
        L1e["OpenRouter: pass-through to the backend that serves the turn"]
        L1f["⚠ one cache per backend: a turn on another backend re-reads the whole prefix"]
        L1n["Rule: stable prefix → system + tools + skills FIRST, variable content AFTER"]
    end

    subgraph L3["L3 — Application cache (the app's domain)"]
        L3a["second-hand market: listing classification by id"]
        L3b["find-a-car: car valuation by (model, year, km)"]
        L3c["assistant: FAQ answers"]
    end

    REQ["Request"] --> L0 --> L2
    L2 -- miss --> L1 --> PROV["Provider (pinned per session: next step)"]
    L3 -. "the app decides before calling llm_brain" .-> REQ
```

## L0 — Client

We don't touch it. Claude Code and aider manage what to keep in session
on their own.

## L1 — Provider prompt cache

The cache that **really saves money** in coding agents: system prompt +
tools + skills (tens of KB) repeat identically every turn. Every provider
does it its own way:

| Provider | Mechanism |
|----------|-----------|
| Anthropic | explicit `cache_control` on blocks |
| OpenAI | automatic prefix cache |
| DeepSeek | automatic on-disk context cache |
| Gemini | explicit context caching, with TTL and storage cost |
| OpenRouter | pass-through to the backend serving the turn — verified 2026-09-22; the backend can change between turns, see below |
| Local (vLLM / llama.cpp) | prefix cache / KV cache: `none` strategy on the layer side |

The translator must: accept hints in both dialects, translate or drop
them, **never reorder the prefix** (system, tools, skills first; variable
content after), and report cache hits in usage. If this piece is wrong,
Claude Code behind the proxy costs double.


### Which backend serves the turn (2026-09-22)

OpenRouter is not one cache. A model is served by many endpoints —
`deepseek-v4-pro` has 16 — and **the prompt cache lives in the backend
that served the turn**, so a turn that lands somewhere the prefix has
never been pays the whole history at full price.

Measured on one live `px-claude` session (dev, 203 requests, $2.44):

| turn | input | cache read | cost |
|------|-------|------------|------|
| warm | ~2.000 | ~114.000 | **$0.011** |
| cold | ~112.000 | 0 | **$0.102** |

Ten of its last 28 turns were cold — **~$0.91 in ten minutes** — and warm
and cold alternate within seconds on a prefix that only grows, which is
what a turn changing backend looks like rather than a TTL expiring. Two
identical probe requests confirmed it directly: `StreamLake`, then
`SiliconFlow`.

OpenRouter names the backend in `provider`: at the top level of a
non-streamed body and of an OpenAI chunk, and inside `message` in an
Anthropic `message_start`. The proxy reads it in both dialects, streamed
or not, and stores it on the request row; the board's **Providers** table
shows requests, cache-hit share, prompt tokens paid at full price, cost
and latency per backend.

Pinning is the next step, not yet done: `provider.order` plus
`allow_fallbacks` in the upstream body (OpenRouter has no sticky-session
feature of its own). The measurement comes first, because if the
fragmentation is *inside* one provider's fleet, pinning the provider is
not enough. Price is a second reason to choose: across those 16 endpoints
input runs from $0.919 to $1.91 per M and cache reads from $0.0766 to
$0.33 per M.
## L2 — Gateway response cache

Useful for apps (assistant FAQs, repeated classifications), **harmful for
coding** (files change between requests, the cached answer is stale). Per
profile:

| Profile | L2 |
|---------|----|
| `dev` | OFF |
| `assistant` | exact match ON |
| `market` | semantic ON |
| `car` | exact match ON |

## L3 — Application

The app's domain, not the layer's (listing classification by id, car
valuation by model/year/km, FAQ). The layer can expose a helper (suggested
cache key) so it isn't reinvented.
