# Research alignment — 2026-08-07

What shipped in this session, what it claims, what it explicitly does not
claim, what was rejected, and what the alignment pass itself found.

Tree state at time of writing: HEAD `17e0838` (merge), **nothing committed**,
18 tracked files modified, 5 untracked docs, one untracked fixture directory.
`cargo fmt --all -- --check` clean. `cargo clippy --all-targets --all-features
-- -D warnings` clean. **`cargo test --workspace --release --no-fail-fast`:
297 passed, 1 FAILED, 1 ignored.**

---

## 0. Headline

| item | verdict |
|---|---|
| **R15** — true evidence-turn metric | **READY TO MERGE.** Every number independently recomputed and reproduces to the digit. Instrument only, no accuracy claim, no gate needed. |
| **R16** — SQL tiebreak on the default FTS path | **NOT MERGEABLE AS-IS.** The measurement is exactly as reported, but it turns a determinism test red and shipped with `suite_passed: true`. |
| **third changeset** — `void_turn_deferred` + converter `--all` | in the tree, **unreported by both implementers**; benign but must be attributed before merge. |
| `preference-retrieval-diagnosis` | **REJECTED** (spec, not analysis). Prereg written; run BLOCKED. |
| `bitemporal-and-temporal-wiring` | **REJECTED.** No prereg written — the design must be rebuilt before a measurement plan means anything. |

---

## 1. What shipped

### R15 — the oracle's evidence metric was diluted 12.2×, and now isn't

**This is the most important finding of the session, and it is a finding
against ourselves.**

`oracle::is_answer_key` counted every turn of every `answer_`-prefixed session
as an answer key: **10,960 turns** against LongMemEval's **896** turns actually
labelled `has_answer: true`. "key-recall 55.6%" was never evidence recall — it
was evidence-*session* turn coverage, a proxy with a denominator 12.2× too
large. The 98.1% we have quoted for two months is *session* recall, which a
40-turn session satisfies even when the one evidence turn is missing.

What landed (instrument only — retrieval output byte-identical):

* `dataset::Turn.has_answer: Option<bool>` — the label is no longer discarded
  at load; skipped on serialize when absent, so datasets round-trip unchanged.
* `ingest::memory_key()` — one authority for the key format, shared by the
  write path and evidence scoring, frozen by `memory_key_format_is_frozen`.
  Byte-identical to the inline `format!`s it replaces, so archived bench brains
  stay reusable and no re-ingest is triggered.
* `OracleRow`: `answer_keys_*` → `answer_session_turns_*` with `#[serde(alias)]`
  so the whole JSONL archive still loads; `evidence_turns_{total,retrieved}`,
  `rank_first_evidence_turn`, `evidence_keys_missed` added.
* `oracle-evidence` subcommand — rescores archived rows offline for $0, never
  rewrites its input.
* `stratified_ab.rs` renamed its second copy of the diluted computation
  (`key_recall` → `answer_session_turn_coverage`).

**Refusals, not zeroes.** The metric emits `None` — never `Some(0)` — when the
dataset carries no label, when ingest is `PerSession` (the field would count
sessions while its name says turns), and when the retrieved key set is not
turn-shaped (the `Graph` path, which falls back to `--- Session <id>` parsing).
Without that last guard the Graph path would silently report a fabricated 0/N.
All three are pinned by tests.

**Verification.** All figures recomputed independently in Python straight from
the labels: r12-baseline 793/896 = 88.504% micro / 90.543% macro, 479 labelled
/ 21 unlabelled, 27 zero-evidence, 409 full; oracle-baseline 749, porter 783,
cap 749, r12-actr 794; per category `single-session-user 65/66,
knowledge-update 140/144, single-session-assistant 54/56, temporal-reasoning
229/259 (11 zero), multi-session 276/327 (4 zero), single-session-preference
29/44 with 9 of 30 zero`. Every one matches. The committed 45-question CI
fixture was checked line-by-line against the archive: all 45 rows are verbatim
copies, the trimmed dataset is index `i%12==0` + {64,126,227} as documented,
and its evidence-key set is identical to the full dataset's for all 45
questions despite the trimming.

### R16 — SQL tiebreak on the default FTS path

`, m.id` added at two sites in `sqlite_store.rs`: the non-fusion default
`fts_search` ORDER BY (`:2024`) and the fusion channel subqueries in
`ranked_ids` (`:2055`) whose *rank positions* feed RRF. Two unit tests build a
genuine bm25 tie, insert the lexicographically larger id first, and assert a
literal smaller id wins at `LIMIT 1`; both fail if the clause is reverted.

