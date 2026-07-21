# Local weak-actor RERANK A/B — prerequisites satisfied, BLOCKED by hardware

Date: 2026-07-21 | main tip `efdd960` | attempted on this machine (Intel i5, 8 GB).

## What this is

The 2026-07-20 deep-research pass named the **local weak-actor RERANK A/B** the
*designated-decisive* un-run test: a strong cloud actor papers over retrieval
misses (spreading measured net ≤ 0 there), but a weak local actor cannot — so if
session-preserving RERANK spreading converts to accuracy anywhere, it converts
here. Blocked for months on "ollama not installed." That prerequisite is now
cleared; a different wall is now the binding one.

## Prerequisites cleared this session

- **ollama 0.32.1 installed** (`brew install ollama`), server runs on :11434.
- **Models pulled:** `llama3.2:3b` (weak actor), `qwen2.5:7b` (judge).
- **Context-window fix identified and applied.** `OpenAiActor`/`OpenAiJudge`
  (`crates/spectral-bench-accuracy/src/actor.rs`) send no `num_ctx`, so ollama
  would silently truncate the ~13–16k-token memory contexts to its 2048 default —
  which would have invalidated the A/B. Fixed out-of-band by baking the window
  into derived models via Modelfile: `llama32-32k` (num_ctx 32768, temp 0) and
  `qwen25-16k` (num_ctx 16384, temp 0). **For a real run this must be handled** —
  either these derived models or a `num_ctx`/`options` field added to the actor.
- Harness connectivity to the local OpenAI-compatible endpoint confirmed (a warm
  `/v1/chat/completions` call returns correctly).

## The hardware wall (measured, not estimated)

This machine: **Intel Core i5-8259U (4 cores), 8 GB RAM, no usable GPU offload**
(ollama logs `n_threads = 4`, CPU-only; the Iris Plus iGPU is not used).

| measurement | value |
|---|---|
| cold load, llama3.2:3b @ 32k ctx | **55 s** |
| 32k KV cache alone | 3584 MiB (K+V f16) |
| one realistic ~12k-token actor call (warm) | **did not finish in 6 m 40 s** |
| RAM during a single loaded model | 135 MB unused, **9.5 GB in swap** (thrashing) |
| llama3.2:3b + qwen2.5:7b co-resident | impossible (2 GB + 4.7 GB models + KV on 8 GB) |

A single actor call on a real memory context exceeds 6.5 minutes and drives the
machine deep into swap. The A/B needs **≥ 60 actor calls + 60 judge calls** (30
questions × 2 arms), with the 7B judge unable to co-reside with the actor
(forcing 55 s+ model swaps between every actor and judge call). Realistic total:
**many hours to > a day**, with steady OOM/swap-failure risk. Two further code
frictions compound it: `retry.rs` `MAX_TOTAL_RETRY_MS = 60_000` abandons any call
slower than 60 s (a cold load alone is 55 s), and the reqwest clients set no
explicit timeout.

## Verdict

**Not the lever's fault and not ollama's — this 8 GB CPU-only box cannot run a
valid weak-actor A/B at meaningful scale.** The test is now *fully staged*: the
only remaining requirement is adequate hardware (a GPU box, or a machine with
≥ 32 GB so actor + judge co-reside and prompt-eval is GPU-accelerated), or
running it in Permagent's environment. On such hardware the run is:

```bash
BIN=./target/release/spectral-bench-accuracy
DS=~/spectral-local-bench/longmemeval/longmemeval_s.json
# Arm A: FTS baseline
$BIN run --dataset "$DS" --work-dir ./ku-work --output ku-fts.json \
  --categories knowledge-update --max-questions 30 \
  --retrieval-path topk_fts --no-expand-queries \
  --actor-api openai --base-url http://localhost:11434 \
  --actor-model llama32-32k --judge-model qwen25-16k
# Arm B: FTS + session-preserving RERANK spreading
SPECTRAL_ASSOC_RERANK=15 SPECTRAL_ASSOC_SEEDS=3 \
$BIN run ... --output ku-spread.json   # same flags
```

Then paired analysis on the clean intersection (exclude transport failures).
A net-positive here — where the cloud actor was net ≤ 0 — is the accuracy proof.

**Before any real run, also fix the two code frictions** (add `num_ctx` to the
OpenAI-compat actor/judge bodies, and raise/relax `MAX_TOTAL_RETRY_MS` for slow
local models) so cold loads and slow generations don't register as transport
failures. Prereqs are done; only the box is missing.
