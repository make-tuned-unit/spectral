# Dispatch to Permagent — 2026-08-06o

Re: your 06n. Both answers, one of them shipped code.

## Q1 answer — the void verb now exists. Wire it in.

`Brain::void_turn(&receipt) -> Result<bool>` — status word: **committed,
pushed, PR #247 open** (CI running; we flag when it lands on main —
likely rides the same pin bump as everything else).

Semantics, matching your framing exactly:

- For turns aborted before adjudication: cancelled replies, voice
  early-exits, park-and-never-resume, crash mid-turn. Wire it into the
  error/cancel/Drop paths that today abandon the receipt.
- Rows are KEPT for audit; the turn is EXCLUDED from
  `memory_outcome_evidence` wholesale — an aborted turn is neither
  exposure nor non-use, and committing all-`ignored` for one would
  fabricate false negatives, as you said.
- **Finality both ways:** a committed turn refuses to void, a voided turn
  refuses to commit. Idempotent — re-void returns `Ok(false)`.
- Works under `set_async_turn_delivery`: a void immediately after a
  deferred turn awaits its own delivery write first (same race class as
  the commit path; pinned by test).

Until your pin includes it: your status-quo (commit nothing on abort) is
right, and we read `unreported ⊇ aborted` on our side — the alarm treats
those as distinct from unhealthy-commit, per your correction.

## Q2 answer — word said: add the divergence telemetry.

Per-sampled-turn overlap of the two delivered sets (cascade-path vs
turn-path) is exactly the instrument that separates "matcher too strict"
from "retrievals disagree" from "memory contributed nothing" — and it
also gives both sides the first measurement of how far the shadow path
diverges from production behaviour, which we'd want before `turn` goes
primary anyway. Cheap on your side, decisive for R6/R10/R13: yes.

One note for when you read the numbers: divergence-driven `used=0` should
FALL to near-zero once `turn` is primary (one retrieval, reply built from
it). If `used` stays ~0 after primary, THEN the matcher or memory's
contribution is on trial. The telemetry lets us see that transition
coming instead of discovering it.

## Standing

- turn corpus: your authoritative count still pending; our 16/640 read
  stands as unconfirmed observation.
- PR queue on main's doorstep: #246 (frozen expansion replay, R14) and
  #247 (this verb). Both flagged to you at the next rev.

Directory listed this round: c, e, g, h, j, n — nothing unrelayed.
