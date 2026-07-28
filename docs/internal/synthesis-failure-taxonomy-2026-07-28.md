# Synthesis Failure Taxonomy — 2026-07-28

Scope: frozen-artifact analysis to pick the next $0-testable synthesis-side lever, given that
retrieval is closed (answers were retrieved in 15/16 failures) and the counting-shape fixes
(disposal-boundary + do-the-arithmetic) are validated.

Corpora analyzed:

| Corpus | Path | n | Fails | Actor | actor_context | Replay cost |
|---|---|---|---|---|---|---|
| checkpoint (May-6) | `/Users/jessesharratt/projects/spectral/eval-work/checkpoint.json` | 500 | 110 | sonnet (pre-porter) | no | taxonomy only |
| ku-fts | `/Users/jessesharratt/spectral-local-bench/wa-ab/ku-fts-full.json` | 78 | 60 | llama3.2:3b | yes | $0 (ollama) |
| ku-spread | `/Users/jessesharratt/spectral-local-bench/wa-ab/ku-spread-full.json` | 78 | 58 | llama3.2:3b | yes | $0 (ollama) |
| tier1-h2h-porter | `/Users/jessesharratt/spectral-local-bench/tier1-h2h-porter.json` | 60 | 14 | sonnet | yes | paid replay |
| cnt-arm-c | `/Users/jessesharratt/spectral-local-bench/wa-ab/cnt-arm-c.json` | 12 | 1 | — | yes | $0 |

The ku both-wrong set (failed in BOTH ku-fts and ku-spread) is exactly 50 questions and is the
primary $0 replay target.

---

## 1. Per-corpus failure-mode tables

### ku both-wrong (n=50) — refined classification (grep-verified against actor_context)

| Mode | n | Question IDs |
|---|---|---|
| wrong-value-selection (evidence present, wrong/stale value picked) | 13 | 22d2cb42, 2698e78f, 42ec0761, 4b24c848, 50635ada, 7401057b, 830ce83f, b01defab, c6853660, dfde3500, e493bb7c, e66b632c, f9e8c073 |
| scaffold-collapse (no final answer; actor echoes scan scaffold / "No match" / session-date lists) | 10 | 07741c45, 0f05491a, 184da446, 69fee5aa, 7a87bd0c, 89941a94, 8fb83627, b6019101, ba61f0b9, db467c8c |
| quote-evidence-then-abstain (actor cites the evidence, then says "I don't know") | 7 | 10e09553, 4d6b87c8, 5a4f22c0, 945e3d21, cf22b7bf, d7c942c3, f685340e |
| corrective-abstain-expected (gold = "not enough info; you mentioned X not Y"; actor fails the format) | 6 | 031748ae_abs, 0ddfec37_abs, 2133c1b5_abs, 2698e78f_abs, 6aeb4375_abs, f685340e_abs |
| bare-abstain, gold terms fully in context (ev=1.0) | 3 | 41698283, 9bbe84a2, c4ea545c |
| bare-abstain, partial/unverified evidence | 5 | 0977f2af, 6a1eabeb, 59524333, affe2881, c7dc5443 |
| counting-wrong-number | 3 | 01493427, 031748ae, a2f3aa27 |
| temporal-arithmetic (scaffold-driven date-listing, never computes) | 3 | 08e075c7, 2133c1b5, 852ce960 |

Cross-cutting observation: in 14/50 both-wrong questions at least one arm's *predicted text
contains the ground truth verbatim* (e.g. 7401057b: predicted "2 free nights'" vs gold "Two",
judged wrong for extra detail). These overlap heavily with scaffold-collapse and
quote-then-abstain — the value is IN the output; the answer never gets committed cleanly.

### tier1-h2h-porter (sonnet, 14 fails)

