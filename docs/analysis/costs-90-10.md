---
title: Costs and the 90/10 plan
---

# Costs and the 90/10 plan

The starting plan (a cheap driver for 90% of queries, a reasoning model for
10%) is right in shape. Two corrections.

## Tiers, not models

The models named at the start ("Gemini 2.0 Flash", "DeepSeek R1") date from
early 2025; agent-orchestrator's own presets already use
`deepseek/deepseek-v4-flash`. The system **must not hard-code models** but
define tiers:

| Tier        | Role                                          | Model |
|-------------|-----------------------------------------------|-------|
| `fast`      | 90% of queries: edits, reading, explanations  | config — candidate [bonsai-2-27b](../models/bonsai-2-27b.md) |
| `reasoning` | hard bugs, complex logic design               | config |
| `premium`   | optional, rare cases                          | config |

Prices and availability are checked on the price lists at decision time,
never from memory.

## The real cost

A coding agent burns 10–50× the tokens of a chat: it re-reads files, runs
tools, retries. The levers that matter, in order:

1. **Which tool you use** — aider is far more frugal than Claude Code
   ([comparison](./cli.md)).
2. **Prompt caching** — system prompt + tools + skills repeat identically
   every turn ([L1 cache](../architecture/cache.md)).
3. **Context trimming** — repo map instead of whole files.
4. Only then: the model's price per token.

## What is not realistic

Replacing the subscription is realistic with cheap models via API. It is
**not** with Claude via pay-per-use API: the Max subscription is heavily
subsidised and the API would cost more. Claude stays at most in the
`premium` tier, for rare cases.

## What this page still lacks

Numeric estimates of tokens/month per usage profile and 90/10 vs 70/30
scenarios: they come after [Phase 0](../roadmap.md) with measured
consumption.
