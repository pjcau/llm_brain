---
title: Diagrams
---

# Diagrams

Mermaid sources in `diagrams/*.mmd`. After editing a source:
`npm run sync-diagrams` updates every embedded copy in the pages.

## 1. System

{/* diagram: 01-system-overview */}
```mermaid
flowchart LR
    subgraph CLIENTS["Clients (no client changes: base URL + brain_* key)"]
        CC["Claude Code · px-claude<br/>ANTHROPIC_BASE_URL · brain/auto"]
        AID["aider · px-aider<br/>OPENAI_API_BASE · architect/editor"]
        OC["OpenCode (benchmarked)"]
        APPS["apps<br/>find-a-car (car) · assistant · market (later)"]
        AGO["agent-orchestrator (ago, not yet wired)"]
    end

    subgraph BRAIN["llm_brain — one Rust binary on the VPS (brain serve)"]
        direction TB
        EP_A["/v1/messages · count_tokens<br/>(Anthropic dialect)"]
        EP_O["/v1/chat/completions · /v1/models<br/>(OpenAI dialect)"]
        AUTH["Auth<br/>sha256(key) → profile · IP/expiry · rate limit"]
        BUD["Budget ring 2<br/>70% fast only · 85% max_tokens cap · 100% 429"]
        RT["Model resolution<br/>brain/&lt;tier&gt; · claude-* → tier<br/>brain/auto: tier per session (Jev)"]
        SAN["Sanitizer + models[] fallback<br/>+ catalog max_tokens cap"]
        TAP["Streaming tap<br/>usage · cost · serving backend → SQLite"]
        BOARD["Board (/)<br/>spend · cache · providers · anomalies"]
    end

    subgraph UPSTREAM["Upstream"]
        OR["OpenRouter<br/>one key per profile, daily limit (ring 1)<br/>deepseek-v4-flash/pro · glm-5.3 · bonsai-2-27b"]
        OL["local GPU (wish, Phase 3)"]
    end

    CC --> EP_A
    AID & OC & APPS & AGO --> EP_O
    EP_A & EP_O --> AUTH --> BUD --> RT --> SAN --> OR
    OR --> TAP --> BOARD
    SAN -.-> OL
```

## 2. Flow of a request from Claude Code (hooks and skills)

{/* diagram: 02-request-flow-claude-code */}
```mermaid
sequenceDiagram
    autonumber
    actor U as User
    participant CC as Claude Code (CLI, local)
    participant H as Hooks / Skills / MCP (local)
    participant PX as llm_brain /v1/messages
    participant JV as Jev (decision model)
    participant OR as OpenRouter (Anthropic dialect)

    U->>CC: prompt
    CC->>H: UserPromptSubmit hook, skill match (all local)
    H-->>CC: enriched context (skill md, CLAUDE.md)
    CC->>PX: POST /v1/messages<br/>model=brain/auto, system[cache_control], tools[], stream=true
    PX->>PX: key → profile dev · rate limit · budget ring
    PX->>JV: first turn of the session only: which rung?
    JV-->>PX: e.g. agent → deepseek-v4-pro
    PX->>PX: sanitize (unsupported fields, mid-conversation system turns → user)<br/>system untouched · max_tokens cap (models[] fallback: OpenAI dialect only)
    PX->>OR: same Anthropic request, profile's OpenRouter key
    OR-->>CC: Anthropic SSE passed through unbuffered (tap reads usage)
    CC->>H: PreToolUse hook → runs tool locally → PostToolUse hook
    CC->>PX: POST /v1/messages (tool_result), same session id
    PX->>OR: same tier as the first turn — prompt cache stays warm
    OR-->>CC: stream
    CC->>H: Stop hook
    PX-->>PX: request row: tokens, cache read, cost, backend → board
```

## 3. Cache layers

