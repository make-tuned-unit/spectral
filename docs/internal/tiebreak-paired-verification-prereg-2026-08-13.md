# R17/R18 tiebreak sweep — paired verification prereg (2026-08-13)

**$0, retrieval-only oracle, no model calls. Registered before any arm runs.**

## What changed

Deterministic final sort keys added at the R17 site and all twelve R18 sites in
`crates/spectral-ingest/src/sqlite_store.rs` — every remaining product
`ORDER BY … LIMIT` without a unique final key, including the
`prune_wing_keeping_recent_per_source` DELETE boundary (the row the register
says to do first). Conventions follow the already-tiebroken sites: `id DESC`
after `datetime(created_at) DESC` (matches `list_wing_memories_capped` and the
`idx_memories_wing_recency` index order), plain `id`/`m.id`/`memory_id`
ascending after score-shaped keys (matches R16).

Pinned by three new tests covering the three severity classes of the family
(DELETE boundary; guaranteed tie block on default `signal_score = 0.5`;
LIMIT-1 single pick), each run in BOTH insertion orders on independent stores,
verified to FAIL with the clauses reverted. One finding from that verification
is recorded here because it changes how these tests must be written:

> **Tied-key emission order differs by plan on this build.** An index scan
> emits ties by the index's own trailing key (`idx_memories_wing_recency`
> already ends in `id DESC`, so the untiebroken prune subquery happened to
> agree with the pinned order), while a temp sort emits insertion order. A
> single adversarial arrangement can therefore coincide with the pinned
> winner — the R16 test construction is not sufficient for sites the planner
> serves from an index. The prune test exercises both plans (index present and
> dropped).

## Why a paired run at all

Most changed sites are maintenance/telemetry paths, but two are plausibly
reachable from the bench pipeline:

1. `find_recent_episode` is called during **ingest** (episode assignment,
   `ingest.rs`), so an `ended_at` tie could move a memory's `episode_id`,
   which the cascade's episode layer can see.
2. `fingerprint_search` feeds TACT tier-1. On wing-less bench corpora it
   should never fire, but "should never" is an assumption, not a measurement.

## Design

Paired A/B, single variable = the tiebreak clauses. Two binaries from the same
tree (baseline = `b3375e8`, arm = working tree), same regenerated dataset, same
flags, `--fresh-brains` both sides, expansion-free oracle path.

- Corpus: LoCoMo full answerable labelled, regenerated on this machine with
  `locomo_to_oracle.py --all` — **1438 questions / 2140 evidence turns, matching
  R19's published figures exactly** (membership gate satisfied by count and
  category split; the original bytes are not on this machine to diff).
- Arms: `base_topk` vs `tie_topk` (`--retrieval-path topk_fts`), `base_casc`
  vs `tie_casc` (`--retrieval-path cascade`), N = 400 each (first 400
  questions, identical across arms by construction).
- Metric: per-question `context_hash` diff count, plus evidence-turn recall
  and token deltas from the oracle rows. Any nonzero diff is characterized
  (pure reorder vs set change) before anything else is claimed.
- Escalation rule, fixed now: if any arm pair shows >0 context diffs at N=400,
  the pair re-runs at full N=1438 to size the shift. If 0 diffs, the result is
  recorded as "no measurable bench-path effect at N=400" — NOT as proof the
  sites are unreachable.

## What this is not

No PASS/FAIL gate and no accuracy claim: "which row survives a tie" has no
ground truth to win against. This is a baseline-shift quantification in the
R16 mold. The determinism rationale stands independent of the outcome: the
byte-identical invariant must not rest on SQLite's plan choice, and R18's
DELETE site decides what is *destroyed*.

## Environment caveat

Different host than every prior run in the register (previous sessions ran
under `/Users/jessesharratt`; this machine is an Apple M4 Mac mini (16 GB; the shell reports x86_64 under Rosetta, which earlier misled this doc) under `/Users/j`).
Paired arms are internally valid (both binaries, one machine); cross-session
absolute comparisons are not, and none will be made.
