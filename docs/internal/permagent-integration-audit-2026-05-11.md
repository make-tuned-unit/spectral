# Spectral API Audit for Permagent Integration

**Date:** 2026-05-11
**Main HEAD:** `5b9c457ffca95f806d08c0b0ff35e6bbae97c2e4`
**Main HEAD message:** `fix: add get_memory/set_description/list_undescribed to Brain wrapper (#78)`

---

## 1. Repo state

### Last 10 merged PRs

| # | Title | Merged |
|---|-------|--------|
| 79 | feat(retrieval): session-aware retrieval events | 2026-05-09 |
| 78 | fix: add description methods to Brain wrapper | 2026-05-09 |
| 77 | fix(retrieval): wrap rebuild_co_retrieval_index in transaction | 2026-05-08 |
| 76 | feat(retrieval): co-retrieval index — surface related memories | 2026-05-08 |
| 75 | feat(schema): description field + Brain endpoints for Librarian | 2026-05-08 |
| 74 | feat(ingest): store declarative density at ingest time (P6) | 2026-05-08 |
| 73 | feat(retrieval): recall→recognition feedback loop (P1+P2) | 2026-05-08 |
| 72 | feat(ranking): declarative density signal boost | 2026-05-08 |
| 71 | fix(bench): cascade K uses pipeline_config.k not CLI max_results | 2026-05-08 |
| 70 | feat(bench): question-type routing + session-grouped formatting + recency fix | 2026-05-08 |

### Tags

```
experimental-k50-2026-05-01
baseline-2026-05-01-topk20
baseline-2026-05-01-pre-audit
```

**34 commits on main since latest tag** (`experimental-k50-2026-05-01`). All PRs #41–#79 are post-tag. No release tags exist.

---

## 2. Public API surface — Brain

File: `crates/spectral-graph/src/brain.rs`

All methods below are **sync with internal `self.rt.block_on()`** unless noted otherwise. Callers from an async context must use `spawn_blocking`.

### Brain::get_memory

```rust
pub fn get_memory(&self, id: &str) -> Result<Option<spectral_ingest::Memory>, Error>
```
Sync-with-block_on. Added in PR #78.

### Brain::set_description

```rust
pub fn set_description(&self, id: &str, description: &str) -> Result<(), Error>
```
Sync-with-block_on. Added in PR #78.

### Brain::list_undescribed

```rust
pub fn list_undescribed(&self, limit: usize) -> Result<Vec<spectral_ingest::Memory>, Error>
```
Sync-with-block_on. Added in PR #78.

### Brain::recall

```rust
pub fn recall(&self, query: &str, context_visibility: Visibility) -> Result<HybridRecallResult, Error>
```
Sync-with-block_on.

### Brain::recall_cascade

```rust
pub fn recall_cascade(
    &self,
    query: &str,
    context: &spectral_cascade::RecognitionContext,
    config: &spectral_cascade::orchestrator::CascadeConfig,
) -> Result<spectral_cascade::result::CascadeResult, Error>
```
Calls `run_cascade_pipeline` — no direct `block_on` at this level, but the pipeline internally does block_on.

### Brain::recall_cascade_with_pipeline

```rust
pub fn recall_cascade_with_pipeline(
    &self,
    query: &str,
    context: &spectral_cascade::RecognitionContext,
    pipeline_config: &crate::cascade_layers::CascadePipelineConfig,
) -> Result<spectral_cascade::result::CascadeResult, Error>
```
Same as `recall_cascade` — pipeline-internal block_on.

### Brain::remember

```rust
pub fn remember(
    &self,
    key: &str,
    content: &str,
    visibility: Visibility,
) -> Result<RememberResult, Error>
```
Delegates to `remember_with`. Sync-with-block_on.

### Brain::remember_with

```rust
pub fn remember_with(
    &self,
    key: &str,
    content: &str,
    opts: RememberOpts,
) -> Result<RememberResult, Error>
```
Sync-with-block_on.

### Brain::set_compaction_tier

```rust
pub fn set_compaction_tier(
    &self,
    memory_id: &str,
    tier: spectral_ingest::CompactionTier,
) -> Result<(), Error>
```
Sync-with-block_on.

