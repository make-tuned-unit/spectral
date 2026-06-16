# Fan-out Feasibility Audit

**Scope:** READ-ONLY audit of the Spectral Rust workspace on `main` to determine
how much of the agreed federation v1 design (read-time fan-out, single-tenant, N
small local brains, a coordinator "Henry" that opens N child brains, queries each,
and merges by provenance with `source_brain_id` as tiebreaker) lives ABOVE the
Brain API versus requires core changes.

Every factual claim cites `file:line` against the worktree. No source was changed;
no benchmarks or builds were run. Where evidence is inconclusive (kuzu is a vendored
C++ dep), that is stated explicitly.

---

## 1. BRAIN HANDLE MODEL

**Can ONE process hold N Brain instances on N distinct DB paths simultaneously today? Yes — construction is fully per-instance with no global/static DB and no path collision.**

- The `Brain` struct owns its stores by value: `store: KuzuStore` and
  `memory_store: Box<dyn MemoryStore>` are plain owned fields, not shared globals
  (`crates/spectral-graph/src/brain.rs:398-416`, specifically `:405-406`). Each
  `Brain` also owns its own `tokio::runtime::Runtime` (`brain.rs:415`), its own
  `BrainIdentity` (`:399`), and its own `Mutex<Vec<OntologyEntity>>` runtime-entity
  list (`:403`). Nothing is `static`.

- `Brain::open()` is fully per-call construction. It is parameterized entirely by
  `BrainConfig { data_dir, ontology_path, memory_db_path, ... }`
  (`brain.rs:54-92`). All paths come from the config:
  - kuzu opens at `config.data_dir.join("graph.kz")` (`brain.rs:437`)
  - sqlite opens at `config.memory_db_path` or `config.data_dir.join("memory.db")`
    (`brain.rs:439-441`)
  Two `Brain::open` calls with distinct `data_dir`s therefore touch entirely
  disjoint files — no path collision by construction.

- **KuzuStore ownership is per-instance.** `KuzuStore { db: Database }` holds an
  owned kuzu `Database` by value (`crates/spectral-graph/src/kuzu_store.rs:186-188`).
  `KuzuStore::open` calls `Database::new(path, SystemConfig::default())`
  (`kuzu_store.rs:199-206`). There is no `OnceLock`, `lazy_static`, `once_cell`, or
  `static` DB anywhere in `kuzu_store.rs` (confirmed by grep: the only matches for
  static-like constructs in `spectral-graph` are stateless compiled-regex
  `OnceLock`s in `activity.rs:111-117`, used for redaction, not database state).

- **SqliteStore ownership is per-instance.** `SqliteStore { conn: Arc<Mutex<Connection>> }`
  (`crates/spectral-ingest/src/sqlite_store.rs:74-75`), opened per path via
  `SqliteStore::open_with_config(&memory_db_path, ...)` (`brain.rs:445-448`,
  `sqlite_store.rs:109`). The `Arc<Mutex<>>` is created inside `open`, so each
  instance gets its own connection. No global pool.

- **Proof that ≥2 brains can be opened in one process:** `open_is_idempotent`
  (`crates/spectral-graph/tests/brain_tests.rs:41-52`) opens `brain1`, drops it,
  then opens `brain2` on the same path — sequential, not co-resident.
  `concurrent_brain_opens_same_path`
  (`crates/spectral-graph/tests/concurrency_tests.rs:256-305`) opens `brain1` and
  `brain2` **simultaneously and live in the same process** (`:260` and `:264`,
  with both used at `:269-282`). Note this test uses the **same** `data_dir`; it is
  documented as a known-limitation probe (`concurrency_tests.rs:247-255`) and its
  assertions are permissive (it tolerates the second open failing, `:296-304`).

