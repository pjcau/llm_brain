---
title: From OpenRouter to a GPU
---

# OpenRouter today, GPU tomorrow

:::note Status: a wish, not a plan
Phase 3 is not planned and nothing of it is built. The page is here for
when (if) it's needed.
:::

Today every tier is an OpenRouter model id, reached with the profile's
OpenRouter key. The layer is kept so that **moving a tier to an own GPU
stays a config change** for the clients: they only ever see `brain/<tier>`
aliases.

## What it would take

- **An upstream per tier.** Today the proxy has one upstream (OpenRouter)
  and forwards each dialect as-is. vLLM and llama.cpp expose an
  OpenAI-compatible endpoint, so the OpenAI dialect would only need a
  per-tier base URL; Claude Code's Anthropic dialect would need either a
  server that speaks `/v1/messages` or the translator described in the
  [API layer](./api-layer.md).
- **Prompt cache.** The [L1 cache](./cache.md) becomes the local prefix /
  KV cache: nothing to do on the layer side, and no backend
  fragmentation.
- **Hybrid end state**: `fast` local, heavier tiers in the cloud.

## The bridge: same model in the cloud and locally

[`prism-ml/ternary-bonsai-2-27b`](../models/bonsai-2-27b.md) (the
`reasoning` tier) is on OpenRouter **and** has open weights as a 7 GB
ternary GGUF: moving it changes where the tier points, not the model.

## Indicative hardware requirements

| Model | VRAM |
|-------|------|
| bonsai-2-27b GGUF PQ2_0 (7.2 GB) | 8–12 GB (or a Mac with MLX) |
| bonsai-2-27b GGUF PTQ1_0 (6.0 GB) | 8 GB |

The available GPU is discussed later ([decisions](../decisions.md)).
