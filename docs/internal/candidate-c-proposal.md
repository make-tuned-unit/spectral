# Candidate C — Per-Session Context Isolation

**Date:** 2026-05-14
**Status:** Proposal. Awaiting review before implementation.
**Source:** `docs/internal/actor-level-interventions-investigation.md`
Section 3, Candidate C.

---

## Section 1 — Mechanism

### What "per-session context isolation" means

Instead of sending all retrieved memories to the actor in a single
LLM call, split the retrieved set by session and make one
extraction call per session. Each extraction call sees only that
session's turns plus the question — no cross-session content. A
final aggregation call merges the per-session extractions into the
answer.

### Current actor path (single-call)

The flow in `eval.rs:eval_single()` today:

```
ingest → retrieve (cascade, K=60) → format_hits_grouped()
  → actor.answer(question, date, &memories, shape)  [1 call]
  → judge.grade()  [1 call]
```

The actor receives all ~60 memories grouped by session in a single
prompt (~20K input tokens). One call. The actor reads all sessions,
synthesizes, answers.

### Proposed actor path (per-session isolation)

```
ingest → retrieve (cascade, K=60) → format_hits_grouped()
  → split memories by session header
  → for each session: extract_call(question, session_memories)  [N calls]
  → aggregate_call(question, all_extractions)  [1 call]
  → judge.grade()  [1 call]
```

**Step 1: Split.** The retrieved memories are already grouped by
session via `format_hits_grouped()` (`retrieval.rs:242`). Each
group starts with `--- Session <id> (<date>) ---`. The split
parses these groups — no retrieval changes needed.

**Step 2: Extract (N calls).** For each session group, one LLM
call:

```
Given this conversation session, answer the following about the
user: {question}

List every relevant item mentioned, with a verbatim quote. If
this session contains nothing relevant, say "Nothing relevant."

Session:
{session_turns}
```

Each call processes a small context (~500-2K input tokens — a few
turns plus the question). The model's full attention budget is
spent on one session. There is no cross-session topic competition.

**Step 3: Aggregate (1 call).** One call receives all per-session
extraction results:

```
You are answering a question based on evidence extracted from
multiple conversation sessions. Today's date is {question_date}.

Question: {question}

Evidence from each session:
{per_session_extractions}

Instructions:
1. List every unique item from the extractions above.
2. Deduplicate items that refer to the same thing across sessions.
3. State the final count.
```

The aggregation call processes structured extractions (~2K input
tokens), not raw conversation text. Its job is deduplication and
counting, not extraction.

### Where this sits in the bench harness

**Files changed:**

| File | Change |
|---|---|
| `eval.rs` | `eval_single()`: add branch for Candidate C between retrieve and judge |
| `actor.rs` | Add `IsolatedActor` struct implementing the `Actor` trait, wrapping `AnthropicActor` for extraction/aggregation calls |
| `main.rs` | Add `--actor-mode isolated` CLI flag |
| `retrieval.rs` | Add `split_by_session(memories: &[String]) -> Vec<(String, Vec<String>)>` utility |

**No changes to:** retrieval, ranking, cascade, ingest, judge,
Spectral core. This is purely bench-actor-side.

### Scope gate: counting questions only

Per-session isolation is designed for counting questions where the
actor must enumerate items across sessions. It should NOT apply to:
- Preference questions (need full context to infer preference)
- Temporal questions (already route to TopkFts, not cascade)
- General recall (single-session, no cross-session competition)

The implementation uses the existing `QuestionType` classifier:
isolation fires for `Counting` and `CountingCurrentState` shapes
only. All other shapes use the existing single-call path.

---

## Section 2 — The 4 GENUINE_MISS cases

PR #117 confirmed all 4 as GENUINE_MISS (all answer sessions
retrieved, actor fails to count). Here is how per-session isolation
interacts with each.

### Case #8: Tanks (46a3abf7) — isolation SHOULD help

**GT:** 3 tanks. **Actor:** 2 (missed 5-gallon betta tank).

