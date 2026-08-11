# Adjacency mechanism diagnostic — the ±1 rule is confirmed, my framing is not

**2026-08-11. $0, offline, on the archived R28 cascade arms** (`c0` / `c_adj`,
full N = 1,438). No brains, no model calls, no new measurement — this is a
re-read of rows already on disk. `scripts/diagnose_adjacency_mechanism.py`.

R28 established **that** adjacency works on the production path (+18.22pp). It
never established **why**. An effect without a mechanism is a corpus fit that
has not been found yet, so this asks what the recovered turns actually look
like.

## 1. It really is the ±1 rule — 390/390

| | n | share |
|---|---:|---:|
| Evidence turns adjacency recovers that sit **next to a turn the baseline already retrieved** | **390/390** | **100.0%** |

Not one recovered turn arrived because the candidate pool got wider. The lever
does exactly what it says on the tin, and the +18.22pp is attributable to the
structural rule rather than to incidental re-ranking. This was the outcome most
worth checking and it is unambiguous.

## 2. 252 of those turns had **no lexical bridge at all**

Overlap here is content-word overlap between the question and the evidence turn
itself, under one fixed crude tokenizer applied identically to both classes.

| overlap | recovered by adjacency | still missed by both |
|---|---:|---:|
| **0 — no lexical bridge** | 252 (64.6%) | 370 (74.6%) |
| 1 | 124 | 116 |
| 2 | 13 | 10 |
| 3+ | 1 | 0 |
| **total** | **390** | **496** |

**252 evidence turns share zero content words with their question and were
still retrieved.** No amount of BM25 re-ranking could ever have surfaced those
— they were never candidates. That is the substantive finding, and it is
consistent with the coreference inversion measured on 2026-08-09: the evidence
is the named person's own reply, which does not contain their name.

## 3. Where I was wrong

I predicted adjacency would be **enriched** for the zero-overlap class — that it
specifically rescues the coreference misses. **It is not.** The recovery *rate*
says the opposite:

| overlap class | recovered / all missed | rate |
|---|---:|---:|
| **0 — no lexical bridge** | 252/622 | **40.5%** |
| 1 | 124/240 | **51.7%** |
| 2 | 13/23 | 56.5% |
| 3+ | 1/1 | 100% |

Adjacency recovers zero-overlap turns at a **lower** rate than turns with a thin
lexical bridge. It is essentially **indifferent to lexical overlap** — slightly
biased *against* the coreference class, not toward it.

The honest reading: the miss population as a whole is coreference-shaped (both
classes are ~65–75% zero-overlap), and adjacency helps because **it is
orthogonal to the lexical channel**, not because it targets its failures. It
recovers whatever happens to sit beside something BM25 could find, and in
strictly-alternating dialogue that is often the reply that answers the question.

That is a weaker and more structural story than "it solves coreference", and it
should replace the mechanism sentence wherever adjacency gets written up.

## 4. What this changes

- **Strengthens** the R28 result: the gain is attributable to a stated
  structural rule, not to an unexplained re-ranking side effect.
- **Weakens** the generalisation argument. If adjacency worked *by* attacking
  the coreference inversion, it would transfer to any corpus with that
  inversion. Being overlap-indifferent instead means its value depends on the
  **dialogue geometry** — question-then-answer in adjacent turns. LoCoMo is
  two-party and strictly alternating, which is the best possible case. On a
  corpus where the answer is three turns away or interleaved between speakers,
  the ±1 rule has much less to grab.
- **496 evidence turns remain missed by both arms**, 370 of them with no lexical
  bridge. That residual is untouched by everything measured in this programme.

## 5. Pricing the next lever without running it

The same archived rows price a wider window (±2, ±3, …) for $0. For each of the
496 residual evidence turns: how far is it from a turn the **baseline** already
retrieved?

| window | new turns | cumulative | share of residual |
|---|---:|---:|---:|
| ±2 | +119 | 119 | 24.0% |
| ±3 | +55 | 174 | 35.1% |
| ±4 | +35 | 209 | 42.1% |
| ±5 | +26 | 235 | 47.4% |
| ±6 | +19 | 254 | 51.2% |
| **unreachable at any window ≤6** | — | **242** | **48.8%** |

Two things follow.

**ADJ2 is worth at most +119 turns — a +5.6pp ceiling on micro recall** (119 of
2,140 evidence turns), for a previously-predicted ~30% token increment. That is
a *ceiling*, not a forecast: widening also admits distractors, and the marginal
turns are the ones the ±1 rule already declined to reach. Compare adjacency's
own +390. **The window axis is decaying fast** and ADJ2 does not look like the
next experiment.

**Half the residual is structurally unreachable by any ±N rule.** 242 evidence
turns sit more than six turns from anything the lexical channel retrieved, and
370 of the 496 have no lexical bridge either. **Neither channel we own can
reach them at any setting.** That, not ADJ2, is the honest description of the
frontier — and it is an argument for a second modality rather than a wider
window on the one we have.

## 6. Limits

- Retrieval only. Nothing here says answers improve.
- The overlap tokenizer is crude by design and fixed before use; it is a
  like-for-like comparison, not a calibrated linguistic measure.
- "Adjacent to a retrieved turn" is computed from harness key arithmetic, the
  same key format the bench-scoped implementation parses.
- Single corpus. See §4 — that is now a sharper concern than it was.

**Refs:** `cascade-transfer-result-2026-08-10.md` (R28),
`speaker-attribution-diagnostic-2026-08-09.md` (the 8.5× inversion),
`turn-adjacency-result-2026-08-10.md` (R25).
