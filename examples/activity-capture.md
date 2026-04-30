# Activity Capture

Wiring a desktop activity capture system into Spectral. The agent observes
user activity, stores it with full provenance, infers relationships, and
builds a searchable knowledge base that improves over time.

## When to use this pattern

Your agent passively observes a stream of activity events and needs to:
- Store observations with source attribution and device identity
- Infer and assert relationships between entities mentioned in activity
- Recall activity context using both memory search and graph traversal
- Strengthen memories that prove useful for downstream tasks

If you only need keyword search over a log, a full-text index is simpler.
Use Spectral when you need to relate facts across events (multi-hop
queries) and want recall quality to improve with use via reinforcement.

## Architecture

```
┌──────────────┐
│  Activity    │  (screen text, app switches, file edits, etc.)
│  Source      │
└──────┬───────┘
       │ events
       ▼
┌──────────────┐  remember_with()  ┌──────────────────┐
│  Capture     │──────────────────▶│    Spectral      │
│  Agent       │                   │     Brain        │
│              │  assert()         │                  │
│  (classify,  │──────────────────▶│  ┌────────────┐  │
│   extract)   │                   │  │ SQLite     │  │
│              │  recall()         │  │ (memories) │  │
│              │◀─────────────────│  ├────────────┤  │
│              │                   │  │ Kuzu       │  │
│              │  reinforce()      │  │ (graph)    │  │
│              │──────────────────▶│  └────────────┘  │
└──────┬───────┘                   │                  │
       │                           │  spectrogram     │
       ▼                           │  (cross-wing)    │
┌──────────────┐                   └──────────────────┘
│  Downstream  │
│  Consumer    │  (agent tasks, search UI, daily summary)
└──────────────┘
```

The capture agent runs a continuous loop:
1. **Observe** — receive activity event from the source
2. **Store** — `remember_with` the raw observation with provenance
3. **Extract** — identify entities and relationships in the event
4. **Assert** — write relationships to the knowledge graph
5. **Recall** — periodically query for context to inform downstream tasks
6. **Reinforce** — strengthen memories that downstream consumers used

## Brain setup

Configure custom wing rules to categorize activity events by domain, and
enable spectrogram for cross-wing pattern matching.

```rust
use std::sync::Arc;
use spectral::{Brain, BrainBuilder, EntityPolicy, Visibility};

let brain = Brain::builder()
    .data_dir("./activity-brain")
    .ontology_path("./ontology.toml")
    .wing_rules(vec![
        (r"(?i)(vscode|editor|file|code|commit)".into(), "development".into()),
        (r"(?i)(slack|email|message|chat|meeting)".into(), "communication".into()),
        (r"(?i)(jira|ticket|sprint|backlog|task)".into(), "project_management".into()),
        (r"(?i)(browser|search|docs|wiki|stackoverflow)".into(), "research".into()),
    ])
    .enable_spectrogram(true)
    .entity_policy(EntityPolicy::AutoCreateWithCanonicalizer(
        Arc::new(|mention: &str| {
            mention
                .trim()
                .to_lowercase()
                .replace(char::is_whitespace, "_")
        }),
    ))
    .build()?;
```

**Wing rules** classify each memory into a topic area. Custom rules let
you match your activity domains. The defaults work for general text but
activity events benefit from domain-specific patterns.

**Spectrogram** (`enable_spectrogram(true)`) computes a 7-dimension
cognitive fingerprint for each memory on ingest. This powers
`recall_cross_wing()` — finding structurally similar memories across
different topic areas.

## Capturing activity events

Store each activity event with full provenance using `remember_with`.

```rust
use spectral::{RememberOpts, DeviceId, Visibility};

let event_content = "Alice opened authentication.rs in VSCode and modified the OAuth token refresh logic";
let result = brain.remember_with(
    "activity-evt-1001",
    event_content,
    RememberOpts {
        source: Some("desktop_capture".into()),
        device_id: Some(DeviceId::from_descriptor("alice-macbook")),
        confidence: Some(0.85),
        visibility: Visibility::Private,
    },
)?;

println!(
    "Stored: wing={:?}, hall={:?}, signal={:.2}, fingerprints={}",
    result.wing, result.hall, result.signal_score, result.fingerprints_created
);
```

Key design decisions:
- **Unique keys** — use a monotonic event ID or UUID so each observation is distinct
- **Source** — tag the origin system (`"desktop_capture"`, `"browser_extension"`, etc.)
- **Device ID** — `DeviceId::from_descriptor()` is deterministic, so the same descriptor always produces the same ID across restarts
- **Confidence** — lower for noisy sources (OCR, screen capture), higher for structured sources (git commits, API events)

