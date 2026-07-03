# Permagent-realistic benchmark — spec

**Date:** 2026-07-03
**Status:** proposal, not yet run
**Why this exists:** every Spectral number to date is F1/AUC on LongMemEval — a
QA benchmark that is (a) synthesis-bound (the LLM actor, not retrieval, is the
ceiling) and (b) not Permagent's job. This spec measures Spectral on its
*actual* job, against the *real* alternatives, with **cost, determinism, and
auditability as first-class metrics** — because on accuracy alone Spectral ties
or loses to the boring stack, and the only honest way to know if it's worth
building is to measure the whole picture.

## 0. The one question this must answer

> Given Permagent's real workload and constraints, does Spectral beat
> `pgvector + a strong embedding model + MinHash` — the stack Jesse could have
> assembled off the shelf — on a metric that matters to Permagent?

If the answer is no, we stop building Spectral and use the boring stack. This
doc is designed to be able to return "no."

## 1. What Permagent actually asks of a memory layer

From the real deployment (`~/.permagent/brain/memory.db`, 151 MB, ~1738
memories over ~48 days — live DB, figures drift; `~/.permagent/spectral/permagent.db`
with 170 recognition_events + 6789 recognition_set_members). Permagent is a "coworker
brain": it ingests an ambient activity stream and feeds relevant context back to
the agent proactively, organized by project. So the real jobs are:

1. **Ingest cheaply and continuously** — an activity stream, on-device, at ~$0
   marginal cost. (Not: batch-embed a static corpus once.)
2. **Recall for proactivity** — given what the user is doing now, surface the
   memories that help. Precision matters more than a QA answer.
3. **Scope by project** — recall stays inside the relevant project/wing.
4. **Handle updates** — return the *current* fact when it has changed.
5. **Do 1–4 offline, reproducibly, and auditably** (explain *why* a memory
   surfaced).

Note: none of these is "answer a trivia question about a conversation." That is
why LongMemEval was the wrong target.

## 2. Systems under test (no strawmen — the fairness lesson)

The recognition post-mortem taught us: a weak baseline produces a flattering,
worthless result. So each competitor is configured to its documented best, by
someone who wants it to win.

| system | what it is | how it competes |
|---|---|---|
| **Spectral** | this library (FTS5 + cascade + wings + graph + recognition) | the subject |
| **pgvector + strong embedding** | Postgres + a *current* top embedding model (e.g. a 2025 large embedder or `text-embedding-3-large`), top-k + metadata filter | the "boring stack" Jesse could have built |
| **Mem0** | OSS agent-memory layer (extraction + vector + graph) | direct product competitor |
| **Zep / Graphiti** | temporal knowledge-graph memory | the temporal/update competitor |
| **MinHash + BM25 (local)** | the classical deterministic stack, no model | Spectral's real deterministic rival — beats its recognition engine already |
| **long-context baseline** | stuff the whole recent history in the prompt, no retrieval | the "do you even need a memory layer" control |

Losers must be reported as prominently as winners. If MinHash+BM25 (which needs
no Spectral) matches Spectral, that is the headline.

## 3. Tasks (grounded in real data, not synthetic)

### T1 — Ingestion cost & throughput  *(Phase 0, $0 for Spectral/classical)*
Ingest the real activity stream (the ~243 raw `compaction_tier` ambient memories,
plus the full ~1738-memory brain as a bulk load). Measure per system:
- wall-clock to ingest N events; events/sec sustained.
- **$ cost** — embedding API calls (pgvector/Mem0/Zep) vs $0 (Spectral/classical).
  Report $/1k events and projected $/month at Permagent's real ingest rate.
- storage bytes/event; peak RAM.
- **on-device feasible?** (binary + hardware).

### T2 — Recall for proactivity  *(Phase 1, needs labels)*
Replay the **~431 real unique queries** (744 events) from `retrieval_events`. Two ground-truth
sources, kept separate:
- **(a) agreement** — overlap of each system's top-k with what Permagent's
  production cascade actually retrieved (`recognition_set_members`). Weak signal
  (it rewards mimicking Spectral) — reported but not decisive.
- **(b) human-labeled relevance** — hand-judge relevance for a stratified **100
  query × top-10** sample (1000 judgments; ~1 day of labeling). This is the real
  ground truth. Metric: **precision@5, nDCG@10**.

Report accuracy **alongside** the T1 cost for the same run — the deliverable is
an accuracy-vs-cost scatter, not an accuracy number in isolation.

### T3 — Project scoping  *(Phase 1)*
Using the **~129 events with a real `rc_focus_wing`**, measure wing-precision:
for a project-scoped query, what fraction of top-k belongs to the correct
project? Spectral has native wings; competitors use metadata filters. Fair game —
whoever organizes better wins.