**Failure mechanism:** Cross-session topic filtering (sub-problem
2). The missed tank is in `answer_c65042d7_2`, whose primary topic
is high nitrite levels in the community tank. The betta tank is
introduced as background: "I have a 5-gallon tank with a solitary
betta fish named Finley." In the full-context call, the model
classifies this session as "about water chemistry," not "about
tanks owned," and skips the background mention.

**With isolation:** The extraction call for `answer_c65042d7_2`
sees only that session plus the question "How many tanks do you
currently own?" The session mentions the 20-gallon community tank
(explicitly), the 5-gallon betta tank (first turn), and nitrite
levels. With no competing sessions, the model has no reason to
classify this as "not about tanks." The 5-gallon tank should be
extracted.

**Confidence:** High. This is the textbook case for context
isolation — subordinate reference in a session with a different
primary topic.

### Case #9: Weddings (gpt4_2f8be40d) — isolation SHOULD help

**GT:** 3 weddings. **Actor:** 2 (missed Emily+Sarah and Jen+Tom).

**Failure mechanism:** Cross-session topic filtering (sub-problem
2). Sessions `answer_e7b0637e_2` and `_3` are primarily about the
user's own wedding planning. Attended-wedding references are
subordinate: "My friend Emily finally got to tie the knot with her
partner Sarah" and "the bride, Jen, looked stunning." In the
full-context call, the model follows the wedding-planning topic
and skips these subordinate mentions.

**With isolation:** The extraction call for `answer_e7b0637e_2`
sees one session about wedding planning with a reference to
Emily+Sarah's wedding. The question "How many weddings have you
attended?" focuses the extraction. Without 19 other sessions
competing for attention, the model should register "Emily finally
got to tie the knot" as a wedding-attendance reference.

**Confidence:** Moderate-high. The decontextualization risk is
real here — see Section 5. The session discusses the user's OWN
wedding planning, so the extraction call might extract "my
wedding" rather than "Emily's wedding I attended." The question
wording provides some framing ("weddings you attended"), but this
is the case most likely to show the decontextualization failure
mode.

### Case #3: Bike expenses (gpt4_d84a3211) — isolation does NOT help

**GT:** $185 ($40 lights + $25 chain + $120 helmet). **Actor:** $40.

**Failure mechanism:** Sub-problem 1 (within-turn partial
extraction). The $25 chain is in the SAME turn as the $40 lights:
"it cost me $25. While I was there, I also got a new set of bike
lights installed, which were $40." The actor extracted the $40 but
not the $25 from the same sentence pair.

**With isolation:** The extraction call for `answer_2880eb6c_2`
sees the same sentence pair. Context isolation removes
cross-session competition, but this failure is at sentence
granularity — the model reads a sentence pair and extracts one
fact, skipping the adjacent one. One-session context doesn't fix
within-turn attention drop.

The $120 helmet (in `answer_2880eb6c_1`) is a parenthetical:
"where I bought my Bell Zephyr helmet for $120." Isolation MIGHT
help this component — it's a subordinate reference in a session
about the bike shop. But the actor also fabricated "no specific
costs are given" while mentioning the helmet, suggesting a deeper
extraction failure than topic filtering.

**Confidence:** Low. The primary miss ($25 chain) is sub-problem
1, which isolation doesn't address. The secondary miss ($120
helmet) might benefit, but the fabrication behavior suggests a
different failure mode.

### Case #7: Movie festivals (gpt4_a56e767c) — isolation MAY help

**GT:** 4 festivals. **Actor:** 3 (Austin, AFI, Portland).

**Failure mechanism:** Sub-problem 3 (unknown). All 3 answer
sessions were retrieved. The 4th festival's identity is unclear —
it may be in a non-answer session or may be a GT interpretation
issue.

**With isolation:** If the 4th festival is mentioned as a
subordinate reference in a session the actor skimmed, isolation
would surface it. If the 4th festival is a GT accuracy question,
isolation doesn't help.

**Confidence:** Unknown. Cannot predict without knowing which
festival is the 4th.

### Summary

| Case | Sub-problem | Isolation helps? | Confidence |
|---|---|---|---|
| #8 Tanks | 2 (topic filtering) | Yes | High |
| #9 Weddings | 2 (topic filtering) | Yes (with risk) | Moderate-high |
| #3 Bike expenses | 1 (within-turn) | No (primary miss) | Low |
| #7 Festivals | 3 (unknown) | Maybe | Unknown |

