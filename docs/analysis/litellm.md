---
title: LiteLLM
---

# LiteLLM

:::note Superseded (2026-09-19)
The first plan was "LiteLLM for a week in Phase 0, measure, then build the
native proxy". It was dropped once OpenRouter turned out to speak both
dialects and to offer per-key daily limits without a database: Phase 0 ran
on **OpenRouter directly**, and llm_brain is a Rust reverse proxy with no
LiteLLM in it ([Stack](./stack.md), [decisions](../decisions.md)).
:::

What still holds:

- **Why not as the gateway**: budgets only with Postgres, one more service
  to run, config-only routing, and it doesn't reuse llm_brain's usage DB,
  board or `brain/auto`.
- **If a format translator is ever needed** (a local provider that speaks
  neither dialect), LiteLLM's Anthropic ↔ OpenAI mapping is a reference,
  alongside claude-code-router. llm_brain being all Rust, it would be a
  reference to port, not a library to embed.
