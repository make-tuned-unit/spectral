# Dispatch → Permagent: activate the dormant recognition/ambient wins

**TL;DR:** two of Spectral's namesake subsystems were built but never fed — they
do nothing today because the consumer isn't passing the signals. Both are
measured, deterministic, `$0`, embedding-free, and on `feat/dormant-subsystems-
measured`. Turning them on is mostly your-side wiring, not new Spectral work.

## 1. Ambient context boost — one-line activation, measurable disambiguation

**What it does:** disambiguates a vague query by what the user is doing *right
now*. Same query "review notes" → the *work* note when they're in the work
context, the *recipe* note when cooking, etc.

**Why it's dormant:** `apply_ambient_boost` is ON by default in the cascade
path, but every recall gets `RecognitionContext::empty()` — so the boost is
identity. **It fires the moment you populate the context.**

**Measured** (`ambient_weight_sweep`): with the new penalty-only default
(`wing_match=1.0`, `mismatch=0.7`) it disambiguates **11/12** ambiguous cases
and **never hijacks an explicit query** (6/6) — an explicit query's relevance
survives the 0.7 damp; only ambient noise is suppressed.

**What we need from you:** on your recall calls, build a real
`RecognitionContext` instead of `empty()`:
- `focus_wing` = the wing the user is currently in (you already have this signal
  — e.g. `"permagent"` when they're in that surface, or the active
  project/context wing).
- `recent_activity` = last-N activity episodes (you already ingest an activity
  wing).
- `now` = query time (you already anchor this for recency).
That's it — no new API. `recall_cascade(query, ctx, cfg)` already threads it.
Consider A/B: same query set with `empty()` vs populated context, measure
top-1 correctness on genuinely ambiguous queries.

## 2. Spectrogram cross-wing resonance — a product feature, not a tweak

**What it does:** `recall_cross_wing(seed)` surfaces memories that share a
*cognitive shape* with the seed across unrelated life domains, on 7
deterministic dimensions (entity density, action type, decision polarity,
causal depth, emotional valence, temporal specificity, novelty) — **no keyword
overlap, no embeddings**.

**Measured** (`spectrogram_resonance_bench`): given a new *decision*, resonance
found the user's decisions across four unrelated domains (health, home,
finance, travel) sharing **zero keywords** — **4/4 found, 0/4 action-type false
positives**, vs an FTS structural ceiling of 2/4. This is the "you've decided
like this before" / "you've hit this kind of problem before" recall that
keyword and vector search can't do cheaply.

**Why it's dormant:** requires `enable_spectrogram: true` at brain open (default
false); fingerprints are written at `remember` time when enabled.

**What we need from you (a product decision, not just wiring):**
- Set `enable_spectrogram: true` in your `BrainConfig`. (Cost: a deterministic
  fingerprint written per memory — we're quantifying the write overhead now;
  will report exact numbers.)
- Decide the **surface**: this is a feature — "similar past decisions", "related
  patterns across your life", the recognition-aware nudge Henry can give
  ("last three times you decided under time pressure, you regretted the rushed
  option"). Tell us where it lives and we'll shape `recall_cross_wing`'s output
  (currently seed + resonant hits with per-dimension match evidence) to fit.
- The 7 dimensions are wing-agnostic and privacy-safe (numeric fingerprints, no
  content leaves the store) — good for cross-domain surfacing without exposing
  raw memories.

## What we're doing on our side (no action needed from you)

- Ambient weights are now tunable (`CascadePipelineConfig::ambient_weights`) with
  the measured frontier default — retune later from your real usage data.
- Pushing spectrogram to its limit now: tunable `MatchTolerances` on
  `recall_cross_wing` + a scale precision/recall sweep + write-cost numbers.
  Will report the frontier and any default change before you pin.

## The ask, distilled

1. **Ambient:** populate `RecognitionContext` (focus_wing + recent_activity) on
   recall — cheapest measurable lift, no new API. Confirm you can wire it.
2. **Spectrogram:** decide whether to enable it and where resonance surfaces in
   the product — then we finalize the output shape together.
