# Librarian Queue Signal Design (DESIGN)

**Status:** Draft for review. DO NOT IMPLEMENT until approved.
**Date:** 2026-05-10

## Context

Permagent's Librarian scheduler currently implements Behavior A
(warm for full window) with a TODO for Behavior C (unload when
queue empty). Spectral needs to expose a "queue empty" signal so
Permagent knows when the Librarian has no more work and can be
unloaded.

---

## What constitutes a Librarian "job"

A job is a unit of work the Librarian performs on a memory or set
of memories. Three job types, in priority order:

### 1. Description generation (highest priority)

- **Trigger:** Memory has `description IS NULL`
- **Work:** LLM call to generate a prose summary of the memory
- **Completion:** `Brain::set_description(id, text)` writes the
  description and sets `description_generated_at`
- **Existing API:** `Brain::list_undescribed(limit)` returns
  memories needing this work

### 2. Spectrogram backfill

- **Trigger:** Memory has no row in `memory_spectrogram`
- **Work:** Pure computation (no LLM) — 7-dimension cognitive
  analysis
- **Completion:** Spectrogram row written
- **Existing API:** `Brain::backfill_spectrograms()` does a
  batch of 100

### 3. Consolidation (future — not yet implemented)

- **Trigger:** Multiple memories in the same wing/episode with
  high semantic overlap (Jaccard > threshold)
- **Work:** LLM call to merge overlapping memories into one
- **Completion:** Merged memory created, originals marked
  consolidated
- **Existing API:** `spectral-archivist` crate has
  `Consolidator` trait (NoOp default)

---

## Where the queue is stored

**The queue is implicit, not a separate table.** Jobs are derived
from the current state of the memory store:

```sql
-- Description jobs: memories without descriptions
SELECT COUNT(*) FROM memories WHERE description IS NULL;

-- Spectrogram jobs: memories without spectrograms
SELECT m.id FROM memories m
LEFT JOIN memory_spectrogram ms ON m.id = ms.memory_id
WHERE ms.memory_id IS NULL;

-- Consolidation jobs: future (archivist identifies candidates)
```

**Why implicit, not a job queue table:**
- No queue management overhead (enqueue, dequeue, ack, retry)
- Idempotent by construction — re-running a query picks up new
  work and skips completed work
- No stale jobs — if a memory is deleted, its "job" vanishes
- Simpler failure recovery — a crashed Librarian just restarts
  and re-queries

**Tradeoff:** Cannot prioritize individual jobs or track
per-job progress. Acceptable at current scale (<10k memories).

---

## API surface

### Option A: `pending_jobs() -> PendingJobs` (recommended)

```rust
pub struct PendingJobs {
    pub undescribed: usize,
    pub unspectrogrammed: usize,
    pub consolidation_candidates: usize,
}

impl Brain {
    /// Returns the count of pending Librarian jobs by type.
    /// All queries are O(1) index scans — safe to poll frequently.
    pub fn pending_jobs(&self) -> Result<PendingJobs, Error>;

    /// Returns true when all job counts are zero.
    pub fn librarian_queue_empty(&self) -> Result<bool, Error> {
        let pj = self.pending_jobs()?;
        Ok(pj.undescribed == 0
            && pj.unspectrogrammed == 0
            && pj.consolidation_candidates == 0)
    }
}
```

**Pros:**
- Simple, cheap, no state management
- Permagent polls at its natural scheduling interval (~30s)
- Counts let Permagent decide priority and batch size

**Cons:**
- Polling, not push. Wastes a query if nothing changed.

### Option B: `stream_jobs() -> Stream<LibrarianJob>` (rejected)

```rust
pub enum LibrarianJob {
    Describe { memory_id: String },
    Spectrogram { memory_id: String },
    Consolidate { group: Vec<String> },
}
```

**Rejected because:**
- Requires Spectral to maintain a streaming connection
- Adds async complexity to the Brain API
- Consolidation candidates aren't known until a scan runs
- Overkill for a poll-every-30s use case

### Option C: Callback/webhook (rejected)

**Rejected because:**
- Spectral is a library, not a service. No HTTP server to send
  webhooks from.
- Would invert the dependency: Spectral calling Permagent.

---

## How Permagent polls

```
Permagent Librarian scheduler loop:

1. Check Brain::librarian_queue_empty()
2. If empty:
   - Behavior C: unload Librarian, stop polling
   - Resume when: next ingest_turn() call sets a flag
3. If not empty:
   - Get pending_jobs() for counts
   - Process highest-priority type first:
     a. Undescribed: call list_undescribed(batch_size=10),
        generate descriptions via LLM, call set_description()
     b. Unspectrogrammed: call backfill_spectrograms()
        (handles its own batching)
     c. Consolidation: future
   - Sleep 5s, repeat
```

---

## How "queue empty" is determined

`librarian_queue_empty()` returns true when:
- `list_undescribed(1)` returns empty
- `memories_without_spectrogram(1)` returns empty
- consolidation candidates count is 0 (future: always 0 for now)

All three are cheap index-backed queries. Combined cost is <1ms.

---

## Re-triggering after new ingest

When `ingest_turn()` creates a new memory, the queue is implicitly
non-empty (new memory has no description, no spectrogram). But the
Librarian may be unloaded (Behavior C).

**Signal to re-activate Librarian:**

Option 1 (simple): Permagent checks `librarian_queue_empty()` after
each `ingest_turn()`. If false, activate Librarian.

Option 2 (cleaner): `ingest_turn()` returns an `IngestTurnResult`
that includes `librarian_work_pending: bool`. Permagent uses this
to decide whether to wake the Librarian.

**Recommendation:** Option 2. The information is available at
ingest time (we just created an undescribed memory), so returning
it avoids a separate query.

---

## Performance considerations

- `pending_jobs()` is 3 COUNT queries, all index-backed: <1ms
- Safe to call every 5-30 seconds without measurable overhead
- `list_undescribed(10)` is an indexed query with LIMIT: <1ms
- `backfill_spectrograms()` is pure computation: ~0.1ms/memory
- Description generation is LLM-bound (~1-5s per memory)

The Librarian's throughput bottleneck is LLM calls for
descriptions, not Spectral queries.

---

## Open questions

1. **Batch size for descriptions:** 10 per cycle? 50? The LLM
   cost is ~$0.001/description. At 500 memories post-ingest,
   that's $0.50 — negligible. Batch size affects latency of the
   Librarian cycle, not cost.

2. **Priority between new and old memories:** Should the Librarian
   describe the newest memories first (most likely to be recalled
   soon) or oldest first (longest without descriptions)?
   Recommendation: newest first (ORDER BY created_at DESC in
   list_undescribed).

3. **Consolidation trigger:** When should consolidation be
   activated? After a threshold of memories per wing? After a
   time period? This is deferred until the archivist's
   consolidation pass is implemented.
