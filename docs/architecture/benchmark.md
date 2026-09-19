---
title: Nightly benchmark
---

# Nightly benchmark: improve quality and cost every day

Goal: every day, know **what the current tier configuration costs and
what it's worth**, and try **one** alternative candidate without risking
the working budget.

## What already exists in agent-orchestrator

| Module | Use |
|--------|-----|
| `core/evaluator.py` | `EvalCase` (prompt, expected, rubric), `EvalSuite`, `RubricEvaluator`, `LLMJudge`, `JsonDataset` |
| `core/verifiers/` | verifiers: this is where the "run the tests" verifier goes |
| `dashboard/evals_routes.py` | `POST /api/evals/run`, `GET /api/evals/runs`, `GET /api/evals/compare` (delta between two runs) |
| `core/benchmark.py` | latency, throughput, cost per provider |
| `core/usage.py` | tokens and cost per call |

Missing: report persistence (in-memory today, capped at 50), the
scheduler, the test verifier, the quality/cost chart.

## The suite: it doesn't exist, it has to be built

There is no benchmark repo: it gets built in `bench/` inside llm_brain,
starting from 10 tasks and growing with real fixes.

### Task format (`bench/tasks/<id>.yaml`)

```yaml
id: ago-0001
repo: https://github.com/pjcau/agent-orchestrator
commit_before: <sha of the commit before the fix>
commit_fix: <sha of the fix, human reference only>
prompt: |
  The test tests/core/test_usage.py::test_daily_budget fails: ...
verify: pytest tests/core/test_usage.py -q
timeout_s: 600
max_cost_usd: 0.30
tags: [python, budget, small]
```

### Where the tasks come from

1. **agent-orchestrator**: 1692 tests, Python, rich git history. Extract
   the commits with "fix" in the message that also touch a test:
   `commit_before` = parent, `verify` = that test. First source, and the
   easiest to automate (`brain bench mine`).
2. **Real fixes made during work** with Claude Code/aider behind llm_brain:
   every test-verified fix becomes a task. This is the channel that grows
   the suite for free.
3. **Small synthetic tasks** (5–6) to cover missing types: refactor with
   existing tests, adding a FastAPI endpoint, parsing.
4. Later, "chat" tasks with a rubric for `assistant` and extraction tasks
   for find-a-car (listing input → expected JSON: verifiable without an
   LLM).

### Protocol detail: the fix's tests arrive after the tool

The tests that prove a fix usually land **in the fix commit**. So a task
lists them in `test_files_from_fix`; the runner checks out the worktree at
`commit_before` (the model cannot see them), runs `setup` and the tool,
**then** `git checkout <commit_fix> -- <those files>` and `verify`. Same
protocol as SWE-bench's test patch.

### The runner (`brain bench run`)

For each task: clean worktree at `commit_before` → launch the tool
headless (aider: `aider --message "$prompt" --yes`; Claude Code:
`claude -p "$prompt" --output-format json`) with env pointed at llm_brain
and the `brain_bench_…` key → run `verify` → collect the run's usage from
llm_brain (header `X-Brain-Run: <run_id>` on every request) → write one
row per (task, tool, model, run).

Each row records: **pass**, cost $, tokens in/out, `cached_tokens`,
malformed tool calls, turns, time to result.

## Comparing tools, not just models

Same task, same model, two tools: **Claude Code vs aider**. Extra
metrics: € per solved task, turns, time to result, tokens per turn. That
is the answer to "who burns less and gets there first", and it's in the
same suite.

## The daily cycle

1. Nightly cron → `POST /api/evals/run` with the **current** tier config
   (the day's baseline).
2. Same run with **a single** changed candidate: an alternative model for
   `fast`, or a parameter (`reasoning_effort`, aider's edit format,
   `max_tokens`).
3. `GET /api/evals/compare` → quality delta and cost delta.
4. **Promotion rule**: the candidate replaces the baseline if pass@1 ≥
   baseline − 2 points **and** cost ≤ baseline − 10%. Otherwise the next
   candidate waits in the queue.
5. Dashboard: quality/cost frontier chart over time, baseline
   highlighted.

## Guardrails

- Dedicated OpenRouter key `benchmark` with a daily `limit`
  ([Budget](./budget.md)): the benchmark can never eat the working
  budget.
- One candidate per night: comparisons stay readable and the cost
  predictable (~2× the suite).
- The suite grows only by adding real bugs fixed during work: every real
  fix becomes a future test.
