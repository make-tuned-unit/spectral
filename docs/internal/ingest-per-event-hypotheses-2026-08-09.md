# Track C — what the unprofiled 73% of per-event ingest cost actually is

**Code reading, not measurement.** Nothing here is a claim. It converts the
decomposition's black box into named, separately-measurable hypotheses and
specifies the profile that would settle them.

## The gap being explained

`ingest-gap-decomposition-2026-08-03.md` decomposed Spectral's 0.318 ms/event
against MinHash+BM25:

| component | ms/event | share |
|---|---:|---:|
| batched insert + FTS5 floor | 0.017 | 5% |
| per-event transaction commit | 0.068 | 21% |
| **Spectral's own per-event work** | **0.233** | **73%** |

It attributed the 73% to "classification, signal scoring, episode/session
handling, content hashing" and called it "currently a black box." **That
attribution was a guess, and the code suggests it is largely the wrong one.**

## What `Brain::remember_with` actually does per event

After the single `spectral_ingest::ingest::ingest(...)` call — which is the part
containing classify/score/hash — the method performs, **per event**:

1. **Session association write** (`block_on`, when `session_id` is set — the
   bench always sets it via `episode_id`/`session_id`).
2. **Declarative density**: computed in-process, then a **separate store
   UPDATE**.
3. **Read-back of the just-written row** (`get`), to recover the persisted
   `content_hash` and `created_at`.
4. **Ed25519 signing** over that payload, then a **signature write**.

That is up to **four additional round trips and one asymmetric-crypto
operation** *after* the insert that the decomposition modelled as the cost.

## Hypotheses, in the order I would bet on them

- **H1 — signing dominates.** Ed25519 sign is ~20–50 µs on this class of
  hardware, and it is bracketed by a read-back query and a signature write.
  Plausibly the single largest line item, and it is **pure overhead against
  MinHash+BM25, which signs nothing.** This is the auditability edge showing up
  as a throughput cost — a real trade, but it should be *priced*, not hidden
  inside "classification".
- **H2 — the density UPDATE is a second write per event.** `declarative_density`
  is cheap to compute; issuing it as its own UPDATE is not. It is a strong
  candidate to fold into the initial insert, which would remove a whole
  round trip for zero behaviour change.
- **H3 — the read-back is avoidable.** It exists only to learn what the store
  persisted. If `ingest` returned the stored `content_hash`/`created_at`, the
  query disappears.
- **H4 — classify/score/hash.** The decomposition's stated cause. Still real,
  but on this reading it is competing with three round trips and a signature
  for the 0.233 ms.

**If H1–H3 hold, most of the 73% is round-trip and crypto overhead that
batching and plumbing can remove without touching the classifier** — a very
different remediation than "optimise classification."

## The profile that would settle it

Opt-in per-stage timing behind an env var (`SPECTRAL_INGEST_PROFILE=1`),
accumulating nanos per stage into a thread-local and dumping totals at process
exit. Stages: `ingest_call`, `session_assoc`, `density_compute`,
`density_write`, `readback`, `sign`, `sig_write`.

Gated so the shipped path is untouched, and reported as ms/event alongside the
0.233 ms figure it is decomposing.

**Must run on an otherwise-idle machine.** Timing an ingest while another
ingest is running produces numbers that mean nothing; this was written while
R24 occupied the machine, which is exactly why it contains no measurements.

## Honest note on whether this matters

Ingest throughput is a **competitive-positioning** gap, not a consumer-binding
one. Permagent — our only user — operates at a corpus size where 428 ev/s and
3,148 ev/s are indistinguishable in practice. The reason to close it is that our
own record claims we "lose the systems axes," and an unprofiled 73% means we
cannot say *why* we lose or what it would cost to stop losing.

Retrieval quality (R24) should stay ahead of this in priority. This is
worth doing because it is cheap and because "black box" is not an acceptable
state for a number we publish about ourselves.

**Refs:** `ingest-gap-decomposition-2026-08-03.md`, `phase0-rerun-2026-08-03.md`,
`PHASE0_RESULTS.md`.
