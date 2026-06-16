# Federation Foundation Audit

**Scope:** READ-ONLY audit of the Spectral Rust workspace against current `main`
(HEAD `97f40f0a6d9b87715c391783275a2fc792e1b9f8`). Verifies the federation
foundations claimed by prior history (consolidate_into from PRs #131/#132, a
shared-DB OnceLock design, multi-tenant analysis) **against the actual code**.

**Federation, as defined for this audit:** many agents, each with a distinct
brain, sharing memories into a master brain.

**Verdict up front:** The claimed foundations are largely *not* what the history
describes. `consolidate_into()` exists but is an **intra-database**
summary-linking primitive, not a cross-brain transport. There is **no**
shared-DB OnceLock design in the code (no global statics in either store), and
**no** export/import/merge/sync path between brains. The
`visibility_federation_precedent` test is a **single-brain** visibility-filter
test, not a cross-brain demonstration. The only true cross-brain artifact is
`Provenance::source_brain_id`, which records authorship but moves no data.

---

## 1. `consolidate_into()` — Real Signature and Semantics

**It exists, at three layers (trait, impl, two facades):**

- Trait declaration: `crates/spectral-ingest/src/lib.rs:541`
- Concrete impl (the only one): `crates/spectral-ingest/src/sqlite_store.rs:2211`
- Brain facade: `crates/spectral-graph/src/brain.rs:1442`
- Top-level crate facade: `crates/spectral/src/lib.rs:397`

**Important correction to the history:** the implementation lives on the
**SQLite memory store** (`SqliteStore`), *not* on the kuzu graph store. The kuzu
store (`KuzuStore`, `crates/spectral-graph/src/kuzu_store.rs:186`) has no
consolidate method. So `consolidate_into` operates on free-text *memories*, not
graph triples/entities.

**Trait signature** (`crates/spectral-ingest/src/lib.rs:541-546`):

```rust
fn consolidate_into(
    &self,
    source_keys: &[String],
    target_key: &str,
    opts: &ConsolidateOpts,
) -> Pin<Box<dyn Future<Output = anyhow::Result<ConsolidationResult>> + Send + '_>>;
```

**Params:**
- `source_keys: &[String]` — keys of memories to mark as consolidated.
- `target_key: &str` — key of the summary memory they fold into. **Must already
  exist in the same `memories` table** (`sqlite_store.rs:2226-2237`).
- `opts: ConsolidateOpts` — only field is `on_invalid_source:
  InvalidSourcePolicy` ∈ {`AbortAll`, `SkipAndReport`}, default `SkipAndReport`
  (`lib.rs:582-603`).

**What it actually does TODAY** (`sqlite_store.rs:2222-2347`):
1. Verifies the target key exists in `memories`; target-not-found is fatal
   regardless of policy (`sqlite_store.rs:2231-2237`).
2. For each source: skips if `source == target`
   (`SkipReason::SourceEqualsTarget`), skips/aborts if source missing
   (`SourceNotFound`), and skips if the source is **already consolidated into a
   *different* target** (`AlreadyConsolidatedElsewhere`)
   (`sqlite_store.rs:2242-2284`).
3. **Idempotent**: re-consolidating the same source→target pair is a no-op that
   still reports success (`sqlite_store.rs:2272-2276`; test
   `consolidate_into_idempotent` at `sqlite_store.rs:4622`).
4. Inserts a row into `consolidation_edges (source_key, target_key)` via
   `INSERT OR IGNORE` (`sqlite_store.rs:2287-2292`).
5. **Chain flattening**: if a source was itself previously a target, its inbound
   edges are re-pointed to the new target; the original edges are preserved for
   history (`sqlite_store.rs:2294-2302`; test `consolidate_into_chain_flattening`
   at `sqlite_store.rs:4689`).
6. **Signal-score merge**: sums the `signal_score` of newly consolidated sources
   into the target's `signal_score`, **capped at 1.0**
   (`sqlite_store.rs:2305-2327`; test `consolidate_into_score_capped_at_one` at
   `sqlite_store.rs:4720`).

**What it does NOT do:**
- It does **not copy or move** any memory content, embeddings, fingerprints, or
  rows. It only writes link rows and bumps one score column.
- It does **not** dedup by content. The only "dedup" is the
  already-consolidated-elsewhere skip, keyed on `source_key`, not on content.
- It operates on **nodes** (memory rows) within one DB; the "edges" are
  `consolidation_edges` link rows, *not* graph edges in kuzu.

**The schema proves it is single-database** (`sqlite_store.rs:258-264`):

