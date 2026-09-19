---
title: From OpenRouter to a GPU
---

# OpenRouter today, GPU tomorrow

:::note Status: a wish, not a plan
Phase 3 is not planned. Of this page, only the rule "no code outside
`providers/` knows the provider" applies today, and it costs nothing. The
rest is here for when (if) it's needed.
:::

We start with OpenRouter: every model with a single key and no
infrastructure. But the layer is designed from the start so that
**moving to an own GPU is only a config change**.

## Design rules

- Tiers point to `(provider, model)`; `provider` can be `openrouter`
  today and `local` (Ollama/llama.cpp) or `vllm` tomorrow.
  `providers/local.py` already exists; vLLM exposes an OpenAI-compatible
  endpoint and fits `providers/openai.py` with a different `base_url`.
- **No code outside `providers/` knows the provider.** The translator
  produces the internal format; the provider converts it.
- The [L1 cache](./cache.md) changes nature locally (vLLM prefix cache /
  llama.cpp KV cache): the cache manager needs a `none` strategy.
- A hybrid tier is already foreseen by agent-orchestrator's `hybrid`
  preset: `fast` local, `reasoning` cloud. Probably the end state.

## The bridge: same model in the cloud and locally

[`prism-ml/ternary-bonsai-2-27b`](../models/bonsai-2-27b.md) is on
OpenRouter **and** has open weights as a 7 GB ternary GGUF. Phase 0 →
Phase 3 doesn't change the model, it changes `provider` in the tier.

## Indicative hardware requirements

| Model | VRAM |
|-------|------|
| bonsai-2-27b GGUF PQ2_0 (7.2 GB) | 8–12 GB (or a Mac with MLX) |
| bonsai-2-27b GGUF PTQ1_0 (6.0 GB) | 8 GB |

The available GPU is discussed later ([decisions](../decisions.md)).