**Caveat for this section:** No test in the tree opens N brains on **N distinct
paths** co-resident in one process. The same-path test exists; the distinct-path
case (which is what fan-out needs) is *more* favorable on the file-locking axis but
is **not directly proven by an existing test** and is subject to the kuzu-statics
risk in §5. The handle model itself (owned, per-instance, no global) is confirmed;
the in-process co-residence of the kuzu C++ layer is the open question (§5).

---

## 2. QUERY SURFACE

**The recall entry points are `&self` methods on a single Brain handle, so a coordinator can call recall on each of N handles and collect N result sets with the fan-out logic living entirely above the trait.**

- `recall_cascade(&self, ...)` (`crates/spectral-graph/src/brain.rs:1233-1240`)
- `recall_cascade_with_pipeline(&self, ...)` (`brain.rs:1246-1267`)
- `recall(&self, ...)` (`brain.rs:996-1002`) and `recall_at(&self, ...)` (`:1009-1050`)
- `recall_local(&self, ...)` (`brain.rs:1056-1058`) / `recall_local_at` (`:1061-1067`)
- `aaak` — not read in full, but the AAAK option/result types are plain data
  (`AaakOpts` `brain.rs:273-302`, `AaakResult` `:304-317`); the method follows the
  same `&self` pattern as every other recall method in the `impl Brain` block.

All recall methods take `&self`, never `&mut self`, so a coordinator holding
`Vec<Brain>` (or `Vec<Arc<Brain>>`) can iterate and call recall on each. The
fan-out + merge is ordinary application code above the Brain API — no trait change
required to *call* recall N times.

**Global single-brain state check — none found that blocks fan-out:**
- No process-wide DB singleton (see §1).
- The only `static`s in the recall-adjacent crates are stateless compiled regexes
  (`spectral-graph/src/activity.rs:111-117`) — shared immutable `Regex`, safe across
  instances, carry no per-brain state.
- Each `Brain` owns its own tokio `Runtime` (`brain.rs:415`), so there is no shared
  async executor that N brains contend on at construction; recall blocks on the
  owning brain's runtime (`recall_at` → `self.rt.block_on(...)` `brain.rs:1015-1022`).
- `recall_topk_fts` writes a best-effort `RetrievalEvent` into **its own**
  `memory_store` (`brain.rs:1211-1222`) — a per-brain side effect, not a shared cache.

**One soft caveat:** the time anchor. `recall_cascade` does not take a `now`; the
non-`_at` recall paths default decay to `Utc::now()` (`recall` `brain.rs:1001`;
`RecallTopKConfig::now == None` falls back to `Utc::now()`, documented at
`brain.rs:334-343`). This is per-call behavior, not global state, but a coordinator
that wants deterministic cross-brain merge should pass an explicit anchor
(`recall_at` / pipeline config carrying `now`) so all N children decay from the same
instant. Not a blocker; a correctness note for the coordinator.

---

## 3. RESULT MERGE INPUTS

**A recall returns `MemoryHit`s (wrapped in `CascadeResult` / `HybridRecallResult`). `MemoryHit` does NOT carry `source_brain_id` or provenance. This is the one small core touch v1 needs.**

Return types:
- `recall_cascade` / `recall_cascade_with_pipeline` →
  `spectral_cascade::result::CascadeResult` (`brain.rs:1238`, `:1261-1266`), whose
  payload is `merged_hits: Vec<MemoryHit>`
  (`crates/spectral-cascade/src/result.rs:6-8`).
- `recall` / `recall_at` → `HybridRecallResult { memory_hits: Vec<MemoryHit>, tact, graph }`
  (`brain.rs:113-120`, built at `:1045-1049`).
- `recall_local` is `recall` with `Visibility::Private` (`brain.rs:1056-1058`).

The hit type is **`spectral_ingest::MemoryHit`**
(`crates/spectral-ingest/src/lib.rs:141-178`). Its fields:
`id, key, content, wing, hall, signal_score, visibility, hits, source, device_id,
confidence, created_at, last_reinforced_at, episode_id, declarative_density,
description` (`lib.rs:143-177`).

