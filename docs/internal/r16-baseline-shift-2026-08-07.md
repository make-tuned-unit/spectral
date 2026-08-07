# R16 — SQL tiebreak on the default FTS path · baseline shift (2026-08-07)

**What this is:** a determinism repair with a measured, non-zero effect on
default output. **No accuracy claim is made and none is implied.** Under
Rule 2 it needs no prereg (instrument/determinism fix); under Rule 6 it is
recorded here as a baseline shift with an oracle diff, not as a null and
not as a win.

> **UNBLOCKED 2026-08-07 (later same day) — merged.** The blocker below asked
> whether the tiebreak *exposes* a pre-existing order-invariance violation or
> *introduces* one. Answered by experiment **on `main`, with no R16 change
> present**: the same 24 memories with the same timestamps, inserted in a
> shuffled rather than chronological order, drift across the same 5-year clock
> shift (`s22`/`s1` and `s21`/`s12` swap). The violation is R20's and predates
> R16; the old test passed only because chronological insertion made rowid
> order agree with age order. Re-baselined under R20's interim requirement —
> `recency_decay_is_order_invariant_in_the_topk_path` replaced by
> `topk_additive_recency_reorders_under_a_clock_shift` and
> `topk_ranking_is_independent_of_insertion_order`, both of which fail on
> `main` and pass here. Workspace suite: zero failures. R20 remains open.
> **Historical blocker text follows.**
>
> **MERGE BLOCKED 2026-08-07.** The implementation report claimed
> `suite_passed: true`. It is **false**:
> `crates/spectral/tests/deterministic_anchor.rs:83`
> `recency_decay_is_order_invariant_in_the_topk_path` fails with this change
> present and passes with the two `, m.id` clauses reverted (bisected and
> round-tripped, file restored byte-identical). The measurement below is
> independently re-derived and is **exactly** as reported — 10/500, the exact
> 10 ids, 9 reorder + 1 membership, every metric unmoved. The blocker is the
> red test, not the numbers. Root cause is a **pre-existing** defect this
> change exposed: the top-k path scores on FTS *rank position* with an
> **additive** recency term, so ranking is a function of the wall clock. Opened
> as **R20**; merge is blocked until the test is fixed or explicitly
> re-baselined with a recorded justification.
> Ref: `research-alignment-2026-08-07.md` §2, `REPAIR_REGISTER.md` R16/R20.

---

## The change

Two SQL sites in `crates/spectral-ingest/src/sqlite_store.rs`, one clause each.

**Site 1 — the shipping default path** (`fts_search`, non-fusion;
`fts_fusion` defaults `false`):

```sql
-  ORDER BY bm25(memories_fts, 1.0, 1.0, 0.5) LIMIT ?2
+  ORDER BY bm25(memories_fts, 1.0, 1.0, 0.5), m.id LIMIT ?2
```

**Site 2 — the fusion channel subqueries** (`ranked_ids`), which the
register row missed:

```sql
-  ORDER BY bm25({table}{weights}) LIMIT ?2
+  ORDER BY bm25({table}{weights}), m.id LIMIT ?2
```

Site 2 matters because each channel's *result position* is the RRF input
(`1/(60 + rank)`). The existing `.then(a.0.cmp(&b.0))` breaks ties in the
**fused** score only — one layer too late. Fixing site 1 alone would have
reproduced PR #238's failure mode (a partial sweep that looks complete)
one layer down.

### Why `m.id` and not something else

- `memories.id` is `TEXT PRIMARY KEY`, no `COLLATE` ⇒ BINARY ⇒ byte order.
- It is already selected: `MEMORY_COLUMNS` begins with `id` and the
  projection is `MEMORY_COLUMNS.replace(", ", ", m.")`. No projection change.
- It is `key_to_id(key) = format!("{:016x}", u64::from_be_bytes(blake3(key)[..8]))`
  (`crates/spectral-graph/src/brain.rs:4425`) — a pure function of the memory
  key. Fixed 16-char width, so lexicographic order == numeric order, and the
  resulting order **reproduces across independently-built brains**, not merely
  across repeat reads of one file.
- Rejected alternatives: `rowid` (insertion order — differs between two brains
  built from the same data), `created_at` (low-resolution, ties itself),
  `m.key` (wider, slower, and disagrees with the key the fusion path already
  tiebreaks on).

### Why the fix is justified

**Not because ties are large.** The rationale previously recorded in the
register and in `landscape-research-2026-08-07.md` §G0 — that FTS5's IDF
clamp (`1e-6`) makes common-term documents "collapse into one large tie
block" — is **empirically false on this corpus** and has been struck from
both documents. The clamp is real, but the tf/doclen factor still varies, so
a pure-`"the"` query returns ~2585 near-zero but *distinct* scores
(-2.105206e-06, -2.105152e-06, -2.103540e-06, …).

