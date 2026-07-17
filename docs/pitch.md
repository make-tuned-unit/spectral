# Spectral — pitch & announcement copy

Reusable, truthful positioning snippets. Every claim maps to code or a measured
benchmark; keep it that way when editing. See `README.md` for the full story and
`docs/RESULTS.md` / `benches/RESULTS.md` for the numbers behind the figures.

## One-liner

> Deterministic, embedding-free memory for AI agents — recall, recognition, and
> adaptive feedback, on one SQLite file. No vector DB, no GPU, no LLM on the
> recall path.

## Elevator pitch (≈60 words)

> Most agent memory is a vector database: an embedding call per query, a service
> to run, and results that drift with the model. Spectral is the opposite — an
> embedded Rust library that recalls (FTS + BM25), recognizes ("have I seen this
> before?"), and adapts to use, all deterministically and embedding-free. One
> SQLite file you own. Federation-ready, poisoning-resistant, ~99% session-recall.

## The six kinds of memory (the taxonomy)

| Kind | Answers | Cost |
|---|---|---|
| Recall | "What do I know about X?" | $0, deterministic |
| Recognition | "Have I seen this before — and is it new?" | $0, deterministic |
| Relational | "How does X relate to Y?" | $0, deterministic |
| Episodic / temporal | "What happened around then?" | $0, deterministic |
| Adaptive | "What matters *now*?" | $0, deterministic |
| Federated | "What do *we* collectively know?" | $0, deterministic |

## X / Twitter (≤280)

> Agent memory without a vector database.
>
> Spectral is an embedded Rust library that recalls, *recognizes* ("have I seen
> this?"), and adapts to use — deterministically, embedding-free, on one SQLite
> file. $0 per query, local-first, federation-ready. ~99% session-recall on
> LongMemEval-S.

## Show HN

**Title:** Show HN: Spectral – deterministic, embedding-free memory for AI agents (Rust)

**Body:**
> Spectral is an embedded memory library for AI agents that skips embeddings and
> vector databases entirely. It gives an agent six kinds of memory behind one
> `Brain` handle on a single SQLite file:
>
> - Recall (FTS5 + BM25) — "what do I know about X?"
> - Recognition — "have I seen this before, and is it new?" — via landmark
>   fingerprinting (Shazam-style) + winnowed k-grams (MOSS) + cognitive-psych
>   scoring, returning a familiarity/novelty verdict with the exact features
>   behind it. (That's where the name comes from: landmarks are spectral peaks
>   above the noise floor.)
> - A typed knowledge graph (2-hop, ontology-validated)
> - Episodic / temporal recall
> - An adaptive feedback loop — used memories strengthen, unused ones decay
> - Read-time federation across brains (provenance-ranked, visibility-scoped,
>   poisoning-resistant)
>
> The point is cost and control: recall and recognition make zero model calls
> (`recognition_token_cost == 0` is structural), so the memory layer is free to
> query and byte-reproducible, and everything lives in one SQLite file you own.
>
> On LongMemEval-S it reaches ~99% session-recall across all six memory-question
> types, embedding-free. It's v0.0.1 and experimental; the retrieval numbers are
> in-sample, held-out expected lower — the repo is candid about what's measured
> vs. not. Apache-2.0.

## LinkedIn / longer

> **Agent memory you can afford and actually own.**
>
> Most "agent memory" today is a vector database — an embedding call on every
> read and write, a service to operate, and rankings that shift when the model
> updates. For a lot of teams that's the wrong shape: it costs per query, it's
> hard to audit, and the data leaves the box.
>
> Spectral takes the other path. It's an embedded Rust library that gives an
> agent six kinds of memory — recall, recognition, relational (graph), episodic,
> adaptive, and federated — all deterministic, all embedding-free, all on one
> SQLite file. Recall and recognition make zero model calls, so the memory layer
> is free to query and byte-reproducible. It recognizes as well as recalls
> ("have I seen this before?"), it learns from use, and it federates across
> brains with built-in poisoning resistance.
>
> ~99% session-recall on LongMemEval-S across every memory-question type. Local-
> first by construction — for teams who keep control of their data.
>
> v0.0.1, experimental, Apache-2.0. github.com/make-tuned-unit/spectral

## Honesty guardrails (don't cut these)

- "No LLM on the recall path" is true for the library; the benchmarked 81.5%
  accuracy uses an optional Haiku query-expansion call (≈$0.25/1k). Disclose it.
- Retrieval numbers are **in-sample**; held-out expected lower.
- Recognition is strong at near-duplicate/verbatim, **not** a paraphrase matcher.
- Sybil resistance in an *untrusted* federation is a deployment-trust property,
  not a code guarantee.
