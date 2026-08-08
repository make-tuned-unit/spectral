# Result — the BM25-only LoCoMo baseline (2026-08-07)

**Preregistered in `bm25-locomo-baseline-prereg-2026-08-07.md` before any
measurement.** That document commits us to publishing this number whatever it
is, with no re-runs to find a better one. This is that publication.

**What this is:** a **floor measurement**. It answers one question — what
end-to-end LoCoMo accuracy does a memory layer with **zero model inference at
read or write time** achieve? It is not a competitive claim, and there is no
comparison table, for reasons given under "Not comparable to anything
published" below.

## The headline

**65.02%** on the full LoCoMo answerable set (935/1438), **95% CI
[62.79%, 67.25%]** (cluster-robust, ±2.23pp).

```
zero model inference in the memory layer  ->  65.02%  end-to-end
                                              95.06%  session recall (micro)
```

**The finding that matters is the gap between those two lines.** A memory layer
that runs no model at all retrieves at least one evidence session for **98.9%**
of questions and 95.06% of all evidence sessions — but only 65.02% of the
answers are then judged correct. Retrieval is not what is failing.

Sharper: mean session recall on questions judged **correct** is 99.21%; on
questions judged **incorrect** it is 93.26%. A 5.94pp difference. Whatever
separates a right answer from a wrong one on LoCoMo, it is overwhelmingly not
whether the evidence was retrieved.

We are not claiming the remaining 35pp is all reader error — the ~6.4% wrong
answer key and the judge both live in there, and we did not decompose it. What
the number does establish is that **lexical retrieval is not the binding
constraint on this benchmark**, which is the thing a floor measurement is for.

## Configuration — exactly as preregistered

