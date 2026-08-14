## Verdict per claim

C1: PARTIAL — FTS ordering is tie-broken, but ordinary recall uses `Utc::now()` and wing-tier retrieval lacks a stable secondary order, so outputs are not universally byte-reproducible (`crates/spectral-graph/src/brain.rs:1896`).

C2: MET — Cascade recall structurally assigns `total_recognition_token_cost: 0`, while recognition performs local fingerprint extraction and store lookups (`crates/spectral-graph/src/brain.rs:2382`).

C3: NOT MET — One `Brain` handle opens separate `graph.sqlite`, `memory.db`, and `recognition.db` files rather than one SQLite file (`crates/spectral-graph/src/brain.rs:995`).

C4: PARTIAL — Direct FTS recall uses FTS5 BM25, but the public hybrid/cascade paths also use fingerprint and wing tiers rather than being simply “FTS5 + BM25” (`crates/spectral-ingest/src/sqlite_store.rs:2160`).

C5: MET — Recognition extracts landmark pairs and winnowed k-grams, scores exact matched evidence, and returns Recognized/Familiar/Novel verdicts (`crates/spectral-recognition/src/lib.rs:65`).

C6: MET — Graph recall traverses exactly two hops, and asserted triples are checked against ontology predicate domain/range constraints (`crates/spectral-graph/src/brain.rs:2463`).

C7: MET — Memories carry episode membership, can be listed chronologically by episode, and support explicit valid-time recall (`crates/spectral-ingest/src/sqlite_store.rs:2946`).

C8: MET — Cascade recall reinforces returned memories, while the Archivist transaction decays stale memories and boosts recently reinforced ones (`crates/spectral-graph/src/cascade_layers.rs:491`).

C9: PARTIAL — Federation performs read-time fan-out with provenance and visibility filtering, but filtering occurs only after each child has already truncated an unscoped recall (`crates/spectral-graph/src/federation.rs:384`).

C10: PARTIAL — Default RRF, per-child caps, content deduplication, finite-score sanitation, and deterministic tie-breaking resist score inflation and bounded flooding, but member identities are unauthenticated and Sybil corroboration remains (`crates/spectral-graph/src/federation.rs:98`).

C11: CANNOT VERIFY — In-scope code computes mean session recall from per-row counters, but contains no complete LongMemEval-S artifact establishing the claimed 98.6% (`crates/spectral-bench-accuracy/src/oracle.rs:559`).

C12: CANNOT VERIFY — The code references the published 81.5% result but provides no in-scope 492-row judged artifact from which 401/492 can be independently recomputed (`crates/spectral/src/policy.rs:455`).

C13: MET — Library retrieval and graph-ranking paths are local computations, with the only optional summarizer explicitly outside read-time recall (`crates/spectral-graph/src/brain.rs:2813`).

C14: MET — Every crate inherits the workspace’s `Apache-2.0` manifest declaration, consistent with the Apache License 2.0 text (`Cargo.toml:21`).

## Findings (max 15, ranked most severe first)

### X-01 Federation retrieves and mutates outside the requested visibility scope

- Severity: P1 ship-blocker for this client
- Location: `crates/spectral-graph/src/federation.rs:384`
- Evidence: `child.brain.recall_cascade(query, context, config)` performs maximally permissive recall and optional reinforcement; visibility filtering occurs later at line 395, after ranking, truncation, and write-back.
- Repro or proof: Add more than `config.k` high-ranked Private hits plus a lower-ranked Public hit to a writable child, then call `fan_out_recall(..., Visibility::Public)`; the Public hit can be absent while filtered Private hits are reinforced.
- Proposed fix: Call `recall_cascade_scoped(..., visibility)` for each child and require or enforce read-only child handles for federation.
- Confidence: high

### X-02 The advertised single-SQLite-file storage model is not implemented

- Severity: P1 ship-blocker for this client
- Location: `crates/spectral-graph/src/brain.rs:995`
- Evidence: `Brain::open` separately opens `graph.sqlite`, the configurable/default `memory.db`, and a `recognition.db` sidecar at line 1086.
- Repro or proof: Open a new writable brain and inspect its data directory; three SQLite database paths are constructed by the code.
- Proposed fix: Move graph and recognition tables into `memory.db` under one shared storage/transaction layer, or revise the client promise to say one handle over multiple local SQLite files.
- Confidence: high

### X-03 Recognition enrollment is not atomic with memory ingestion

- Severity: P1 ship-blocker for this client
- Location: `crates/spectral-graph/src/brain.rs:1836`
- Evidence: The primary memory is committed before sidecar enrollment, and enrollment failure is explicitly treated as non-fatal, leaving recallable content absent from recognition.
- Repro or proof: Make `recognition.db` unwritable after opening, call `remember`, then compare successful FTS recall with recognition returning no match.
- Proposed fix: Store recognition indexes in the primary transaction or introduce a durable pending-derivation record that makes incomplete enrollment visible and automatically retryable.
- Confidence: high

### X-04 Wing-tier recall has an untied limited ordering

