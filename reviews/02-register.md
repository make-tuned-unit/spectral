# Phase 2 — Synthesis register (2026-08-14)

Sources: `01-codex.md` (CX-nn, 12 findings, DO NOT SHIP), `01-cursor.md`
(CU-nn, 14 findings, SHIP WITH FIXES), `01-opus5.md` (OP-nn, 15 findings,
SHIP WITH FIXES). Codex and cursor both emitted the brief's example prefix
`X-`; relabelled CX-/CU- here. "via verdict" = the agent supported the row in
its claim-verdict section without filing a separate finding.

## Deduplicated findings register

| ID | Title | Sev | Location | Agreeing | Disagreeing | Status |
|---|---|---|---|---|---|---|
| R-01 | Federation fan-out lacks visibility scoping before peer truncation | P1 | federation.rs:384,392 | CX-01, CU-06, OP-01 | — | AGREED |
| R-02 | Federated read mutates writable peers (reinforce + query-metadata written into peer) | P1 | cascade_layers.rs:491; federation.rs:280 | CX-01, CU-02, OP-02 | — | AGREED |
| R-03 | "One SQLite file" is actually three databases | P1 | brain.rs:995–1086 | CX-02, CU-04, OP via C3 verdict | — | AGREED |
| R-04 | Wall-clock `Utc::now()` default breaks reproducible recall (incl. additive recency channel) | P1 | brain.rs:1899/1904; context.rs:26; ranking.rs:707 | CX-05, CU-03+CU-08, OP via C1 verdict | — | AGREED |
| R-05 | Default auto-reinforce on recall: read path mutates ranking state; write errors swallowed | P1 | cascade_layers.rs:294,491; brain.rs:3634–3654 | CU-01, OP-10, CX-07 | — | AGREED |
| R-06 | Wing-tier `ORDER BY signal_score` lacks tiebreak before truncate | P1 | sqlite_store.rs:2048 (+ :1860) | CX-04, CU-05 | — | AGREED |
| R-07 | `remember()` spans two DBs non-atomically; partial failure returns Ok | P1 | brain.rs:1836–1845 | CX-03, OP-06 | — | AGREED |
| R-08 | Sync import/tombstones unauthenticated: forgeable authorship, remote hard-delete primitive | P1 | federation_sync.rs:296–299, 432–455 | OP-03 | — | SINGLE-SOURCE → rebuttal |
| R-09 | No `busy_timeout` on any SQLite connection | P1 | sqlite_store.rs:277–282; graph_store.rs:116; store.rs:187 | OP-04 | — | SINGLE-SOURCE → rebuttal |
| R-10 | `graph.sqlite` not in WAL; deletion-guarantee `wal_checkpoint` calls are no-ops | P1 | graph_store.rs:115–121, 158–162 | OP-05 | — | SINGLE-SOURCE → rebuttal |
| R-11 | Memory signature omits `key`; `verify_hit` unused in production | P1 | identity.rs:223–244; brain.rs:1218–1242 | OP-07 | — | SINGLE-SOURCE → rebuttal |
| R-12 | Federation members unauthenticated → Sybil corroboration (+ `RawScore` escape hatch) | P2 | federation.rs:112–118, 145 | CX-11, CU-11, OP via C10 verdict | — | AGREED |
| R-13 | Episode listing `ORDER BY created_at` lacks tiebreak | P2 | sqlite_store.rs:2955 | CX-08, CU-09 | — | AGREED |
| R-14 | Read-only open silently substitutes empty recognition index when sidecar absent | P2 | brain.rs:1087 | CX-06 | — | SINGLE-SOURCE |
| R-15 | Associative spreading suppresses storage errors | P2 | spreading.rs:228 | CX-09 | — | SINGLE-SOURCE |
| R-16 | Consolidation source caps select nondeterministic evidence | P2 | sqlite_store.rs:4167 | CX-10 | — | SINGLE-SOURCE |
| R-17 | "Unused memories decay" claim vs turn ledger's explicit refusal to auto-decay | P2 | turn.rs:507 | CU-07 | CX & OP verdict C8 MET | DISPUTED → rebuttal |
| R-18 | Graph document scan iterates a `HashSet` (order instability) | P2 | graph_store.rs:627 | CU-10 | — | SINGLE-SOURCE |
| R-19 | Recognition evidence tie order = `HashMap` iteration order | P2 | score.rs:289–296 | OP-09 | — | SINGLE-SOURCE |
| R-20 | Visibility is post-filter after SQL LIMIT; graph BFS traverses Private edges | P2 | brain.rs:2160, 2481–2494 | OP-08 | — | SINGLE-SOURCE |
| R-21 | `FanoutResult::failed` ignorable; total peer failure looks like success | P2 | federation.rs:384–390 | OP-11 | — | SINGLE-SOURCE |
| R-22 | Three write APIs bypass the read-only guard (wrong error type) | P2 | brain.rs:2619–2636, 3016, 4310 | OP-12 | — | SINGLE-SOURCE |
| R-23 | `neighborhood()` unbounded BFS holds the graph lock | P2 | graph_store.rs:576–621 | OP-13 | — | SINGLE-SOURCE |
| R-24 | Two invariant tests cannot fail (both match arms pass; literal-vs-literal assert) | P2 | concurrency_tests.rs:260–308; federation.rs:678 | OP-14 | — | SINGLE-SOURCE |
| R-25 | Graph conflict ordering incomplete for equal `asserted_at` | P3 | graph_store.rs:430 | CX-12 | — | SINGLE-SOURCE |
| R-26 | `Brain::open` docs misstate storage layout | P3 | spectral/src/lib.rs:247 | CU-12 | — | SINGLE-SOURCE (fold into R-03 remedy) |
| R-27 | quinn-proto advisory path = reqwest optional http3 (not default) | P3 | spectral/Cargo.toml:31 | CU-13 | — | SINGLE-SOURCE |
| R-28 | "Recall = FTS5 + BM25" copy understates the actual pipeline | P3 | brain.rs:2315 | CU-14, CX via C4 PARTIAL | OP C4 MET | DISPUTED → rebuttal |
| R-29 | NOTICE placeholder copyright holder; scripts default to dead host path | P3 | NOTICE:2–4; scripts/run_accuracy_ab.sh:12 | OP-15 | CX & CU C14 MET | DISPUTED → rebuttal |

