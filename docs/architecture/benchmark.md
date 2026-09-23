---
title: Nightly benchmark
---

# Benchmark: what a tier configuration costs and what it's worth

Goal: know **what the current tier configuration costs and what it's
worth**, compare tools and models on real tasks, and eventually try
**one** alternative candidate every night without risking the working
budget.

| Piece | Status |
|-------|--------|
| Task format, runner, `bench_runs` table, `brain bench run\|report` | **built** (`crates/brain/src/bench/`) |
| Tools: `aider`, `claude` (Claude Code), `opencode` | **built** |
| Direct to OpenRouter or `--via-proxy <url>` | **built** |
| Suite | **1 task** (`ago-0001`) |
| Task mining from git history, nightly cron, candidate promotion, frontier chart | **not built yet** |

## Task format (`bench/tasks/<id>.yaml`)

```yaml
id: ago-0001
repo: https://github.com/pjcau/agent-orchestrator
commit_before: ca581a25…          # what the tool sees
commit_fix: 9cd05e29…             # the human fix
prompt: |
  In src/agent_orchestrator/core/agent.py, mid-run context compaction drops …
setup: python3 -m venv .venv && .venv/bin/pip install -q -e '.[dev]'
test_files_from_fix: [tests/test_context_benchmark.py, evals/context_benchmark.py]
verify: .venv/bin/pytest tests/test_context_benchmark.py -q -k keep_head
timeout_s: 1800
max_cost_usd: 0.30
tags: [python, config-default, small]
```

**The fix's tests arrive after the tool.** The tests that prove a fix
usually land in the fix commit, so the runner checks out a worktree at
`commit_before` (the model cannot see them), runs `setup` and the tool,
**then** `git checkout <commit_fix> -- <test_files_from_fix>` and
`verify` (exit 0 = pass). Same protocol as SWE-bench's test patch.

## The runner (`brain bench run`)

```bash
brain bench run --tool aider|claude|opencode [--tier fast] [--model <id>] [--only ago-0001] \
                [--docker <image>] [--via-proxy https://brain.<host>]
brain bench report [--run <id>]
```

For each task, sequentially: clean worktree at `commit_before` → `setup`
→ the tool headless (aider `--message … --yes-always`, architect/editor
split on the `reasoning` tier as in `brain setup aider`; `claude -p … --output-format json
--permission-mode acceptEdits`; `opencode run --format json`) → fix tests
→ `verify` → one row in `bench_runs` (run, task, tool, tier, model, pass,
cost, seconds, exit code, notes). Tool stdout/stderr go to
`bench/.runs/<run>/<task>.{out,err}`.

- **Cost** (direct runs) = delta of the `benchmark` OpenRouter key's usage
  (`GET /key`) before and after the task, flagged in notes if above
  `max_cost_usd`. Via the proxy the row has no cost: it is in the proxy's
  `requests` (board, `benchmark` profile).
- **Direct** runs use `OPENROUTER_KEY_BENCHMARK`; **`--via-proxy`** uses
  `BRAIN_BENCH_KEY` (a client key of the `benchmark` profile) and
  exercises the proxy the way the clients use it (`--model` may be
  `brain/<tier>`).
- `--docker <image>` runs a tool that isn't installed on the host; the
  key is passed as `-e NAME`, never on the command line.

Results so far (Claude Code vs OpenCode vs aider on `ago-0001`, `:exacto`
A/B): [Phase 1 runbook](../phase-1.md#agents-compared-through-the-proxy).

## Where tasks come from

1. **agent-orchestrator** history: commits with "fix" in the message that
   also touch a test → `commit_before` = parent, `verify` = that test.
   `ago-0001` was mined by hand; a `brain bench mine` command is not built.
2. **Real fixes made during work** through llm_brain: every test-verified
   fix becomes a task. The channel that grows the suite for free.
3. **Small synthetic tasks** for missing types (refactor with existing
   tests, parsing).
4. Later, extraction tasks for find-a-car (listing input → expected JSON,
   verifiable without an LLM) and rubric tasks for `assistant`.

## Comparing tools, not just models

Same task, same model, different tools: $ per solved task, turns, time to
result. That answers "who burns less and gets there first" in the same
suite; the first answer is in the Phase 1 table.

## The nightly cycle (not built yet)

1. Nightly cron → `brain bench run` with the **current** tier config (the
   day's baseline).
2. Same run with **a single** changed candidate: an alternative model for
   one tier, or a parameter (`reasoning_effort`, aider's edit format,
   `max_tokens`).
3. **Promotion rule**: the candidate replaces the baseline if pass@1 ≥
   baseline − 2 points **and** cost ≤ baseline − 10%. Otherwise the next
   candidate waits.
4. Board: quality/cost frontier over time, baseline highlighted.

It needs a suite of ~10 tasks first: with one task, a pass/fail flip is
noise (two runs of the same model already differ 72 s vs 345 s).

## Guardrails

- Dedicated `benchmark` profile and OpenRouter key with a daily limit
  (0.50 $/day, [Budget](./budget.md)): the benchmark can never eat the
  working budget.
- Tasks run sequentially: costs are measured per key, so parallel runs
  would mix them.
- One candidate per night (when built): comparisons stay readable and the
  cost predictable (~2× the suite).
