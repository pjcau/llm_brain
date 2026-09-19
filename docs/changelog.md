---
title: Iteration log
sidebar_position: 2
---

# Iteration log

| # | Date | What changed |
|---|------|--------------|
| 1 | 2026-09-19 | First draft: 90/10 plan, what agent-orchestrator has, LiteLLM, CLIs, open decisions. |
| 2 | 2026-09-19 | API layer abstracting caches and providers, how Claude Code and aider react (hooks and skills), integration with the GitHub apps, "OpenRouter today, GPU tomorrow" principle, 5 diagrams. |
| 3 | 2026-09-19 | Candidate model `prism-ml/ternary-bonsai-2-27b`: data verified on OpenRouter and HuggingFace, the bridge between cloud and local GPU. |
| 4 | 2026-09-19 | Migration from a single `CONSIDERAZIONI.md` to a Docusaurus site: short overview (< 2000 words) + detail pages. `scripts/sync-diagrams.py` keeps `diagrams/*.mmd` as the single source. |
| 5 | 2026-09-19 | Budget as the primary requirement: verified OpenRouter per-key limits (`limit_reset: daily`) and the Anthropic-compatible `/api/v1/messages` endpoint. New pages Budget, Nightly benchmark, Stack. Phase 0 → OpenRouter directly without LiteLLM. Phase 3 GPU downgraded to a wish. |
| 6 | 2026-09-19 | Cache logic: decision flow (diagram 07), the case of apps with no client-side cache, prompt caching through OpenRouter verified (automatic providers vs `cache_control`, `cached_tokens`, sticky routing), L2 software comparison and recommendation (SQLite exact, then sqlite-vec semantic for `assistant` only). |
| 7 | 2026-09-19 | **Answers to the decisions**: llm_brain is a single standalone backend (agent-orchestrator becomes a client, modules copied not imported); GPU deferred; Claude Code and aider together, measured; apps: assistant and find-a-car first; translator deferred; budget confirmed with `dev` up to 3 €/day + monthly ceiling 30 € soft / 40 € hard. Diagrams 01, 04, 05, 06 updated. |
| 8 | 2026-09-19 | Technical choices: **secrets management** (two families, hashed client keys, one key per profile on both sides, management key outside the runtime, diagram 08); the benchmark suite must be built from scratch (task format, sources, headless runner); repo structure decided. New open decision: where llm_brain runs relative to the apps. |
| 9 | 2026-09-19 | Topology clarified (assistant local, find-a-car and market on VPS/AWS): **no refresh tokens**, static per-app keys + rotation + `user` for rate limiting; proposal llm_brain on the VPS with Caddy, sops+age, Litestream for SQLite; table of where each platform keeps its key (SSM/Secrets Manager, docker secrets, LoadCredential). Diagram 09. |
| 10 | 2026-09-19 | **Authentication flow** client ↔ llm_brain: issuing only from the CLI on the server (no self-service), middleware step by step (headers, format, hash, revocation/expiry, IP allowlist, rate limit per key and per `user`, budget, audit), `api_keys` data model, client-side rules, commands. Diagram 10 (sequence). |
| 11 | 2026-09-19 | **Client compatibility** verified against Claude Code's official gateway guide: matrix for Claude Code / aider / app SDKs / agent-orchestrator (headers, `user`, correlation, 401/429), the gateway contract (endpoints, headers to forward, `system` untouched, `retry-after` > 60 for exhausted budget), the real risk = fields non-Claude models reject → **sanitizer** for the `dev` profile. The layer never prepends a system prompt for `dev`. |
| 12 | 2026-09-19 | **Hosting AWS vs VPS**: traffic estimate (~3 000 req/day, ~2.5 GB/month: negligible), monthly cost per option (Hetzner ~€4–5, Lightsail $7–12, EC2 t4g $13–20 with paid IPv4, Fargate/Lambda no), performance impact per component (the model dominates; +100–150 ms per hop to OpenRouter everywhere; SQLite and concurrency are not a limit). Recommendation: same cloud as the apps. Hosting line 5–15 €/month in the budget. Diagram 11. |
| 13 | 2026-09-19 | **Decided: llm_brain on a Hetzner VPS, apps with no location constraint.** Corrected the "never mixed" recommendation: mixed is acceptable because the public endpoint is protected by the auth design and the extra latency (+5–10 ms) is nothing next to the model. Open decision 1 closed; name/stack of the apps and benchmark repos remain. |
| 14 | 2026-09-19 | Documentation translated to English with English paths; site theme and logo; repository pushed to GitHub with a Pages deploy workflow. |
| 15 | 2026-09-19 | **Decided: all code in Rust** (axum, tokio, reqwest, serde_json, rusqlite, clap, governor, tracing). Stack page rewritten; agent-orchestrator modules become a port; repo layout is a Cargo workspace with `crates/brain/`; the docs helper is rewritten in Rust (`tools/sync-diagrams`); honest cost section. **claude-kit** added as a git submodule for skills, agents and hooks. |