Measured $0 on the merge commit in a clean detached worktree, 500 LongMemEval
questions, reused brains. Independently re-derived from
`~/spectral-local-bench/r16-merge-2026-08-07/{pre,post}.jsonl`:

* pre-arm vs published `r12-baseline.jsonl`: **0/500** context_hash diffs.
* pre vs post: **exactly 10/500** (2.0%), and exactly the 10 ids claimed
  (`2ebe6c92, 8ebdbe50, 9bbe84a2, a1cc6108, a82c026e, b46e15ee, b9cfe692,
  e61a7584, gpt4_6dc9b45b, gpt4_b4a80587`).
* 9 pure reorder, 1 membership change (`b9cfe692` swaps
  `sharegpt_ErOTMZ3_149:turn:1:user` out, `:turn:3:user` in).
* By path: TopkFts 10/167, Cascade 0/333.
* Evidence-turn recall pre→post identical (793/896), **per-question change
  count 0**.

**The measurement is exactly as reported. The merge readiness is not.**

---

## 2. R16 lands a red test, and was reported green

`crates/spectral/tests/deterministic_anchor.rs:83`
`recency_decay_is_order_invariant_in_the_topk_path` **FAILS**. Implementer 2
reported `"suite_passed": true`. That is false.

Bisected and round-tripped independently in this session (file restored
byte-for-byte, sha256 `df52dc90…` before and after):

| state | result |
|---|---|
| working tree (R16 present, 2 × `m.id LIMIT ?2`) | **1 failed** / 5 passed |
| `, m.id` reverted at both sites, nothing else touched | **6 passed** |
| restored | **1 failed** / 5 passed |

### Root cause — and it is not the tiebreak

The test's own docstring asserts the invariant it checks:

> *"`ranking::apply_recency_weight` (top-k FTS and the cascade) is
> **multiplicative** … so that path is order-invariant under a clock shift, by
> construction."*

**That is wrong about the code path the test exercises.** `recall_topk_fts`
does not call `apply_recency_weight`. It calls `apply_reranking_pipeline`,
where three things compound:

1. **The base score is the FTS *rank position*** —
   `ranking.rs:345-347`, `scores[i] = 1.0 - (i as f64 / n)`. A pure reorder of
   a bm25-tied pool is therefore **not score-neutral**; it changes the numbers.
2. **Recency is ADDITIVE, not multiplicative** — `ranking.rs:411`,
   `scores[i] += RECENCY_BOOST_WEIGHT * freshness`, with the multiplicative
   form explicitly removed in the comment above it. An additive term does not
   preserve order under a clock shift: at +5 years the freshness term shrinks
   ~32× at the default 365-day half-life, so it stops being able to override
   rank-position differences.
3. **Truncation happens after reranking** — `brain.rs:2101` fetches
   `k × fetch_mult` (20 × 3 = 60 ≥ the fixture's 24 memories, so the pool is
   *all* of them) and `brain.rs:2147` truncates to `k` afterwards. So a
   boundary flip changes the retrieved **set**, not just its order — which is
   exactly what the failure shows (`s22` out, `s4` in).

So: R16 changed the order of a fully-tied pool; the rank-position base turned
that into different scores; the additive recency term's clock-dependence turned
those into a different set at the shifted clock.

### What this does and does not mean

**It does NOT mean R16 broke byte-identical determinism.** Repeat runs at a
fixed `now` are unchanged — `reproducible_retrieval_is_stable_across_repeated_calls`
passes, and the pre-arm reproduced itself at 0/500. R16's own claim (the LIMIT
boundary is decided by our SQL rather than SQLite's plan) is true and is
strengthened, not weakened, by this finding.

**It DOES mean a shipped-path invariant is genuinely violated, and was
violated before R16.** Top-k ranking is a function of the wall clock. The
`deterministic_anchor` suite exists precisely to catch that, and it did — R16
merely flipped which side of the coin the fixture lands on. This is a **new
defect, recorded as R20**, not an R16 regression.

**It also means one sentence of R16's framing is too strong.** "9 of 10 are
pure reorder" is measured and correct, but "reorder" should not be read as
"harmless": with a rank-position base score, reordering a tied pool feeds
different numbers into every downstream boost. The empirical result — no
oracle metric moved, zero per-question evidence change — stands on its own
measurement and is not affected by this.

**Consequence: R16 does not merge until `recency_decay_is_order_invariant_in_the_topk_path`
is either fixed or explicitly re-baselined with a recorded justification.**
Re-baselining is the cheaper option and is defensible — the test asserts a
property of a function the path does not call — but it must be recorded as
such, and R20 must be opened at the same time. Silently deleting or relaxing
the assertion would be the single worst outcome available here.

