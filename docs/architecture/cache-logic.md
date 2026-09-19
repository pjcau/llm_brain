---
title: Cache logic and software
---

# Cache logic: how it works and with which software

Question: *"what software could I use? was the case where the client does
no caching considered?"* Short answer: **the "client with no cache" case
is the apps' case, and it is the main one**. Claude Code and aider have
their own L0; apps via SDK send everything every time. For them all the
logic lives in the layer.

## What happens without a client that caches

An app sends the entire prompt on every request. Two problems and two
answers:

1. **The prefix repeats** (instructions, output schema, examples): if the
   provider has a prompt cache, you pay 0.1×–0.5× **only if the prefix is
   byte-identical and at the start**. An app that composes the system
   prompt with f-strings and puts the date/time or user id in it breaks
   the cache on every call. → The layer **keeps the system prompt per
   profile**, versioned, and prepends it itself: the app sends only the
   variable data.
2. **The same question comes back** (assistant FAQ, same listing
   reclassified): no provider knows. → **L2 in the layer**, exact or
   semantic, per profile.

## The decision flow

{/* diagram: 07-cache-decision */}
```mermaid
flowchart TD
    A["Incoming request<br/>(profile, alias, system?, messages, tools, temperature)"] --> B{"Does the client send a system prompt?"}
    B -- "no (app via SDK)" --> B1["The layer prepends the profile's<br/>versioned system prompt"]
    B -- "yes (Claude Code, aider)" --> B2["Prefix left untouched,<br/>never reordered"]
    B1 --> C["Normalize → key =<br/>sha256(profile, alias, system_v, tools, messages, temperature)"]
    B2 --> C
    C --> D{"Profile L2 policy"}
    D -- "OFF (dev)" --> H
    D -- "exact" --> E{"hit in SQLite<br/>and not expired?"}
    D -- "semantic" --> F{"neighbour with cos ≥ 0.95<br/>same namespace?"}
    E -- "yes" --> R1["Response from cache<br/>usage: cost=0, source=l2"]
    F -- "yes" --> R1
    E -- "no" --> H
    F -- "no" --> H
    H["L1 — prepare the call"] --> H1{"Provider with explicit cache?<br/>(Anthropic, Gemini, Qwen)"}
    H1 -- "yes" --> H2["Insert cache_control after<br/>system+tools and on the second-to-last turn<br/>(max 4 breakpoints)"]
    H1 -- "no (OpenAI, DeepSeek, Grok, Groq…)" --> H3["Nothing to do:<br/>a stable prefix is enough"]
    H2 --> I["session_id = profile+conversation<br/>→ OpenRouter sticky routing"]
    H3 --> I
    I --> J["Call"]
    J --> K["Read usage.prompt_tokens_details:<br/>cached_tokens, cache_write_tokens, cache_discount"]
    K --> L{"Cacheable in L2?<br/>no tool_use, no error,<br/>no truncated stream, temp ≤ 0.3"}
    L -- "yes" --> M["Write to L2 with the profile's TTL"]
    L -- "no" --> N["Respond"]
    M --> N
```

Fixed rules:

- **Never reorder the prefix**: system, tools, skills first; variable
  content after. One moved line invalidates everything.
- **For Claude Code (`dev` profile) the layer doesn't touch `system`**: the
  attribution block must stay first and intact; server-side system
  prompt prepending applies to apps only ([details](./client-compatibility.md)).
- **Don't cache in L2**: responses with `tool_use`, errors, truncated
  streams, requests with `temperature` > 0.3 (unless an explicit policy).
- **L2 always OFF for `dev`**: files change between requests.
- **Always** read `prompt_tokens_details` and save `cached_tokens` and
  `cache_discount` in usage: without this you don't know whether the
  cache works.

## L1 — Prompt cache through OpenRouter (verified 2026-09-19)

| Provider | Mode | Write | Read |
|----------|------|-------|------|
| OpenAI, DeepSeek, Grok, Groq, Moonshot, Z.AI, Gemini 2.5 | **automatic** (prefix) | 1× (OpenAI 1.25×) | 0.1×–0.5× |
| Anthropic | explicit: `cache_control` per block | 1.25× (5 min) / 2× (1 h) | 0.1× |
| Gemini (explicit), Qwen | explicit: `cache_control` | 1.25× | 0.1×–0.25× |

- Hits reported in `usage.prompt_tokens_details.cached_tokens`,
  `cache_write_tokens`, `cache_discount` (negative on writes).
- **Sticky routing**: after a hit OpenRouter sends subsequent requests to
  the same endpoint; controllable with `session_id` (expires after 10
  min). The layer sets it to `profile+conversation`.
- **Open models on third-party providers (e.g. bonsai on Darkbloom): not
  documented.** To measure in Phase 0 with `cached_tokens`: if it's
  always 0, that model has no L1 and the real cost is the full one.

Software: **nothing to install**. It's the provider. The layer's job is a
stable prefix + inserting `cache_control` where needed + `session_id`.
agent-orchestrator's `providers/openrouter.py` already injects
`cache_control` for the CLI's `cache_context`: that's the piece to
extend.

## L2 — Response cache: the options

| Option | Type | Pros | Cons |
|--------|------|------|------|
| **SQLite in llm_brain** (extend `core/cache.py`, InMemory today) | exact | ~100 lines, zero dependencies, key = normalized hash, TTL per profile | exact only |
| Redis | exact | shared across processes, native TTL | one more service; not needed for a single user |
| **GPTCache** (Python library) | semantic | pluggable: embedding + vector store + eviction policy | heavy dependency, not very active |
| LiteLLM cache (in-memory / redis / redis-semantic) | exact + semantic | ready if you use LiteLLM | LiteLLM is out of Phase 0 |
| Helicone / Portkey | exact via header | zero code | external proxy: one more hop and prompts leave to third parties |
| **sqlite-vec + embeddings** | semantic | light, in-process, same SQLite file | to write: ~200 lines |

For the semantic cache the embedding has a cost: it only pays if
embedding cost ≪ saved response cost. With a cheap embedding model via
OpenRouter or local (`sentence-transformers`, CPU) the math works for the
assistant; for listing classification, exact match by `listing id` (L3,
app side) is simpler and safer.

## Recommendation

| Layer | Phase | Choice |
|-------|-------|--------|
| L1 | 0 | OpenRouter, measuring `cached_tokens` for every candidate model |
| L1 | 1 | server-side system prompt per profile + `cache_control` + `session_id` |
| L2 exact | 1 | SQLite, extending `core/cache.py` with a persistent backend and a key from the normalized request; ON for `assistant`, `car`; OFF for `dev` |
| L2 semantic | 2, `assistant` only | `sqlite-vec` + cheap embeddings; threshold 0.95; namespace = system prompt version |
| External proxies | — | no: one more hop and data leaves |

## What to measure to know it works

- `cached_tokens / prompt_tokens` per profile and per model (target for
  `dev`: > 60% with Claude Code, whose system prompt is large).
- L2 hit rate for `assistant` (`core/cache.py` already has
  `CacheStats.hit_rate`).
- € saved = summed `cache_discount` + cost avoided by L2 hits.
All three go in the dashboard next to the budget.
