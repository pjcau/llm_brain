---
title: Configuration reference
sidebar_position: 4
---

# Configuration reference

Everything needed to run `brain` on the laptop and point the clients at
the proxy. The server side is on [VPS deployment](./deploy-vps.md).
**No secrets here**: keys are always referenced by environment variable
name; the values live only in an uncommitted `.env` (mode 0600),
`deploy/server.local.env` or your shell.

## Install `brain` and use it from any folder

```bash
cd ~/Documents/myProjects/llm_brain
cargo install --path crates/brain --root ~/.local      # → ~/.local/bin/brain (already on PATH)
```

`brain` needs to find `config/` and its SQLite file. From inside the repo
it walks up to find them; from anywhere else it uses **`BRAIN_HOME`**:

```bash
# in .env (sourced by ~/.bashrc), or exported in the shell
BRAIN_HOME=/home/<you>/Documents/myProjects/llm_brain
```

With `BRAIN_HOME` set, `bench/tasks`, `bench/.cache` and `bench/.runs`
resolve inside the repo whatever the current directory, and `.env` is
loaded from there if the cwd has none. `--config DIR` and `--db FILE`
override.

| Symptom | Cause |
|---------|-------|
| `Command 'brain' not found` | not installed on PATH: run the `cargo install` line above (or call `target/release/brain`) |
| `no config/profiles.yaml found upwards … set BRAIN_HOME` | called from another folder without `BRAIN_HOME` |
| `OPENROUTER_KEY_DEV not set` / auth error | `.env` not sourced in this shell: `set -a; . $BRAIN_HOME/.env; set +a` in `~/.bashrc`, then open a new terminal |
| `px-claude` / `px-aider` missing | they live in `~/.bashrc`, which only interactive shells read; open a new terminal or `source ~/.bashrc` |

## Files in the repo

| File | Purpose | Secrets? |
|------|---------|----------|
| `config/profiles.yaml` | one profile per client: tier, daily hard limit, monthly soft cap, L2 cache policy (not built yet), optional `router` | no — the key is `OPENROUTER_KEY_<PROFILE>` in the env |
| `config/tiers.yaml` | `fast` / `reasoning` / `medium` / `agent` / `max` / `premium` → OpenRouter model, fallback, context, prices, output cap; the global `brain/auto` router | no |
| `.env.example` | the variable names to fill in | no (template) |
| `.env` | the actual keys, **git-ignored**, `chmod 600` | **yes** — never commit, never paste in chat |
| `deploy/server.local.env` | VPS IP, hostname, board credentials, `BRAIN_BASE_URL`, `BRAIN_DEV_KEY`; git-ignored | **yes** |
| `bench/tasks/*.yaml` | benchmark tasks ([format](./architecture/benchmark.md)) | no |

### `config/profiles.yaml`

```yaml
profiles:
  - name: dev
    description: Claude Code + aider on the laptop
    tier: fast
    daily_limit_usd: 5.0
    monthly_soft_usd: 60.0
    l2_cache: off
  - name: ago                # agent-orchestrator as a client (not integrated yet)
    tier: fast
    daily_limit_usd: 1.0
    monthly_soft_usd: 5.0
    l2_cache: off
  - name: benchmark          # benchmark runner, never at the expense of dev
    tier: fast
    daily_limit_usd: 0.5
    monthly_soft_usd: 5.0
    l2_cache: off
  - name: assistant          # local assistant app (chat, RAG)
    tier: fast
    daily_limit_usd: 0.5
    monthly_soft_usd: 15.0
    l2_cache: exact
    router:                  # its own brain/auto ladder (chat, not coding); replaces the global one
      model: typesafe/jev-1.13
      context: Message from a user to a personal chat assistant that can search the user's documents
      fallback: fast
      baseline: medium
      min_confidence: 0.5
      session_ttl_s: 7200
      ladder:
        - {tier: fast,   when: a greeting, a short factual question, a lookup or FAQ …}
        - {tier: medium, when: an explanation, a summary or comparison, drafting a text …}
        - {tier: agent,  when: a multi-step analysis, a long structured document, planning …}
  - name: car                # find-a-car (valuation, extraction)
    tier: fast
    daily_limit_usd: 0.5
    monthly_soft_usd: 15.0
    l2_cache: exact
```

