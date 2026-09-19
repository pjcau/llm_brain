# llm_brain

One LLM backend for every app and CLI: an OpenAI/Anthropic-compatible
reverse proxy with per-profile budgets, keys, caching and a nightly
benchmark. Design docs live in `docs/` and are published at
https://pjcau.github.io/llm_brain/ — read `docs/index.md` first, then
`docs/decisions.md`.

## Rules

- **All code is Rust** (Cargo workspace; `tools/` for helpers, `crates/brain` for the service). No Python, no shell scripts beyond `deploy/`.
- Every design iteration is recorded: add a row to `docs/changelog.md`, update or add the page, keep `docs/index.md` under 2000 words.
- Diagrams: edit `diagrams/*.mmd` only, then `npm run sync-diagrams` (it runs `cargo run -p sync-diagrams`).
- Code and tests go together: every module has unit tests; HTTP is mocked with `wiremock`; anything that needs a real tool runs in Docker via `testcontainers` (feature `docker-tests`, see `crates/brain/tests/`). CI runs fmt, clippy `-D warnings`, tests and the docker tests.
- Never commit secrets: config references env var names, `.env` is ignored.
- Docs site: `npm start` serves on port 3010 (3000 is taken by another project).

## claude-kit (submodule `.claude-kit/`)

Skills, agents and hooks from https://github.com/pjcau/claude-kit. Use:

- @.claude-kit/skills/commit.md
- @.claude-kit/skills/doc.md
- @.claude-kit/skills/debug.md
- @.claude-kit/skills/refactor.md
- @.claude-kit/skills/create-pr.md
- @.claude-kit/skills/review-pr.md
- Agents: `.claude-kit/agents/architect.md`, `code-reviewer.md`, `test-runner.md`, `dependency-checker.md`, `software-engineering/`

After `git clone`, run `git submodule update --init` to fetch it.
