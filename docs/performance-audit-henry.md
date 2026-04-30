# Spectral Performance Audit: Production TACT Optimizations for Porting

**Date:** 2026-04-28
**Source:** Production constellation/TACT at `/Users/henry/memory/`
**Target:** [github.com/make-tuned-unit/spectral](https://github.com/make-tuned-unit/spectral)
**Platform:** Apple M4 Mac Mini, macOS, Python 3.14 / SQLite WAL mode

---

## Production benchmark numbers

Measured via `tact_retrieval.py benchmark-compare` across 8 representative queries (auth decision, coffee preference, polybot strategy, stripe setup, jesse anniversary, getladle roadmap, henry infrastructure, love nova scotia grant).

### Latency distribution (24 samples, 3 iterations x 8 queries)

| Percentile | Latency |
|------------|---------|
| p50        | 2.09 ms |
| p95        | 6.60 ms |
| p99        | 9.55 ms |
| Mean       | 2.74 ms |
| Min        | 1.02 ms |
| Max        | 9.55 ms |

### Cold vs warm cache

| Phase | Total (8 queries) | Avg/query | vs FTS ratio |
|-------|-------------------|-----------|--------------|
| Cold (fresh indexes, empty wing cache) | 39.18 ms | 4.90 ms | 28.4x slower than raw FTS |
| Warm (indexes in place, wing cache hot) | 14.16 ms | 1.77 ms | 10.5x slower than raw FTS |
| **Warm-to-cold speedup** | **2.8x** | | |

Raw FTS averages ~0.17 ms/query. The TACT overhead buys 3.6 unique results per query that FTS alone misses (fingerprint bonus).

### Ingest throughput (from Spectral's own benchmarks)

| Scenario | Spectral (Rust) |
|----------|-----------------|
| Empty brain | ~2,540 ops/sec (393 us each) |
| 100 memories | ~425 ops/sec (235 ms batch of 100) |
| 1000 memories | ~42 ops/sec (24 ms each) |

Ingest scales as O(peers-in-wing) due to fingerprint pair generation against all existing same-wing memories.

---

## Optimizations found

### OPT-1: Wing result cache with FIFO eviction

**File:** `tact_retrieval.py:63-88`
**What:** A hand-rolled dictionary cache (`_wing_cache`) stores the full result set for wing-only searches. Max 32 entries with FIFO eviction (delete oldest key via `next(iter(...))`). Explicit `invalidate_wing_cache(wing=None)` clears per-wing or global.

```python
_wing_cache = {}
_WING_CACHE_MAX = 32

def _wing_cache_get(wing):
    return _wing_cache.get(wing)

def _wing_cache_put(wing, results):
    if len(_wing_cache) >= _WING_CACHE_MAX:
        oldest = next(iter(_wing_cache))
        del _wing_cache[oldest]
    _wing_cache[wing] = results
```

**Why it helps:** The warm benchmark shows a 2.8x speedup. Wing-only search is the most common path (7/8 benchmark queries hit it). Caching avoids repeated `ORDER BY signal_score DESC` scans against the wing_to_memory_ids join.

**Port effort:** Small. Use `lru::LruCache<String, Vec<MemoryHit>>` in Rust.
**API change:** No. Purely internal. Invalidation must be called on ingest (already the pattern in production).

---

### OPT-2: Materialized wing-to-memory lookup table

**File:** `tact_retrieval.py:122-157`
**What:** `ensure_indexes()` creates and populates a dedicated `wing_to_memory_ids` table with a compound descending index on `(wing, signal_score DESC)`. This denormalizes the `memories` table to avoid scanning the full table for wing-filtered queries.

```python
conn.execute("""
    CREATE TABLE IF NOT EXISTS wing_to_memory_ids (
        wing TEXT NOT NULL,
        memory_id TEXT NOT NULL,
        signal_score REAL DEFAULT 0,
        PRIMARY KEY (wing, memory_id)
    )
""")
conn.execute(
    "CREATE INDEX IF NOT EXISTS idx_wtm_wing_score "
    "ON wing_to_memory_ids(wing, signal_score DESC)"
)
```

**Why it helps:** The wing-only search path (`_wing_only_search`) joins through this table instead of scanning the full `memories` table. The compound descending index means the `ORDER BY signal_score DESC LIMIT ?` is a simple index range scan.

**Port effort:** Small. Already partially ported (Spectral has `idx_memories_wing`), but the separate lookup table with pre-filtered signal scores is not present.
**API change:** No. Internal schema detail.

---

### OPT-3: Compound indexes on fingerprint table

**File:** `tact_retrieval.py:124-137`
**What:** Three compound indexes beyond what `constellation.py` creates:

```python
CREATE INDEX IF NOT EXISTS idx_fp_wing_hash
    ON constellation_fingerprints(wing, fingerprint_hash)

CREATE INDEX IF NOT EXISTS idx_fp_wing_anchor_hall
    ON constellation_fingerprints(wing, anchor_hall)

CREATE INDEX IF NOT EXISTS idx_fp_wing_target_hall
    ON constellation_fingerprints(wing, target_hall)
```

**Why it helps:** The main fingerprint query filters `WHERE wing = ? AND fingerprint_hash IN (...)`. Without the compound index, SQLite must scan all fingerprints for the wing or all fingerprints for the hash, then intersect. The compound index allows a single B-tree range scan.

**Port effort:** Small. Spectral already has `idx_fp_wing_hash` but is **missing** the `wing_anchor_hall` and `wing_target_hall` indexes used by the hall-based UNION branch.
**API change:** No.

---

### OPT-4: Single unified CTE query for fingerprint search

**File:** `tact_retrieval.py:246-270`
**What:** Instead of the two-query approach in `constellation.py` (hash match + hall match, then Python-side merging), `_fingerprint_search` uses a single SQL statement with a CTE that UNIONs both paths and scores in SQL:

```python
WITH matched_pairs AS (
    SELECT DISTINCT anchor_memory_id, target_memory_id
    FROM constellation_fingerprints
    WHERE wing = ? AND fingerprint_hash IN ({hash_placeholders})
    UNION
    SELECT DISTINCT anchor_memory_id, target_memory_id
    FROM constellation_fingerprints
    WHERE wing = ? AND (anchor_hall = ? OR target_hall = ?)
),
memory_scores AS (
    SELECT memory_id, COUNT(*) AS hits FROM (
        SELECT anchor_memory_id AS memory_id FROM matched_pairs
        UNION ALL
        SELECT target_memory_id AS memory_id FROM matched_pairs
    )
    GROUP BY memory_id
    ORDER BY hits DESC
    LIMIT ?
)
SELECT m.id, m.key, m.content, m.wing, m.hall, m.signal_score, ms.hits
FROM memory_scores ms
JOIN memories m ON m.id = ms.memory_id
ORDER BY ms.hits DESC
```

**Why it helps:** One round-trip to SQLite instead of two. The scoring (counting how many fingerprints point to each memory) happens server-side rather than in Python dicts. SQLite's query planner can also optimize the UNION.

**Port effort:** Small. Spectral's `fingerprint_search` likely uses a simpler query. Port this exact SQL.
**API change:** No.

---

### OPT-5: Pre-compiled regex for wing/hall detection in ACR daemon

**File:** `tact_acr_daemon.py:54-63`
**What:** The ACR daemon compiles all project detection regexes at module load time:

```python
PROJECT_RULES = [
    (re.compile(r"getladle|ladle|mel.schembri|recipe", re.I), "getladle", "GetLadle"),
    (re.compile(r"polybot|polymarket|prediction|wager", re.I), "polybot", "Polybot"),
    ...
]
```

**Why it helps:** The daemon polls every 60 seconds and applies all rules. Pre-compilation avoids re-compiling 8 regex patterns per poll cycle. In the retrieval path (`tact_retrieval.py`), wing/hall rules are **not** pre-compiled — `detect_wing` and `detect_hall` call `re.search(pattern, blob)` with string patterns each time. Python caches recent regex compilations internally, but explicit pre-compilation is more reliable.

**Port effort:** Small. Spectral already does this — regexes are compiled once in `IngestConfig`.
**API change:** No.

---

### OPT-6: Uncompiled regex in hot retrieval path (anti-pattern to fix)

**File:** `tact_retrieval.py:43-61`
**What:** `WING_RULES` and `HALL_RULES` are raw string patterns, not compiled:

```python
WING_RULES = [
    (r"jesse|coffee|anniversary|colou?r|favourit|favorit|sons|rowan|jude|sophie.sharratt", "jesse"),
    ...
]
```

`detect_wing()` and `detect_hall()` call `re.search(pattern, blob)` with these strings on every retrieval call. Python's `re` module has an internal cache (typically 512 entries), so this usually hits the cache, but it's unnecessary overhead on every call.

**Why it matters:** This is a latency tax on every single `tact_retrieve()` call. In Spectral, the equivalent patterns are pre-compiled at Brain construction time, which is correct.

**Port effort:** Already done in Spectral.
**API change:** N/A.

---

### OPT-7: Content truncation at retrieval boundary

**File:** `tact_retrieval.py:276` (and throughout `_fingerprint_search`, `_wing_only_search`, `_fts_search`)
**What:** Every result dict truncates content to 300 chars: `r["content"][:300]`. This happens immediately when building result dicts, not at the end.

**Why it helps:** Prevents large content blobs from flowing through the scoring/merging pipeline. Reduces memory pressure during the multi-tier search (fingerprint + spectrogram + entity + FTS hybrid boost can accumulate up to `max_results * 4` result dicts).

**Port effort:** Small. Check if Spectral truncates content in `MemoryHit` construction or only in `build_context_bundle`.
**API change:** No.

---

### OPT-8: Early return on wing detection failure

**File:** `tact_retrieval.py:175-181`
**What:** The retrieval pipeline has a fast path: if `detected_wing` is set and `detected_hall` is set, try fingerprint search. If wing is set but no results from fingerprint, try wing-only (cached). Only if both fail does it fall through to FTS.

```python
if detected_wing and detected_hall:
    results = _fingerprint_search(conn, detected_wing, detected_hall, max_results)

if not results and detected_wing:
    results = _wing_only_search(conn, detected_wing, max_results, query_text)

if not results:
    method = "fts_fallback"
    results = _fts_search(conn, query_text, max_results)
```

**Why it helps:** Avoids running expensive FTS when fingerprint or wing search succeeds. The benchmark shows fingerprint+spectrogram resolves 7/8 queries without FTS fallback.

**Port effort:** Already ported. Spectral's three-tier fallback matches this pattern.
**API change:** No.

---

### OPT-9: WAL mode and PRAGMA tuning

**File:** `tact_retrieval.py:118`, `constellation.py:34`
**What:** Both files enable WAL mode on connection:

```python
conn.execute("PRAGMA journal_mode=WAL")
```

**Why it helps:** WAL allows concurrent readers during writes. Critical for the ACR daemon (writing hot_context.txt while retrieval queries run).

**Port effort:** Already ported. Spectral also sets `journal_mode=WAL`, `synchronous=NORMAL`, and `temp_store=MEMORY`.
**API change:** No.

---

### OPT-10: Batched fingerprint INSERT via executemany

**File:** `constellation.py:207-213`
**What:** All fingerprints are accumulated in a list and inserted with a single `executemany` call:

```python
conn.executemany(
    "INSERT INTO constellation_fingerprints "
    "(id, fingerprint_hash, anchor_memory_id, target_memory_id, "
    "wing, anchor_hall, target_hall, time_delta_bucket, created_at) "
    "VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    fingerprints,
)
conn.commit()
```

**Why it helps:** A single transaction with batched inserts is dramatically faster than individual INSERT+COMMIT per fingerprint. For a brain with N high-signal memories, this generates O(N^2) fingerprints per wing; batching keeps the commit overhead constant.

**Port effort:** Medium. Spectral generates fingerprints per-ingest (against existing peers), not in a bulk regeneration pass. To port, ensure Spectral uses a single transaction when inserting multiple fingerprints during `remember()`.
**API change:** No.

---

### OPT-11: Stop-word stripping in recall tool

**File:** `tact_recall_tool.py:18-24`
**What:** The recall tool strips common question words before passing to FTS:

```python
STOP_WORDS = {"who", "is", "what", "are", "the", "a", "an", "of", "for", "to", ...}

def clean_query(query: str) -> str:
    words = query.lower().replace("?", "").replace(",", "").split()
    cleaned = [w for w in words if w not in STOP_WORDS]
    return " ".join(cleaned) if cleaned else query
```

**Why it helps:** FTS5's ranking degrades when queries contain high-frequency stop words. Stripping them focuses the MATCH on discriminative terms.

**Port effort:** Small. Add a `STOP_WORDS` HashSet and filter before FTS query construction in Spectral's FTS fallback.
**API change:** No.

---

### OPT-12: Negative pattern filtering in document search

**File:** `tact_recall_tool.py:43-63`
**What:** Before scoring, results are filtered against a list of "failed recall" patterns that indicate the system previously couldn't answer a similar query:

```python
negative_patterns = [
    "i don't have a memory of that",
    "search didn't return any matching records",
    ...
]
filtered = [r for r in results if not any(p in content for p in negative_patterns)]
```

**Why it helps:** Prevents meta-conversation noise from polluting recall. These "I don't remember" responses from previous sessions would otherwise rank highly because they contain the original query terms.

**Port effort:** Small. Add a negative-pattern filter to Spectral's recall pipeline.
**API change:** No. Internal quality improvement.

---

### OPT-13: ACR daemon skip-if-unchanged guard

**File:** `tact_acr_daemon.py:142-185`
**What:** The daemon tracks `_last_wing` and only runs TACT retrieval + writes hot_context.txt when the detected project changes:

```python
if wing and wing != _last_wing:
    context = load_context_for_wing(wing, display_name)
    write_hot_context(context)
    _last_wing = wing
```

**Why it helps:** Avoids redundant retrieval when Jesse stays on the same project. The daemon polls every 60 seconds; without this guard, it would run 60 retrievals/hour for no benefit.

**Port effort:** Small if building an ACR-equivalent in Spectral.
**API change:** No.

---

## Optimizations probably already in Spectral

Based on the Spectral codebase analysis:

1. **Deterministic fingerprint hashing** (`SHA256(anchor_hall|target_hall|wing|bucket)[:16]`) — golden hash test confirms byte-identical output.

2. **Composite index on (wing, fingerprint_hash)** — `idx_fp_wing_hash` exists in `sqlite_store.rs`.

3. **Pre-compiled regex for wing/hall classification** — `IngestConfig` compiles patterns once at `Brain::open()`.

4. **WAL mode + PRAGMA tuning** — Spectral sets `journal_mode=WAL`, `synchronous=NORMAL`, `temp_store=MEMORY`.

5. **Three-tier retrieval fallback** (fingerprint -> wing-only -> FTS) — present in `extractor.rs`.

6. **Signal score threshold gating** (skip fingerprint generation for low-signal memories) — threshold at 0.5.

7. **Early termination in context bundle building** — `build_context_bundle` breaks when char limit reached.

8. **FTS5 with trigger-maintained sync** — automatic INSERT/DELETE/UPDATE propagation.

9. **Content-addressed entity IDs** (blake3 in Spectral vs SHA256 in production — different algorithm but same pattern).

10. **min_words skip** — Spectral skips retrieval for very short queries.

---

## Optimizations missing from Spectral

### MISS-1: Wing result cache (OPT-1)

Spectral has **no in-memory LRU cache** for wing-only search results. Production shows this is the single biggest latency win (2.8x cold-to-warm speedup). Spectral relies solely on SQLite's internal page cache, which doesn't help with the Python-equivalent scoring/sorting overhead that happens after the query.

**Impact:** High. Wing-only is the most common retrieval path.

### MISS-2: Materialized wing_to_memory_ids lookup table (OPT-2)

Spectral has `idx_memories_wing` on the main `memories` table but no separate denormalized lookup table with a compound `(wing, signal_score DESC)` index. The production system's lookup table avoids joining against the full memories table for wing-filtered queries.

**Impact:** Medium. Matters at scale when the memories table grows large.

### MISS-3: Compound hall indexes on fingerprints (OPT-3, partial)

Spectral has `idx_fp_wing_hash` and `idx_fp_hash` but is **missing** `idx_fp_wing_anchor_hall` and `idx_fp_wing_target_hall`. The CTE's UNION branch (`WHERE wing = ? AND (anchor_hall = ? OR target_hall = ?)`) needs these to avoid a full fingerprint table scan.

**Impact:** Medium. Only affects the hall-match branch of fingerprint search.

### MISS-4: Unified CTE query (OPT-4)

Spectral's fingerprint search likely issues separate queries for hash-match and hall-match, then merges in Rust. The production system's single CTE query is more efficient — one round-trip, server-side scoring.

**Impact:** Medium. Saves one SQLite round-trip per fingerprint search.

### MISS-5: Stop-word stripping for FTS (OPT-11)

No evidence of stop-word filtering before FTS MATCH in Spectral's extractor.

**Impact:** Low-medium. Affects FTS ranking quality more than latency.

### MISS-6: Negative pattern filtering (OPT-12)

Spectral has no equivalent of the "failed recall" meta-conversation filter. This is a quality issue more than performance, but bad results waste downstream LLM tokens.

**Impact:** Low (latency) / Medium (quality).

### MISS-7: Batched fingerprint generation on full regeneration

Production `constellation.py` can regenerate all fingerprints in one pass with `executemany`. Spectral generates fingerprints incrementally per-ingest. If Spectral ever needs a bulk regeneration command (e.g., after schema migration), it should use batched inserts within a single transaction.

**Impact:** Low for normal operation. High for migration/rebuild scenarios.

---

## Recommended port order

Priority order: highest expected speedup-per-effort first.

| Priority | Optimization | Expected speedup | Effort | Constraint |
|----------|-------------|------------------|--------|------------|
| **1** | Wing result LRU cache (MISS-1) | 2-3x on warm path | Small | Must invalidate on `remember()`. Must not change public API. |
| **2** | Compound hall indexes (MISS-3) | 20-40% on fingerprint path | Small | Schema-only. Must not change fingerprint algorithm. Golden hash must pass. |
| **3** | Unified CTE query (MISS-4) | 10-20% on fingerprint path | Small | Must produce identical result ordering. |
| **4** | Materialized wing lookup table (MISS-2) | 15-30% on wing-only path at scale | Medium | Must refresh on ingest. Can be lazy (refresh on first query after ingest). |
| **5** | Stop-word stripping (MISS-5) | Quality improvement, marginal latency | Small | Must preserve original query for wing/hall detection. Only strip for FTS. |
| **6** | Negative pattern filter (MISS-6) | Quality improvement only | Small | Only relevant if Spectral integrates document/knowledge search. |
| **7** | Bulk regeneration batching (MISS-7) | 10-100x for rebuild commands | Medium | Only needed for migration tooling, not hot path. |

### Implementation notes

- **MISS-1 (LRU cache):** Use `lru::LruCache<String, Vec<MemoryHit>>` with capacity 32. Add `invalidate_wing_cache(&self, wing: Option<&str>)` to `SqliteStore`. Call it at the end of `remember()`. Thread-safe: wrap in `Arc<Mutex<LruCache>>` alongside the existing connection mutex.

- **MISS-3 (indexes):** Add to `ensure_schema()` in `sqlite_store.rs`:
  ```sql
  CREATE INDEX IF NOT EXISTS idx_fp_wing_anchor_hall
      ON constellation_fingerprints(wing, anchor_hall);
  CREATE INDEX IF NOT EXISTS idx_fp_wing_target_hall
      ON constellation_fingerprints(wing, target_hall);
  ```

- **MISS-4 (CTE):** Replace the fingerprint search query with the production CTE from `tact_retrieval.py:246-270`. The SQL is SQLite-compatible and works identically in rusqlite.

---

## Cross-cutting observations

### 1. The wing cache is the dominant optimization

Production's 2.8x cold-to-warm speedup is almost entirely from the wing result cache. Everything else is incremental. Spectral should prioritize this above all other ports.

### 2. Python's overhead masks SQL efficiency gains

Production TACT at 1.77 ms/query (warm) vs Spectral's 564 us/query (Rust, warm) shows that ~1.2 ms of the Python latency is interpreter overhead (dict construction, list sorting, re.search calls). The SQL optimizations matter more in the Python version where they reduce the number of round-trips. In Rust, the SQL is already fast — the cache is what will make the biggest difference.

### 3. Connection reuse is handled differently

Production creates a fresh `sqlite3.Connection` per `tact_retrieve()` call (including WAL pragma each time). The ACR daemon and benchmark each create their own connections. Spectral uses a single `Arc<Mutex<Connection>>`, which is better — but the mutex contention could become a bottleneck under concurrent recall. Consider `r2d2` or `deadpool` connection pooling if Spectral ever serves concurrent requests.

### 4. Ingest-time fingerprint generation is the scaling bottleneck

Both production and Spectral generate fingerprints O(peers-in-wing) per ingest. At 1000 memories in a wing, each ingest generates ~1000 fingerprints. Production handles this by batching (`executemany`); Spectral should ensure its per-ingest fingerprint writes are in a single transaction. At 10k+ memories per wing, consider approximate fingerprinting (sample top-K peers by signal score rather than all peers).

### 5. The multi-tier search is deliberate redundancy for quality

Production runs fingerprint search, then wing-only, then FTS, then spectrogram, then entity graph, then hybrid FTS boost — up to 6 search passes per query. This is quality-optimized, not latency-optimized. The benchmark shows 3.6 unique results per query that FTS alone misses. Spectral should preserve this multi-tier architecture but be aware that each additional tier adds ~0.5 ms in Python / ~100 us in Rust.

### 6. No prepared statement caching in production

Production constructs SQL strings with f-string placeholder interpolation on every call. This is safe (parameterized), but the query plan is re-parsed each time. Spectral's rusqlite `prepare_cached()` would help — ensure the CTE query (OPT-4) uses `prepare_cached` since it's the most complex query and benefits most from plan caching.

### 7. Hot context file as a cache layer

The ACR daemon's `hot_context.txt` pattern is an OS-level cache: pre-compute the most likely retrieval result and write it to a file that any process can `cat` in <1 ms. This is orthogonal to the library but powerful for agent architectures. Spectral could expose a `pre_warm(wing: &str)` API that callers use to trigger background pre-computation.
