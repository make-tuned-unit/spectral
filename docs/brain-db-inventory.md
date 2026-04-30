# brain.db Inventory Report

Generated: 2026-04-28
Purpose: Plan migration tooling for Spectral (github.com/make-tuned-unit/spectral)

---

## Schema

### Core tables

**`memories`** — Primary memory store (928 rows)
```sql
CREATE TABLE memories (
    id          TEXT PRIMARY KEY,
    key         TEXT NOT NULL UNIQUE,
    content     TEXT NOT NULL,
    category    TEXT NOT NULL DEFAULT 'core',
    embedding   BLOB,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL,
    session_id  TEXT,
    wing        TEXT DEFAULT NULL,
    hall        TEXT DEFAULT NULL,
    room        TEXT DEFAULT NULL,
    valid_from  TEXT DEFAULT NULL,
    valid_until TEXT DEFAULT NULL,
    superseded_by TEXT DEFAULT NULL,
    confidence  REAL DEFAULT 1.0,
    signal_score REAL DEFAULT 0.5,
    last_retrieved TEXT DEFAULT NULL
);
```

Indexes: `idx_memories_category`, `idx_memories_key`, `idx_memories_session`, `idx_memories_wing`, `idx_memories_hall`, `idx_memories_room`, `idx_memories_wing_hall`, `idx_memories_wing_room`, `idx_memories_valid_from`, `idx_memories_valid_until`, `idx_memories_current` (partial, WHERE valid_until IS NULL), `idx_memories_signal`, `idx_memories_last_retrieved`

FTS: `memories_fts` (fts5 on key + content, synced via triggers)

Views: `current_facts` (valid_until IS NULL AND hall='fact'), `memory_timeline` (all memories with status column)

**`constellation_fingerprints`** — Cross-memory relationship hashes (3,145 rows)
```sql
CREATE TABLE constellation_fingerprints (
    id TEXT PRIMARY KEY,
    fingerprint_hash TEXT NOT NULL,
    anchor_memory_id TEXT NOT NULL,
    target_memory_id TEXT NOT NULL,
    wing TEXT,
    anchor_hall TEXT,
    target_hall TEXT,
    time_delta_bucket TEXT,
    created_at TEXT,
    FOREIGN KEY (anchor_memory_id) REFERENCES memories(id),
    FOREIGN KEY (target_memory_id) REFERENCES memories(id)
);
```

Indexes: `idx_fp_hash`, `idx_fp_wing`, `idx_fp_wing_hash`, `idx_fp_wing_anchor_hall`, `idx_fp_wing_target_hall`

**`knowledge_graph`** — Subject-predicate-object triples (34 rows)
```sql
CREATE TABLE knowledge_graph (
    id TEXT PRIMARY KEY,
    subject TEXT NOT NULL,
    predicate TEXT NOT NULL,
    object TEXT NOT NULL,
    valid_from TEXT NOT NULL,
    valid_until TEXT,
    source_memory_id TEXT,
    confidence REAL DEFAULT 1.0,
    created_at TEXT NOT NULL,
    FOREIGN KEY (source_memory_id) REFERENCES memories(id)
);
```

FTS: `knowledge_graph_fts` (fts5 on subject, predicate, object)
Views: `current_knowledge`, `entity_summary`

**`entities`** — Named entities extracted from memories (489 rows)
```sql
CREATE TABLE entities (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    category TEXT NOT NULL,
    aliases TEXT DEFAULT '[]',
    summary TEXT,
    first_seen TEXT,
    last_seen TEXT,
    mention_count INTEGER DEFAULT 1,
    created_at TEXT,
    updated_at TEXT
);
```

**`entity_relationships`** — Directed relationships between entities (655 rows)
```sql
CREATE TABLE entity_relationships (
    id TEXT PRIMARY KEY,
    source_entity_id TEXT NOT NULL,
    target_entity_id TEXT NOT NULL,
    relationship_type TEXT NOT NULL,
    evidence TEXT,
    valid_from TEXT,
    valid_until TEXT,
    confidence REAL DEFAULT 0.8,
    created_at TEXT,
    FOREIGN KEY (source_entity_id) REFERENCES entities(id),
    FOREIGN KEY (target_entity_id) REFERENCES entities(id)
);
```

