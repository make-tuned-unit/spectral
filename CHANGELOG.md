# Changelog

All notable changes to Spectral will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed
- Sanitized all personal data from public codebase. Replaced with generic placeholders (Alice, Bob, Carol, Acme, Apollo, Polaris, Vega). Default classifier rules and ontology now ship with placeholder examples only. Added `Ontology::empty()` constructor.

### Performance
- `SqliteStore` now memory-maps the database file via adaptive `mmap_size` PRAGMA (50 MB – 1 GB based on file size). This eliminates p99 latency spikes caused by SQLite page cache eviction on databases larger than ~10 MB. On the migrated-brain reference benchmark (1000 iterations): aggregate warm p95 improved 9% (2.35 → 2.14 ms), p99 improved 17% (2.65 → 2.20 ms), worst per-query p99 dropped from 17.18 ms to 7.41 ms, and queries with p99/p95 > 2.0x went from 9 to 2. Configurable via `SqliteStoreConfig::mmap_size` (or `BrainConfig::sqlite_mmap_size`).

### Added (feat/activity-ingestion)
- Activity ingestion and continuous-awareness API. `ActivityEpisode` type for coalesced user activity, `Brain::ingest_activity()` for batched UPSERT ingestion, `Brain::probe()` for single-shot recognition, `Brain::probe_recent()` for episode-window recognition, and retention APIs (`prune_activity_older_than`, `prune_activity_keep_recent`). `RedactionPolicy` trait with `DefaultRedactionPolicy` (strips SSH credentials, URL tokens, Bearer tokens, API key patterns), `NoOpRedactionPolicy`, `ExcludeBundlesPolicy`, and `ComposeRedaction`. Configurable via `BrainConfig::activity_wing` and `BrainConfig::redaction_policy`. New `MemoryStore` methods: `list_wing_memories_since`, `delete_wing_memories_before`, `prune_wing_keeping_recent_per_source`.

### Added (feat/spectral-archivist)
- `spectral-archivist` workspace crate for memory-quality maintenance. Phase 1 passes (pure algorithmic, no LLM dependency): duplicate detection via Jaccard similarity, gap detection (missing summaries, facts, people, projects), reclassification suggestions for general-wing memories, signal score decay/boost based on retrieval recency, and consolidation candidate identification. `Consolidator` and `Indexer` traits for LLM-mediated maintenance passes (default NoOp implementations; Phase 2 will ship reference implementations). CLI binary for running maintenance passes.

### Added (feat/signal-scorer)
- `DefaultSignalScorer` and `SignalScorer` trait in `spectral-ingest`. Heuristic signal scoring based on hall classification and content keywords. Ports the production-validated reference implementation. Pure Rust, no external dependencies, sub-microsecond per memory. Configurable via `SignalScorerConfig`.

### Added (feat/bench-real)
- `spectral-bench-real` workspace crate for measuring recall latency and accuracy against real Spectral brains
- 30 curated benchmark queries covering single-word, multi-word, concept, temporal, cross-domain, and adversarial patterns
- JSON output format suitable for CI integration

### Added (chore/repo-polish)
- `SECURITY.md` updated with disclosure policy
- `CODE_OF_CONDUCT.md` adopting Contributor Covenant 2.1
- GitHub issue templates (bug, feature, config) and PR template
- Dependabot config for weekly Cargo and Actions updates
- `examples/` directory with integration pattern docs (chat memory, activity capture)

### Changed
- `TactConfig::default().min_words` from 3 to 1. Short programmatic queries (e.g., single-word entity lookups) now reach the FTS and fingerprint search paths instead of being silently skipped. Consumers wanting the previous behavior should set `TactConfig::min_words = 3` explicitly.

### Breaking Changes
- `BrainConfig` adds required field `entity_policy: EntityPolicy`. Consumers using `BrainBuilder` are unaffected. Consumers constructing `BrainConfig { ... }` directly must add `entity_policy: EntityPolicy::Strict` to preserve current behavior.

