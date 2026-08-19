# Spectral — Deletion Guarantees

**Headline: `forget` is verified, not assumed.** Deletion returns a
per-substrate receipt, re-probes recall and recognition for the deleted
content, and is enforced by a schema-derived sweep that fails automatically
if a future substrate is added without being wired into `forget`. Physical
byte erasure is a separate, explicit step (`Brain::vacuum`) with its own
byte-scan test.

Every claim below maps to a test in
[`crates/spectral-graph/tests/deletion_guarantees.rs`](../crates/spectral-graph/tests/deletion_guarantees.rs)
via [`deletion-guarantees-inventory.json`](deletion-guarantees-inventory.json),
and a gate test
([`deletion_claims_gate.rs`](../crates/spectral-graph/tests/deletion_claims_gate.rs))
fails the build if a claim in this document loses its test. The suite was
pre-registered before it was written:
[`internal/deletion-guarantees-prereg-2026-07-29.md`](internal/deletion-guarantees-prereg-2026-07-29.md).

Why this matters: no published agent-memory system we surveyed proves
deletion, and embedding-based stores structurally cannot (deleted content
persists in index geometry). SQLite substrates can — so we prove it.

---

## D1 — Completeness (schema-derived, not sampled)

**Claim.** `forget(key)` removes every row referencing the memory — its id,
its key, or its content — from **every table in every database file** the
brain owns (`memory.db`, `recognition.db`, `graph.sqlite`).

**How it is enforced.** The test does not check a hand-maintained list. It
enumerates all tables from `sqlite_master` at test time, seeds a memory whose
content carries a unique sentinel token, verifies the sweep *finds* the
sentinel/id in the expected substrates before deletion (memories, FTS,
spectrogram, annotations, sessions, consolidation edges, co-retrieval pairs,
retrieval events, recognition enrollment), forgets, and asserts **zero
matching rows in every enumerated table**. A new substrate added later that
is not wired into `forget` enters the assertion set automatically and fails
the suite by construction.

**Documented allowlist** (the only tables exempt, each named in the test with
its justification):

- `sync_tombstones`, `replicated_set_tombstones` — federation retraction
  markers. A tombstone must outlive the object it retracts (that is its
  function; see D5). They carry object hashes only, never content.
