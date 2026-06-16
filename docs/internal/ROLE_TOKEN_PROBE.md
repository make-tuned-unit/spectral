# Role-Aware Token-Reduction Probe

**Status**: Retrieval-only probe. FREE (no actor, no judge, no API, no bench
run). Simulation on the published admitted set — no library code changed.
**Main SHA**: post-#173 (`dec1242`).
**Source**: `~/spectral-local-bench/eval-report-n500.json` (the published
expansion-ON run) + `longmemeval/longmemeval_s.json` (gold `has_answer`
turns). n=492 instrumented questions (8 transport failures excluded).
**Recall probe result**: PASS — capping assistant turns at 36% of the
admitted set cuts **~40% of retrieved-context tokens (~6.4k tok/query) with
0.00pp answer-key recall loss in every category.**

> **⚠️ FINAL VERDICT (paid confirmation gate, §5): SHELVED.** The recall pass
> did **not** hold on accuracy. Single-session-assistant regressed ~5pp
> (beyond the ±2pp band) under the cap, even though the gold answer turn was
> retained — the non-answer assistant context was load-bearing for SSA
> synthesis. Recall-viability was necessary but not sufficient. The cap is
> implemented but inert (env-gated, default-off); **do not ship as default.**
> See §5.

---

## 0. Premise check — is there a published Memanto tokens/query figure?

**No.** Every Memanto reference in the repo concerns *accuracy*, not tokens:

- `BENCH_METHODOLOGY.md:492,504` — Memanto tokens/query and $/1k are
  explicitly **"Undisclosed"**; only accuracy (~89.8% LongMemEval) is cited.
- `item-20-judge-proposal-v2.md:359-363` — Memanto's 89.8% as an accuracy
  calibration reference.
- `backlog.md:71` — architectural framing, no token figure.
- `RUN_NOTES.md` / a dedicated external-research-synthesis token figure: no
  such file/figure exists (`external-research-synthesis-2026-05-12.md`
  contains no tokens/query number).

**Therefore the "worse than Memanto on tokens" comparison is UNSOURCED.**
This probe is framed around **absolute token reduction under the recall
guard**, not a competitor target.

---

## 1. Baseline (current main retrieval, expansion-ON)

Admitted set = the published run's `retrieved_memory_keys` (per-type-K
admission, role-labelled `…:turn:N:user|assistant`). Token baseline = the
run's measured `actor_input_tokens` (system tokens minus expansion).

| Category | n | Mean admitted turns | Assistant % of admitted | Mean actor-input tok |
|----------|---|--------------------|-------------------------|----------------------|
| knowledge-update | 78 | 46.9 | 63.0% | 16,358 |
| multi-session | 131 | 55.0 | 65.8% | 19,947 |
| single-session-assistant | 55 | 41.8 | 65.7% | 13,902 |
| single-session-preference | 25 | 39.6 | 61.2% | 14,380 |
| single-session-user | 70 | 38.7 | 65.9% | 14,106 |
| temporal-reasoning | 133 | 40.3 | 61.5% | 14,246 |
| **OVERALL** | **492** | **45.2** | **64.1%** | **16,047** |

**Role-split confirmation.** The cited ~59.7%-assistant figure is **low for
this run**: assistant turns are **64.1% of admitted keys** (and 65.4% of the
rendered `actor_context` turns). Assistant-turn dominance is real and slightly
worse than reported.

**The asymmetry that makes this probe interesting.** Assistant turns are 64%
of the *context* but carry almost none of the *answers*:

| | Gold answer-turns that are USER | …that are ASSISTANT | Assistant % |
|---|---|---|---|
| knowledge-update | 144 | 0 | 0.0% |
| multi-session | 326 | 1 | 0.3% |
| **single-session-assistant** | **5** | **51** | **91.1%** |
| single-session-preference | 44 | 0 | 0.0% |
| single-session-user | 64 | 2 | 3.0% |
| temporal-reasoning | 259 | 0 | 0.0% |
| **TOTAL** | **842** | **54** | **6.0%** |

Only **6.0%** of gold answer evidence lives in assistant turns — and that 6%
is **concentrated in single-session-assistant (91.1%)**, which by definition
asks what the assistant said. Every other category's answers are 0–3%
assistant. This predicts the result: assistant turns are mostly removable
ballast, *except* in SSA — the binding canary.

