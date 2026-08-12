# R30 — does adjacency improve ANSWERS? · **NULL on the primary**

**$0, fully on-device.** ollama `qwen25-16k` (7.6B Q4) as actor and judge,
temp 0, no cloud calls. LoCoMo multi-session, **n = 280** (the complete
category), `--retrieval-path cascade`, single variable. Preregistered at
`9ffd8cd` before any arm ran; graded-metric amendment registered at `648781f`
while A0 was still running and **before A_ADJ existed**.

## Result

| metric | A0 | A_ADJ | Δ | statistic | verdict |
|---|---:|---:|---:|---|---|
| **containment (PRIMARY)** | 11.79% | 13.57% | **+1.79pp** | McNemar exact **p = 0.3833** (13 vs 8) | **NULL** |
| local judge (secondary) | 12.14% | 13.21% | +1.07pp | McNemar exact p = 0.7111 (16 vs 13) | null |
| item recall (graded) | 22.99% | 26.71% | **+3.73pp** | Wilcoxon **p = 0.0280** (60 changed) | nominally positive |

Context: **1,492 → 3,361 tokens (2.25×)**, matching R29's independently measured
2.27×.

**Verdict on the registered rule: NULL.** The primary metric shows no detectable
effect (p ≥ 0.05).

## What this means, stated plainly

**+18.22pp of evidence recall — and +7.57pp over a token-matched control —
buys at most a couple of points of answer quality here, and nothing that clears
the primary's significance bar.** This is the conversion the entire retrieval
programme has assumed and never tested. It is not established.

That is the headline and it should not be softened by the graded metric.

## The graded metric, and why it is not a PASS

Item recall is **+3.73pp at p = 0.0280**, and all three metrics point the same
direction. Three reasons that is not a win:

1. **It is a secondary**, registered mid-flight. The primary was fixed in
   advance and it is null.
2. **Three metrics were tested.** Bonferroni puts p = 0.0280 at **~0.084** —
   not significant. Quoting 0.0280 without that correction would be exactly the
   selective reporting this register exists to prevent.
3. It is the *most* favourable framing on the *most* favourable slice.

The honest reading is a **small positive trend consistent across three
measures, none of which survives its own bar.**

## The genuinely good news, which was a registered risk

**2.25× context did NOT hurt.** The prereg named reader-dilution as the real
danger — "context that lowers accuracy is a regression, not a win" — and it did
not materialise on any of the three metrics. The drowning hypothesis is not
supported at this scale.

That is a real result: it removes the main *objection* to spending the context,
without supplying a *reason* to.

## Why this is weak evidence, in both directions

- **Floor effect, near-miss.** A0 at 11.79% sits just above the 10% line this
  prereg registered as UNINFORMATIVE. A 7B Q4 reader on the hardest category is
  a blunt instrument; the published cloud-actor figure for multi-session is
  39.64%.
- **Underpowered for what was observed.** The prereg put the detectable effect
  at ~8pp. The observed primary effect is 1.79pp. **This run could not have
  detected the effect it found**, which is why the graded metric was added — and
  why "no detectable effect at this N with this reader" is the correct phrasing,
  **never "no effect"**.
- **Not comparable to any published number.** Local judge, 384-token cap,
  n = 280 of one category.
- **A stronger reader could go either way.** It might convert the extra evidence
  the 7B model wasted — or need it less, having been better at the 1,492-token
  context to begin with. Untested.

## Hygiene

280/280 both arms, 0 empty predictions, 0 transport failures. Judge-parse
failures 4 (A0) and 5 (A_ADJ) — the known R21 class, affecting **only** the
secondary judge metric; the deterministic primary reads `predicted` directly
and is immune. That is the payoff for making the model-free metric primary.

A0's mean context (1,492) independently reproduces R29's cascade baseline
(1,500), confirming the retrieval configuration was what it was believed to be.

## What follows

- **No accuracy claim for adjacency.** R29's retrieval PASS stands on its own
  terms and does not license one.
- **The conversion assumption stays open**, and it is now open *with evidence*
  rather than by omission. The correct next test is a **stronger reader**, which
  needs the bench key rotated and a budget — the one thing $0 cannot buy.
- **Not authorised by this prereg:** full-corpus replication, other categories,
  or any subgroup analysis. No post-hoc subgroups were run, deliberately.

**Refs:** `adjacency-accuracy-prereg-2026-08-11.md`,
`cascade-token-match-result-2026-08-11.md` (R29),
`adjacency-mechanism-diagnostic-2026-08-11.md`.
