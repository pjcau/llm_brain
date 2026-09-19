---
title: Roadmap
---

# Roadmap

| Phase | What | Output | Exit criterion |
|-------|------|--------|----------------|
| 0 | **OpenRouter directly** (no LiteLLM): one key per profile with a daily limit, [bonsai-2-27b](./models/bonsai-2-27b.md) as the first `fast`, aider and Claude Code pointed at OpenRouter, ~1 week | real consumption numbers, model choice per tier, first benchmark suite | daily limit never exceeded; table of tokens/day and €/week per tool and tier; malformed tool calls < threshold |
| 1 | Aware reverse proxy, **standalone `llm_brain` repo**: endpoints in both dialects, profiles → key, alias → model, **budget with degradation**, SQLite usage | llm_brain usable by the CLI, by agent-orchestrator (`ago` profile) and by assistant + find-a-car; spend visible per profile | Claude Code and aider work a full day without protocol errors; degradation kicks in at the planned thresholds |
| 2 | Nightly benchmark with automatic promotion, escalation on failure, L2 cache per profile | quality/cost improving every day | quality/cost frontier chart in the dashboard; at least one candidate promoted with data |
| 3 *(wish, not planned)* | `fast` tier on a local GPU (bonsai GGUF), `hybrid` preset | cloud only for `reasoning` | — |

Detail per phase: [Stack](./analysis/stack.md) and [Budget](./architecture/budget.md) (0–1), [API layer](./architecture/api-layer.md) (1), [Benchmark](./architecture/benchmark.md) (2), [GPU](./architecture/gpu.md) (3, wish).
