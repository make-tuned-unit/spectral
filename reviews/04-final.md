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

## Coverage — the last baseline gap, now measured

Phase 0 recorded coverage as "not measured (no tool installed)" and no
reviewer could speak to it. Measured with `cargo llvm-cov --workspace`
(v0.8.7):

| | lines | regions | functions |
|---|---:|---:|---:|
| **Workspace total** | **67.34%** | **66.56%** | **67.69%** |

Most modules sit in the 90s (`policy.rs` 99.6%, `answerability.rs` 98.9%,
`render.rs` 96.7%). One outlier dominates the shortfall:
**`spectral/src/lib.rs` at 47.9%** — the public facade. That is where the
thin coverage is, and it is worth knowing that the crate's *front door* is
its least-exercised file even though the engines behind it are well covered.

Not turned into a finding here (no reviewer raised it and it is outside the
approved plan), but it is the obvious next test target, and `cargo llvm-cov`
in CI would keep the number honest.

## Final state

Every review finding is now either fixed, or on the three-item list above with
a stated reason and a recommendation. `cargo audit` exits 0, coverage is
measured at 67.34% lines, the ignored-test count is back to its pre-existing
3, and the suite is at 930 passing.

---

## Addendum: `Visibility::allows` had no direct test, and its own property test was tautological

Found after the review closed, prompted by an external note that a pure
verdict function needs at least one test aimed at the function itself.

`Visibility::allows` is the predicate every sovereignty guarantee reduces to.
It had **23 production call sites and zero direct tests**. Worse, the headline
sovereignty property test
(`spectral-graph/tests/property_invariants.rs::no_scoped_recall_ever_returns_an_inadmissible_hit`)
used `allows` as its *oracle*: production filtered with
`str_to_vis(..).allows(visibility)` and the assertion compared against
`parse_vis(&h.visibility).allows(*scope)`. Both sides shared the predicate, so
the test could only ever confirm that production agreed with itself.

### Measured, not argued

Four mutation runs, each with the source restored afterwards:

| mutant | old oracle | new oracle |
|---|---|---|
| `allows` inverted (`<=`) only | property test **passed** | property test **passed** |
| `allows` **and** the SQL rank predicate inverted | property test **passed** | property test **FAILED** |

The second row is the point. With the tautological oracle, inverting the
*entire admissibility rule in both implementations at once* — a total
sovereignty breach, private content served to public scopes — left the
property test green. It now fails.

### Why inverting `allows` alone does not leak

This was a genuine surprise and is worth recording. Inverting `allows` by
itself does not produce a leak through FTS recall, because the R-20 visibility
pushdown wrote the rule a **second, independent time** as the SQL rank
expression `VIS_RANK_SQL >= {vis_rank}`, which does not call `allows`. SQL
removes the inadmissible rows before the Rust `retain` ever runs. The
predicate being wrong in Rust is masked by the predicate being right in SQL.

That is real defence in depth rather than luck, but it is *undocumented*
defence in depth, and it cuts both ways: it also means a bug in `allows`
cannot be detected by any test that only exercises FTS recall. The 24 tests
that do catch the single inversion all reach `allows` through paths with no SQL
predicate — federation fan-out, spreading activation, cascade retrieval.

### Fixed

1. `spectral-core/src/visibility.rs` — a new `allows_truth_table` module
   enumerating all 16 (content, context) pairs with literal expected values,
   plus tests for asymmetry, reflexivity, and that the table contains both
   outcomes (so a collapsed all-true table cannot pass a matching bug). The
   doc comment forbids re-deriving the table from `>=`, `Ord`, or `allows`.
   Verified against three mutants — `<=`, `>`, and `true` — all killed.
2. The property test now asserts through `admissible_independently`, a
   hand-written rank comparison local to the test file, so production and test
   no longer share an implementation.

Suite: 1174 passing, `clippy --all-targets --all-features -D warnings` clean,
`fmt --check` clean.

### Generalisable lesson

A test whose oracle calls the function under test proves only self-consistency.
Two of this project's guarantees were being checked that way. The check that
catches it is cheap: invert the function and confirm *that specific test* goes
red, not merely that some test somewhere does.

---

## Addendum 2: both sweeps run against Spectral, and what the detectors miss

`permagent-runtime-87` ran two sweeps on its own codebase after the `allows`
fix and reported both clean. Both were run here. Result: Spectral is clean on
the first, and the second found a real gap one layer below the original bug.

### Sweep 1 — self-oracle detector: 34 candidates, zero real

Replicated the peer's construction (within one `assert_eq!`, split at the
top-level comma, flag any non-trivial call on both sides): **34 hits across
1446 assertion sites**, against its 35 across 4003. Every one is a false
positive, in the same two shapes it identified:

- **Determinism / discrimination properties** where calling the function twice
  *is* the claim — `entity_id("person","a:b")` vs `entity_id("person:a","b")`
  (a real pinned collision), `stem("boxes")` vs `stem("box")`,
  `from_descriptor("Laptop")` vs `from_descriptor("laptop")`,
  `scalar_bits(&a)` vs `scalar_bits(&b)`.
- **Test-local helpers on both sides** — `d(2024,3,31)` date builders, `to_bits`
  float comparisons.

The peer's distinction is the load-bearing one and it holds here:
*both-sides-same-call is only a self-oracle when one side is the pipeline and
the other reaches inside it.* Both sides being the subject is a determinism
test.

### The detector cannot see the bug that motivated it

Worth stating plainly, because "zero real" is easy to over-read. The bug this
all started from was:

```rust
assert!(label.allows(*scope), ...)
```

A **single-argument predicate**. There are no two sides to compare, so a
comma-splitting `assert_eq!` detector structurally cannot flag it. Both
sweeps' clean results are therefore evidence about a different shape than the
one that was actually broken.

### Sweep 2 — the detector for the shape that *did* break

Rebuilt around the real shape: a test asserting through a production
**predicate** (`fn -> bool`) that production also uses to make the same
decision. 38 such predicates outside `cfg(test)`, ranked by production call
count. `allows` sits at the top with 23 — the ranking put the actual defect
first, which is some evidence the metric is the right one.

Every security-relevant predicate below it was then checked individually:

| predicate | verdict |
|---|---|
| `allows` (23 calls) | **was the defect** — fixed in the previous addendum |
| `fully_forgotten` | clean — has a hand-built *sabotaged* report asserted false, plus independent raw-SQL residue counts. Mutating it to `true` fails `d2_sabotaged_deletion_is_detected_by_probes` specifically |
| `verify_memory_signature` | clean — round-trip is backed by real negatives (`tampering_any_signed_field_fails`, `resigning_under_a_different_key_fails`, `foreign_key_cannot_impersonate_origin`) |
| `admits`, `accepts_object`, `accepts_tombstone`, `verify_hit`, `is_complete`, `predicate_is_single_valued` | clean — not used as oracles |

A third shape neither detector covers is worth naming: the **round-trip
oracle**, `assert!(verify(sign(x)))`, which passes if both halves are wrong
compatibly. The defence is negative and known-answer tests, which
`identity.rs` has.

### Sweep 3 — doubled rules, triaged by whether violation is silent

The peer's sharpening of my point is the better formulation: *the danger is
not redundancy, it is redundancy over a rule whose violation is silent.* A
doubled row cap fails visibly the moment anyone counts; a doubled visibility
predicate fails invisibly, because the correct copy serves correct-looking
results while the broken copy waits for someone to delete the other one.

Spectral has exactly one instance of the dangerous variant: the visibility
rule, stated as `Visibility::allows` in Rust and as `VIS_RANK_SQL >= n` in
`spectral-ingest`. Everything else doubled in SQL is a bound or an ordering
(`LIMIT`, `ORDER BY` tiebreaks), where a mismatch shows up as a wrong count.

**This found a real gap.** The SQL copy — the one that actually enforces
visibility for FTS recall, and the only one left if someone deletes the Rust
`retain` as "obviously redundant" — had **no direct test**. Every existing
visibility test went through `spectral-graph`, where the Rust `retain` would
mask a bug in the SQL. So both copies were individually untested while the
pair looked covered, symmetrically.

Fixed: `sqlite_store::visibility_pushdown`, five tests placed in
`spectral-ingest` *specifically* because the Rust `retain` lives in another
crate and is not in that call path, so nothing there can be masked by it. They
pin the hand-written admissible set per scope, the negative direction, that the
predicate runs **before** `LIMIT` (the private row carries the top score, so
filtering after a `LIMIT 1` would return nothing), that a Private context
admits every label, and that an unknown label fails **closed**.

Mutation-verified, four mutants, all killed: predicate inverted (4/5 fail),
predicate removed entirely (4/5), unknown label defaulting to public (4/5),
`team`/`org` ranks swapped (1/5 — the set-equality test).

The Rust `retain` site now carries a comment saying why it must not be deleted
as redundant, naming both copies' tests.

### The triage question, as a standing check

Not "is this rule stated twice" but: **if both copies were wrong, would anyone
notice without a test?** If no, both copies need their own direct test, in a
place the other cannot mask.

---

## Addendum 3: count implementations, not call sites — and a fallback arm that hid behind a known label