---

## 3. A third changeset is in the tree that neither implementer reported

Both implementers wrote "FILE DISCIPLINE: only the files listed above were
edited", and both ran the test suite over a tree that also contains:

* `crates/spectral-graph/src/brain.rs` (+50), `crates/spectral/src/turn.rs`
  (+21), `crates/spectral/tests/turn_ledger.rs` (+33) — `void_turn_deferred` /
  `drain_pending_voids` / `pending_voids`, implementing the API Permagent
  asked for in `permagent-reply-2026-08-07y.md` (a `Drop`-safe, non-blocking,
  non-failing void enqueue). Additive; no default-path behaviour change.
* `scripts/locomo_to_oracle.py` — a new `--all` CLI flag **and a pool-guard
  bypass**, staged for `bm25-locomo-baseline-prereg-2026-08-07.md`
  (`locomo_full_answerable.json`, 1,438 answerable questions).

Implementer 1 stated: *"The converter got docstring corrections only — no
behaviour change."* **That is false.** New flag, new control flow. The
docstring corrections are also real and are good ones, but the claim as
written is not.

Neither `suite_passed` therefore isolates its own work. Nothing here is
harmful; the accounting is what is wrong, and the fix is attribution at commit
time, not code.

---

## 4. What these results claim

1. **The oracle now measures turn-level evidence recall** (`has_answer`),
   which is what LongMemEval ships the label for. Nothing more.
2. **Shipped-config evidence-turn recall is 88.5% micro / 90.5% macro over
   479 labelled questions, with 27 retrieving zero evidence.**
3. **`single-session-preference` is 29/44 = 65.9%, with 9 of 30 questions
   retrieving zero evidence** — the most localized retrieval gap the project
   has ever measured.
4. **The default FTS `LIMIT` boundary is now decided by our SQL**, on
   `m.id = key_to_id(key)`, a pure function of the memory key — so the order
   reproduces across independently-built brains, not merely across repeat
   reads of one file.
5. **That change moved default output on 10/500 LongMemEval questions and
   moved no oracle metric.**

## 5. What these results explicitly do NOT claim

1. **No accuracy claim, from either changeset.** No gate was run, none was
   required (Rule 2), and none would have been honest — nothing moved.
2. **88.5% is not an improvement on anything.** It is the *same retrieval*
   against a denominator 12.2× smaller. Any reading of "55.6% → 88.5%" as
   progress is a category error, and the two numbers are not comparable.
3. **The backfilled per-run figures are re-descriptions, not results.** They
   are recomputed offline from rows already on disk; no arm was re-run.
4. **10/500 does not generalize.** It is LongMemEval brains only (~500–600
   memories, `per_turn`, k=40, shape routing). The live 2,585-memory Permagent
   brain is unmeasured.
5. **The ~0.4 ms p50 cost of the tiebreak is directional only** — measured
   under concurrent load.
6. **Evidence-turn recall is UNDEFINED, not zero,** on the 21 `_abs`
   LongMemEval questions and on every LoCoMo-converted set (LoCoMo carries no
   per-turn labels — see R19).
7. **We do not assert what the correct metric would have shown for the
   answerability (run 1/2/3), supersession, or LoCoMo k-lever experiments.**
   Their per-arm row files were not retained, or their corpus carries no
   labels. Unknown, in both directions, and left unknown. The published
   verdicts are untouched; only the framing of the metric is corrected.

---

## 6. Negative results and refusals — equal billing

These are outputs, not failures to produce output.

