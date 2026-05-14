# T2 — Topology Design Decisions (Kuzu Graph)

Investigation date: 2026-05-14. Answers the three open questions
from backlog item T2.

---

## Q1. Degree cap

### Current state

`KuzuStore::neighborhood()` (`kuzu_store.rs:343`) performs a
bidirectional BFS bounded only by `max_hops`. At each hop, ALL
incoming and outgoing `Triple` edges are followed. No per-hop
fanout cap, no degree limit. The only deduplication is a `visited`
HashSet that prevents revisiting entities.

`Brain::recall_graph()` (`brain.rs:1268`) calls
`self.store.neighborhood(seed, 2)` — hardcoded 2 hops. This is
the sole consumer.

Schema (`schema.rs:98-108`):

```
Triple(FROM Entity TO Entity, predicate STRING, confidence DOUBLE,
       source_doc_id BLOB, source_brain_id BLOB, asserted_at TIMESTAMP,
       visibility STRING, weight DOUBLE)
```

No schema-level constraints on edge count per node.

### What was measured

**Direct neighborhood-size measurement was not possible.** Kuzu
stores data in a binary format; querying requires running Rust
code through `KuzuStore::neighborhood()`. This investigation is
read-only (no code changes), so no measurement script was written.

What *is* observable:

- **Permagent ontology:** ~400+ entities defined
  (`~/.permagent/brain/ontology.toml`). Of these, ~15 are
  manually curated with aliases (henry malcolm, permagent,
  spectral, jesse sharratt, sophie, etc.). The remaining ~385 are
  auto-created by the entity extraction pipeline — many are noisy
  phrases mistyped as "person" (e.g. "right now", "dead simple",
  "heavy messages").
- **Permagent graph.kz:** 7.3MB file. Non-trivial; implies
  hundreds to thousands of triples.
- **27 predicates** defined (works_on, founded_by, built_with,
  location, wife, son, etc.), all with wildcard domain/range.
- **LongMemEval bench ontology:** `version = 1\n` — empty. The
  bench Kuzu graph contains zero entities. `recall_graph()` always
  returns empty `seed_entities` and falls through to the FTS
  fallback (`retrieval.rs:396`). This means **no bench measurement
  of graph neighborhood is possible** with the current corpus.

**Structural analysis (from code + ontology):**

Central entities like "jesse sharratt" and "permagent" likely have
high degree. With ~400 entities, ~27 predicates, and bidirectional
BFS at max_hops=2, a well-connected seed could plausibly reach
50-200+ entities. The ontology noise (auto-created phrase entities)
makes this worse: phrases like "right now" or "dead simple" may
match across many documents via `ingest_document()` canonicalization,
creating spurious entity nodes with `Mentions` edges and potentially
`Triple` edges from `assert()` calls.

### Recommendation

**Add a per-hop fanout cap as a precondition for T1, not
independently.** Rationale:

1. The graph is inert today. Adding a cap to dead code is
   speculative engineering.
2. If T1 decides to wire the graph into the cascade, the first
   step of that work should be measuring actual neighborhood sizes
   on a populated graph (Permagent brain). The measured p90 and max
   determine whether a cap is needed and what value.
3. A reasonable starting point if measurement shows dense nodes:
   cap at 20-30 entities per hop (similar to co-retrieval's
   `limit: 50` on `related_memories`). This keeps 2-hop
   neighborhoods at 400-900 max, comparable to the entity count
   itself.
4. If measurement shows the graph is sparse (p90 < 10 at 1-hop),
   `max_hops=2` alone is sufficient and a cap adds complexity for
   no benefit. "No cap needed" is a valid answer — but only with
   evidence.

