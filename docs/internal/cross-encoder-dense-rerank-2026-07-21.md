# Cross-encoder rerank (#3) + dense hybrid (#4) — measured over the existing pool

Date: 2026-07-21 | main tip `efdd960` | $0, no LLM (retrieval-metric only).

## What this answers

Two of the un-run levers from the 2026-07-20 deep-research pass:
- **#3 cross-encoder rerank over the existing pool** — literature's "most consistent
  precision lever, recall-neutral."
- **#4 hybrid dense (BM25 + dense RRF)** — expected NULL on LongMemEval-S (lexical
  regime) per the lit's corpus-dependence finding; a semantic-regime lever.

## Toolchain note (why Python)

The Rust path is blocked: `fastembed`/`ort-sys` has **no prebuilt ONNX binaries for
`x86_64-apple-darwin`** and no source build is set up (the standing toolchain quirk).
Python `onnxruntime` ships prebuilt macOS-x86_64 wheels, so the probe runs on Python
`fastembed` 0.7.4 in a venv. Same models, different runtime.

## Method

Per hard case, take the shipped cascade's ordered candidate pool
(`SPECTRAL_CASCADE_K=500`, expansion OFF — today's default retrieval), keep the
**top-120 as the rerank window** (retrieve-many → rerank-120 → keep top-40), and
re-order three ways, recomputing answer-key recall@40 and answer-**session**
recall@40 (the accuracy-gating metric; denominators are full-haystack from the oracle):

- **baseline** — pool order (what ships)
- **ce** — cross-encoder `ms-marco-MiniLM-L-6-v2` over (question, content)
- **dense** — `bge-small-en-v1.5` cosine(question, content)
- **hybrid** — RRF(bm25_rank, dense_rank), k=60

Set: the 31 cached `oracle-hard` multi-session cases (the retrieval-starved regime).

## Results

| arm | answer-key recall@40 | **session recall@40** | Δ session vs baseline |
|---|---|---|---|
| baseline | 0.4185 | 0.9758 | — |
| **ce** | 0.5405 | **0.9919** | **+0.0161** |
| dense | 0.5687 | 0.9839 | +0.0081 |
| hybrid | 0.5186 | 0.9839 | +0.0081 |

**Zero session losses** across all 31 cases in every arm — no rerank ever displaced
an answer session out of the top-40. Recall-safe, as the literature predicts for CE.

### The gain is real but concentrated in 3 cases

28 of 31 cases are already fully session-retrieved at baseline (sr=1.0) — **synthesis-
bound, no retrieval headroom** (re-confirming retrieval isn't their bottleneck). Only
3 have headroom, and the entire aggregate sr lift comes from them:

| case | answer sessions | baseline | ce | dense | hybrid |
|---|---|---|---|---|---|
| 2ce6a0f2 | 4 | 0.75 | **1.00** | **1.00** | **1.00** |
| 6d550036 | 4 | 0.75 | **1.00** | 0.75 | 0.75 |
| gpt4_15e38248 | 4 | 0.75 | 0.75 | 0.75 | 0.75 |

- **CE recovers the missing answer session in 2 of 3 headroom cases** (2ce6a0f2, 6d550036).
- Dense/hybrid recover 1 (2ce6a0f2 only).
- gpt4_15e38248 is unrecoverable by any arm: its missing operand sits at BM25 pool-rank
  **302**, beyond the top-120 rerank window (and beyond any sane window).

## Verdict

**Cross-encoder (#3) is the best of the three and is recall-safe** — it lifts mean
session recall +1.6pp and answer-key recall +12pp on the hard set with **zero session
losses**, matching the literature's "consistent precision lever." Dense and hybrid (#4)
help less here (+0.8pp), consistent with the predicted near-null on LongMemEval-S's
lexical regime — BM25 already wins the vocabulary-overlap cases, so dense adds little.

**Three hard caveats before this is a lever to ship:**
1. **Retrieval-metric only, NOT accuracy.** Every prior retrieval lift on this benchmark
   (ACR +18–40pp key recall, K=80 +8.5pp) failed to convert to end-to-end accuracy
   because 28/31 cases are synthesis-bound and a capable actor already has what it needs.
   The +1.6pp CE session lift rests on **2 case-recoveries out of 31** — underpowered,
   and its accuracy conversion is unproven (needs the actor A/B, lever #2, hardware-blocked).
2. **Product-stance collision.** CE and dense both require a neural model + `ort`/onnx
   runtime — exactly what Spectral's no-embedding, local-first, offline commitment
   excludes. This is a **value decision** (accept a model dependency for a small,
   unproven-to-accuracy precision gain), not a benchmark question. On this Intel Mac the
   Rust `ort` path doesn't even build.
3. **Window-bounded.** Reranking recovers only operands already inside the top-120 pool;
   the deep vocab-mismatch tail (pos 197/302) is untouched — see the dense deep-operand
   addendum.

Net: **CE is the strongest retrieval-precision lever measured on Spectral to date and is
recall-safe, but it is (a) unproven to accuracy on this benchmark and (b) blocked by the
no-embedding product stance.** File it with the other measured-but-not-shipped retrieval
levers; its real test is a semantic-regime production workload with a weak actor — the
same regime lever #2 targets.