### Brain::annotate

```rust
pub fn annotate(
    &self,
    memory_id: &str,
    input: spectral_ingest::AnnotationInput,
) -> Result<spectral_ingest::MemoryAnnotation, Error>
```
Sync-with-block_on.

### Brain::list_annotations

```rust
pub fn list_annotations(
    &self,
    memory_id: &str,
) -> Result<Vec<spectral_ingest::MemoryAnnotation>, Error>
```
Sync-with-block_on.

### Brain::probe

```rust
pub fn probe(
    &self,
    context: &str,
    opts: crate::activity::ProbeOpts,
) -> Result<Vec<crate::activity::RecognizedMemory>, Error>
```
Sync-with-block_on (via `recall` internally).

### New public methods added in last 10 merged PRs

| Method | PR | Signature |
|--------|---:|-----------|
| `related_memories` | #76 | `pub fn related_memories(&self, memory_id: &str, limit: usize) -> Result<Vec<spectral_ingest::RelatedMemory>, Error>` |
| `rebuild_co_retrieval_index` | #76 | `pub fn rebuild_co_retrieval_index(&self) -> Result<usize, Error>` |
| `events_for_session` | #79 | `pub fn events_for_session(&self, session_id: &str, limit: usize) -> Result<Vec<spectral_ingest::RetrievalEvent>, Error>` |
| `memories_for_session` | #79 | `pub fn memories_for_session(&self, session_id: &str) -> Result<Vec<String>, Error>` |
| `count_retrieval_events` | #73 | `pub fn count_retrieval_events(&self) -> Result<usize, Error>` |
| `count_retrieval_events_by_method` | #73 | `pub fn count_retrieval_events_by_method(&self, method: &str) -> Result<usize, Error>` |
| `get_memory` | #78 | See above |
| `set_description` | #78 | See above |
| `list_undescribed` | #78 | See above |
| `recall_topk_fts` | #59 | `pub fn recall_topk_fts(&self, query: &str, config: &RecallTopKConfig, visibility: Visibility) -> Result<Vec<spectral_ingest::MemoryHit>, Error>` |
| `aaak` | pre-#70 | `pub fn aaak(&self, opts: AaakOpts) -> Result<AaakResult, Error>` |

All are sync-with-block_on.

---

## 3. Public types

### Memory

File: `crates/spectral-ingest/src/lib.rs`

```rust
pub struct Memory {
    pub id: String,
    pub key: String,
    pub content: String,
    pub wing: Option<String>,
    pub hall: Option<String>,
    pub signal_score: f64,
    pub visibility: String,              // serde default "private"
    pub source: Option<String>,
    pub device_id: Option<[u8; 32]>,
    pub confidence: f64,                 // serde default 1.0
    pub created_at: Option<String>,
    pub last_reinforced_at: Option<String>,
    pub episode_id: Option<String>,
    pub compaction_tier: Option<CompactionTier>,
    pub declarative_density: Option<f64>,   // ADDED PR #74
    pub description: Option<String>,        // ADDED PR #75
    pub description_generated_at: Option<String>,  // ADDED PR #75
}
```

**Recently added fields:**
- `declarative_density` — PR #74 (2026-05-08)
- `description` — PR #75 (2026-05-08)
- `description_generated_at` — PR #75 (2026-05-08)

### MemoryHit

File: `crates/spectral-ingest/src/lib.rs`

```rust
pub struct MemoryHit {
    pub id: String,
    pub key: String,
    pub content: String,
    pub wing: Option<String>,
    pub hall: Option<String>,
    pub signal_score: f64,
    pub visibility: String,
    pub hits: usize,
    pub source: Option<String>,
    pub device_id: Option<[u8; 32]>,
    pub confidence: f64,
    pub created_at: Option<String>,
    pub last_reinforced_at: Option<String>,
    pub episode_id: Option<String>,
    pub declarative_density: Option<f64>,   // ADDED PR #74
    pub description: Option<String>,        // ADDED PR #75
}
```

**Recently added fields:** same as Memory minus `description_generated_at`.

### RememberOpts

File: `crates/spectral-graph/src/brain.rs`