`permagent-runtime-87` requalified its own sweep after the note that an
`assert_eq!` splitter cannot see `assert!(pred(x))`, reran with the
predicate-based detector (51 candidates, clean), and returned a sharper
discriminator than the one in Addendum 2:

> **Count implementations, not call sites.** N call sites of one function is
> defence in depth with no masking — mutating the function kills all N at once.
> Two *implementations* of one rule mask each other regardless of how few call
> sites each has.

That is correct and it reframes the `allows` case properly: the exposure came
from the second implementation, not from the first's 23 call sites. It also
means nothing about SQL was essential — two Rust functions encoding one rule
would be identically dangerous. Addendum 2's sweep only looked for
SQL-vs-Rust pairs, so it would have missed that.

### Sweep 4 — duplicate *implementations* in Spectral

Detector: a rule keyed on string literals leaves a fingerprint. Collect the
literal set of every production `fn`/`const` (outside `cfg(test)`) and flag
pairs in different items sharing most of their set. 282 items, 43 candidate
pairs at Jaccard ≥ 0.55. Most are duplicated bench fixtures (identical query
corpora across `recall_path_cost.rs` / `tact_tier_probe.rs`, verb lists across
probe harnesses) — harmless. The rest resolved as:

| candidate | verdict |
|---|---|
| `is_stopword` (recognition) vs `is_fts_stopword` (graph) | **two different rules, not two copies.** `FTS_STOPWORDS` deliberately excludes ambiguous content-homographs (`it`, `can`, `will`, `march`) that `STOPWORDS` includes. The divergence is load-bearing and *is* pinned: simulating the tempting "dedupe the two lists" refactor fails `does_not_drop_content_homographs` specifically |
| `predicate_is_single_valued` vs `..._pub` | delegation — one implementation, two surfaces. Safe by the discriminator |
| `visibility_to_str`/`str_to_vis` vs `str_to_visibility` (graph_store) vs `QuerySpec::visibility` (bench-real) | three label parsers, but on different data and with different strictness. See below |

### The asymmetry that makes visibility labels tricky

`Private` is the safe default in one direction and the dangerous one in the
other, for the same enum:

- as a **content** label, `Private` is maximally restricted → lenient parsing
  is fail-**closed**;
- as a **context**, `Private` is the admits-everything clearance → defaulting a
  scope to `Private` silently **widens** the query.

That is a real shipped bug, in `spectral-bench-real` (`visibility = "piblic"`
quietly measured a wider query), which is why scope parsing errors there and
content parsing is lenient in `str_to_vis`. All 11 `str_to_vis` call sites were
checked: every one parses a stored content label, and the context always
arrives as a typed enum. No lenient context fallback exists anywhere.

### The real finding: a fallback arm hiding behind a known label

`str_to_vis` matches `"team"`/`"org"`/`"public"` and sends everything else to
`Private`. Mutating `_ => Private` into `_ => Public` fails **9 tests**, which
reads as solid coverage.

It is not. `"private"` has no arm of its own, so the literal label flows
through the same `_` arm as unknown input, and all 9 catchers were exercising
the arm via the *known* label. Isolating the arm's two jobs — adding an
explicit `"private" => Private` and leaving `_ => Public` — the entire
workspace passed: **1179 green, with unknown labels failing OPEN.**

A corrupt, hand-edited, or future-schema label (`partner`) would have been
readable in every scope. The SQL copy could not cover for it either: its
`ELSE 0` arm only guards FTS recall, while federation fan-out, spreading
activation and cascade retrieval filter in Rust with no SQL predicate ahead of
them.

Fixed: `brain::str_to_vis_defaults`, four tests pinning the write→read
round-trip, `"private"` specifically, twelve unrecognised labels failing closed
(including the `piblic` typo, `PUBLIC` for case-sensitivity, and a plausible
future `partner`), and the consequence stated directly — an unknown-labelled
row must be no more admissible than a private one, in every scope. The
isolating mutant that previously passed 1179/0 now fails 2 of the 4.

The SQL side's identical double-duty `ELSE 0` arm was re-checked with the same
isolating mutant and **is** caught by `an_unknown_label_is_treated_as_private_not_public`
from Addendum 2, so that claim holds.

### Generalisable: mutation testing lies about catch-all arms

A `_`/`else`/`default` arm that also serves a known input gives a false green.
Mutating it appears covered, but the coverage comes from the known input, not
the fallback. **To test a default arm, isolate it first** — give the known
input its own arm, then mutate the fallback. If the suite still passes, the
default was never tested. This is the same claim-substitution error as
"the suite caught it": the mutation was caught, but not for the reason it
appeared to be.