### Added (EntityPolicy)
- `EntityPolicy` enum: `Strict` (default, existing behavior), `AutoCreate`, `AutoCreateWithCanonicalizer(Arc<dyn Fn>)` for runtime entity creation
- `Brain::assert_typed()` for asserting triples with explicit entity types (bypasses predicate-based type inference)
- `BrainConfig::entity_policy` field (default `Strict`)
- `Error::AmbiguousEntityType` for predicates with multiple valid domain/range types under AutoCreate
- Auto-created entities persist to ontology TOML file and survive brain reopen

### Added
- `Brain::ingest_text()` — extract triples from natural-language text via LLM, validate against ontology, assert valid triples, and store original text as a memory
- `IngestTextOpts`, `IngestTextResult`, `RejectedTriple`, `RejectionReason` types for controlling and inspecting text ingestion
- `ExtractionPrompt` in `spectral-graph::extract` for building LLM prompts and parsing responses
- `Error::MissingLlmClient` and `Error::Llm` variants for LLM-related errors
- `Brain::reinforce()` — Memify feedback loop: increase signal_score on useful memories, reset decay clock
- `ReinforceOpts`, `ReinforceResult` types for controlling reinforcement
- Time-based signal decay in `Brain::recall()` (1% per week, capped at 50%, read-only)
- `last_reinforced_at` field on `Memory` and `MemoryHit` for tracking reinforcement history
- `MemoryStore::reinforce_memory()` trait method with SQLite implementation
- Triple reinforcement is deferred pending Kuzu schema work for primary keys on rel tables
- Cognitive Spectrogram: cross-wing fingerprint matching in `spectral-spectrogram` crate
- `Brain::recall_cross_wing()` — find memories across wings with similar cognitive structure
- `Brain::backfill_spectrograms()` — compute spectrograms for existing memories
- `BrainConfig::enable_spectrogram` — opt-in spectrogram computation on ingest
- `SpectrogramAnalyzer` with 7 cognitive dimensions: entity_density, action_type, decision_polarity, causal_depth, emotional_valence, temporal_specificity, novelty
- `memory_spectrogram` SQLite table with idempotent migration
- `MemoryStore` spectrogram trait methods: write_spectrogram, load_spectrogram, load_spectrograms, memories_without_spectrogram

### Performance
- Wing result LRU cache (32 entries) in `SqliteStore` — serves repeated `wing_search()` from memory, invalidated on `write()`
- Compound `(wing, anchor_hall)` and `(wing, target_hall)` indexes on `constellation_fingerprints` — accelerates hall-match path
- Unified CTE for fingerprint search — replaces N per-hash SQLite round-trips + per-id memory fetches with a single server-side scored query
- Single-transaction `write()` — wraps memory + all fingerprint inserts in `BEGIN..COMMIT` for atomicity and 2.0-2.4x faster batch ingest
- `DeviceId` content-addressed identifier in spectral-core
- `Memory.source`, `Memory.device_id`, `Memory.confidence` fields with backward-compatible defaults
- `Brain::remember_with()` for ingestion with full metadata
- `Brain::device_id()` accessor; `BrainConfig.device_id` optional setter
- Idempotent SQLite schema migration adds columns to existing brains on open

### Notes
- These additions are non-breaking. Existing code calling `Brain::remember(key, content, visibility)` continues to work; new fields default to `None`/`1.0`.
- `MemoryStore::write()` trait signature unchanged — the new fields are on the `Memory` struct it already accepts.

### Breaking changes
- **`Brain::recall()`** now takes `context_visibility: Visibility` to filter
  results. Use `Visibility::Private` for the previous see-everything behavior.
- **`Brain::recall_graph()`** — same change.
- **`Brain::remember()`** now takes `visibility: Visibility` to control who can
  see the stored memory. Default to `Visibility::Private` for fail-safe.
- **`Memory`** and **`MemoryHit`** structs gain a `visibility: String` field.

### Added
- Visibility enforcement in recall paths — entities, triples, and memory hits
  are filtered by the caller's visibility context.
- **`Brain::recall_local()`** — convenience for `recall(query, Visibility::Private)`.
- SQLite memory schema: `visibility TEXT NOT NULL DEFAULT 'private'` column.
- Initial workspace scaffolding for spectral-core, spectral-graph,
  spectral-tact, spectral-spectrogram, and the umbrella `spectral` crate.