**Baseline answer-key recall** (all-instances, turn-level pooled —
`has_answer` turns admitted / total `has_answer` turns): **88.8% overall**
(95.1% KU, 87.0% MS, 96.4% SSA, 70.3% SSP, 89.4% SSU, 89.2% temporal).
Consistent with the session-level 93.2% in `ALL_CATEGORY_RECALL_AUDIT.md`
(turn-level is stricter).

---

## 2. Candidate — assistant-turn cap in admission

**Smallest change, admission ordering only.** After the rerank pipeline
produces the final ranked hits, cap assistant turns at a fraction `f` of the
admitted size: keep **all** user turns in rank order, keep the **highest-
ranked** assistant turns up to the cap, drop the lowest-ranked assistant
turns. K, TACT, reranking signals, routing, and expansion are untouched.

**Where it goes** (proposal, not applied): `spectral-graph/src/
cascade_layers.rs`, immediately after `apply_reranking_pipeline` (line 183),
before `Ok(results)` (line 209):

```rust
// Cap assistant turns at `f` of the admitted set (user turns preserved).
let cap = (results.len() as f64 * ASSISTANT_CAP_FRAC).floor() as usize;
let mut kept_asst = 0;
results.retain(|h| {
    if h.key.ends_with(":assistant") {
        let keep = kept_asst < cap; kept_asst += keep as usize; keep
    } else { true }
});
```

**Measurement method.** Because the transform is a pure re-admission of an
already-ranked set, it is applied directly to the published admitted keys —
this measures the delta against the *exact* published baseline with no
retrieval drift. Token estimate per turn = `len(content)/4 + 5` (the system's
own estimator, `result.rs:9-10`); validated against ground truth: estimated
baseline turn-tokens mean **15,989** vs real `actor_input` mean **16,047**
(ratio 1.00 — turn content is essentially the entire actor input).

---

## 3. Trade curve (token reduction vs. per-category recall loss)

Pre-committed gate (same as the K-sweep): viable only if all-instances
answer-key recall loss **≤1pp in EVERY category**. Temporal and multi-session
are the usual canaries; here the binding canary is **single-session-assistant**.

| Cap `f` | Token reduction | Min category Δrecall | Binding category | Gate |
|---------|-----------------|----------------------|------------------|------|
| 0.50 | −19.8% | 0.00pp | — | ✅ PASS |
| 0.44 | −29.2% | 0.00pp | — | ✅ PASS |
| 0.40 | −36.2% | 0.00pp | — | ✅ PASS |
| 0.38 | −37.3% | 0.00pp | — | ✅ PASS |
| **0.36** | **−40.3%** | **0.00pp** | — | ✅ **PASS (max viable)** |
| 0.34 | −42.8% | **−1.82pp** | single-session-assistant | ❌ FAIL |
| 0.32 | −46.0% | −1.82pp | single-session-assistant | ❌ FAIL |
| 0.30 | −47.0% | −3.6pp | single-session-assistant | ❌ FAIL |
| 0.20 | −61.0% | −8.9pp | single-session-assistant | ❌ FAIL |
| 0.00 (drop all asst) | −89.7% | −87.5pp | single-session-assistant | ❌ FAIL |

The curve is flat-then-cliff: recall is untouched down to f=0.36, then SSA
falls off as the cap starts evicting high-ranked gold assistant turns. KU/MS/
SSP/SSU/temporal never lose recall at any cap ≥0.30 (their answers aren't in
assistant turns).

---

## 4. Result — PASS at f = 0.36

**Cap assistant turns at 36% of the admitted set:**

- **−40.3% retrieved-context tokens** (~6,443 tok/query saved; mean actor
  input 16,047 → ~9,604).
- **0.00pp recall loss in every category** — on both metrics (pooled
  turn-level and per-question all-gold-present):

| Category | Pooled recall base→cand | All-gold-hit base→cand |
|----------|-------------------------|------------------------|
| knowledge-update | 95.1 → 95.1 (+0.00) | 92.3 → 92.3 (+0.00) |
| multi-session | 87.0 → 87.0 (+0.00) | 80.9 → 80.9 (+0.00) |
| single-session-assistant | 96.4 → 96.4 (+0.00) | 96.4 → 96.4 (+0.00) |
| single-session-preference | 70.3 → 70.3 (+0.00) | 60.0 → 60.0 (+0.00) |
| single-session-user | 89.4 → 89.4 (+0.00) | 90.0 → 90.0 (+0.00) |
| temporal-reasoning | 89.2 → 89.2 (+0.00) | 86.5 → 86.5 (+0.00) |

