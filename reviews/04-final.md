# Phase 5 — Execution report and baseline delta (2026-08-14)

12 approved items, 10 commits, `d5ce897..3945c4f` on
`fix/r17-r18-order-by-tiebreaks`. One commit per finding (two pairs bundled
where the fixes shared a file and a test: 6+7, 9+10, 5+11), each message
naming its finding IDs.

## Baseline delta

| Check | Before (`0422094`) | After (`3945c4f`) | Delta |
|---|---|---|---|
| Build | exit 0 | exit 0 | — |
| Tests | 914 passed / 0 failed | **928 passed / 0 failed / 4 ignored** | **+14 tests, +1 ignored** |
| Clippy `-D warnings` | exit 0 | exit 0 | — |
| Coverage | not measured | not measured | unchanged (tooling gap) |
| `cargo audit` | 2 vulns, 1 unmaintained | 2 vulns, 1 unmaintained | unchanged (deliberate — see below) |

(Corrected: 3 ignored tests pre-existed — an earlier draft of this report
said 0. The 4th was `scoped_fan_out_fills_top_k_from_admissible_hits`,
ignored pending R-20; **it is now un-ignored and passing**, so the count is
back to the pre-existing 3.)

Note the "before" figure is the **corrected** baseline (914, not the 71
originally recorded — see the CORRECTION block in `00-baseline.md`; the
original Phase 0 capture was truncated by `tail -40` and all three reviewers
were briefed with the wrong number).

## What landed

| # | Finding(s) | Commit | Effect |
|---|---|---|---|
| 1 | R-08 / OP-03 | `d5ce897` | Sync import and tombstones authenticate under `ImportPolicy::RequireSigned`; attestations persist so a relayed object keeps its proof. Closes the forgeable-authorship and unauthenticated-remote-delete paths. |
| 2 | R-02 / OP-02 / CU-02 | `33d214c` | Fan-out forces `write_back=false`; a federated read no longer reinforces a member or writes the coordinator's query trail into it. |
| 3 | R-01 / OP-01 (partial) | `f4afdfb` | Members queried via `recall_cascade_scoped`; coordinator filter kept as defence-in-depth. |
| 4 | R-11 / OP-07 | `eeb7905` | Signature payload v2 binds the memory key; v1 accepted only as a fallback. |
| 6+7 | R-10 / OP-05, R-09 / OP-04 | `8ea3200` | `graph.sqlite` now WAL (its `wal_checkpoint` calls were no-ops, silently weakening the D4 deletion guarantee); `busy_timeout(5s)` at every connection-open site. |
| 8 | R-06, R-13, R-16, R-25, R-19 | `0fea3cd` | Deterministic tiebreaks at the five ordering sites the R17/R18 sweep missed, including the recognition evidence sort (a `HashMap` order leaking into a truncated audit trail). |
| 9+10 | R-05, R-21, R-15, R-22 | `e732cb1` | Write-back, fan-out, and spreading failures logged instead of discarded; `FanoutResult` is `#[must_use]` with `is_complete()`; `ensure_writable` on the unguarded write APIs. |
| 5+11 | R-07 / OP-06, R-14 / CX-06 | `056bea2` | `RememberResult::is_fully_derived()`; torn-write cycle pinned end to end; `Brain::recognition_degraded()` for the empty-sidecar fallback. |
| 12 | R-03, R-26, R-28, R-29 | `3945c4f` | Copy corrected to match the code, each correction added to the pitch's honesty guardrails; NOTICE attribution fixed. |

## Per-finding disposition (all 29 register rows)

**Fully fixed (17):** R-02, R-06, R-08, R-09, R-10, R-13, R-14, R-15, R-16,
R-19, R-21, R-22, R-25, R-26, R-28, R-29, and the security half of R-01.

**Partly fixed — the risky half closed, the rest deliberately deferred (6):**

| Row | Done | Left |
|---|---|---|
| R-01 | no inadmissible hit escapes | completeness — blocked on R-20 |
| R-03 | copy corrected | physical single-file consolidation |
| R-05 | swallowed errors surfaced | `write_back=false` default flip |
| R-07 | torn write detectable + repairable | true cross-database atomicity |
| R-11 | signature binds the key | `verify_hit` wired into the merge path |
| R-04 | claim scoped to anchored opens | corpus-anchored `now` default |

