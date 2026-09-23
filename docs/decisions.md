---
title: Decisions
---

# Decisions

In force, grouped by topic. A decision that was later replaced is listed
once, under what replaced it; the full history is in the
[changelog](./changelog.md).

## Scope and shape

| Date | Decision | Where |
|------|----------|-------|
| 2026-09-19 | **llm_brain is a single, standalone backend** that centralizes every LLM request; agent-orchestrator and the apps are clients. agent-orchestrator's modules inspired the design, nothing is imported from it. | [agent-orchestrator](./analysis/agent-orchestrator.md) |
| 2026-09-19 | **Budget is the primary requirement**: OpenRouter key per profile with a daily limit (ring 1) + degradation in the layer (ring 2). | [Budget](./architecture/budget.md) |
| 2026-09-19 | OpenRouter today; a local GPU must stay a config change. The GPU (Phase 3) is a wish, not a plan. | [GPU](./architecture/gpu.md) |
| 2026-09-19 | No LiteLLM, no translator: the layer is a **reverse proxy**, because OpenRouter already speaks both dialects. A translator comes only with a provider that doesn't. | [Stack](./analysis/stack.md) |
| 2026-09-19 | Apps: **assistant and find-a-car first**, then the second-hand market. | [Apps](./architecture/apps.md) |

## Code and docs

| Date | Decision | Where |
|------|----------|-------|
| 2026-09-19 | **All code in Rust** (axum, tokio, reqwest, rusqlite, clap), helpers included: one static binary. | [Stack](./analysis/stack.md) |
| 2026-09-19 | [claude-kit](https://github.com/pjcau/claude-kit) as the `.claude-kit/` submodule for skills, agents and hooks. | |
| 2026-09-19 | Docs: Docusaurus site in English on GitHub Pages, overview < 2000 words, a changelog row at every iteration. | |

## Security

| Date | Decision | Where |
|------|----------|-------|
| 2026-09-19 | Two families of secrets: upstream OpenRouter keys only on the server, client keys `brain_<profile>_…` stored hashed; the management key never at runtime. | [Secrets](./architecture/secrets.md) |
| 2026-09-19 | **No refresh tokens**: machine-to-machine, static per-app keys with rotation and revocation, issued only from the server's CLI; end users authenticate to the app; apps pass `user` for per-user rate limiting. | [Auth](./architecture/auth-topology.md) · [Flow](./architecture/auth-flow.md) |

## Hosting

| Date | Decision | Where |
|------|----------|-------|
| 2026-09-19 | **llm_brain on a small VPS** with a protected public HTTPS endpoint; **apps anywhere**. Performance is not a criterion (the model dominates). Planned on Hetzner; **running on Contabo** since 2026-09-20 (same shape, x86_64). AWS only at an HA step. | [Hosting](./analysis/hosting-costs.md) · [Deploy](./deploy-vps.md) |

## Models, tiers and tools

| Date | Decision | Where |
|------|----------|-------|
| 2026-09-20 | **`fast` = deepseek-v4-flash, `reasoning` = bonsai-2-27b** (architect role, fallback deepseek-v4-pro), from the first benchmark rows. Replaces bonsai as the `fast` candidate (2026-09-19). | [Phase 0](./phase-0.md) · [Bonsai](./models/bonsai-2-27b.md) |
| 2026-09-20 | **Daily agent = Claude Code through the proxy (`px-claude`)**, `agent` tier = deepseek-v4-pro. **aider = cheap editor** for targeted changes. OpenCode benchmarked as the open-source agent. Replaces "Claude Code and aider on equal terms" (2026-09-19). | [Which CLI](./analysis/cli.md) · [Phase 1](./phase-1.md) |
| 2026-09-22 | **`brain/auto`: tier chosen once per session by a decision model** (Jev), never per turn. Ladder `fast → medium → agent → max`, fallback `agent`; per-profile ladders for apps. `openrouter/auto` rejected. | [Auto routing](./architecture/auto-routing.md) |

## Budgets

| Date | Decision | Where |
|------|----------|-------|
| 2026-09-22 | Budgets in USD per profile (`profiles.yaml`): **`dev` 5 $/day, 60 $/month soft** (raised from 3 €/day after the first measured day); `assistant` and `car` 0.50 $/day, 15 $/month; `ago` 1/5; `benchmark` 0.50/5. The benchmark never spends `dev`'s key. | [Budget](./architecture/budget.md#budget-per-profile) |

## Open

1. **Other repos with tests** besides agent-orchestrator, to extract
   benchmark tasks from (the suite has one task today). → [Benchmark](./architecture/benchmark.md)
2. **Provider pinning** per model (`provider.order`), now that the
   serving backend is recorded and cross-backend cache misses are
   measured. → [Cache](./architecture/cache.md)
