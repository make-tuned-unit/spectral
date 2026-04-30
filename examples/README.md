# Spectral Integration Examples

Real-world integration patterns for using Spectral.

## Patterns

- **[Conversational memory](chat-memory.md)** — Using Spectral as the memory layer for a chat-based agent. Shows ingest of chat turns, recall for context, reinforcement based on user response.

- **[Activity capture](activity-capture.md)** — Wiring a desktop activity capture system into Spectral. Shows how to use `remember_with`, `assert`, `recall`, and `reinforce` together for an agent that learns from observed user activity.

For runnable code examples, see:
- `crates/spectral/examples/quickstart.rs` — minimal end-to-end demo
- `crates/spectral-graph/examples/try_brain.rs` — full smoke test of all APIs