{/* diagram: 03-cache-layers */}
```mermaid
flowchart TB
    subgraph L0["L0 — Client (not ours)"]
        L0a["Claude Code: session context, files read"]
        L0b["aider: repo map, chat history"]
    end

    subgraph L2["L2 — Response cache (llm_brain, designed, not built)"]
        L2a["Exact match: hash(normalized messages + model + tools) → response, TTL"]
        L2b["Semantic (optional, apps only): query embedding → similar response"]
        L2n["⚠ OFF for coding agents: files change, response goes stale"]
    end

    subgraph L1["L1 — Provider prompt cache (cache hints passed through untouched)"]
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
    L2 -- miss --> L1 --> PROV["Provider (pinning per model: next step)"]
    L3 -. "the app decides before calling llm_brain" .-> REQ
```

## 4. App integration

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

## 5. Modules: what agent-orchestrator inspired, what exists

{/* diagram: 05-agent-orchestrator-modules */}
```mermaid
flowchart LR
    subgraph AO["agent-orchestrator (becomes a client)"]
        A1["providers/openai.py<br/>base_url = llm_brain, api_key = brain_ago_…"]
        A2["dashboard, graph, agent runtime…<br/>unchanged"]
    end

    subgraph REUSE["Ideas ported (not code)"]
        R1["core/usage.py<br/>per-day budget check"]
        R2["core/cache.py<br/>cache policy · hit rate"]
        R4["core/evaluator.py<br/>eval suite · compare"]
    end

    subgraph NEW["llm_brain — crates/brain/src"]
        N1["proxy/mod.rs<br/>both dialects · auth chain · forward"]
        N2["auth.rs · keys.rs<br/>client keys (hashed) · OpenRouter keys"]
        N3["budget.rs<br/>daily + monthly, 70/85/100 rings"]
        N4["proxy/route.rs<br/>brain/auto per session"]
        N5["proxy/sanitize.rs · tap.rs · usage_parse.rs"]
        N6["L2 response cache<br/>(designed, not built)"]
        N8["bench/<br/>real-bug suite (nightly promotion: Phase 2)"]
        DB[("SQLite<br/>requests · keys · usage · events · bench")]
    end

    A1 --> N1
    N1 --> N2 --> N3 --> N4 --> N5
    R1 -.-> N3
    R2 -.-> N6
    R4 -.-> N8
    N5 & N8 --> DB
```

## 6. Budget rings

{/* diagram: 06-budget-rings */}
```mermaid
flowchart TB
    subgraph R3["Ring 3 — Client and board (informational)"]
        C1["board: spend per day/month, projection vs soft caps"]
        C2["aider: cap on repo-map and chat-history tokens"]
    end
    subgraph R2["Ring 2 — llm_brain (soft, with degradation)"]
        B1["daily AND monthly budget per profile (dev: 5 $/day, 60 $/month soft)"]
        B2["spend = sum of recorded request costs (OpenRouter's cost, else tier prices)"]
        B3["70% → fast tier only · 85% → reduced max_tokens · 100% → 429, retry-after ≥ 3600"]
    end
    subgraph R1["Ring 1 — OpenRouter (hard, cannot be bypassed)"]
        O1["prepaid credits: negative balance = 402"]
        O2["one key per profile with limit + limit_reset: daily (dev: 5 $)"]
        O5["fixed monthly top-up = hard monthly ceiling"]
        O4["GET /api/v1/key: usage_daily, limit_remaining (usage snapshot)"]
    end
    R3 --> R2 --> R1
    O4 -. "reconciliation on the board" .-> C1
```

## 7. Cache decision flow (L2 steps designed, not built)

{/* diagram: 07-cache-decision */}
```mermaid
flowchart TD
    A["Incoming request<br/>(profile, alias, system?, messages, tools, temperature)"] --> B{"Does the client send a system prompt?"}
    B -- "no (app via SDK)" --> B1["NOT BUILT: the layer prepends the profile's<br/>versioned system prompt"]
    B -- "yes (Claude Code, aider)" --> B2["Prefix left untouched,<br/>never reordered (built)"]
    B1 --> D
    B2 --> D
    D{"Profile l2_cache policy<br/>(NOT BUILT: read, no effect today)"}
    D -- "off (dev)" --> H
    D -- "exact" --> E{"sha256(profile, alias, system_v, tools, messages, temperature)<br/>hit in SQLite and not expired?"}
    D -- "semantic" --> F{"neighbour with cos ≥ 0.95<br/>same namespace?"}
    E -- "yes" --> R1["Response from cache<br/>usage: cost=0, source=l2"]
    F -- "yes" --> R1
    E -- "no" --> H
    F -- "no" --> H
    H["L1 — client's cache_control and prefix passed through (built)"] --> J["Call OpenRouter"]
    J --> K["Read usage: cached / cache-write tokens,<br/>serving backend → requests row (built)"]
    K --> L{"Cacheable in L2?<br/>no tool_use, no error,<br/>no truncated stream, temp ≤ 0.3"}
    L -- "yes" --> M["NOT BUILT: write to L2 with the profile's TTL"]
    L -- "no" --> N["Respond"]
    M --> N
```

