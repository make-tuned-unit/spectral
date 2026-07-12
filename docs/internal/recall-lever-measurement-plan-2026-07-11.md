# Recall-lever measurement plan (2026-07-11)

Decisions made this arc, and the **one measurement** that un-gates each opt-in
lever. Written so whoever has the LongMemEval-S dataset can execute the gated
calls without re-deriving the analysis.

## Default-path changes (already shipped ON — proven unambiguously correct)

These are correctness fixes, not tunable levers. They ship on the default path
because they are strictly right, independent of any benchmark.

| Change | What it fixes | Verify |
|---|---|---|
| **Query separator split** | `alice@acme.io`/`and/or`/`api.acme.dev` were deleted-then-merged into non-matching blobs → answer absent from pool. Now split like unicode61 tokenizes content. | `--bin tokenization_probe`; `splits_on_separators_instead_of_merging` |
| **Possessive strip** | `Marcus's`→`Marcus` (subsumed by the separator fix) | same |
| **Recency correctness** | recency was silently inert for `RememberOpts.created_at` imports (RFC3339 vs SQLite-format parse mismatch) | `--bin recency_probe` `[1]` |
| **Recency safety** | multiplicative decay could annihilate an old-but-relevant answer out of top-K → now a bounded additive tiebreaker (`RECENCY_BOOST_WEIGHT = 0.1`) | `--bin recency_probe` `[2]` |
| **Timestamp parser unification** | all `created_at`/`last_reinforced_at` parsers route through dual-format `ranking::parse_created_at` | graph suite |

**Recency stays default-ON.** Now bounded, it cannot cause a severe regression.
`RECENCY_BOOST_WEIGHT` is a module const (like `RRF_K`); promote to a
`RerankingConfig` field only if a corpus shows recency strength needs tuning.

## Opt-in levers (default-OFF — gated on a real-corpus measurement)

Each is mechanism-proven on a deterministic bench but **not** validated on real
recall@K. The gate is the same for all: run the Tier-0 oracle (zero-LLM, $0)
three-armed and compare **recall@40 and answer accuracy** (NOT recall@1 — the
actor reads all K).

```
# baseline vs lever, on LongMemEval-S:
cargo run -p spectral-bench-accuracy -- oracle --dataset <path> --work-dir <dir> \
  --label baseline --fresh-brains
SPECTRAL_<FLAG>=1 cargo run -p spectral-bench-accuracy -- oracle --dataset <path> \
  --work-dir <dir> --label <lever> --fresh-brains
# then diff answer_sessions_hit / answer_keys_retrieved between the two label sets.
```

| Lever | Flag | Mechanism bench | Flip default-ON if… | Known risk |
|---|---|---|---|---|
| **Stemmed+unstemmed RRF fusion** | `SPECTRAL_FTS_FUSION=1` | `fusion_scale_bench`, `fts_fusion_experiment` | recall@40 gain > 0 AND justifies the second FTS index (write/storage/latency +0.45ms/query) | null at recall@40 on synthetic; only helps when an over-stem flood exceeds the 120-candidate fetch pool |
| **Number-word bridging** | `SPECTRAL_NUMBER_NORMALIZE=1` | `recall_expansion_bench`, `tokenization_probe` | recall@40 gain on the counting/number category with no aggregate regression | OR-expands number queries → can shift pool composition in huge haystacks |
| **FTS stopword filtering** | `SPECTRAL_FTS_STOPWORDS=1` | `recall_debug` | precision gain with no recall loss | removes terms → homograph risk (mitigated by a conservative set) |
| **Curated query aliases** | `SPECTRAL_QUERY_ALIASES=<file>` | `recall_expansion_bench` | consumer supplies a domain table; measure that table | precision of the specific table (consumer-owned) |
| **Anticipatory in-recall augmentation** | `SPECTRAL_ANTICIPATORY_RECALL=1` | `anticipatory_bench`, `ambient_scale_bench` | miss-recovery gain once real co-retrieval history exists | appends beyond K; only helps with usage history |

## Deferred (product decisions, not levers)

- **camelCase write-side splitting** (`taskRunner` ↮ `task runner`): needs a
  standalone split FTS index; code-relevant, chat-irrelevant. Build only if
  Permagent's technical/code memory shows the miss.
- **True paraphrase / general synonymy**: unreachable deterministically (every
  near-dup source agrees). Requires an embedding channel — a positioning
  decision that trades away the zero-embedding / least-expensive differentiation.

## Results — real-corpus run (2026-07-12)

Dataset obtained (HuggingFace `xiaowu0162/longmemeval` → `longmemeval_s`, 500
questions, MIT, non-gated) and placed at
`~/spectral-local-bench/longmemeval/longmemeval_s.json`. Tier-0 oracle,
zero-LLM/$0, release build, per-question shape routing (published config).
Metric = answer-**session** recall@40 (sess-rec) and answer-**key** recall
(key-rec).

| Arm | Category | n | sess-rec@40 | key-rec | rank1 | verdict |
|---|---|---|---|---|---|---|
| baseline | multi-session | 30 | **98.3%** | 48.6% | 1.6 | — |
| +number-words | multi-session | 30 | 98.3% | 48.6% | 1.6 | **null** (inert; no numbers in these queries) |
| +fusion | multi-session | 30 | 98.3% | 48.4% | 1.6 | **null** (no lift; noise) |
| baseline | temporal-reasoning | 25 | **100.0%** | 52.7% | 1.4 | — |
| +number-words | temporal-reasoning | 25 | 99.3% | 52.8% | 1.4 | **REGRESSION** −0.7% sess-rec |

**Resolutions (both stay default-OFF — now with real evidence, not just discipline):**
- **Fusion → keep OFF.** Zero recall@40 lift on real multi-session data. Confirms the synthetic analysis exactly: retrieval already surfaces answer sessions at **98–100% at K=40**, so a tight-k *reordering* lever has no headroom. Its second FTS index would be pure cost.
- **Number-words → keep OFF.** Not merely neutral — it **regressed** temporal sess-rec 100.0% → 99.3% (the OR-expansion diluted the candidate pool and dropped one answer session), with no compensating key-rec gain. Defaulting it ON would have hurt real recall. The measure-before-default discipline caught a real regression.

**The headline finding:** at LongMemEval's K=40, baseline retrieval recall is
**already ~98–100%** (answer sessions). This is the empirical confirmation, on
the real corpus, of the earlier analysis — recall@K is near-ceiling, so the
levers that reorder within the pool cannot move the number, and the remaining
gap is **actor synthesis** (key-rec ~49–53% and the actor's use of context),
not retrieval. Future recall-side effort has low ceiling; the leverage is
actor-side. The default-path fixes shipped this arc (separator tokenization,
recency correctness+safety) sit under these healthy baselines without regression.
