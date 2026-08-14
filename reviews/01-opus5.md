## Verdict per claim

**C1 — Recall is deterministic / byte-reproducible for a given brain state: PARTIAL.** Default recall anchors recency decay to the wall clock (`crates/spectral-graph/src/brain.rs:1904` → `crates/spectral-cascade/src/context.rs:55`) and every recall mutates the brain it just read via auto-reinforce + event log (`crates/spectral-graph/src/cascade_layers.rs:491-510`), so "same brain state" does not survive being queried; the repo pins this itself with an `assert_ne!` in `crates/spectral/tests/deterministic_anchor.rs:108-131`.

**C2 — Zero model calls on recall + recognition; `recognition_token_cost == 0` structural: MET.** `llm_client` has exactly one call site in the entire library, `Brain::ingest_text` (`crates/spectral-graph/src/brain.rs:3879`), which is a write path — no recall or recognition path can reach a model; the field itself is a literal (`crates/spectral-graph/src/brain.rs:2386`), so the guarantee rests on the dependency graph, not on the counter.

**C3 — One Brain handle over one SQLite file; embedded library, no service: NOT MET (embedded part MET).** One brain opens three independent SQLite databases — `graph.sqlite`, `memory.db`, `recognition.db` (`crates/spectral-graph/src/brain.rs:995`, `:1004`, `:1086`) — contradicting `README.md:146` ("a single SQLite file you own"); the "no service" half is correct (no network transport anywhere in the library).

**C4 — Recall = FTS5 + BM25: MET.** `CREATE VIRTUAL TABLE memories_fts USING fts5(...)` at `crates/spectral-ingest/src/sqlite_store.rs:495`, ranked by `ORDER BY bm25(memories_fts, 1.0, 1.0, 0.5), m.id` at `crates/spectral-ingest/src/sqlite_store.rs:2160`.

**C5 — Recognition = landmark fingerprinting + winnowed k-grams + scoring with exact features: MET.** Shazam-style peak-pair landmarks at `crates/spectral-recognition/src/extract.rs:315-334`, Schleimer winnowing at `crates/spectral-recognition/src/extract.rs:339-363`, and the three-variant `Verdict` plus per-feature `Evidence` at `crates/spectral-recognition/src/lib.rs:100-120`.

**C6 — Typed knowledge graph, 2-hop, ontology-validated: MET.** Domain/range validation at `crates/spectral-graph/src/ontology.rs:298-320`, and `recall_graph` runs `neighborhood(seed, 2)` at `crates/spectral-graph/src/brain.rs:2463`.

**C7 — Episodic/temporal recall exists: MET.** `episodes` table at `crates/spectral-ingest/src/sqlite_store.rs:659` with `list_episodes` at `:2901`, plus relative-date resolution at `crates/spectral/src/temporal.rs:150`.

**C8 — Adaptive feedback loop: used strengthen, unused decay: MET.** `reinforce` bumps `signal_score` and resets `last_reinforced_at` (`crates/spectral-graph/src/brain.rs:3571-3592`); decay is applied at read time from that timestamp (`crates/spectral-graph/src/brain.rs:4741-4761`).

**C9 — Read-time federation across brains: provenance-ranked, visibility-scoped: PARTIAL.** In-process fan-out and RRF provenance ranking are real (`crates/spectral-graph/src/federation.rs:365`, `:511-519`), but the fan-out calls the explicitly-unscoped `recall_cascade` (`crates/spectral-graph/src/federation.rs:384`) against the warning at `crates/spectral-graph/src/brain.rs:2335-2341`, so scoping is a coordinator-side post-filter (`:392-396`) applied after each peer already truncated.