```rust
pub struct RememberOpts {
    pub source: Option<String>,
    pub device_id: Option<DeviceId>,
    pub confidence: Option<f64>,
    pub visibility: Visibility,
    pub created_at: Option<DateTime<Utc>>,
    pub episode_id: Option<String>,
    pub compaction_tier: Option<spectral_ingest::CompactionTier>,
    pub wing: Option<String>,   // ADDED PR #56 — bypasses classifier when Some
}
```

**Recently added fields:**
- `wing` — PR #56 (2026-05-07)

### AnnotationInput

File: `crates/spectral-ingest/src/lib.rs`

```rust
pub struct AnnotationInput {
    pub description: String,
    pub who: Vec<EntityRef>,
    pub why: String,
    pub where_: Option<String>,
    pub when_: chrono::DateTime<chrono::Utc>,
    pub how: String,
}
```

### EntityRef

File: `crates/spectral-ingest/src/lib.rs`

```rust
pub struct EntityRef {
    pub canonical_id: String,
    pub display_name: String,
}
```

### MemoryAnnotation

File: `crates/spectral-ingest/src/lib.rs`

```rust
pub struct MemoryAnnotation {
    pub id: String,
    pub memory_id: String,
    pub description: String,
    pub who: Vec<EntityRef>,
    pub why: String,
    pub where_: Option<String>,
    pub when_: chrono::DateTime<chrono::Utc>,
    pub how: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}
```

### RecognitionContext

File: `crates/spectral-cascade/src/context.rs`

```rust
pub struct RecognitionContext {
    pub recent_activity: Vec<ActivityEpisode>,
    pub now: DateTime<Utc>,
    pub focus_wing: Option<String>,
    pub persona: Option<String>,
    pub session_id: Option<String>,   // ADDED PR #79
}
```

**Recently added fields:**
- `session_id` — PR #79 (2026-05-09)

### RelatedMemory

File: `crates/spectral-ingest/src/lib.rs`. **Added PR #76.**

```rust
pub struct RelatedMemory {
    pub memory_id: String,
    pub co_count: u64,
    pub memory: Option<Memory>,
}
```

### RetrievalEvent

File: `crates/spectral-ingest/src/lib.rs`. **Added PR #73, modified PR #79.**

```rust
pub struct RetrievalEvent {
    pub query_hash: String,
    pub timestamp: String,
    pub memory_ids_json: String,
    pub method: String,
    pub wing: Option<String>,
    pub question_type: Option<String>,
    pub session_id: Option<String>,   // ADDED PR #79
}
```

### CompactionTier

File: `crates/spectral-ingest/src/lib.rs`

```rust
pub enum CompactionTier {
    Raw,
    HourlyRollup,
    DailyRollup,
    WeeklyRollup,
}
```

---

## 4. Schema state

All tables from `crates/spectral-ingest/src/sqlite_store.rs`.

### memories

```sql
CREATE TABLE IF NOT EXISTS memories (
    id            TEXT PRIMARY KEY,
    key           TEXT NOT NULL UNIQUE,
    content       TEXT NOT NULL,
    category      TEXT NOT NULL DEFAULT 'core',
    wing          TEXT DEFAULT NULL,
    hall          TEXT DEFAULT NULL,
    signal_score  REAL DEFAULT 0.5,
    visibility    TEXT NOT NULL DEFAULT 'private',
    created_at    TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at    TEXT NOT NULL DEFAULT (datetime('now')),
    source        TEXT DEFAULT NULL,
    device_id     BLOB DEFAULT NULL,
    confidence    REAL NOT NULL DEFAULT 1.0
);
-- ALTER TABLE additions (auto-migrated):
--   last_reinforced_at    TEXT DEFAULT NULL
--   episode_id            TEXT DEFAULT NULL
--   declarative_density   REAL DEFAULT NULL          -- PR #74
--   compaction_tier       TEXT DEFAULT NULL
--   description           TEXT DEFAULT NULL           -- PR #75
--   description_generated_at TEXT DEFAULT NULL         -- PR #75
```

