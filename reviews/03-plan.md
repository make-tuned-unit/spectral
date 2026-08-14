# Phase 4 — Adjudication and remediation plan (2026-08-14)

## Adjudication

Applied tie-break rules in order. After the rebuttal round only two conflicts
remained; neither needed rule 4 (UNRESOLVED):

1. **C3 label** (2× PARTIAL vs 1× NOT MET post-rebuttal, identical facts):
   adjudicated **PARTIAL** — a three-conjunct claim ("one handle" true,
   "embedded/no service" true, "one SQLite file" false). Rule 2: all sides
   cite the same code; the PARTIAL label describes it more precisely.
2. **Ship verdict** (Codex DO NOT SHIP vs cursor/Opus SHIP WITH FIXES): the
   evidence is agreed; the split is framing. Adjudicated: **DO NOT SHIP
   as-is against the pitch-as-acceptance-criteria; SHIP WITH FIXES once the
   P1 set below lands and the copy corrections are made.** This is scoping,
   not averaging: with 11 open P1s including three security/data-loss items
   (rule 3), today's answer to "do we meet the client promise" is no.

All four formerly single-source P1s were confirmed 3/3 in rebuttal and carry
reproducible or exhaustively-traced evidence (rule 1).

## Remediation plan (ranked; no P0s exist — nothing is deployed)

Severity P1 first, security/data-loss before behavior, then bundled quick wins.

