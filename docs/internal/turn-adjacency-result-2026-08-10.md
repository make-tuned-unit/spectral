# R25 — turn adjacency · **PASS on the token-matched control** (2026-08-10)

**$0. Retrieval-only oracle, LoCoMo, full N = 1,438, `--retrieval-path
topk_fts`, R19 turn labels. No model calls, no paid runs, model-free.**
Preregistered at `a63e446` **before implementation**:
`turn-adjacency-prereg-2026-08-09.md`.

Capability is **bench-scoped and default off**. See "What does NOT follow".

## Result

**PRIMARY (preregistered): ADJ1 vs KMATCH — both at the same token budget.**

| | evidence micro | zero-evidence | tokens |
|---|---:|---:|---:|
| A0″ baseline (k=40) | 59.86% (1281/2140) | 357 | 1.00× |
| **KMATCH** (k=105, no adjacency) | 72.06% (1542/2140) | 228 | 2.62× |
| **ADJ1** (k=40, ±1 neighbours) | **78.79% (1686/2140)** | **119** | 2.50× |
| ADJ2 (k=40, ±2 neighbours) | 84.53% (1809/2140) | 63 | 3.63× |

| comparison | Δ | nonzero pairs | p | verdict |
|---|---:|---:|---:|---|
| **ADJ1 vs KMATCH** (token-matched) | **+6.73pp (+144)** | 303 [+219/−84] | <0.0001 | **PASS** |
| *ADJ1 vs k=40 (flattering)* | *+18.93pp (+405)* | *344 [+344/−0]* | *<0.0001* | *PASS* |
| ADJ2 vs ADJ1 (cost-unmatched) | +5.75pp (+123) | 116 [+116/−0] | <0.0001 | PASS |

**At an identical token budget, spending context on the *neighbours of what you
found* beats spending it on *more of what BM25 ranked next* — by 144 evidence
turns.**

**Zero-evidence questions fall 357 → 119 → 63 — an 82% reduction.** On the
corpus where retrieval is the binding constraint, dialogue structure removes
most of the total misses.

## Why the token-matched primary mattered

The flattering comparison is **+18.93pp**. The honest one is **+6.73pp**. Both
are real; only the second answers "is this lever worth building", because
"more context retrieves more evidence" is arithmetic, not a finding.

Had ADJ1-vs-k=40 been the headline, this document would claim roughly three
times the effect it is entitled to.

## The mechanism: adjacency is not a cheaper k

Of the 859 evidence turns missed at k=40, by which lever recovers them:

| recovered by | turns | share |
|---|---:|---:|
| **only adjacency** | **272** | **31.7%** |
| only k=105 | 128 | 14.9% |
| both | 133 | 15.5% |
| **neither** | **326** | **38.0%** |

**Only 15.5% overlap.** Adjacency reaches 272 turns k=105 cannot reach at all,
at the same cost. Deep-BM25-ranked turns and dialogue-adjacent turns are
**structurally different populations**, exactly as the discourse-pair account
predicts. This is the mechanism claim, and it is confirmed rather than assumed.

**Speaker attribution (R24) contributes zero exclusively** — all 68 turns it
recovers are also recovered by k=105. R24 remains a genuine cheap win at fixed
k=40 (+2.76pp for +0.3% tokens) but it is **subsumed by k-raising, not
complementary to it.** That materially narrows how R24 should be described.

## A prediction made before the run, and how it did

Before ADJ2 ran, this was recorded: *"real but clearly diminishing — roughly a
third of ADJ1's increment, and loses on tokens."* Reasoning: the d=2 mass
(14.3% of misses) is under a third of d=1's (47.1%).

**Actual: +123 turns = 30% of ADJ1's +405.** Recall tracked the distance
distribution. The diagnostic-then-predict method is now two-for-two (it also
predicted adjacency's 2.62× token cost from archived data, measured 2.50×).

## Diminishing returns, quantified

| step | turns gained | extra context | efficiency |
|---|---:|---:|---:|
| k=40 → ADJ1 | +405 | +1.50× | **270 turns/unit** |
| ADJ1 → ADJ2 | +123 | +1.13× | 109 turns/unit |
| *(→ session-completion T≥3, priced not run)* | *~84* | *+2.74×* | *~31 turns/unit* |

A 2.5× efficiency drop to ±2, then an 8× drop beyond. **±2 is still on a
reasonable part of the frontier; the cliff comes after it.**

## What does NOT follow

- **No accuracy claim.** Retrieval only, no end-to-end arm, none budgeted.
  2.50–3.63× context is a lot to hand a reader, and higher evidence recall
  could plausibly produce *worse* answers if dilution dominates. Nothing here
  says otherwise.
- **ADJ2 vs ADJ1 is cost-unmatched** and is reported as such. The honest
  question — is ±2 better than spending the same context another way — needs a
  k≈145 control; R27's k=150 arm supplies it.
- **`recall_cascade` is unmeasured**, and it is the only path Permagent calls.
  R28 tests transfer; until it reports, none of this is known to reach
  production.
- **Bench-scoped implementation.** `apply_turn_adjacency` parses the harness key
  format `{session}:turn:{index}:{role}`. Production needs real sequence
  metadata on the memory — a separate design question.
- **Corpus-shaped.** LoCoMo is two-party with strictly alternating turns
  (272/272 sessions). Adjacency's ideal case. Expect this to be bounded the way
  R24 was bounded by LongMemEval's lack of named speakers.

## Reproducing

```bash
bash scripts/run_adjacency_arms.sh          # kmatch, adj1, adj2 at full N
python3 scripts/score_r24.py --baseline kmatch.jsonl --arm adj1.jsonl --label ADJ1
```

**Refs:** `turn-adjacency-prereg-2026-08-09.md`,
`turn-adjacency-diagnostic-2026-08-09.md` (which priced this from archived data
before it was built), `speaker-field-result-2026-08-09.md` (R24, subsumed here),
`cascade-transfer-prereg-2026-08-10.md` (R28).