Nullable: `wing`, `hall`, `source`, `device_id`, `last_reinforced_at`, `episode_id`, `declarative_density`, `compaction_tier`, `description`, `description_generated_at`

### retrieval_events (PR #73)

```sql
CREATE TABLE IF NOT EXISTS retrieval_events (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    query_hash      TEXT NOT NULL,
    timestamp       TEXT NOT NULL,
    memory_ids_json TEXT NOT NULL,
    method          TEXT NOT NULL,
    wing            TEXT,
    question_type   TEXT
);
-- ALTER TABLE addition:
--   session_id TEXT DEFAULT NULL    -- PR #79
```

Nullable: `wing`, `question_type`, `session_id`

### co_retrieval_pairs (PR #76)

```sql
CREATE TABLE IF NOT EXISTS co_retrieval_pairs (
    memory_id_a TEXT NOT NULL,
    memory_id_b TEXT NOT NULL,
    co_count    INTEGER NOT NULL DEFAULT 0,
    last_updated TEXT NOT NULL,
    PRIMARY KEY (memory_id_a, memory_id_b)
);
```

No nullable columns.

### memory_annotations

```sql
CREATE TABLE IF NOT EXISTS memory_annotations (
    id          TEXT PRIMARY KEY,
    memory_id   TEXT NOT NULL,
    description TEXT NOT NULL,
    who         TEXT NOT NULL,
    why         TEXT NOT NULL,
    where_      TEXT,
    when_       TEXT NOT NULL,
    how         TEXT NOT NULL,
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (memory_id) REFERENCES memories(id) ON DELETE CASCADE
);
```

Nullable: `where_`

### memory_spectrogram

```sql
CREATE TABLE IF NOT EXISTS memory_spectrogram (
    memory_id         TEXT PRIMARY KEY,
    entity_density    REAL,
    action_type       TEXT,
    decision_polarity REAL,
    causal_depth      REAL,
    emotional_valence REAL,
    temporal_specificity REAL,
    novelty           REAL,
    peak_dimensions   TEXT,
    created_at        TEXT DEFAULT (datetime('now')),
    FOREIGN KEY (memory_id) REFERENCES memories(id)
);
```

Nullable: all dimension columns

### constellation_fingerprints

```sql
CREATE TABLE IF NOT EXISTS constellation_fingerprints (
    id                TEXT PRIMARY KEY,
    fingerprint_hash  TEXT NOT NULL,
    anchor_memory_id  TEXT NOT NULL,
    target_memory_id  TEXT NOT NULL,
    wing              TEXT,
    anchor_hall       TEXT,
    target_hall       TEXT,
    time_delta_bucket TEXT,
    created_at        TEXT,
    FOREIGN KEY (anchor_memory_id) REFERENCES memories(id),
    FOREIGN KEY (target_memory_id) REFERENCES memories(id)
);
```

Nullable: `wing`, `anchor_hall`, `target_hall`, `time_delta_bucket`, `created_at`

### episodes

```sql
CREATE TABLE IF NOT EXISTS episodes (
    id             TEXT PRIMARY KEY,
    started_at     TEXT NOT NULL,
    ended_at       TEXT NOT NULL,
    memory_count   INTEGER NOT NULL DEFAULT 0,
    wing           TEXT NOT NULL,
    summary_preview TEXT,
    created_at     TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at     TEXT NOT NULL DEFAULT (datetime('now'))
);
```

Nullable: `summary_preview`

---

## 5. Cargo coordinates

### Workspace crates

| Crate | Version |
|-------|---------|
| spectral | 0.0.1 |
| spectral-core | 0.0.1 |
| spectral-ingest | 0.0.1 |
| spectral-graph | 0.0.1 |
| spectral-cascade | 0.0.1 |
| spectral-spectrogram | 0.0.1 |
| spectral-tact | 0.0.1 |
| spectral-archivist | 0.0.1 |
| spectral-bench-real | 0.0.1 |
| spectral-bench-accuracy | 0.0.1 |

### Re-export crate

`spectral` (the root crate at `crates/spectral/src/lib.rs`) re-exports `Brain` and key types:

