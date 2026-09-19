---
title: Tool landscape
---

# Tool landscape

To analyse with criteria: maturity, supported dialects, routing, budget,
licence. Comparison matrix to be filled in Phase 0.

## Gateways

- **LiteLLM** — general-purpose proxy/gateway ([dedicated page](./litellm.md))
- **OpenRouter** — hosted aggregator, one API for every model; the
  project's starting point
- **claude-code-router** — proxy specific to Claude Code → other models;
  reference for the Anthropic ↔ OpenAI translator

## CLIs / IDEs

- **aider** — [the safe choice](./cli.md)
- **OpenCode** — multi-provider CLI, TUI similar to Claude Code
- **Cline / Roo Code** — VS Code extensions, BYO endpoint
- **Continue** — IDE assistant with per-model/role config

## Your related repos

- [claude-kit](https://github.com/pjcau/claude-kit) — portable hooks/skills
- [AgentsBoard](https://github.com/pjcau/AgentsBoard) — macOS mission
  control for Claude Code/Codex: a potential client of the layer
