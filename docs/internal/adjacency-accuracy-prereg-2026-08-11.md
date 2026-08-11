# R30 — PREREG: does adjacency improve ANSWERS, or just retrieval?

**Registered 2026-08-11, before any arm ran.** $0 — **fully on-device**, ollama
`qwen25-16k` (7.6B Q4) as both actor and judge, no cloud calls, no API spend.

## The question this programme has never answered

Every result to date is **retrieval-only**. R29 established that adjacency beats
equal-budget k-raising by +7.57pp on evidence recall. Not one measurement says
whether a reader **answers better**. The stated risk has always been the
opposite: at 2.27× context the reader may be *drowned*, and 2.3× context that
lowers accuracy is a regression, not a win.

**No paid budget exists.** That has been the standing reason this went
unmeasured. It is not a good enough reason, because the machine has a local
model and the question is answerable on-device for $0.

## Design

| arm | config | retrieval (R29, measured) | tokens |
|---|---|---:|---:|
| **A0** | cascade defaults | 58.60% evidence recall | 1,500 |
| **A_ADJ** | `SPECTRAL_ADJACENCY=1` | 76.82% | 3,401 |

Single variable. Same actor, same judge, same prompts, same seed conditions,
temperature 0, `--no-expand-queries`, `--retrieval-path cascade`.

**This is the decision-relevant comparison** — "should Permagent turn this on"
— not the token-matched one. A_KMULT is a **secondary** arm, run only if the
primary completes and the machine is free.

## Slice: multi-session (n = 280), and why

Multi-session is where adjacency's retrieval gain is largest (39.09% → 58.86%,
**+19.77pp**) and where evidence presence separates accuracy most (the failure
analysis puts +37.5pp of retrieval's total accuracy value in this category).

**This is deliberately the most favourable slice, and that cuts both ways:**

- If accuracy does **not** move here, it will not move anywhere, and that is a
  strong negative result about the whole retrieval programme.
- If it **does** move, the result is **slice-specific and does not generalise**
  without a full-corpus replication that this prereg does not authorise.

Stating this now so a positive cannot later be quoted as a corpus-wide number.

## Why a weak local actor is the right instrument, and its cost

A 7B Q4 model scored **0/3** on the smoke probe. That is not a defect for this
design: a weak reader **cannot compensate** for missing evidence, so it is
maximally sensitive to a retrieval change — the same logic behind the project's
earlier weak-actor A/Bs.

But it imposes a real limit, registered in advance: **if baseline accuracy is at
the floor, no improvement is detectable.** If A0 scores below ~10%, the run is
**UNINFORMATIVE** on a floor effect and will be reported as such, not as a null.

## Deviations from the standard method, declared

1. **Not full N.** The standing rule is N = 1,438. On-device that is ~47 hours
   *per arm*. n = 280 (the whole multi-session slice, not a sample of it) is
   what a local actor affords. **The N=250 lesson does not apply** — that was a
   biased *subset* of a corpus; this is a complete category.
2. **`SPECTRAL_ACTOR_MAX_TOKENS=384`**, applied identically to both arms. The
   4096 default let one rambling answer cost 214s vs 64s. LoCoMo answers are
   short; 384 is far above any real answer. Same bound both arms or it is
   comparing two actors.
3. **Local judge, not the cloud judge** behind the published 65.02%. **No number
   from this run is comparable to any published figure** — it is an internal
   paired A/B only.

## Metrics and statistic

- **Primary: deterministic normalized containment.** Lowercase, strip
  punctuation and articles; correct iff every comma-separated ground-truth item
  appears as a substring of the prediction. Fixed here, computed offline from
  the stored `predicted` / `ground_truth`, **no model in the loop** — so it is
  reproducible by anyone with the report and cannot drift with judge mood.
- **Secondary: the local LLM judge's verdict**, already computed inline.
- **Statistic:** exact two-sided McNemar on the paired per-question correctness,
  discordant counts always shown, via the existing `compare` subcommand and
  `scripts/paired_mcnemar.py`.

Both metrics are computed on the same runs and **both are reported**, whichever
direction each points.

### AMENDMENT, 2026-08-11 16:20 — registered while arm A0 was still running and
### BEFORE arm A_ADJ existed at all

**A0 lands at 11.9% on both metrics** — barely above the 10% floor this prereg
registered as UNINFORMATIVE. Inspecting A0's own predictions (the treatment arm
does not exist yet, so nothing here can be steered by the comparison) shows why,
and it is a **design flaw of mine, not a property of the lever**:

multi-session ground truths are **multi-item lists** — *"swimming, catching
frisbees, balancing on a skateboard, sit, …"* — and both registered metrics are
**all-or-nothing**. A model that recovers 4 of 8 items scores exactly the same
as one that recovers none. I chose the slice for its retrieval headroom without
noticing it is also the slice with the least gradable answers.

**Amendment: add a third metric — item-level recall.** Fraction of the
comma-separated ground-truth items present in the prediction, per question,
averaged. Statistic: **Wilcoxon signed-rank** on the paired per-question
fractions.

- The binary containment metric **remains primary and unchanged**. No goalposts
  move: whatever it says will be reported as the primary result.
- Item recall is registered as a **graded secondary that carries the
  information when the binary metrics sit at the floor**, which they do.
- It is strictly better powered here: it uses the 88% of questions where a
  partial answer currently registers as an unqualified zero.

Registered now, in the open, rather than added after seeing A_ADJ — at which
point it would be indefensible.

## Verdict rules — fixed now

| condition | verdict |
|---|---|
| A_ADJ > A0, p < 0.05 on the primary | **PASS** — retrieval converts to answers on this slice |
| A_ADJ < A0, p < 0.05 | **REGRESSION** — the dilution risk is real and adjacency should not ship |
| p ≥ 0.05 | **NULL** — +18.22pp of evidence recall does not convert |
| A0 accuracy < ~10% | **UNINFORMATIVE** — floor effect, not a null |

**A NULL here would be the most important result of the session** and must not
be softened. The programme's entire retrieval case rests on an assumed
conversion that has never been measured.

## Power, honestly

At n = 280 with an assumed discordant rate ~20%, this detects roughly an **8pp**
difference at ~80% power. **A 3–5pp true effect would likely read as NULL.** The
verdict is "no detectable effect at this N", never "no effect".

## Predictions, on the record

I expect **NULL or a small positive**. Reasons: the +37.5pp category figure came
from *having* evidence versus not, and adjacency moves many questions from
partial to fuller evidence rather than from zero — R29 measured zero-evidence
143 vs 257, a real but partial shift. Against that, a 7B model at 3.4k context
is closer to its comfortable working set than at 1.5k, so dilution may not bite
at this scale.

If it comes out **strongly positive**, my first suspicion should be the judge
or the containment rule, not the lever.

**Refs:** `cascade-token-match-result-2026-08-11.md` (R29),
`cascade-transfer-result-2026-08-10.md` (R28),
`failure-analysis-2026-08-08.md`.
