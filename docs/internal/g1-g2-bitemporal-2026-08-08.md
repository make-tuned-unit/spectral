# G1 + G2 — bi-temporal validity, and the resolver finally has a sink

**2026-08-08. $0, no model calls, no benchmark.** Shipped for **auditability**.
**No accuracy claim is made and none is implied** — see "Why there is no
number" below.

## The two halves only work together

**G1** — `memories` carried `created_at`/`updated_at` only. Those are *system*
time: when we learned something. Nothing recorded *valid* time: when it was
true in the world. `render.rs` therefore dates a memory by its ingestion,
conflating the two.

**G2** — `spectral::resolve_relative_dates` turns "yesterday" plus an anchor
into a date, deterministically, with no model and no clock read. It shipped
exported, with 11 tests, and **wired to nothing**.

Those facts are the same fact. The resolver had no honest sink: writing a
resolved world-date into `created_at` would have been a lie about when we
learned the fact. Adding valid-time columns gives it somewhere to go.

## What shipped

Schema (idempotent migration, `sqlite_store.rs`):

- `valid_from TEXT DEFAULT NULL` — when the fact became true.
- `valid_to TEXT NOT NULL DEFAULT '9999-12-31T23:59:59.999Z'`.

**The sentinel is load-bearing, not cosmetic.** `NULL` would force
`valid_to IS NULL OR valid_to > ?`, which cannot use an index. The sentinel
keeps every validity check a plain range comparison.

Half-open `[from, to)`, so adjacent versions of a fact never both match at the
boundary instant — pinned by `the_window_is_half_open`.

API:

- `invalidate_at(key, valid_to)` — **never deletes**. The row stays queryable,
  which is the entire point.
- `keys_as_of(instant)` — `COALESCE(valid_from, created_at) <= t < valid_to`.
- `set_valid_from(key, t)` — the sink for G2.
- `validity_of(key)`.

## Two implementation findings worth keeping

**1. A backfill `UPDATE` during migration is a trap.** The first version wrote
`UPDATE memories SET valid_from = created_at`. That fires the FTS sync
triggers, and on a database upgrading from an older schema those triggers
reference FTS columns that do not exist yet — so the migration failed on
exactly the legacy databases it exists to upgrade. Caught by
`schema_migration_adds_columns_idempotent`, which is a test that earns its
keep.

The fix removes the write entirely: `valid_from IS NULL` means *unrecorded*,
and the predicate reads `COALESCE(valid_from, created_at)`. Legacy rows behave
as valid from ingestion — a read-time approximation we can state, rather than a
write we would afterwards have to trust.

**2. The first test premise was wrong, not the code.** Tests wrote memories
with a default `created_at` of *now*, then asked what was visible "last May",
and failed. That behaviour is correct: a memory ingested today was genuinely
not in the brain in May. The tests now record the facts in January so the
question is coherent. Recording this because the tempting fix — loosening the
predicate until the test passed — would have destroyed the property being
tested.

## Why there is no accuracy number

**No public benchmark tests bi-temporal modelling at all.** That is why Zep's
claim on it is currently unfalsifiable, and we are not entitled to a better
epistemic position than they are just because we shipped it too.

What is asserted is a **property**, and it is asserted by test:
`a_past_answer_stays_reproducible_after_the_fact_changes` — a fact invalidated
on 2026-06-01 remains visible to a May query and invisible to a July one,
stably across repeated calls, with unrelated facts unaffected.

That property is what makes an eval against a mutating store meaningful. Absent
it, a run against a store that has since changed is unreproducible *in
principle* — you cannot even state what it measured. Given the day's other
findings, that is not an abstract concern.

## Scope — what was NOT done

- **`as_of` is not surfaced through `Brain`.** `Brain::memory_store` is a
  private `Arc<dyn MemoryStore>`, so reaching it needs methods on the trait —
  a default-path API change across every store implementation. Deliberately
  deferred rather than rushed.
- **The resolver is not wired into the ingest path.** The join is demonstrated
  end-to-end in `bitemporal_valid_time.rs`, but resolving at ingest by default
  changes what every memory's valid-time *is*, which is a behaviour change on
  the default write path and needs a prereg.
- **`RenderOptions::relative_offsets` remains untested for accuracy.** It is
  built, off by default, and sits on the axis that already paid (R11's +14.2pp
  came from bare dates alone). Testing it needs a paid actor A/B and therefore
  its own prereg. Recorded as the highest-value untested rendering lever.
- No `t_expired` (system-time close). Full four-timestamp modelling is a
  superset of this; two columns buy the audit property, four buy retroactive
  correction history. Not needed yet.

## Tests

10 total, all $0 and offline. 6 store-level (`bitemporal_tests`) and 4
integration (`crates/spectral/tests/bitemporal_valid_time.rs`), covering:
resolver determinism, resolved-valid-time driving `as_of` visibility,
invalidation-never-deletes, the half-open boundary, sentinel behaviour,
pre-migration row visibility, repeat-call stability, and missing-key reporting.

Workspace suite green, clippy clean.

**Refs:** `landscape-research-2026-08-07.md` §G1/§G2, TOKI (arXiv 2606.06240),
Zep/Graphiti, `temporal.rs`, `REPAIR_REGISTER.md`.
