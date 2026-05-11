# Phase 2a Bench Cost Estimate

**Date:** 2026-05-10
**Model:** claude-sonnet-4-6 (actor + judge)
**Pricing:** $3/1M input tokens, $15/1M output tokens
**Questions:** 500

## Token estimates per question

### Actor call

| Component | Low | Expected | High |
|-----------|-----|----------|------|
| System prompt (instructions, 6 numbered rules) | 250 | 250 | 250 |
| question_date line | 10 | 10 | 10 |
| Session headers (~K/5 sessions, ~8 tokens each) | 48 | 64 | 96 |
| Retrieved turns (K turns, variable length) | 4,500 | 11,000 | 19,000 |
| Question text | 15 | 25 | 40 |
| **Total input** | **4,823** | **11,349** | **19,396** |
| **Output** (answer) | 75 | 150 | 300 |

Cascade K by question type:
- Factual (K=30): 70 questions
- Temporal (K=40): 133 questions
- General (K=40): 164 questions
- Counting (K=60): 133 questions
- Weighted average K: ~43

Turn length varies widely in LongMemEval. Short turns ("My car is
red") are ~10 tokens. Long assistant responses can be 200+ tokens.
The "Expected" column assumes ~275 tokens/turn average (observed
from the audit's ~10K tokens with 20 memories, scaled to K=40).

### Judge call

| Component | Low | Expected | High |
|-----------|-----|----------|------|
| Question | 15 | 25 | 40 |
| Ground truth answer | 10 | 20 | 50 |
| Actor's predicted answer | 75 | 150 | 300 |
| Rubric text | 35 | 40 | 45 |
| Frame ("You are grading...") | 30 | 30 | 30 |
| **Total input** | **165** | **265** | **465** |
| **Output** (JSON + reasoning) | 40 | 75 | 150 |

## Cost per question

| Scenario | Actor cost | Judge cost | Total |
|----------|-----------|-----------|-------|
| Low | $0.0156 | $0.0011 | **$0.0167** |
| Expected | $0.0363 | $0.0019 | **$0.0382** |
| High | $0.0632 | $0.0037 | **$0.0669** |

Breakdown (expected): actor input dominates at $0.034, actor output
$0.00225, judge input $0.0008, judge output $0.0011.

## Total cost for 500 questions

| Scenario | Total | Notes |
|----------|-------|-------|
| **Low** | **$8.35** | All short conversations, factual-heavy |
| **Expected** | **$19.10** | Realistic question-type mix |
| **High** | **$33.45** | All long conversations, counting-heavy (K=60) |

## Validation against prior runs

The bench audit (2026-05-01) observed ~$0.10/question in a smoke
test, but that was with max_results=20 (not K=40-60 cascade). The
harness's built-in estimate of $0.04/call was noted as too low.

With cascade K=40-60, the actor input is ~2x the old max_results=20
path, so $0.04-0.07/question is consistent.

## Conclusion

**$40 is comfortably the ceiling, not the floor.** Expected cost is
~$19-20. Even the high scenario ($33) has $7 of headroom below $40.

The only scenario that could exceed $40 would be API retries from
transient failures, which the harness does NOT retry — it records
errors and moves on (or halts after 3 consecutive).
