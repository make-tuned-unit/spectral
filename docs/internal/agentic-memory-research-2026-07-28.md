# Agentic-memory research sweep (2026-07-28) — verified findings vs. the Spectral stack

Deep-research pass (multi-angle search → source fetch → 3-vote adversarial claim
verification, 110 agents). Question: what does 2024–2026 agentic-memory research
offer a deterministic, embedding-free stack — at $0 inference — and what do top
LongMemEval scorers do actor-side?

## Externally corroborated: our positioning is replicated

- **Deterministic architectures don't buy aggregate accuracy** — LETHE (a ~700-line
  SQLite+FTS5 store, structurally Spectral's nearest published neighbor) is
  statistically indistinguishable from a naive in-memory store in aggregate
  (63.4% vs 62.9%, McNemar p=0.724, n=385); architectures differentiate
  **per-category** (LETHE 82% vs Mem0 31% on prefix-collision; reversed 0% vs 55%
  cross-lingual).
- **Consolidation pipelines don't beat raw RAG** — a full human-inspired lifecycle
  architecture ties raw RAG on streaming LongMemEval (70.1% vs 71.2%, overlapping
  CIs); its measured value is store compression, not accuracy.
- Both match our own record (MinHash+BM25 Phase-0; retrieval-lever nulls): the
  honest edges are cost, privacy/deletion, audit, per-category shape — not
  aggregate lift. **Independently replicated — cite it.**
- Positioning data: **Zep scores 71.20% official LongMemEval; our shipped number
  is 81.5%.** Mastra OM holds the top official gpt-4o score at 84.23%.

## Validated design choices (no work needed)

- **Bi-temporal 4-timestamp schema (Zep, arXiv:2501.13956)** — event timeline
  (t_valid/t_invalid) + transaction timeline (created/expired) is exactly our
  bi-temporal fact-validity design; Zep's invalidation is LLM-detected with
  deterministic bookkeeping — precisely our "deterministic detection, pluggable
  judgement" adjudication seam. Graphiti has no LLM-free ingestion mode (open
  issue #1193); our zero-LLM ingest remains a differentiator.
- **Mutation-time-only LLM is the cheapest measured seam** — LETHE's mutation-time
  hook lifts 63.4%→91.7% on non-primitive cases at ~$0.17/385 with the recall
  path untouched. Externally validates the shipped adjudication-prompt design
  (486459c).

## New $0 levers, ranked

1. **Dated-observation actor-context format (from the top scorer).** Mastra OM
   (94.87% gpt-5-mini / 84.23% official gpt-4o) uses NO retrieval on the hot
   path — a stable append-only chronological log of dated observations. The
   actor-side discipline is deterministic string assembly: **up to three dates
   per observation (created, referenced, computed relative offset), date-grouped
   evidence, chronological order.** Our temporal-arithmetic and value-selection
   failure modes are exactly what explicit dates + precomputed offsets
   ("(4 months ago)") attack. $0-portable to the bench context builder; needs a
   fresh local weak-actor A/B (format changes context → frozen-context replay
   can't test it).
2. **n-hop BFS retrieval channel** — recursive CTE over the ontology edge table;
   one of Zep's two inference-free channels (the other is BM25, which we have).
   HippoRAG evidence says gains concentrate ONLY on multi-hop shapes (+20.9 R@5
   on 2Wiki, *below* baseline on HotpotQA). Tier-0 oracle candidate; honest
   prior is a null (retrieval is not our bottleneck) — run, record.
3. **PPR over the entity graph with lexical linking** — published systems hide
   inference in entity linking (query NER + encoders); a lexical/FTS-linked
   variant is untested in the literature (LinearRAG's "zero LLM tokens" still
   uses dense encoders). Tier-0 oracle candidate only.
4. **ACT-R base-level activation ln(Σ t_j^−d) as a rerank prior** — computable
   from timestamps + access counts we already store, but NO end-task evidence
   exists anywhere (simulation-level only). Oracle-test or ignore.

## Not portable at $0 (recorded so we don't relitigate)

- **Sleep-time compute (Letta)**: ~5x test-time reduction, but the mechanism IS
  idle-time LLM inference; only the scheduling pattern is free (we already
  precompute deterministically at ingest).
- **Zep/Graphiti ingestion, HippoRAG entity linking, LinearRAG semantic
  bridging, lifecycle similarity substrates**: all require embeddings or LLM
  extraction as published.
- **Self-evolving memory (A-MEM, Mem0, …)**: top-line numbers are vendor-reported;
  the independent record (LETHE, HIMA) shows aggregate parity with naive
  baselines. No replicated aggregate gain to chase.

## Recommended sequence

1. Implement lever 1 (dated-observation formatting) behind a flag; validate at
   $0 with the local weak-actor A/B (temporal + multi-session first).
2. Oracle-check levers 2–4 in one batch ($0, sequential per disk constraint).
3. Cite the LETHE/HIMA replication in MEASURED_RECORD.md.