- Severity: P1 ship-blocker for this client
- Location: `crates/spectral-ingest/src/sqlite_store.rs:2048`
- Evidence: `ORDER BY signal_score DESC` has no unique secondary key before `truncate(max_results)`, so equal-score rows can yield different selected subsets.
- Repro or proof: Insert more than `max_results` memories into one wing with identical signal scores, rebuild or perturb the SQLite query plan, and compare `wing_search` results.
- Proposed fix: Change the query to `ORDER BY signal_score DESC, id ASC` and add a regression test with tied scores exceeding the limit.
- Confidence: high

### X-05 Default recall exposes wall-clock-dependent bytes

- Severity: P1 ship-blocker for this client
- Location: `crates/spectral-graph/src/brain.rs:1899`
- Evidence: `recall()` passes `Utc::now()` into decay, and the resulting time-dependent `signal_score` values are returned to callers.
- Repro or proof: Call `recall()` twice against an unchanged database at different times and serialize `memory_hits`; decayed scores can differ.
- Proposed fix: Make corpus-anchored or caller-supplied time the deterministic default, reserving wall-clock decay for an explicitly named live-recall API.
- Confidence: high

### X-06 Read-only open silently disables recognition when its sidecar is absent

- Severity: P2 fix within 30 days
- Location: `crates/spectral-graph/src/brain.rs:1087`
- Evidence: If `recognition.db` is missing, read-only open substitutes an empty in-memory index, causing existing memories to appear Novel without reporting degraded state.
- Repro or proof: Create a brain containing `memory.db`, remove or omit its recognition sidecar, open read-only, and recognize verbatim stored content.
- Proposed fix: Return a clear missing-derivation error or expose an explicit degraded-health status requiring caller opt-in.
- Confidence: high

### X-07 Async recall feedback silently loses writes

- Severity: P2 fix within 30 days
- Location: `crates/spectral-graph/src/brain.rs:3643`
- Evidence: Both `reinforce_batch` and `log_retrieval_event` results are discarded inside asynchronous write-back.
- Repro or proof: Enable asynchronous write-back, force a database write error, and observe that recall succeeds with no surfaced failure or retry record.
- Proposed fix: Record task failures through tracing plus a durable retry queue or return a write-back receipt that callers can await and inspect.
- Confidence: high

### X-08 Episodic ordering is incomplete for equal timestamps

- Severity: P2 fix within 30 days
- Location: `crates/spectral-ingest/src/sqlite_store.rs:2955`
- Evidence: Episode memories are ordered only by `created_at`, although bulk ingestion can assign identical timestamps.
- Repro or proof: Insert multiple episode memories with the same `created_at`, perturb insertion/query plans, and compare `list_memories_by_episode` ordering.
- Proposed fix: Use `ORDER BY datetime(created_at), id` and add a same-timestamp episode regression test.
- Confidence: high

### X-09 Optional associative spreading suppresses storage failures

- Severity: P2 fix within 30 days
- Location: `crates/spectral-graph/src/spreading.rs:228`
- Evidence: Episode expansion proceeds only under `if let Ok(mems)` and silently converts any database error into missing associated results.
- Repro or proof: Enable spreading, force `list_memories_by_episode` to fail, and observe successful but incomplete recall.
- Proposed fix: Return `Result` from associative spreading and propagate errors, or attach explicit degradation warnings to the recall result.
- Confidence: high

### X-10 Consolidation source caps can select nondeterministic evidence

- Severity: P2 fix within 30 days
- Location: `crates/spectral-ingest/src/sqlite_store.rs:4167`
- Evidence: Consolidation edges are ordered only by second-resolution `consolidated_at`, while `recall_with_provenance` stops after `max_sources_per_hit`.
- Repro or proof: Create more same-timestamp source edges than the requested source cap and compare selected sources after database-plan perturbation.
- Proposed fix: Add `source_key ASC, target_key ASC` tie-breakers to both consolidation-edge queries.
- Confidence: high

### X-11 Federation cannot authenticate distinct contributors

- Severity: P2 fix within 30 days
- Location: `crates/spectral-graph/src/federation.rs:112`
- Evidence: `add_brain` authenticates no identity, so one operator can register multiple brains and manufacture RRF corroboration.
- Repro or proof: Register several attacker-controlled brains containing identical content and observe their contributions summing as cross-origin agreement.
- Proposed fix: Require verified signed brain identities plus an administrator-controlled principal registry, and count corroboration once per authenticated principal.
- Confidence: high

### X-12 Graph conflict ordering is incomplete for equal assertion times

- Severity: P3 nice-to-have
- Location: `crates/spectral-graph/src/graph_store.rs:430`
- Evidence: Multi-valued live facts are ordered by `from_id, predicate, asserted_at` without a unique row identifier, despite consumers receiving ordered object vectors.
- Repro or proof: Insert conflicting triples with identical `asserted_at` values and compare returned object order after database-plan changes.
- Proposed fix: Append `rowid` to the ordering and cover equal-timestamp assertions in a regression test.
- Confidence: high

## Production readiness

DO NOT SHIP — The codebase builds cleanly and substantially implements local zero-model retrieval, recognition, typed graph traversal, temporal memory, adaptive feedback, and bounded federation defenses, but two central client promises are currently false or unsafe: a brain spans three SQLite databases rather than one, and federated visibility is applied only after unscoped ranking and possible reinforcement. The remaining nondeterministic limited queries, non-atomic recognition sidecar, and silent degradation paths also prevent treating deterministic recall and recognition as production-grade guarantees.