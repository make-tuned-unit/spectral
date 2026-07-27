# Local weak-actor RERANK A/B — RUN (M1 mac mini, 2026-07-27)

Unblocks `local-actor-ab-hardware-blocked-2026-07-21.md`. The staged, designated-decisive
test ran to completion on the M1 mac mini (16 GB, Metal GPU) — the box that clears the
Intel-8GB wall the prior doc measured.

## TL;DR (final, powered)

**RERANK spreading does NOT convert to accuracy under a weak actor.** Full knowledge-update
category (n=78): A 18/78 (23.1%) vs B 20/78 (25.6%), McNemar **p=0.81** — a wash (10 recovered
/ 8 regressed). The encouraging n=30 pilot (+10pp) was regression-to-the-mean: on the **fresh
48 held-out questions spreading is net-negative** (5 recovered / 6 regressed). The +3 lived
entirely in the pilot's original 30. Retrieval-lever family confirmed exhausted with a
measured null, not an asserted one.

## Pilot result (n=30, superseded by the powered run below)

| arm | config | accuracy |
|---|---|---|
| A | FTS baseline (`topk_fts`, no expand) | **7/30 (23.3%)** |
| B | + session-preserving RERANK spreading (`SPECTRAL_ASSOC_RERANK=15 SPECTRAL_ASSOC_SEEDS=3`) | **10/30 (33.3%)** |

Paired, clean intersection **30/30** (zero transport/auth failures either arm; config
fingerprints differ, so the arms are genuinely the lever apart):

- net **+3 questions (+10.0pp)** for spreading
- 5 recovered (B-only), 2 regressed (A-only), 5 both-correct, 18 both-wrong
- **McNemar exact two-sided p = 0.4531 — NOT significant** (discordant n=7)
- recovered qids: `07741c44 6aeb4375 71315a70 9ea5eabc ce6d2d27`
- regressed qids: `45dc21b6 6071bd76`

Both arms return 40 keys; spreading changed retrieval *composition* within the top-40
window, not count — swapping session-preserving evidence into reach of the weak actor.

## Powered result (n=78, full knowledge-update category — DECISIVE)

| arm | accuracy |
|---|---|
| A | FTS baseline | **18/78 (23.1%)** |
| B | + RERANK spreading | **20/78 (25.6%)** |

Paired clean **78/78** (the 2 transport blips retried and recovered; 0 data loss —
the timeout fix worked). Net **+2 (+2.5pp)**; discordant **10 recovered / 8 regressed**;
**McNemar exact two-sided p = 0.8145 — decisively not significant.**

Fresh-split diagnostic (the tell):

| split | recovered | regressed | net |
|---|---|---|---|
| pilot 30 (in-sample) | 5 | 2 | +3 |
| fresh 48 (held-out) | 5 | 6 | **−1** |

**Verdict: the pilot's +10pp was regression to the mean.** On genuinely held-out questions
spreading is net-negative; overall it churns questions near-symmetrically without systematic
gain. This is the honest number the design asked for — a weak actor cannot convert
session-preserving RERANK spreading to accuracy on knowledge-update. The retrieval-lever
family is exhausted; further tuning here is not where accuracy comes from (consistent with
`TIER1_PORTER_WIDEN.md`: the actor/synthesis stage is the ceiling, not retrieval).

## Harness fixes required to make the run valid (not in the lever)

The prior doc's predicted frictions were all real on current main:

1. **`localhost` → IPv6 `::1`**, which ollama refuses → use `--base-url http://127.0.0.1:11434`.
2. **reqwest default timeout** aborted legitimate ~65 s warm prompt-eval calls as
   "operation timed out". Fixed: explicit 600 s timeout on `OpenAiActor`/`OpenAiJudge`
   clients (`crates/spectral-bench-accuracy/src/{actor,judge}.rs`).
3. **cold-load / model-swap timeouts** — pin both models resident (`keep_alive:-1`);
   actor `llama32-32k` (num_ctx 32768) + judge `qwen25-16k` (num_ctx 16384) co-reside in
   ~11.4 GB, impossible on the old 8 GB box.
4. `retry.rs` error logging switched to `{e:#}` to surface the reqwest source chain
   (that is how "operation timed out" was diagnosed).

Frozen: `~/spectral-local-bench/wa-ab/{ku-fts.json, ku-spread.json, analyze_ab.py, run_ab.sh}`.
