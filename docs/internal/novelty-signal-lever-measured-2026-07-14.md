# Lever measured & rejected: novelty → signal_score (2026-07-14)

**Question.** The write path already computes recognition familiarity
(MinHash-based, in `brain.rs` recurrence feedback) but uses it only to reinforce
priors. Its novelty (`1 − familiarity`) never reaches `signal::score_memory`.
Should we fold that novelty into the signal score — demoting redundant
restatements, boosting genuinely-new memories?

**Answer: no.** Measured all-downside, no upside. Keep novelty a separate
dimension (spectrogram / recurrence feedback), not mixed into signal.

## Why

Novelty (novel ↔ redundant) and durability (durable-fact ↔ ephemeral-chatter)
are **orthogonal axes**. The AAAK always-in-prompt bar is a *durability*
threshold (hall ∈ {fact,preference,rule,decision} AND signal ≥ 0.70). Folding an
orthogonal axis into that threshold can only move points along a direction that
carries no information about whether they belong above or below it.

`novelty_signal_probe` (deterministic, $0) enrolls an 8-memory corpus, then
scores five stimuli spanning both axes and sweeps the novelty weight `w`:

| stimulus            | durable | novelty | base | symmetric (w=.15) |
|---------------------|:------:|:------:|:----:|:----:|
| novel durable       | yes | 1.00 | 0.70 | 0.77 |
| restated durable    | yes | 0.29 | 0.85 | 0.82 |
| novel ephemeral     | no  | 1.00 | 0.50 | 0.57 |
| gibberish           | no  | 1.00 | 0.50 | 0.57 |
| restated ephemeral  | no  | 0.38 | 0.50 | 0.48 |

Novelty *works* — the MinHash correctly flags `restated durable` (0.29, it saw
the enrolled `I am vegetarian`) and gibberish as maximally novel (1.00). But the
weight sweep shows folding it into signal helps nothing and eventually hurts:

| w | bad-flips | **bar-fixes** | rank-changes |
|--:|:--:|:--:|:--:|
| 0.10 | 0 | **0** | 0 |
| 0.20 | 0 | **0** | 0 |
| 0.30 | 0 | **0** | 1 |
| 0.40 | 2 | **0** | 1 |
| 0.50 | 2 | **0** | 1 |
| 1.00 | 3 | **0** | 1 |

- **bar-fixes = 0 at every weight.** This is the only column that could justify
  the change — a weight where novelty rescues a bar decision the base signal got
  wrong. It never happens: the durability signal (hall + keywords) already gets
  every bar right.
- **bad-flips rises with weight.** Above the safety ceiling (`w ≳ 0.40`) the
  perturbation overwhelms the base gap and starts *breaking* correct decisions —
  a restated allergy drops below the bar (safety-critical loss), or novel
  chatter floats above it (prompt pollution).
- **rank-changes** appear at `w ≥ 0.30` but, with bar-fixes = 0, they only
  reorder the AAAK greedy fill without any measured correctness benefit.

## Decision

Novelty stays where it already lives — a first-class recognition/spectrogram
dimension and the recurrence-feedback near-duplicate signal (which already flags
restatements for consolidation upstream, the actual mechanism novelty-in-signal
would have duplicated). It is deliberately **not** an input to
`signal::score_memory`. `novelty_signal_probe` is retained as the $0 regression
guard that encodes this reasoning.

Consistent with the project's other measured negatives (read-time LLM
consolidation −9.2pp; RRF fusion / number-words null on real LongMemEval): a
plausible lever, measured, correctly declined before it could dilute the score's
meaning.