**C10 — Federation is "poisoning-resistant": PARTIAL.** The code genuinely defends score-inflation flooding (RRF over ranks, `crates/spectral-graph/src/federation.rs:511-519`; per-origin content dedup `:496-503`; per-child cap `:60` + `:427-429`), but it does not defend Sybil corroboration — self-acknowledged at `crates/spectral-graph/src/federation.rs:112-118` — and the sync layer accepts unauthenticated authorship and retractions (`crates/spectral-ingest/src/federation_sync.rs:296-299`, `:432-455`).

**C11 — 98.6% session-recall on LongMemEval-S: CANNOT VERIFY.** Dataset absent from this host (`datasets/` does not exist); internally the repo labels this the in-sample number against a 92.9% held-out LoCoMo figure at `README.md:166`.

**C12 — 81.5% end-to-end (401/492): CANNOT VERIFY.** Arithmetic is internally consistent (`401 / (500 − 8)` at `docs/RESULTS.md:26`) and self-labelled in-sample at `docs/RESULTS.md:82`, but the dataset is not present to re-run.

**C13 — $0 per query on all six library query paths: MET.** No query path can reach the single `llm_client` call site (`crates/spectral-graph/src/brain.rs:3879`); note the benchmarked 81.5% config adds an optional paid pre-retrieval expansion call per `README.md:139-141`, so C12 and C13 do not describe the same configuration.

**C14 — Apache-2.0 licensing coherent: PARTIAL.** `Cargo.toml:16` sets `license = "Apache-2.0"` and all eleven manifests inherit it (`crates/*/Cargo.toml:5`), and `LICENSE:1-3` is Apache 2.0 — but `NOTICE:2-4` names a placeholder copyright holder and organization.

## Findings (max 15, ranked most severe first)

**OP-01 — Federated fan-out queries every peer with no visibility boundary**
- Severity: P1
- Location: `crates/spectral-graph/src/federation.rs:384`
- Evidence: `let result = match child.brain.recall_cascade(query, context, config) {` — `recall_cascade` funnels to `recall_cascade_scoped(..., Visibility::Private)` (`crates/spectral-graph/src/brain.rs:2348`), whose own doc at `:2335-2341` says "this entry point applies no visibility boundary — it returns every hit in the brain".
- Repro or proof: static trace. Peer filters under `Private` (a no-op) at `cascade_layers.rs:413`, then `results.truncate(config.k)` at `cascade_layers.rs:482`; the coordinator filters only afterwards at `federation.rs:392-396`. A peer whose top-k is entirely Private contributes zero hits while reporting `Ok`.
- Proposed fix: call `child.brain.recall_cascade_scoped(query, context, config, visibility)` at `federation.rs:384`, passing the fan-out's `visibility` through; keep the coordinator filter as defence-in-depth.
- Confidence: high

**OP-02 — Federated read mutates the peer it reads: score inflation plus query-metadata leak into the peer's store**
- Severity: P1
- Location: `crates/spectral-graph/src/cascade_layers.rs:491`
- Evidence: `if config.write_back && !brain.is_read_only() {` — `write_back` defaults to `true` (`cascade_layers.rs:294`), and the event written at `:498-509` carries `query_hash` and `session_id` from the *querying* coordinator.
- Repro or proof: `FederationCoordinator::add_brain` (`federation.rs:280`) takes any `Brain` by value and never checks `is_read_only()`; `poison_bench.rs:107` opens all three members `read_only: false`, so the in-tree consumer exercises exactly this path.
- Proposed fix: have `add_brain`/`add_brain_weighted` reject a non-read-only child, or force `config.write_back = false` inside `fan_out_recall_with_policy` before dispatch.
- Confidence: high