`daily_limit_usd` is both the OpenRouter key's own limit (ring 1, pushed
with `brain upstream sync`) and the proxy's 100% line (ring 2);
[Budget](./architecture/budget.md). Every profile has a `description:`;
the excerpt shortens most of them to comments.

### `config/tiers.yaml`

```yaml
tiers:
  fast:                                   # daily driver, editor in aider, Claude Code's background model
    model: deepseek/deepseek-v4-flash
    fallback: qwen/qwen3.7-flash
    context: 1048576
    input_usd_per_m: 0.09
    output_usd_per_m: 0.18
    max_output_tokens: 16384              # policy cap; the provider's own max comes from the catalog
  reasoning:                              # architect in aider
    model: prism-ml/ternary-bonsai-2-27b
    fallback: deepseek/deepseek-v4-pro
    context: 262144
    input_usd_per_m: 0.075
    output_usd_per_m: 0.5
    max_output_tokens: 32768
  medium:                                 # rung between fast and agent
    model: z-ai/glm-5.3-flash
    fallback: deepseek/deepseek-v4-flash
    context: 1310720
    input_usd_per_m: 0.15
    output_usd_per_m: 0.50
    max_output_tokens: 32768
  agent:                                  # Claude Code's main model when pinned; brain/auto's fallback
    model: deepseek/deepseek-v4-pro       # `…:exacto` to sort providers by tool-call accuracy
    fallback: qwen/qwen3.7-plus
    context: 1048576
    input_usd_per_m: 0.96
    output_usd_per_m: 1.91
    max_output_tokens: 32768
  max:                                    # top rung
    model: z-ai/glm-5.3
    fallback: deepseek/deepseek-v4-pro
    context: 1310720
    input_usd_per_m: 0.84
    output_usd_per_m: 2.64
    max_output_tokens: 32768
  premium:
    model: null

# brain/auto — docs/architecture/auto-routing.md
router:                                   # global ladder; a profile's own `router` replaces it
  model: typesafe/jev-1.13
  fallback: agent                         # decision model down or unsure
  baseline: agent                         # the board's "would have cost on" reference
  min_confidence: 0.5
  session_ttl_s: 7200
  ladder:                                 # light → heavy; `when` is what the decision model reads
    - {tier: fast,   when: a trivial edit in one place with no reasoning needed …}
    - {tier: medium, when: a focused change in one or two files …}
    - {tier: agent,  when: a change across several files, a bug to debug, tests to run and fix …}
    - {tier: max,    when: a large refactor, a design decision, a subtle concurrency or security bug …}
```