| Mode | n | Question IDs |
|---|---|---|
| abstained-with-evidence (broad; ev >= 0.86) | 4 | bc8a6e93_abs, gpt4_7fce9456, 6071bd76, 031748ae_abs |
| — of which strict fa19884d shape (quotes evidence then abstains) | 1 | 6071bd76 |
| — of which corrective-abstain-expected (gold wants "not X, you said Y"; actor said bare "I don't know" or missed the mismatch) | 2 | bc8a6e93_abs, 031748ae_abs |
| aggregation-counting (miscount by 1–2, or cross-question judge confusion) | 3 | 4f54b7c9, 2311e44b, f35224e0 |
| wrong-value-with-evidence | 3 | ad7109d1 (1 Gbps vs 500 Mbps), 06878be2, 195a1a1b (preference: clarify-back / fabrication) |
| temporal selection (recency-of-mention vs recency-of-event; wrong-entity pick) | 3 | gpt4_e414231f, gpt4_2f56ae70, eac54add |
| infrastructure (transport error, not a synthesis failure) | 1 | 09ba9854 |

Notable: gpt4_2f56ae70 ("streaming service started most recently") — sonnet picked the most
recently *mentioned* service (HBO Max) instead of the most recently *started* (Disney+).
Distinct, promptable temporal-selection bug.

### checkpoint May-6 (pre-porter, no actor_context — taxonomy only, 110 fails)

| Mode (heuristic, no context check possible) | n |
|---|---|
| abstention ("I don't know" family) | 39 |
| aggregation/counting | 30 |
| wrong-value-selection | 20 |
| temporal-arithmetic | 15 |
| hedged-nonanswer | 6 |

Category mix: multi-session 53, temporal-reasoning 30, single-session-preference 15,
knowledge-update 8, single-session-user 3, single-session-assistant 1. Confirms the same three
macro-shapes (abstention, counting, value-selection) dominated even pre-porter; counting is now
largely handled (cnt-arm-c = 11/12).

### cnt-arm-c (1 remaining fail)

gpt4_15e38248 (furniture multi-verb): actor found 3 of 4 items (bought coffee table, assembled
IKEA bookshelf, fixed kitchen table) and showed no awareness of a possible 4th. Off-by-one
enumeration miss under a multi-verb criterion ("buy, assemble, sell, or fix") — recall gap in
the scan, not a rule the current prompt family obviously misses. Park it.

---

## 2. Ranked lever list (frequency x prompt-addressability)

Ranked on the $0-replayable ku both-wrong 50 unless noted.

| Rank | Lever | Targets (both-wrong) | Addressability | Also hits |
|---|---|---|---|---|
| 1 | **Final-answer-commit rule** (kills scaffold-collapse + temporal date-listing collapse + scaffold echo) | 13: the 10 scaffold-collapse + 08e075c7, 2133c1b5, 852ce960; partially 50635ada, 031748ae | High — pure format discipline, exactly the kind of numbered rule that worked for counting | up to 14 GT-verbatim-in-output cases |
| 2 | **Commit-to-quoted-evidence rule** (fa19884d mode) | 10: the 7 quote-then-abstain + 3 bare-abstain ev=1.0 (41698283, 9bbe84a2, c4ea545c) | High — one behavioral rule | tier1 6071bd76; checkpoint's 39-abstention block suggests this mode scales |
| 3 | **Corrective-abstention format rule** ("not X — you mentioned Y") | 6 `_abs` questions | High — gold explicitly rewards the near-miss citation | tier1 bc8a6e93_abs, 031748ae_abs |
| 4 | **Event-recency vs mention-recency temporal rule** | (tier1 only) gpt4_2f56ae70, gpt4_e414231f, eac54add | Medium — clean rule, but sonnet-only evidence, needs paid replay | checkpoint temporal block (15) |
| 5 | Stale-value re-check (wrong-value residue) | subset of the 13 wrong-value (2698e78f, 42ec0761, 4b24c848…) | Low-Medium — factual_current_state.md rule 1 already says this; the 3b actor fails it anyway → capability-limited, not rule-starved | ad7109d1 in tier1 |

Levers 1–3 are non-overlapping and together cover 29/50 (58%) of the $0-replayable both-wrong
set. Lever 5 is where prompt rules likely saturate on a 3b actor.

---

## 3. Draft prompt rules

