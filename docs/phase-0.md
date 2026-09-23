---
title: Phase 0 runbook
sidebar_position: 3
---

# Phase 0: measure on OpenRouter (done)

Goal ([roadmap](./roadmap.md)): real consumption numbers per tool and tier,
the model choice for `fast`, and the first benchmark rows — without ever
exceeding a daily limit. No proxy: Claude Code and aider talked to
OpenRouter directly with one key per profile; the `brain` CLI provisioned
the keys, snapshotted usage, read the tools' logs and ran the suite.

:::info Status
Done on 2026-09-20 — the planned week was cut short because the
[Phase 1 proxy](./phase-1.md) was built the same day and every request now
goes through it. This page keeps what was measured and what it decided.
How to install and operate `brain` (keys, setup, board, events, bench) is
in the [Configuration reference](./configuration.md).
:::

## What Phase 0 built (still in use)

| Piece | Where |
|-------|-------|
| One OpenRouter key per profile with its daily `limit` — ring 1 of the [budget](./architecture/budget.md) (`brain upstream provision\|sync\|list`) | `keys.rs`, `openrouter.rs` |
| `GET /key` snapshots per profile → SQLite, day × profile spend vs limit (`brain usage`) | `usage.rs`, `db.rs` |
| Client configuration for Claude Code and aider (`brain setup`) | `setup.rs` |
| Tool-log reader: aider chat history and Claude Code transcripts → requests, errors, retries, tokens, cache hit (`brain events`) | `events.rs` |
| Benchmark runner: worktree at `commit_before`, tool headless, fix's tests brought in after the tool, `verify`, cost = key delta (`brain bench`) | `bench/` |
| Real aider in a container against a mock OpenRouter | `crates/brain/tests/aider_container.rs` (feature `docker-tests`) |

## First benchmark rows (2026-09-20, task `ago-0001`)

| Tool | Model | Result | Time | Cost (key delta) |
|------|-------|--------|------|------------------|
| Claude Code (host) | bonsai-2-27b | FAIL — `429 Provider returned error`, 12 min of retries | 746 s | ~0.006 $ * |
| aider (Docker) | bonsai-2-27b | FAIL | 831 s | ~0.008 $ * |
| **aider (Docker)** | **deepseek-v4-flash** | **PASS** — the fix's test passes | **260 s** | **0.004 $** |
| Claude Code (host) | deepseek-v4-flash | FAIL — one correct `Read`, then an empty reply; Claude Code stops | 16 s | ~0.000 $ |
| aider (Docker) | bonsai alone, `whole`, 30-min timeout | FAIL — provider rate limits in series, whole-file rewrites | 900 s | 0.013 $ |
| aider (Docker) | architect bonsai + editor deepseek-v4-flash, `whole`, fallback on | FAIL — no rate limits any more, but the editor's whole-file edit was not applied (101k tokens in one turn) | 400 s | 0.003 $ |
| **aider (Docker)** | **deepseek-v4-flash, `diff`** | **PASS** | **39 s** | < 0.001 $ |
| **aider (Docker)** | **architect bonsai + editor deepseek-v4-flash, `diff`, fallback on** | **PASS** — bonsai proposes, deepseek applies | **74 s** | < 0.001 $ |

\* the two bonsai runs overlapped on the same key, so their costs are mixed.
Prices at the time: deepseek-v4-flash $0.04 / $0.08 per M (they roughly
doubled by 2026-09-22, see [`config/tiers.yaml`](./configuration.md#configtiersyaml)).

Same task, from 260 s / 30k→8.7k tokens (`whole`) to 39 s / 12k→676
tokens (`diff`): **the edit format was worth more than the model choice.**

## Anomalies and their root causes

| # | Symptom | Root cause | Fix |
|---|---------|------------|-----|
| 1 | Both tools FAIL on bonsai; Claude Code retries a `429` for 12 min | bonsai has a single provider: 33 s per request vs 5.5 s for deepseek-v4-flash, no prompt cache, rejects Claude Code's parallel requests ([probes](./models/bonsai-2-27b.md#measured-on-2026-09-20)) | bonsai leaves the daily-driver role |
| 2 | aider on bonsai: `Provider returned error` + retries | the same provider rejects large requests intermittently | OpenRouter `models[]` fallback per tier in the aider settings file; for Claude Code the proxy adds it (OpenAI dialect) |
| 3 | editor edit not applied, 101k tokens in one turn | aider gives unknown models the `whole` format | the fast tier gets `diff` / `editor-diff` in the generated settings |
| 4 | Claude Code on a non-Claude model: empty reply after the first tool result | the tool-loop reliability risk from [Claude Code and aider](./architecture/claude-code-aider.md) | fixed in the proxy (system-turn fix, [Phase 1](./phase-1.md#agents-compared-through-the-proxy)) |

## Decided (2026-09-20)

- `fast` = `deepseek/deepseek-v4-flash` (cheapest, reasoning + tools,
  solved the task).
- **bonsai moves to `reasoning`**: architect in aider's architect/editor
  split, where it proposes and deepseek-v4-flash applies the edits, so its
  latency matters less and it never produces edits itself.
- No integration into agent-orchestrator until the local setup has run for
  a while; it stays a read-only source of benchmark tasks.

## What Phase 0 left open

- `brain bench mine` (task extraction from git history) is not written;
  `ago-0001` was mined by hand.
- Claude Code's headless bench flags are unit-tested, not container-tested
  (the Claude Code image hangs on first start, see
  [Configuration](./configuration.md#brain-commands)).