**Best case: +2 questions flipped** (#8, #9). Realistic: +1 to +2.
Maximum multi-session lift: +10pp (2/20). If #7 also flips: +15pp.

---

## Section 3 — Cost and latency

### Per-question cost

The investigation doc estimated $0.04 per extraction call (full
actor call cost). This overestimates. Extraction calls process
~1-2K input tokens (one session) vs ~20K for the full actor. Using
Sonnet pricing ($3/M input, $15/M output):

| Call type | Input tokens | Output tokens | Cost |
|---|---|---|---|
| Full actor (current) | ~20,000 | ~1,000 | ~$0.075 |
| Per-session extraction | ~1,500 | ~200 | ~$0.008 |
| Aggregation | ~3,000 | ~500 | ~$0.017 |

**Per counting question (Candidate C):**
- 15 sessions typical × $0.008 = $0.12 (extraction)
- 1 × $0.017 = $0.017 (aggregation)
- Total actor: ~$0.14
- Plus judge: ~$0.075
- **Per question: ~$0.21**

**Current per counting question:**
- 1 actor: ~$0.075
- 1 judge: ~$0.075
- **Per question: ~$0.15**

**Delta: +$0.06/question (+40%).** Not the 5-10x the investigation
estimated — the extraction calls are much smaller than full actor
calls.

### Per-bench-run cost

Candidate C applies to counting questions only. In the 20-question
multi-session category, roughly 10-12 are counting-type (the rest
are preference, recall, etc.).

| Run scope | Current | With Candidate C | Delta |
|---|---|---|---|
| 12 counting questions | $1.80 | $2.52 | +$0.72 |
| Full 20-question multi-session | $3.00 | $3.72 | +$0.72 |
| Full 120-question bench | $18.00 | $18.72 | +$0.72 |

**The cost delta is modest.** +$0.72 per bench run, concentrated
in multi-session counting questions. This is not a blocker.

### Latency

**Serial execution:** 15 extraction calls × ~2s each = ~30s per
question vs ~10s current. 3x latency increase.

**Parallel execution:** Extraction calls are independent per
session. With parallel requests (tokio/rayon or concurrent
reqwest calls), all extractions finish in ~2-3s (bounded by the
slowest single call). Total: ~5s per question vs ~10s. **Faster
than current** if parallelized, because each call is smaller.

**Recommendation:** Implement with parallel extraction calls.
The bench harness already uses `reqwest::blocking::Client`; for
the proposal, sequential is acceptable (bench is not
latency-sensitive). Production (Permagent) would need async
parallel.

---

## Section 4 — Attribution plan

### Experimental design

Same discipline as item #8 validation — matched control/treatment
runs:

- **Control:** Current main with descriptions, cascade K=40.
  Multi-session baseline: 11/20 (55%). This is the existing
  `20260514-item8-with-descriptions/multi-session/report.json`.
- **Treatment:** Same config, Candidate C enabled for counting
  questions via `--actor-mode isolated`.
- **Delta:** Treatment - Control = isolated Candidate C lift.

### Run scope

Only multi-session needs re-running — Candidate C only fires for
counting questions, which are concentrated in multi-session. The
other 5 categories are unaffected (no counting questions use the
cascade path).

- **20 questions, 1 run.** Cost: ~$3.72 (actor + judge).
- Control already exists from the item #8 validation run.

### Pre-validation shortcut

Before running the full 20-question bench, pre-validate on cases
#8 and #9 directly:

1. Take the 3 answer sessions from each case.
2. For each session, make one extraction call: "Given this session,
   list every [tanks owned / weddings attended]."
3. Check: does isolation surface the missed items?

Cost: 6 calls, ~$0.05. Time: 2 minutes. If neither case flips,
skip the full bench run — the mechanism doesn't work on the target
cases.

---

## Section 5 — Risks and honest assessment

### Risk 1: Decontextualization

Per-session calls lack cross-session framing. Specific failure
mode: a question like "How many weddings did you attend?" sent to
a session about the user's own wedding planning might extract the
user's wedding plans as "a wedding," inflating the count. The
question provides some framing ("attended"), but within a
wedding-planning session, the distinction between "my wedding" and
"weddings I went to" requires inference that the full context
provides naturally.

**Mitigation:** The extraction prompt explicitly includes the
original question, which frames what to extract. The aggregation
call receives all extractions and deduplicates — if a session
incorrectly extracts "my own wedding," the aggregation call can
filter it using cross-session context from the other extractions.

**Severity:** Moderate for case #9 (weddings). Low for case #8
(tanks — "tanks I own" has no ambiguity with planning topics).
This risk could cause regression on currently-correct counting
questions where the full-context actor correctly distinguishes
items.

### Risk 2: Aggregation failure mode

The combine step introduces its own failure surface. The
aggregation call must deduplicate items across sessions — e.g.,
if two sessions both mention "Emily's wedding," the aggregation
must count it once. If the extraction calls use different
phrasings ("Emily and Sarah's wedding" vs "my friend Emily's
ceremony"), the aggregation call must recognize these as the
same event.

**Severity:** Low-moderate. The aggregation call processes
structured extractions (short text with quotes), not raw
conversation. Deduplication on short, extracted items is a simpler
task than extraction from long sessions. But it is a new failure
surface that doesn't exist in the single-call path.

### Risk 3: Sub-problem 1 remains unaddressed

Context isolation does not help within-turn extraction failures
(case #3). If the model reads "it cost me $25...also got bike
lights...$40" and extracts only $40, isolating the session doesn't
change this — both items are in the same turn. This means Candidate
C addresses at most 2-3 of 4 GENUINE_MISS cases.

### Risk 4: Scope creep into production

If Candidate C validates on bench, there's pressure to bring it to
Permagent's production actor. The production cost is more
significant: per-session extraction on every user query, not just
bench runs. The bench harness and Permagent's actor share no code.
Adopting Candidate C in production would require reimplementation
in Permagent's async pipeline, latency budget allocation, and
potentially promoting the `QuestionType` classifier to Spectral
core.

**Mitigation:** This proposal is bench-only. Production adoption
is a separate decision with its own cost/benefit analysis, gated
on bench validation results.

### Honest assessment and recommendation

**Is this worth implementing?**

Yes, with the pre-validation shortcut. The cost is modest (+$0.72
per bench run, +40% per counting question), the mechanism is
theoretically sound (eliminates cross-session attention competition
— the confirmed failure mechanism for 2 of 4 GENUINE_MISS cases),
and the pre-validation costs $0.05 and takes 2 minutes.

**Is the expected lift worth the cost delta?**

At +$0.72 per bench run for a potential +5-10pp on multi-session
(+1-2pp overall), yes. The cost is trivial relative to the insight
gained. Even a null result is valuable — it confirms the
GENUINE_MISS floor is structural and stops further actor-level
investigation.

**Recommended sequence:**

1. **Pre-validate** on cases #8 and #9 (6 extraction calls,
   $0.05). If neither missed item surfaces in isolation, stop.
2. **If pre-validation succeeds:** Implement `--actor-mode
   isolated` in the bench harness. Run 20-question multi-session
   treatment.
3. **If bench shows +1-2 questions:** Validate the lift is real
   (not noise at N=20). Run a second multi-session treatment to
   check reproducibility.
4. **Production decision:** Separate from bench validation. Only
   consider if bench lift is confirmed and the counting-question
   scope gate holds.

**What could make this not work:**

- Decontextualization regression (Risk 1) wipes out gains
- The missed items in cases #8/#9 are actually not the cross-session
  topic filtering mechanism — the model may fail to extract them
  even in isolation for a different reason
- The aggregation step (Risk 2) miscounts, converting extraction
  gains into a different failure
- The lift is real but the N=20 sample is too small to distinguish
  from noise (+1 question = +5pp, well within LLM variance)

The last point is the most honest concern. At N=20, a +1 question
flip is within noise. The pre-validation shortcut addresses this:
if the mechanism works on the target cases in isolation, we have
mechanistic evidence beyond the bench score. If the pre-validation
fails, the bench number is moot.
