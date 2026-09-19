---
title: LiteLLM
---

# LiteLLM: use it, but the right way

LiteLLM already does OpenAI-compatible proxy + routing + budget + fallback.

| Pros | Cons |
|------|------|
| Ready now, zero code | One more service to run |
| Mature, many providers | Config-only routing, not semantic |
| Built-in budgets and alerts | Doesn't reuse your usage DB / dashboard / router |

## Recommendation: sequence, don't choose

- **Phase 0 — plain LiteLLM, ~1 week, zero code.** Measure real
  consumption (tokens/day, fast/reasoning split, cost) and pick models
  with data in hand.
- **Phase 1 — native proxy in llm_brain**, built with those numbers.

Building before measuring is the classic way to optimize the wrong thing.

:::note Superseded
After verifying that OpenRouter speaks both dialects and offers per-key
daily limits without a database, Phase 0 runs on **OpenRouter directly**
and LiteLLM is out of it ([Stack](./stack.md)). This page stays as the
record of the reasoning.
:::

## Reuse as a library

LiteLLM already has the Anthropic ↔ OpenAI mapping. It is one of the three
options for the translator ([decisions](../decisions.md)): write it from
scratch, use LiteLLM as a library inside llm_brain, or adapt
claude-code-router.
