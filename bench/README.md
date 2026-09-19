# Benchmark suite

Real bugs from your repos, verified by the repos' own tests. Format and
protocol: https://pjcau.github.io/llm_brain/architecture/benchmark

```bash
# Phase 0: aider vs Claude Code on the fast tier, paid by the `benchmark` key
brain bench run --tool aider  --tier fast
brain bench run --tool claude --tier fast
brain bench report
```

Each task YAML: `commit_before` (what the tool sees), `setup` (deps),
`prompt`, `test_files_from_fix` (tests taken from `commit_fix` *after* the
tool ran), `verify` (exit 0 = pass), `timeout_s`, `max_cost_usd`.
The repo cache lives in `bench/.cache/` (ignored).
