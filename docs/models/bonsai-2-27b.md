---
title: Bonsai 2 27B (ternary)
---

# `prism-ml/ternary-bonsai-2-27b`

**Role today: the `reasoning` tier** (aider's architect), fallback
deepseek-v4-pro. Not in the `brain/auto` ladder. Picked up on 2026-09-19
because it did well in the benchmarks seen on OpenRouter; measured on our
workload on 2026-09-20.

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

## Why it was interesting

It is **the same model in the cloud and locally**: a ~7 GB ternary 27B
runs on a consumer 8–12 GB GPU, so a move to [local GPU](../architecture/gpu.md)
would change only the tier's provider, not the model. It was first the
candidate for `fast`; the measurements below moved it to `reasoning`.

## Measured on 2026-09-20

| Check | Result |
|-------|--------|
| Anthropic-format requests (system array, `cache_control`, 25 tools, 60 KB) | accepted |
| Latency, trivial reply at ~15k input tokens | **33 s** (deepseek-v4-flash: 5.5 s) |
| Prompt cache | `cache_read_input_tokens: 0` on every call: **no L1 cache** on this provider |
| Under Claude Code's parallel requests | `429 Provider returned error`, 12 minutes of retries, task failed |
| aider, `ago-0001` | failed after 831 s |
| Default output | starts with a `thinking` block: budget `max_tokens` accordingly |

Second finding (2026-09-20, real aider session): `Provider returned error`
with 4 retries on a large architect request, while six small probes passed
in 1.4 s each — the single provider degrades under size/load. Mitigation
in place: OpenRouter `models[]` fallback to deepseek-v4-pro
([configuration](../configuration.md#aider-model-fallbacks-and-edit-formats-aidermodelsettingsyml)).

Confirmed by benchmark (2026-09-20): architect bonsai + editor
deepseek-v4-flash in `diff` format, fallback on → `ago-0001` **PASS in 74 s**.

Verdict (decided 2026-09-20): **bonsai is the `reasoning` tier**, not the
`fast` one. Its strengths (reasoning, 262K context, open weights) fit the
architect role; its weaknesses (one slow provider, no prompt cache) are
tolerable there because the reasoning tier is called rarely and never has
to apply edits. `fast` is `deepseek/deepseek-v4-flash`, which solved
`ago-0001` with aider. Fallback for reasoning: `deepseek/deepseek-v4-pro`.

## Still open

- **Local**: tok/s on a consumer GPU with the PQ2_0 GGUF, and whether
  ternary quality holds against the cloud int4 endpoint. Only relevant if
  the [GPU phase](../architecture/gpu.md) happens.
- **Single provider** on OpenRouter: availability risk, covered by the
  deepseek-v4-pro fallback.
