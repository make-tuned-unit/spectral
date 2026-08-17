# Spectral — pitch & announcement copy

Reusable, truthful positioning snippets. Every claim maps to code or a measured
benchmark; keep it that way when editing. See `README.md` for the full story and
`docs/RESULTS.md` / `benches/RESULTS.md` for the numbers behind the figures.

## One-liner

> Deterministic, embedding-free memory for AI agents — recall, recognition, and
> adaptive feedback, in one SQLite-backed folder. No vector DB, no GPU, no LLM on the
> recall path.

## Elevator pitch (≈60 words)

> Most agent memory is a vector database: an embedding call per query, a service
> to run, and results that drift with the model. Spectral is the opposite — an
> embedded Rust library that recalls (FTS + BM25), recognizes ("have I seen this
> before?"), and adapts to use, all deterministically and embedding-free. One
> SQLite-backed brain folder you own. Federation-ready, resistant to
> score-flooding — 98.6%
> session-recall and 81.5% end-to-end accuracy on LongMemEval-S.

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
> file. $0 per query, local-first, federation-ready. 98.6% session-recall and
> 81.5% end-to-end accuracy on LongMemEval-S.

## Show HN

**Title:** Show HN: Spectral – deterministic, embedding-free memory for AI agents (Rust)

**Body:**
> Spectral is an embedded memory library for AI agents that skips embeddings and
> vector databases entirely. It gives an agent six kinds of memory behind one
> `Brain` handle over a single local brain folder:
>
> - Recall (FTS5 + BM25, plus deterministic local rerank tiers) — "what do I know about X?"
> - Recognition — "have I seen this before, and is it new?" — via landmark
>   fingerprinting (Shazam-style) + winnowed k-grams (MOSS) + cognitive-psych
>   scoring, returning a familiarity/novelty verdict with the exact features
>   behind it. (That's where the name comes from: landmarks are spectral peaks
>   above the noise floor.)
> - A typed knowledge graph (2-hop, ontology-validated)
> - Episodic / temporal recall
> - An adaptive feedback loop — used memories strengthen; unused-ness is
>   tracked, and decay is opt-in (the Archivist), not ambient
> - Read-time federation across brains (provenance-ranked, visibility-scoped,
>   resistant to score-flooding)
>
> The point is cost and control: recall and recognition make zero model calls
> (`recognition_token_cost == 0` is structural), so the memory layer is free to
> query and byte-reproducible (under a read-only or time-anchored open —
> see the guardrails), and everything lives in one local folder you own.
>
> On LongMemEval-S it reaches 98.6% session-recall — 81.5% end-to-end accuracy
> (401/492) — across all six memory-question types, embedding-free. It's v0.0.1 and experimental; the retrieval numbers are
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
> 98.6% session-recall and 81.5% end-to-end accuracy on LongMemEval-S across
> every memory-question type. Local-first by construction — for teams who keep
> control of their data.
>
> v0.0.1, experimental, Apache-2.0. github.com/make-tuned-unit/spectral

## Honesty guardrails (don't cut these)

**This list is copied downstream.** As of 2026-08-17 the permagent.ai site repo
keeps a derived copy in `docs/design/POSITIONING-AND-DEMO-PLAN.md`. This file
is the source of truth — it is tied to measurements, and CI cross-checks the
recognition numbers against `recognition-benchmark-results.json`. When you edit
a guardrail here, propagate it: the copies diverging is silent, and the failure
shows up as a false claim on a public page rather than as a red test.

- Never quote session-recall alone: pair 98.6% (retrieval stage) with 81.5%
  end-to-end accuracy (401/492) in the same sentence. No "~99%" rounding.
- "No LLM on the recall path" is true for the library; the benchmarked 81.5%
  accuracy uses an optional Haiku query-expansion call (≈$0.25/1k). Disclose it.
- Retrieval numbers are **in-sample**; held-out expected lower.
- Recognition is strong at near-duplicate/verbatim, **not** a paraphrase matcher.
- Recognition's verbatim claim is **whitespace-dependent**. Feature extraction
  tokenizes on spaces, so a short passage in an unspaced script (Japanese,
  Chinese) returns `Familiar` rather than `Recognized` even for byte-identical
  input — measured: English recognizes at 34 characters, Japanese does not at
  41, and the same Japanese text with spaces inserted does. Longer CJK
  passages do recognize. Space-separated scripts are unaffected (Russian,
  Arabic, Korean and Thai all verify). **This is not only a consumer
  concern:** the library's own ambient-recurrence loop requires an
  identity-bearing `Recognized` verdict before it will reinforce a prior and
  populate `RememberResult.recurrence` (`brain.rs`, `remember_with`), so that
  loop is silently inert for short CJK — re-encounters never reinforce and
  `recurrence` stays `None`, with no error. `forget()`'s `recognize_clear`
  probe is likewise vacuously true for such content, so `fully_forgotten()`
  reports clean without the probe having proven anything (the deletion itself
  is unaffected). See `crates/spectral-recognition/tests/script_coverage.rs`.
- Sybil resistance in an *untrusted* federation is a deployment-trust property,
  not a code guarantee.
- "Poisoning-resistant" means **score-flood resistant**: RRF over ranks plus a
  per-child cap. It is not a claim about authorship. Federation members are
  unauthenticated by default; the sync layer authenticates objects and
  retractions only under `ImportPolicy::RequireSigned`.
- A brain is a **folder of SQLite databases** (`memory.db`, `graph.sqlite`,
  `recognition.db`), not one file, and writes are not atomic across them —
  never say "a single SQLite file".
- "Deterministic/byte-reproducible" holds for a read-only or time-anchored
  open. The default recall path anchors recency to the wall clock and
  auto-reinforces what it returns, so repeated queries can reorder.
- "Recall = FTS5 + BM25" understates it: the live path is TACT
  (fingerprint -> wing -> FTS) plus deterministic reranking. All still $0.
- "Unused memories decay" is not ambient behaviour: use strengthens, and decay
  runs only via the opt-in Archivist.
- **Never say "frequency-domain", "spectrum", or "FFT".** There is no frequency
  transform anywhere in Spectral. Three separate things share the
  spectral/fingerprint vocabulary and none is one: the **TACT fingerprint** is a
  deterministic hash of four categorical fields
  (`make_fingerprint_hash(hall, target_hall, wing, time_bucket)`) used as a
  routing key; **recognition** is MinHash plus winnowed k-grams
  (Schleimer/MOSS); and `SpectralFingerprint` is seven hand-engineered cognitive
  dimensions, **retired as a recall path** (enabling write-time spectrograms
  changed 0/500 retrieval contexts) and now behind the `spectrogram-legacy`
  feature. The name is branding. This reached a live permagent.ai page as
  "frequency-domain recall" before being corrected on 2026-08-17, which is why
  it is written down here.
- Recognition's own benchmark table must keep its **adversarial-paraphrase row**
  (PAWS: 0.4875 peak-pair, 0.4917 MinHash-128, 0.4853 BGE-small — every system
  at chance) and the row where **plain MinHash-128 beats us** on lexical
  re-encounter (0.9988 vs 0.9946). Cutting either turns a published trade-off
  surface into a cherry-pick.
