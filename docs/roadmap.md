---
title: Roadmap
---

# Roadmap

| Phase | What | Status | Exit criterion |
|-------|------|--------|----------------|
| 0 | **OpenRouter directly**: one key per profile with a daily limit, CLIs pointed at it, first benchmark task, logs ingested for errors and anomalies | **done** (2026-09-19 → 20) — [runbook](./phase-0.md) | met: tiers chosen from data, no daily limit exceeded |
| 1 | **Key-authenticated reverse proxy** on the VPS: both dialects, profiles → OpenRouter key, `brain/<tier>` aliases, budget with degradation, per-request usage, board | **done and live** (2026-09-20), extended since with `brain/auto`, the catalog, provider tracking — [runbook](./phase-1.md) | met for Claude Code, aider and find-a-car; agent-orchestrator not wired yet |
| 1.x | **Now**: provider pinning per model (cache misses across backends), agent-orchestrator as a client, assistant live, Litestream backups | in progress | pinning measured against the 82.5 % cache baseline of `agent`; every client on the proxy |
| 2 | Nightly benchmark with automatic promotion, escalation on failure, L2 response cache per profile | not started (the suite has 1 task) | at least one candidate promoted with data |
| 3 *(wish)* | `fast` tier on a local GPU (bonsai GGUF) | not planned | — |

Detail: [Budget](./architecture/budget.md) and [API layer](./architecture/api-layer.md)
(0–1), [Auto routing](./architecture/auto-routing.md) and [Cache](./architecture/cache.md) (1.x),
[Benchmark](./architecture/benchmark.md) (2), [GPU](./architecture/gpu.md) (3).