```rust
pub use spectral_graph::brain::{
    AaakOpts, AaakResult, AssertResult, CrossWingRecallResult, EntityPolicy,
    HybridRecallResult, IngestResult, IngestTextOpts, IngestTextResult,
    RecallResult, RecallTopKConfig, ReinforceOpts, ReinforceResult,
    RejectedTriple, RejectionReason, RememberOpts, RememberResult,
    ResonantMemoryHit,
};
pub use spectral_graph::Error;
pub use spectral_core::device_id::DeviceId;
pub use spectral_core::visibility::Visibility;
```

**Note:** `Brain` itself is NOT in the `pub use` list. It is accessed through `spectral::graph::brain::Brain` or by `use spectral_graph::brain::Brain` directly. The `spectral` crate provides a `BrainBuilder` wrapper instead (defined in `crates/spectral/src/lib.rs`).

### Git pinning

```toml
spectral-graph = { git = "https://github.com/make-tuned-unit/spectral.git", rev = "5b9c457" }
```

Or branch pinning:

```toml
spectral-graph = { git = "https://github.com/make-tuned-unit/spectral.git", branch = "main" }
```

No release tags are meaningful — all 3 tags are bench experiment markers, not release versions.

---

## 6. Known gotchas

### All Brain methods are sync-with-internal-block_on

Every `Brain` method calls `self.rt.block_on(...)` on an internal single-threaded tokio runtime. If called from within an existing tokio context (e.g., Permagent's async agent loop), this will panic:

```
Cannot start a runtime from within a runtime
```

**Fix:** Wrap all `Brain` calls in `tokio::task::spawn_blocking(move || brain.method(...))`.

### RememberOpts requires explicit construction

`RememberOpts` derives `Default` but the default `visibility` is `Visibility::Private`. Callers must either use `..Default::default()` or set every field. The `wing: Option<String>` field (PR #56) is new — downstream code pinned before PR #56 will fail to compile if constructing `RememberOpts { ... }` without `wing`.

### MemoryHit construction in tests

`MemoryHit` has 14 fields. Test code constructing `MemoryHit` directly must include `declarative_density` (PR #74) and `description` (PR #75) or compilation fails. Use `..Default::default()` if `MemoryHit` derives Default, but **it does not** — every field must be specified.

### RecognitionContext::session_id is new (PR #79)

`RecognitionContext` gained `session_id: Option<String>` in PR #79. Code using struct literal construction must add this field. The `RecognitionContext::empty()` constructor handles it, but direct construction does not.

### Brain::remember_with does NOT upsert by key

Calling `remember_with` twice with the same key creates two memories. There is no upsert. This is the idempotency gap flagged in the ingest contract design doc.

### PR #78 wrapper methods

PR #78 added `get_memory`, `set_description`, and `list_undescribed` to the `Brain` struct. Before PR #78, these were only on the `MemoryStore` trait (async). Consumers previously had to access `brain.memory_store` directly (which is private). PR #78 is the intended public API — calling the trait methods directly is not possible from outside `spectral-graph`.

---

## 7. CI shape

File: `.github/workflows/ci.yml`

### Jobs

| Job | OS | Steps |
|-----|----|-------|
| test | `[ubuntu-latest, macos-latest]` | Unit/integration tests + doc tests |
| lint | `ubuntu-latest` | Format + Clippy |
| build | `ubuntu-latest` | Default features + no-default-features |

### Exact invocations

**Format:**
```
cargo fmt --all -- --check
```

**Clippy:**
```
cargo clippy --all-targets --all-features -- -D warnings
```

**Test:**
```
cargo test --workspace --lib --tests
cargo test --workspace --doc
```

**Build:**
```
cargo build -p spectral
cargo build -p spectral --no-default-features
```

### Notes for Permagent CI alignment

- Clippy uses `-D warnings` (warnings are errors) with `--all-targets --all-features`
- Tests use `--workspace --lib --tests` (no `--doc` in the first pass, separate `--doc` pass)
- OS matrix: `ubuntu-latest` + `macos-latest` for tests, `ubuntu-latest` only for lint and build
- Rust toolchain: `stable` via `dtolnay/rust-toolchain@stable`
- Cache: `Swatinem/rust-cache@v2`
