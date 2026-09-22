---
name: sync-docs
description: Bring docs/ back in sync with the code changed in this session and clean what went stale — changelog row, affected pages, diagrams, site build. Incremental by design; runs in seconds and no-ops when nothing changed.
allowed-tools: Bash, Read, Edit, Write, Grep, Glob
user-invocable: true
---

# sync-docs — incremental documentation pass

Keeps `docs/` true to the repo after a change. This is the **delta** pass, run
at the end of a working turn. For a full rewrite of every page against the whole
codebase use `.claude-kit/skills/doc/SKILL.md` instead.

## Phase 0 — is there anything to do?

```bash
git status --short
git diff --stat HEAD -- crates config diagrams tools deploy package.json
```

**If nothing changed under `crates/`, `config/`, `diagrams/`, `tools/`, `deploy/`,
stop here and say "docs already in sync".** A turn that only read code, answered a
question or ran a query changes no documentation. Do not invent an iteration.

Design decisions taken in conversation *do* count as a change even with no diff:
if this turn settled a design question (a number, a trade-off, an order of work),
it needs a changelog row.

## Phase 1 — changelog

`docs/changelog.md` is one row per design iteration, newest at the bottom of the
head block (read the last rows for the house voice: dense, factual, numbers in
the text, no marketing).

- Next `#` = last row's number + 1, date = today (absolute, never "yesterday").
- One row per iteration, not one per file touched.
- Say **what changed and what it is worth** — a measured number beats an adjective.
- If a row for this iteration already exists, extend it instead of adding a second.

## Phase 2 — the pages

Only the pages the change actually touches:

- `docs/architecture/*.md` — one page per subsystem; the change belongs where the
  subsystem already lives (routing → `auto-routing.md`, caches → `cache.md` and
  `cache-logic.md`, budgets → `budget.md`, keys → `secrets.md`).
- `docs/configuration.md` — any new key in `config/*.yaml`, env var **name**, CLI
  flag or command.
- `docs/index.md` — only if the entry map changed. Hard limit: `wc -w docs/index.md`
  must stay **under 2000 words**; if the change does not belong in the map, link it
  from the right page instead.

Rules:

- Every figure must come from the repo or a measurement made in this session
  (`config/tiers.yaml`, the catalog, the board, a test). **Never carry a number over
  from memory** — re-read it before writing it.
- No duplicated prose across pages: state it once, link the rest.
- Pages are written in English, whatever language the conversation used.
- Config references env var **names** only. No keys, no tokens, no passwords, no
  `deploy/server.local.env` values, no board credentials.

## Phase 3 — clean what went stale

Grep the docs for facts the change invalidated, and fix them where they are wrong:

```bash
grep -rn "<old model id>\|<old price>\|<old flag>\|<old port>" docs/
```

Typical rot in this repo: prices and model ids that moved in `config/tiers.yaml`,
renamed CLI commands, ports, profile names, a "planned" feature that now exists
(move it from roadmap to the page that describes it), dead relative links.

## Phase 4 — diagrams

Never edit the mermaid embedded in a `.md`. Edit `diagrams/*.mmd`, then:

```bash
npm run sync-diagrams
```

## Phase 5 — verify

```bash
npm run build; echo "build exit=$?"
```

**Judge the exit code, not the log text.** Non-zero → fix and rebuild before
reporting done.

## Report

Two or three lines: the changelog row added, the pages touched, the build result.
If the pass was a no-op, one line is enough.
