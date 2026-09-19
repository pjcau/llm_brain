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
    subgraph CLIENTS["Clients (no client changes)"]
        CC["Claude Code<br/>ANTHROPIC_BASE_URL"]
        AID["aider<br/>OPENAI_API_BASE"]
        OC["OpenCode / Cline / Continue"]
        APPS["GitHub apps<br/>assistant · find-a-car (first) · second-hand market (later)"]
        AGO["agent-orchestrator<br/>(client: providers/openai.py with base_url)"]
    end

    subgraph BRAIN["llm_brain — single backend (FastAPI)"]
        direction TB
        EP_A["/v1/messages<br/>(Anthropic dialect)"]
        EP_O["/v1/chat/completions<br/>(OpenAI dialect)"]
        TR["Translator<br/>single internal format<br/>(messages, tools, stream, cache hints)"]
        POL["Policy / Profiles<br/>per client: default tier, budget, cache"]
        RT["Tier router<br/>fast · reasoning · premium<br/>+ escalation on failure"]
        CACHE["Cache manager<br/>L1 provider prompt cache<br/>L2 gateway response cache"]
        USG["Usage / Budget<br/>SQLite · daily and monthly limit per profile"]
        PV["providers/<br/>openrouter (today) · openai-compat · local (later)"]
    end

    subgraph UPSTREAM["Upstream providers"]
        OR["OpenRouter<br/>e.g. bonsai-2-27b (int4)"]
        GG["Google Gemini"]
        DS["DeepSeek"]
        OL["Ollama / llama.cpp (local GPU)<br/>e.g. bonsai-2-27b GGUF 7 GB — same model"]
        AN["Anthropic (premium tier only)"]
    end

    CC --> EP_A
    AID --> EP_O
    OC --> EP_O
    APPS --> EP_O
    AGO --> EP_O
    EP_A --> TR
    EP_O --> TR
    TR --> POL --> RT
    RT --> CACHE --> PV
    RT -.-> USG
    PV --> OR & GG & DS & OL & AN
```

## 2. Flow of a request from Claude Code (hooks and skills)

{/* diagram: 02-request-flow-claude-code */}
```mermaid
sequenceDiagram
    autonumber
    actor U as User
    participant CC as Claude Code (CLI, local)
    participant H as Hooks / Skills / MCP (local)
    participant EP as llm_brain /v1/messages
    participant TR as Translator
    participant RT as Tier router
    participant C as Cache
    participant P as Provider (fast)
    participant R as Provider (reasoning)

    U->>CC: prompt
    CC->>H: UserPromptSubmit hook, skill match (all local)
    H-->>CC: enriched context (skill md, CLAUDE.md)
    CC->>EP: POST /v1/messages<br/>model=claude-*, system[cache_control], tools[], stream=true
    EP->>TR: normalize (Anthropic → internal format)
    TR->>RT: request + client profile "claude-code"
    RT->>RT: model claude-* → tier (fast by default)
    RT->>C: L2 lookup (request hash) — miss for coding
    C->>P: call with cache hint translated for the provider
    P-->>TR: stream (provider-format chunks)
    TR-->>CC: Anthropic SSE (message_start, content_block_delta, tool_use…)
    CC->>H: PreToolUse hook → runs tool locally → PostToolUse hook
    CC->>EP: POST /v1/messages (tool_result)
    Note over RT,R: Escalation: if the client signals failure<br/>(tests/lint KO, or explicit /model reasoning)<br/>the router sends it to the reasoning tier
    RT->>R: same request, reasoning tier
    R-->>CC: response via TR
    CC->>H: Stop hook
    RT-->>EP: usage (tokens, cache hits, cost) → dashboard
```

## 3. Cache layers

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
        L1e["OpenRouter: pass-through to the underlying provider"]
        L1n["Rule: stable prefix → system + tools + skills FIRST, variable content AFTER"]
    end

    subgraph L3["L3 — Application cache (the app's domain)"]
        L3a["second-hand market: listing classification by id"]
        L3b["find-a-car: car valuation by (model, year, km)"]
        L3c["assistant: FAQ answers"]
    end

    REQ["Request"] --> L0 --> L2
    L2 -- miss --> L1 --> PROV["Provider"]
    L3 -. "the app decides before calling llm_brain" .-> REQ
```

## 4. App integration

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

## 5. Modules from agent-orchestrator

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

## 6. Budget rings

{/* diagram: 06-budget-rings */}
```mermaid
flowchart TB
    subgraph R3["Ring 3 — Client (informational)"]
        C1["Claude Code: statusline / Stop hook shows today's spend"]
        C2["aider: cap on repo-map and chat-history tokens"]
    end
    subgraph R2["Ring 2 — llm_brain (soft, with degradation)"]
        B1["daily AND monthly budget per profile (dev: 3 €/day, 30 €/month)"]
        B2["pre-call estimate: input × price + max_tokens × price"]
        B3["70% → fast tier only · 85% → reduced max_tokens · 100% → 429 with a clear message"]
        B4["usage per profile → dashboard"]
    end
    subgraph R1["Ring 1 — OpenRouter (hard, cannot be bypassed)"]
        O1["prepaid credits: negative balance = 402"]
        O2["one key per profile with limit + limit_reset: daily (dev: 3 €)"]
        O5["fixed monthly top-up (≈ 40 €) = hard monthly ceiling"]
        O3["held-cost: requests that don't fit the balance are rejected up front"]
        O4["GET /api/v1/key: usage_daily, limit_remaining"]
    end
    R3 --> R2 --> R1
    O4 -. "reconciliation" .-> B4
```

## 7. Cache decision flow

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

## 8. Secrets flow

