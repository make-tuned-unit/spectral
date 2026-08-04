# Source survey — deterministic and LLM-driven agent memory — 2026-08-03

## The finding that matters most

**The LongMemEval state of the art does no retrieval ranking at all.**

Mastra's Observational Memory — 84.23% with gpt-4o (the official benchmark
model), 94.87% with gpt-5-mini — uses **static retrieval**:

> "OM uses static retrieval — the main agent accesses observations without
> dynamic per-turn reranking. Context window has fixed structure: observations
> (prefix) + message history. No dynamic injection or query-based retrieval."

There is no ranker, no query-conditioned scoring, no per-question routing. The
whole system is a compressed observation log placed in the prompt prefix.

This is independent, external confirmation of what every measurement in this
repo has been saying: **retrieval ranking is not where LongMemEval accuracy
lives.** Spectral spent months on ranking levers and refuted every one of them;
the benchmark leader skipped ranking entirely and beat the oracle configuration
by 2 points.

## How Mastra actually wins

Two background LLM agents at **write** time:

| tier | what it holds |
|---|---|
| 1. Message history | raw turns, recent |
| 2. Observations | dated, prioritized event log written by an **Observer** agent (`gemini-2.5-flash`) |
| 3. Reflections | condensed observations from a **Reflector** agent that merges related items and *removes superseded ones* |

Compression 3–6× on text (6× in their LongMemEval runs), ~30k-token context.
Append-only prefix gives high prompt-cache hit rates, cutting cost 4–10x versus
uncached.

**This is the mechanism Spectral's whole thesis rejects.** It is LLM-driven
memory construction — non-deterministic, token-costly at write, opaque in its
pruning decisions. DMF's entire paper is an argument against it. And it is
currently winning the benchmark by a clear margin.

## The honest strategic read

Spectral and DMF are competing on **cost and determinism**; Mastra is competing
on **accuracy** and buying it with LLM tokens at write time. These are different
axes, and on the accuracy axis the deterministic approaches are structurally
behind, because the winning move is exactly the one they forbid.

Numbers, stated plainly:

| system | LongMemEval | memory-layer LLM cost |
|---|---:|---|
| Mastra OM (gpt-5-mini) | 94.87% | Observer + Reflector agents at write time |
| Mastra OM (gpt-4o) | 84.23% | same |
| **Spectral (Sonnet 4.6)** | **81.5%** | **~$0.25/1k queries, optional; recall itself $0** |
| DMF (vs Mem0, LongMemEval-10) | 0.717 (n=60) | 0 |
| Mem0 | 81.6% (Supermemory) / 71.2% (Zep) per Mastra's comparison | LLM summarisation |

Spectral at 81.5% is competitive with the gpt-4o-era field at a fraction of the
cost. It is not going to reach 94% by improving retrieval, and this survey is
the third independent line of evidence for that.

## The one transferable deterministic idea

**Three-date temporal anchoring.** Every Mastra observation carries:

1. the observation's own date,
2. the *referenced* date mentioned in content ("my flight is January 31"),
3. a computed *relative* offset ("2 days from today").

They credit this for temporal-reasoning at 95.5%. It is pure deterministic
string assembly — no model needed — and it is the same pattern the Spectral
harness comment already attributes to "Mastra OM, 84.23%".

**Spectral has (1) and (3) implemented and (3) is default-off and never
measured.** `RenderOptions::relative_offsets` produces
`--- Session s1 (2023/02/15, 4 months ago) ---`. It has unit tests. It has never
been run end to end.

(2) — extracting dates *referenced inside* content and anchoring them — is not
implemented, and is the genuinely new piece. `spectral::temporal` already has
`resolve_relative_dates`, so the parsing exists; it is not wired into rendering.

### Why this cannot be settled at $0

Relative offsets change only the **rendered text**, not `retrieved_keys`. The
oracle's metrics all derive from retrieved keys, so it is blind to this — the
same limitation established for `SessionOrder::ByRank`
(`retrieval-foundation-2026-08-02.md`). Measuring it requires a paid actor run.

That makes it the strongest remaining candidate for the next paid experiment:
deterministic, $0 at runtime, already built, and credited by the benchmark
leader for its best category.

## Other sources noted, not yet read

- **TiMem** — temporal-hierarchical memory consolidation (arXiv 2601.02845)
- **"Hindsight is 20/20"** — agent memory that retains, recalls, reflects
  (arXiv 2512.12818)

Both are consolidation/reflection-shaped, i.e. the same write-time-synthesis
family as Mastra rather than the retrieval family. Reading them is unlikely to
produce a deterministic retrieval lever, on the pattern established here.

## Recommendation

Stop looking for retrieval levers. Three independent lines now agree:

1. Spectral's own measured record — every ranking lever refuted, including the
   query-conditioned family closed this session.
2. Category evidence — `single-session-preference` is 93.3% session-recall and
   56.0% accuracy; `knowledge-update` is 99.4% and 87.2%. The gap is synthesis.
3. This survey — the benchmark leader does no ranking at all.

The two honest paths forward are **(a)** compete on cost/determinism/auditability
and stop treating LongMemEval accuracy as the scoreboard, or **(b)** accept an
LLM at write time and build an observer/reflector equivalent, which contradicts
the current thesis and should be a deliberate decision rather than a drift.

Sources: [Mastra Observational Memory research](https://mastra.ai/research/observational-memory),
[Mastra announcement](https://mastra.ai/blog/observational-memory),
[ZenML LLMOps database entry](https://www.zenml.io/llmops-database/observational-memory-human-inspired-context-compression-for-agent-systems).
