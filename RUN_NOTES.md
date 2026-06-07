# Bench Run Notes

## Query Expansion — full-163 bench (expansion ON, clean denominator)

- Result: overall 77.8% (119/153), MS 78.6% (103/131), SSP 72.7% (16/22). Clean denominator 153;
  10 transport failures quarantined, 9 recovered-after-retry, 0 auth failures. Expansion proven-on
  (Haiku +10 terms/question, logged). Cost ~$12.40.
- TRUE lever contribution: +5.3pp vs clean V3 baseline (~72.5%, 108/149). The +11.5pp vs V3's
  REPORTED 66.3% double-counts the retry/clean-denominator fix — do not attribute that to expansion.
- MS (the reproducible weak category, flat across Bench B 71.4% and V3) cleared its 71% pre-committed
  bar at 78.6% — largest real gain of the investigation, within projected +6/+12 (+11 correct).
- SSP +9.4pp but n=22 with ~50% stochastic failures — NOT reliable signal, do not attribute to expansion.
- Mechanism confirmed: expansion bridges FTS vocabulary-coverage gaps on topically-distant incidental
  evidence turns (e.g. "siblings"->"sisters/brothers"). Session levers tested: descriptions (INERT —
  retrieval-neutral, not the regression cause, not a lever), actor prompts (NULL), query expansion (WIN).
- Merged feat/query-expansion, flag on-by-default, reversible.
- Comparison basis: expansion-on vs clean V3 baseline at identical conditions (cascade, max-results 40,
  v3 cleaned descriptions). NOT vs Bench B (contaminated, predates retry/clean-denominator infra).

### Follow-ups (next funded arc, not done)
- 4 entity-specific cases auto-expansion can't reach (proper nouns / numbers / prior job titles the
  LLM can't infer blind) — need corpus-aware term seeding (entity graph or first-pass retrieval). The
  path past +5.3pp.
- 1 hard semantic case (f35224e0, "15" as passing numeric in narrative) — no keyword bridges it;
  seed of an eventual semantic/embedding-retrieval investigation.
- Full n=500 + Bench-B-successor headline number: next round, now standing on a measured, understood fix.

---

## Session summary — query expansion (SHIPPED) + extract→operate (SHELVED, negative result)

### SHIPPED: Query expansion (PR #157, c0e3765, merged, flag on-by-default, reversible)
- Pre-retrieval Haiku term generation → multi-query FTS → rank fusion.
- MEASURED: +5.3pp on MS+SSP vs clean V3 baseline (72.5%→77.8%, n=163, clean denominator).
  MS 66.9→78.6% (the reproducible weak category, previously flat). SSP +9.4pp but n=22, ~50%
  stochastic — NOT reliable. (The +11.5pp vs V3's reported 66.3% double-counts the retry/denominator
  fix — do not attribute that to expansion.)
- ALL-CATEGORY EFFECT: UNMEASURED. Projected ~73-74% from the recall map, NOT confirmed. Requires an
  n=500 all-category bench to claim a corpus number. Do not cite a corpus score until measured.
- Confirmed SAFE corpus-wide: 0 accuracy-impacting regressions; strong categories (SSU 100% recall)
  have nothing to displace. On-by-default validated.

### SHELVED (negative result): extract→code-operate pipeline for OPERATION-class (MS counting)
- Hypothesis: 80% of actor failures are deterministic operations (count/sum/max/date-math) the LLM
  fumbles; route to code. CONFIRMED the diagnosis (24/26 fixable with PERFECT extraction) but every
  buildable stage floored far below the 24 ceiling:
  - Operation was never the bottleneck (code-operate w/ perfect extraction = 24; w/ real = 1-2).
  - Extraction from monolithic 20K context: model drowns. Chunked per-session fixed format/drowning
    but exposed a recall-vs-precision tradeoff with no sweet spot: Qwen under-triggers (3/26),
    Haiku over-extracts (1/26 raw, 5/26 with code filters).
  - Code-side qualification (dedup/tense/speaker filters) ceilings at 5/26.
  - LLM qualification call: 5/26 measured (vs 13 projected), and BROKE 2 cases code got right.
    Combined optimal (union of code-logic + LLM qualify): 7/26.
  - Residual splits across qualification errors (10), extraction recall floor (9), abstention (2) —
    three hard problems, no clean fix.
- CONCLUSION: the MS-counting synthesis gap requires SEMANTIC qualification ("which tank is current,"
  "which events last week") that neither code nor a cheap LLM call does reliably. No cheap
  architectural lever exists for this class. SHELVED — do not re-attempt prompt/pipeline tuning;
  the recall-vs-precision and qualification curves were measured and floor out. Total investigation
  cost: $0.21.
- Note: spectrogram is WRONG data shape for this (measures cognitive dimensions, not operational
  items/dates) — ruled out, do not wire it for operation extraction.

### Strategic state toward Memanto (~89.8%)
- Retrieval is ~95% recall — largely SOLVED. Descriptions confirmed retrieval-NEUTRAL (not a lever,
  not the regression cause; closed). Expansion handles the residual vocabulary-coverage misses.
- Remaining gap is ACTOR-SIDE SEMANTIC SYNTHESIS (counting qualification, recency, preference
  inference) — a hard, model-capability-bound problem, not an architecture/config one. No cheap
  lever found. Next arc is a deliberate synthesis investment (stronger actor reasoning, or per-class
  handling for temporal/recency where data is more structured than counting), NOT more retrieval work.
- The all-category n=500 bench (headline number vs Memanto) is deferred — fund when a publishable
  corpus score is the goal, not reflexively.