**OP-03 — Sync import and tombstones are wholly unauthenticated: forgeable authorship and a remote hard-delete primitive**
- Severity: P1
- Location: `crates/spectral-ingest/src/federation_sync.rs:432-455`, `:296-299`
- Evidence: `fn apply_tombstone_tx(tx: &rusqlite::Connection, wing_id: &str, target_hash: &str)` — no author parameter at all, and the body `DELETE FROM memories WHERE id = ?1`. The import INSERT column list at `:296-299` has no `signature` column while storing an off-the-wire `author_id` as `source_brain_id`.
- Repro or proof: static reasoning only. `provenance()` (`:516-519`) then reports the forged author as `Origin::Shared { author_id }`; `object_hash` (`:68-76`) covers `author_id` but a hash authenticates nothing.
- Proposed fix: add a `signature` field to `MemoryObject` and `Tombstone`, verify it against the claimed `author_id` before the INSERT/DELETE, and reject objects and retractions that fail. Until then, document the layer as trusted-transport-only in the crate docs.
- Confidence: high

**OP-04 — No `busy_timeout` on any SQLite connection: concurrent writers fail immediately with SQLITE_BUSY**
- Severity: P1
- Location: `crates/spectral-ingest/src/sqlite_store.rs:277-282`, `crates/spectral-graph/src/graph_store.rs:116`, `crates/spectral-recognition/src/store.rs:187`
- Evidence: the PRAGMA batch at `sqlite_store.rs:277-282` sets `journal_mode`, `synchronous`, `temp_store`, `mmap_size` — and no `busy_timeout`; grep for `busy_timeout|busy_handler` across all three store modules returns zero hits.
- Repro or proof: static reasoning only. SQLite's default busy timeout is 0, so a second writer (a second `Brain` handle, or the `spectral-archivist` binary which opens `memory.db` directly per `crates/spectral-archivist/src/main.rs:41`) returns `SQLITE_BUSY` on first contention rather than waiting.
- Proposed fix: `conn.busy_timeout(Duration::from_secs(5))` on every connection opened in all three stores, including the reader pool at `sqlite_store.rs:214`.
- Confidence: high

**OP-05 — `graph.sqlite` runs in rollback-journal mode while the other two databases run WAL**
- Severity: P1
- Location: `crates/spectral-graph/src/graph_store.rs:115-121`
- Evidence: `pub fn open(path: &Path) -> Result<Self, Error> { let conn = Connection::open(path)?; create_schema(&conn)?;` — no PRAGMA batch at all; grep for `journal_mode` across `crates/spectral-graph/src/` returns zero hits.
- Repro or proof: static reasoning only. Consequences: every graph write takes an EXCLUSIVE lock blocking all graph readers (unlike `memory.db`), and the two `PRAGMA wal_checkpoint(TRUNCATE)` calls in `graph_store.rs:158` and `:162` that the deletion-guarantee doc at `crates/spectral-graph/src/brain.rs:2769-2771` relies on are silent no-ops on a non-WAL file.
- Proposed fix: apply the same PRAGMA batch as `sqlite_store.rs:277-282` inside `GraphStore::open`, then re-verify the D4 byte-scan claim in `docs/DELETION_GUARANTEES.md`.
- Confidence: high

**OP-06 — `remember()` spans two databases with no atomicity; partial failures return `Ok`**
- Severity: P1
- Location: `crates/spectral-graph/src/brain.rs:1838-1845`
- Evidence: `if let Ok(mut engine) = self.recognition.lock() { if let Err(e) = engine.enroll(&result.memory.id, content) { ... derivation_warnings.push(...) } }` — the memory row is already committed to `memory.db` at `:1732`, and enrollment into `recognition.db` is a separate uncoordinated transaction whose failure only appends a warning string.
- Repro or proof: static reasoning only. A crash between `brain.rs:1732` and `:1839` leaves a memory that is permanently invisible to `recognize()`; the same applies to the density write (`:1748`) and the signature write (`:1778`). `RememberResult` (`:1870`) carries no field telling the caller the write was partial.
- Proposed fix: surface `derivation_warnings` on `RememberResult` as a first-class non-empty check, and extend `repair_derivations` (`brain.rs:3216`) to re-enroll memories missing from the recognition index so the torn state is recoverable.
- Confidence: high

