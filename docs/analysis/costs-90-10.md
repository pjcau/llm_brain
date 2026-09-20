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
| `fast`      | 90% of queries: edits, reading, explanations  | `deepseek/deepseek-v4-flash` (decided 2026-09-20) |
| `reasoning` | hard bugs, complex logic design               | [`prism-ml/ternary-bonsai-2-27b`](../models/bonsai-2-27b.md), fallback deepseek-v4-pro |
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

## Fast-tier price landscape (OpenRouter API, 2026-09-20)

Per 1M tokens; every row supports tools and reasoning.

| Model | in $ | out $ | ctx | cache read $ | 1M in + 50k out |
|-------|------|-------|-----|--------------|-----------------|
| qwen/qwen3.7-flash | 0.030 | 0.13 | 1M | 0.006 | $0.037 |
| **deepseek/deepseek-v4-flash** (`fast`) | **0.036** | **0.073** | 1M | 0.007 | **$0.040** |
| openai/gpt-5-nano | 0.05 | 0.40 | 400k | 0.005 | $0.07 |
| z-ai/glm-5.3-flash | 0.09 | 0.30 | 1.3M | 0.018 | $0.105 |
| google/gemini-2.5-flash-lite (cheapest Gemini) | 0.10 | 0.40 | 1M | 0.010 | $0.12 |
| google/gemini-3.1-flash-lite | 0.25 | 1.50 | 1M | 0.025 | $0.325 |
| google/gemini-2.5-flash | 0.30 | 2.50 | 1M | 0.030 | $0.425 |
| google/gemini-3.8-flash (current "Flash") | 0.75 | 3.75 | 1M | 0.075 | $0.94 |
| anthropic/claude-haiku-4.5 | 1.00 | 5.00 | 200k | 0.10 | $1.25 |

The last column is the cost of a coding agent's typical mix (~20 input
tokens per output token). Gemini Flash was the price reference in its
2.0/2.5 era; today the current Flash costs ~20× deepseek-v4-flash on
input and ~50× on output, and even 2.5 Flash-Lite is 3–5×. Gemini enters
the `fast` tier only if quality justifies it — measurable with
`brain bench run --tool aider --tier fast --model google/gemini-2.5-flash-lite`.

## What is not realistic

Replacing the subscription is realistic with cheap models via API. It is
**not** with Claude via pay-per-use API: the Max subscription is heavily
subsidised and the API would cost more. Claude stays at most in the
`premium` tier, for rare cases.

## What this page still lacks

Numeric estimates of tokens/month per usage profile and 90/10 vs 70/30
scenarios: they come after [Phase 0](../roadmap.md) with measured
consumption.
