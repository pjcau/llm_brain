---
title: Which CLI
---

# Which CLI

:::note Decided 2026-09-20
- **Claude Code through the proxy is the daily agent** (`px-claude`, model
  `brain/auto`; the `agent` tier is deepseek-v4-pro).
- **aider is the cheap editor** for targeted changes (architect/editor),
  not the agent.
- **OpenCode is benchmarked** as the open-source third agent.

This replaces "both go forward on equal terms" (2026-09-19). Numbers:
[agents compared through the proxy](../phase-1.md#agents-compared-through-the-proxy).
:::

## Claude Code — the daily agent

- `ANTHROPIC_BASE_URL` → llm_brain, client key of the `dev` profile
  (`brain setup claude-code --proxy`).
- Keeps habits, skills, hooks, which [always work](../architecture/claude-code-aider.md).
- Token-hungry (large system prompt, many turns); the proxy's sanitizer
  removes the Claude-only fields other models reject
  ([client compatibility](../architecture/client-compatibility.md)).
- Needs a model with a reliable tool loop: that is why its baseline is the
  `agent` tier, and `brain/auto` only moves light sessions down the ladder.

## aider — the cheap editor

- Native OpenAI-compatible endpoint (`brain setup aider --proxy`).
- **architect/editor** is the original 90/10: `--model` = `reasoning`
  tier (architect), `--editor-model` = `fast` tier.
- Frugal with tokens (repo map, doesn't read everything): the cheapest way
  to apply a change you already know you want.

## OpenCode

Open source, a TUI similar to Claude Code, native multi-provider. Passes
the benchmark through the proxy with far fewer tokens than Claude Code
(`brain bench run --tool opencode`); kept as a measured alternative.

## Routing

Superseded by [`brain/auto`](../architecture/auto-routing.md): a decision
model picks the tier once per session. Escalation on failure (retry with a
higher tier when tests fail) is a design, **not built yet**
([API layer](../architecture/api-layer.md#escalation)).