| | |
|---|---|
| commit | `9119c93` (merge of #255; ≥ the merges landing R15 and R16, as the prereg requires) |
| retrieval | `--retrieval-path topk_fts` — plain FTS5/BM25. No cascade, no shape routing, no wings, no constellation tier, no recognition. |
| expansion | `--no-expand-queries` |
| **model calls in the memory layer** | **zero**, at read and at write |
| tokenizer | shipped default (`porter unicode61`), no env levers set |
| ingest | `per_turn`; fingerprints at shipped default (they do not participate in `topk_fts`) |
| actor / judge | `claude-sonnet-4-6` — these are the **reader**, not the memory layer |
| dataset | `~/spectral-local-bench/locomo_full_answerable.json`, **1,438 questions**, no sampling |
| max results | 40 |

**The actor and judge are the reader.** The claim is about a memory layer that
does no inference. A model still reads the retrieved context and answers. Any
quotation of this number that drops that distinction is misquoting it.

## Why the interval is not the one vendors report

LoCoMo is **10 conversations**, not 1,438 independent questions. Every question
on conversation N shares that conversation's haystack, speakers and annotation
quality, so questions are not independent draws and a binomial interval over
n=1,438 is the wrong variance.

We report a **cluster-robust interval**: the SE of the ten per-conversation
accuracies, with a **t(9) critical value**. The small-G correction matters — a
percentile cluster bootstrap over only 10 clusters is anti-conservative — so we
take the wider, more defensible figure as the headline and show the others.

| interval | 95% CI | half-width | note |
|---|---|---:|---|
| **cluster-robust, t(9)** | [62.79%, 67.25%] | **±2.23pp** | **headline** |
| percentile cluster bootstrap | [63.46%, 66.76%] | ±1.65pp | anti-conservative at G=10 |
| naive binomial over n=1438 | — | ±2.46pp | **wrong** — ignores clustering |

Between-conversation SD: **3.12pp** over G=10. Per-conversation accuracy ranges
**60.90% – 70.37%**.

**Deviation from the prereg, disclosed:** the prereg anticipated "~±5.5pp"
clustered. That figure was carried over from `landscape-research-2026-08-07.md`
as a general claim about the LoCoMo literature; it was **not** measured on our
data. Our measured interval is **±2.23pp** — narrower.

Two honest consequences, neither convenient:

1. **The clustered interval here is *narrower* than the naive binomial**
   (design effect 0.91×), because the ten conversations turn out to be
   unusually homogeneous — 3.12pp of spread across all ten. The general
   argument "LoCoMo is 10 conversations, so vendor ±0.31pp understates
   uncertainty ~8×" **still stands** (our ±2.23pp is 7.2× their ±0.31pp), but
   the specific ~±5.5pp figure does not reproduce on this dataset at this n,
   and we should stop quoting it as if it were measured.
2. A narrower interval is the more flattering result, which is exactly why this
   deviation is stated here rather than quietly absorbed. The prereg exists to
   make substitutions like that visible.

## The retrieval-side companion: session recall only

Evidence-**turn** recall is **`n/a` on LoCoMo — undefined, not 0%**.
`scripts/locomo_to_oracle.py` marks evidence *sessions* with an `answer_`
prefix and never emits per-turn `has_answer` labels, so R15's metric correctly
refuses to score it. Reporting a coverage number here would repeat exactly the
12×-diluted mistake R15 exists to correct. Teaching the converter `dia_id` →
turn is register **R19**, gated on a strip-and-diff byte-equality check so the
held-out samples cannot silently move.

Session recall is computed directly from this run's own `retrieved_memory_keys`
against the dataset's `answer_`-prefixed `haystack_session_ids` — no second run,
no extra spend.

| | micro (pooled) | macro (mean of per-question ratios) |
|---|---:|---:|
| **all** | **95.06%** (1846/1942) | **97.13%** |
| single-session-user | 99.05% (834/842) | 99.05% |
| temporal-reasoning | 97.42% (340/349) | 97.74% |
| multi-session | 89.48% (672/751) | 90.67% |

Questions retrieving **zero** evidence sessions: **16 / 1438 (1.11%)**.

Multi-session is the weak slice on both axes — 89.48% session recall and 39.64%
accuracy — which is what you would expect from a lexical retriever asked to
assemble an answer spread across several conversations.

## Per-category accuracy

| category | n | accuracy | 95% CI (cluster bootstrap) |
|---|---:|---:|---|
| single-session-user | 841 | **70.15%** | [66.00%, 74.15%] |
| temporal-reasoning | 317 | **73.82%** | [68.95%, 79.37%] |
| multi-session | 280 | **39.64%** | [35.25%, 44.06%] |

## Mandatory caveats — these travel with the number

Any publication of this figure must carry these in the same document.

1. **LoCoMo's answer key is ~6.4% wrong** — 99 score-corrupting errors in 1,540
   questions, including temporal-reasoning errors. The practical ceiling is
   near **93.6%**, not 100%.
2. **The standard judge accepts ~62.8%** of deliberately-wrong but
   topically-adjacent answers. It rewards vagueness — which is the signature of
   weak retrieval, so the bias flatters a floor measurement like this one. It
   also cuts the other way on specific answers; see "What the judge did to us"
   below.
3. **Not comparable to anything published.** The same system has scored
   **58.44 / 65.99 / 75.14 / 84** on LoCoMo depending on who ran it — a
   25.6-point spread wider than any published gap between systems. Harness
   dominates system. We therefore build no comparison table and this number
   must not be placed next to a vendor's.
4. **Full-context baselines beat memory systems** in several of those systems'
   own papers (Mem0: 72.9 vs 68.4; MIRIX: 87.52 vs 85.38).

## What the judge did to us

Caveat 2 is usually cited for judge *leniency*. On a floor measurement it also
bites in the other direction, and we owe a concrete example rather than a
general warning.

Of the 499 clean answers judged wrong, **31 (6.2%, or 2.16% of all questions)**
contain every content word of the ground truth, and **137 (27.5%)** contain at
least half.

**We are not claiming those are 31 stolen points.** Manual inspection of a
sample shows most are legitimately wrong — the answer contains the ground-truth
words while asserting the opposite ("There is no information about Jean… Gina
has visited Rome", ground truth "Rome"), or bundles a wrong claim with a right
one. A crude token-overlap flag is not a scoring correction and **no correction
is applied**. The honest statement is narrower: the judge is strict enough on
over-inclusive answers (predicted "Cooking and travel" against ground truth
"Cooking" → wrong) that caveat 2's leniency finding should not be read as a
one-directional bias in our favour.

## Run hygiene — every failure, as promised

| | |
|---|---:|
| transport failures | **0** |
| auth failures | **0** |
| judge-parse failures | **4** |
| clean | 1434 |
| recovered after retry | 0 |
| empty predictions | 0 |
| error predictions | 0 |
| **questions retrieving zero memories** | **0** |
| missing usage records | 0 |

Every question retrieved the full 40 memories — `retrieved_memory_count` has
min = median = max = 40 — so no question was answered from an empty context.

### The 4 judge-parse failures, and why the headline is the conservative one

All four failed identically: `trailing characters at line 3 column 1`. The
judge emitted valid JSON followed by extra prose, and the parser rejected the
whole response. The harness records a parse failure as **not correct**.

**In three of the four, the truncated error text shows the judge's own verdict
was `"correct": true`.** So the recorded figure is biased *down*. Three
readings of the same run:

| reading | accuracy |
|---|---:|
| **935/1438 — parse failures counted not-correct (recorded, headline)** | **65.02%** |
| 935/1434 — parse failures excluded from the denominator (the report JSON's own `overall_accuracy`) | 65.20% |
| 938/1438 — the three visible `"correct": true` verdicts honoured | 65.23% |

The spread is 0.21pp, an order of magnitude inside the ±2.23pp interval, and
changes no conclusion. **We publish the conservative one and do not re-score** —
re-scoring after seeing the result is exactly the move the prereg forbids.

The parser's intolerance of trailing content after valid JSON is a real harness
defect. It is **not fixed here** (fixing it and re-running would be a re-roll);
it is opened as a register row for a later run.

> **Follow-up 2026-08-08: R21 is now fixed** — `judge::first_json_object` takes
> the first *balanced* object instead of the first-`{`-to-last-`}` span.
> **The 65.02% above was NOT re-scored and will not be.** Any future run is
> using a different scorer than this one did, and must say so when compared
> against this number.

## Cost and latency

| | |
|---|---:|
| total spend | **$17.38** |
| per question | $0.01208 |
| actor / judge split | $13.42 / $3.96 |
| preregistered estimate | $18.26 — **within 5%** |
| wall clock | 10,694 s (2 h 58 m), single-threaded |
| mean context tokens | 2,841 (p95 3,221) |
| **retrieval wall time** | **mean 1.03 ms, p95 2.0 ms** |

Retrieval costs ~1 ms and $0. The $17.38 is entirely the reader — the actor and
judge model calls — which is the point of the configuration: the memory layer
made no model calls at all.

**Cost-estimator defect found during this run, disclosed.** The binary's
`--confirm-cost` pre-flight printed **$115.04**. That estimator is a flat
`$0.04 per call × 2 calls` constant (`eval.rs:97 model_cost_per_call`), roughly
**6.5× conservative** for this workload. The first launch was killed after 7
questions (~$0.09) purely to measure the true per-question cost against the
prereg's $0.0127. It confirmed the prereg, and the run was restarted from a
clean work directory. **Same configuration, nothing tuned** — this was a budget
check, not a re-roll. The aborted seven questions are retained at
`aborted-costcheck-work/` and are not part of the scored set.

## What this claims, and what it does not

**Claims:**
- A memory layer doing **zero model inference at read or write time** scores
  **65.02% [62.79, 67.25]** on the full LoCoMo answerable set (1,438 questions,
  no sampling), read by `claude-sonnet-4-6`.
- That same layer achieves **95.06% session recall** at ~1 ms and $0 per query,
  and retrieves at least one evidence session for 98.9% of questions.
- Session recall differs by only 5.94pp between questions judged correct and
  incorrect (99.21% vs 93.26%), so on this benchmark **retrieval is not the
  binding constraint**.
- No published BM25-only end-to-end LoCoMo baseline existed before this one.
  The original paper used DRAGON dense retrieval; the vendors compare against
  each other. The field has been measuring memory systems with **no lexical
  floor** to say what the machinery is worth.

**Does not claim:**
- Anything about how Spectral's full configuration performs. This run
  deliberately disables cascade, shape routing, wings, the constellation tier
  and recognition.
- Any comparison to any vendor number (caveat 3).
- That this generalizes past LoCoMo. It is one dataset with a ~6.4%-wrong
  answer key and a lenient judge.
- Any statement about which is better, a lexical floor or a model-driven memory
  layer. It reports where the floor is. Interpreting the gap is a separate
  argument that needs its own measurement.

## Reproducing this

```bash
~/spectral-local-bench/bm25-locomo-baseline-2026-08-07/launch.sh   # the run (~3 h, ~$17)

python3 scripts/analyze_locomo_baseline.py \
  --report  ~/spectral-local-bench/bm25-locomo-baseline-2026-08-07/bm25-locomo-baseline.json \
  --dataset ~/spectral-local-bench/locomo_full_answerable.json \
  --iters 20000
```

The headline interval is analytic and does not depend on `--iters`. The
percentile-bootstrap row does, mildly: 20,000 iterations gives ±1.65pp,
2,000 gives ±1.63pp. The figures in this document use `--iters 20000`.

`scripts/analyze_locomo_baseline.py` produces **every** number in this
document — all three intervals, per-category, per-conversation, session recall,
the judge-strictness counts, hygiene and cost — from the report JSON plus the
dataset. Nothing here is reproducible only from a session transcript. Its
bootstrap is seeded (`--seed 20260807`), so the intervals reproduce exactly.

**Committed to the repo, deliberately.** R16's analyzer lived only in the bench
directory, and the 88.5% evidence-turn figure was for a while reproducible only
from a transcript. This one is version-controlled with the claim it supports.

Artifacts kept at `~/spectral-local-bench/bm25-locomo-baseline-2026-08-07/`:
`bm25-locomo-baseline.json` (the full 1,438-row report), `launch.sh`,
`run.log`, `work/checkpoint.json`, and `aborted-costcheck-work/` (the 7-question
budget check).

**Publication status:** in-repo. Taking this to an external venue is Jesse's
explicit call and has not been made.

**Refs:** `bm25-locomo-baseline-prereg-2026-08-07.md` (the prereg),
`landscape-research-2026-08-07.md` (caveat sources),
`turn-level-evidence-recall-2026-08-07.md` (R15, why evidence-turn recall is
`n/a` here), `REPAIR_REGISTER.md` R19.
