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

Tiers as configured (decided 2026-09-20 from the first benchmark rows):
`fast` = `deepseek/deepseek-v4-flash` ($0.04 / $0.08 per M), fallback
`qwen/qwen3.7-flash`; `reasoning` = `prism-ml/ternary-bonsai-2-27b`
($0.075 / $0.5), fallback `deepseek/deepseek-v4-pro`. In aider that is
architect = bonsai, editor = deepseek-v4-flash.

## Steps

0. Install the CLI on your PATH and point it at the repo:
   `cargo install --path crates/brain --root ~/.local` and `BRAIN_HOME=<repo>`
   in `.env` ([details](./configuration.md#install-brain-and-use-it-from-any-folder)).
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
   (`/model prism-ml/ternary-bonsai-2-27b` in Claude Code, `--architect` in aider).
6. Snapshot usage a few times a day (cron every hour is fine):
   `cargo run -p brain -- usage snapshot`; read `usage report`.
7. Run the suite once with each tool: `brain bench run --tool aider` and
   `--tool claude`; compare with `brain bench report`.

## Test it yourself: aider from Docker, Claude Code from the host

aider is not installed on the host; the image built for the container test
works as the runtime. `brain setup aider --docker llm-brain-aider-test:latest`
prints a shell function; paste it into `~/.bashrc` together with the keys:

The full functions (`or-aider`, `or-claude`), the `.env` variable names,
the YAML files and the cron line are in the
[Configuration reference](./configuration.md) — variable names only, no
secrets.

Then, inside any git repo: `or-aider` (architect = reasoning tier, editor =
fast tier, `/model` to switch) or `or-claude`. Plain `claude` and a plain
`aider` install keep working as before: the functions only set variables
for that one invocation. The `dev` key pays, capped at 3 $/day.

The metadata JSON from `brain setup aider` lives at
`~/.local/share/llm_brain/aider-model-metadata.json`, so aider shows real
costs for the tier models.

## The board

`docker compose -f deploy/docker-compose.board.yml up -d --build` →
**http://127.0.0.1:8090/**: budget per profile with the degradation
states, anomalies (OpenRouter traffic only), requests per day × tool ×
model with errors/retries/tokens/cache/latency, benchmark runs. It
snapshots and ingests on its own every 10 minutes
([details](./configuration.md#the-board-brain-serve-in-docker)).

## What brain watches while you work

No proxy yet, so the layer reads what the tools leave on disk:

| Source | What it gives | Where |
|--------|---------------|-------|
| aider chat history (`--chat-history-file`) | one `Tokens: N sent, M received` line per LLM round trip, the model, and every notice: `litellm.*Error`, `Retrying…`, `did not conform to the edit format`, `failed to apply` | `$BRAIN_DATA/aider-chat.md` |
| Claude Code session transcripts | per assistant message: model and `usage` (input, cache read, cache write, output); API errors as `isApiErrorMessage` entries | `~/.claude/projects/*/*.jsonl` |
| OpenRouter `GET /key` | real spend per profile, day and month | `brain usage snapshot` |

`brain events ingest` reads only the new bytes of each file (offsets in
SQLite) and `brain events report` prints day × tool × model: requests,
errors, bad edits, retries, tokens, cache hit, estimated cost from the tier
prices. Anomaly rules, on purpose simple:

- error rate (errors + bad edits) > 10% on ≥ 5 requests
- ≥ 3 retries in a day
- cache hit < 30% on ≥ 10 requests (unstable prefix, or a provider without
  a prompt cache — the open question on bonsai)
- > 150k prompt tokens per request (context not trimmed)

Hourly cron, both together:

```
0 * * * * cd ~/Documents/myProjects/llm_brain && ./target/release/brain usage snapshot >> usage.log 2>&1 && ./target/release/brain events ingest >> usage.log 2>&1
```

At the end of the week: `brain usage report`, `brain events report --days 7`,
`brain bench report`. Those three tables are the Phase 0 deliverable.

## Plan for the week (agreed on 2026-09-20)

1. You: aider from Docker and Claude Code from the host, both on the `dev`
   key, on real work. Switch tier by hand when the fast model struggles.
2. brain: hourly `usage snapshot` + `events ingest`; you glance at
   `events report` when something feels off.
3. Anomalies get a row in the [changelog](./changelog.md) with the cause
   found (model, tool, prompt shape) — that is the input for Phase 1's
   sanitizer and escalation rules.
4. First benchmark rows for both tools on `ago-0001`; more tasks mined as
   real fixes happen.
5. **No integration into agent-orchestrator** until the local setup has run
   for the week: it stays a read-only source of benchmark tasks.

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

Same task, from 260 s / 30k→8.7k tokens (`whole`) to 39 s / 12k→676
tokens (`diff`): the edit format was worth more than the model choice.
**bonsai works as the reasoning tier** in the architect/editor split with
the OpenRouter fallback — the configuration `brain setup aider` generates.

What the rows say, with the [direct probes](./models/bonsai-2-27b.md#measured-on-2026-09-20):
bonsai's single provider is 6× slower than deepseek-v4-flash, has no prompt
cache and rejects Claude Code's parallel requests; deepseek-v4-flash is
cheaper ($0.04/$0.08), has reasoning and tools, and solved the task with
aider. Claude Code on a non-Claude model fails for a different reason:
the model answers nothing after a tool result — the tool-loop reliability
risk from [Claude Code and aider](./architecture/claude-code-aider.md).

**Decided (2026-09-20)**: `fast` = `deepseek/deepseek-v4-flash`;
**bonsai moves to the `reasoning` tier** — it must work, but as the
architect/reasoning role, not as the daily driver. In aider's
architect/editor split bonsai proposes and deepseek-v4-flash applies the
edits, so its latency matters less and it never has to produce edits itself.
The aider+bonsai failure is being re-run with logs to find its cause.

## Exit criteria (from the roadmap)

- No daily limit ever hit: `usage report` never shows `EXHAUSTED`.
- A table of $/day per profile and $/task per tool from `usage report` and
  `bench report`.
- Malformed tool calls / protocol errors with Claude Code on non-Claude
  models: counted by hand this week, automated in Phase 1
  ([why they happen](./architecture/client-compatibility.md)).

## Known gaps, on purpose

- Per-request cost is not collected: `GET /key` reports USD per key only;
  tokens come from the tools' logs and the cost is estimated from tier
  prices. Phase 1's proxy reads `usage` from every response.
- `brain bench mine` (automatic task extraction from git history) is not
  written; the first task (`ago-0001`) was mined by hand.
- Claude Code's headless flags (`-p … --output-format json
  --permission-mode acceptEdits`) are unit-tested but not container-tested:
  `claude` is not installable via pip. Verified manually in step 7.
