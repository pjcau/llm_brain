---
title: Compatibility with CLIs and apps
---

# Is it compatible? Claude Code, aider, app SDKs, agent-orchestrator

Short answer: **yes for all of them**, because every client uses the
official OpenAI or Anthropic SDKs, which support `base_url` + key. The
points of attention are in Claude Code, and what a gateway must do is
officially documented (verified on 2026-09-19 against Claude Code's
"gateway compatibility" guide).

## Matrix

| Client | Base URL | Key → header | `user` per end user | Run correlation | 401 | 429 |
|--------|----------|--------------|---------------------|-----------------|-----|-----|
| **Claude Code** | `ANTHROPIC_BASE_URL` | `ANTHROPIC_AUTH_TOKEN` → `Authorization: Bearer`; `ANTHROPIC_API_KEY` → `x-api-key`; `apiKeyHelper` → both | not needed (`dev` profile, one user) | free: headers `x-claude-code-session-id`, `x-claude-code-agent-id`; extras via `ANTHROPIC_CUSTOM_HEADERS` | shows the error | automatic retries; honours `retry-after` (see below) |
| **aider** | `OPENAI_API_BASE` | `OPENAI_API_KEY` → `Bearer` (via internal LiteLLM) | not needed | `extra_params`/`extra_headers` in model settings (to verify in Phase 0) | error | retry with backoff (LiteLLM) |
| **Apps (OpenAI SDK, Python/Node)** | `base_url` / `OPENAI_BASE_URL` | `api_key` → `Bearer` | `user="u_…"` native in `chat.completions.create` | `default_headers={"X-Brain-Run": …}` | `AuthenticationError`, no retry | `RateLimitError` after 2 retries, honours `retry-after` |
| **Apps (Anthropic SDK)** | `base_url` | `api_key` → `x-api-key` | `metadata={"user_id": …}` | `default_headers` | same | same |
| **agent-orchestrator** | `OPENAI_BASE_URL` (read by the SDK, no code change: today it builds `AsyncOpenAI(api_key=…)`) | `OPENAI_API_KEY` = `brain_ago_…` | optional | — | same | same |

llm_brain accepts **both** headers and maps `user` and `metadata.user_id`
onto the same field. Everything else is configuration.

## Claude Code: the gateway contract (from the official docs)

### What llm_brain must expose
- `POST /v1/messages` (arrives as `/v1/messages?beta=true`: match the
  path), **SSE streaming without buffering**, forwarding `ping`s too:
  Claude Code aborts a stream silent for 300 s.
- `POST /v1/messages/count_tokens`: optional; without it `/context`
  estimates by characters.
- `HEAD /api/hello`: startup probe, can be rejected.
- `GET /v1/models?limit=1000` only if the dev enables
  `CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY=1`; response in < 3 s,
  **no redirects** (even http→https fails it and the key must not
  travel). Only ids containing `claude` or `anthropic` enter the picker.

### Headers and body
- Forward `anthropic-version` and `anthropic-beta` **unchanged**; treat
  `anthropic-*` headers and body fields as **open lists** (every release
  adds some).
- **`system` array intact, attribution block first**: since v2.1.181 it
  is stable for the whole conversation, so the prompt cache works; **the
  layer prepends nothing for the `dev` profile** (the "server-side system
  prompt" rule applies to apps only).
- `cache_control` forwarded where it is, never convert `system` to a
  string.
- Consumable (not to forward): `x-claude-code-session-id`,
  `x-claude-code-agent-id`, `x-claude-code-parent-agent-id` → usage per
  session and per subagent **without parsing the body**.

### Responses
- `retry-after` in **integer seconds**: ≤ 60 → Claude Code waits and
  retries; **> 60 → it stops and shows the error at once**. So: throttle
  → `retry-after: 20`; **day's budget exhausted → `retry-after: 3600` +
  `x-should-retry: false`**, so the CLI clearly says "budget exhausted"
  instead of retrying.
- Error bodies **not wrapped**: Claude Code's recovery logic reads the
  upstream error text.

### The real risk: non-Claude models behind OpenRouter
The documentation states explicitly that Anthropic **does not support**
routing Claude Code to non-Claude models through a gateway. It works in
practice (that's what claude-code-router does), but Claude Code sends
fields a non-Claude model rejects with `400`:

| Field / header | Symptom | Remedy |
|----------------|---------|--------|
| `thinking: {"type":"adaptive"}` (also on unknown aliases) | `400` on `thinking`/`adaptive` | client-side `CLAUDE_CODE_DISABLE_ADAPTIVE_THINKING=1`, or llm_brain strips/rewrites it |
| `context_management` + beta header | `400 Extra inputs are not permitted` | `CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS=1` |
| beta tool fields (`strict`, `defer_loading`) | `400` on the tool schema | same |
| `output_config` (effort, structured output) | `400` | same, or strip in llm_brain |

So for the `dev` profile llm_brain is not a translator but has a
**sanitizer**: it removes the fields the target model doesn't accept. A
few lines, but maintained release after release: that's the true cost of
Claude Code on cheap models. To measure in Phase 0 how much each release
breaks.

### Models: aliases and mapping
Claude Code sends `claude-*` ids (main) and uses a separate model for
background tasks. Recommended client-side configuration:

```
ANTHROPIC_BASE_URL=https://brain.example/
ANTHROPIC_AUTH_TOKEN=brain_dev_…
ANTHROPIC_MODEL=brain/fast
ANTHROPIC_DEFAULT_HAIKU_MODEL=brain/fast      # background tasks
CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS=1
CLAUDE_CODE_DISABLE_ADAPTIVE_THINKING=1
```

`/model brain/reasoning` switches tier by hand. For an unknown alias Claude
Code assumes a 200K context: correct it with `modelOverrides` in the
settings if the model behind has less.

### Subscription
With `ANTHROPIC_AUTH_TOKEN`/`ANTHROPIC_API_KEY`/`apiKeyHelper` active,
**the claude.ai subscription is not used**: exactly the goal. With only
`ANTHROPIC_BASE_URL` and a saved claude.ai login, traffic passes through
the gateway but consumes the subscription.

### Rotation without restarts
`apiKeyHelper` in `settings.json`: a script that prints the key (e.g.
`pass show brain/dev`). Claude Code re-reads it and sends it in both
headers. Rotation = change the entry in `pass`.

## aider

```
OPENAI_API_BASE=https://brain.example/v1
OPENAI_API_KEY=brain_dev_…
aider --model openai/brain/reasoning --editor-model openai/brain/fast
```

You need `.aider.model.metadata.json` with the aliases' context and
prices (otherwise aider misestimates costs) and a Phase 0 test of the best
edit format for the `fast` model.

## The apps

No custom SDK: `OpenAI(base_url=…, api_key=…)` and
`client.chat.completions.create(model="brain/fast", user=user_id, …)`.
The profile (tier, budget, cache, versioned system prompt) is decided by
the key. Error handling as in [Authentication flow](./auth-flow.md).