**`entity_mentions`** — Links entities to source memories (1,547 rows)
```sql
CREATE TABLE entity_mentions (
    id TEXT PRIMARY KEY,
    entity_id TEXT NOT NULL,
    memory_id TEXT NOT NULL,
    context TEXT,
    created_at TEXT,
    FOREIGN KEY (entity_id) REFERENCES entities(id),
    FOREIGN KEY (memory_id) REFERENCES memories(id)
);
```

### Auxiliary tables

**`documents`** — Ingested external documents, primarily Slack messages (4,957 rows)
```sql
CREATE TABLE documents (
    id TEXT PRIMARY KEY,
    source_type TEXT NOT NULL,  -- slack (4951), gdrive (3), manual (1), project (1), system_capability (1)
    source_id TEXT NOT NULL,
    source_url TEXT,
    title TEXT NOT NULL,
    content TEXT NOT NULL,
    chunk_index INTEGER DEFAULT 0,
    chunk_total INTEGER DEFAULT 1,
    category TEXT DEFAULT 'document',
    embedding BLOB,
    metadata TEXT,
    first_seen_at TEXT NOT NULL,
    last_synced_at TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    is_deleted INTEGER DEFAULT 0
);
```

FTS: `documents_fts` (fts5 on title, content, source_type, category — uses content table `documents_fts_content`)

**`wing_to_memory_ids`** — Denormalized wing→memory lookup with scores (926 rows)
```sql
CREATE TABLE wing_to_memory_ids (
    wing TEXT NOT NULL,
    memory_id TEXT NOT NULL,
    signal_score REAL DEFAULT 0,
    PRIMARY KEY (wing, memory_id)
);
```

**`memory_spectrogram`** — Dimensional analysis of memories (362 rows)
```sql
CREATE TABLE memory_spectrogram (
    memory_id TEXT PRIMARY KEY,
    entity_density REAL,
    action_type TEXT,
    decision_polarity REAL,
    causal_depth REAL,
    emotional_valence REAL,
    temporal_specificity REAL,
    novelty REAL,
    peak_dimensions TEXT,
    created_at TEXT,
    FOREIGN KEY (memory_id) REFERENCES memories(id)
);
```

**`embedding_cache`** — Cached embeddings by content hash (881 rows)
```sql
CREATE TABLE embedding_cache (
    content_hash TEXT PRIMARY KEY,
    embedding BLOB NOT NULL,
    created_at TEXT NOT NULL,
    accessed_at TEXT NOT NULL
);
```

**`llm_spend_log`** — LLM usage tracking (171 rows)
```sql
CREATE TABLE llm_spend_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp TEXT NOT NULL DEFAULT (datetime('now')),
    caller_id TEXT NOT NULL,
    model_requested TEXT NOT NULL,
    model_used TEXT,
    provider TEXT,
    input_tokens INTEGER DEFAULT 0,
    output_tokens INTEGER DEFAULT 0,
    total_tokens INTEGER DEFAULT 0,
    cost_usd REAL DEFAULT 0.0,
    latency_ms INTEGER DEFAULT 0,
    status TEXT DEFAULT 'success',
    error_message TEXT
);
```

---

## Memory inventory

| Metric | Value |
|--------|-------|
| Total memories | 928 |
| Date range | 2026-03-23 to 2026-04-28 (~36 days) |
| Total content bytes | 1,308,437 (~1.25 MB) |

### Count per wing (descending)

| Wing | Count |
|------|-------|
| alice | 280 |
| general | 200 |
| infra | 151 |
| vega | 86 |
| acme | 56 |
| polarismedia-site | 49 |
| apollo | 24 |
| polaris-media | 17 |
| polaris-media-listener | 17 |
| love-nova-scotia | 8 |
| agent-lightning | 7 |
| canadian-parents-french | 4 |
| cortex | 4 |
| carol | 4 |
| aidvocate | 3 |
| polaris | 3 |
| _reference | 2 |
| familylife | 2 |
| polaris | 2 |
| *(11 more wings with 1 each)* | 11 |

