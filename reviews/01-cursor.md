## Verdict per claim
- **C1 PARTIAL** — Anchors/`recall_at` exist, but default recall/cascade recency uses `Utc::now()` and default cascade `write_back` mutates scores, so identical brain+query can diverge over time (`crates/spectral-cascade/src/context.rs:26`, `crates/spectral-graph/src/cascade_layers.rs:294`).
- **C2 MET** — Recognition has no default network/ML stack and cascade hardcodes `total_recognition_token_cost: 0` (`crates/spectral-recognition/src/lib.rs:28`, `crates/spectral-graph/src/brain.rs:2386`).
- **C3 PARTIAL** — One embedded `Brain` handle, but open creates `graph.sqlite` + `memory.db` + `recognition.db` (plus ontology/identity), not one SQLite file (`crates/spectral-graph/src/brain.rs:995`, `crates/spectral-graph/src/brain.rs:1086`).
- **C4 PARTIAL** — FTS5+BM25 is real (`ORDER BY bm25(...), m.id`), but primary cascade/TACT recall also uses fingerprint/wing tiers and additive re-ranking (`crates/spectral-ingest/src/sqlite_store.rs:2160`, `crates/spectral-graph/src/brain.rs:2315`).
- **C5 MET** — Landmarks, pair fingerprints, winnowed k-grams, scored verdict+evidence, near-duplicate/verbatim scope (`crates/spectral-recognition/src/lib.rs:8`, `crates/spectral-recognition/src/lib.rs:221`).
- **C6 MET** — Ontology-validated `assert` and graph `neighborhood(..., 2)` / `recall_graph` 2-hop (`crates/spectral-graph/src/brain.rs:1251`, `crates/spectral-graph/src/brain.rs:2463`).
- **C7 MET** — Episode store/list APIs and temporal question routing exist (`crates/spectral-graph/src/brain.rs:4184`, `crates/spectral/src/policy.rs:177`).
- **C8 PARTIAL** — Used memories strengthen via reinforce/auto-reinforce; unused decay is soft read-time scoring / opt-in Archivist, and turn ledger explicitly refuses auto-decay (`crates/spectral-graph/src/brain.rs:3567`, `crates/spectral/src/turn.rs:507`).
- **C9 MET** — Read-time fan-out merges provenance-labeled hits with visibility filtering (`crates/spectral-graph/src/federation.rs:353`, `crates/spectral-graph/src/federation.rs:392`).
- **C10 PARTIAL** — Default RRF+`per_child_cap` defends score-inflation flooding; sybil/auth and `RawScore` remain residual/vulnerable as coded (`crates/spectral-graph/src/federation.rs:64`, `crates/spectral-graph/src/federation.rs:145`).
- **C11 CANNOT VERIFY** — LongMemEval-S not present; no `98.6` artifact under `crates/`/`scripts/` to consistency-check (`scripts/` has no match; closest in-crate citation is unrelated).
- **C12 CANNOT VERIFY** — Same host/dataset gap; only a prose citation of `81.5%` appears (`crates/spectral/src/policy.rs:455`), no `401/492` artifact in scope.
- **C13 MET** — Library recall/recognize/graph/episode/adaptive/federated paths do not invoke `LlmClient`; TACT is regex-only (`crates/spectral-tact/src/lib.rs:17`, `crates/spectral-graph/src/brain.rs:2131`).
- **C14 MET** — Root `LICENSE` is Apache-2.0 and workspace/crate manifests inherit `license = "Apache-2.0"` (`LICENSE:1`, `Cargo.toml:21`).

## Findings
### X-01 Default recall mutates ranking state
- Severity: P1
- Location: `crates/spectral-graph/src/cascade_layers.rs:294`
- Evidence:
```294:509:crates/spectral-graph/src/cascade_layers.rs
            write_back: true,
            ...
    if config.write_back && !brain.is_read_only() {
        const AUTO_REINFORCE_STRENGTH: f64 = 0.01;
        ...
        brain.write_back(keys, event, AUTO_REINFORCE_STRENGTH);
```
- Repro or proof: `cargo test -p spectral-graph cascade_auto_reinforces_returned_memories -- --exact`
- Proposed fix: Default `write_back: false`; require `Brain::turn` / explicit reinforce for strengthening.
- Confidence: high

