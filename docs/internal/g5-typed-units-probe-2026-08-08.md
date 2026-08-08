# G5 — bounded typed memory units · premise REFUTED on the read path (2026-08-08)

**$0, offline, no model calls.** Computed from the published baseline's own
retrieved keys and the R19-labelled dataset.

G5 was the only untested **axis** rather than another lever on the same axis:
Memobase-style `(topic, sub_topic) → content` slots, bounded in count, with a
read path of *recency-ordered SQL plus deterministic greedy token-fill*. Its
claim is strong and worth taking seriously — **retrieval as a problem
disappears**, and cost moves to write time.

The claim has two halves. One is cheap to test and is refuted. The other is
expensive, untested here, and independently disfavoured by the project's own
record.

## Half one — the read path. Refuted at equal budget.

If a recency-ordered token-fill suffices, then retrieval is buying nothing and
the ranking work of the last several months was misdirected. That is directly
measurable: take the newest N turns, use the query for nothing at all, and
score evidence-turn recall against the same labels the baseline was scored on.

| read path | ev-turn recall | zero-evidence questions | ~context tokens |
|---|---:|---:|---:|
| recency-only, newest 40 turns | **5.47%** (117/2140) | 1328/1436 (92.5%) | 1,171 |
| recency-only, newest 80 turns | 12.10% (259/2140) | 1208/1436 (84.1%) | 2,343 |
| recency-only, newest 160 turns | 24.67% (528/2140) | 984/1436 (68.5%) | 4,747 |
| **BM25 top-40 (shipped)** | **59.86%** (1281/2140) | **357/1436 (24.9%)** | **2,841** |

At a comparable budget, **BM25 retrieval is worth about 11× a recency-only read
path** (59.86% vs 5.47%). Even at 1.7× the budget, recency reaches 24.67% —
under half of what retrieval delivers for less.

The corpus explains why: **602 turns per question on average** (max 689). Forty
turns is 6.6% of it, and LoCoMo scatters evidence across sessions by
construction. Recency selects a window that is uncorrelated with where the
answer is.

**This is the strongest positive evidence the project has that its retrieval
layer earns its place.** Today's other findings sharpen rather than soften it:
R19 showed retrieval is the *binding constraint* (57pp separation between right
and wrong answers), and G4 showed the incremental ranking levers are exhausted.
Retrieval matters enormously, the current retriever captures most of the
available value, and the remaining ~30pp of headroom (59.86% → 89.7% at k=500)
has not been reached by any lever tried.

## Half two — the unit. Not tested, and doubly disfavoured.

The probe above uses **raw turns**. G5 proper changes the *unit*: extract
bounded typed slots at write time so a recency fill is dense with facts rather
than dense with conversation. This measurement does **not** refute that, and
saying otherwise would overclaim.

Two independent reasons it is nonetheless a poor bet, both already in the
record and neither established here:

1. **Extraction is LLM consolidation, which is measured destructive.** It made
   a frontier model fail **54% of problems it had previously solved**, *even
   consolidating from ground truth*; append-only doubled accuracy (arXiv
   2605.12978). That is the core loop of Mem0/LangMem and most commercial
   products.
2. **Extracted artifacts lose exactly what our one validated lever depends on.**
   Timestamp-marked verbatim chunks score **50.2%** on temporal questions
   against **31.2%** for extracted artifacts, and summarised representations
   score **7.5%** (arXiv 2601.00821). R11 — the project's only preregistered,
   held-out accuracy win — was **+14.2pp from bare dates on verbatim turns**.
   A typed-slot unit is the representation that measurement says to avoid.

And a third, specific to this project: write-time extraction requires model
inference at write time, which contradicts the zero-inference property the
whole baseline was built to measure.

## Verdict

- **Read-path half: refuted.** Recency-ordered fill is 11× worse than BM25 at
  equal budget on this corpus. Retrieval is not a problem that disappears.
- **Unit half: untested here**, and disfavoured by two prior measurements plus
  the thesis constraint. Testing it properly means paying for extraction and
  accepting a representation the record says is worse — a large spend against a
  hostile prior.
- **The axis is not closed by this**, but it is no longer the obvious
  unexplored direction it looked like when the retrieval nulls were thought to
  be exhaustive. R19 changed that picture: the nulls were measured on a diluted
  metric, and there is real headroom inside the retrieval axis we already have.

## Limits

One corpus (LoCoMo), one budget family, retrieval-side only — no actor arm.
The recency baseline is the *simplest* form of the G5 read path; a smarter
bounded-slot read path (e.g. type-stratified fill) is untested. What is
established is narrow and sufficient: **on this corpus, using the query beats
not using it, by a wide margin, at equal cost.**

**Refs:** `landscape-research-2026-08-07.md` §G5,
`r19-locomo-turn-labels-2026-08-08.md`, `g4-proximity-result-2026-08-08.md`,
`MEASURED_RECORD.md` (R11, the consolidation and summarisation findings).