## Asserting inferred relationships

After storing the raw observation, extract entities and relationships and
assert them into the graph.

```rust
// Inferred from the activity event above
brain.assert(
    "Alice",
    "works_on",
    "authentication.rs",
    0.85,
    Visibility::Private,
)?;

brain.assert(
    "authentication.rs",
    "contains",
    "OAuth token refresh",
    0.9,
    Visibility::Private,
)?;
```

With `AutoCreateWithCanonicalizer`, the brain creates entity nodes for
`"alice"`, `"authentication.rs"`, and `"oauth_token_refresh"` if they
don't already exist. The canonicalizer normalizes these names.

For complex events, use `assert_typed` to supply explicit entity types
when the predicate alone is ambiguous:

```rust
brain.assert_typed(
    ("Person", "Bob"),
    "reviewed",
    ("File", "authentication.rs"),
    0.9,
    Visibility::Team,
)?;
```

This bypasses predicate-based type inference and directly specifies that
Bob is a `Person` and `authentication.rs` is a `File`.

## Recalling activity context

### Memory + graph hybrid recall

Query the brain with natural language. The hybrid search combines
fingerprint matching, FTS, and graph traversal.

```rust
let result = brain.recall("who worked on authentication recently?", Visibility::Team)?;

// Memory hits — raw activity observations
for hit in &result.memory_hits {
    println!(
        "[{}] {} (wing={:?}, score={:.2})",
        hit.key, hit.content, hit.wing, hit.signal_score
    );
}

// Graph results — entity relationships
for triple in &result.graph.triples {
    println!(
        "  {} --{}-- {}",
        triple.subject_name, triple.predicate, triple.object_name
    );
}
```

The memory hits give you raw observations ("Alice opened
authentication.rs..."). The graph triples give you structured
relationships (`Alice --works_on-- authentication.rs`). Together they
answer both "what happened?" and "how do things relate?"

### Cross-wing recall with spectrogram

Find structurally similar activity across different domains. For example,
find communication patterns that resemble a development pattern:

```rust
let cross = brain.recall_cross_wing(
    "Alice refactored the payment module",
    Visibility::Private,
    5,
)?;

if let Some(seed) = &cross.seed_memory {
    println!("Seed: [{}] {}", seed.key, seed.content);
}

for resonant in &cross.resonant_memories {
    println!(
        "  Resonant: [{}] {} (score={:.2}, dimensions={:?})",
        resonant.memory.key,
        resonant.memory.content,
        resonant.resonance_score,
        resonant.matched_dimensions,
    );
}
```

This might surface a communication event like "Alice discussed payment
module changes in the architecture channel" — a different wing
(communication vs development) but structurally similar cognitive
pattern. The spectrogram matches on dimensions like entity density,
action type, and causal depth.

## Reinforcement loop

When a downstream consumer (agent task, search UI, daily summary) uses
recalled memories, reinforce them so they rank higher in future queries.

```rust
use spectral::ReinforceOpts;

// Daily summary agent used these memories
let useful_keys = vec![
    "activity-evt-1001".to_string(),
    "activity-evt-1042".to_string(),
];

let reinforced = brain.reinforce(ReinforceOpts {
    memory_keys: useful_keys,
    strength: 0.15, // slightly higher for explicit consumer usage
})?;

println!(
    "Reinforced {} memories, {} not found",
    reinforced.memories_reinforced,
    reinforced.memories_not_found.len()
);
```

The reinforcement loop creates a feedback signal:
- Memories that downstream consumers actually use get stronger
- Unused memories slowly decay (1% per week, capped at 50%)
- Over time, the brain naturally surfaces high-value observations

This is especially important for activity capture where the observation
volume is high but only a fraction of events are useful for any given
downstream query.

## Trade-offs

**Strengths:**
- Full provenance chain — every memory carries source, device ID, confidence, and timestamps
- Graph relationships enable multi-hop queries ("what files did people who reviewed the auth module also work on?")
- Cross-wing spectrogram finds non-obvious connections across activity domains
- Reinforcement naturally surfaces high-value observations without manual curation
- No embedding model needed — capture agents can run offline, on-device

**Limitations:**
- Entity extraction is the agent's responsibility — Spectral stores and retrieves, but the capture agent must decide what entities and relationships to assert (unless using `ingest_text()` with an LLM)
- Wing classification depends on regex rules — noisy or domain-specific activity may need custom rules to classify well
- Single-machine scale — practical up to ~10k memories today; high-volume activity sources may need a retention policy
- Spectrogram dimensions are fixed (7 cognitive dimensions) — not tunable per domain
- Graph queries are currently 2-hop BFS — deeper traversals aren't yet supported