```sql
CREATE TABLE IF NOT EXISTS consolidation_edges (
    source_key      TEXT NOT NULL,
    target_key      TEXT NOT NULL,
    consolidated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (source_key, target_key),
    FOREIGN KEY (source_key) REFERENCES memories(key) ON DELETE CASCADE
);
```

The `FOREIGN KEY (source_key) REFERENCES memories(key)` constraint means a
source must be a row in the **same** brain's `memories` table. **This primitive
structurally cannot reference another brain's memories.** It is a
within-brain summarization/forgetting mechanism (consolidated sources are
excluded from recall — see `list_unconsolidated`, `consolidated_source_keys`,
and the `NOT IN (SELECT source_key FROM consolidation_edges)` filters at
`sqlite_store.rs:1114` and `:1216`), not a federation transport.

---

## 2. Multi-Tenancy Reality

**Construction model: one database per path, per `Brain` instance. No shared
in-process state. No OnceLock anywhere in the store layer.**

`Brain::open` (`crates/spectral-graph/src/brain.rs:431`) constructs **two
independent, owned stores per brain**:
- Graph: `KuzuStore::open(&config.data_dir.join("graph.kz"))`
  (`brain.rs:437`).
- Memory: `SqliteStore::open_with_config(&memory_db_path, ...)`, where
  `memory_db_path` defaults to `data_dir/memory.db` (`brain.rs:442-446`,
  and the `memory_db_path` config field at `brain.rs:59-60`).

**KuzuStore connection model** (`crates/spectral-graph/src/kuzu_store.rs`):
- The struct holds a single owned `Database` handle: `pub struct KuzuStore { db:
  Database }` (`kuzu_store.rs:186-188`).
- `open()` calls `Database::new(path, SystemConfig::default())`
  (`kuzu_store.rs:199-205`); fresh `Connection::new(&self.db)` per operation
  (`kuzu_store.rs:218-220`). The module header documents this
  (`kuzu_store.rs:6-10`).

**SqliteStore connection model** (`crates/spectral-ingest/src/sqlite_store.rs`):
- The struct holds `conn: Arc<Mutex<Connection>>` (`sqlite_store.rs:75`).
- `open_with_config()` opens one `Connection::open(path)` and wraps it
  (`sqlite_store.rs:109-131`). A single mutex-guarded connection per instance.

**OnceLock / statics search result:** There is **no** `OnceLock`, `lazy_static`,
or global `static` connection/database in either store. The only `OnceLock`
usages in the workspace are compiled-regex caches (`activity.rs:111-117`,
`spectral-archivist/src/gaps.rs:21`) — unrelated to DB tenancy. **The
"shared-DB OnceLock design" the history claims does not exist in the code on
main.** (If it existed in a prior branch, it is not present here.)

**What the current model means for "many distinct brains" in one process:**
- At the **Rust** level, each `Brain` is fully isolated: distinct paths,
  distinct owned handles, no shared statics. Constructing N brains in one
  process is structurally supported and each writes to its own files.
- The history's claim that **kuzu does not isolate tenant state in-process due to
  shared C++ statics** is a claim about the kuzu C++ library's internals. **This
  audit found no code in this repo that confirms or refutes that** — the
  embedded kuzu engine is a vendored dependency (`use kuzu::{...}` at
  `kuzu_store.rs:16`) and its C++ globals are not visible in this workspace. I
  cannot cite a file:line in this repo for the C++-statics claim, so it remains
  **unverified from the code here**. The Rust wrapper does instantiate a
  separate `Database` per path, which is the correct usage if isolation holds;
  whether the underlying engine honors it across multiple `Database` handles in
  one process is outside what this repo's source can prove.

---

## 3. Sharing Primitives — Inventory

Everything that exists today for moving or relating memories/edges:

| Primitive | Location | Cross-brain? | What it does |
|---|---|---|---|
| `consolidate_into` | `sqlite_store.rs:2211`, `brain.rs:1442` | **No** | Intra-DB summary linking + score merge (see §1). FK-bound to one `memories` table. |
| `list_consolidated` / `list_unconsolidated` / `consolidated_source_keys` | `sqlite_store.rs:2349`, `:2395`, `:2417` | No | Read-side helpers for the consolidation table; used to exclude consolidated sources from recall. |
| `Visibility` enum + `allows()` | `crates/spectral-core/src/visibility.rs:71-90` | Filter only | `Private<Team<Org<Public>`. `allows()` decides if content *could* be shared into a clearance context. It is a **filter predicate**, not a transport. Doc comment references "federation" (`visibility.rs:27`) but no code moves data. |
| `Provenance { source_brain_id }` | `crates/spectral-graph/src/provenance.rs:62`; stored on triples at `kuzu_store.rs:79`, schema `schema.rs:109` | **Records origin only** | Each triple records the `BrainId` that authored it (`BrainId`, `crates/spectral-core/src/identity.rs:27`). This is the one genuine cross-brain concept, but it only *tags* data — it does not import/merge anything. |
| Export / import / merge / sync between brains | — | — | **None found.** Grep for `fn export`/`fn import`/`fn merge`/`fn ingest_from`/`fn pull_from`/`fn sync` across `spectral-ingest`, `spectral-graph`, `spectral` returned no brain-to-brain transport functions. |

