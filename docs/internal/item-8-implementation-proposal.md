# Item #8: Compiled-Truth Boost via Description-Enriched FTS — Implementation Proposal

**Date**: 2026-05-13
**Branch**: `feat/item-8-compiled-truth-boost`
**Status**: Proposal — awaiting review before implementation.

---

## Section 1 — Current State Inventory

### Description column on memories table

The `description` column exists on the `memories` table (`sqlite_store.rs:372-373`), added via ALTER TABLE migration. The `set_description()` method (`sqlite_store.rs:1647-1668`) writes descriptions and timestamps. The `list_undescribed()` method (`sqlite_store.rs:1670-1689`) finds memories without descriptions. The `MemoryHit` struct includes `description: Option<String>` (`lib.rs:177`).

**All plumbing exists for reading/writing descriptions. The gap is FTS indexing.**

### FTS5 virtual table schema

The FTS5 virtual table indexes **two columns only** (`sqlite_store.rs:166-168`):

```sql
CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
    key, content, content=memories, content_rowid=rowid
);
```

Three triggers keep FTS in sync (`sqlite_store.rs:170-183`):
- `memories_ai` (AFTER INSERT): inserts `(rowid, key, content)` to FTS
- `memories_ad` (AFTER DELETE): deletes from FTS
- `memories_au` (AFTER UPDATE): deletes old + inserts new `(key, content)`

**The `description` column is NOT in the FTS5 table, NOT in any trigger, and NOT searchable by FTS queries.**

### FTS search path

`fts_search()` (`sqlite_store.rs:868-904`):
1. OR-joins quoted query words
2. Runs `SELECT ... FROM memories_fts WHERE memories_fts MATCH ?1 ORDER BY rank`
3. Joins back to `memories` table for full columns

Both retrieval paths use this:
- `recall_topk_fts` (`brain.rs:1103-1172`): sanitizes query → `fts_search()` → re-ranking pipeline
- `cascade_retrieve` (`brain.rs:1043-1095`): TACT first, then `fts_search_direct()` → `fts_search()` to fill up to K

### Re-ranking pipeline

`apply_reranking_pipeline()` (`ranking.rs:282-400`) applies signals additively/multiplicatively to FTS rank position scores. Current signals: signal_score blending, ambient boost, declarative density, co-retrieval, recency decay, entity boost, episode diversity, context dedup.

`RerankingConfig` (`ranking.rs:237-272`) controls which signals are active. No description-specific signal exists.

### What's missing

1. `description` not in FTS5 virtual table → descriptions are invisible to search
2. No FTS trigger fires when `set_description()` updates the description column
3. No description-aware ranking signal

---

## Section 2 — Schema Migration Design

### Approach: Drop and recreate FTS5 table

FTS5 virtual tables do **NOT** support `ALTER TABLE ... ADD COLUMN`. The only way to add a column to an FTS5 table is to drop it and recreate it. SQLite FTS5 with `content=memories` (external content mode) doesn't store its own copy of the data — it stores only the index. Dropping the FTS table loses the index but no data.

**Migration steps:**

```sql
-- 1. Drop existing FTS table and triggers
DROP TRIGGER IF EXISTS memories_ai;
DROP TRIGGER IF EXISTS memories_ad;
DROP TRIGGER IF EXISTS memories_au;
DROP TABLE IF EXISTS memories_fts;

-- 2. Create new FTS table with description column
CREATE VIRTUAL TABLE memories_fts USING fts5(
    key, content, description,
    content=memories, content_rowid=rowid
);

-- 3. Recreate triggers with description column
CREATE TRIGGER memories_ai AFTER INSERT ON memories BEGIN
    INSERT INTO memories_fts(rowid, key, content, description)
    VALUES (new.rowid, new.key, new.content, COALESCE(new.description, ''));
END;

CREATE TRIGGER memories_ad AFTER DELETE ON memories BEGIN
    INSERT INTO memories_fts(memories_fts, rowid, key, content, description)
    VALUES ('delete', old.rowid, old.key, old.content, COALESCE(old.description, ''));
END;

CREATE TRIGGER memories_au AFTER UPDATE ON memories BEGIN
    INSERT INTO memories_fts(memories_fts, rowid, key, content, description)
    VALUES ('delete', old.rowid, old.key, old.content, COALESCE(old.description, ''));
    INSERT INTO memories_fts(rowid, key, content, description)
    VALUES (new.rowid, new.key, new.content, COALESCE(new.description, ''));
END;

-- 4. Rebuild FTS index from existing memories
INSERT INTO memories_fts(rowid, key, content, description)
SELECT rowid, key, content, COALESCE(description, '') FROM memories;
```

