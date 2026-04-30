# Conversational Memory

Using Spectral as the persistent memory layer for a chat-based agent.

## When to use this pattern

Your agent has multi-turn conversations and needs to:
- Remember facts from earlier in the conversation (or earlier conversations)
- Recall relevant context before generating a response
- Strengthen memories that proved useful, so they surface more readily next time
- Scope memory visibility per user or team

If your agent only needs single-session context, a sliding window over the transcript is simpler. Use Spectral when context must persist across sessions or when the agent needs to relate facts from different conversations.

## Architecture

```
  User message
       │
       ▼
┌─────────────┐    recall()     ┌──────────────┐
│  Agent loop  │───────────────▶│   Spectral   │
│             │◀───────────────│    Brain     │
│  (LLM call) │  memory_hits   │              │
│             │                │  ┌──────────┐│
│             │  remember_with()│  │ SQLite   ││
│             │───────────────▶│  │ + Kuzu   ││
│             │                │  └──────────┘│
│             │  reinforce()   │              │
│             │───────────────▶│              │
└─────────────┘                └──────────────┘
       │
       ▼
  Agent response
```

The agent loop follows three phases each turn:
1. **Recall** — query the brain for relevant context before calling the LLM
2. **Respond** — include recalled context in the system prompt, generate response
3. **Ingest + reinforce** — store the new turn, reinforce memories that helped

## Brain setup

Use `AutoCreateWithCanonicalizer` so the agent can assert relationships
between entities mentioned in conversation without requiring them in the
ontology upfront.

```rust
use std::sync::Arc;
use spectral::{Brain, BrainBuilder, EntityPolicy, Visibility};

let brain = Brain::builder()
    .data_dir("./chat-brain")
    .ontology_path("./ontology.toml")
    .entity_policy(EntityPolicy::AutoCreateWithCanonicalizer(
        Arc::new(|mention: &str| mention.trim().to_lowercase()),
    ))
    .build()?;
```

The canonicalizer lowercases and trims entity mentions so that "Alice",
"alice", and " Alice " all resolve to the same node in the graph.

## Ingesting chat turns

Store each user message as a memory. Use `remember_with` to attach
provenance metadata.

```rust
use spectral::{RememberOpts, Visibility};

// Ingest a user message
let user_msg = "We decided to use PostgreSQL for the analytics database";
let result = brain.remember_with(
    "chat-turn-42",
    user_msg,
    RememberOpts {
        source: Some("chat".into()),
        visibility: Visibility::Team,
        ..Default::default()
    },
)?;

println!(
    "Stored in wing={:?}, hall={:?}, fingerprints={}",
    result.wing, result.hall, result.fingerprints_created
);
```

The key (`"chat-turn-42"`) should be unique per message. A UUID or
monotonic counter works well. Spectral classifies the content into a wing
(topic area) and hall (memory type) automatically using regex rules.

### Asserting relationships from conversation

When the agent extracts a structured fact from the conversation, assert it
into the knowledge graph:

```rust
brain.assert(
    "Acme Corp",
    "uses_technology",
    "PostgreSQL",
    0.9,
    Visibility::Team,
)?;
```

With `AutoCreateWithCanonicalizer`, entities that don't exist in the
ontology are created automatically. The canonicalizer ensures consistent
naming.

## Recalling context before responding

Before calling the LLM, query the brain for relevant context:

```rust
let query = "what database are we using for analytics?";
let result = brain.recall(query, Visibility::Team)?;

// Memory hits from fingerprint + FTS search
for hit in &result.memory_hits {
    println!("[{}] {} (score: {:.2})", hit.key, hit.content, hit.signal_score);
}

// Graph neighborhood — related entities and triples
for triple in &result.graph.triples {
    println!(
        "{} --{}-- {}",
        triple.subject_name, triple.predicate, triple.object_name
    );
}

// Inject into system prompt
let context = &result.tact.context_block;
let system_prompt = format!(
    "You are a helpful assistant.\n\nRelevant context:\n{context}"
);
```

`recall()` performs hybrid search: fingerprint matching, wing-scoped
search, FTS fallback, and graph neighborhood traversal. The
`context_block` is a pre-formatted string suitable for system prompt
injection.

## Reinforcing useful recalls

After the agent responds, reinforce the memories that were actually
useful. This increases their `signal_score` so they rank higher in future
recalls.

```rust
use spectral::ReinforceOpts;

// The agent used these memories to answer the user's question
let useful_keys: Vec<String> = result
    .memory_hits
    .iter()
    .take(3) // top 3 were included in context
    .map(|h| h.key.clone())
    .collect();

let reinforced = brain.reinforce(ReinforceOpts {
    memory_keys: useful_keys,
    strength: 0.1, // default increment
})?;

println!("Reinforced {} memories", reinforced.memories_reinforced);
```

Over time, frequently-useful memories accumulate higher signal scores
while unused memories decay (1% per week, capped at 50%). This creates a
natural relevance gradient without explicit curation.

## Visibility for multi-user scenarios

Spectral's four-level visibility system (`Private`, `Team`, `Org`,
`Public`) controls what each query can see:

```rust
// Alice stores a private note
brain.remember_with(
    "alice-note-1",
    "My quarterly review is next Thursday",
    RememberOpts {
        source: Some("chat".into()),
        visibility: Visibility::Private,
        ..Default::default()
    },
)?;

// Bob stores a team-visible decision
brain.remember_with(
    "bob-note-1",
    "Team agreed to ship v2 by end of March",
    RememberOpts {
        source: Some("chat".into()),
        visibility: Visibility::Team,
        ..Default::default()
    },
)?;

// A Team-scoped query sees Bob's note but not Alice's private note
let team_results = brain.recall("quarterly plans", Visibility::Team)?;

// A Private-scoped query sees everything
let all_results = brain.recall("quarterly plans", Visibility::Private)?;
```

Visibility is enforced on read, not write. A `Private` query sees all
visibility levels. A `Team` query sees `Team`, `Org`, and `Public` but
not `Private`. This lets you scope agent context to the appropriate
audience.

## Trade-offs

**Strengths:**
- No embedding model needed — works offline, no GPU, no API calls for recall
- Hybrid search catches both exact topical matches (fingerprints) and fuzzy keyword matches (FTS)
- Graph traversal answers "how does X relate to Y?" questions that pure vector search misses
- Reinforcement creates a natural relevance signal without manual curation

**Limitations:**
- Fingerprint search is vocabulary-dependent — paraphrase queries where none of the original words appear will rely on FTS fallback rather than fingerprint matching
- Single-machine scale today (practical up to ~10k memories)
- No built-in conversation windowing — the agent must decide what to store and what to discard
- Graph relationships must be explicitly asserted; Spectral doesn't automatically extract entities from free text (unless you use `ingest_text()` with an LLM client)
