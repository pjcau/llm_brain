# Benchmark suite

Real bugs from your repos, verified by the repos' own tests. Format and
protocol: https://pjcau.github.io/llm_brain/architecture/benchmark

```bash
# through the proxy, the way the clients use it (BRAIN_BENCH_KEY = a `benchmark` client key)
brain bench run --tool claude   --tier agent --via-proxy https://brain.<host>
brain bench run --tool opencode --tier agent --via-proxy https://brain.<host>
# direct to OpenRouter with the `benchmark` profile's key (OPENROUTER_KEY_BENCHMARK)
brain bench run --tool aider --tier reasoning --docker llm-brain-aider-test:latest   # architect + fast editor
brain bench report
```

Each task YAML: `commit_before` (what the tool sees), `setup` (deps),
`prompt`, `test_files_from_fix` (tests taken from `commit_fix` *after* the
tool ran), `verify` (exit 0 = pass), `timeout_s`, `max_cost_usd`.
The repo cache lives in `bench/.cache/`, each task's tool output in
`bench/.runs/` (both ignored). Tasks are mined by hand for now
(`bench/tasks/ago-0001-*.yaml`); there is no nightly run yet.