## 8. Secrets flow

{/* diagram: 08-secrets-flow */}
```mermaid
flowchart LR
    subgraph CLIENTS["Clients — each holds ONLY its own llm_brain key"]
        CC["Claude Code<br/>ANTHROPIC_AUTH_TOKEN=brain_dev_…"]
        AID["aider<br/>OPENAI_API_KEY=brain_dev_…"]
        AST["assistant<br/>brain_assistant_…"]
        CAR["find-a-car<br/>BRAIN_CAR_KEY=brain_car_…"]
        BEN["brain bench --via-proxy<br/>BRAIN_BENCH_KEY=brain_benchmark_…"]
    end

    subgraph BRAIN["llm_brain (VPS)"]
        AUTH["Auth<br/>Authorization: Bearer / x-api-key<br/>sha256(key) → api_keys → profile"]
        KEYS[("api_keys (SQLite)<br/>hash · profile · name · expiry · IPs · revoked")]
        SEC[".env 0600 (brain user)<br/>OPENROUTER_KEY_DEV, _AGO, _BENCHMARK, _ASSISTANT, _CAR"]
        PRX["Proxy → OpenRouter<br/>with the profile's key"]
    end

    subgraph ADMIN["Admin only, never in the runtime"]
        MGMT["OPENROUTER_MANAGEMENT_KEY<br/>brain upstream provision | sync | list"]
        PROV["OpenRouter Provisioning API<br/>per-profile key with a daily limit"]
    end

    CC & AID & AST & CAR & BEN --> AUTH --> KEYS
    AUTH --> PRX
    SEC --> PRX
    MGMT --> PROV -. "created key → copied into SEC" .-> SEC
```

## 9. Topology

{/* diagram: 09-topology */}
```mermaid
flowchart LR
    subgraph LOCAL["Home (laptop)"]
        CC["Claude Code · aider<br/>brain_dev_…"]
        AST["assistant (local)<br/>brain_assistant_…"]
        FAC["find-a-car (Docker)<br/>brain_car_…"]
    end

    subgraph VPS["VPS (Contabo, x86_64)"]
        CADDY["Caddy: Let's Encrypt on an sslip.io hostname<br/>/v1/* → proxy · board behind basic auth"]
        API["brain serve (systemd, brain user)"]
        DB[("SQLite<br/>requests · keys · usage · events")]
        ENV[".env 0600<br/>OpenRouter keys per profile"]
    end

    LS["Litestream → S3/B2<br/>(configured, not installed yet)"]
    USERS["End users (later, public apps)"] -- "the APP's auth<br/>(never llm_brain keys in a frontend)" --> FAC
    CC & AST & FAC -- "HTTPS + Bearer brain_*<br/>+ user: opaque id" --> CADDY
    CADDY --> API --> DB
    ENV --> API
    DB -.-> LS
    API -- "profile's OpenRouter key" --> OR["OpenRouter"]
```

## 10. Authentication sequence