## Conflicting claim verdicts

| Claim | Codex | cursor | Opus 5 | Conflict |
|---|---|---|---|---|
| C3 one SQLite file | NOT MET | PARTIAL | NOT MET | severity of wording |
| C4 recall = FTS5+BM25 | PARTIAL | PARTIAL | MET | substantive |
| C8 adaptive loop (unused decay) | MET | PARTIAL | MET | substantive |
| C9 federation visibility-scoped | PARTIAL | MET | PARTIAL | substantive |
| C14 Apache-2.0 coherent | MET | MET | PARTIAL | substantive (NOTICE) |

Unanimous: C1 PARTIAL, C2 MET, C5 MET, C6 MET, C7 MET, C10 PARTIAL,
C11 CANNOT VERIFY, C12 CANNOT VERIFY, C13 MET.

Production readiness split: **DO NOT SHIP (Codex) vs SHIP WITH FIXES (cursor,
Opus 5)** — adjudicated in Phase 4, not averaged.

## Phase 3 outcomes (single rebuttal round — see 02-rebuttal-*.md)

- **R-08, R-09, R-10, R-11: SINGLE-SOURCE → AGREED (×3).** Codex and cursor
  each independently confirmed all four with fresh evidence (cursor verified
  the `wal_checkpoint` no-op return `(0,-1,-1)`; Codex traced the tombstone
  path `:393-398`; Opus survived the WAL-persistence counterargument by
  exhausting every `journal_mode` call site).
- **C3 → PARTIAL (adjudicable).** Codex NOT MET→PARTIAL, Opus NOT
  MET→PARTIAL, cursor PARTIAL→NOT MET. All agree on the facts (one handle,
  three files); residual is labeling of a conjunctive claim. Adjudicated
  PARTIAL in Phase 4: the handle/embedded conjuncts hold, the one-file
  conjunct is false.
- **C4 → unanimous PARTIAL.** Opus withdrew MET after re-checking the entry
  point (`cascade_retrieve_scoped` runs TACT first, FTS backfill only).
  R-28 stands AGREED.
- **C8 → unanimous PARTIAL.** Codex and Opus withdrew MET; Opus added the
  decisive fact that `decayed_signal_score` has exactly one call site
  (`brain.rs:1936`), absent from the canonical cascade paths. R-17 stands,
  upgraded to AGREED.
- **C9 → unanimous PARTIAL.** Cursor withdrew MET, reconciling with its own
  CU-06.
- **C14 → unanimous PARTIAL.** Codex and cursor withdrew MET; Opus proved
  "Alice Doe"/"Polaris" are the repo's own retired demo-fixture names
  (`brain.rs:4304-4306`). R-29 upgraded to AGREED.
- No agent withdrew any finding. No second rebuttal round (per protocol).

## Missed by all three (visible in baseline data)

- Nobody filed a finding on the **coverage gap** (no tool installed, coverage
  never measured). ~~Nor on 11 of 20 test binaries being empty.~~
  **RETRACTED:** that second half rested on a truncated Phase 0 capture; the
  suite actually has 914 tests across 106 binaries. See the CORRECTION block
  in `00-baseline.md`. The reviewers were briefed with the wrong figure.
- **crossbeam-epoch RUSTSEC-2026-0204** (dev-deps) and **paste unmaintained**:
  no reviewer proposed a disposition (acceptable-risk note or bump).
- No reviewer contradicted the baseline; no failing test went unmentioned
  (there were none).