**OP-07 — Memory signature does not cover `key` or `id`, permitting verbatim re-key substitution**
- Severity: P1
- Location: `crates/spectral-core/src/identity.rs:223-244`
- Evidence: `buf.extend_from_slice(source_brain_id.as_bytes()); for field in [content_hash, created_at, visibility] {` — the signed payload is domain ‖ brain_id ‖ content_hash ‖ created_at ‖ visibility, and nothing else.
- Repro or proof: static reasoning only. `verify_hit` (`crates/spectral-graph/src/brain.rs:1218-1242`) recomputes the hash from `hit.content` and never reads `hit.key`, so a peer can re-serve a genuinely-signed memory under any key — e.g. as the answer to a different question — and verification still returns `true`. Compounding this, `verify_hit` is called by nothing outside tests: the "contributor grant set" its doc at `:1215-1217` depends on does not exist in the codebase.
- Proposed fix: include the length-prefixed `key` in `memory_signing_payload` (bump `MEMORY_SIG_DOMAIN` to `-v2` and accept v1 for legacy rows), and wire `verify_hit` into `merge_and_rank` behind an explicit key registry.
- Confidence: high

**OP-08 — Visibility is a Rust post-filter applied after the SQL `LIMIT`, so scoped callers get silently under-filled results**
- Severity: P2
- Location: `crates/spectral-graph/src/brain.rs:2160`, `crates/spectral-graph/src/cascade_layers.rs:413`, `crates/spectral-graph/src/brain.rs:2481-2494`
- Evidence: `candidates.retain(|m| str_to_vis(&m.visibility).allows(visibility));` at `brain.rs:2160` runs *after* `fts_search(&words, fetch_k)` at `:2155` has already applied `LIMIT ?2` in SQL. The graph path is worse: `recall_graph` filters at `:2481-2494` only after a full 2-hop BFS has traversed *through* Private edges.
- Repro or proof: static reasoning only. A `Public` caller on a brain that is 90% Private gets roughly a tenth of `k`; and in the graph path, hop-2 entities reachable only via a Private edge are still returned, disclosing the existence of the private connection.
- Proposed fix: push the visibility predicate into the SQL `WHERE` clause of `fts_search` and `find_triples_directed`, so the `LIMIT` and the BFS frontier are both computed over admissible rows only.
- Confidence: high

**OP-09 — Recognition evidence ties are resolved by `HashMap` iteration order, breaking the audit trail's reproducibility**
- Severity: P2
- Location: `crates/spectral-recognition/src/score.rs:289-296`
- Evidence: `let mut evidence: Vec<Evidence> = acc.into_values().flat_map(|a| a.evidence).collect();` then a comparator tiebreaking on `.then_with(|| a.feature.cmp(&b.feature))` — on `feature` but not on `memory_id`.
- Repro or proof: static reasoning only. Two memories sharing a pair hash produce `Evidence` rows with identical `feature` and identical `weight` (doc frequency is per-hash, `store.rs:71`); the comparator returns `Equal`, the stable sort preserves randomly-seeded `HashMap` order, and `evidence.truncate(config.max_evidence)` at `:296` then drops a nondeterministic subset. The guard test `c1_insertion_order_independence` (`tests/invariants.rs:107-136`) passes only because its 6-doc fixture has no shared pair hash. Contrast the correct `traces` sort at `score.rs:201-206`, which does tiebreak on `memory_id`.
- Proposed fix: append `.then_with(|| a.memory_id.cmp(&b.memory_id))` at `score.rs:294`, and extend the invariants fixture with two documents sharing a pair hash.
- Confidence: high

