# Shape-Routed Actor Strategies Proposal

**Branch:** `feat/bench-shape-routed-actors`
**Status:** Draft — awaiting review before implementation

## Motivation

Tonight's bench (73.3% overall) shows the actor's generic prompt is the limiting factor for ~75% of remaining failures. The existing `QuestionType` classifier does meaningful work at the retrieval layer — but the actor receives no shape information and uses a single prompt for all 6 LongMemEval categories.

This proposal extends `QuestionType` with sub-shapes, routes actor prompts per shape, allows per-question retrieval path override, and emits routing telemetry.

**Target lift:** +10–15pp overall on next bench checkpoint (→ 80%+).

---

## 1. Extended QuestionType Enum

8 variants. Existing top-level shapes preserved; sub-shapes added for the heterogeneous General/Counting/Factual buckets.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionType {
    /// "How many", "total" — exhaustive session scan, no recency signal.
    Counting,
    /// "How many ... currently/still" — current count, recency-priority.
    CountingCurrentState,
    /// Date arithmetic, ordering, duration.
    Temporal,
    /// "What is", "where", "who" — single-entity retrieval.
    Factual,
    /// "What is my current X" — most-recent-wins factual.
    FactualCurrentState,
    /// "Suggest/recommend/tips/advice" — preference inference.
    GeneralPreference,
    /// "Remind me/going back to/we discussed" — assistant recall.
    GeneralRecall,
    /// Catch-all fallback.
    General,
}
```

**Rationale per variant:**

| Variant | Why it exists |
|---------|---------------|
| `Counting` | Existing. Exhaustive session scan needed. |
| `CountingCurrentState` | "How many X do I currently have?" needs recency, not historical total. Split from Counting by recency keywords. |
| `Temporal` | Existing. Date-table reasoning needed. |
| `Factual` | Existing. Single-entity direct answer. |
| `FactualCurrentState` | "What is my current Y?" — most recent session wins. Split from Factual by recency keywords. |
| `GeneralPreference` | "Any suggestions/tips/advice?" — preference inference from context. 100% accuracy sub-split on 31-question General sample. |
| `GeneralRecall` | "Remind me/going back to" — assistant recall of prior conversation. 100% accuracy sub-split on General sample. |
| `General` | Catch-all. Today's 9-instruction generic prompt. Safety floor. |

---

## 2. Extended `classify` Function

Two-level decision tree. Level 1 is the existing classifier (unchanged). Level 2 adds sub-gates.

```
classify(question):
  q = question.to_lowercase()

  // Level 1: existing top-level classifier (unchanged)
  if temporal-counting pattern  → base = Temporal
  if counting pattern           → base = Counting
  if temporal pattern           → base = Temporal
  if factual pattern            → base = Factual
  else                          → base = General

  // Level 2: sub-gates
  match base:
    Counting:
      if recency_pattern(q) → CountingCurrentState
      else → Counting

    Factual:
      if recency_pattern(q) → FactualCurrentState
      else → Factual

    General:
      if preference_pattern(q) → GeneralPreference
      if recall_pattern(q)     → GeneralRecall
      else → General

    Temporal → Temporal  (no sub-gate)