Measured tie distribution across 150 LongMemEval brains (exact float equality
on bm25, boundary at K):

| query form | brains whose LIMIT boundary sits inside a tie block | median block | max |
|---|---:|---:|---:|
| full question text | **0 / 120** | — | — |
| 1 term | 36 / 150 (24%) | 2 | 5 |
| 3 terms | 12 / 150 (8%) | 2 | — |
| 5 terms | 5 / 150 (3.3%) | 2 | — |
| common terms only | 0 / 40 | — | — |

Ties are **rare and small**. The 10/500 shift measured below is consistent
with that, not with large tie blocks.

The justification that survives is the one that does not depend on tie size:
**the byte-identical invariant must not rest on SQLite's plan choice.** #238
added memory-id tiebreaks to all five sorts in `ranking.rs` and missed the
SQL beneath them, so until today "deterministic retrieval" rested on an
external guarantee that a schema change, an `ANALYZE`, or a SQLite upgrade
can move. It now rests on a total order in our own SQL.

---

## The measurement

$0. Both arms reuse the 500 existing brains at
`~/spectral-local-bench/oracle-work` — **no re-ingest**. Defaults: shape
routing, `per_turn`, k=40.

**Run on a clean detached worktree at the merge commit `17e0838`**, not on a
dirty tree. This was required: an earlier pass measured the same 10/500 on a
working tree carrying 35+ modified files, and a second crate was under
concurrent edit in this repo at the time of writing. The worktree isolates
the diff to exactly the two-line SQL change.

```bash
git worktree add --detach <tmp>/r16wt HEAD          # 17e0838
cd <tmp>/r16wt && cargo build --release -p spectral-bench-accuracy \
    --bin spectral-bench-accuracy
./target/release/spectral-bench-accuracy oracle \
  --dataset ~/spectral-local-bench/longmemeval/longmemeval_s.json \
  --work-dir ~/spectral-local-bench/oracle-work \
  --output <arm>.jsonl --label <arm>
# then apply the two `, m.id` clauses, rebuild, re-run, and:
./target/release/spectral-bench-accuracy oracle-diff \
  --baseline pre.jsonl --candidate post.jsonl
```

### Preconditions established before trusting the diff

1. **Pre-arm reproduces the published record byte-identically**: 0/500
   `context_hash` diffs vs `~/spectral-local-bench/r12-baseline.jsonl`.
2. **Pre-arm is self-reproducible**: two consecutive runs of the same binary,
   0/500 `context_hash` diffs. R16 was latent, exactly as the register said —
   SQLite's plan is stable *today*, which is the point: nothing we control
   made it so.

### Result