**Decision: deferred to T1. When T1 begins, measure first, then
decide.** The measurement is a 30-minute task (write a small binary
that opens the Permagent brain's Kuzu store and iterates entities).
Don't guess.

---

## Q2. Adjacency basis

### Current state

The Kuzu graph contains exactly one edge type: `Triple`, connecting
`Entity` to `Entity` via explicit predicates. The schema
(`schema.rs:98-108`) defines:

- **Entity nodes:** id, entity_type, canonical, visibility, weight
- **Triple edges:** predicate, confidence, weight, source_doc_id,
  source_brain_id, asserted_at, visibility
- **Document nodes + Mentions edges:** linking documents to the
  entities they mention (span-level). These are structural metadata,
  not adjacency for traversal — `neighborhood()` only follows
  `Triple` edges.

Triples enter the graph through two paths:
1. `Brain::assert()` / `Brain::assert_typed()` (`brain.rs:522+`) —
   explicit knowledge assertions, typically from an extraction
   pipeline or direct API calls.
2. `Brain::ingest_document()` (`brain.rs:1304`) — upserts entity
   nodes from canonicalized mentions and creates `Mentions` edges
   (not `Triple` edges). Does NOT create inter-entity triples.

The ontology's predicate set is explicit-relational: `works_on`,
`founded_by`, `wife`, `son`, `location`, `built_with`, etc. These
encode knowledge-graph-style structural facts.

### Assessment of blending co-access or semantic similarity

**Co-access adjacency:** Already exists as a separate signal.
Co-retrieval pairs (PR #90, `co_retrieval_pairs` table in SQLite)
capture session co-occurrence at memory granularity, live in the
cascade ranking pipeline at weight 0.10. Duplicating this signal
into the Kuzu graph would:
- Conflate two semantically different kinds of adjacency
  (structural knowledge vs. behavioral co-occurrence)
- Require a sync mechanism between SQLite co_retrieval_pairs and
  Kuzu edges
- Make the Kuzu graph's meaning ambiguous: does an edge mean
  "structurally related" or "frequently retrieved together"?

**Semantic similarity:** Would require embedding computation at
entity level and a threshold-based edge creation step. This inverts
the "zero-LLM recognition" architectural commitment for retrieval.
Spectral's retrieval is deterministic; adding embedding-based
adjacency to the graph layer undermines that.

### Recommendation

**Keep the Kuzu graph pure explicit-predicate.** Rationale:

1. Each adjacency type has a clean semantic role today:
   co-retrieval = behavioral, Kuzu graph = structural knowledge.
   Preserving this separation keeps the signals interpretable and
   independently tunable.
2. No concrete failure case exists where explicit-predicate edges
   miss something that blended edges would catch. The documented
   bench failures (GENUINE_MISS, RETRIEVAL_MISS) are not graph
   failures — they are either actor-attention or FTS-vocabulary
   problems.
3. If a future failure case shows that "entity A and entity B are
   related but no predicate connects them," the right response is
   to add the predicate to the ontology, not to blur the graph's
   semantic basis.

**Decision: pure explicit-predicate. Do not blend.**

---

## Q3. Relationship to co-retrieval pairs

### Current state

Two adjacency signals exist:

| Property | Co-retrieval pairs | Kuzu neighborhood |
|---|---|---|
| **Data store** | SQLite `co_retrieval_pairs` table | Kuzu `graph.kz` |
| **Granularity** | Memory-to-memory | Entity-to-entity |
| **Basis** | Behavioral: session co-occurrence | Structural: explicit predicate triples |
| **Construction** | `rebuild_co_retrieval_index()` from `retrieval_events` | `assert()` / extraction pipeline |
| **Cascade integration** | Live. Additive boost at weight 0.10 (`ranking.rs:328-335`) | Inert. Only consumed by `--retrieval-path graph` bench flag |
| **Query path** | `related_memories(memory_id, limit)` → memory IDs | `neighborhood(entity_id, max_hops)` → entities/triples |
| **Memory bridging** | Direct: returns memory IDs, used in re-ranking | Indirect: returns entity canonicals, then FTS search per entity (`retrieval.rs:385-393`) |

### Overlap analysis

**Direct empirical overlap measurement was not possible.** The two
signals operate at different granularity (memories vs. entities) and
are stored in different databases (SQLite vs. Kuzu). Comparing them
requires: (a) a populated Kuzu graph (bench has none — empty
ontology), and (b) code to map entity neighborhoods back to memory
sets and intersect with co-retrieval memory sets. This investigation
is code-read-only.

**Structural analysis of overlap:**

The signals are fundamentally different in what they capture:

- **Co-retrieval** answers: "which memories tend to appear together
  in retrieval results?" This is query-behavioral — it emerges
  from FTS patterns and user question distribution. A memory about
  "Jesse's favorite restaurant" and a memory about "Jesse's
  commute" might co-retrieve because both mention "Jesse," not
  because restaurants and commutes are structurally related.

- **Kuzu neighborhood** answers: "which entities are structurally
  connected by explicit predicates?" Jesse `works_on` Permagent,
  Jesse `location` Halifax — these are knowledge-graph facts. They
  could surface memories *about* related entities even when the
  query text doesn't match (vocabulary bridging via entity
  canonicals).

**Where they might overlap:** Both signals would boost a memory
about Permagent when the query mentions Jesse, since Jesse
`works_on` Permagent (Kuzu) and memories about Jesse and Permagent
likely co-retrieve (co-retrieval). For well-connected entities with
high retrieval frequency, overlap would be substantial.

**Where they diverge:**

- Kuzu catches *structural relationships the user never queried
  about.* If Jesse `wife` Sophie exists in the graph but no query
  ever retrieved both together, co-retrieval misses this but Kuzu
  has it.
- Co-retrieval catches *emergent associations with no structural
  basis.* Two memories co-retrieve because they share vocabulary
  patterns, not because the entities in them are predicate-linked.

### Recommendation

**Complementary in theory, but the Kuzu path has a critical
architectural gap that makes it low-value today.** Rationale:

1. **The entity→memory bridging gap is the real problem.**
   `recall_graph()` (`retrieval.rs:375-412`) converts entity
   neighborhoods back to memories by running FTS searches on entity
   canonical names. This means the graph path is: query →
   canonicalize → entity match → 2-hop BFS → canonical names → FTS
   search per name. It's an elaborate detour that ultimately still
   relies on FTS. If the canonical name doesn't match memory
   content (common with auto-created noise entities like "right
   now" or "dead simple"), the FTS step returns nothing useful.

2. **Co-retrieval operates directly on memories.** No bridging gap.
   `related_memories()` returns memory IDs that feed directly into
   re-ranking. This is why co-retrieval shipped (PR #90) and
   delivers measurable value at weight 0.10, while the graph path
   remains experimental.

3. **The ontology quality problem amplifies the bridging gap.**
   The Permagent ontology has ~400 entities, but ~385 are
   auto-created noise. A 2-hop BFS from a real entity (jesse) will
   traverse noisy entities (if connected via triples), producing
   canonical names that FTS-search to irrelevant memories.
   Co-retrieval has no equivalent noise problem — it's built from
   actual retrieval co-occurrence, which is self-filtering.

4. **Complementary value exists but requires preconditions.** The
   Kuzu graph's unique value — structural relationships the user
   never queried about — is real but requires: (a) a clean
   ontology (manual curation or better extraction), (b) a direct
   entity→memory link (not the FTS detour), and (c) measured
   evidence that entity-neighbor memories add recall that
   co-retrieval misses.

**Decision: complementary in principle, but co-retrieval subsumes
the graph's practical value today given the bridging gap and
ontology noise.** T1 should not wire the graph into the cascade
unless it also closes the entity→memory bridging gap. The graph
has unique structural signal, but delivering it requires more than
just calling `neighborhood()` and FTS-searching the results.

---

## Implications for T1

Given these three decisions, T1 is **not a simple wiring task.**
It is also **not a clear retirement**, because the structural
signal the Kuzu graph captures is genuinely complementary to
co-retrieval. T1 is a conditional wiring task with preconditions:

1. **Measure neighborhood sizes** on the Permagent brain's Kuzu
   graph. If central entities produce 100+ entity neighborhoods at
   2 hops, add a per-hop fanout cap before wiring. (Q1)

2. **Close the entity→memory bridging gap.** The current path
   (entity canonical → FTS search) is architecturally weak. Two
   options:
   - **Direct:** Add a `memory_ids` field to Entity nodes or a
     `Mentions` → memory link, so neighborhood traversal returns
     memories directly without FTS.
   - **Indirect:** Use entity canonicals as boosting signals in
     the existing FTS/cascade ranking (similar to co-retrieval
     boost), not as independent FTS queries.

3. **Ontology cleanup.** The ~385 auto-created noise entities
   dilute graph signal. Either curate the ontology or improve the
   extraction pipeline's entity quality before measuring graph
   lift.

4. **Still gated on item #8 bench validation** (attribution
   confounding rule). Don't measure graph lift while
   description-enriched FTS lift is un-isolated.

If preconditions 1-3 are met and item #8 completes, T1 is a
wiring task. If measuring shows the graph adds negligible recall
over co-retrieval (after fixing the bridging gap), T1 becomes a
retirement. The design questions are resolved; the empirical
question remains open.

**Retirement is premature.** The graph's structural signal is real
and distinct from co-retrieval. But wiring it in its current form
(FTS detour + noisy ontology) would not produce measurable lift,
and could add noise to ranking. The honest sequence is: fix the
preconditions, then measure, then wire or retire.