### Count per hall (descending)

| Hall | Count |
|------|-------|
| event | 574 |
| fact | 171 |
| artifacts | 89 |
| advice | 50 |
| preference | 22 |
| discovery | 20 |
| activity | 1 |
| system | 1 |

### Signal score distribution

| Metric | Value |
|--------|-------|
| Min | 0.5 |
| Max | 1.0 |
| Median | 0.55 |
| Count > 0.7 | 114 (12.3%) |

### Content length distribution

| Metric | Value |
|--------|-------|
| Min | 15 chars |
| Max | 19,404 chars |
| Median | 512 chars |
| Count > 1,000 chars | 333 (35.9%) |

### Category values (legacy field, predates wing/hall)

`conversation`, `core`, `daily`, `fact`, `polaris`, `openbird`, `vega`, `projects`, `task`

### Sample memories (anonymized)

| id (truncated) | wing | hall | signal | len | content excerpt |
|----------------|------|------|--------|-----|-----------------|
| fd8824aa... | infra | event | 0.6 | 1,371 | Task [failed] via claude-code. Description: PHASE 2: RESTART BACKEND. Find the running uvicorn process... |
| b6dfc170... | general | advice | 0.55 | 963 | This log captures a highly fragmented and multi-threaded user session, suggesting [REDACTED] is context-switching rapidly between several different tasks... |
| 3ea182ec... | general | event | 0.6 | 920 | Task [completed] via claude-code. Description: Build [REDACTED] from source and restart. Steps: 1. cd to the directory 2. Run cargo build --release... |
| f913f4bd... | alice | advice | 0.85 | 5,830 | [REDACTED] can you submit this as a task to Claude Code via the task runner: Task: Wire [REDACTED] Waitlist to [REDACTED] + [REDACTED]. Context: The [REDACTED] site... |
| ea55bb6e... | infra | event | 0.6 | 2,461 | Task [completed] via claude-code. Description: Fix agent positioning in [REDACTED].tsx in command-center-ui. Agents clustered instead of spread across the lab... |

---

## Fingerprint inventory

| Metric | Value |
|--------|-------|
| Total fingerprints | 3,145 |
| Unique anchor memories | 177 |
| Unique target memories | 180 |
| Avg fingerprints per memory | 3.39 (928 memories total) |
| Avg fingerprints per anchor | 17.8 |

### Time bucket distribution

| Bucket | Count |
|--------|-------|
| same_week | 1,302 (41.4%) |
| same_day | 977 (31.1%) |
| same_month | 866 (27.5%) |

### Top wings in fingerprints

| Wing | Count |
|------|-------|
| alice | 1,326 |
| infra | 861 |
| vega | 276 |
| acme | 190 |
| general | 78 |
| xwing:alice+infra | 39 |
| polaris-media | 36 |
| love-nova-scotia | 28 |
| *(82 more, mostly xwing: cross-wing pairs)* | ~311 |

Note: `xwing:` prefixed wings represent cross-wing fingerprints linking memories across two different wings.

### Sample fingerprint rows

| id | fingerprint_hash | anchor_memory_id | target_memory_id | wing | anchor_hall | target_hall | time_delta_bucket |
|----|-----------------|------------------|------------------|------|-------------|-------------|-------------------|
| f2a16a72fcad48c5 | 4c355a4f544a52f5 | e04e761c-d4d7-... | 01e381ce-39a7-... | polaris-media | fact | fact | same_day |
| c61771f3bd97495e | 4c355a4f544a52f5 | e04e761c-d4d7-... | 0bfb57a3-3358-... | polaris-media | fact | fact | same_day |
| 52c7402480fd467a | 4c355a4f544a52f5 | e04e761c-d4d7-... | f857fb2f-557a-... | polaris-media | fact | fact | same_day |
| 695406bce158446f | 4c355a4f544a52f5 | e04e761c-d4d7-... | 8017a8df-9d00-... | polaris-media | fact | fact | same_day |
| dfcfcaaf1a934163 | 4c355a4f544a52f5 | e04e761c-d4d7-... | b15c927f-6c0a-... | polaris-media | fact | fact | same_day |

