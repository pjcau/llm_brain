---
title: Which CLI
---

# Which CLI

:::note Decided
**Both** go forward in Phase 0. For every task the benchmark measures who
burns fewer tokens/€ and who reaches the result first (turns, time). The
choice, or a per-task-type coexistence, comes out of the data.
:::

## aider — the safe choice

- Native OpenAI-compatible endpoint, no hacks.
- **architect/editor** mode is exactly the 90/10: `--editor-model` = `fast`
  tier, `--model` (architect) = `reasoning` tier, manual switch with
  `/model`.
- Frugal with tokens (repo map, doesn't read everything).
- Manual routing = a perfect v1.

## Claude Code pointed at the proxy

- `ANTHROPIC_BASE_URL` towards an Anthropic-compatible proxy (what
  claude-code-router does).
- Pro: keep habits, skills, hooks — which [always work](../architecture/claude-code-aider.md).
- Con: token-"hungry"; some features assume Claude (caching, tool schema,
  thinking). To be evaluated, not assumed.

## OpenCode

Open source, a TUI similar to Claude Code, native multi-provider. Third
candidate, to try in the measurement phase.

## On automatic routing

The regex classifier in `router.py` is weak by nature (keyword matching).
The better signal is **escalation on failure**: the `fast` model produces
an edit; if tests/lint fail, retry with `reasoning`. That's v2, after
manual routing. Details in [API layer](../architecture/api-layer.md#escalation).
