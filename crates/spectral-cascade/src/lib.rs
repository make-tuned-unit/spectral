//! Recognition context and result types for Spectral's retrieval pipeline.
//!
//! The retrieval pipeline is a single-pass design: TACT tiered search
//! (fingerprint → wing → FTS fallback) supplemented by raw FTS when TACT
//! returns fewer than K results, followed by a unified re-ranking pipeline
//! (signal score blend, ambient boost, declarative density, co-retrieval,
//! recency decay, entity cluster boost, episode diversity cap, context chain
//! dedup), auto-reinforce, and retrieval event logging.
//!
//! There are no layers, no ordered fallthrough, and no early stopping.
//! All re-ranking stages always run on every query.
//!
//! This crate provides [`RecognitionContext`] (ambient state for
//! context-conditional scoring) and [`CascadeResult`] (pipeline output).

pub mod context;
pub mod result;

pub use context::RecognitionContext;