### T4 — Update handling  *(Phase 1)*
Curate ~30 real update chains from the brain where a fact changed (Permagent's
`knowledge-update` category exists in the data). Query for the current value;
score returns the *latest* fact, not a stale one. This is Zep's home turf — if
Spectral loses here, say so.

### T5 — Determinism & auditability  *(Phase 0, $0)*
- **Determinism:** run each query twice; fraction byte-identical. Spectral/
  classical → 1.0 expected; embedding-kNN → measure (ANN indexes drift).
- **Auditability:** can the system name *why* a memory surfaced (matched term/
  feature/edge)? Score 0–2 per system with concrete examples. Not a number
  either wins on accuracy, but a real product property Permagent may need.

## 4. Metrics — accuracy is one axis of five

The core deliverable is a **radar/table across five co-equal axes**, not a single
score:

1. **Recall quality** — precision@5 / nDCG@10 (T2), wing-precision (T3),
   update-correctness (T4).
2. **Cost** — $/1k ingested events + $/1k queries (T1, T2). *This is where
   Spectral's real claim lives.*
3. **Latency** — p50/p95 ingest & query, on the target device.
4. **Determinism** — reproducibility rate (T5).
5. **Auditability** — 0–2 rubric (T5).

Headline artifact: **accuracy-vs-cost scatter** with every system plotted.
Spectral's thesis is a *point on that plot* ("parity recall at ~1/100th the
cost, fully offline"), not a peak on any single axis.

## 5. Fairness rules (baked in from this session's mistakes)

1. **Strongest reasonable baseline** for every competitor. No BGE-small-on-a-
   lexical-task repeats. Use a current top embedder; configure Mem0/Zep per docs.
2. **Pre-register** expected results and **kill criteria** (below) *before*
   running. Commit them.
3. **No favorable-by-construction tasks.** T1–T5 come from real usage, not from
   "what Spectral is good at." If a task only Spectral can do, it is context, not
   score.
4. **Report losses first.** The doc's exec summary leads with where Spectral is
   beaten.
5. **Held-out.** Label the relevance set (T2b) blind to which system produced
   which result.

## 6. Kill criteria (pre-registered — what makes us drop Spectral)

Drop Spectral for the boring stack if **any** of:
- pgvector+strong-embedding matches Spectral on T2 recall (Δ nDCG@10 < 0.03)
  **and** Permagent's real query volume makes the cost difference < \$5/month.
- MinHash+BM25 (no Spectral needed) matches Spectral on T2/T3.
- Spectral's only wins are on determinism/auditability **and** Permagent's
  product doesn't actually need either.

Validate Spectral (keep building) if:
- It holds T2 recall parity (Δ nDCG@10 ≥ −0.03) at **≥10× lower cost** than the
  best embedding stack, fully offline, with auditable retrieval, **and** that
  cost/offline/audit gap is load-bearing for Permagent at real volume.

## 7. The ground-truth problem (honest caveat)

The real `recognition_events` have **no negative outcomes** (all 149 Positive)
and no per-item relevance labels — they can't score discrimination on their own.
So T2's real signal requires the ~1000 hand judgments (T2b). Without them we can
only report cost/latency/determinism/auditability (Phases 0), which are still
decisive if Spectral's *accuracy* turns out to tie — because then the whole case
rests on cost/offline/audit, which Phase 0 measures at \$0.

## 8. Phasing & effort

- **Phase 0 (T1, T5) — ~1 day, \$0 (Spectral/classical) + minor API \$ for the
  embed-stack cost measurement.** Establishes the cost/determinism/auditability
  profile. If Spectral's cost/offline/audit edge is *not* real here, stop — the
  thesis is dead before spending on labels.
- **Phase 1 (T2b labels + T2/T3/T4) — ~3–4 days** incl. adapter code for
  pgvector/Mem0/Zep and the 1000 relevance judgments. The real accuracy verdict.
- Total: ~1 week + modest API spend, most of it in competitor adapters and
  labeling, not Spectral.

## 9. What this deliberately does NOT test

Raw QA F1 (synthesis-bound, already done), recognition AUC (settled — classical
methods win; see `RECOGNITION_BASELINE.md`), federation, and multi-device sync.
Those are either resolved or not on Permagent's critical path today.

---

**Bottom line:** run Phase 0 first. It is cheap and it tests the *only* claim
Spectral has left standing — cheapest-at-parity, offline, auditable. If Phase 0
shows that edge is real and load-bearing, Phase 1 tells you whether the accuracy
holds. If Phase 0 shows the edge doesn't matter at Permagent's real volume, you
have your answer without labeling a thing: use the boring stack.