This ~40% retrieved-context reduction is the **free token savings** available
under the recall guard. The assistant ballast (high-ranked enough to be
admitted, but not answer-bearing) is the slack.

### Caveat / next gate (do NOT run now)

Answer-key recall holding ≠ actor accuracy holding. The actor may use
non-answer assistant context for phrasing, disambiguation, or temporal
anchoring in ways recall does not capture. **Before shipping, the next gate
is a paid actor-replay** of the f=0.36 admission on the frozen STABLE_FAIL 26
(or a small stratified sample incl. SSA + multi-session), confirming accuracy
is within the ±2pp noise band. That is a separate, paid step — flagged, not
run. No shipping decision here.

### Why recall holds for SSA at f=0.36 (necessary, not sufficient)

SSA is the only category whose answers are assistant turns, and those gold
turns are high-ranked (relevance to "what did you tell me…"). The cap evicts
only the *lowest-ranked* assistant turns, so SSA's gold survives until f<0.36.
**This is why recall held — but §5 shows accuracy did not.**

---

## 5. Confirmation gate (PAID) — FAIL → SHELVE

**Status**: Paid actor-replay, Sonnet 4.6 actor + judge, expansion-ON. Cap
implemented at `cascade_layers.rs` (post-`apply_reranking_pipeline`, env-gated
`SPECTRAL_ASSISTANT_CAP_FRAC`, default-off; canonical block green). Harness:
`crates/spectral-bench-accuracy/src/bin/cap_confirm.rs` — paired uncapped vs
capped on the **same** brain + expansion + formatter (only admission differs),
n=5 per arm. Sample: 8 currently-passing single-session-assistant (the canary,
each with its gold answer in an assistant turn) + 4 passing multi-session.
Spend ≈ $7. No full n=500 re-run.

### Per-category accuracy, capped vs uncapped (n=5/case)

| Category | n cases | Uncapped | Capped | Δ accuracy | Token cut | Gate (±2pp) |
|----------|---------|----------|--------|-----------|-----------|-------------|
| multi-session | 4 | 20/20 = 100% | 20/20 = 100% | **+0.0pp** | −38.7% | ✅ pass |
| single-session-assistant | 8 | 40/40 = 100% | 38/40 = 95% | **−5.0pp** | −39.5% | ❌ **fail** |
| overall | 12 | 60/60 = 100% | 58/60 = 97% | −3.3pp | −39.2% | fail |

Two SSA cases each flipped 5/5 → 4/5 (`18dcd5a5`, `1d4da289`).

### Why it failed — the ballast mattered

In **both** regressed cases the gold assistant answer turn is the
**highest-ranked assistant turn (rank 0) and was retained** by the cap —
recall did not break. The actor still flipped reps with the answer turn
present. So the loss is not a retrieval miss; it is **synthesis**: removing
the surrounding non-answer assistant context degraded the actor's ability to
answer SSA questions. This is exactly the failure mode the gate was built to
catch — *recall held but accuracy didn't, so the assistant "ballast" was
load-bearing for SSA.* The recall-only probe (§4) was necessary but not
sufficient.

**Statistical honesty**: n=5 × 8 SSA cases = 40 reps; 2 single-rep flips is a
−5pp point estimate but not statistically significant at this n (40/40 vs
38/40, Fisher p≈0.5). However, the pre-committed gate is a ±2pp point-estimate
band, not a significance test — −5pp exceeds it — and the retained-gold
mechanism points to a real (if small) synthesis effect rather than pure noise.
The conservative, pre-committed call is to shelve.

### Decision

**SHELVE the global f=0.36 cap.** The −40% token cut is *not* accuracy-safe as
a routing-agnostic global cap: it regresses the SSA canary beyond the noise
band. Per the pre-committed gate, not merge-ready. The implementation is kept
**inert** (env-gated, default-off) for reproduction; do not flip the default.

**Possible future direction (not pursued here):** the cap is clean on
multi-session (0pp) and recall held for KU/SSP/SSU/temporal — a *category-aware*
cap that exempts SSA could recover most of the savings, but that requires a
routing change (explicitly out of scope for this candidate) and its own gate.

---

## Provenance

Pure aggregation/simulation over `eval-report-n500.json` (admitted keys,
`actor_input_tokens`) and `longmemeval_s.json` (`has_answer` gold turns).
No library code changed → no canonical block required. The candidate is
specified as a proposal (§2) for the future paid-replay gate.