```

**Sub-gate regex patterns:**

- **recency:** `\b(currently|right now|most recent|latest|newest|do i still|now)\b`
- **preference:** `\b(suggest|recommend|tips?|advice|recommendations?|what should i)\b` OR `\bany (tips?|advice|suggestions?|ideas?|thoughts?|recommendations?)\b`
- **recall:** `\b(remind me|going back to|previous|earlier conversation|we (discussed|talked about)|can you remind me)\b`

Patterns derived from per-question analysis with 100% accuracy on the General sub-split (31-question sample) and verified on Counting/Factual recency questions.

---

## 3. Per-Shape Actor Strategy

Each variant maps to a prompt template (markdown file in `src/prompts/`) and a retrieval path.

| Variant | Prompt template | Retrieval path | Rationale |
|---------|----------------|----------------|-----------|
| `Counting` | `counting_enumerate.md` | cascade | Exhaustive session scan; cascade's L1/L2 diversity helps |
| `CountingCurrentState` | `counting_current_state.md` | cascade | Recency-priority count |
| `Temporal` | `temporal.md` | **topk_fts** | Cascade hurts temporal by −15pp; topk_fts recovers |
| `Factual` | `factual_direct.md` | cascade | Single-entity, focused retrieval |
| `FactualCurrentState` | `factual_current_state.md` | cascade | Most-recent-wins factual |
| `GeneralPreference` | `preference.md` | cascade | Preference inference from context |
| `GeneralRecall` | `assistant_recall.md` | cascade | Find and quote prior conversation |
| `General` | `generic_fallback.md` | cascade | Today's 9-instruction prompt verbatim |

All templates share these invariant elements from today's generic prompt:
- Today's date statement
- Memory format explanation (session headers, turn labels)
- "Don't know" fallback rule (current instruction 3)
- Concise answer preference

Each template adds shape-specific reasoning instructions (see Section 5).

---

## 4. Actor Trait Signature Change

Breaking change. `QuestionType` added as parameter:

```rust
pub trait Actor: Send + Sync {
    fn answer(
        &self,
        question: &str,
        question_date: &str,
        memories: &[String],
        shape: QuestionType,
    ) -> Result<String>;
    fn name(&self) -> &str;
}
```

Impact on implementations:
- **`AnthropicActor`** — uses `shape` to select prompt template via `shape.prompt_template()` → `include_str!`
- **`MockActor`** — adds `_shape: QuestionType`, ignores it
- **`FailingActor`** (test) — adds `_shape: QuestionType`, ignores it
- **`FailNthActor`** (test) — adds `_shape: QuestionType`, ignores it

---

## 5. Prompt Template Summaries

All 8 templates live in `crates/spectral-bench-accuracy/src/prompts/` and are loaded via `include_str!`.

### `counting_enumerate.md`
Specialized instruction: "Scan EVERY session header. For each match, list the item explicitly with its source session. Deduplicate before counting. State the final count last."

### `counting_current_state.md`
Specialized instruction: "When the question asks about a current count ('currently', 'still', 'now'), the answer is the most recent state, not the historical total. Identify the most recent session that mentions the count, and use that as the answer."

### `temporal.md`
Specialized instruction: "Before answering, identify the session dates of every event mentioned in the question. List them with their dates. Then perform the requested calculation. Show the values used."

### `factual_direct.md`
Specialized instruction: "State the answer in as few words as possible. If the answer is a name, state just the name. If a number, just the number. No qualifiers."

### `factual_current_state.md`
Specialized instruction: "Identify the most recent session mentioning the entity. The value from that session is the answer, even if older sessions mention different values."

### `preference.md`
Specialized instruction: "The question asks for suggestions or recommendations. Identify the user's relevant preferences from the conversation (explicit statements OR implicit signals from past activities). Tailor your suggestion to those preferences."

### `assistant_recall.md`
Specialized instruction: "The question refers to a prior conversation. Find the relevant session and quote or paraphrase what was said. If not found, state clearly what is present in the sessions."

### `generic_fallback.md`
Today's existing 9-instruction prompt verbatim from PR #84. Safety floor for unclassified questions.

---

## 6. Retrieval Path: Per-Question Override

Currently `eval.rs:246` branches on `self.config.use_cascade` (run-level) and `self.config.retrieval_path` (run-level). This PR lifts retrieval path selection to per-question:

```rust
impl QuestionType {
    pub fn retrieval_path(&self) -> RetrievalPath {
        match self {
            Self::Temporal => RetrievalPath::TopkFts,
            _ => RetrievalPath::Cascade,
        }
    }
}
```

The `eval_question` method computes `QuestionType::classify(&question.question)` and uses `qtype.retrieval_path()` instead of `self.config.retrieval_path`. The run-level `use_cascade` flag becomes a feature gate: when `use_cascade = false`, all shapes fall back to `self.config.retrieval_path` (preserving backward compat for non-cascade runs).

---

## 7. Telemetry

New struct on `QuestionResult`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyTelemetry {
    pub shape: String,              // e.g. "counting_current_state"
    pub prompt_template: String,    // e.g. "counting_current_state.md"
    pub retrieval_path_chosen: String, // e.g. "topk_fts"
}
```

Added to `QuestionResult`:
```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub strategy_telemetry: Option<StrategyTelemetry>,
```

Populated in `eval_question` after classification. Feeds downstream attribution analysis and per-strategy ablation reporting (backlog items #15, #16).

---

## 8. File Changes Summary

| File | Change |
|------|--------|
| `src/retrieval.rs` | Extend `QuestionType` enum (8 variants), extend `classify()` with sub-gates, add `retrieval_path()` and `prompt_template()` methods, add `cascade_profile()` for new variants |
| `src/actor.rs` | Add `shape: QuestionType` to `Actor` trait and all impls, `AnthropicActor` selects prompt via `shape.prompt_template()` |
| `src/eval.rs` | Per-question retrieval path via `qtype.retrieval_path()`, populate `StrategyTelemetry`, pass `shape` to actor |
| `src/report.rs` | Add `StrategyTelemetry` struct and field on `QuestionResult` |
| `src/prompts/` (new dir) | 8 `.md` prompt template files |

---

## 9. Out of Scope

- No new Spectral core PR. Classifier stays in spectral-bench-accuracy.
- No config-file-driven gate registry. Inline regex is consistent with existing style.
- No L2 episode integration, co-retrieval boost, or compiled-truth boost.
- No judge rubric changes.
- No `--max-results` per shape. Reuse existing values.
- No prompt template hot-reloading. `include_str!` at compile time.

---

## 10. Verification Plan

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace --lib --tests
```

Plus a dry-run on a sample question per category to verify routing telemetry is populated.