**There is no `source_brain_id` / `Provenance` field on `MemoryHit`.** The closest
field is `source: Option<String>` (`lib.rs:155-156`), which is the free-text
ingest source (e.g. a filename), **not** the originating brain identity.

Note the asymmetry: provenance with `source_brain_id` **does** exist, but only on
the **graph triple** path, not the memory/recall path:
- `Provenance { source_doc_id, source_brain_id, asserted_at }`
  (`crates/spectral-graph/src/provenance.rs:57-65`) — types only, "tags origin".
- `Triple.source_brain_id: BrainId` (`kuzu_store.rs:78-79`), stamped at assert time
  from `*self.identity.brain_id()` (`brain.rs:863-872`, specifically `:869`).
- `MemoryHit` (the recall return) has no equivalent.

**Where the provenance touch would go (precise):** the merged/ranked recall hits are
`MemoryHit`s, and `MemoryHit` is the one type a coordinator can rank by origin.
Two options, smallest first:

1. **Coordinator-side tagging (no core change).** Because `recall_*` is `&self` and
   the coordinator knows which `Brain` it called, Henry can wrap each returned
   `MemoryHit` with the brain's id obtained from `Brain::brain_id()`
   (`brain.rs:522-524`) — e.g. collect `(BrainId, MemoryHit)` pairs and rank on the
   pair. **This needs zero core changes**: the coordinator already holds the handle
   and the id. This is the recommended v1 path.

2. **If provenance must live on the hit itself** (e.g. results are serialized and
   detached from the handle, given `MemoryHit: Serialize/Deserialize` at
   `lib.rs:141`), add `source_brain_id: Option<BrainId>` (or `[u8;32]`) to
   `struct MemoryHit` (`crates/spectral-ingest/src/lib.rs:142-178`) with
   `#[serde(default)]`. It would default `None` at the SQL build sites inside
   `spectral-ingest` (the `fts_search` / fingerprint-search row constructors that
   populate `MemoryHit`) and be stamped by the recall path — concretely in
   `Brain::recall_at` where hits are mapped (`brain.rs:1027-1041`) and in the
   cascade assembly (`brain.rs:1252-1255`), setting it from
   `self.identity.brain_id()`. This is the "one small core touch" — a single new
   optional field plus a stamp in the recall builders. It does not alter any trait
   method signature.

**Tiebreaker gotcha:** `BrainId` derives only `Clone, Copy, PartialEq, Eq, Hash`
— **not `Ord`/`PartialOrd`** (`crates/spectral-core/src/identity.rs:26-27`). To use
`source_brain_id` as a sort tiebreaker, the coordinator must order on
`brain_id.as_bytes()` (`identity.rs:41-43`) or the design must add an `Ord` derive.
Trivial, but worth stating since the spec names `source_brain_id` as the tiebreaker.

---

## 4. REGISTRY

**There is no existing "known brains / known paths" registry. v1 introduces it from scratch (small).**

- `BrainConfig` (`brain.rs:54-92`) describes a **single** brain's paths
  (`data_dir`, `ontology_path`, `memory_db_path`). There is no list-of-brains, no
  directory-scan, no config enumerating sibling brains.
- A grep for multi-brain / registry constructs surfaces only test-local pairs
  (`brain_tests.rs:44-48`, `concurrency_tests.rs:260-264`) and the documented
  "one writer per brain" guidance (`docs/operational-considerations.md:70-107`).
  Nothing enumerates or discovers a set of brains.
- The coordinator "Henry" and its registry of child brain paths are **net-new**.
  This is small (a `Vec<PathBuf>` or a config list, each opened via `Brain::open`)
  and lives entirely above the Brain API.

---

## 5. THE KUZU CAVEAT, CONCRETELY

**This is the one real technical risk to the in-process plan. Evidence in-repo shows kuzu has already produced a Linux SIGABRT, and the failure is in `create_schema` during `Brain::open` (the FFI layer), not in recall. The internals are a vendored C++ dep and not in-tree, so the multi-handle-in-one-process question cannot be settled from source alone — it needs an empirical test (specified below).**