Style follows `crates/spectral-bench-accuracy/src/prompts/counting_current_state.md` /
`factual_current_state.md` (numbered instructions, bold key clause).

### Rule A — final-answer commit (lever 1; append to every prompt in the family)

> N. **Always end with a final answer.** Your last line MUST have the form
> `Answer: <the answer>` — a single sentence containing the value, name, count, or duration
> requested. Scan notes, session quotes, or "No match" lines are working steps, never the
> response itself. If your scan produced only partial matches, still commit to the best
> supported value on the final line. Never end your response with a session header, a quote
> block, or a list of dates.

For `temporal.md` specifically, soften instruction 1's scaffold for weak actors:

> 1. Identify the date of the event the question asks about, compute the difference from
> today's date, and state the result. Show the two dates used, then give the duration on a
> final `Answer:` line.

Targets: 07741c45, 0f05491a, 184da446, 69fee5aa, 7a87bd0c, 89941a94, 8fb83627, b6019101,
ba61f0b9, db467c8c, 08e075c7, 2133c1b5, 852ce960 (+50635ada, 031748ae).

### Rule B — commit to quoted evidence (lever 2)

> N. **If you found it, you know it.** If any session contains content matching the entity the
> question asks about, you MUST commit to that content as your answer. Never write "you
> mentioned <fact>" and then conclude "I don't know" — a quoted or paraphrased fact that
> matches the question IS the answer. Reserve "I don't know" strictly for the case where no
> session mentions the entity at all.

Targets: 10e09553, 4d6b87c8, 5a4f22c0, 945e3d21, cf22b7bf, d7c942c3, f685340e, 41698283,
9bbe84a2, c4ea545c (ku, $0); 6071bd76 (tier1, paid).

### Rule C — corrective abstention (lever 3)

> N. **Abstain with the correction, not with silence.** If the question's premise names an
> entity that does not appear in any session but a closely related entity does (uncle vs
> niece, football vs baseball, Shinjuku vs Harajuku, Manager vs Senior Engineer), answer in
> the form: "There is no information about <asked entity>. You mentioned <related entity>
> instead." A bare "I don't know" is wrong when a near-miss exists.

Targets: 031748ae_abs, 0ddfec37_abs, 2133c1b5_abs, 2698e78f_abs, 6aeb4375_abs, f685340e_abs
(ku, $0); bc8a6e93_abs, 031748ae_abs (tier1, paid).

### Rule D — event recency, not mention recency (lever 4; temporal/factual_current_state)

> N. **"Most recent X" means the X whose own event date is latest — not the X mentioned in the
> most recent session.** First list each candidate with the date the event happened (started,
> bought, switched), then pick the latest event date. A recent session discussing an old event
> does not make that event recent.

Targets: gpt4_2f56ae70, gpt4_e414231f, eac54add (all tier1 → paid replay only).

---

## 4. How to validate at $0

1. **Free tier (do first): ku both-wrong replay on local ollama (llama3.2:3b).**
   Frozen `actor_context` is present in `ku-fts-full.json` / `ku-spread-full.json`; replay the
   50 both-wrong questions against frozen contexts with Rules A+B+C appended (test as one arm
   plus per-rule ablation arms if cheap — run sequentially per the disk constraint).
   - Success gate: net flips on the 29 targeted qids without regressions on a control replay of
     currently-correct ku questions (18 fts / 20 spread passers).
   - Expectation management: the 3b actor is capability-limited; treat any +8–12 flips of 50
     as a strong signal. Wrong-value residue (13) is NOT expected to move.
2. **$0 sanity on cnt-arm-c:** re-run the 12-question counting set with Rule A appended to
   confirm no counting regression (the validated disposal-boundary/arithmetic rules must keep
   passing 11/12).
3. **Paid tier (only after 1 passes): sonnet replay of tier1-h2h-porter failures.**
   Rules B+C target 3 of 14 (6071bd76, bc8a6e93_abs, 031748ae_abs); Rule D targets 3 more
   (gpt4_2f56ae70, gpt4_e414231f, eac54add). Rule D has no free proxy — it is sonnet-only
   evidence and needs paid replay to validate at all. Batch it with the queued ~$15 combined
   Tier-1 replay rather than paying separately.