| result | status |
|---|---|
| **R16's original rationale was empirically false and is deleted, not repeated.** The claim that FTS5's `1e-6` IDF clamp collapses common-term documents into one large tie block does not hold on this corpus: the tf/doclen factor still varies, so a pure-`"the"` query yields ~2,585 *distinct* near-zero scores and 0/40 brains have the LIMIT boundary inside a tie block. Real ties are rare and small (0/120 full-question queries straddle; single-term queries straddle in 24%, median block 2, max 5). | **retracted, with the measurement that retracted it** |
| **The `ORDER BY rank` "pure latency win" is REJECTED.** Scores are bit-identical, but `ORDER BY rank, m.id` reintroduces the temp B-tree (5.29 ms vs 5.06 ms), so the latency win and the determinism fix are mutually exclusive; untiebroken it moves the LIMIT boundary into FTS5's *undocumented internal* ordering — a Rule 3 regression for ~1.3 ms. The persistent rank-config form is rejected separately (writes `memories_fts_config`, fails read-only, silently falls back to unweighted `bm25(1,1,1)` on existing brains). | **REJECTED** |
| **`answer_keys_*` deltas for the answerability and supersession preregs are unrecoverable.** Per-arm row files not retained. | **UNKNOWN and stated as unknown** |
| **LoCoMo evidence-turn recall is unavailable, not 0%.** The tool prints `UNAVAILABLE on both arms … Delta unknown — not zero` rather than emitting a number. | **refusal by construction** |
| **`PerSession` ingest refuses the turn metric** rather than reporting session counts under a field named "turns". | **refusal by construction** |
| **The Graph retrieval path refuses the turn metric** when its key set is not turn-shaped, instead of reporting a fabricated 0/N. | **refusal by construction** |
| **fm=3 (cascade widening) remains unshipped** despite being retrieval-Pareto-safe, because its only end-to-end actor A/B was directionally *worse* (14 fails vs 11, n=30, unpinned temperature). A retrieval proxy does not ship a default change. | **held, correctly** |
| **The rank-position base score means "9/10 were pure reorder" must not be read as "harmless".** | **framing corrected** |

---

## 7. What was rejected, and why

### `preference-retrieval-diagnosis` — REJECTED (the spec, not the analysis)

The mechanism story is plausible, the five hypothesis verdicts are internally
consistent, and the refusal to ship a lever on a retrieval proxy is correct.
Three things disqualified it:

1. **The determinism section made an affirmatively false statement about the
   tree it would land on.** It cited `sqlite_store.rs:2018` as having "no
   secondary sort key" and stated *"the production query … still has no
   tiebreak (R16) — I am NOT fixing that here."* R16 is live and uncommitted
   in this very tree. Every rank number in the diagnosis was computed against
   HEAD's ordering, which is not this tree's ordering.
2. **The proposed instrument fabricates a metric under a shipped code path.**
   `evidence_keys` hardcoded `{sid}:turn:{i}:{role}` and took no
   `IngestStrategy`. Under `PerSession` the ingest emits `{session_id}:session`,
   so the intersection is empty by construction and every question would report
   `evidence_turns_retrieved = 0` — a fabricated 0% that reads exactly like a
   catastrophic regression. (R15 as shipped refuses instead. That is the
   difference between the two specs in one line.)
3. **Incomplete R15 closure.** `answer_keys_retrieved` is not report-only:
   `oracle.rs` derives the zero-key count from it *and* uses it in
   candidate-vs-baseline decision logic. Renaming the field without moving the
   comparison logic leaves R15's stated consequence still true after the
   change.

Also unresolved: the fidelity claim's derivation chain
(wing-less → TACT tier 3 → non-fusion top-k) was verified correct, but
`policy.rs:315-318` routes Temporal to `RetrievalRoute::TopkFts` — a different
path — and temporal-reasoning is 132 of the 479 questions, published under the
same "Jaccard 0.972" fidelity umbrella.