### X-02 Federation fan-out mutates writable children
- Severity: P1
- Location: `crates/spectral-graph/src/federation.rs:384`
- Evidence:
```384:391:crates/spectral-graph/src/federation.rs
            let result = match child.brain.recall_cascade(query, context, config) {
                Ok(result) => result,
                Err(e) => {
                    failed.push((origin, e.to_string()));
                    continue;
                }
            };
            recognition_token_cost += result.total_recognition_token_cost;
```
- Repro or proof: `cargo test -p spectral-graph read_only_child_is_not_mutated_by_fan_out -- --exact` (writable children still take default cascade write-back)
- Proposed fix: Call `recall_cascade_scoped` with a forced no-write config, or require/enforce `read_only` children in `add_brain`.
- Confidence: high

### X-03 Wall-clock default breaks byte-stable recall ordering
- Severity: P1
- Location: `crates/spectral-cascade/src/context.rs:26`
- Evidence:
```26:29:crates/spectral-cascade/src/context.rs
    /// **Defaults to `Utc::now()` in [`empty()`](Self::empty).** This is
    /// correct for live queries but silently wrong for historical replay —
    /// use [`with_now()`](Self::with_now) to anchor recency to the query
    /// date when scoring historical or time-travel data.
```
- Repro or proof: `cargo test -p spectral --test deterministic_anchor -- --exact`
- Proposed fix: Make reproducible/corpus-anchored time the default for library recall APIs; keep wall-clock only behind an explicit live mode.
- Confidence: high

### X-04 “One SQLite file” claim is false in code
- Severity: P1
- Location: `crates/spectral-graph/src/brain.rs:995`
- Evidence:
```995:1086:crates/spectral-graph/src/brain.rs
        let graph_path = config.data_dir.join("graph.sqlite");
        ...
            .unwrap_or_else(|| config.data_dir.join("memory.db"));
        ...
        let recognition_db = config.data_dir.join("recognition.db");
```
- Repro or proof: static reasoning only
- Proposed fix: Change client copy to “one brain directory / embedded multi-DB SQLite store”, or physically consolidate stores.
- Confidence: high

### X-05 Untiebroken wing `ORDER BY signal_score` still feeds TACT
- Severity: P1
- Location: `crates/spectral-ingest/src/sqlite_store.rs:2048`
- Evidence:
```2048:2050:crates/spectral-ingest/src/sqlite_store.rs
                            "SELECT {MEMORY_COLUMNS} FROM memories WHERE wing = ?1
                             ORDER BY signal_score DESC"
```
- Repro or proof: static reasoning only (SQLite tie order undefined; later stable term-boost preserves it)
- Proposed fix: Add `, id` (same R17/R18 pattern) here and at `list_wing_memories` (`:1860`).
- Confidence: high

### X-06 Federation visibility is post-filter after Private top-k
- Severity: P1
- Location: `crates/spectral-graph/src/federation.rs:392`
- Evidence:
```392:396:crates/spectral-graph/src/federation.rs
            let visible = result
                .merged_hits
                .into_iter()
                .filter(|hit| crate::brain::str_to_vis(&hit.visibility).allows(visibility))
                .collect::<Vec<_>>();
```
- Repro or proof: static reasoning only (Private rows can consume `k` before Team/Public survivors)
- Proposed fix: Fan out via `recall_cascade_scoped(..., visibility)` so truncation is over admissible hits.
- Confidence: med

### X-07 “Unused decay” is not the library adaptive loop
- Severity: P2
- Location: `crates/spectral/src/turn.rs:507`
- Evidence:
```507:512:crates/spectral/src/turn.rs
    /// This is deliberately **evidence, not policy**. A memory can go unused
    ...
    /// Wiring this to automatic decay or forgetting
    /// would let the write path erase evidence of a read-path defect, and must
    /// not be done without separate validation.
```
- Repro or proof: static reasoning only
- Proposed fix: Narrow the claim to “used memories strengthen; unused tracked; decay via Archivist/opt-in”, or ship a measured unused-decay policy behind a flag.
- Confidence: high

### X-08 Additive recency makes clock shifts reorder top-k
- Severity: P2
- Location: `crates/spectral-graph/src/ranking.rs:707`
- Evidence:
```707:729:crates/spectral-graph/src/ranking.rs
    // Recency: ADDITIVE bounded boost for fresh content ...
            let freshness = 0.5_f64.powf(age_days / config.recency_half_life_days);
            scores[i] += RECENCY_BOOST_WEIGHT * freshness;
```
- Repro or proof: `cargo test -p spectral --test deterministic_anchor recall_path_is_stable_when_anchored_and_clock_dependent_when_not -- --exact`
- Proposed fix: Require explicit `now` for any path that applies this channel, or disable recency unless anchored.
- Confidence: high