| metric | pre | post | delta |
|---|---|---|---|
| **contexts changed** | — | — | **10 / 500 (2.0%)** |
| session-recall (**macro**, mean of per-question ratios) | 98.03% | 98.03% | 0 |
| session-recall (**micro**, pooled) | 97.78% | 97.78% | 0 |
| key-recall (**macro**) | 55.51% | 55.51% | 0 |
| key-recall (**micro**, pooled) | 53.02% | 53.02% | 0 |
| net answer-keys delta | — | — | +0 |
| zero-answer-key questions | 2 | 2 | 0 |
| session-recall improved / regressed | — | — | 0 / 0 |
| mean context tokens | 14212.8 | 14212.8 | +0 |
| **evidence-turn recall, micro** (R15's `has_answer`) | 793/896 = 88.50% | 793/896 = 88.50% | **0** |
| evidence-turn recall, macro | 90.54% | 90.54% | 0 |
| zero-evidence questions | 27/479 | 27/479 | 0 |
| questions with ANY change in evidence-turn count | — | — | **0** |

`key-recall` is `answer_keys_*`, which R15 established is **evidence-session
turn coverage, diluted 12.2×** — it is reported here only to show it did not
move, and must not be cited as a retrieval-quality figure.

The 10 changed questions, **all on `TopkFts` (10/167); 0/333 `Cascade`**:

| question | category | change |
|---|---|---|
| `2ebe6c92` | temporal-reasoning | reorder-only |
| `8ebdbe50` | single-session-user | reorder-only |
| `9bbe84a2` | knowledge-update | reorder-only |
| `a1cc6108` | multi-session | reorder-only |
| `a82c026e` | single-session-user | reorder-only |
| `b46e15ee` | temporal-reasoning | reorder-only |
| **`b9cfe692`** | temporal-reasoning | **membership: one document** |
| `e61a7584` | knowledge-update | reorder-only |
| `gpt4_6dc9b45b` | temporal-reasoning | reorder-only |
| `gpt4_b4a80587` | temporal-reasoning | reorder-only |

9/10 are **pure reordering** — identical retrieved key sets, identical
`n_retrieved` (40), identical `context_tokens_est`, identical
`answer_keys_retrieved`, identical `answer_sessions_hit`.

`b9cfe692` swaps exactly one document inside a single session:
`sharegpt_ErOTMZ3_149:turn:1:user` out, `sharegpt_ErOTMZ3_149:turn:3:user`
in. No metric moves.

### Artifacts

`~/spectral-local-bench/r16-merge-2026-08-07/`
- `pre.jsonl`, `post.jsonl` — the two 500-row arms from the merge commit.
- `r16_analyze.py` — the script that produces **every** number in the table
  above, including the 793/896 evidence-turn recall, from those two files
  plus `longmemeval_s.json`. Archived deliberately: the 88.50% figure in
  `turn-level-evidence-recall-2026-08-07.md` was previously reproducible only
  from a session transcript.

```bash
cd ~/spectral-local-bench/r16-merge-2026-08-07
python3 r16_analyze.py --pre pre.jsonl --post post.jsonl \
  --dataset ~/spectral-local-bench/longmemeval/longmemeval_s.json
```

An earlier, dirty-tree pair is retained for history at
`~/spectral-local-bench/r16-2026-08-07/`. It agrees at 10/500 with the same
10 ids; the merge-commit pair supersedes it.

---

## How the new default is pinned

1. **The archived paired diff** — any future run on these brains must
   reproduce exactly 10/500 with those exact 10 question ids.
2. **Two unit tests** (`sqlite_store.rs`, tests module):
   `fts_tie_is_broken_by_memory_id_default_path` and
   `fts_tie_is_broken_by_memory_id_fusion_path`. Each builds a **genuine**
   bm25 tie — byte-identical content, single-token keys ⇒ identical doclen
   and term frequencies — inserts the **lexicographically larger** id first
   so scan order disagrees with id order, and asserts a **literal** smaller
   id wins at `LIMIT 1`. Nothing is computed at runtime, so the assertion
   cannot pass under both orderings. Verified by reverting the SQL: both
   tests fail with `left: "ffffffffffffff01", right: "0000000000000001"`.
3. **The tiebreak key itself** — `key_to_id(key)`, a pure function of the
   memory key, so the order reproduces across brains and machines.

---

## Rejected in the same pass: `ORDER BY rank`

§G0 recorded a "bonus" finding: `ORDER BY bm25(...)` forces a temp B-tree,
while `ORDER BY rank` under a query-time `rank MATCH 'bm25(1.0,1.0,0.5)'`
gets FTS5's ordered scan — "identical scores, pure latency win."

**Scores are identical** (verified bit-exactly: `-9.62855697443987` on both
forms, equal on every sampled row, identical id lists). **It is not a free
win, and it is rejected.**

EXPLAIN QUERY PLAN, two brains:

```
ORDER BY bm25(memories_fts,1.0,1.0,0.5)                    -> SCAN fts VIRTUAL TABLE INDEX 0:M3  + USE TEMP B-TREE FOR ORDER BY
ORDER BY bm25(memories_fts,1.0,1.0,0.5), m.id              -> SCAN fts VIRTUAL TABLE INDEX 0:M3  + USE TEMP B-TREE FOR ORDER BY
... rank MATCH 'bm25(1.0,1.0,0.5)' ORDER BY rank           -> SCAN fts VIRTUAL TABLE INDEX 32:rM3   (no temp B-tree)
... rank MATCH 'bm25(1.0,1.0,0.5)' ORDER BY rank, m.id     -> SCAN fts VIRTUAL TABLE INDEX 0:rM3  + USE TEMP B-TREE FOR ORDER BY
```

Latency, 200 interleaved reps on a copy of the live 2585-memory Permagent
brain, 8-term OR bag matching 1890 rows, k=40:

| form | p50 | p10 | p90 |
|---|---|---|---|
| `ORDER BY bm25(...)` (before) | 5.06 ms | 4.40 | 8.90 |
| `ORDER BY bm25(...), m.id` (**shipped**) | 5.44 ms | 4.31 | 9.41 |
| `ORDER BY rank` | 3.73 ms | 3.35 | 6.27 |
| `ORDER BY rank, m.id` | 5.29 ms | 4.47 | 9.29 |

Three reasons, in order of weight:

1. **Mutually exclusive with the fix.** The tiebreak forces a sort, which
   reintroduces the temp B-tree, which erases the win (5.29 ≈ 5.06 ms). You
   can have the determinism fix or the latency fix, not both.
2. **It trades a controlled guarantee for an uncontrolled one.** Without a
   tiebreak, `ORDER BY rank` decides the LIMIT boundary by FTS5's
   *undocumented internal* result ordering — strictly weaker than SQLite's
   temp B-tree. That is the Rule 3 defect ("output depends on an external,
   uncontrolled guarantee"), marketed as a latency win.
3. **The prize is ~1.3 ms** against ~9 ms/question end-to-end oracle
   `retrieval_wall_ms`, on the largest real brain in existence.

Also rejected on the way past: the **persistent** rank config
(`INSERT INTO memories_fts(memories_fts, rank) VALUES('rank','bm25(...)')`).
It writes to `memories_fts_config`, so it (i) fails under `read_only: true`
brains and (ii) silently falls back to unweighted `bm25(1,1,1)` on every
already-built brain on disk that lacks the config row — a silent, unpinned
scoring change on existing data.

**Cost of what we did ship:** ~0.4 ms p50 (5.06 → 5.44 ms) on the largest
real brain, because the temp B-tree key widens. Measured under concurrent
cargo load (p90 ≈ 1.8× p50 for every variant), so it is **directional
only** — but it is a real cost and is stated here rather than discovered
later by a latency gate.

---

## Scope: what was deliberately NOT touched

> **CORRECTION 2026-08-07.** The enumeration below said "six further sites,
> verified by grep against the post-change file". It is **materially
> incomplete**: a re-grep finds **twelve** untiebroken `ORDER BY … LIMIT`
> product sites across **eleven** functions. The most serious omission is
> `:2686` in `prune_wing_keeping_recent_per_source` — a **DELETE** whose choice
> of which rows are *destroyed* is decided by an untiebroken
> `datetime(created_at) DESC LIMIT`. Also omitted: `:2744`
> (`find_recent_episode`, `LIMIT 1`), `:2785`/`:2795` (`list_episodes`),
> `:3569` (`recommend_by_lift`, two sort keys but no unique final key),
> `:3776` (`events_for_session`). The corrected, authoritative table is in
> `REPAIR_REGISTER.md` R18; the table below is left as written for history.
> Ref: `research-alignment-2026-08-07.md` §8.

The product surface is **not** exactly two sites. `sqlite_store.rs` has six
further untiebroken `ORDER BY … LIMIT` clauses on product paths. They are
**not** in this change — folding any of them in would contaminate the clean
10/500 attribution. Each gets its own register row and its own paired run:

| site (post-change line) | fn | key | exposure |
|---|---|---|---|
| `:1779` | `list_memories_by_signal` | `signal_score DESC` | **highest — `signal_score` defaults 0.5, so the LIMIT boundary is inside a guaranteed tie block.** Reached from `brain.rs:4341` (`aaak`). |
| `:1843`, `:1849` | `fingerprint_search` | `hits DESC` (small integer `COUNT(*)`), plus an untiebroken outer `ms.hits DESC` | large ties structurally guaranteed |
| `:2462` | `list_wing_memories_since` | `datetime(created_at) DESC` | low-resolution timestamp |
| `:3458` | `list_undescribed` | `created_at DESC` | low-resolution timestamp |
| `:3484` | `related_memories` | `co_count DESC` | small integer |
| `:4073` | `list_unconsolidated` | `m.created_at DESC` | low-resolution timestamp |

`list_memories_by_signal` is very likely a **larger** determinism exposure
than the one R16 fixes, and is opened at higher priority than
`fingerprint_search`.

Bench binaries carry the same untiebroken clause and are deliberately left
alone: `stmt_cache_probe.rs:56`, `bm25_weights_experiment.rs:245,280`,
`fts_fusion_experiment.rs:119`.

---

## What this claims, and what it does not

**Claims:**
- The default FTS path's LIMIT boundary is now decided by our SQL, on a key
  that reproduces across independently-built brains.
- Default output changed on 10/500 LongMemEval questions (2.0%), 9 of them
  reordering only, 1 a single-document swap.
- No oracle metric moved — session-recall, key-recall, evidence-turn recall,
  zero-key count and mean tokens are all bit-identical between arms.

**Does not claim:**
- Any accuracy improvement or regression. No gate was run because none is
  required (Rule 2) and none would be honest: nothing moved.
- That 10/500 generalizes. It is measured on LongMemEval brains
  (~500–600 memories, `per_turn`, k=40, shape routing). Permagent's live
  brain is 2585 memories with a different key distribution; the count there
  is **unmeasured**. It cannot be larger *in kind* — the tiebreak only
  reorders documents that were already exactly tied — but do not quote
  10/500 as a general figure.
- Anything about the six sites listed above.

---

**Refs:** `landscape-research-2026-08-07.md` §G0 (corrected by this doc),
`turn-level-evidence-recall-2026-08-07.md` (R15), `REPAIR_REGISTER.md` R16.
