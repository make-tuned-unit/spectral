# Fully-local accuracy harness — no cloud dependency — 2026-07-15

Runs the Spectral accuracy loop (actor + judge) against a **local**
OpenAI-compatible model, so the entire pipeline — storage, recall, *and* the
agent that reads the memory — is on-device. Two purposes:

1. **Local-first proof.** The memory layer is already network-free (verified:
   `spectral-graph/ingest/tact/core/spectrogram` have no network deps; storage is
   local SQLite files). This closes the loop: the *agent* can be local too, so a
   user who wants to retain control of all their data never sends it anywhere.
2. **The retrieval-failure-bound test bed.** A strong cloud actor (sonnet, 93% on
   knowledge-update) compensates for missing memories, so retrieval improvements
   don't convert (measured: TACT spreading net ≤ 0 there). A **weaker local
   actor cannot compensate** — if a needed memory isn't retrieved, it fails. That
   is exactly the regime where associative spreading's recovery should show up as
   accuracy. Same run answers both questions.

## Setup (one-time, on the user's machine)

```bash
# 1. Install ollama (https://ollama.com) — or any OpenAI-compatible local server
#    (llama.cpp --server, LM Studio, vLLM). ollama serves /v1/chat/completions.
brew install ollama          # macOS; or the official installer
ollama serve &               # starts the local server on :11434

# 2. Pull a model with a large context window (contexts here are ~13-16k tokens).
#    Bigger = stronger actor (harder to show retrieval effect); smaller = the
#    retrieval-stress regime. Try one of each.
ollama pull qwen2.5:7b       # capable, 128k context
ollama pull llama3.2:3b      # weaker — the retrieval-stress actor
```

## Run an A/B (fully local — no ANTHROPIC_API_KEY needed)

The bench Run command now takes `--actor-api openai` and talks to `base_url`.
Baseline vs associative spreading, temp=0 (pinned), on knowledge-update:

```bash
BIN=./target/release/spectral-bench-accuracy
DS=~/spectral-local-bench/longmemeval/longmemeval_s.json
WORK=./oracle-work            # reuse cached brains
M=llama3.2:3b                 # the weaker "can't compensate" actor

# Arm A: FTS baseline
$BIN run --dataset "$DS" --work-dir "$WORK" --output ku-fts-local.json \
  --categories knowledge-update --max-questions 30 \
  --retrieval-path topk_fts --no-expand-queries \
  --actor-api openai --base-url http://localhost:11434 \
  --actor-model "$M" --judge-model "$M"

# Arm B: FTS + session-preserving RERANK spreading (accuracy-safest config —
# +16-23pp key-recall at ~constant context, no distraction, no lost sessions).
SPECTRAL_ASSOC_RERANK=15 SPECTRAL_ASSOC_SEEDS=3 \
$BIN run --dataset "$DS" --work-dir "$WORK" --output ku-spread-local.json \
  --categories knowledge-update --max-questions 30 \
  --retrieval-path topk_fts --no-expand-queries \
  --actor-api openai --base-url http://localhost:11434 \
  --actor-model "$M" --judge-model "$M"

# (recall-max alternative, but grows context ~+20%):
#   SPECTRAL_ASSOC_COMBINED=1 SPECTRAL_ASSOC_CROSS=3 \
#   SPECTRAL_ASSOC_CROSS_BUDGET=2500 SPECTRAL_ASSOC_BUDGET=3000 SPECTRAL_ASSOC_SEEDS=3
```

Then the same paired analysis used for the cloud A/Bs (exclude any transport
failures, compare `correct` on the clean intersection). A *net-positive* here —
where the strong cloud actor was net ≤ 0 — would be the first evidence that
spreading converts to accuracy when the actor cannot paper over a retrieval miss.

## Notes

- `--actor-api openai` uses `OPENAI_API_KEY` if set, else a dummy (ollama ignores
  it). No cloud key required.
- temp=0 is pinned on both actor and judge (deterministic A/B).
- A local judge is weaker than sonnet; for the judge specifically, consider the
  larger local model (qwen2.5:7b) even when testing a small actor, to keep
  grading reliable. Or spot-check judge calls.
- Contexts are ~13–16k tokens; ensure the model's context window covers it
  (qwen2.5 / llama3.2 are 128k, fine).

## Status

Actor/judge code (`OpenAiActor`, `OpenAiJudge`) and the `--actor-api` selector are
built and compile; **not yet run against a live local model** in this
environment (no local server available here). The user runs the setup above to
execute the fully-local A/B.
