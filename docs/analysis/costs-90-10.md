---
title: Costs and the 90/10 plan
---

# Costs and the 90/10 plan

The starting plan (a cheap driver for 90% of queries, a reasoning model for
10%) was right in shape. It became a **ladder of tiers**, and the choice of
rung is made per session by [`brain/auto`](../architecture/auto-routing.md),
not by hand. Goal unchanged: the whole LLM bill under **40 €/month**.

## Tiers, not models

The system **does not hard-code models**: clients ask for a tier
(`brain/<tier>` or `brain/auto`), `config/tiers.yaml` maps it to an
OpenRouter model. Prices from the OpenRouter catalog, checked 2026-09-22
(about 2× the 2026-09-20 prices), per 1M tokens:

| Tier        | Role                                          | Model | in $ | out $ | Fallback |
|-------------|-----------------------------------------------|-------|------|-------|----------|
| `fast`      | trivial edits, short questions                | `deepseek/deepseek-v4-flash` | 0.09 | 0.18 | qwen/qwen3.7-flash |
| `medium`    | focused change in one or two files            | `z-ai/glm-5.3-flash` | 0.15 | 0.50 | deepseek-v4-flash |
| `agent`     | Claude Code's main model: multi-file work, debugging | `deepseek/deepseek-v4-pro` | 0.96 | 1.91 | qwen/qwen3.7-plus |
| `max`       | large refactors, design, subtle bugs (not yet measured against `agent`) | `z-ai/glm-5.3` | 0.84 | 2.64 | deepseek-v4-pro |
| `reasoning` | aider's architect role                        | [`prism-ml/ternary-bonsai-2-27b`](../models/bonsai-2-27b.md) | 0.075 | 0.50 | deepseek-v4-pro |
| `premium`   | optional, rare cases                          | unset | — | — | — |

`tiers.yaml` is the source of truth; prices are re-read from the catalog
(`brain models`), never from memory.

## The real cost

A coding agent burns 10–50× the tokens of a chat: it re-reads files, runs
tools, retries. The levers that matter, in order:

1. **Which tool, and for what** — Claude Code through the proxy is the
   daily agent (reliable tool loop, ~0.07–0.12 $ per benchmark task on
   `agent`); aider is the cheap editor for targeted changes (< 0.001 $ on
   the same task); OpenCode matches the agent at a fraction of the tokens
   ([measured](../phase-1.md#agents-compared-through-the-proxy),
   [Which CLI](./cli.md)).
2. **Which rung** — `brain/auto` sends each session to the lightest tier
   that fits; the board shows the saving against the `agent` baseline.
3. **Prompt caching** — system prompt + tools + skills repeat identically
   every turn ([L1 cache](../architecture/cache.md)).
4. **Context trimming** — repo map instead of whole files.
5. Only then: the model's price per token.

## Fast-tier price landscape (2026-09-20 snapshot)

Compared when `fast` was chosen. Prices have roughly doubled since; the
ranking was the point. Per 1M tokens, all with tools and reasoning.

| Model | in $ | out $ | 1M in + 50k out |
|-------|------|-------|-----------------|
| qwen/qwen3.7-flash | 0.030 | 0.13 | $0.037 |
| **deepseek/deepseek-v4-flash** (`fast`) | **0.036** | **0.073** | **$0.040** |
| openai/gpt-5-nano | 0.05 | 0.40 | $0.07 |
| z-ai/glm-5.3-flash | 0.09 | 0.30 | $0.105 |
| google/gemini-2.5-flash-lite | 0.10 | 0.40 | $0.12 |
| google/gemini-3.8-flash | 0.75 | 3.75 | $0.94 |
| anthropic/claude-haiku-4.5 | 1.00 | 5.00 | $1.25 |

The last column is a coding agent's typical mix (~20 input tokens per
output token). Current Gemini Flash costs ~20× deepseek-v4-flash on input;
Gemini enters a tier only if the benchmark justifies it
(`brain bench run --tool aider --tier fast --model <id>`).

## What is not realistic

Replacing the subscription is realistic with cheap models via API. It is
**not** with Claude via pay-per-use API: the Max subscription is heavily
subsidised and the API would cost more. Claude stays at most in the
`premium` tier, for rare cases.

## Measured spend

Per-profile spend, cache hit and cold turns are on the board
(`brain serve`); budgets per profile are in
[Budget](../architecture/budget.md#budget-per-profile).
