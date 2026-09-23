---
title: Compatibility with CLIs and apps
---

# Is it compatible? Claude Code, aider, OpenCode, app SDKs

**Yes for all of them**, because every client uses the official OpenAI or
Anthropic SDKs (or a CLI built on them), which support `base_url` + key.
The points of attention are in Claude Code; what a gateway must do is
documented in Claude Code's "gateway compatibility" guide (checked
2026-09-19) and implemented in `proxy/mod.rs` and `proxy/sanitize.rs`.

## Matrix

| Client | Base URL | Key → header | End user | Session | 401 / 429 |
|--------|----------|--------------|----------|---------|-----------|
| **Claude Code** (`px-claude`) | `ANTHROPIC_BASE_URL` | `ANTHROPIC_AUTH_TOKEN` → `Authorization: Bearer`; `ANTHROPIC_API_KEY` → `x-api-key`; `apiKeyHelper` → both | from `metadata.user_id`, reduced to `cc:<session id>` | `x-claude-code-session-id` | shows the error; retries only if `retry-after` ≤ 60 (see below) |
| **aider** (`px-aider`) | `OPENAI_API_BASE` | `OPENAI_API_KEY` → `Bearer` (via LiteLLM) | not needed | — | retry with backoff (LiteLLM) |
| **OpenCode** (benchmark) | provider config in `opencode.json` | Bearer | not needed | — | — |
| **Apps (OpenAI SDK)** | `base_url` | `api_key` → `Bearer` | `user="u_…"` | `x-brain-session` header | `AuthenticationError`, no retry / `RateLimitError`, honours `retry-after` |
| **Apps (Anthropic SDK, or the `claude` CLI as a subprocess)** | `base_url` / `ANTHROPIC_BASE_URL` | `x-api-key` or Bearer | `metadata={"user_id": …}` | `x-brain-session` | same |

llm_brain accepts **both** headers and maps `user` and `metadata.user_id`
onto the same end-user field. The session header keeps a
[`brain/auto`](./auto-routing.md) decision for a whole conversation.
Everything else is configuration.

## Claude Code: the gateway contract

### What llm_brain exposes
- `POST /v1/messages` (arrives as `/v1/messages?beta=true`), **SSE
  streamed through without buffering** (Caddy `flush_interval -1`):
  Claude Code aborts a stream silent for 300 s.
- `POST /v1/messages/count_tokens`: forwarded as-is; if OpenRouter
  doesn't implement it, `/context` estimates locally.
- `HEAD /api/hello`: startup probe, always `204` (no key checked, nothing behind it).
- `GET /v1/models`: `brain/auto`, the `brain/<tier>` aliases and the tier
  models. Claude Code only asks for it with
  `CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY=1`, and only ids containing
  `claude` or `anthropic` enter its picker, so in practice the model is
  set by env.

### Headers and body
- `anthropic-version` and `anthropic-beta` are forwarded unchanged; the
  body is forwarded as-is apart from the sanitizer below (open lists, no
  allowlisting).
- **`system` array intact**, `cache_control` forwarded where it is:
  Claude Code's system block is stable for the whole conversation, so the
  prompt cache works. llm_brain prepends nothing.
- Consumed, not forwarded: `x-claude-code-session-id` → end user and
  `brain/auto` session, **without parsing the body**.

### Responses
- `retry-after` in **integer seconds**: ≤ 60 → Claude Code waits and
  retries; **> 60 → it stops and shows the error at once**. So the
  per-key throttle answers `retry-after: 5`; **budget exhausted →
  `retry-after` ≥ 3600 + `x-should-retry: false`**, so the CLI clearly
  says "budget exhausted" instead of retrying.
- Upstream error bodies are passed through **unwrapped**: Claude Code's
  recovery logic reads the upstream error text.

### Non-Claude models behind OpenRouter: the sanitizer
Anthropic **does not support** routing Claude Code to non-Claude models
through a gateway. It works in practice, but Claude Code sends fields a
non-Claude model rejects, so the proxy removes them on the Anthropic
dialect (`sanitize_anthropic`, logged per request):

| Field | Symptom without the sanitizer | What the proxy does |
|-------|-------------------------------|---------------------|
| `thinking: {"type":"adaptive"}` | `400` on `thinking`/`adaptive` | drops `thinking` when its type is `adaptive` |
| `context_management` | `400 Extra inputs are not permitted` | drops it |
| `output_config` (effort, structured output) | `400` | drops it |
| `tools[].strict`, `tools[].defer_loading` | `400` on the tool schema | drops both from every tool |
| mid-conversation `role: "system"` entries (Claude Code's reminders) | **no error**: the model ends the turn with no content → "no visible output" and loops (measured: 29% of requests on deepseek-v4-pro) | turns them into user turns and merges consecutive user turns, so the last message is always a user one |

`px-claude` also sets `CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS=1` and
`CLAUDE_CODE_DISABLE_ADAPTIVE_THINKING=1`, so most of these never leave
the client; the sanitizer covers what the flags miss (verified live: it
still removed `output_config` and `thinking.adaptive` with the flags set).
Each Claude Code release can add fields: that is the maintenance cost of
Claude Code on cheap models.

### Models
Claude Code sends `claude-*` ids for the main model and uses a separate
model for background tasks. `claude-*`, `sonnet`, `opus`, `haiku` map to
the profile's default tier; `brain/<tier>` and `brain/auto` pick
explicitly. `brain setup claude-code --proxy` prints the `px-claude`
function:

```
ANTHROPIC_BASE_URL="$BRAIN_BASE_URL"  ANTHROPIC_AUTH_TOKEN="$BRAIN_DEV_KEY"
ANTHROPIC_MODEL=brain/auto            ANTHROPIC_DEFAULT_HAIKU_MODEL=brain/fast
CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS=1  CLAUDE_CODE_DISABLE_ADAPTIVE_THINKING=1
```

Pin a tier with `ANTHROPIC_MODEL=brain/agent px-claude` or `/model
brain/<tier>`. For an unknown alias Claude Code assumes a 200K context:
correct it with `modelOverrides` if the model behind has less.

### Subscription
With `ANTHROPIC_AUTH_TOKEN`/`ANTHROPIC_API_KEY`/`apiKeyHelper` set, **the
claude.ai subscription is not used**: exactly the goal. With only
`ANTHROPIC_BASE_URL` and a saved claude.ai login, traffic passes through
the gateway but consumes the subscription.

### Rotation without restarts
Optional: `apiKeyHelper` in `settings.json`, a script that prints the key
(e.g. `pass show brain/dev`). Claude Code re-reads it and sends it in both
headers.

## aider

`brain setup aider --proxy` prints `px-aider`:

```
OPENAI_API_BASE="$BRAIN_BASE_URL/v1"  OPENAI_API_KEY="$BRAIN_DEV_KEY"
aider --architect --model openai/brain/reasoning --editor-model openai/brain/fast
```

`brain setup aider` also prints `.aider.model.metadata.json` (context and
prices of the aliases, otherwise aider misestimates costs) and the model
settings with edit formats for the `brain/*` names (aider matches settings
by name). aider is the cheap editor for targeted changes, not the agent.

## The apps

No custom SDK: `OpenAI(base_url=…, api_key=…)` and
`client.chat.completions.create(model="brain/fast", user=user_id, …)`, or
the Anthropic equivalents. The profile (tier, router, budget) is decided
by the key. Error handling as in [Authentication flow](./auth-flow.md);
per-app status in [App integration](./apps.md).
