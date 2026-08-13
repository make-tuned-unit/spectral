# R17/R18 tiebreak sweep — paired verification result (2026-08-13)

**$0, retrieval-only oracle, LoCoMo answerable-labelled (regenerated, membership
matches R19: 1438/2140), N = 400 per arm, both `topk_fts` and `cascade`.
Preregistered in `tiebreak-paired-verification-prereg-2026-08-13.md` before any
arm ran.**

## Result

| pair | context diffs | pure reorder | set change | evidence turns | tokens |
|---|---:|---:|---:|---:|---:|
| `base_topk` vs `tie_topk` | **0/400** | 0 | 0 | 366/578 both | 790,259 both |
| `base_casc` vs `tie_casc` | **0/400** | 0 | 0 | 362/578 both | 598,550 both |

**Zero context-hash differences on either path.** Evidence-turn recall and
estimated token totals are identical to the row. The escalation rule (>0 diffs
→ full N) did not fire.

## What this says, and what it does not

- **Says:** the R17/R18 tiebreak sweep does not move bench retrieval at this
  sample on either path. The two plausible reachability routes named in the
  prereg — episode assignment at ingest via `find_recent_episode` ties, and
  tier-1 `fingerprint_search` — produced no observable effect: same-second
  `ended_at` ties either did not occur or did not change assignment, and
  tier-1 has nothing to fire on in a wing-less corpus.
- **Does NOT say:** that the sites are unreachable in production, or that the
  change is a no-op. The R16 lesson applies verbatim: with
  `idx_memories_wing_recency` the planner already emitted the pinned order at
  the prune site, so on this build several clauses are plan-neutral **today**.
  The clauses exist so the invariant does not rest on the planner's choice —
  in particular at the DELETE boundary, where the untiebroken form let the
  query plan decide which rows are destroyed.
- **No accuracy claim** — there is nothing to claim; retrieval is
  byte-identical.

## Verification of the tests themselves

The three new pinned tests were run against a build with the clauses reverted:
**all three fail** (and pass with the clauses present). The first construction
was vacuous for two of three — recorded in the prereg — because tied-key
emission differs by plan: an index scan emits the index's own trailing-key
order, a temp sort emits insertion order. The committed tests run both
insertion orders on independent stores, and the prune test additionally
exercises both plans (index present and dropped).

## Process deviation, recorded

The prereg file was written before any arm ran, but — unlike every prior
prereg in this register — it was **not committed** before the arms, so there
is no commit hash proving precedence, only file mtimes in one working tree.
For a no-gate baseline-shift quantification the stakes are low, but the
discipline exists precisely so that claim never has to rest on trust. Next
prereg gets committed first.

## Environment

Different host from all prior register runs (Intel Mac, `/Users/j`). Both
binaries built from the same tree and target dir (baseline at `b3375e8`,
arm = baseline + the tiebreak commit); dataset regenerated on this machine
with `locomo_to_oracle.py --all`. Paired comparisons are internally valid;
no cross-session absolute is cited.

**Refs:** `tiebreak-paired-verification-prereg-2026-08-13.md`,
`r16-baseline-shift-2026-08-07.md` (the class), REPAIR_REGISTER R17/R18.