### Backward compatibility

- **Memories without descriptions**: `COALESCE(description, '')` ensures NULL descriptions become empty strings in FTS. Empty strings don't contribute to BM25 scoring. These memories remain searchable via key + content as before.
- **Existing databases (Permagent, bench)**: The migration runs at `SqliteStore::open()`, same as all other migrations. The FTS index is rebuilt from the base `memories` table, so no data is lost. The rebuild is O(N) where N = number of memories. For Permagent's 1,350 memories, this takes <1 second.
- **New databases**: Created with the 3-column FTS schema from the start.

### Migration detection

Add a migration check in the `open()` method (pattern matches existing migrations at `sqlite_store.rs:330-411`). Check whether `memories_fts` has a `description` column by querying its schema. If not, run the drop-and-recreate migration.

FTS5 virtual tables don't support `PRAGMA table_info()`. Instead, detect via:
```sql
SELECT sql FROM sqlite_master WHERE name = 'memories_fts';
```
If the SQL doesn't contain `description`, run migration. This is reliable because FTS5 stores its CREATE statement in `sqlite_master`.

---

## Section 3 — Indexing Logic

### Three cases where descriptions enter FTS

**Case 1: Memory inserted with description present**

Rare (descriptions typically populate later), but supported. The AFTER INSERT trigger reads `new.description` and indexes it. No code change needed beyond the trigger update.

**Case 2: Description updated via `set_description()`**

Currently `set_description()` (`sqlite_store.rs:1657-1661`) runs:
```sql
UPDATE memories SET description = ?1, description_generated_at = ... WHERE id = ?2
```

The AFTER UPDATE trigger fires and re-indexes `(key, content, description)`. **This already works with the new trigger definition.** The trigger captures both old and new values, so the FTS index is correctly updated.

**Case 3: Bulk description backfill**

For bench validation and production backfill, a batch of `set_description()` calls is needed. Each call triggers the AFTER UPDATE trigger, which re-indexes the memory in FTS. This is O(N) total FTS operations — acceptable for 600 bench memories or 1,350 Permagent memories.

No new code paths needed for indexing. The trigger-based approach handles all three cases.

---

## Section 4 — Ranking Integration

### Recommendation: Option A — BM25 column weights

**Option A**: Single BM25 score across all 3 indexed columns with FTS5 column weights.

FTS5's `bm25()` function accepts per-column weights:
```sql
ORDER BY bm25(memories_fts, weight_key, weight_content, weight_description)
```

The `fts_search()` function currently uses `ORDER BY rank` which is equivalent to `ORDER BY bm25(memories_fts, 1.0, 1.0)`. Changing to explicit weights:
```sql
ORDER BY bm25(memories_fts, 1.0, 1.0, 0.5)
```

This gives description matches half the weight of content matches in BM25 scoring.

**Option B** (not recommended): Separate description-match signal in the re-ranking pipeline. This would require a second FTS query against descriptions only, then an additive boost in `apply_reranking_pipeline()`. More complex, more code, and the pre-validation showed that Option A's mechanism (descriptions bring sessions INTO top-K) is sufficient — final ordering is handled by the existing re-ranking pipeline.

