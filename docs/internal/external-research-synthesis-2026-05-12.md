# External Research Synthesis: Bench Category Improvements

**Date**: 2026-05-12
**Branch**: `research/external-practices-synthesis-2026-05-12`
**Status**: Research complete. No implementation — decisions in follow-up PRs.

---

## Section 1 — Methodology

### Sources surveyed

1. **Anthropic documentation** (primary):
   - [Prompting best practices](https://platform.claude.com/docs/en/build-with-claude/prompt-engineering/claude-prompting-best-practices) — includes long context prompting section
   - [Prompt engineering for Claude's long context window](https://www.anthropic.com/news/prompting-long-context) — Anthropic blog, Sep 2023 (techniques validated through Claude 2 → Sonnet 4.5 era)
   - [Contextual Retrieval](https://www.anthropic.com/news/contextual-retrieval) — Anthropic blog on BM25+embedding hybrid with document enrichment

2. **Academic papers**:
   - Liu et al., ["Lost in the Middle: How Language Models Use Long Contexts"](https://arxiv.org/abs/2307.03172) (TACL 2024, originally Jul 2023)
   - Hsieh et al., ["Found in the Middle: Calibrating Positional Attention Bias"](https://arxiv.org/abs/2406.16008) (Jun 2024)
   - NeurIPS 2024 poster: "Found in the Middle: How Language Models Use Long Contexts Better via Plug-and-Play Positional Encoding" (Ms-PoE)
   - ICLR 2025: ["Do LLMs Recognize Your Preferences? Evaluating Personalized Preference Following in LLMs"](https://openreview.net/forum?id=QWunLKbBGF) (PersonalLLM benchmark)
   - ICLR 2025: ["Eliciting Human Preferences with Language Models"](https://arxiv.org/abs/2310.11589) (GATE framework)
   - Apple ML Research 2025: ["Aligning LLMs by Predicting Preferences from User Writing Samples"](https://arxiv.org/html/2505.23815v1) (PROSE method)
   - ["Extracting Implicit User Preferences in Conversational Recommender Systems Using Large Language Models"](https://www.mdpi.com/2227-7390/13/2/221) (MDPI Mathematics, 2025)
   - ACM TOIS 2025: ["LLMCDSR: Enhancing Cross-Domain Sequential Recommendation with LLMs"](https://dl.acm.org/doi/10.1145/3715099)
   - NAACL 2025: ["Reasoning Aware Self-Consistency"](https://aclanthology.org/2025.naacl-long.184/) (RASC)
   - ACL 2025 Findings: ["Confidence Improves Self-Consistency in LLMs"](https://aclanthology.org/2025.findings-acl.1030.pdf) (CISC)
   - Arxiv 2025: ["Focused Chain-of-Thought: Efficient LLM Reasoning via Structured Input Information"](https://arxiv.org/html/2511.22176v1)
   - ACM Computing Surveys 2025: ["Multi-Step Reasoning with Large Language Models, a Survey"](https://dl.acm.org/doi/10.1145/3774896)
   - PARSE (Amazon Science 2025): ["LLM Driven Schema Optimization for Reliable Entity Extraction"](https://arxiv.org/html/2510.08623v1)

3. **Industry/engineering sources**:
   - [Mem0 architecture](https://github.com/mem0ai/mem0) and [State of AI Agent Memory 2026](https://mem0.ai/blog/state-of-ai-agent-memory-2026) — multi-signal retrieval (semantic + BM25 + entity matching)
   - [Mem0 paper](https://arxiv.org/pdf/2504.19413) (Apr 2025)
   - [Google LangExtract](https://github.com/google/langextract) — multi-pass entity extraction with source grounding
   - Haystack blog: ["Advanced RAG: Query Expansion"](https://haystack.deepset.ai/blog/query-expansion) (2024)
   - Elastic blog: ["Advanced RAG techniques: Data processing & ingestion"](https://www.elastic.co/search-labs/blog/advanced-rag-techniques-part-1) (2024)

4. **Search queries used** (reproducibility):
   - "lost in the middle LLM long context attention position bias 2024 2025"
   - "LLM reliable counting entity extraction long context structured input 2024 2025"
   - "conversational recommendation system implicit preference cross-domain transfer 2024 2025"
   - "BM25 FTS vocabulary gap query expansion deterministic retrieval 2024 2025"
   - "Anthropic Claude prompting best practices long context extraction 2025 2026"
   - "RAG query expansion without LLM deterministic synonym expansion document enrichment 2024 2025"
   - "PROSE implicit preference sub-components LLM recommendation without fine-tuning 2025"
   - "LLM extract then verify two-pass entity extraction long document"

### Constraint reminder

Spectral's five architectural commitments:
1. Deterministic recognition; no LLM-in-loop except in the actor
2. Single-pipeline cascade
3. Recognition architecture (co-retrieval, declarative density, descriptions, session signal)
4. No external vector/embedding dependencies; FTS K=40 with re-ranking
5. Bench uses Claude Sonnet 4.5 actor; model upgrade out of scope

---

## Section 2 — Domain 1: Long-Context Entity Extraction and Reliable Counting

**Spectral failure mode**: Multi-session counting (55%). Actor receives 80-90 retrieved memories, scans them, and undercounts by 1-2. Missed items are explicitly stated but are subordinate clauses within conversations about a different primary topic ("embedded-reference-in-different-primary-context").

### Technique 1: Quote-First Extraction (Scratchpad)

**Source**: [Anthropic long context prompting blog](https://www.anthropic.com/news/prompting-long-context) (Sep 2023); [Anthropic prompting best practices](https://platform.claude.com/docs/en/build-with-claude/prompt-engineering/claude-prompting-best-practices) (current).

**Technique**: Before answering, instruct the actor to extract and quote relevant passages from the input. Anthropic's recommended pattern: "Find quotes from [source] that are relevant to [task]. Place these in `<quotes>` tags. Then, based on these quotes, [do the task]."

**Evidence**: Anthropic's own benchmarks show scratchpad extraction reduces error rate by 36% on 95K-token contexts (Claude 2: 93.9% → 96.1%). "Comes at a small cost to latency, but improves accuracy." The technique forces the model to materialize evidence before reasoning, which mechanistically addresses Spectral's failure mode — the actor currently reasons over sessions holistically rather than first extracting candidate mentions.

**Applicability**: **Compatible**. This is a prompt-level change to the actor template. No retrieval or pipeline changes needed. The actor already receives session-grouped memories; adding a "first, quote every mention of [counted item] from each session" instruction is a single-prompt modification. Cost: doubles output tokens (quotes + answer). At Sonnet 4.5 pricing, ~$0.003 per question.

### Technique 2: Two-Call Extract-Then-Count Pattern

**Source**: Google [LangExtract](https://github.com/google/langextract) (2025) — multi-pass extraction for recall improvement. Databricks ["End-to-End Structured Extraction with LLM"](https://community.databricks.com/t5/technical-blog/end-to-end-structured-extraction-with-llm-part-1-batch-entity/ba-p/98396) (2024). General pattern described in [Multi-Step Reasoning survey](https://dl.acm.org/doi/10.1145/3774896) (ACM Computing Surveys 2025).

**Technique**: Call 1 extracts structured candidates (JSON array of {item, source_session, quote}). Call 2 receives the candidate list plus the original question, deduplicates, and produces the final count. Separation of concerns: extraction is a recall-optimized task; counting is a precision-optimized task.

**Evidence**: LangExtract's multi-pass strategy reports improved recall on long documents by running extraction multiple times and merging results. The Databricks pattern decomposes extraction into phases (entity recognition → relationship extraction → validation), each focused on a single goal. No directly comparable bench numbers on counting specifically, but the decomposition principle is well-established in the multi-step reasoning literature.

**Applicability**: **Compatible with modifications**. Requires a second LLM call, which doubles actor cost per question (~$0.006 total). The bench harness currently makes one actor call; would need a two-call mode. No retrieval changes. The two-call pattern fits within "no LLM-in-loop except in the actor" — both calls are actor calls, not retrieval calls.

### Technique 3: Position-Aware Formatting

**Source**: [Lost in the Middle](https://arxiv.org/abs/2307.03172) (TACL 2024). [Anthropic long context tips](https://platform.claude.com/docs/en/build-with-claude/prompt-engineering/claude-prompting-best-practices).

**Technique**: The "Lost in the Middle" finding: LLMs exhibit U-shaped attention — tokens at the beginning and end of input receive higher attention than those in the middle. Performance on multi-document QA degrades significantly when relevant information is in the middle of long contexts. Anthropic's own guidance: "Put longform data at the top... queries at the end can improve response quality by up to 30%."

**Evidence**: Liu et al. show "performance is often highest when relevant information occurs at the beginning or end of the input context." The effect is particularly strong for models using RoPE (Rotary Position Embedding), which introduces a decay effect. However, note that this finding is from 2023 models — newer models (Claude Sonnet 4.5, Opus 4.6) may have reduced but not eliminated this bias. Anthropic's MRCR v2 benchmark shows Opus 4.6 at 76% retrieval accuracy at 1M tokens, suggesting position bias is reduced but not zero.

**Applicability**: **Compatible**. Spectral already places memories above the question (correct per Anthropic guidance). The remaining lever is randomizing or rotating session order across bench runs to diagnose whether position bias explains specific failures (e.g., are the missed wedding sessions consistently in the middle?). This is a diagnostic technique, not a fix. If position bias is confirmed, shuffling session order per-query or placing answer-likely sessions at extremes would help — but we can't know which sessions contain answers at retrieval time.

### Technique 4: Self-Consistency Sampling

**Source**: [RASC](https://aclanthology.org/2025.naacl-long.184/) (NAACL 2025). [CISC](https://aclanthology.org/2025.findings-acl.1030.pdf) (ACL 2025 Findings).

**Technique**: Generate N independent completions for the same prompt, take majority vote. RASC adds quality-weighted voting; CISC uses model confidence to weight votes. Reported 70% reduction in sample usage vs. naive self-consistency while maintaining accuracy.

**Evidence**: Well-established technique for improving reliability. CISC shows that 10 confidence-weighted samples match 18.6 uniform samples. However, this addresses stochastic variance (the model knows the answer but sometimes generates wrong output), not systematic blindness (the model consistently fails to notice embedded references). Spectral's counting failures are systematic — the actor consistently undercounts by 1-2, suggesting it doesn't see the embedded references, not that it randomly misses them.

**Applicability**: **Compatible but likely low-lift for Spectral's failure mode**. Requires N actor calls per question (3-5x cost increase). Would help if failures are stochastic; unlikely to help if failures are systematic attention misses. Worth testing on 2-3 failure cases: if 5 independent completions all miss the same item, self-consistency won't help. If some catch it, it will.

### Technique 5: Focused Chain-of-Thought with Structured Input

**Source**: ["Focused Chain-of-Thought: Efficient LLM Reasoning via Structured Input Information"](https://arxiv.org/html/2511.22176v1) (Nov 2025).

**Technique**: Pre-process structured input into "context blocks" focused on the specific task, then have the model reason only over the relevant blocks. Key insight: rather than asking the model to scan all input for relevant information, pre-extract the relevant portions into a focused context, reducing noise.

**Evidence**: Produces "significantly shorter reasoning traces" and improved accuracy on structured extraction tasks compared to natural-language-input baselines. The technique works at the prompt level — no model fine-tuning required.

**Applicability**: **Compatible with modifications**. In Spectral's case, this would mean pre-filtering retrieved memories to only those mentioning the counted entity (or related terms) before passing to the actor. This is a retrieval-side change — a focused re-retrieval pass using the question's entity as a filter. However, this risks circular failure: if the entity appears as a subordinate mention, the pre-filter may not catch it either. The technique is more applicable when entities can be reliably identified lexically, which is exactly the case where Spectral's actor already succeeds.

### Domain 1 Summary

| Technique | Bucket | Estimated lift for Spectral | Cost |
|-----------|--------|---------------------------|------|
| Quote-first extraction | Compatible | Medium-high — directly addresses the failure mechanism | +1x output tokens per question |
| Two-call extract-then-count | Compatible with modifications | Medium — separation helps but both calls face same attention challenge | +1x actor call per question |
| Position-aware formatting | Compatible (diagnostic) | Low — Spectral already follows Anthropic placement guidance | Zero (diagnostic only) |
| Self-consistency sampling | Compatible but low-lift | Low — failures appear systematic, not stochastic | 3-5x actor cost |
| Focused CoT with structured input | Compatible with modifications | Low — pre-filtering risks the same miss as the actor | Retrieval-side change |

---

## Section 3 — Domain 2: Implicit Preference Modeling and Cross-Domain Transfer

**Spectral failure mode**: Single-session-preference (60%). Two sub-failures: (a) actor refuses to transfer preferences across domains (Seattle hotels → Miami hotels) when prompt is restrictive; (b) session-user confusion when different session IDs appear.

### Technique 1: Explicit-OR-Implicit Framing (Pre-#91 Prompt)

**Source**: Spectral's own bench data. Pre-#91 prompt: "explicit statements OR implicit signals from past activities." Post-#91: "explicit statements... Prefer these over inferred preferences." The pre-#91 framing achieved 60%; post-#91 achieved 35%.

**Technique**: Frame preference instructions to explicitly permit inference from implicit behavioral signals, not just stated preferences. The ICLR 2025 PersonalLLM benchmark ([Do LLMs Recognize Your Preferences?](https://openreview.net/forum?id=QWunLKbBGF)) found that "models need to infer from implicit preferences and dynamically apply this understanding across conversation contexts rather than simply retrieving explicit preferences."

**Evidence**: Spectral's own -25pp regression is the strongest evidence. The PersonalLLM benchmark supports the principle: LLMs that rely only on explicit preferences underperform those that infer from behavioral signals. The ["Extracting Implicit User Preferences in Conversational Recommender Systems"](https://www.mdpi.com/2227-7390/13/2/221) paper (2025) confirms that implicit signal extraction significantly improves recommendation quality in conversational systems.

**Applicability**: **Compatible — already implemented** via the full revert in `fix/preference-prompt-full-revert` branch. The pre-#91 "explicit OR implicit" framing is the field-validated approach.

### Technique 2: Preference Decomposition (PROSE)

**Source**: Apple ML Research, ["Aligning LLMs by Predicting Preferences from User Writing Samples"](https://arxiv.org/html/2505.23815v1) (May 2025).

**Technique**: PROSE breaks preferences into sub-components (style, tone, structure, content focus) and infers each independently from user writing samples. Then verifies inferred preferences across multiple samples. Achieves 33% improvement over CIPHER (prior state-of-the-art) without fine-tuning.

**Evidence**: Evaluated on Qwen2.5, GPT-mini, and GPT-4o. The key insight — decomposing "preference" into typed sub-components — is transferable. In Spectral's context, this would mean the actor explicitly decomposing "hotel preference" into {location preference, amenity preference, style preference, budget preference} and checking each against conversation history.

**Applicability**: **Compatible with modifications**. Would require changing the actor prompt to decompose the recommendation question into preference sub-components before synthesizing. No retrieval changes. Could be implemented as a prompt-level change. Risk: increases output length and cost without guaranteed lift on Spectral's specific failure cases, which are about cross-domain transfer rather than preference granularity.

### Technique 3: Cross-Domain Transfer via LLM Bridging

**Source**: [LLMCDSR](https://dl.acm.org/doi/10.1145/3715099) (ACM TOIS 2025). [Cross-domain Recommendation from Implicit Feedback](https://openreview.net/forum?id=wi8wMFuO0H) (ICLR 2024 submission).

**Technique**: Use LLMs to bridge domain gaps by understanding that preferences in one domain (hotels in Seattle) are transferable evidence about preferences in another domain (hotels in Miami). LLMCDSR explicitly uses LLMs to "bridge the domain gap and align single- and cross-domain data."

**Evidence**: The cross-domain recommendation literature consistently finds that user preferences transfer across related domains, particularly for attribute-level preferences (style, quality tier, amenity type) as opposed to item-specific preferences. The challenge is determining which preference dimensions transfer and which don't.

**Applicability**: **Incompatible as architecturally described** — LLMCDSR uses fine-tuned models and embedding-space alignment. However, the core insight is applicable via prompt framing: instruct the actor to identify transferable preference dimensions explicitly. E.g., "If the user has expressed preferences about hotels in one city, those preferences (amenity type, style, budget) likely apply to hotels in other cities." This reduces to a prompt-level instruction, which is compatible.

### Technique 4: Session-User Clarity Instruction

**Source**: Spectral's own failure analysis. Camping case in counting_enumerate (PR #93). Colleagues case in preference (PR #94).

**Technique**: Add explicit instruction: "All retrieved memories are about you across multiple sessions. Different session IDs do not mean different users." Prevents the actor from attributing memories in different sessions to different people.

**Evidence**: Directly observed in two distinct failure cases across two prompt templates. The actor literally said "wait, that was a different user in Session answer_f7b22c66."

**Applicability**: **Compatible — already implemented** in both counting_enumerate.md and preference.md (via `fix/preference-prompt-full-revert`).

### Technique 5: Preference Elicitation via Clarifying Questions (GATE)

**Source**: [GATE: Eliciting Human Preferences with Language Models](https://arxiv.org/abs/2310.11589) (ICLR 2025).

**Technique**: Instead of single-shot recommendation, the system asks clarifying questions to elicit preferences. GATE uses "free-form, language-based interaction" to iteratively refine understanding of user intent.

**Evidence**: GATE outperforms static preference extraction in interactive settings. However, it fundamentally requires multi-turn interaction with the user.

**Applicability**: **Incompatible**. Spectral's bench is single-shot per query. The preference actor receives memories and a question and must produce a recommendation in one call. Multi-turn preference elicitation is out of scope architecturally.

### Domain 2 Summary

| Technique | Bucket | Estimated lift for Spectral | Cost |
|-----------|--------|---------------------------|------|
| Explicit-OR-implicit framing | Compatible (implemented) | Already measured: restores 60% baseline | Zero (prompt change) |
| Preference decomposition (PROSE) | Compatible with modifications | Low-medium — addresses granularity, not cross-domain transfer | +output tokens |
| Cross-domain transfer bridging | Compatible (as prompt instruction) | Medium — could help Miami/furniture cases specifically | Zero (prompt change) |
| Session-user clarity | Compatible (implemented) | Targeted — fixes 1 specific failure | Zero (prompt change) |
| GATE preference elicitation | Incompatible | N/A — requires multi-turn interaction | N/A |

---

## Section 4 — Domain 3: Vocabulary-Gap Retrieval

**Spectral failure mode**: 3 single-session-preference RETRIEVAL_MISS cases where query and answer session share semantic meaning but few lexical terms. "homegrown ingredients" vs "basil and mint"; "battery life" vs "power bank"; "coffee creamer recipe" vs "flavored creamer with almond milk."

### Technique 1: Contextual BM25 (Document Enrichment)

**Source**: [Anthropic Contextual Retrieval blog](https://www.anthropic.com/news/contextual-retrieval) (2024).

**Technique**: Before indexing, prepend each chunk with a short (50-100 token) contextual description generated by an LLM. The description explains the chunk's content in broader terms. Then index the enriched text in BM25. Example: a memory about "fresh basil and mint from the garden" gets a description like "User grows herbs and vegetables at home for cooking." Now a query for "homegrown ingredients" matches via the description.

**Evidence**: Anthropic's own benchmarks: Contextual Embeddings alone reduced retrieval failures by 35%. Combined Contextual Embeddings + Contextual BM25 reduced failures by 49%. With reranking: 67% reduction. Measured as 1 - recall@20.

**Applicability**: **Compatible — this is exactly backlog item #8 (compiled-truth boost)**. Spectral already has a `description` field on memories (PR #75) and a Librarian pipeline in Permagent to populate descriptions. The remaining work is: (a) populate descriptions for bench memories, (b) include descriptions in FTS index, (c) boost hits that match on description. No vector embeddings needed — the descriptions are indexed as additional FTS text. This is the highest-confidence technique in the entire synthesis because it has Anthropic's own validation and maps directly to existing Spectral infrastructure.

### Technique 2: Deterministic Query Expansion (WordNet / Synonym Lists)

**Source**: Classic IR technique. Recent validation: [Improving Terminologies Synonym Expansion Model](https://jisis.org/wp-content/uploads/2024/05/2024.I2.015.pdf) (2024). [WordNet-based query expansion](https://arxiv.org/abs/1309.4938) (classic, revalidated 2024).

**Technique**: Expand the query with synonyms from WordNet or a domain-specific thesaurus before running BM25. "homegrown ingredients" → "homegrown ingredients OR garden vegetables OR fresh herbs." Fully deterministic — no LLM call.

**Evidence**: WordNet expansion has a long history in IR. Recent (2024) studies using TextRank for term identification + WordNet for synonym expansion show improvements on standard benchmarks. However, the lift is modest compared to semantic approaches, and the quality depends heavily on the thesaurus coverage for the specific domain vocabulary.

**Applicability**: **Compatible**. Could be added as a pre-retrieval stage in the cascade pipeline. Spectral would need a synonym dictionary or WordNet integration. Risk: false expansion ("battery" → "assault"?) and expansion noise overwhelming true matches. The 3 failure cases have domain-specific vocabulary gaps that general synonyms may not bridge ("homegrown ingredients" → "garden herbs" requires domain knowledge, not just synonymy).

### Technique 3: LLM-Generated Query Expansion (HyDE / Pseudo-Document)

**Source**: [Multi-model pseudo-document generation (MPQE)](https://www.sciencedirect.com/science/article/abs/pii/S0306457325004844) (2025). Haystack ["Advanced RAG: Query Expansion"](https://haystack.deepset.ai/blog/query-expansion) (2024).

**Technique**: Use an LLM to generate a hypothetical answer or pseudo-document for the query, then use terms from the generated text as expanded query terms for BM25. Query: "homegrown ingredients" → LLM generates: "Fresh herbs like basil, mint, and cilantro from the home garden, along with tomatoes and peppers" → BM25 now searches for "basil", "mint", "garden", "tomatoes."

**Evidence**: MPQE reports 3-17% improvement in BM25 effectiveness on MS MARCO and TREC DL benchmarks. HyDE (Hypothetical Document Embeddings) is a well-validated technique, though originally designed for vector retrieval.

**Applicability**: **Incompatible**. Requires an LLM call in the retrieval path, violating commitment #1 (deterministic recognition; no LLM-in-loop except in the actor). The LLM call would be a query expansion step before FTS, which is definitionally LLM-in-loop retrieval.

### Technique 4: Multi-Signal Retrieval Fusion

**Source**: [Mem0 architecture](https://mem0.ai/blog/state-of-ai-agent-memory-2026) (2026). [Hybrid BM25 Retrieval overview](https://www.emergentmind.com/topics/hybrid-bm25-retrieval) (2024).

**Technique**: Run multiple retrieval signals in parallel — semantic similarity, keyword matching (BM25), entity matching — and fuse scores. Mem0 reports their combined score "outperforms any individual signal," with +29.6pp on temporal queries and +23.1pp on multi-hop reasoning.

**Evidence**: Mem0's LOCOMO benchmark results show multi-signal fusion outperforms OpenAI's native memory by ~26%. The architecture uses semantic vector search, graph-based relationship storage, and key-value lookups in parallel.

**Applicability**: **Partially compatible**. Spectral already has multiple retrieval signals (FTS, co-retrieval pairs, declarative density, session signal, ambient boost) fused via `apply_reranking_pipeline()`. This is architecturally the same pattern as Mem0's multi-signal fusion, minus the semantic vector component. The vector component violates commitment #4. However, Spectral could add more deterministic signals (entity matching, description-text matching) to the fusion without vectors. The architecture supports it — each signal is a weight in `RerankingConfig`.

### Technique 5: Document-Level Summary Indexing

**Source**: RAG survey: ["Retrieval-Augmented Generation for LLMs: A Survey"](https://www.rivista.ai/wp-content/uploads/2025/09/2312.10997v5.pdf) (2025 update). [Advanced RAG techniques](https://www.elastic.co/search-labs/blog/advanced-rag-techniques-part-1) (Elastic, 2024).

**Technique**: Create hierarchical indexes: store both individual chunks and document/session-level summaries. Summaries use broader vocabulary than individual chunks, bridging vocabulary gaps. A summary of a session about "growing tomatoes, basil, and mint in the garden" would likely include terms like "home gardening," "fresh ingredients," and "organic produce."

**Evidence**: The RAG survey identifies hierarchical indexing as a standard technique: "Data summaries are stored at each node in hierarchical index structures, aiding in swift traversal and determining which chunks to extract." Elastic's implementation stores summaries alongside chunks for multi-granularity retrieval.

**Applicability**: **Compatible with modifications — this is backlog item #12 (L2 episode summaries)**. Session-level summaries indexed in FTS would bridge vocabulary gaps by introducing broader vocabulary. The cascade architecture decision doc already identifies this as a pipeline stage. The distinction from Technique 1 (contextual BM25) is granularity: Technique 1 enriches individual memories; Technique 5 creates session-level summaries as separate retrieval targets.

### Domain 3 Summary

| Technique | Bucket | Estimated lift for Spectral | Cost |
|-----------|--------|---------------------------|------|
| Contextual BM25 (document enrichment) | Compatible (= item #8) | High — Anthropic-validated, maps to existing infra | Librarian pipeline + FTS index change |
| WordNet query expansion | Compatible | Low-medium — general synonyms may not bridge domain gaps | WordNet integration |
| LLM query expansion (HyDE) | Incompatible | N/A — violates no-LLM-in-loop | N/A |
| Multi-signal fusion | Partially compatible (no vectors) | Medium — Spectral already does this; adding more signals helps | Per-signal implementation |
| Session-level summary indexing | Compatible with modifications (= item #12) | Medium-high — broader vocabulary at session granularity | Summary generation + schema change |

---

## Section 5 — Cross-Cutting Findings

### Theme 1: The field consistently recommends multi-step actor patterns

Across all three domains, the most effective techniques involve decomposing the actor's task:

- **Counting**: Quote-first extraction separates evidence gathering from reasoning
- **Preference**: PROSE decomposes preferences into typed sub-components
- **Retrieval**: Contextual BM25 pre-computes descriptions at ingest time (multi-step at index time, not query time)

The common principle: single-pass holistic reasoning over long contexts is unreliable. Decomposition into focused sub-tasks improves accuracy. This is consistent with Spectral's own finding that the counting actor "tracks the primary topic and doesn't register subordinate mentions."

**Implication for Spectral**: The quote-first extraction pattern is the lowest-cost way to introduce decomposition. It doesn't require a second LLM call — it changes what the actor produces in its single call (quotes first, then answer).

### Theme 2: The field consistently uses vector embeddings for vocabulary-gap retrieval

Every high-performing retrieval system in the literature uses semantic embeddings to bridge vocabulary gaps:
- Mem0: semantic + BM25 + entity matching
- Contextual Retrieval: contextual embeddings + contextual BM25
- MPQE: pseudo-document generation for embedding-based retrieval
- LLMCDSR: embedding-space alignment across domains

**This is incompatible with Spectral's commitment #4.** The field's primary answer to vocabulary-gap retrieval is vector similarity, and Spectral has deliberately excluded it. The workarounds — document enrichment (Technique 1), synonym expansion (Technique 2), summary indexing (Technique 5) — are the field's secondary answers. They work, but they don't match the effectiveness of semantic embeddings.

**Implication for Spectral**: Document enrichment (item #8) is the best available path within Spectral's constraints. If the 3 vocabulary-gap failures remain after descriptions are populated and indexed, the architectural commitment to no-vector-embeddings may need revisiting — but that's a future decision, not a current one.

### Theme 3: Position bias is reducing but not eliminated in modern models

"Lost in the Middle" was published with 2023-era models. Newer models (Claude Sonnet 4.5, Opus 4.6) have reduced but not eliminated the U-shaped attention pattern. Anthropic's own guidance still recommends placing queries at the end of long contexts "for up to 30% improvement." The "Found in the Middle" calibration methods (2024) and Ms-PoE (NeurIPS 2024) are model-training-time interventions, not applicable to prompt engineering.

**Implication for Spectral**: Position bias is a plausible contributing factor to counting failures (sessions in the middle of the input get less attention) but is not directly fixable from the prompt side. The quote-first extraction pattern is an indirect mitigation — it forces the model to attend to all sessions before reasoning.

### Theme 4: What the field does NOT have good answers for

**Embedded-reference extraction in different-primary-context text.** This is Spectral's core multi-session failure mode, and I found no research directly addressing it. The closest is Google's LangExtract multi-pass strategy, but that addresses missed entities in general, not specifically subordinate mentions within different-topic contexts. The "Lost in the Middle" literature addresses position-based attention but not topic-based attention filtering.

**Cross-domain preference transfer in single-shot settings.** The recommendation literature (LLMCDSR, cross-domain transfer networks) addresses this with multi-turn or embedding-based approaches. The single-shot setting (one query, one response, no follow-up) with deterministic retrieval is an underexplored combination. Spectral's pre-#91 prompt ("explicit OR implicit signals") is essentially a handcrafted solution to a problem the field solves with more complex architectures.

---

## Section 6 — Mapped Recommendations

### Failure Mode 1: Multi-session counting (55% → target 65%+)

**Ranked by expected lift within Spectral's constraints:**

1. **Quote-first extraction** (Technique 1)
   - **Integration**: Add to counting_enumerate.md: "Before counting, quote every mention of [the counted item] you find in each session. Place quotes in `<quotes>` tags with session IDs. Then count the unique items from your quotes."
   - **Effort**: Small (prompt change only)
   - **Lift estimate**: Medium-high. Directly addresses the failure mechanism by forcing evidence materialization before reasoning. Anthropic's own 36% error reduction on long-context extraction supports this.
   - **Risk**: Increases output tokens ~2x. Some risk of the actor still missing subordinate mentions during the quote-extraction pass (same attention challenge, but now the task is "find mentions" rather than "count items," which may activate different attention patterns).

2. **Two-call extract-then-count** (Technique 2)
   - **Integration**: Bench harness change: call 1 uses an extraction-focused prompt ("List every [item] mentioned in these sessions as a JSON array with session_id and quote"); call 2 uses a counting-focused prompt ("Given these extracted items, deduplicate and count").
   - **Effort**: Medium (harness change + new prompt template)
   - **Lift estimate**: Medium. Addresses the same mechanism as quote-first but with stronger separation. Call 1 is purely extraction (high recall, accept false positives); call 2 is purely deduplication/counting (high precision).
   - **Risk**: 2x actor cost. If call 1 still misses embedded references, call 2 can't recover them. The marginal lift over quote-first (single-call) is uncertain.

3. **Position-bias diagnostic** (Technique 3)
   - **Integration**: Run a diagnostic bench where session order is randomized per question. Compare failure rates for answer sessions by position (first, middle, last).
   - **Effort**: Small (bench config change)
   - **Lift estimate**: Low (diagnostic only). If position bias is confirmed, session shuffling becomes a mitigation, but the lift is bounded by how many failures are position-caused vs. topic-caused.
   - **Risk**: None (diagnostic).

### Failure Mode 2: Single-session-preference (60% → target 65%+)

1. **Explicit-OR-implicit framing** (already implemented)
   - Restores the pre-#91 baseline of 60%. No additional work needed.

2. **Cross-domain transfer prompt instruction** (Technique 3 from Domain 2)
   - **Integration**: Add to preference.md: "When the user has expressed preferences about [topic] in one context (e.g., hotels in Seattle), those attribute-level preferences (style, amenities, budget) likely apply in related contexts (e.g., hotels in Miami). Apply transferable preference dimensions when direct evidence for the specific question is unavailable."
   - **Effort**: Small (prompt addition)
   - **Lift estimate**: Low-medium. Directly targets the Miami hotel and furniture rearranging cases. Risk of over-generalizing (transferring cooking preferences to hiking).
   - **Risk**: May cause false positives on cases where domain transfer is incorrect. Need to verify against the 7 currently-correct cases.

3. **Preference decomposition** (PROSE-inspired, Technique 2 from Domain 2)
   - **Integration**: Add to preference.md: "Before recommending, list the user's relevant preference dimensions: style, quality tier, budget, specific features, past positive experiences. Then synthesize a recommendation grounded in those dimensions."
   - **Effort**: Small (prompt addition)
   - **Lift estimate**: Low. The 5 ACTOR_MISS failures are about inference breadth, not preference granularity.
   - **Risk**: Increases output length. May not address the core failure mode.

### Failure Mode 3: Vocabulary-gap retrieval (3 specific cases)

1. **Contextual BM25 / compiled-truth boost (item #8)**
   - **Integration**: Populate descriptions for bench memories via Librarian, index descriptions in FTS, add description-match boost to re-ranking.
   - **Effort**: Medium-large (Librarian pipeline, FTS index change, re-ranking weight)
   - **Lift estimate**: High. Anthropic's 49% retrieval failure reduction validates the approach. The 3 vocabulary-gap cases are textbook examples of what description enrichment fixes.
   - **Risk**: Quality depends on Librarian-generated descriptions. If descriptions are too generic ("user discussed food"), they don't bridge the gap. If too specific ("user grows basil"), they do.

2. **Session-level summary indexing (item #12)**
   - **Integration**: Generate session-level summaries, store as FTS-indexed entities, retrieve alongside individual memories.
   - **Effort**: Large (schema change, summary generation, retrieval changes)
   - **Lift estimate**: Medium. Summaries introduce broader vocabulary but the lift depends on summary quality.
   - **Risk**: Summary-vs-memory ranking decisions. When to surface a summary vs. constituent memories is an open design question (noted in cascade-architecture-decision.md).

3. **WordNet synonym expansion**
   - **Integration**: Pre-retrieval stage: expand query terms with WordNet synonyms before FTS.
   - **Effort**: Medium (WordNet integration, expansion logic)
   - **Lift estimate**: Low. The vocabulary gaps are domain-specific, not general synonymy. "Homegrown ingredients" → WordNet → "domestic, native" — not "basil, mint, garden."
   - **Risk**: Expansion noise, false matches.

---

## Section 7 — Open Questions

### 1. Is the embedded-reference failure mode attention-based or topic-based?

The "Lost in the Middle" literature addresses position-based attention. Spectral's failure mode appears to be topic-based: the actor attends to sessions' primary topics and filters out subordinate mentions. These are different mechanisms. No research directly addresses topic-based attention filtering in LLMs. The quote-first extraction pattern is a promising mitigation, but it's not validated against this specific failure mode.

**Worth investigating**: Does forcing the actor to "scan each session independently for mentions of [item]" (per-session extraction) outperform "scan all sessions for mentions of [item]" (holistic extraction)? Per-session extraction might avoid the topic-filtering problem because each session is processed in isolation, but it's more expensive (N calls for N sessions).

### 2. How much lift does description quality drive in contextual BM25?

Anthropic's 49% retrieval improvement assumes high-quality contextual descriptions. Spectral's descriptions will be Librarian-generated. The quality of these descriptions — specifically, whether they introduce the right bridging vocabulary — determines whether item #8 fixes the 3 vocabulary-gap cases or not.

**Worth investigating**: Before building the full pipeline, manually write ideal descriptions for the 3 failure cases and re-run retrieval to confirm they bridge the gap. This is a 30-minute validation that de-risks a medium-large engineering effort.

### 3. Does the quote-first pattern compose with existing prompt instructions?

The counting_enumerate.md prompt already has 5 instructions (scan, embedded-reference, session-user clarity, synthesis, conciseness). Adding a quote-first instruction makes 6. At some point, instruction count itself becomes a reliability risk (the model may prioritize some instructions over others). The Anthropic guidance says "be specific about desired output format and constraints" but doesn't address instruction-count limits.

**Worth investigating**: Test the quote-first pattern on 3-5 multi-session counting failures before committing to a full bench run. If the actor extracts the embedded references in the quoting pass, the mechanism is validated. If it still misses them, the failure is deeper than instruction framing.

### 4. Is Spectral's no-vector-embedding commitment permanently load-bearing?

The field's consistent answer to vocabulary-gap retrieval is semantic embeddings. Spectral's document enrichment approach is the best available workaround, but if 3-5 failure cases remain after descriptions are populated, the question becomes: is the operational cost of vector embeddings (model download, encoding latency, disk) worth the retrieval quality gain?

The original decision (RESULTS.md) was based on operational simplicity and the paraphrase-query weakness being limited to "zero word overlap" cases. The 3 preference failures are exactly this class. This isn't an urgent question — item #8 should be tried first — but it's the architectural question that the research surfaces most clearly.

### 5. What is the field learning about Sonnet 4.5's specific attention patterns?

The bench uses Sonnet 4.5 (now superseded by Sonnet 4.6 and Opus 4.7). Anthropic's Opus 4.7 prompting guide notes "more literal instruction following" and "will not silently generalize an instruction from one item to another." This literalism may exacerbate the embedded-reference problem (the model follows the "count [item]" instruction literally and doesn't generalize to subordinate mentions). Conversely, it may make the quote-first instruction more reliable (the model will follow "quote every mention" literally).

The bench is currently committed to Sonnet 4.5. If prompt-level interventions plateau, the model choice itself becomes a variable — but that's explicitly out of scope per the architectural commitments.
