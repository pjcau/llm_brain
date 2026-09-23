---
title: Tool landscape
---

# Tool landscape

What was looked at around the project, and where each piece ended up.
The planned comparison matrix was not filled: the choices were made on
[benchmark runs](../phase-1.md#agents-compared-through-the-proxy) instead.

## Gateways

| Tool | What it is | Status in llm_brain |
|------|------------|---------------------|
| **OpenRouter** | hosted aggregator, both dialects, per-key limits | the upstream of every tier |
| **LiteLLM** | general-purpose proxy/gateway | not used ([why](#why-not-litellm)) |
| **claude-code-router** | proxy from Claude Code to other models | not used; reference if a translator is ever needed |

### Why not LiteLLM

The first plan was LiteLLM for a week in Phase 0, then a native proxy. It
was dropped once OpenRouter turned out to speak both dialects and to offer
per-key daily limits without a database: Phase 0 ran on OpenRouter
directly ([Stack](./stack.md)). As a gateway LiteLLM enforces budgets only
with Postgres, is one more service to run, routes by config only, and
doesn't reuse llm_brain's usage DB, board or `brain/auto`. If a format
translator is ever needed, its Anthropic ↔ OpenAI mapping is a reference
to port, alongside claude-code-router.

## CLIs / IDEs

| Tool | Status |
|------|--------|
| **Claude Code** | daily agent, through the proxy ([Which CLI](./cli.md)) |
| **aider** | cheap editor (architect/editor) |
| **OpenCode** | benchmarked (`brain bench run --tool opencode`) |
| **Cline / Roo Code**, **Continue** | not evaluated; any of them works as a client of the OpenAI-compatible endpoint |

## Related repos

- [claude-kit](https://github.com/pjcau/claude-kit) — portable hooks/skills,
  a submodule of this repo
- [agent-orchestrator](./agent-orchestrator.md) — a client (`ago` profile)
- [AgentsBoard](https://github.com/pjcau/AgentsBoard) — macOS mission
  control for Claude Code/Codex: a potential client of the layer
