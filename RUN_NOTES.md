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