{/* diagram: 10-auth-sequence */}
```mermaid
sequenceDiagram
    autonumber
    actor ADM as Admin (you)
    participant CLI as brain CLI (on the server, via SSH)
    participant DB as SQLite api_keys
    participant APP as find-a-car backend
    participant MW as llm_brain auth middleware
    participant BUD as budget / rate limit
    participant OR as OpenRouter

    rect rgb(235,245,255)
    note over ADM,DB: Issuing — only whoever has a shell on the server
    ADM->>CLI: brain keys create --profile car --name prod --expires 2027-01-01
    CLI->>CLI: random 32 bytes → brain_car_<base64url>
    CLI->>DB: INSERT hash=sha256(key), profile=car, name, expires_at
    CLI-->>ADM: prints the key ONCE
    ADM->>APP: puts the key in the app's .env (never in the repo, never in the frontend)
    end

    rect rgb(240,255,240)
    note over APP,OR: Request — every call
    APP->>MW: POST /v1/chat/completions<br/>Authorization: Bearer brain_car_…<br/>user: "u_8f3a"
    MW->>MW: valid prefix? → sha256 → lookup (60 s cache)
    MW->>DB: SELECT profile, revoked_at, expires_at WHERE hash=?
    alt unknown / revoked / expired
        MW-->>APP: 401 {"error":{"type":"authentication_error"}}
    else ok
        MW->>BUD: rate limit (key, user) · profile budget (day, month)
        alt limit exceeded
            BUD-->>APP: 429 with retry-after and a clear message
        else ok
            MW->>OR: same request, car profile's OpenRouter key
            OR-->>MW: response + usage (cached_tokens…)
            MW->>DB: requests row (profile, key, user, tokens, cost, backend) · last_used_at
            MW-->>APP: response in the client's dialect
        end
    end
    end

    rect rgb(255,245,235)
    note over ADM,APP: Rotation — zero downtime
    ADM->>CLI: brain keys create --profile car --name prod-2
    ADM->>APP: deploy with the new key
    ADM->>CLI: brain keys revoke --profile car --name prod
    CLI->>DB: UPDATE revoked_at=now
    end
```

## 11. Hosting: what runs, what is equivalent

{/* diagram: 11-hosting-options */}
```mermaid
flowchart LR
    subgraph RUN["RUNNING — llm_brain on a VPS, apps anywhere"]
        B1["Contabo VPS (x86_64, 4 vCPU, 8 GB)<br/>Caddy + brain serve + SQLite"]
        B2["apps<br/>laptop, AWS, a VPS — no constraint"]
        B3["S3-compatible bucket<br/>Litestream backup (pending)"]
        B2 -- "public HTTPS + per-app key<br/>+ rate limit (+5–10 ms EU→EU)" --> B1 -.-> B3
    end
    subgraph ALT["Equivalent alternatives"]
        A1["Hetzner CAX11 ≈ €4–5"]
        A2["AWS Lightsail $5<br/>if the apps land on AWS"]
        A3["EC2 + managed DB<br/>only at the HA step"]
    end
    B1 -. "same binary, scp + systemd" .-> A1 & A2 & A3
```

## 12. Auto routing (brain/auto)

{/* diagram: 12-auto-routing */}
```mermaid
sequenceDiagram
    autonumber
    participant CC as Claude Code (px-claude)
    participant PX as llm_brain proxy
    participant SC as Session cache (in memory, TTL 2 h)
    participant JV as Jev (OpenRouter /alpha/decisions)
    participant OR as OpenRouter /v1

    CC->>PX: POST /v1/messages · model=brain/auto<br/>x-claude-code-session-id: s1 · first user message
    PX->>SC: get(dev:h:s1)
    SC-->>PX: miss
    PX->>JV: state = first user message<br/>question "tier" (choice) · criteria = the ladder's `when`
    JV-->>PX: choice=medium · confidence 0.95 (~0.5 s, ~0.00002 $)
    PX->>SC: put(dev:h:s1 → medium)
    PX->>OR: same request · model = glm-5.3-flash
    OR-->>CC: stream
    Note over CC,PX: every later turn of the loop (tool results, retries…)
    CC->>PX: POST /v1/messages · model=brain/auto · same session id
    PX->>SC: get(dev:h:s1)
    SC-->>PX: medium (touch: TTL restarts)
    PX->>OR: model = glm-5.3-flash — same model, prompt cache warm
    Note over PX,JV: Jev down, unsure (< min_confidence) or no user text → fallback tier (agent), noted as auto:agent:error / low-confidence / no-task
    Note over PX: budget rings still apply after the decision: at 70% of the day the rung becomes fast
```
