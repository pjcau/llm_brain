---
title: Claude Code and aider behind the layer
---

# How Claude Code and aider react

Question: *"do skills and hooks always work?"* Answer: **yes, they are
client-side**. What can degrade is something else.

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

## Claude Code with `ANTHROPIC_BASE_URL` → llm_brain

### Always works (runs locally in the CLI)

| Feature | Why |
|---------|-----|
| Hooks (`PreToolUse`, `PostToolUse`, `Stop`, `UserPromptSubmit`…) | Shell scripts executed by the CLI |
| Skills | Markdown injected into the context: reaches the model as text |
| CLAUDE.md, slash commands, permissions | Client-side |
| MCP servers | Tools run locally; only the schema reaches the model |
| Subagents | Just more HTTP requests |
| [claude-kit](https://github.com/pjcau/claude-kit) | Portable hooks/skills: no impact |

### Depends on the translator (if wrong, it breaks)

SSE streaming, `tool_use`/`tool_result`, `cache_control`, `thinking`,
`count_tokens`, beta headers, `claude-*` mapping. Full list in
[API layer](./api-layer.md#translator-the-main-technical-risk-deferred).

### Degrades with the model behind (doesn't break, gets worse)

- **Skill adherence**: long prompts; a weak model partly ignores them.
  This is the real risk of "90% on fast".
- **Tool-call reliability**: weak models produce malformed tool calls →
  retries → more tokens → more cost. To measure in Phase 0.
- **Extended thinking**: only if the model behind has reasoning.
- **Context**: the limit is the model's behind, not 1M.
- **Prompt caching**: Claude Code leans on it heavily ([L1 cache](./cache.md)).

## aider with `OPENAI_API_BASE` → llm_brain

- No concept of hooks or skills. Equivalents: `.aider.conf.yml`,
  `--read CONVENTIONS.md`, `--lint-cmd`, `--test-cmd`, `--auto-test`.
- **`--auto-test` is the natural hook for escalation**
  ([details](./api-layer.md#escalation)).
- It uses LiteLLM internally: any `openai/<alias>` works; the
  `brain/fast`, `brain/reasoning` aliases from `/v1/models` are picked
  with `/model`.
- `.aider.model.metadata.json` is needed to tell aider the aliases'
  context and cost, otherwise it misestimates.
- The edit format (`diff`, `whole`, `udiff`) matters a lot for weak
  models: to try for every model in the `fast` tier.
