//! Accuracy benchmarks for Spectral agent memory.
//!
//! Runs the LongMemEval_S benchmark (500 questions across 6 categories)
//! against Spectral and produces published-quality accuracy numbers
//! comparable to Mem0, Letta, Zep, Memanto, and other agent memory systems.
//!
//! # Architecture
//!
//! 1. **Dataset** — loads LongMemEval_S questions with conversation haystacks
//! 2. **Ingest** — converts conversations into Spectral memories (per-turn or per-session)
//! 3. **Retrieval** — queries Spectral recall for each question
//! 4. **Actor** — LLM synthesizes an answer from retrieved memories
//! 5. **Judge** — LLM grades the answer against ground truth
//! 6. **Report** — aggregates scores by category with failure analysis
//!
//! # Quick start
//!
//! ```bash
//! export ANTHROPIC_API_KEY=sk-...
//! spectral-bench-accuracy run --dataset longmemeval_s.json --max-questions 10
//! ```

pub mod actor;
pub mod dataset;
pub mod eval;
pub mod ingest;
pub mod judge;
pub mod report;
pub mod retrieval;

pub use actor::{Actor, AnthropicActor, MockActor};
pub use dataset::{Category, Dataset, Question};
pub use eval::{AccuracyEval, EvalConfig};
pub use judge::{AnthropicJudge, GradeResult, Judge, MockJudge};
pub use report::EvalReport;
pub use retrieval::RetrievalConfig;