### X-09 Episode mate order lacks id tiebreak
- Severity: P2
- Location: `crates/spectral-ingest/src/sqlite_store.rs:2955`
- Evidence:
```2955:2956:crates/spectral-ingest/src/sqlite_store.rs
                "SELECT {MEMORY_COLUMNS} FROM memories WHERE episode_id = ?1 ORDER BY created_at"
```
- Repro or proof: static reasoning only
- Proposed fix: `ORDER BY created_at, id` (spreading consumes this order).
- Confidence: med

### X-10 Graph document scan iterates a `HashSet`
- Severity: P2
- Location: `crates/spectral-graph/src/graph_store.rs:627`
- Evidence:
```627:628:crates/spectral-graph/src/graph_store.rs
        'doc_scan: for entity_id in &visited {
            for doc in find_mentioning_documents(&conn, entity_id)? {
```
- Repro or proof: static reasoning only
- Proposed fix: Iterate a sorted `Vec` of visited ids before document collection.
- Confidence: med

### X-11 Poisoning resistance is merge-policy only
- Severity: P2
- Location: `crates/spectral-graph/src/federation.rs:145`
- Evidence:
```145:149:crates/spectral-graph/src/federation.rs
    pub fn raw_scores() -> Self {
        Self {
            fusion: FusionMethod::RawScore,
            per_child_cap: None,
        }
```
- Repro or proof: static reasoning only
- Proposed fix: Keep RRF+cap mandatory for public API; require signed `verify_hit` before merge for untrusted members; document “poisoning-resistant” as score-flood resistant only.
- Confidence: high

### X-12 Public `Brain::open` docs misstate storage layout
- Severity: P3
- Location: `crates/spectral/src/lib.rs:247`
- Evidence:
```247:249:crates/spectral/src/lib.rs
    /// Uses `<path>/memory.db` for graph, memories, and full-text indexes,
    /// plus `<path>/recognition.db` for the recognition sidecar,
    /// `<path>/ontology.toml` if present (empty ontology otherwise),
```
- Repro or proof: static reasoning only
- Proposed fix: Document `graph.sqlite` + `memory.db` + `recognition.db` to match `Brain::open` implementation.
- Confidence: high

### X-13 `quinn-proto` enters lockfile via `reqwest` (http-llm / neural paths)
- Severity: P3
- Location: `crates/spectral/Cargo.toml:31`
- Evidence:
```31:32:crates/spectral/Cargo.toml
default = ["http-llm"]
http-llm = ["dep:reqwest"]
```
- Repro or proof: `rg -n 'name = "quinn"|\"quinn\"' Cargo.lock` (reqwest lists `quinn`; not activated without reqwest http3 feature)
- Proposed fix: Keep `default-features = false` (already); consider `default = []` if audit noise on optional graphs must be zero.
- Confidence: med

### X-14 Recall≠FTS+BM25 alone undermines C4 marketing equality
- Severity: P3
- Location: `crates/spectral-graph/src/brain.rs:2315`
- Evidence:
```2315:2320:crates/spectral-graph/src/brain.rs
    /// Run the integrated retrieval pipeline with ambient boost.
    ///
    /// TACT tiered search (fingerprint → wing → FTS fallback) supplemented
    /// by raw FTS, then unified re-ranking: signal blend, ambient boost,
```
- Repro or proof: static reasoning only
- Proposed fix: State “FTS5+BM25 plus deterministic local re-rank/TACT tiers”, reserve pure BM25 for `fts_search_direct`/`recall_topk_fts` without extras.
- Confidence: high

## Production readiness
**SHIP WITH FIXES** — Workspace builds/tests/clippy are clean and the core local, embedding-free engines (FTS/BM25, recognition fingerprints, ontology graph, federation RRF) are real and largely coherent, but production copy currently overclaims (“one SQLite file”, “Recall = FTS5 + BM25”, unused-memory decay, broad “poisoning-resistant”), and default cascade/federation paths are not read-stable: they wall-clock-rank and auto-reinforce unless callers opt into anchors/`write_back: false`/`read_only`. Fix the fan-out mutation/visibility chokepoint and remaining untiebroken wing ORDER BY before treating federated or byte-reproducible recall as client-ready.