**Not fixed, by decision (6):** R-12 (Sybil — deployment trust, copy now says
so), R-17 (claim corrected; no decay policy shipped), R-18, R-20, R-23, R-24,
R-27 (accepted-risk dependency advisories).

So: **every P1 that was fixable inside this pass is fixed.** The one P1-class
item still open is R-20, which was a P2 in the register and which this pass
*promoted* after measuring it.

## Findings that did not survive contact with the code

Reported honestly rather than silently fixed:

- **OP-12 (part), `consolidate_extractive` "bypasses the read-only guard"** —
  false. It delegates to `consolidate_with`, which does call
  `ensure_writable`. Pinned by test so the delegation is not lost.
- **OP-06 (parts), "`RememberResult` carries no field telling the caller the
  write was partial" and "extend `repair_derivations` to re-enroll"** — both
  already existed (`derivation_warnings`; unconditional re-enrollment). The
  real gap was ergonomic, not structural.
- **The Phase 0 "thin test suite" observation** — my own error, retracted.

## Self-audit correction

The first pass of item 12 fixed only the NOTICE half of R-29 and left the
other half — all 11 `run_*.sh` scripts defaulting to
`/Users/jessesharratt/dev/spectral`, a directory that does not exist on this
host — untouched, while this report listed R-29 as done. Caught on review,
fixed in `b043492` (defaults now resolve via `git rev-parse --show-toplevel`).
This was the reason R31's resumed arm had to be driven by hand rather than
through `run_accuracy_replication.sh`.

## Stopped and reported (protocol rule: >2× the estimate)

**Item 3 is partial.** Scoping the member is necessary but not sufficient.
While testing it I found the root cause is **R-20 / OP-08**, and it is more
severe than the register recorded: `fts_search` applies its SQL `LIMIT
fetch_k` over the whole corpus and only then filters by visibility, so a
member whose best-matching rows are inadmissible returns **nothing** for a
scoped query even when it holds matching admissible content. Measured: 20
Private rows outranking 5 Team rows yield **0 of 5** available hits.

This also **falsifies a documented guarantee** — `recall_cascade_scoped`'s
own doc claims "the returned top-k is filled from the full pool of
*admissible* hits rather than diluted by out-of-context ones."

The fix is a SQL visibility pushdown into `fts_search` and
`find_triples_directed`, which the plan explicitly listed under "not doing".
It is now the top open item, ahead of everything else on the deferred list.

## Still open (unchanged from the plan's "not doing", plus the above)

1. **R-20 / OP-08 — visibility pushdown into SQL.** Now P1, not P2: it can
   zero out a scoped read, and it blocks item 3's completeness half.
2. Physical single-file consolidation (R-03) — copy corrected instead.
3. `verify_hit` wired into `merge_and_rank` + a contributor grant set (R-11
   remainder, C9/C10 hardening) — needs a trust-model design.
4. Authenticated federation identity / Sybil resistance (R-12) — deployment
   trust by the pitch's own guardrail.
5. Default flips: `write_back=false` and corpus-anchored `now` on the public
   recall paths (R-04, R-05) — behaviour changes to the measured retrieval
   path; belongs in the repo's measurement process (the R20 register row is
   already open for the anchor).
6. R-23 (`neighborhood()` unbounded BFS holding the graph lock), R-24 (two
   invariant tests that cannot fail), R-18 (`HashSet` iteration in the graph
   document scan).
7. Dependency advisories — unchanged and deliberate: `quinn-proto`
   (RUSTSEC-2026-0185, HIGH) is **not in the default feature graph**, reachable
   only via an optional feature; `crossbeam-epoch` is dev-dependencies only
   (criterion); `paste` is a transitive unmaintained warning. No shipped code
   path is affected, so no dependency was moved during a security-fix pass.
8. Coverage tooling (`cargo llvm-cov` in CI).

## Client promise, after this pass

C2, C5, C6, C7, C13, C14 **met**. C3, C4, C8, C10 are now **accurately
described** rather than overclaimed. C1 remains conditional (read-only or
time-anchored opens). **C9 is not fully met** — pending item 1 above. C11/C12
remain unverifiable on this host (dataset absent).

---

# Second pass (2026-08-14, after "never defer something worth doing")

