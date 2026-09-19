---
title: Decisions
---

# Decisions

## Taken

| Date | Decision | Consequence |
|------|----------|-------------|
| 2026-09-19 | Start with OpenRouter; moving to a local GPU must stay a config change. | rule: no code outside `providers/` knows the provider |
| 2026-09-19 | `prism-ml/ternary-bonsai-2-27b` is the `fast` candidate to evaluate in Phase 0. | [Bonsai](./models/bonsai-2-27b.md) |
| 2026-09-19 | Documentation as a Docusaurus site, overview < 2000 words, updated at every iteration. | |
| 2026-09-19 | **Budget is the primary requirement**: OpenRouter key per profile with a daily limit + degradation in the layer + benchmark with a separate key. | [Budget](./architecture/budget.md) |
| 2026-09-19 | Phase 0 on OpenRouter directly, without LiteLLM. | [Stack](./analysis/stack.md) |
| 2026-09-19 | **llm_brain is a single, standalone backend** that centralizes every LLM request. agent-orchestrator **uses it** as a client, like the apps. | own repo; `usage.py`, `cache.py`, `openrouter.py`, `evaluator.py` are **ported** from agent-orchestrator, not depended on — [What to reuse](./analysis/agent-orchestrator.md) |
| 2026-09-19 | Local GPU: discussed later. Phase 3 stays a wish. | |
| 2026-09-19 | **Claude Code and aider together** in Phase 0: measure who burns less and reaches the result first. | the benchmark compares tools too, not just models — [Benchmark](./architecture/benchmark.md) |
| 2026-09-19 | Apps to integrate: **assistant and find-a-car first**, then the second-hand market. | [Apps](./architecture/apps.md) |
| 2026-09-19 | Translator deferred: in Phase 1 the layer is a reverse proxy. | |
| 2026-09-19 | Budget per profile confirmed, with **`dev` up to 3 €/day** as a peak cap; the monthly ceiling stays ≈ 40 €. | [Budget](./architecture/budget.md#budget-per-profile) |
| 2026-09-19 | **llm_brain runs on a VPS (Hetzner, ~€4–5/month)** with a protected public HTTPS endpoint; **the apps have no location constraint** (AWS, VPS, home). Performance is not a criterion: model time dominates. | [Hosting](./analysis/hosting-costs.md#decision-2026-09-19-llm_brain-on-a-vps-apps-anywhere) |
| 2026-09-19 | `llm_brain` repo structure: Cargo workspace `crates/brain/` + `tools/` + `bench/` + `config/` + `deploy/` + `.claude-kit/` + `website/`. | [Stack](./analysis/stack.md#repo-structure-updated-for-rust) |
| 2026-09-19 | The benchmark suite **doesn't exist and must be built** in `bench/`, first source agent-orchestrator (git history + tests). | [Benchmark](./architecture/benchmark.md#the-suite-it-doesnt-exist-it-has-to-be-built) |
| 2026-09-19 | Secrets: two families (upstream only in llm_brain, client keys hashed), one key per profile on both sides, management key never at runtime. | [Secrets](./architecture/secrets.md) |
| 2026-09-19 | **No refresh tokens**: llm_brain is machine-to-machine, static per-app keys with rotation and revocation; end users authenticate to the app, never to llm_brain; apps pass `user` for per-user rate limiting. | [Auth](./architecture/auth-topology.md) |
| 2026-09-19 | Documentation in English, styled site with its own logo, published on GitHub Pages of the `llm_brain` repo. | |
| 2026-09-19 | **All code in Rust** (axum + tokio + reqwest + rusqlite + clap), including helpers: one static binary on the VPS. agent-orchestrator's modules are ported, not copied. | [Stack](./analysis/stack.md) |
| 2026-09-19 | [claude-kit](https://github.com/pjcau/claude-kit) added as a git submodule (`.claude-kit/`) for the skills, agents and hooks used in the dev workflow. | [Stack](./analysis/stack.md) |

## Open

1. **Name and stack of assistant and find-a-car** (non-public repos):
   needed for the profile, the versioned system prompt and the budget
   estimate. → [Apps](./architecture/apps.md)
2. **Besides agent-orchestrator, which other repos of yours have tests**
   to extract benchmark tasks from? → [Benchmark](./architecture/benchmark.md)