| # | Row(s) | Fix | Files touched | Est. diff | Proving test | Risk |
|---|---|---|---|---|---|---|
| 1 | R-08 | Add `signature` to `MemoryObject`/`Tombstone`; verify against claimed `author_id` (existing `spectral-core/identity.rs` machinery) before import INSERT / tombstone DELETE; reject on failure | `federation_sync.rs`, `identity.rs` | ~250 lines | forged-author pack and forged tombstone are rejected; signed ones apply | med — wire-format change; no external consumers yet |
| 2 | R-02 | Force `write_back = false` on the config passed to children inside `fan_out_recall_with_policy`; docs on `add_brain` | `federation.rs` | ~15 lines | `read_only_child_is_not_mutated_by_fan_out` (cursor's proposed name): member scores byte-identical before/after fan-out | low |
| 3 | R-01 | Fan out via `recall_cascade_scoped(..., visibility)`; keep coordinator post-filter as defence-in-depth | `federation.rs:384` | ~10 lines | peer whose top-k is Private + one lower Public hit: Public hit now surfaces | low |
| 4 | R-11 | Include length-prefixed `key` in `memory_signing_payload`; bump domain to v2, accept v1 on verify for legacy rows | `identity.rs`, `brain.rs` | ~100 lines | re-keyed verbatim hit fails `verify_hit`; v1 rows still verify | med — dual-version verify |
| 5 | R-07 | Expose `derivation_warnings` on `RememberResult`; extend `repair_derivations` to re-enroll memories missing from recognition index | `brain.rs` | ~120 lines | kill recognition.db writability → `remember` returns warning; `repair_derivations` heals; `recognize` then matches | low |
| 6 | R-10 | Apply the `sqlite_store.rs:277` PRAGMA batch (WAL etc.) in `GraphStore::open`; re-run deletion-guarantee test over graph.sqlite | `graph_store.rs` | ~15 lines | `PRAGMA journal_mode` returns `wal`; D4 byte-scan test passes against graph.sqlite | low — WAL is persistent and auto-migrates from DELETE mode |
| 7 | R-09 | `busy_timeout(5s)` on every connection open (6 sites incl. reader pool) | `sqlite_store.rs`, `graph_store.rs`, recognition `store.rs` | ~12 lines | two writer handles: second waits and succeeds instead of SQLITE_BUSY | low |
| 8 | R-06 + R-13 + R-16 + R-25 + R-19 | Complete this branch's own R17/R18 mission: add deterministic tiebreaks at every site the reviewers found — wing `ORDER BY signal_score` (`:2048`, `:1860`), episode `ORDER BY created_at` (`:2955`), consolidation edges (`:4167`), graph conflicts (`graph_store.rs:430`), recognition evidence sort (`score.rs:294`) | `sqlite_store.rs`, `graph_store.rs`, `score.rs` | ~20 lines + tests | tied-value regression test per site (equal scores/timestamps > limit) | low |
| 9 | R-05 (partial) | Surface the swallowed write-back errors (`let _ =` → `tracing::warn!`); add `#[must_use]`/`is_complete()` + warn on fan-out child failure (R-21) | `brain.rs:3634`, `federation.rs:387` | ~25 lines | log assertion on forced write failure | low |
| 10 | R-22 | `ensure_writable` in `set_entity_field`, `consolidate_extractive`, `reclassify_wings_in` | `brain.rs` | ~6 lines | read-only brain returns `Error::ReadOnly`, not `Error::Schema` | low |
| 11 | R-14 | Read-only open with missing recognition sidecar: return explicit degraded status instead of silent empty index | `brain.rs:1087` | ~20 lines | open read-only w/o sidecar → status flag set; recognize of stored content reports degraded, not Novel | low |
| 12 | R-03 + R-26 + R-28 + R-29 + C1/C8/C10 copy | Truth-in-copy batch: pitch/README "one SQLite file" → "one brain directory (three SQLite files)"; "Recall = FTS5+BM25" → "+ deterministic local tiers/re-rank"; determinism claim scoped to read-only/anchored opens; "poisoning-resistant" → "score-flood-resistant (RRF + caps); authorship/Sybil = deployment trust"; "unused ones decay" → "unused-ness tracked; decay opt-in (Archivist)"; fix `lib.rs:247` doc; **NOTICE real attribution — needs the legal holder name from you** | `docs/pitch.md`, `README.md`, `spectral/src/lib.rs`, `NOTICE` | ~60 lines docs | n/a (prose) + guardrails section updated | none |

Total estimated: ~650 lines across 12 items, each committable independently
with its finding ID.

## Not doing (with reasons)

- **Physical single-file DB consolidation** (R-03 full remedy): a storage
  migration with real data-loss risk; the copy fix removes the false claim at
  zero risk. Revisit as a versioned migration if "one file" matters commercially.
- **Wiring `verify_hit` into `merge_and_rank` + key/grant registry** (R-11
  full remedy, C9/C10 hardening): requires a trust-model design (grant sets)
  that doesn't exist yet; item 4 fixes the signature soundness gap first.
- **Authenticated federation identity / Sybil resistance** (R-12): the pitch's
  own guardrail already concedes this is a deployment-trust property; item 12
  makes the copy match. Code-level identity is a roadmap feature, not a fix.
- **Flipping defaults `write_back=false` and corpus-anchored `now`** (R-04/
  R-05 full): behavior changes to the measured retrieval path; this repo
  gates such flips behind its own measurement process (the R20 register row
  is already open for the anchor default). Interim: copy correction + error
  surfacing (items 9, 12).
- **SQL-level visibility pushdown + BFS budget + test rewrites** (R-20, R-23,
  R-24, R-15, R-18): real P2s, but each touches query semantics or test
  architecture; slot into the 30-day window, not this pass.
- **Dependency advisories**: quinn-proto is not in the default feature graph
  (CU-13) — record as accepted-risk with the feature note; crossbeam-epoch is
  dev-only; paste is a transitive unmaintained warning. No code change now.
- **Coverage tooling**: recommend `cargo llvm-cov` in CI as a follow-up;
  outside this review's write scope.

## Honest answer: do we meet the client promise (docs/pitch.md)?

**No — not as written, today.** Per claim:

| Claim | Answer |
|---|---|
| C1 determinism | **No as stated.** True only for read-only opens with anchored time; default path is wall-clock-dependent and self-mutating. Items 8, 9, 12 narrow the gap; full "yes" needs the default flips (deferred). |
| C2 zero model calls | **Yes** — structural, verified by all three (single `llm_client` call site, write path only). |
| C3 one SQLite file | **No** (three files). Copy fix (item 12) makes the claim true-as-restated. |
| C4 recall = FTS5+BM25 | **Partially** — FTS5+BM25 is real but the live pipeline is more; copy fix. |
| C5 recognition mechanism | **Yes.** |
| C6 typed 2-hop ontology graph | **Yes.** |
| C7 episodic/temporal | **Yes.** |
| C8 adaptive loop | **Half.** Strengthen: yes, default. Unused-decay: opt-in only, and absent from canonical paths; copy fix or ship a measured decay policy. |
| C9 federation visibility-scoped | **No until items 2–3 land** (then yes at the fan-out boundary). |
| C10 poisoning-resistant | **Only as score-flood resistance.** Authorship/tombstone forgery is open until item 1; Sybil is explicitly out of code scope. |
| C11 98.6% / C12 81.5% | **Cannot verify on this host** (dataset gone); recorded artifacts are internally consistent and self-labelled in-sample. Guardrails already require pairing the numbers. |
| C13 $0 per query | **Yes.** |
| C14 Apache-2.0 | **Yes except NOTICE placeholder** (item 12; needs your legal name). |

**HARD STOP per protocol. No code has been modified. Awaiting approval of the
plan (full, subset, or none) before Phase 5.**