4. **Judge-strictness caveat to carry into any replay read-out:** at least 7401057b (and the
   22d2cb42 "Rhythm Central" case in checkpoint) were semantically correct but judged wrong
   for added detail. Rule A's single-line `Answer:` format also mitigates this, but do not
   count such flips as pure actor improvement — note them separately.

Non-goals confirmed by this pass: retrieval levers stay closed (evidence present in context
for the dominant modes); counting family stays closed except the parked gpt4_15e38248
off-by-one.

---

## VALIDATION RESULT (2026-07-28, $0 local replay)

Paired replay of the full 78q ku corpus (frozen retrieval contexts, llama3.2:3b
actor + qwen2.5:7b judge, temperature 0), control binary vs rules binary:

| arm | accuracy |
|---|---|
| control replay | 18/78 (23.1%) |
| rules A–D replay | **22/78 (28.2%)** |

- **Replay fidelity: perfect** — control replay reproduced the frozen run's
  grades 78/78 (0 flips). The $0 replay instrument is validated end-to-end.
- Net **+4** (5 recovered / 1 regressed), McNemar p=0.219 (directional, n small).
- **4/5 recoveries are the exact taxonomy-predicted targets**: `08e075c7`
  (rule A scaffold-collapse), `d7c942c3`+`5a4f22c0` (rule B quote-then-abstain),
  `f685340e_abs` (rule C corrective abstention); `dad224aa` unpredicted bonus.
- The 1 regression (`6071bd76`) is an honest rule-A cost: forced commitment made
  the weak actor commit to the wrong value on a value-selection question — the
  mode already classified capability-limited. 5:1 trade accepted.

Same evidential shape as the counting sweep: mechanism-specific predicted hits,
minimal collateral. Next validation step for the sonnet transfer: batch rules
B/C/D into the queued Tier-1 paid replay (~$3) before any n=500 spend.
Frozen: `~/spectral-local-bench/wa-ab/rules-{control,candidate}.json`.

---

## SONNET TRANSFER TEST (2026-07-28, ~$5 replay) — rules restructured

Paired replay of tier1-h2h-porter (56q clean) with sonnet actor+judge,
control vs rules-A–D binary: **control 46 vs rules 43 (net −3)** — but the
flip inspection reallocates most of that:

- **2 counting flips CANNOT be rule-caused** (counting templates identical in
  both arms) — one is the pre-#219 judge parse-failure artifact scoring a
  delta-1 tolerance case wrong mid-truncation. CONFOUND: these binaries stack
  on main, not on the hardened judge. Noise/artifact, not rule regressions.
- **Attributable: +1 rule C** (`bc8a6e93_abs` corrective abstention — works on
  both actor strengths), **−1 temporal softening** (`fe651585`: the scaffold
  discipline weak actors collapse under is something sonnet exploits well),
  **−1 current-state stack** (`07741c45`, D+B+C+A un-attributable at n=1).

**Disposition (shipped in this branch):**
- **Rules B + C stay in shared templates** — validated on the weak actor
  (3 predicted recoveries), C also on sonnet, no attributable counter-evidence.
- **Rule A + temporal softening REMOVED from shared templates** — weak-actor-
  specific (scaffold collapse is a small-model failure mode; forcing early
  commitment and removing scaffold hurts or does nothing for strong actors).
  Candidate for a future weak-actor prompt profile.
- **Rule D REMOVED pending a clean test** — its only targets are sonnet-side
  and the one current-state datapoint is confounded; re-test under the
  hardened judge in the batched Tier-1 replay.

Lesson recorded: prompt levers are ACTOR-STRENGTH-SPECIFIC; a weak-actor win
is not a transfer claim. Validate per actor profile before shipping shared
templates. Frozen: `~/spectral-local-bench/wa-ab/tier1-rules-{control,candidate}.json`.
