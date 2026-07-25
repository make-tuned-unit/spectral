# Constellation fingerprint fan-out cap — 2026-07-25

## Question

`Brain::remember` was suspected of costing more per write as a brain grows.
Is that true, what causes it, and can it be bounded without changing what
retrieval returns?

## Method

- Harness: `spectral-bench-real --bin write_path_cost` (deterministic, $0, no
  LLM). N=800 sequential writes, release build, bucketed per 100 writes so
  growth is visible rather than averaged away.
- Both arms run in the same process against the same corpus. The "uncapped"
  arm calls `set_max_fingerprint_peers(None)` explicitly, so the comparison is
  legacy-vs-default regardless of the shipped default.
- Retrieval verification: `spectral-bench-accuracy oracle` + `oracle-diff`,
  fresh brains in both arms (`reuse_brains` is unsafe after an ingest-affecting
  change), arms selected via `SPECTRAL_MAX_FINGERPRINT_PEERS=0` (legacy) vs
  unset (new default).

## Diagnosis

Per-memory `remember` cost grows linearly with corpus size:

| corpus position | uncapped ms/mem | capped ms/mem |
|---|---:|---:|
| 0–100 | 6.5 | 7.4 |
| 300–400 | 29.6 | 13.2 |
| 700–800 | 79.4 | 20.6 |

`MemoryStore::write` itself is flat (~0.45 ms/mem), so the growth is entirely
derived work. The cause is `generate_fingerprints`, which called
`list_wing_memories` — an **unbounded** read with no LIMIT — and emitted one
constellation edge per existing peer. Because ~73% of memories classify into
the `general` wing, each write pairs against nearly the whole corpus: O(N) per
write, O(N^2) stored rows. A 600-memory corpus produced 129,700 edges.

## Result

`IngestConfig::max_fingerprint_peers` (default `Some(64)`), reachable on an
open brain via `Brain::set_max_fingerprint_peers`, with
`SPECTRAL_MAX_FINGERPRINT_PEERS` as an ablation override (`0` = legacy).

| metric | uncapped | capped | delta |
|---|---:|---:|---:|
| total, 800 writes | 30,974 ms | 11,436 ms | **63% faster** |
| growth first→last bucket | 12.2x | 2.8x | — |
| stored edges (600-memory corpus) | 129,700 | 34,710 | −73% |

The advantage widens with corpus size; 63% is the figure at N=800, not a ceiling.

## Retrieval verification

Paired oracle, candidate vs baseline, **42 questions across 4 routing shapes**:

| shape | n | contexts changed | answer-key delta | token delta |
|---|---:|---:|---:|---:|
| multi-session (held-out) | 25 | 0 | 0 | 0 |
| single-session-user | 9 | 0 | 0 | 0 |
| temporal-reasoning | 4 | 0 | 0 | 0 |
| knowledge-update | 4 | 0 | 0 | 0 |

Every context hash, ordered retrieval output, and token count matched. The
held-out arm also reproduced the published baseline exactly (session recall
100.0%, key recall 48.6%, tok-mean 17,943), confirming the harness.

## Honest limitation

This is **not** proven byte-identical everywhere, and cannot be. `time_delta_bucket`
is part of the fingerprint hash, so bounding fan-out necessarily changes which
(hall, bucket) hashes exist. Driving `fingerprint_search` directly with stored
hashes shows the edge sets genuinely differ.

That divergence is confined to the TACT tier-1 reader, the only consumer of
these edges — a path this repo has separately measured at 0/500 retrieval
effect, and which never fired in any oracle arm above (all queries resolved via
FTS). The default is therefore justified by measurement on the routed paths,
not by an identity argument. Callers depending on tier-1 fingerprint retrieval
should set `SPECTRAL_MAX_FINGERPRINT_PEERS=0` and A/B their own workload.

## Selection key (a rejected first design)

The cap initially selected peers by `signal_score DESC`. A direct
`fingerprint_search` comparison falsified it: 2/2 probes diverged because that
ordering systematically retains the *oldest* peers, while the uncapped edge set
surfaces the most recent. The shipped cap selects most-recent-first, which
matches the temporal locality these edges encode. A regression test asserts the
ordering so the mistake cannot silently return.

## Not fixed

With the cap on, `remember` still grows 2.8x over 800 writes. The remaining
source is recognition enrollment (3.0x on its own): `index_minhash` writes one
inverted-index row per shingle, ~180 rows per memory. An LSH-banding path
already exists in `minhash` and would index far fewer rows, but that trades
against recognition recall and needs its own bench run. Untouched here.