### Rationale for Option A

1. **Simplicity**: One line change in `fts_search()` — change `ORDER BY rank` to `ORDER BY bm25(memories_fts, 1.0, 1.0, 0.5)`.
2. **Performance**: No additional queries. BM25 computes all column contributions in a single pass.
3. **Mechanism match**: PR #101's experiment showed descriptions need to bring sessions into top-K, not dominate final ranking. BM25 column weighting does exactly this — description matches contribute to the relevance score but don't override strong content matches.
4. **Weight tuning**: The `0.5` weight for descriptions is conservative. If descriptions prove too dominant (unlikely given they're 50-100 tokens vs content at 200+ tokens), we can lower it. If bridging is too weak, we can raise it.

### Implementation

In `fts_search()` (`sqlite_store.rs:886-891`), change:
```rust
// Before
"ORDER BY rank LIMIT ?2"
// After
"ORDER BY bm25(memories_fts, 1.0, 1.0, 0.5) LIMIT ?2"
```

No changes to the re-ranking pipeline or `RerankingConfig`.

---

## Section 5 — Librarian Prompt Requirements

### Required properties (testable)

| # | Property | Test | Example |
|---|----------|------|---------|
| 1 | **Inflected forms**: Include both singular and plural of key category nouns | Search description for both forms | "User visits doctors. Doctor appointments include..." |
| 2 | **Category-level vocabulary**: Generalize from specific instances to category terms | Description contains at least one category noun not in the raw content | Content has "Dr. Patel" → description has "doctors" |
| 3 | **Concise**: 50-100 tokens | Token count check | "User visits multiple doctors including ENT specialist Dr. Patel..." (28 tokens) |
| 4 | **Accurate**: No category terms for sessions that don't deserve them | Manual spot-check: description categories match content categories | Session about cooking should NOT mention "doctors" or "furniture" |

### Suggested Librarian prompt template

This is guidance for Permagent's Librarian. Spectral documents requirements; Permagent implements.

```
Write a concise description (50-100 tokens) of this memory for search indexing.

Requirements:
- Include category-level nouns that generalize the specific items mentioned
  (e.g., "coffee table" → also say "furniture"; "Dr. Patel" → also say "doctors")
- Include BOTH singular and plural forms of key nouns
  (e.g., "doctor/doctors", "wedding/weddings", "project/projects")
- Include the specific names and details from the content
- Do NOT add category terms the content doesn't support
- Write in third person ("User..." not "I...")

Memory content:
{content}

Description:
```

### Examples

**Good descriptions:**

1. "User visits multiple doctors for health issues. Doctor appointments include ENT specialist Dr. Patel for chronic sinusitis, primary care physician Dr. Smith for antibiotics, and dermatologist Dr. Lee for biopsy follow-up." *(Contains: "doctors" + "doctor", category generalization, specific names, accurate)*

2. "User bought new furniture for their living room. Furniture purchases include a wooden coffee table with metal legs from West Elm and a Casper mattress. User rearranged living room furniture." *(Contains: "furniture" singular + plural via repetition, category generalization from "coffee table", accurate)*

3. "User attended weddings this year. Wedding attendance includes cousin's vineyard wedding in August and sister's recent wedding celebration." *(Contains: "weddings" + "wedding", category generalization, specific details, accurate)*

**Bad descriptions:**

1. "User visits Dr. Patel and Dr. Smith for medical issues." *(Missing: no "doctors"/"doctor" category term — FTS for "doctors" won't match)*

2. "This memory is about the user's daily life and various activities including shopping, health, and social events." *(Too generic: no specific details, no useful bridging vocabulary, would match almost any query)*

3. "User discusses their extensive furniture collection including antique tables, modern sofas, and designer chairs from various high-end retailers." *(Hallucinated: content only mentions a coffee table from West Elm — description adds antique tables, sofas, chairs that aren't in the content)*

---

## Section 6 — Bench Validation Strategy

### Recommendation: Option 2 — Anthropic API one-time precompute

**Chosen approach**: Use the Anthropic API (Claude Haiku) to generate descriptions for the bench corpus as a one-time precompute. Write a small Rust binary or script that:

1. Loads the bench dataset (`longmemeval_s.json`)
2. For each session, generates a description using the Librarian prompt template from Section 5
3. Saves descriptions to a JSON file (`bench_descriptions.json`) mapping session_key → description

During bench runs, descriptions are loaded and applied via `set_description()` before the evaluation loop.

**Cost estimate**: ~600 memories × ~100 input tokens × ~80 output tokens × $0.25/$1.25 per MTok (Haiku) = ~$0.02 input + ~$0.06 output = **<$0.10 total**. Negligible.

**Why not Option 1 (local Librarian)**: Requires qwen2.5:3b setup, introduces a dependency on Permagent's Librarian infrastructure, and the bench team shouldn't be blocked on Permagent's model availability.

**Why not Option 3 (wait for Permagent)**: Bench validation must run without Permagent. The bench corpus is LongMemEval, not Permagent's production data. Permagent's Librarian will never naturally process LongMemEval sessions.

### Implementation

Add a `describe` subcommand to `spectral-bench-accuracy` that:
1. Loads the dataset
2. For each question, ingests memories into a temp brain (existing `ingest_question()`)
3. Calls Haiku to generate descriptions for each memory
4. Writes descriptions to a JSON file

The main `run` subcommand gets an optional `--descriptions <path>` flag. When provided, it loads the description map and calls `set_description()` on each memory after ingestion, before retrieval.

---

## Section 7 — Test Plan

### Pre-validation: reproduce PR #101 results

After FTS schema change (before bench run), replicate PR #101's rank validation:
- Query "doctors" against a brain with description-enriched memories for case #4
- Verify all 3 answer sessions appear in top-60
- Query "furniture" for case #10
- Verify all 4 answer sessions appear in top-60

This is a unit-level test, not a full bench run. Validates the FTS schema change works end-to-end.

### Targeted bench runs

| Category | Questions | Estimated cost | Expected outcome |
|----------|-----------|---------------|-----------------|
| multi-session | 20 | $1.60 | 60% (12/20): cases #4, #10 flip to correct |
| single-session-preference | 20 | $1.60 | 70-75%: 3 vocabulary-gap RETRIEVAL_MISS cases flip |
| **Total** | 40 | **$3.20** | |

Plus description precompute: ~$0.10.

### Acceptance criteria

- multi-session: cases #4 (doctors) and #10 (furniture) score correct
- multi-session overall: >= 12/20 (60%)
- single-session-preference: stable or improved (>= current 60%)
- No regressions on currently-correct cases
- Pre-validation rank check passes for both test cases

---

## Section 8 — Risks and Mitigations

### Risk 1: FTS migration breaks existing databases

**Severity**: High (Permagent has 1,350 memories in production).
**Mechanism**: DROP TABLE + recreate loses the FTS index. If the rebuild fails partway, the FTS table is empty.
**Mitigation**: The migration runs inside a transaction. If the rebuild INSERT fails, the entire migration rolls back and the old FTS table (without description) is preserved. Additionally, test the migration against a copy of Permagent's brain DB (`~/spectral-local-bench/` has bench DBs) before merging.

### Risk 2: Description quality from Haiku vs hand-written

**Severity**: Medium. PR #101 used hand-crafted descriptions with known vocabulary. Haiku-generated descriptions may miss inflected forms or use different vocabulary.
**Mechanism**: If Haiku writes "The user consulted physicians" instead of "User visits doctors", FTS bridging fails.
**Mitigation**: The precompute step saves descriptions to a file that can be inspected and iterated. Run the pre-validation rank check after generating descriptions. If specific cases fail, regenerate those descriptions with refined prompts. The description file is deterministic — once it works, it stays working.

### Risk 3: BM25 description weight causes inappropriate ranking

**Severity**: Low. Description matches bringing irrelevant sessions into top-K.
**Mechanism**: If a description uses a category term that's too generic (e.g., "activities"), many sessions would match queries containing "activities."
**Mitigation**: Description weight is 0.5 (half of content weight). Content BM25 still dominates for sessions with strong content matches. The accuracy requirement (Section 5, property 4) prevents generic descriptions. If needed, weight can be lowered to 0.3 without code restructuring — it's a single constant.

### Risk 4: Mid-range rank cutoff dependency

**Severity**: Medium. PR #101 showed bridged sessions at ranks 23-38. If K is ever tightened below 40, these sessions drop out.
**Mechanism**: The cascade's counting profile uses K=60 (`retrieval.rs:158`). If this is changed in a future PR, description-bridged sessions at ranks 30+ would be excluded.
**Mitigation**: Document K=60 as load-bearing for description bridging. Add a comment at `retrieval.rs:158` noting the dependency. This is informational, not a code guard — K tuning is a rare, deliberate change.

---

## Section 9 — Implementation Order

### Commit 1: FTS5 schema migration

**Files**: `crates/spectral-ingest/src/sqlite_store.rs`

**Changes**:
- Add migration detection: check `sqlite_master` for `description` in FTS5 schema
- Drop and recreate FTS5 table with `(key, content, description)` columns
- Update triggers to include `COALESCE(description, '')`
- Rebuild FTS index from `memories` table
- Update initial schema creation to include description in FTS from the start

**Success criterion**: `cargo test -p spectral-ingest` passes. Existing tests work with new schema. New test: verify FTS search matches against description text.

### Commit 2: BM25 column weighting

**Files**: `crates/spectral-ingest/src/sqlite_store.rs`

**Changes**:
- Change `ORDER BY rank` to `ORDER BY bm25(memories_fts, 1.0, 1.0, 0.5)` in `fts_search()`

**Success criterion**: `cargo test -p spectral-ingest` passes. Existing FTS tests still pass (column weights don't change result set, only ordering when description matches exist).

### Commit 3: Bench description precompute tooling

**Files**: `crates/spectral-bench-accuracy/src/main.rs` (new subcommand), possibly a new `describe.rs` module

**Changes**:
- Add `describe` subcommand that generates descriptions for bench memories via Haiku API
- Add `--descriptions <path>` flag to `run` subcommand
- Load descriptions and apply via `set_description()` after ingestion, before retrieval

**Success criterion**: `cargo check -p spectral-bench-accuracy` passes. `describe` subcommand produces a JSON file with descriptions for all bench memories.

### Commit 4: Pre-validation rank check

**Changes**:
- Generate descriptions for cases #4 and #10 using the `describe` tooling
- Run rank validation (unit test or manual check) confirming PR #101 results reproduce
- Document results in a brief verification note

**Success criterion**: Case #4 doctors: 3/3 answer sessions in top-60. Case #10 furniture: 4/4 answer sessions in top-60.

### Commit 5: Targeted bench runs

**Changes**: None (bench runs are external). Results documented in PR description.

**Success criterion**: multi-session >= 60%, single-session-preference stable or improved.

---

## Section 10 — Summary

| Metric | Value |
|--------|-------|
| Files modified | 2 (`sqlite_store.rs`, bench `main.rs`) |
| New files | 1 (bench `describe.rs` module) |
| FTS schema change | Add `description` as 3rd indexed column |
| Migration approach | Drop + recreate FTS5 table, rebuild from base |
| Ranking change | BM25 column weight 0.5 for descriptions |
| Description source for bench | Haiku API precompute (~$0.10) |
| Expected lift | +2 multi-session (50% → 60%), +1-3 single-session-preference |
| Validation cost | $3.20 bench runs + $0.10 description generation |
| Commits | 5 sequential, each with own success criterion |