- FTS5 shadow tables (`memories_fts_{data,idx,docsize,config}` and, when the
  fusion index is enabled, `memories_fts_raw_*`) — physical segment storage
  of the index. Their *logical* view is swept via `MATCH` (zero results
  required, unjoined — so dangling index entries cannot hide behind the
  recall path's JOIN); physically-dead bytes inside segments are exactly the
  D4 boundary below.

**Finding (fixed on this branch).** The sweep caught a real leak on main:
`episodes.summary_preview` stores a verbatim 200-character prefix of a member
memory's content and survived `forget`. `delete_memory_by_key` now scrubs any
episode preview derived from the deleted content, drops the episode row when
the deleted memory was its last member, and reports the work in a new
`ForgetReceipt::episodes` field.

## D2 — Verification, not assumption

**Claim.** The `ForgetReport` probes (`recall_verification`,
`recognition_verification`) are load-bearing: they detect residue when it
exists, and the report is fail-closed — `fully_forgotten()` is `false`
whenever a probe finds residue (`ResidualFound`) **or cannot answer**
(`VerificationFailed`), never treating a broken probe as success.

**How it is enforced.** After a clean forget, the test re-inserts residue
rows by raw SQL — once into the primary store, once into the recognition
sidecar — simulating a substrate whose delete silently failed, and asserts
the probes `forget` runs detect the residue; a forget over the sabotaged
state never reports `fully_forgotten`.

The recognition probe reports `ResidualFound` if **either** an unguarded
`recognize` returns a verdict naming the deleted id **or**
[`Brain::recognition_residue`](../crates/spectral-graph/src/brain.rs) finds it
still enrolled. Both halves are load-bearing and neither subsumes the other:
a verdict cannot see residue too weak to still win `Recognized`, and the
substrate check cannot see residue whose `enrolled` row was removed while
fingerprints survived.

*Why "unguarded" is named here.* The public `recognize` deliberately withholds
identity for a memory whose row is gone — a verdict a consumer cannot resolve
is worse than a weaker one (2026-08-19). Routing this probe through that guard
would have made it report `VerifiedClear` for exactly the state it exists to
catch: a right-to-be-forgotten guarantee passing by construction. The D2 test
caught that when the guard was first written, which is the reason the two paths
are now distinct — **serveability** (what a consumer may be told) and
**detectability** (what an auditor can find) are different questions, and only
the second is a deletion guarantee.

**Boundary, stated exactly.** `ResidualFound` inside a *single* `forget()`
call requires a substrate delete to fail mid-call; there is no fault-injection
seam in the store, so the test proves probe sensitivity by re-running the
probes over injected residue rather than by faulting the transaction.

## D3 — Residue resistance (adversarial side doors)

**Claim.** After `forget`, the deleted content is unreachable through every
side door we could construct:

- **(a) FTS phrase and prefix search** on distinctive deleted tokens returns
  nothing — through the public recall API *and* through raw unjoined
  `MATCH` against the FTS index.
- **(b) Recognition** of the deleted content — verbatim *and* a ~30%
  token-dropped copy (the re-encounter condition) — yields no `Recognized`
  verdict naming it, no candidate trace, and no evidence row citing its
  features. (Both probes are shown to name the memory *before* deletion, so
  their silence afterward is meaningful.)
- **(c) Graph substrate** (`entity`/`triple`/`mention`/`document`) holds no
  row referencing the memory. Scope, stated exactly: `remember()` never
  mints graph rows — triples and aliases come only from `assert()`/`ingest_*`
  and are keyed by entity/document, not memory key. The sweep proves this
  scope holds before and after forget, over a populated graph.
- **(d) Associative paths seeded from a neighbor** — co-retrieval
  (`related_memories`), lift-ranked recommendation (`recommend`), and
  episode-based associative spreading — do not return the forgotten memory,
  and rebuilding the co-retrieval index from the (scrubbed) retrieval-event
  log cannot re-derive the association.

## D4 — Physical residue boundary (the honest hard part)

**Claim, with its exact boundary.**

- **Logical unreachability is immediate**: after `forget` returns, no query,
  probe, index, or associative path returns the memory (D1–D3).
- **Physical erasure is not immediate**: SQLite retains logically-deleted
  bytes in FTS5 segment b-trees, WAL frames, and free pages. The D4 test
  *asserts* the sentinel bytes are still present in the raw files after
  `forget` alone — that persistence is expected, documented behavior, not a
  bug, and it proves the byte-scan itself works.
- **`Brain::vacuum()` completes physical erasure.** This API was the
  pre-registered gap (the prereg predicted no vacuum path existed; it
  didn't) and was added for this suite: FTS `'optimize'` + truncating WAL
  checkpoint + `VACUUM` + a second checkpoint (in WAL mode, `VACUUM` writes
  the rebuilt image through the WAL — without the final checkpoint the old
  bytes survive in the main file; the byte-scan caught this too) across
  `memory.db`, `recognition.db` (whose pair/gram feature labels quote
  verbatim content fragments), and `graph.sqlite`.

After `forget` + `vacuum`, a byte-scan of every database file and WAL finds
no trace of the deleted content, and other memories are intact.

**Not covered** (out of scope, stated so no one infers otherwise): filesystem
and hardware-level remanence — old bytes in unallocated disk sectors after
file rewrite, SSD wear-leveling copies, OS page cache, and backups. That
layer belongs to full-disk encryption and device policy, not a library.

## D5 — Federation scope (tombstones)

**Claim, scoped exactly.**

- **Retraction propagates and dominates.** `federation_sync::tombstone`
  removes a shared object from the wing manifest, hard-deletes replicated
  copies on peers as the tombstone syncs, and blocks resurrection through
  every subsequent have/want round — including a stale peer re-shipping the
  original pre-retraction pack.
- **The author's native copy is not deleted by the tombstone.** Retraction
  un-shares; it does not reach into the author's private store. Full erasure
  for the author = `tombstone` (federation) + `forget` (local). The test
  pins this boundary so it cannot silently change.
- **Plain `forget` is single-brain.** It writes no tombstone. Observed and
  pinned: a locally-forgotten replicated copy does *not* resurface when the
  peer re-delivers the original pack (the local manifest entry survives and
  import dedups against it) — but this is manifest bookkeeping, not
  retraction: the peer still holds and re-advertises the object. Anyone
  needing federation-wide deletion must use `tombstone`.

---

## Reproduce

```sh
# The proof suite (D1–D5, 9 tests):
cargo test -p spectral-graph --test deletion_guarantees

# The claims gate (this document ↔ inventory ↔ test source):
cargo test -p spectral-graph --test deletion_claims_gate

# Everything around it (store receipts, federation, recognition):
cargo test -p spectral-graph -p spectral-ingest
```

The tests run against throwaway `TempDir` brains; no fixtures, no network,
no LLM. The pre-registration (claims, expectations, and decision rules,
committed before the tests were written) is at
[`internal/deletion-guarantees-prereg-2026-07-29.md`](internal/deletion-guarantees-prereg-2026-07-29.md).

## Outcomes vs pre-registration

| Claim | Pre-registered expectation | Outcome |
|-------|---------------------------|---------|
| D1 | pass; failures are findings to fix | **FIXED** — `episodes.summary_preview` retained deleted content; now scrubbed + receipted |
| D2 | pass | **PASS** — probes detect injected residue; report is fail-closed |
| D3 (a–c) | pass | **PASS** |
| D3 (d) | unknown (never deletion-tested) | **PASS** — including index-rebuild resistance |
| D4 | partial; vacuum API predicted missing | **FIXED** — `Brain::vacuum` added; also required a post-`VACUUM` WAL checkpoint the byte-scan caught |
| D5 | pass for tombstone non-resurrection | **PASS**, with two pinned boundaries: author's native copy survives tombstone; plain forget is single-brain |
