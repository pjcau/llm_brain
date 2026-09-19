---
title: Phase 0 runbook
sidebar_position: 3
---

# Phase 0 runbook: measure for a week on OpenRouter

Goal ([roadmap](./roadmap.md)): real consumption numbers per tool and tier,
the model choice for `fast`, and the first benchmark rows — **without ever
exceeding a daily limit**. No proxy yet: Claude Code and aider talk to
OpenRouter directly; the `brain` CLI provisions keys, snapshots usage and
runs the suite.

## What is in the repo now

| Piece | Where | Tests |
|-------|-------|-------|
| `brain keys provision \| list` — one OpenRouter key per profile with its daily `limit` (`limit_reset: daily`) | `crates/brain/src/keys.rs`, `openrouter.rs` | wiremock: request body, secret returned once, skip/force/only |
| `brain usage snapshot \| report` — `GET /key` per profile → SQLite; day × profile spend vs limit with the 70/85/100 degradation states | `usage.rs`, `db.rs` | wiremock + in-memory SQLite |
| `brain setup claude-code \| aider` — env blocks, `.aider.model.metadata.json` with tier prices | `setup.rs` | exact variables and flags |
| `brain bench run \| report` — clone/fetch cache, detached worktree at `commit_before`, `setup`, tool headless, fix's test files brought in **after** the tool, `verify`, cost = key `usage` delta | `bench/` | local git fixtures: pass, fail, timeout, missing tool, setup failure |
| Real `aider` in a container, launched with the exact env/args, hitting a mock OpenRouter | `crates/brain/tests/aider_container.rs`, `docker/aider-test.Dockerfile` | testcontainers (feature `docker-tests`), run in CI |
| Config | `config/profiles.yaml`, `config/tiers.yaml` | validated by a test |

Tiers as configured (prices from OpenRouter, 2026-09-19):
`fast` = `prism-ml/ternary-bonsai-2-27b` ($0.075 / $0.5 per M), fallback
`deepseek/deepseek-v4-flash` ($0.04 / $0.08); `reasoning` =
`deepseek/deepseek-v4-pro` ($0.42 / $0.84), fallback `qwen/qwen3.7-plus`.

## Steps

1. **OpenRouter account**: load a fixed amount of credits (≈ 40 €: the hard
   monthly wall) and create a **management key**.
2. `cp .env.example .env && chmod 600 .env`, put the management key in it.
3. `cargo run -p brain -- keys provision` → paste the printed
   `OPENROUTER_KEY_*=` lines into `.env`, then **remove the management key**.
   Check on the OpenRouter dashboard: five keys named `llm_brain/<profile>`,
   each with a daily limit.
4. `cargo run -p brain -- setup claude-code` and `setup aider`: paste the env
   blocks into your shell profile / `~/.claude/settings.json`; save the
   metadata JSON as `.aider.model.metadata.json` in the repos you work on.
5. Work normally for a week with both tools. Switch tier by hand
   (`/model deepseek/deepseek-v4-pro` in Claude Code, `--architect` in aider).
6. Snapshot usage a few times a day (cron every hour is fine):
   `cargo run -p brain -- usage snapshot`; read `usage report`.
7. Run the suite once with each tool: `brain bench run --tool aider` and
   `--tool claude`; compare with `brain bench report`.

## Exit criteria (from the roadmap)

- No daily limit ever hit: `usage report` never shows `EXHAUSTED`.
- A table of $/day per profile and $/task per tool from `usage report` and
  `bench report`.
- Malformed tool calls / protocol errors with Claude Code on non-Claude
  models: counted by hand this week, automated in Phase 1
  ([why they happen](./architecture/client-compatibility.md)).

## Known gaps, on purpose

- Token counts per request are not collected: `GET /key` reports USD only.
  Phase 1's proxy reads `usage` from every response.
- `brain bench mine` (automatic task extraction from git history) is not
  written; the first task (`ago-0001`) was mined by hand.
- Claude Code's headless flags (`-p … --output-format json
  --permission-mode acceptEdits`) are unit-tested but not container-tested:
  `claude` is not installable via pip. Verified manually in step 7.
