---
title: Bonsai 2 27B (ternary)
---

# `prism-ml/ternary-bonsai-2-27b`

Flagged because it's doing well in the benchmarks seen on OpenRouter.
Benchmarks are verified in Phase 0 on our workload, not assumed.

## Verified data (2026-09-19, OpenRouter and HuggingFace APIs)

| Item | Value |
|------|-------|
| Base | Qwen3.8-27B, reasoning, tool calling, images |
| Context | 262K tokens, max 32K output |
| OpenRouter price | $0.075/M input · $0.5/M output |
| Supported parameters | `tools`, `reasoning`/`reasoning_effort`, `structured_outputs`, `response_format` |
| Weights | **open, Apache-2.0**, GGUF on HF: `prism-ml/Ternary-Bonsai-2-27B-gguf` |
| Local size | **7.2 GB** (PQ2_0) or **6.0 GB** (PTQ1_0) — ternary / 2-bit |
| Local runtime | llama.cpp, Ollama, MLX (Mac) |
| Hosting on OpenRouter | **a single provider** (Darkbloom, int4 quant) |

## Why it matters more than any other model

It is **the same model in the cloud and locally**. A ~7 GB ternary 27B
runs on a consumer 8–12 GB GPU, so [Phase 0 → Phase 3](../roadmap.md)
doesn't change the model, only `provider` in the tier. Natural candidate
for `fast`; with a high `reasoning_effort`, potentially also a cheap
`reasoning` — two tiers, one model.

## Measured on 2026-09-20

| Check | Result |
|-------|--------|
| Anthropic-format requests (system array, `cache_control`, 25 tools, 60 KB) | accepted |
| Latency, trivial reply at ~15k input tokens | **33 s** (deepseek-v4-flash: 5.5 s) |
| Prompt cache | `cache_read_input_tokens: 0` on every call: **no L1 cache** on this provider |
| Under Claude Code's parallel requests | `429 Provider returned error`, 12 minutes of retries, task failed |
| aider, `ago-0001` | failed after 831 s |
| Default output | starts with a `thinking` block: budget `max_tokens` accordingly |

Verdict (decided 2026-09-20): **bonsai is the `reasoning` tier**, not the
`fast` one. Its strengths (reasoning, 262K context, open weights) fit the
architect role; its weaknesses (one slow provider, no prompt cache) are
tolerable there because the reasoning tier is called rarely and never has
to apply edits. `fast` is `deepseek/deepseek-v4-flash`, which solved
`ago-0001` with aider. Fallback for reasoning: `deepseek/deepseek-v4-pro`.

## To verify in Phase 0, in order

1. **Tool-call quality** behind Claude Code and aider: how many malformed
   per 100 turns.
2. **Skill adherence** (long prompts).
3. **A single provider on OpenRouter** = availability/latency risk: the
   `fast` tier needs a fallback to another model.
4. **Prompt cache**: the endpoint doesn't declare it; real cost to measure
   with Claude Code's system prompt.
5. **Local**: tok/s on your GPU with the PQ2_0 GGUF, and whether ternary
   quality holds against the cloud int4 endpoint.
