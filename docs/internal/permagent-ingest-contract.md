# Permagent -> Spectral Ingest Contract (DESIGN)

**Status:** Draft for review. DO NOT IMPLEMENT until approved.
**Date:** 2026-05-10

## Overview

Permagent currently emits nothing to Spectral. This document specifies
what Permagent will emit per conversation turn, which Spectral API it
will call, and how failures are handled.

---

## A. What Permagent emits per turn

### User message

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| content | String | yes | Raw user text |
| timestamp | DateTime<Utc> | yes | When the message was sent |
| session_id | String | yes | Permagent conversation ID |
| hub_id | String | yes | Hub this conversation belongs to |
| persona_id | String | yes | Aria persona variant in use |
| turn_index | u32 | yes | 0-based position in conversation |

### Assistant message

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| content | String | yes | Aria's response text |
| timestamp | DateTime<Utc> | yes | When the response was generated |
| session_id | String | yes | Same session as the user message |
| turn_index | u32 | yes | Follows user's turn_index |
| model | String | yes | Model that generated this (e.g., "claude-sonnet-4-6") |

### Tool calls

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| tool_name | String | yes | MCP tool or function name |
| args_summary | String | yes | Abbreviated args (NOT full JSON — redact secrets) |
| result_summary | String | yes | First 200 chars of result (redacted) |
| timestamp | DateTime<Utc> | yes | When the tool was invoked |
| session_id | String | yes | Parent session |
| turn_index | u32 | yes | Turn that triggered this tool call |
| duration_ms | u64 | no | Wall-clock time of tool execution |

### Wing/hall classification metadata

Permagent SHOULD pass:
- `wing: Option<String>` — the hub slug (e.g., "apollo", "alice").
  Maps directly to Spectral wings. If None, Spectral's TACT
  classifier derives wing from content.
- `hall_hint: Option<String>` — Permagent can hint at memory type
  ("fact", "preference", "decision") when it has structured intent
  signals (e.g., user explicitly stating a preference). If None,
  Spectral's hall classifier derives from content.

Wing override via `RememberOpts::wing` bypasses the classifier.
Hall hints are advisory — Spectral may override.

---

## B. Spectral API surface

### Proposed function

```rust
impl Brain {
    /// Ingest a single conversation turn from Permagent.
    pub fn ingest_turn(&self, turn: PermagentTurn) -> Result<IngestTurnResult, Error>;
}

pub struct PermagentTurn {
    /// Unique key for idempotency: "{session_id}:turn:{turn_index}:{role}"
    pub key: String,
    /// The text content (user message, assistant response, or tool summary)
    pub content: String,
    /// Role: "user", "assistant", or "tool"
    pub role: String,
    /// When this turn occurred
    pub timestamp: DateTime<Utc>,
    /// Maps to Spectral episode_id
    pub session_id: String,
    /// Wing override (hub slug). None = auto-classify.
    pub wing: Option<String>,
    /// Hall hint. None = auto-classify.
    pub hall_hint: Option<String>,
    /// Source identifier
    pub source: String, // "permagent"
}

pub struct IngestTurnResult {
    pub memory_id: String,
    pub wing: String,      // assigned (may differ from hint)
    pub hall: String,       // assigned
    pub signal_score: f64,
}
```

### Why not batch?

The bench uses `remember_with()` per turn. `ingest_turn` is a thin
wrapper that constructs `RememberOpts` from `PermagentTurn` fields.

Batching (e.g., `ingest_session(turns: Vec<PermagentTurn>)`) is a
future optimization if per-turn latency becomes a problem. The
current `SqliteStore` wraps batch inserts in a single transaction
internally, so per-turn calls are already reasonably fast (~0.5ms
each on empty brain, ~12ms on populated brain with fingerprinting).

### Sync vs async

`Brain::ingest_turn` is **sync** (like all Brain methods). Internally
it uses a dedicated tokio runtime to drive the async `MemoryStore`.
Permagent can call it from any thread without an async runtime.

If Permagent needs async: wrap in `tokio::task::spawn_blocking`.

### Error handling on Permagent side

| Error type | Permagent action |
|-----------|-----------------|
| `Error::Schema` (SQLite corruption) | Log error, skip turn, alert operator |
| `Error::Core` (identity/crypto) | Fatal — brain is broken. Halt agent loop. |
| `Error::Io` (disk full, permissions) | Log error, retry once after 1s. If still failing, skip turn. |
| Any other | Log and skip. Do NOT halt the agent loop for ingest failures. |

