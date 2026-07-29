# Counting-shape sweep — pre-registered 3-arm validation (2026-07-28)

Executes the pre-registered ~$3–4 sweep from `TIER1_PORTER_WIDEN.md` (rec 3):
does the two-stage counting pipeline — shape-gated expansion + prompt-v2 rules —
convert to accuracy on the frozen tier-1 counting set? Actual spend: **$2.24**.

## Setup

12 frozen counting questions (`tier1-counting-ids.txt` ∪ `tier1-cnt-test-ids.txt`),
topk_fts, sonnet-4-6 actor + judge, **hardened judge** (PR #219: abstention rule,
parse-failure exclusion, rubric fingerprint). Arms A/B built from
`fix/bench-judge-calibration` (v1 prompt), arm C from `feat/bench-counting-levers`
(v2 prompt + shape-gated expansion). All arms 12/12 clean; one 529-overload
recovered on retry; **0 judge parse failures** (the new reliability class, live).

## Result — monotonic, zero regressions

| arm | config | accuracy | multi-session |
|---|---|---|---|
| A | v1 prompt, no expansion | 9/12 (75.0%) | 6/9 |
| B | v1 prompt, + expansion | 10/12 (83.3%) | 7/9 |
| C | v2 prompt, + gated expansion | **11/12 (91.7%)** | 8/9 |

Pairwise (clean 12/12 in all comparisons, **0 regressions anywhere**):

- **A→B (+expansion): recovered `gpt4_f2262a51` "How many different doctors"** —
  the exact lexical-gap case TIER1 predicted expansion fixes.
- **B→C (+prompt v2): recovered `gpt4_194be4b3` "How many instruments do I
  currently own"** — the exact disposal-boundary case rule 9 was written for
  (the drums "being sold" exclusion error).
- A→C: +2 net, discordant 2–0 in favor, McNemar p=0.50.
- Remaining failure in all arms: `gpt4_15e38248` (furniture multi-verb count) —
  known hard case, unrecovered by either lever.

## Verdict

**The pre-registered predictions reproduced one-for-one**: expansion recovers the
expansion-predicted case, prompt-v2 recovers the prompt-v2-predicted case, and
nothing regresses. n=12 cannot reach significance (this is a targeted replay of
known failure modes, not a powered sample) — the claim is NOT "counting accuracy
+16.7pp"; the claim is that both levers behave exactly as measured in TIER1 under
the hardened judge, at the predicted questions, with zero collateral damage.

**Ship both levers** (PR #220). Their population-level effect gets measured for
free inside the next full-bench run (~$26 porter-only n=500 or the ~$15 combined
replay) rather than a dedicated powered counting run.

Cost note: expansion added ~$0.012/question (haiku) and +2.7k tokens/query mean;
gating confines that to counting shapes only (~6% of the dataset).

Harness follow-up filed: the pre-flight cost estimate ignores the `--question-id`
filter (reported $40 for a 12-question run); guard is conservative, estimate wrong.

Frozen: `~/spectral-local-bench/wa-ab/{cnt-arm-a,b,c}.json`, `run_counting_sweep.sh`,
`counting-sweep-ids.txt`.
