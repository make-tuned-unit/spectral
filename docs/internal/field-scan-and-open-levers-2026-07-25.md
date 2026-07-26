# Field scan + remaining open levers — 2026-07-25

Two questions: what has the agent-memory field learned recently that Spectral
should act on, and what have we not yet evaluated internally?

## Part 1 — What the field says (July 2026)

### Spectral's core bets are now externally validated

**MemDelta, "Controlled Baselines and Hidden Confounds in Agent Memory
Evaluation"** (arXiv 2606.29914) argues that memory systems routinely claim
improvements without controlling against simple retrieval baselines, and
recommends BM25/lexical search as the mandatory control before crediting a
sophisticated architecture. It also finds that basic retrieval quality — not
memory-synthesis complexity — is frequently the limiting factor.

This is the exact pattern this repo arrived at independently and expensively:
~20 retrieval levers measured and rejected against a strong BM25 baseline.
What has read internally like an idiosyncratic dead end is a published,
generalised finding. The rejection log is an asset, not an embarrassment.

**LongMemEval-V2** (arXiv 2605.12493) reports that systems handle isolated
fact-level retrieval adequately but degrade on session-level coherence,
integrating facts across sessions, and distinguishing current from outdated
information. Spectral's own campaign concluded the same thing — "actor
synthesis is the bottleneck, not retrieval" — from the multi-session failure
triage (9 ACTOR_MISS vs 1 RETRIEVAL_MISS).

**BEAM** scales to 10M tokens across 100 procedurally generated conversations
and tests 10 memory dimensions. Its headline: a 10M context window does *not*
remove the need for a memory system. That supports Spectral's premise directly.

**Privacy is still application-level everywhere else.** The 2026 state-of-memory
review lists consent, retention, and deletion as unsolved and pushed onto
applications. Spectral already has visibility boundaries enforced in the
retrieval path, fail-closed deletion verification (`VerificationStatus`),
tombstones, and right-to-be-forgotten cache invalidation — *in the library*.
This is a genuine differentiator and is currently undersold.

### Where the field has moved and Spectral has not

| finding | Spectral status |
|---|---|
| Winning architecture fuses **three** signals — semantic + BM25 + entity, normalised into one score — and beats any single signal | Spectral has BM25 + entity + RRF fusion of stemmed/unstemmed. Dense was measured near-null, but *on LongMemEval, a lexical-regime corpus*. The published result is that fusion wins on LoCoMo/BEAM. Our null may be corpus-specific rather than general. |
| **Memory staleness** — "high-relevance facts become confidently wrong over time" — listed as a top-3 open problem | Spectral has `consolidation_edges` and supersede/collapse machinery, and recall already filters superseded rows. But nothing *detects* staleness; it is entirely caller-driven and effectively idle. |
| **Contradiction resolution** is a first-class BEAM dimension | Not tested by Spectral's harness at all. |
| **Abstention** is a first-class BEAM dimension | `CascadeResult` carries `max_confidence`; no abstention signal is derived or exposed. |
| Temporal reasoning gave the single biggest measured gain (+29.6 points) | Spectral has episodes, `created_at`, temporal-specificity dimensions — but its temporal work was never targeted at this. |
| Cross-session evolution: "systems treat change as replacement, not evolution" | Spectral's supersede is exactly replacement. |

**Highest-value match: staleness / contradiction resolution.** Spectral already
owns the mechanism and does not use it, and the field now has a benchmark
dimension for it. That is a rare combination — capability present, evaluation
available, gap unmeasured.

**Recommended next benchmark adoption:** BEAM, for contradiction resolution,
event ordering, and abstention. LongMemEval-S is a lexical-regime corpus that
this repo has arguably saturated; continuing to tune against it is what produced
20 nulls.

## Part 2 — Internal levers not previously evaluated

### MEASURED NOW: recall throughput does not scale with concurrency

`SqliteStore` serialises every operation through one `Arc<Mutex<Connection>>`,
including reads, although WAL permits concurrent readers.

| threads | recalls/sec | speedup | efficiency |
|---:|---:|---:|---:|
| 1 | 366 | 1.00x | 100% |
| 2 | 564 | 1.54x | 77% |
| 4 | 564 | 1.54x | 39% |
| 8 | 562 | 1.53x | 19% |

Throughput is **hard-capped at ~564 recalls/sec** no matter how many threads
ask. Wall time scales linearly with thread count past 2, which is the signature
of a serialising lock rather than CPU saturation.

For a single-agent local brain this is irrelevant. For a server process fanning
out concurrent recalls it is a hard ceiling, and it is the largest remaining
structural lever. Fix shape: a small pool of read-only connections for the read
methods, leaving writes on the existing single connection. Not a small change —
the read methods are numerous — so it wants explicit sign-off rather than being
bundled into a performance sweep.

### Still open, in rough value order

1. **Read-connection pool** — the ~564/sec ceiling above.
2. **Recognition enrolment growth** — 3.0x over 800 writes; `index_minhash`
   writes ~180 inverted-index rows per memory. An LSH-banding path already
   exists in `minhash` but trades against recognition recall.
3. **`remember_batch`** — no batch write API exists; each `remember` takes its
   own transaction plus ~6 separate derived-write round trips.
4. **`Brain::open`** at 10–16 ms, growing mildly with corpus. Irrelevant for a
   long-lived process; matters for short-lived CLI invocations.
5. **Bench-only**: `bench-accuracy/src/retrieval.rs` has ~11 per-call
   `Regex::new` in shape routing — distorts harness speed, not library speed.

### Evaluated and rejected (do not re-chase)

Recorded in `read-path-regex-cache-2026-07-25.md`: missing indexes (plan already
optimal), `prepare_cached` (0.22% of a recall), read-path scaling with corpus
size (1.3x over 8x), `async_writeback` as default (durability trade), and
bypassing TACT tiers (regressed multi-session key recall 48.6% → 46.0%).