**Principle:** Ingest failure must never block the conversation.
The user is talking to Aria; Spectral is a background concern.

### Backpressure

Not needed at current scale. `remember_with` completes in <15ms
even on populated brains. Permagent's conversation rate (human
typing speed) is far below Spectral's ingest throughput.

If backpressure is ever needed: bounded channel with drop-oldest
policy. Losing an older turn is acceptable; blocking the
conversation is not.

---

## C. Identity model

### Sessions -> Episodes

| Permagent concept | Spectral concept |
|-------------------|-----------------|
| Conversation (session_id) | Episode (episode_id) |
| Hub | Wing |
| Persona (Aria variant) | Stored in source metadata, not a first-class Spectral concept |

A Permagent session_id maps directly to `RememberOpts::episode_id`.
All turns in a conversation share the same episode.

### User identity

**Single-user now.** Spectral's brain is per-user. Permagent runs
one brain per user. No multi-tenant concerns.

**Multi-user future:** Would require a `user_id` field on memories
and access control in recall. Not designed here — flag as future
work if needed.

### Aria's persona

Aria's persona variant (e.g., "default", "concise", "creative")
affects response style but not memory categorization. Store as
`source: "permagent:{persona_id}"` for provenance, but do not
use it for wing/hall classification.

---

## D. Failure recovery

### Spectral crash mid-conversation

Permagent continues the conversation normally. Turns that were
not yet ingested are lost. On Spectral restart, Permagent resumes
ingesting from the current turn forward.

**Acceptable?** Yes. Missing a few turns in a conversation is
low-impact. The next conversation's turns will be ingested
normally.

**Mitigation if needed:** Permagent could buffer last N turns in
memory and replay on reconnect. Not worth implementing unless
data loss is observed in practice.

### Permagent crash

Unflushed turns are lost. Since ingest is synchronous per-turn
(no batching queue), the only lost data is the turn that was being
processed when the crash occurred.

**Acceptable?** Yes. One turn at most.

### Idempotency

The key format `{session_id}:turn:{turn_index}:{role}` is
deterministic. `remember_with` uses this key for the memory.

**Current behavior:** `remember_with` creates a NEW memory each
time, even if the same key exists. There is no upsert-by-key.

**Required change:** `MemoryStore` needs an upsert semantic:
if a memory with the same key exists, skip (or update content
if different). Without this, a Permagent restart that replays
turns will create duplicate memories.

**Decision needed:** Skip-if-exists (simpler, sufficient) or
update-if-different (handles edits, more complex)?

---

## E. Performance contract

### Latency budget

| Operation | Target | Current measured |
|-----------|--------|-----------------|
| Single ingest (empty brain) | <5ms | ~0.4ms |
| Single ingest (populated, 1000 memories) | <50ms | ~12ms |
| Single ingest (populated, 10k memories) | <100ms | Not measured |

**Target: <100ms per turn.** This is well within human
conversation cadence (~2-5s between turns).

### Async ingest with bounded queue

Not needed at current scale. If latency exceeds 100ms:

1. First: profile. The bottleneck is likely fingerprint generation
   (O(peers-in-wing) per ingest).
2. If fingerprinting is the bottleneck: consider deferring
   fingerprint generation to a background task.
3. If still too slow: bounded async queue (capacity 100 turns,
   drop-oldest on overflow).

### Batch at session end vs streaming each turn

**Recommendation: stream each turn.** Reasons:
- Spectral's recall quality improves with more memories available
- Waiting until session end means Spectral can't help mid-conversation
- If Permagent crashes, all session turns are lost vs. only one

Batching is an optimization for throughput, not a requirement.

---

## Open questions

1. **Tool call granularity:** Should tool calls be separate memories
   or embedded in the assistant turn's content? Separate gives better
   retrieval granularity; embedded preserves context.

2. **Redaction:** Permagent already has redaction policies. Should
   Spectral's `RedactionPolicy` also run, or trust Permagent's output?

3. **Signal score override:** Should Permagent influence signal scores
   (e.g., marking user-stated preferences as high-signal)? Or let
   Spectral's `DefaultSignalScorer` handle it?