Note: These 5 rows all share the same anchor memory and fingerprint_hash — fingerprints are fan-out from one anchor to many targets.

---

## Graph data

### knowledge_graph (SPO triples)

| Metric | Value |
|--------|-------|
| Total triples | 34 |
| Current (valid_until IS NULL) | 34 (all current, none expired) |

**Predicate distribution:**

| Predicate | Count |
|-----------|-------|
| is | 11 |
| role | 2 |
| son | 2 |
| anniversary | 1 |
| board_chair | 1 |
| business | 1 |
| client | 1 |
| company | 1 |
| employer | 1 |
| favourite_coffee_shop | 1 |
| favourite_color | 1 |
| favourite_restaurant | 1 |
| location | 1 |
| partner | 1 |
| primary_browser | 1 |
| primary_messaging | 1 |
| primary_strategy | 1 |
| runs_on | 1 |
| secondary_browser | 1 |
| status | 1 |

### Entity system

| Table | Count |
|-------|-------|
| entities | 489 |
| entity_relationships | 655 |
| entity_mentions | 1,547 |

**Entity categories:**

| Category | Count |
|----------|-------|
| person | 290 |
| concept | 146 |
| project | 25 |
| tool | 21 |
| organization | 4 |
| location | 3 |

**Relationship types (top 7):**

| Type | Count |
|------|-------|
| works_on | 501 |
| built_with | 145 |
| related_to | 3 |
| part_of | 2 |
| manages | 2 |
| founded_by | 1 |
| deployed_on | 1 |

---

## File-level facts

| Metric | Value |
|--------|-------|
| Database size (page_count * page_size) | 61,456,384 bytes (~58.6 MB) |
| Journal mode | WAL |
| Encoding | UTF-8 |
| PRAGMA application_id | 0 (not set) |
| PRAGMA user_version | 0 (not set) |

Note: The large DB size relative to content (~1.25 MB text + ~3.4 MB embeddings) is due to FTS indexes, embedding_cache (881 entries), and document storage (4,957 Slack messages + embeddings).

### Auxiliary table sizes

| Table | Rows |
|-------|------|
| memory_spectrogram | 362 |
| embedding_cache | 881 |
| wing_to_memory_ids | 926 |
| llm_spend_log | 171 |

### Embedding details

- Each embedding is 4,096 bytes (1,024 float32 values)
- 861 of 928 memories have embeddings (67 missing, 7.2%)
- Total embedding data in memories table: 3,526,656 bytes (~3.4 MB)

---

## Migration considerations

### Column mapping to Spectral's Memory struct

| brain.db column | Spectral field | Notes |
|----------------|----------------|-------|
| id | id | Direct map. brain.db uses UUIDs (TEXT). |
| key | key | Direct map. Unique text keys. |
| content | content | Direct map. |
| wing | wing | Direct map. |
| hall | hall | Direct map. |
| signal_score | signal_score | Direct map. Range 0.5–1.0 in brain.db. |
| confidence | confidence | Direct map. |
| created_at | created_at | Direct map. Mixed timestamp formats (see below). |
| — | visibility | **No equivalent in brain.db.** Will need a default value. |
| — | source | **No equivalent in brain.db.** Could derive from `category` or `session_id`. |
| — | device_id | **No equivalent in brain.db.** Will need a default or null. |
| — | last_reinforced_at | **No equivalent in brain.db.** `last_retrieved` is closest but semantically different. |

### brain.db columns with NO Spectral equivalent