What the repo/history actually shows:

- **There is a documented kuzu SIGABRT on Linux**, with a reproducer test and a CI
  diagnostic workflow:
  - `.github/workflows/kuzu-abort-diagnostic.yml` — a `workflow_dispatch` job named
    "Kuzu abort diagnostic" that runs an ignored test `kuzu_schema_abort_repro`
    "single brain" (`:19-23`) **and** a step explicitly labeled
    **"Run all spectral-graph tests (multi-brain abort repro)"** (`:24-28`). The
    existence of a *multi-brain abort repro* step is direct evidence the team
    suspected multiple brains in one process as an abort trigger.
  - Related to issue **#153** (per the diagnostic-workflow commit message,
    `git show 079748b`).

- **The root cause was re-diagnosed and is in schema creation, in-process — not
  teardown.** Commit `4757aa1` ("fix(docs): revise kuzu Linux abort diagnosis —
  schema creation, not teardown") states: *"The abort happens inside `create_schema`
  (schema.rs) due to a kuzu cxxbridge FFI bug: a C++ exception thrown by
  `Connection::query` is not converted into a Rust `Result`, `std::terminate`
  fires."* It also notes the *"Previous diagnosis (glibc 2.39 teardown,
  instance-count threshold) disproven by in-process SIGABRT backtrace."*
  - Relevant code path: `Brain::open` → `KuzuStore::open` → `create_schema(&conn)`
    (`kuzu_store.rs:199-206`, `:203`); `create_schema` lives in
    `crates/spectral-graph/src/schema.rs:31+` and runs `Connection::query`-style
    Cypher DDL. The abort is on the **open/schema** path, i.e. exactly the path a
    fan-out coordinator hits N times when opening N child brains.

- **Honest read of severity for fan-out:**
  - The crash is **platform-specific (Linux)** and surfaced via FFI exception
    translation, *not* via shared C++ statics being corrupted by a second
    `Database`. The corrected diagnosis explicitly **disproved** the
    "instance-count threshold" theory — i.e. the available evidence does **not**
    confirm "opening DB #2 corrupts DB #1 via shared statics." It points instead to
    a schema-DDL FFI fragility that can fire even for a *single* brain on Linux.
  - That cuts both ways. Good news: there is no in-tree evidence of a *shared-static*
    corruption across co-resident `Database`s. Bad news: schema creation is already
    abort-prone on Linux for one brain; doing it N times in one process multiplies
    exposure to the same FFI bug, and the very existence of a "multi-brain abort
    repro" CI step shows the failure was reproducible enough to warrant a dedicated
    diagnostic.

- **What is NOT in evidence:** no `#[serial]`, no `--test-threads=1` gating, no
  cfg-gated "single Database only" guard, and no source-level comment forbidding
  multiple `Database`s was found. The teardown/serialization theory was explicitly
  **disproven** (commit `4757aa1`). So there is no codified in-tree guard that says
  "N kuzu handles in one process is unsafe" — but equally no test proving it is safe
  on Linux. On macOS (this audit's host, `uname -s` = `Darwin`) the abort has not
  been reproduced; the issue is Linux-specific.

- **Inconclusive by nature:** kuzu is a vendored C++ dependency (`kuzu::{Connection,
  Database, SystemConfig}`, `kuzu_store.rs:16`); its internals (whether it holds
  process-global C++ statics that two `Database` instances share) are **not in this
  tree** and cannot be settled by reading repo source. The one concrete C++-layer
  failure we *do* have is the `create_schema` FFI abort, which is about exception
  translation, not provably about shared statics.

- **The empirical test that would settle it:** on **Linux** (the platform where the
  abort reproduces), in a single process, open **N ≥ 3 `Brain`s on N distinct
  `data_dir`s**, keep all handles alive simultaneously, run `recall_cascade` on each
  repeatedly, then drop them in varying orders. If it completes with no SIGABRT, the
  in-process model is empirically safe for read-mostly fan-out. If it aborts inside
  `create_schema`/`Connection::query` or on drop, the safe model is **N brains across
  N processes** with the coordinator querying over local IPC. This test does not
  exist in the tree today (the existing multi-brain repro step uses the *same* path
  and the whole-suite run, not N distinct-path co-resident handles with recall).

**Bottom line for §5:** The handle model and query surface are clean (§1, §2). The
*only* hard risk is kuzu's C++ FFI behavior on Linux, where a schema-creation abort
is already documented for even a single brain. There is no in-tree proof that N
co-resident `Database`s corrupt each other via statics (that theory was disproven),
but there is also no proof of safety. This must be settled empirically on Linux
before committing to the in-process design.

---

## VERDICT

**v1 is a clean ABOVE-the-trait coordinator on the handle/query/merge axes, modulo
(a) one small provenance touch and (b) an unresolved Linux/kuzu FFI risk that gates
whether the coordinator can be in-process or must be multi-process.**

- **Handle model (§1):** ✅ Per-instance, owned stores, no global/static DB, no path
  collision. N brains on N distinct paths is structurally supported in one process.
- **Query surface (§2):** ✅ All `recall_*` are `&self`; fan-out + merge is ordinary
  code above the API. No global single-brain state blocks it. (Soft note: pass an
  explicit `now` for deterministic cross-brain decay.)
- **Merge inputs (§3):** ⚠️ Returns `Vec<MemoryHit>`; `MemoryHit` has **no**
  `source_brain_id`/provenance. v1 either tags origin coordinator-side using
  `Brain::brain_id()` (zero core change — recommended) **or** adds one optional
  `source_brain_id` field to `MemoryHit` (`spectral-ingest/src/lib.rs:142-178`) and
  stamps it in the recall builders (`brain.rs:1027-1041`, `:1252-1255`). Either way
  the `source_brain_id` tiebreaker must sort on `BrainId::as_bytes()` since `BrainId`
  is not `Ord` (`identity.rs:26-27`).
- **Registry (§4):** ⚠️ None exists. v1 adds a small one from scratch, above the API.
- **Kuzu (§5):** ⛔ The real risk. A Linux SIGABRT in `create_schema` (FFI exception
  not translated) is already documented for a single brain, and a "multi-brain abort
  repro" CI step exists. No in-tree evidence of shared-static corruption across
  co-resident `Database`s (that theory was disproven), and no in-tree proof of
  safety. **Inconclusive from source** because kuzu is a vendored C++ dep.

**Residual risks, enumerated:**
1. **(Blocking-decision) Kuzu in-process N-handle safety on Linux** is unproven.
   Until the empirical Linux test in §5 passes, the conservative model is N brains
   across N processes with the coordinator over local IPC. If it passes, in-process
   fan-out is viable. Either way the *coordinator + merge logic* is unchanged; only
   "open in this process" vs "open in a child process" differs.
2. **(Small core touch — optional) Provenance on the hit.** Needed only if hits are
   serialized away from the handle; one `Option<BrainId>` field + stamp in recall.
3. **(Trivial) `BrainId` lacks `Ord`** — sort the tiebreaker on raw bytes, or add an
   `Ord` derive.
4. **(Correctness note) Time anchor** — coordinator should pass an explicit `now` so
   all N children apply identical recency decay; otherwise each defaults to its own
   `Utc::now()`.

Nothing on the **handle / query / merge** axes forces a more complex design. The
only thing that could force multi-process is the kuzu FFI reality in §5, and that is
an empirical question, not an architectural one — the coordinator code is identical
in both deployments.

---

*Audit only. No build, no fixes, no benchmarks. Claims without a `file:line` cite are
flagged as inconclusive in-text (notably: kuzu C++ internals, which are not in-tree).*