**OP-10 — Recall auto-reinforces every hit, so repeating a query changes the ranking it was measuring**
- Severity: P2
- Location: `crates/spectral-graph/src/brain.rs:3634-3654`
- Evidence: `let _ = store.reinforce_batch(&keys, strength).await; let _ = store.log_retrieval_event(&event).await;` — every error is discarded, and this fires on the default recall path from `cascade_layers.rs:510`.
- Repro or proof: static reasoning only. `AUTO_REINFORCE_STRENGTH = 0.01` (`cascade_layers.rs:496`) bumps `signal_score` and resets `last_reinforced_at`, which feeds `decayed_signal_score` (`brain.rs:4741-4761`) and hence the next recall's ranking. Combined with the `Utc::now()` anchor (`brain.rs:1904`), the C1 reproducibility claim only holds under `read_only` opens or the `turn()` path (`crates/spectral/src/lib.rs:46-77`).
- Proposed fix: make `write_back: false` the default for the public `recall*` methods and steer callers to `turn()`/`record_turn_outcome`; at minimum surface the swallowed errors through `tracing::warn!` so a full disk is not invisible.
- Confidence: high

**OP-11 — `FanoutResult::failed` is the only signal that peers dropped out, and nothing forces the caller to look**
- Severity: P2
- Location: `crates/spectral-graph/src/federation.rs:384-390`
- Evidence: `Err(e) => { failed.push((origin, e.to_string())); continue; }` — no `tracing::warn!`, and `FanoutResult` (`:247`) carries no `#[must_use]` and no `is_complete()` helper.
- Repro or proof: `fan_out_recall` returns `Ok` even when every child failed; the sole in-tree consumer ignores the field (`crates/spectral-bench-real/src/bin/poison_bench.rs:125-129` does `let Ok(res) = ... else { continue };`). This is most dangerous for a schema-drifted read-only peer, which is never migrated by design (`crates/spectral-ingest/src/sqlite_store.rs:112-115`) and therefore degrades to "contributes nothing, reports success".
- Proposed fix: `tracing::warn!` on each failure at `:387`, add `#[must_use]` plus `FanoutResult::is_complete()`, and add a `strict: bool` policy field that turns any child failure into an `Err`.
- Confidence: high

**OP-12 — Three public write APIs bypass the read-only guard, returning a driver error instead of `Error::ReadOnly`**
- Severity: P2
- Location: `crates/spectral-graph/src/brain.rs:2619-2636`, `:3016`, `:4310`
- Evidence: `pub fn set_entity_field(&self, entity_id: &EntityId, ...) -> Result<bool, Error> { self.rt.block_on(self.memory_store.set_entity_field(...))` — no `self.ensure_writable(...)` call, unlike its immediate neighbour `set_entity_description` at `:2610`. Same omission in `consolidate_extractive` (`:3016`) and `reclassify_wings_in` (`:4310`).
- Repro or proof: static reasoning only. The `SQLITE_OPEN_READ_ONLY` flag at `sqlite_store.rs:337` catches the write, so this is a contract-coherence break rather than a data leak — callers matching on `Error::ReadOnly` see `Error::Schema("attempt to write a readonly database")` instead. Separately, `reclassify_wings_in` loads the entire table with `list_memories_by_signal(0.0, usize::MAX)` at `:4317`.
- Proposed fix: add `self.ensure_writable("set_entity_field")` / `"consolidate_extractive"` / `"reclassify_wings_in"` at the top of each, and bound the `reclassify_wings_in` scan with a paged cursor.
- Confidence: high

**OP-13 — `neighborhood()` is an unbounded BFS that holds the graph lock for its full duration**
- Severity: P2
- Location: `crates/spectral-graph/src/graph_store.rs:576-621`
- Evidence: `let conn = self.lock()?;` at `:577`, followed by a `for _ in 0..max_hops` loop at `:589` whose `next_frontier` (`:593`) has no size cap and whose `all_triples` accumulates every edge found.
- Repro or proof: static reasoning only. `recall_graph` calls this once per seed entity at `brain.rs:2463`; a hub entity at 2 hops materialises O(E) triples into memory while every other graph reader and writer blocks on the mutex (and `graph.sqlite` is not in WAL — see OP-05 — so it also blocks at the file level).
- Proposed fix: add a `max_frontier`/`max_triples` budget to `neighborhood()` that stops expansion and reports truncation, and release the lock between hops.
- Confidence: high