**What survives and must not be re-litigated:** the k-depth curve correctly
labelled a pool-membership upper bound rather than predicted recall; the honest
citation of its own hostile prior (fm=3's failed actor A/B); and above all the
**gate design** — actor accuracy as the primary endpoint rather than the oracle
proxy, two-stage disjoint split per R11, exact McNemar with a declared p<0.05
and reported b/c, an explicit statement that n=30 cannot carry a preference-only
claim so the subgroup is descriptive-only, and a null pre-committed as
publishable. That design is materially better than the register's precedent and
closes the PR #239 hole. It is preserved verbatim in
`preference-evidence-retrieval-prereg-2026-08-07.md`.

### `bitemporal-and-temporal-wiring` — REJECTED

1. **The load-bearing determinism pin was built on a stale copy of the SQL.**
   Its §4 reproduced the non-fusion FTS statement without `, m.id` (that is
   `git show HEAD:…`, not the working tree) and delivered the change as a
   `format!` literal *replacing the whole statement*. Shipping it verbatim
   drops the tiebreak, and its own `assert_eq!(fts_sql(None), FTS_SQL_LITERAL)`
   would then enshrine the regressed text as "current". A silent baseline shift
   dressed as a pin. Rules 3 and 6.
2. **The as-of predicate is silently ignored on the fusion route** —
   `sqlite_store.rs:2011` branches on `!fusion` and the spec appends the clause
   to the non-fusion arm only. `fts_fusion` defaults false so the *default*
   path is covered, but it is a live bench lever and a public config field.
   With fusion on, `recall_as_of` returns a present-day pool and reports
   success — the exact failure mode the author calls "strictly worse than no
   feature".
3. **"The four new columns are ALWAYS canonical" is contradicted by its own
   migration.** The backfill's `COALESCE(strftime(...), created_at)` fallback
   writes the raw space-form value, i.e. the invariant fails on precisely the
   rows most likely to be malformed.
4. **The backfill is not one-shot.** `UPDATE … WHERE known_from = ''` sits
   outside the guard and runs a full table scan on every `open`; and because
   the columns are `NOT NULL DEFAULT ''`, `'' <= ?3` is TRUE, so any
   unset row matches every as-of predicate as if it had always been known,
   then gets retroactively stamped at the next open. A row's `known_from`
   depends on process lifecycle rather than data. Rule 3.
5. **Serde surface uncovered.** `pub validity: Option<Validity>` with
   `#[serde(default)]` and no `skip_serializing_if` emits `"validity":null` in
   every serialized `Memory`. Unquantified default-path byte change.
6. **Half of a cited "latent defect" does not exist.** `graph_store.rs:476-480`
   compares `t.asserted_at <= as_of` as typed `DateTime<Utc>`, not strings —
   no format hazard there at all.

No prereg is written for this item: the design must be rebuilt before a
measurement plan means anything.

---

## 8. What the alignment pass itself found

Four claims in the shipped docs are not supported by the tree, and are
corrected in the register:

1. **`suite_passed: true` (R16)** — false. Section 2.
2. **"docstring corrections only" (converter)** — false. Section 3.
3. **"the six remaining untiebroken product sites, verified by grep"** —
   materially incomplete. `sqlite_store.rs` has **twelve** further untiebroken
   `ORDER BY … LIMIT` product sites across **eleven** functions, not six. The
   omitted ones include `prune_wing_keeping_recent_per_source` at `:2686` — a
   **DELETE** whose choice of which rows are *destroyed* is decided by an
   untiebroken `datetime(created_at) DESC LIMIT`. That is a higher-severity
   exposure than R17. Full corrected table in R18.
4. **"ONE KNOWN REMAINING VOCABULARY SITE … retrieval.rs is already modified
   in the working tree by other work"** — `crates/spectral-bench-accuracy/src/retrieval.rs`
   is **not** modified (`git diff --stat` empty), and there are **three** stale
   sites, not one: `retrieval.rs:804`, `cascade_layers.rs:186`,
   `cascade_layers.rs:247`. Additionally, the R16 register row written in this
   same tree reintroduces "key-recall" and "zero-answer-key" in prose — the ban
   is structural in code but not yet in the register.

Minor: `~/spectral-local-bench/r16-pre2.jsonl` is reported as "deleted after
the check"; it is absent from that path, so the report is correct there.

---

## 9. Merge readiness and sequence

| # | action | cost | blocks |
|---|---|---|---|
| 1 | **Merge R15.** Attribute the `void_turn_deferred` changeset as its own commit. | $0 | nothing |
| 2 | **Resolve `recency_decay_is_order_invariant_in_the_topk_path`** — fix or re-baseline with a recorded justification, and open R20 either way. | $0 | R16 |
| 3 | **Merge R16** once (2) is green. Attribute the converter `--all` flag separately. | $0 | the BM25 baseline |
| 4 | **R17** (`list_memories_by_signal`, guaranteed tie block) — own paired oracle run. | $0 | — |
| 5 | **R18** (eleven further sites; do `prune_wing_keeping_recent_per_source` first — it is a DELETE). | $0 | — |
| 6 | **Stage 0 of the preference prereg** ($0 oracle screen). Most likely outcome is a recorded null that stops the paid work. | $0 | the paid gate |
| 7 | **BM25-only LoCoMo baseline** — already preregistered, ordering requirement satisfied once R15+R16 land. | **~$18.26** | needs a working key + sign-off |
| 8 | **Preference lever accuracy gate**, only if stage 0 admits it. | **~$40/stage, $80 ceiling** | needs sign-off |

Nothing above is committed or pushed.

---

## Refs

* `turn-level-evidence-recall-2026-08-07.md` — the $0 finding
* `r15-evidence-metric-2026-08-07.md` — R15 implementation and scope
* `r16-baseline-shift-2026-08-07.md` — R16 measurement of record
* `bm25-locomo-baseline-prereg-2026-08-07.md` — preregistered floor measurement
* `preference-evidence-retrieval-prereg-2026-08-07.md` — this session's prereg
* `landscape-research-2026-08-07.md` §G0 — corrected tie-block analysis
* `permagent-reply-2026-08-07y.md` — origin of the `void_turn_deferred` API
* `REPAIR_REGISTER.md` — R15, R16, R17, R18, R19, R20