| Column | Status | Migration action needed |
|--------|--------|----------------------|
| category | 9 distinct values (core, task, conversation, etc.) | Legacy field; may map to `source` or be stored in metadata. |
| embedding | BLOB, 4096 bytes each | Spectral may recompute or import. Decide per-model compatibility. |
| updated_at | TEXT timestamp | Drop or store as metadata. |
| session_id | TEXT, NULL in 747/928 rows (80.5%) | Drop or map to `source`. |
| room | TEXT, NULL in 365/928 rows (39.3%) | No Spectral field. Could be metadata or dropped. |
| valid_from | TEXT, NULL in 420/928 rows (45.3%) | No Spectral field. Temporal validity — consider metadata. |
| valid_until | TEXT | No Spectral field. Used by `current_facts` view. |
| superseded_by | TEXT, always NULL (0 non-null rows) | Unused. Safe to drop. |
| last_retrieved | TEXT, NULL in all 928 rows | Unused. Safe to drop. |

### Data quality concerns

1. **Inconsistent timestamp formats.** `created_at` uses at least two formats:
   - ISO 8601 with timezone: `2026-04-21T12:56:14.967581-03:00`
   - ISO 8601 without timezone: `2026-04-14 00:37:36`
   - Migration tool must normalize to a single format.

2. **High null rates in optional fields:**
   - `session_id`: 80.5% NULL
   - `valid_from`: 45.3% NULL
   - `room`: 39.3% NULL
   - `last_retrieved`: 100% NULL
   - `superseded_by`: 100% NULL

3. **67 memories (7.2%) lack embeddings.** Migration should either recompute or flag these.

4. **Embedding model unknown.** 4096-byte blobs = 1024 float32 dimensions. Migration tool should verify the embedding model matches Spectral's expected model before importing vectors.

5. **`category` vs `hall` overlap.** Both fields categorize memories but with different vocabularies. `hall` (8 values) is the newer taxonomy; `category` (9 values) is legacy. Some values overlap (e.g., `fact` appears in both).

6. **Cross-wing fingerprints use synthetic wing names.** The `xwing:A+B` format in constellation_fingerprints is a derived composite, not a real wing. Migration tool should handle or skip these.

### Tables to migrate vs skip

| Table | Migrate? | Notes |
|-------|----------|-------|
| memories | **Yes** | Core data. 928 rows, ~1.25 MB content. |
| constellation_fingerprints | **Yes** | 3,145 rows. Core to Spectral's constellation system. |
| knowledge_graph | **Yes** | 34 rows. Small but semantically important. |
| entities | **Yes** | 489 rows. Entity extraction results. |
| entity_relationships | **Yes** | 655 rows. Graph edges. |
| entity_mentions | **Yes** | 1,547 rows. Entity-to-memory links. |
| memory_spectrogram | **Yes** | 362 rows. Dimensional analysis. |
| wing_to_memory_ids | **Maybe** | 926 rows. Denormalized index — could be rebuilt from memories. |
| documents | **Decide** | 4,957 rows (mostly Slack). Large but may not belong in Spectral. |
| embedding_cache | **Skip** | 881 rows. Cache that can be rebuilt. |
| llm_spend_log | **Skip** | 171 rows. Operational log, not memory data. |
| FTS virtual tables | **Skip** | Will be rebuilt by Spectral's own FTS setup. |

### Volume summary for capacity planning

| Category | Rows | Estimated bytes |
|----------|------|----------------|
| memories (content) | 928 | ~1.3 MB |
| memories (embeddings) | 861 | ~3.4 MB |
| constellation_fingerprints | 3,145 | ~300 KB |
| knowledge_graph | 34 | ~5 KB |
| entities + relationships + mentions | 2,691 | ~500 KB |
| memory_spectrogram | 362 | ~50 KB |
| documents (if included) | 4,957 | ~20-40 MB (estimate) |
| **Total (excl. documents)** | **~8,000 rows** | **~5.5 MB** |
| **Total (incl. documents)** | **~13,000 rows** | **~25-45 MB** |

The migration is small enough to run as a single-pass batch job. No streaming or chunking infrastructure needed.