**The `visibility_federation_precedent` test** (`crates/spectral-graph/tests/brain_tests.rs:372-393`):
- It opens a **single** `Brain` (`brain_tests.rs:373-374`), asserts one Private
  and one Org fact for "Carol", then recalls in `Visibility::Org` context and
  asserts no triple below Org visibility leaks (`brain_tests.rs:384-392`).
- **What it actually demonstrates:** that *within one brain*, recall honors the
  visibility filter. The name says "federation precedent," but there is **no
  second brain and no cross-brain sharing** in the test. It establishes the
  visibility-filter invariant that a *future* federation would rely on — it does
  not demonstrate federation itself.

---

## 4. The Hard Questions for the RFC (Forks to Decide — Not Solved Here)

Each fork below is tied to what the current primitives do and do not support.

**Fork A — Read-time federation vs. write-time consolidation.**
The codebase has no mechanism for either today. `consolidate_into` is write-time
*within one DB* and FK-bound to that DB (`sqlite_store.rs:263`), so it cannot be
reused as-is for write-time cross-brain merge. There is no query layer that
spans multiple `Brain`/`KuzuStore`/`SqliteStore` instances — `Brain` holds
exactly one of each (`brain.rs:405-406`). The RFC must choose:
(a) **read-time** — a master that fans out queries across N child brains and
merges results at recall, leaving child DBs authoritative; or
(b) **write-time** — physically copy/merge child memories into a master DB.
Nothing in the current code biases toward either; both are greenfield.

**Fork B — How does a master ingest from N children without N-way write
contention?**
`SqliteStore` serializes all access through a single `Arc<Mutex<Connection>>`
(`sqlite_store.rs:75`), and `KuzuStore` is an embedded single-writer engine
(`kuzu_store.rs:187`). A master brain that N children write into directly would
funnel all writes through one mutex / one embedded writer — a contention point
the RFC must address (batching, a queue/inbox, append-only per-child shards, or
a separate ingestion process). No such mechanism exists today.

**Fork C — Conflict / dedup model for overlapping memories across children.**
The only dedup-like behavior today is `consolidate_into`'s
"already-consolidated-elsewhere" skip, keyed on `source_key`, **not on content**
(`sqlite_store.rs:2263-2284`). There is a `content_hash` column and a backfill
(`backfill_content_hashes`, `sqlite_store.rs:2200`; blake3 over content) that
*could* underpin content-level dedup, but nothing wires it into consolidation or
any cross-brain path. Two children asserting the same fact would today produce
two independent rows in two DBs with two different `source_brain_id`s
(`provenance.rs:62`). The RFC must decide: dedup by `content_hash`? by triple
identity? keep all and rank by provenance/agreement? merge `signal_score`
(consolidation already sums-and-caps, `sqlite_store.rs:2323`, suggesting a
precedent) or recompute?

**Fork D — Provenance and visibility semantics under federation.**
`source_brain_id` is recorded per triple (`kuzu_store.rs:79`, `schema.rs:109`)
and `Visibility::allows()` (`visibility.rs:87-89`) already expresses "may this
content cross into a broader-clearance context." But no code consults
`source_brain_id` for access control, and `allows()` is only used as a recall
filter within one brain (per the §3 test). The RFC must decide how a master
enforces per-child visibility on ingested/queried memories and whether
`source_brain_id` becomes an authorization input rather than a passive tag.

---

## Citations I Could NOT Substantiate

- **"Shared-DB OnceLock design":** not present in the code on main; no global
  static DB/connection exists in `spectral-graph` or `spectral-ingest` (only
  regex-cache `OnceLock`s, unrelated). Reported as absent.
- **"kuzu does not isolate tenant state in-process (shared C++ statics)":**
  cannot be confirmed or refuted from this repo — kuzu is a vendored dependency
  and its C++ internals are not in-tree. Stated as unverified.
- **PR #131/#132 authorship of `consolidate_into`:** the function exists on
  main, but I did not trace blame to those PR numbers; cite as "present on main"
  rather than attributing to specific PRs.