`router.context` (prefixed to what the decision model reads) defaults to
"Task given to an autonomous coding agent"; the `assistant` profile sets
its own. Prices are the OpenRouter catalog's, re-checked 2026-09-22 (~2×
since 2026-09-20); the proxy bills from OpenRouter's `cost` when present
and only estimates from these numbers otherwise. A model id may carry an
OpenRouter routing suffix (`:exacto`, `:nitro`, `:floor`); the catalog
lookup strips it ([Provider routing](./architecture/api-layer.md#provider-routing-exacto)).
The server reads both files at start: after changing them, copy them to
the VPS and restart `brain` ([update](./deploy-vps.md#update)).

The proxy caps `max_tokens` per request to the smaller of the provider's
maximum (OpenRouter catalog, refreshed hourly) and the tier's
`max_output_tokens`: the catalog allows 384 000 output tokens for
deepseek-v4-flash, which is how one request once produced 89 000 of them.
`brain models` prints those facts per tier.

### `.env` (from `.env.example`)

```bash
# only while running `brain upstream provision|sync|list`; delete the line afterwards
OPENROUTER_MANAGEMENT_KEY=
# one per profile, created by `brain upstream provision`
OPENROUTER_KEY_DEV=
OPENROUTER_KEY_AGO=
OPENROUTER_KEY_BENCHMARK=
OPENROUTER_KEY_ASSISTANT=
OPENROUTER_KEY_CAR=
# SQLite (usage snapshots, events, requests, benchmark runs); default ./brain.db
BRAIN_DB=data/brain.db
# repo dir, so `brain` works from any folder
BRAIN_HOME=/home/<you>/Documents/myProjects/llm_brain
# optional
# BRAIN_DATA=             tools' logs and generated aider files (default ~/.local/share/llm_brain)
# BRAIN_AIDER_ROOTS=      repos scanned for .aider.chat.history.md, `:`-separated (default ~/Documents/myProjects)
# BRAIN_CLAUDE_PROJECTS=  extra Claude Code projects dir for `events ingest`
# BRAIN_BENCH_KEY=        client key of the `benchmark` profile, for `bench run --via-proxy`
```

## Through the proxy

The daily setup. `BRAIN_BASE_URL` and `BRAIN_DEV_KEY` (a `dev` client key,
[issued on the server](./phase-1.md#issue-a-key-on-the-server-admin-only))
live in `deploy/server.local.env`, sourced by `~/.bashrc`. `brain setup
claude-code --proxy` and `brain setup aider --proxy` print:

```bash
px-claude() {
  ANTHROPIC_BASE_URL="$BRAIN_BASE_URL" ANTHROPIC_AUTH_TOKEN="$BRAIN_DEV_KEY" \
  ANTHROPIC_MODEL=brain/auto ANTHROPIC_DEFAULT_HAIKU_MODEL=brain/fast \
  CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS=1 CLAUDE_CODE_DISABLE_ADAPTIVE_THINKING=1 \
  claude "$@"
}
# pin a tier instead of the per-session decision: ANTHROPIC_MODEL=brain/agent px-claude

px-aider() {
  OPENAI_API_BASE="$BRAIN_BASE_URL/v1" OPENAI_API_KEY="$BRAIN_DEV_KEY" \
  aider --architect --model openai/brain/reasoning --editor-model openai/brain/fast \
    --model-metadata-file $HOME/.local/share/llm_brain/aider-model-metadata.json \
    --model-settings-file $HOME/.local/share/llm_brain/aider-model-settings.yml \
    --llm-history-file $HOME/.local/share/llm_brain/aider-llm.history \
    --yes-always --auto-accept-architect --auto-commits --show-diffs --restore-chat-history \
    --no-suggest-shell-commands --no-check-update --no-show-model-warnings --notifications \
    --read $HOME/.local/share/llm_brain/aider-conventions.md \
    $([ -f CLAUDE.md ] && echo --read CLAUDE.md) "$@"
}
```

`brain setup aider --proxy` also prints the settings file and the
conventions file to save under `~/.local/share/llm_brain/`. `brain/*` are
aliases resolved on the server; the proxy adds the `models[]` fallback
itself. Plain `claude` keeps the subscription: the function sets the
variables for that one invocation only. Putting the same variables in
`~/.claude/settings.json` → `"env"` would send *every* session through the
proxy.

What the aider flags do ("make it behave like Claude Code"):

| Flag | Effect |
|------|--------|
| `--yes-always`, `--auto-accept-architect` | no confirmations; in architect mode the proposal is applied without the "Edit the files?" prompt (the reason edits "were announced but never appeared") |
| `--auto-commits`, `--show-diffs` | every applied edit is committed with a conventional message, and the diff is printed |
| `--restore-chat-history` | resumes the previous conversation of **that repo** (`.aider.chat.history.md`); `brain events ingest` scans the repos under `BRAIN_AIDER_ROOTS` to keep the board fed |
| `--read …/aider-conventions.md`, `--read CLAUDE.md` | standing rules every session (small steps, tests with the change, conventional commits, no secrets, plain-text replies — the last one stops bonsai from emitting fake `<tool_call>` markup as architect), plus the repo's own rules |
| `--no-suggest-shell-commands` | with `--yes-always` on, aider must not auto-run commands the model suggests |

aider cannot accept typing while it is generating; that difference from
Claude Code stays.

### Direct to OpenRouter (Phase 0 path)

Still printed, used by the benchmark and when the proxy is down; the
profile's `OPENROUTER_KEY_*` pays:

| Command | Prints |
|---------|--------|
| `brain setup claude-code` | env block for the host's `claude` (`ANTHROPIC_BASE_URL=https://openrouter.ai/api`, model = `fast`) |
| `brain setup aider` | env block (architect = `reasoning`, editor = `fast`) + model metadata + settings file |
| `brain setup aider --docker llm-brain-aider-test:latest` | `or-aider`: aider from the image, current repo mounted, logs under `BRAIN_DATA` |

Claude Code in Docker (`setup claude-code --docker`, `docker/claude-test.Dockerfile`)
was abandoned: the container hangs on first start in a fresh HOME. The
flag still exists; nothing uses it.

## aider model fallbacks and edit formats (`.aider.model.settings.yml`)

`brain setup aider` prints it; saved at
`~/.local/share/llm_brain/aider-model-settings.yml` and passed with
`--model-settings-file`. Per model, `extra_params.extra_body.models` lists
the tier's primary and fallback, and OpenRouter switches on provider
errors, rate limits or downtime, billing the model actually used (bonsai
has a single provider). The same file sets the **edit format**: aider
gives unknown models `whole` (full-file rewrites: 101k tokens in one turn
and a failed edit in [Phase 0](./phase-0.md)); the fast tier, as editor,
gets `diff` / `editor-diff`. In architect mode the architect's entry
decides the editor format, so `reasoning` declares `editor-diff` too.
aider matches by name, so the `brain/*` aliases (proxy path) get entries
as well, without the chain (the proxy adds it):

```yaml
- name: openai/deepseek/deepseek-v4-flash
  edit_format: diff
  editor_edit_format: editor-diff
  use_repo_map: true
  extra_params:
    extra_body:
      models: ["deepseek/deepseek-v4-flash", "qwen/qwen3.7-flash"]
- name: openai/brain/fast
  edit_format: diff
  editor_edit_format: editor-diff
  use_repo_map: true
- name: openai/prism-ml/ternary-bonsai-2-27b
  editor_edit_format: editor-diff
  use_repo_map: true
  extra_params:
    extra_body:
      models: ["prism-ml/ternary-bonsai-2-27b", "deepseek/deepseek-v4-pro"]
- name: openai/brain/reasoning
  editor_edit_format: editor-diff
  use_repo_map: true
```

## aider model metadata

`brain setup aider` also prints `aider-model-metadata.json` (save it in
`~/.local/share/llm_brain/`) with context and per-token prices from
`tiers.yaml` for the `fast` and `reasoning` models, so aider shows costs.
It has entries for the concrete model ids only, not for `brain/*`.

## The board (`brain serve`, in Docker)

On the VPS the board is part of `brain serve` ([sections](./phase-1.md#the-board)).
A local copy:

```bash
docker compose -f deploy/docker-compose.board.yml up -d --build   # → http://127.0.0.1:8090/
docker compose -f deploy/docker-compose.board.yml down
```

One container (`llm-brain-board`, image from `docker/brain.Dockerfile`)
runs `brain serve`. Every 10 minutes (`--refresh 600`) it takes the usage
snapshots and ingests the tools' logs itself, so no cron is needed while
it runs. Mounts: `config/` (ro) and `data/` (the shared SQLite) from the
repo, `~/.local/share/llm_brain` and `~/.claude/projects` read-only; keys
from `.env` via `env_file`. Bound to `127.0.0.1` only. On macOS OrbStack
works unchanged. Without Docker: `brain serve --bind 127.0.0.1:8080`.

## Tool logs (`brain events`)

What the tools leave on disk, for traffic that does not go through the
proxy (and for the Phase 0 history):

| Source | What it gives | Default location |
|--------|---------------|------------------|
| aider chat history | one `Tokens: N sent, M received` line per round trip, the model, and every notice (`litellm.*Error`, `Retrying…`, edit-format failures) | `$BRAIN_DATA/aider-chat.md`, and `.aider.chat.history.md` in the repos under `BRAIN_AIDER_ROOTS` |
| Claude Code transcripts | per assistant message: model and `usage` (input, cache read/write, output); API errors | `~/.claude/projects/*/*.jsonl`, plus `BRAIN_CLAUDE_PROJECTS` |

`brain events ingest` reads only the new bytes of each file (offsets in
SQLite); `brain events report` prints day × tool × model with anomalies:
error rate > 10% on ≥ 5 requests · ≥ 3 retries in a day · cache hit < 30%
on ≥ 10 requests · > 150k prompt tokens per request. Subscription traffic
is never an anomaly. Without the board, an hourly cron does the same:

```
0 * * * * cd ~/Documents/myProjects/llm_brain && ./target/release/brain usage snapshot >> usage.log 2>&1 && ./target/release/brain events ingest >> usage.log 2>&1
```

## Docker images

```bash
docker build -f docker/aider-test.Dockerfile    -t llm-brain-aider-test .     # or-aider, bench --docker, CI container test
docker build -f docker/opencode-test.Dockerfile -t llm-brain-opencode-test .  # OpenCode for the benchmark / local trials
docker build -f docker/brain.Dockerfile         -t llm-brain:local .          # the board (compose builds it)
```

## `brain` commands

| Command | Needs |
|---------|-------|
| `brain upstream provision [--force] [--only dev,car]` — one OpenRouter key per profile with its daily limit | `OPENROUTER_MANAGEMENT_KEY` |
| `brain upstream sync [--only dev]` — PATCH the daily limit of existing keys to match `profiles.yaml`, secrets unchanged | `OPENROUTER_MANAGEMENT_KEY` |
| `brain upstream list` | `OPENROUTER_MANAGEMENT_KEY` |
| `brain keys create --profile P --name N [--expires DATE] [--ip CIDRs]` · `keys list [--profile P]` · `keys revoke --profile P --name N` | the SQLite file (run on the server) |
| `brain usage snapshot` · `usage report [--days 7]` | `OPENROUTER_KEY_*` |
| `brain models` — tier models with context, provider max output, effective cap, prices from the OpenRouter catalog | network |
| `brain setup claude-code [--profile dev] [--proxy] [--docker IMAGE]` | — |
| `brain setup aider [--profile dev] [--proxy] [--docker IMAGE]` | — |
| `brain serve [--bind 127.0.0.1:8080] [--refresh 600] [--days 7]` — proxy (`/v1/*`, key auth) + board | `OPENROUTER_KEY_*` (upstream) |
| `brain events ingest [--aider-chat FILE] [--claude-projects DIR]` · `events report [--days 7]` | the tools' logs |
| `brain bench run --tool aider\|claude\|opencode [--tier fast] [--model ID] [--only id,…] [--profile benchmark] [--docker IMAGE] [--via-proxy URL] [--tasks DIR] [--cache DIR] [--logs DIR]` — `--tier reasoning` runs aider in architect mode with `fast` as editor; with `--via-proxy` the model defaults to `brain/<tier>` | `OPENROUTER_KEY_BENCHMARK`, or `BRAIN_BENCH_KEY` with `--via-proxy` |
| `brain bench report [--run ID]` | — |

Global flags: `--config DIR`, `--db FILE`. Env: `BRAIN_HOME`, `BRAIN_DB`,
`BRAIN_DATA`, `BRAIN_AIDER_ROOTS`, `BRAIN_CLAUDE_PROJECTS`, `BRAIN_BENCH_KEY`.
