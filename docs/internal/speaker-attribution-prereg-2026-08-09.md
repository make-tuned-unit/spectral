# R23 — speaker attribution · PREREGISTRATION (written before any arm was run)

**$0. Retrieval-only oracle, LoCoMo, 250 questions, `--retrieval-path
topk_fts`, k=40, R19 turn labels. No model calls.** Written and committed
before the first arm executed. **Requires re-ingest** (this is the first
ingest-affecting lever in this series).

## The target, measured first

`speaker-attribution-diagnostic-2026-08-09.md` established, from the R22 A0 arm
and with no new retrieval:

- **62.9%** of missed evidence turns in zero-evidence questions share **no**
  content word with the question.
- **100% of those (44/44)** name a proper noun in the question that never
  appears in the evidence turn. Zero are predicate paraphrase.
- Among such questions: the missed evidence contains the queried name **4.3%**
  of the time; the retrieved top-40 contains it **36.6%**. An **8.5×
  inversion**.

The failure is that a question names a person and the answer is that person's
own first-person turn, which never says their name — while BM25 spends its
budget on turns where *someone else* addresses them.

## This is metadata restoration, not inference — which is why it is legitimate

The obvious objection is that binding names to turns means inferring who spoke,
and inferring it from evidence turns would be fitting. That objection does not
apply here, and the reason is checkable:

- Raw LoCoMo (`locomo10.json`) carries **`speaker_a` / `speaker_b` per
  conversation and a `speaker` field on every turn**.
- **272/272 sessions have strictly alternating speakers**, so speaker identity
  is exactly recoverable from turn parity.
- Our converted dataset kept only `role` (user/assistant), which is a faithful
  1:1 proxy for speaker within a conversation. **The converter dropped the
  names; the corpus always had them.**

So this restores corpus metadata that exists independently of the answers. It
also mirrors production exactly: **Permagent has speaker identity as metadata**,
not as something to infer. Nothing here is derived from a question, an answer,
or an evidence label.

## Arms — fixed before running

Identical brains per arm within an ingest condition; single variable per arm.
Arms B and C require their own ingest (content changes), so the brain sets
differ between ingest conditions by construction — that is stated here rather
than discovered later.

| arm | change | ingest |
|---|---|---|
| **A0'** | baseline, re-ingested unchanged | fresh |
| **B** | **PRIMARY** — prefix each turn's indexed content with its speaker name (`"Caroline: <text>"`) | fresh |
| **C** | speaker name added as a separate indexed field, not inline in content | fresh |

**A0' is a precondition arm, not decoration.** It must reproduce A0's
231/356 / 53 zero-evidence. A re-ingest that moves the baseline on its own
would invalidate B and C, and R16 has already shown this corpus is sensitive to
ordering changes.

## Primary metric and decision rule — fixed before running

**Primary:** evidence-turn micro-recall. Baseline **231/356 = 64.89%**.
**Primary comparison:** B vs A0'.
**Statistic:** exact two-sided McNemar on the paired per-question
`all evidence turns retrieved` indicator (`scripts/analyze_rrf_arms.py`).

**PASS** requires *both*: p < 0.05 two-sided, **and** micro-recall increase
≥ **+2.0pp** (≥ +8 evidence turns). Anything else is **NULL**; a significant
decrease is **REFUTED** and published with equal prominence.

**Secondary, reported but not decisive:** zero-evidence count (53 baseline),
the multi-session slice (44.70% — the weakest and the one every RRF arm made
worse), context tokens, and — as the mechanism check — whether the
**name-containment inversion closes**: the 4.3% / 36.6% figures recomputed on
arm B.

## The risk, stated in advance

**Dilution.** Prefixing every turn with its speaker's name makes a query naming
Caroline match **every turn Caroline ever spoke** — roughly half the corpus.
The name stops being a high-IDF discriminator and becomes close to a stopword
within that conversation. That could easily cost more than the 62.9% it
targets, and it is the direct analogue of the failure that killed RRF: giving a
signal more reach than its precision supports.

Arm C exists because of this: a separate field lets the matcher use the name
without dumping it into the same lexical channel as content.

**A decrease is a prespecified outcome, not a measurement failure.**

## What would make this uninterpretable

- **A0' failing to reproduce A0** (231/356 micro, 53 zero-evidence). If the
  precondition fails, the run is void and nothing is claimed.
- Comparing B or C against the *old* A0 rather than the re-ingested A0'.
  Different ingest, different brains — only A0' is a valid control.

## Registered non-goals

- **No paid runs.** No end-to-end actor arm regardless of outcome; if retrieval
  moves, the accuracy claim stays unmade and queued.
- **No cascade measurement**, and therefore no cascade change — same limit as
  R22.
- No tuning of the prefix format after seeing results. Two arms, fixed above.
- The LoCoMo `speaker` field is used **only** to build the name↔role binding.
  It is never consulted per question, and never at query time.

## Honest limit, acknowledged up front

LoCoMo is the **harder** case than production: we must attach names our
converter dropped, whereas Permagent already holds speaker identity as
metadata. A LoCoMo result therefore **understates** the production case rather
than overstating it — but it is also a two-speaker corpus, where "half the
turns" is the worst possible dilution ratio. A many-speaker corpus would dilute
less. Neither of those is a reason to discount a negative result.

**Register row:** R23. **Refs:**
`speaker-attribution-diagnostic-2026-08-09.md` (the target),
`rrf-composition-result-2026-08-09.md` (R22, which queued this),
`failure-analysis-2026-08-08.md` §3.