{/* diagram: 08-secrets-flow */}
```mermaid
flowchart LR
    subgraph CLIENTS["Clients — each holds ONLY its own llm_brain key"]
        CC["Claude Code<br/>ANTHROPIC_API_KEY=brain_dev_…"]
        AID["aider<br/>OPENAI_API_KEY=brain_dev_…"]
        AST["assistant<br/>env BRAIN_API_KEY=brain_assistant_…"]
        CAR["find-a-car<br/>env BRAIN_API_KEY=brain_car_…"]
        AGO["agent-orchestrator<br/>brain_ago_…"]
    end

    subgraph BRAIN["llm_brain"]
        AUTH["Auth<br/>Authorization: Bearer / x-api-key<br/>sha256(key) → keys table → profile"]
        KEYS[("keys (SQLite)<br/>hash · profile · created · last used · revoked")]
        SEC["Runtime secret store<br/>.env 0600 or sops+age<br/>OPENROUTER_KEY_DEV, _AGO, _ASSISTANT, _CAR, _BENCH"]
        PRX["Proxy → OpenRouter<br/>with the profile's key"]
    end

    subgraph ADMIN["Admin only, never in the runtime"]
        MGMT["OPENROUTER_MANAGEMENT_KEY<br/>used once by 'brain keys provision'"]
        PROV["OpenRouter Provisioning API<br/>creates a per-profile key with a daily limit"]
    end

    CC & AID & AST & CAR & AGO --> AUTH --> KEYS
    AUTH --> PRX
    SEC --> PRX
    MGMT --> PROV -. "created key → pasted into SEC" .-> SEC
```

## 9. Topology

{/* diagram: 09-topology */}
```mermaid
flowchart LR
    subgraph LOCAL["Home (local network)"]
        CC["Claude Code · aider<br/>brain_dev_…"]
        AST["assistant (stays local)<br/>brain_assistant_…"]
        AGO["agent-orchestrator<br/>brain_ago_…"]
    end

    subgraph REMOTE["VPS / AWS"]
        subgraph BRAIN["llm_brain (decided: on the VPS)"]
            CADDY["Caddy: automatic HTTPS<br/>only /v1/*, per-key rate limit"]
            API["FastAPI"]
            DB[("SQLite<br/>usage · budget · keys · cache")]
            ENV["secrets: sops+age → .env 0600<br/>or systemd LoadCredential"]
        end
        CAR["find-a-car (backend)<br/>brain_car_… from Secrets Manager / SSM or .env"]
        SHM["second-hand market (backend)<br/>brain_market_…"]
        LS["Litestream → S3/B2<br/>continuous SQLite replication"]
    end

    USERS["End users<br/>(browser / mobile)"] -- "the APP's auth<br/>(never llm_brain keys in the frontend)" --> CAR & SHM
    CAR & SHM -- "HTTPS + Bearer brain_*<br/>+ user: <opaque id>" --> CADDY
    CC & AST & AGO -- "HTTPS (or Tailscale)<br/>+ Bearer brain_*" --> CADDY
    CADDY --> API --> DB
    ENV --> API
    DB --> LS
    API -- "profile's OpenRouter key" --> OR["OpenRouter"]
```

## 10. Authentication sequence

{/* diagram: 10-auth-sequence */}
```mermaid
sequenceDiagram
    autonumber
    actor ADM as Admin (you)
    participant CLI as brain CLI (on the server, via SSH/Tailscale)
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
    ADM->>APP: puts the key in SSM / .env (never in the repo, never in the frontend)
    end

    rect rgb(240,255,240)
    note over APP,OR: Request — every call
    APP->>MW: POST /v1/chat/completions<br/>Authorization: Bearer brain_car_…<br/>user: "u_8f3a" · X-Brain-Run (opt.)
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
            MW->>DB: usage(profile, key, user, run, cost) · last_used_at
            MW-->>APP: response in the client's dialect
        end
    end
    end

    rect rgb(255,245,235)
    note over ADM,APP: Rotation — zero downtime
    ADM->>CLI: brain keys create --profile car --name prod-2
    ADM->>APP: deploy with the new key
    ADM->>CLI: brain keys revoke prod
    CLI->>DB: UPDATE revoked_at=now
    end
```

## 11. Hosting options

{/* diagram: 11-hosting-options */}
```mermaid
flowchart LR
    subgraph A["Discarded alternative — everything on AWS"]
        A1["EC2 t4g.small (2 vCPU ARM, 2 GB)<br/>Caddy + FastAPI + SQLite + Litestream<br/>≈ $14/m + EBS 20 GB ≈ $2 + IPv4 ≈ $3.7"]
        A2["find-a-car · market<br/>same VPC / same region"]
        A3["S3 (Litestream backup) ≈ $0.1<br/>SSM Parameter Store: free"]
        A2 -- "private network, security group<br/>no public exposure" --> A1
        A1 --> A3
        H1["Home: CLI + assistant"] -- "Tailscale" --> A1
    end
    subgraph B["CHOSEN — llm_brain on a VPS, apps anywhere"]
        B1["Hetzner CAX11/CX23 (2 vCPU, 4 GB)<br/>≈ 4–5 €/m all-in"]
        B2["find-a-car · market<br/>on AWS, a VPS or wherever needed"]
        B3["Backblaze B2 / Hetzner Object Storage<br/>backup ≈ 0"]
        B2 -- "public HTTPS + per-app key<br/>+ rate limit (+5–10 ms EU→EU)" --> B1 --> B3
        H2["Home: CLI + assistant"] -- "Tailscale or HTTPS" --> B1
    end
    X["Mixed is acceptable: the public endpoint is protected by the auth design;<br/>the extra latency (+5–10 ms) is nothing next to the model"]
```