**OP-14 — Two of the tests guarding central invariants cannot fail**
- Severity: P2
- Location: `crates/spectral-graph/tests/concurrency_tests.rs:260-308`, `crates/spectral-graph/src/federation.rs:678`
- Evidence: `match brain2_result { Ok(brain2) => { ... } Err(e) => { eprintln!("LIMITATION: Second Brain::open on same path failed: {e}` — both arms pass. And `assert_eq!(result.recognition_token_cost, 0);` asserts a value that is a hardcoded literal at `brain.rs:2386`.
- Repro or proof: `concurrent_brain_opens_same_path` also never contends — it opens `brain2` while `brain1` is idle, then writes sequentially — so the multi-handle invariant that C3 depends on is untested in either direction. The comments at `:266` and `:292` still reference Kuzu, a graph engine no longer in the dependency tree (see also the stale Kuzu block at `brain.rs:961-971`).
- Proposed fix: replace the `match` with a decided contract (either assert both handles coexist under concurrent writes from threads, or assert the second open errors), and change the token-cost gate into a compile-time dependency assertion like the existing `c3_default_features_are_inference_free` (`crates/spectral-recognition/tests/invariants.rs:336-344`).
- Confidence: high

**OP-15 — Placeholder legal attribution in NOTICE, and every benchmark script defaults to a decommissioned host path**
- Severity: P3
- Location: `NOTICE:2-4`, `scripts/run_accuracy_ab.sh:12`
- Evidence: `Copyright 2026 Alice Doe` / `This product includes software developed at Polaris Media.` in NOTICE; `BIN="${BIN:-/Users/jessesharratt/dev/spectral/target/release/spectral-bench-accuracy}"` in the script.
- Repro or proof: `grep -rn "jessesharratt" scripts/ crates/` returns 12 hits across all 11 `run_*.sh` scripts; that user directory no longer exists on this host, so every script fails at its default invocation. Apache-2.0 §4(d) requires the NOTICE file be propagated downstream, so the placeholder ships to every consumer.
- Proposed fix: set the real copyright holder and organization in NOTICE and in `Cargo.toml:20` (`authors = ["Spectral Contributors"]`); change the script default to `BIN="${BIN:-$(git rev-parse --show-toplevel)/target/release/spectral-bench-accuracy}"`.
- Confidence: high

## Production readiness

**SHIP WITH FIXES** — conditional on OP-01 through OP-07 landing first, and on the client's deployment being single-writer and non-federated until then. The single-brain library core is genuinely solid: the FTS5+BM25 recall path, the recognition engine, the ontology-validated graph, and the episodic and adaptive layers are all real, well-tested, and honestly documented, and the central commercial claim — no model call and no per-query cost on any read path — is structurally true, resting on a dependency graph with exactly one `llm_client` call site on a write path. The problems are concentrated at the seams. Federation, the newest and least-exercised subsystem, has no in-tree production consumer and ships three defects that would be serious the first time a real peer is added: it queries peers with no visibility boundary (OP-01), silently mutates peers it reads (OP-02), and accepts unauthenticated authorship and remote deletions on the sync path (OP-03) — while the `verify_hit` machinery that would fix the last of these exists, is correct, and is called by nothing outside tests. Below that, the storage layer's invariants do not hold up to the claims made about them: three databases rather than one, no cross-store atomicity on the primary write, no `busy_timeout` anywhere, and one of the three files silently running a different journal mode than the other two — with the concurrency test that should have caught this written so it passes either way. None of these are hard to fix, and several are one-line changes; but "poisoning-resistant" and "visibility-scoped" and "a single SQLite file" are the phrases in the pitch doc, and today the code does not support them at the strength the pitch states.