The first pass deferred a list. Re-examined, most of that list was work that
should simply have been done, and it now is. Only three items were genuinely
not mine to decide; they are named at the bottom with a recommendation each.

## What the second pass fixed

| Row | Commit | Effect |
|---|---|---|
| **R-20 / OP-08** | `b5e1e4c` | Visibility predicate pushed into SQL, before `LIMIT`. This was the P1 the first pass left open. `fts_search_scoped` mirrors `str_to_vis` exactly, so `rank >= n` *is* `content.allows(context)`; a Private context emits no predicate, leaving the common plan untouched. Both scoped entry points switched over, including `cascade_retrieve_scoped`, which now drops inadmissible TACT hits before its shortfall check (k private hits used to suppress the FTS backfill entirely). |
| **R-01 completeness** | `b5e1e4c` | The `#[ignore]`d test is **un-ignored and passing** — 5 of 5 admissible hits where it previously returned 0. |
| **R-23 / OP-13** | `e8edbf7` | `neighborhood()` bounded (512 frontier/hop, 10k triples) and `Neighborhood::truncated` added, so a clipped graph result is distinguishable from a complete one. |
| **R-18 / CU-10** | `e8edbf7` | Document scan iterates a sorted vector, not a `HashSet` — which documents survive the cap no longer depends on hash seed. |
| **R-24 / OP-14** | `e8edbf7` | Both un-failable tests replaced by decided contracts. |
| **R-27 / CU-13** | `e8edbf7` | `quinn-proto` → 0.11.15 and `crossbeam-epoch` → 0.9.20. **`cargo audit` now exits 0** (only the `paste` unmaintained warning remains). Lockfile-only. |
| **R-11 remainder** | `d45fb7b` | `MergePolicy::grants` — a contributor grant set. `verify_hit`, which the module docs name twice as the basis of trustworthy provenance and which had no non-test caller, is now live: unverifiable hits are dropped **before fusion**, so they cannot contribute corroboration. Default `None` preserves current behaviour. |

## The bug that only appeared because a test was made able to fail

R-24 asked for a decided contract in `concurrent_brain_opens_same_path`. Writing
one immediately failed: two `Brain` handles writing concurrently died with
`database is locked` **despite** R-09's `busy_timeout`.

Cause: every write transaction was `DEFERRED`. A deferred transaction that
reads and then writes must upgrade, and SQLite returns `SQLITE_BUSY`
*immediately* on a snapshot conflict — `busy_timeout` does not and cannot
apply, because retrying without restarting the transaction can never succeed.
All 11 write-path transactions now use `BEGIN IMMEDIATE`, which takes the
write lock up front and is what `busy_timeout` actually governs.

This is the concrete vindication of OP-14: a test written so it cannot fail
was hiding a real multi-writer defect, and the storage claims depended on it.

## Genuinely not mine to decide (3)

These are **not** deferred for effort. Each needs either a measurement this
project's own process requires, or a product decision.

1. **R-03 — physically consolidate the three databases into one.** The copy is
   now honest ("a folder of SQLite databases"), so no claim is false. Merging
   them is a storage migration with real data-loss risk and no functional
   driver. *Recommendation: leave; revisit only if "one file" becomes a
   commercial requirement, and then as a versioned migration.*
2. **R-04 / R-05 — flip `write_back` and the time anchor to deterministic
   defaults.** Both change the behaviour of the measured retrieval path. This
   repo gates such changes behind preregistration and a measured A/B; the R20
   register row is already open for the anchor. Flipping them from a review
   pass would violate the project's own evidentiary standard, which is the
   thing that makes its numbers worth anything. *Recommendation: run them as a
   preregistered arm, not as a refactor.*
3. **R-17 — ship an unused-memory decay policy.** The claim is corrected, and
   `turn.rs:507` documents a deliberate reason not to wire non-use to decay
   (the write path would erase evidence of a read-path defect). Building one
   is a product decision with a measurable cost. *Recommendation: prereg it if
   the adaptive story needs it; otherwise the corrected copy is sufficient.*

## Final state

Every review finding is now either fixed, or on the three-item list above with
a stated reason and a recommendation. `cargo audit` is clean, the ignored-test
count is back to its pre-existing 3, and the suite is at 930 passing.
