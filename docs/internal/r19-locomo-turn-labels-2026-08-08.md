# R19 — per-turn evidence labels on LoCoMo, and the finding they overturn

**2026-08-08. $0 — no re-run, no model calls.** The labels are recovered from
the LoCoMo source and the retrieval was already recorded in the published
baseline's report JSON.

## What R19 does

`scripts/locomo_to_oracle.py` marked evidence *sessions* with an `answer_`
prefix and emitted no per-turn labels, so R15's evidence-turn metric reported
`n/a` on every LoCoMo-converted set. LoCoMo turns carry `dia_id` (`"D1:3"`) and
each QA carries `evidence: ["D1:3"]`, so the evidence turns were recoverable
all along.

Two things the implementation had to get right, both called out in the register
row before it was written:

- **Match by `dia_id`, never by turn index.** The converter drops empty-text
  turns, so position *i* in the converted session is not position *i* in the
  source.
- **Deep-copy sessions per QA.** Which turns are evidence differs per question;
  sharing turn dicts would let one question's labels leak into another's.

Result on the full answerable set: **1,438 questions, 2,140 evidence turns
labelled**, 2 questions whose `evidence` dia_ids match no non-empty turn.

## The gate, enforced in code

The register made regeneration conditional on proving sample membership had not
moved — otherwise the R11 held-out set stops being the set that was measured.
That check is now `--verify`, not a convention:

```bash
python3 scripts/locomo_to_oracle.py locomo10.json /dev/null --all \
  --verify ~/spectral-local-bench/locomo_full_answerable.json
# GATE PASSED: byte-identical after stripping `has_answer`.
#   1438 questions, 2140 evidence turns labelled, sample unmoved.
```

It regenerates with the given flags, strips `has_answer`, and compares
**serialized bytes** — the point being that a reader loading the old file and
the stripped new one cannot tell them apart. On failure it diffs question ids
and says whether membership moved, order moved, or content moved, then exits
non-zero without writing.

## The finding: the published baseline's interpretation was wrong

Rescoring `bm25-locomo-baseline.json` — the same run, same retrieved keys,
nothing re-executed:

| metric | session recall (published) | **evidence-turn recall (R19)** |
|---|---:|---:|
| micro (pooled) | 95.06% | **59.86%** (1281/2140) |
| macro (mean) | 97.13% | **68.63%** |
| questions retrieving **zero** evidence | 16 (1.11%) | **357 (24.86%)** |
| recall \| judged **correct** | 99.21% | **88.62%** |
| recall \| judged **incorrect** | 93.26% | **31.54%** |
| **difference** | **5.94pp** | **57.08pp** |

Per category:

| category | evidence-turn micro | macro | zero-evidence Qs |
|---|---:|---:|---:|
| single-session-user | 73.41% (657/895) | 74.95% | 200 |
| temporal-reasoning | 71.89% (266/370) | 74.84% | 70 |
| multi-session | **40.91%** (358/875) | 42.53% | 87 |

**Dilution on LoCoMo is 20.6×** — 44,162 turns in evidence sessions against
2,140 true evidence turns. Worse than LongMemEval's 12.2×, because LoCoMo's
evidence sessions are longer.

### What this reverses

`bm25-locomo-baseline-result-2026-08-07.md` concluded, in its headline section:

> *"Retrieval is not what is failing... lexical retrieval is not the binding
> constraint on this benchmark."*

That is **false**, and the correction is not marginal:

- A quarter of all questions — 357 of 1,436 — reach the actor with **no
  evidence turn at all**. The actor is being asked to answer from context that
  does not contain the answer. That it still scores 65.02% says more about
  LoCoMo's judge and answer key than about the retrieval.
- Evidence-turn recall separates right from wrong answers by **57.08pp**.
  Retrieval is not merely relevant to correctness on this benchmark; it is the
  dominant measured factor among the things we can see.
- Multi-session is the floor on every axis: 40.91% evidence-turn recall and
  39.64% accuracy. Those two numbers being nearly equal is not a coincidence
  worth ignoring.

### Why the error happened, since it is the same one twice

R15 established that session recall is saturated by construction — a long
evidence session counts as recalled when the answering turn is absent. The
baseline document *cites R15*, states correctly that evidence-turn recall is
`n/a` on LoCoMo, and then reasons from session recall anyway as though a high
value meant retrieval was sufficient.

The metric was correctly labelled and still misread. Labelling a diluted metric
does not stop it being used as if it were the real one — the only thing that
does is making the real one computable, which is what R19 is.

## What this changes downstream

- **The retrieval-lever programme is not exhausted.** Every refuted lever
  (porter, widening, spreading, ACT-R, cascade-k) was scored on session recall
  or the diluted key-recall. A lever that fixed the 357 zero-evidence questions
  would have read as noise on both. The nulls are not wrong, but they are
  weaker evidence than they were treated as.
- **G4 (term proximity) moves up.** It is admission-changing, so it can move
  zero-evidence questions, which is now the measured target.
- **The 65.02% headline is unaffected** and stays published as measured.

## Reproducing

```bash
python3 scripts/analyze_locomo_baseline.py \
  --report  ~/spectral-local-bench/bm25-locomo-baseline-2026-08-07/bm25-locomo-baseline.json \
  --dataset ~/spectral-local-bench/locomo_full_answerable_labelled.json
```

Expected keys are constructed exactly as `ingest::memory_key` builds them —
`{session_id}:turn:{index}:{role}`, index enumerated per session, a format
frozen by `memory_key_format_is_frozen`. If the two ever drifted, evidence
recall would read 0% and look like a catastrophic regression rather than a bug.

**Refs:** `turn-level-evidence-recall-2026-08-07.md` (R15),
`bm25-locomo-baseline-result-2026-08-07.md` (corrected by this),
`REPAIR_REGISTER.md` R19.
