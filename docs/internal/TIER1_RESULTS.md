# Tier-1 actor-replay results — cap and porter-vs-expansion

**Date:** 2026-07-02
**Total campaign spend:** ~$12 (actor+judge arms) + $0.35 (frozen Haiku
assets: expansion cache $0.125, paraphrase set $0.22). All Tier-0 work $0.
**Method:** paired case-level flips on targeted samples, per
`docs/internal/ORACLE_TIER0.md`. Actor/judge: Sonnet 4.6, same as published.

## Four-config retrieval picture (Tier-0, $0, full 500)

| Config | Sess-recall | Zero-evidence | Key-recall | Tok/q | LLM in retrieval |
|---|---|---|---|---|---|
| Neither | 96.9% | 8 | 51.8% | 13,675 | none |
| Porter only | 97.6% | 4 | 54.7% | 14,266 | **none** |
| Expansion only | 98.2% | 4 | 61.5% | 16,198 | Haiku/query |
| Both | 98.2% | 4 | 62.7% | 16,727 | Haiku/query |

Porter and expansion substantially overlap: both fix the same 4
zero-evidence questions; porter adds ~nothing on top of expansion
(5 sessions up / 5 down). The interesting question became replacement,
not addition.

## Arm A — assistant-turn cap 0.36: **REJECTED**

40-question sample, SSA-weighted (14/40), expansion-ON both arms.

| Arm | Accuracy | Ctx tok/q |
|---|---|---|
| baseline | 34/40 (85.0%) | ~16,125 |
| cap 0.36 | 28/40 (70.0%) | ~7,780 |

Flips: 1 fail→pass, **7 pass→fail — all 7 single-session-assistant.**
Mechanism verified on 7e00a6cb: the recommended hostel name lived in a
truncated assistant turn (context halved, 21 truncation marks); the actor
answered from surviving-but-wrong recommendations. The GeneralRecall shape
exemption does NOT protect SSA: those questions classify as Factual/General
but their evidence is in assistant turns. −53% tokens is real; the accuracy
price concentrates exactly where the original probe warned. **Do not ship**
without an evidence-aware gate (e.g. never truncate turns containing
retrieval-matched keys) — future work, re-gate through the same pipeline.
This is the Tier-0 blind spot doing its job: recall metrics were held by
construction; only the paid composition test could catch it, and did.

## Arm B — porter-only vs expansion-only head-to-head: **PORTER HOLDS**

60-question stratified sample (2 transport failures excluded per
clean-denominator rule → n=58 evaluated pairs shown as 60-sample results).

| Arm | Accuracy | Ctx tok/q | Retrieval overhead |
|---|---|---|---|
| expansion-only | 43/60 (71.7%) | ~16,509 | Haiku call, ~169 tok/q, $0.25/1k |
| **porter-only** | **46/60 (76.7%)** | ~14,558 | **0 tokens, 0 LLM, $0.00** |

Flips: 5 fail→pass (porter wins), 2 pass→fail. Of the 2: 06878be2 is a
real retrieval miss (expansion's synonyms surfaced camera-gear context
porter didn't); 2311e44b porter's answer contains the correct facts
(page 250/440) — judge-marginal. n=60 cannot claim porter is BETTER
(within noise), but supports the claim that matters:

> **A fully deterministic retrieval pipeline — no embedding model, no LLM
> call, no per-query token cost — matches the LLM-assisted configuration's
> accuracy on this sample.**

Recommended next: porter-only full n=500 (~$26) to make this the published
headline ("zero-LLM end-to-end at parity"), replacing the current
expansion-dependent 81.5% story with a strictly cheaper, strictly more
reproducible one. Pre-registered expectation from these results:
porter-only full-set accuracy within ±2pp of the 81.5% published number.

## Harness fix shipped with this analysis

Bench checkpoints now carry a config fingerprint (question filter, routing,
SPECTRAL_* env levers) and refuse to resume across configurations — arms of
an A/B comparison sharing a work_dir previously inherited each other's
results silently (caught here when both arms returned byte-identical
predictions; the wasted arms cost ~$0 since inherited questions skip API
calls, but the failure mode was silent).

## Frozen assets (all replayable at $0)

`~/spectral-local-bench/`: expansion-cache.json (500), paraphrases.json
(200), oracle-{baseline,porter,spectrogram,cap}.jsonl,
oracle-{baseline,porter,cap}-exp.jsonl, tier1-*-ids.txt, tier1-*.json.
