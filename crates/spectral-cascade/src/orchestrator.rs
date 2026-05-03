//! Cascade orchestration: run layers in order with token budgets.

use std::collections::HashSet;

use crate::result::CascadeResult;
use crate::{Layer, LayerResult};

/// Configuration for the cascade.
#[derive(Debug, Clone)]
pub struct CascadeConfig {
    /// Total token budget across all layers.
    pub total_budget: usize,
    /// Stop cascade as soon as a layer returns Sufficient with
    /// confidence >= confidence_threshold. Default true.
    pub stop_on_sufficient: bool,
    /// Confidence threshold for early stopping. A layer returning
    /// Sufficient with confidence >= threshold halts the cascade.
    /// Default 0.85.
    pub confidence_threshold: f64,
}

impl Default for CascadeConfig {
    fn default() -> Self {
        Self {
            total_budget: 4096,
            stop_on_sufficient: true,
            confidence_threshold: 0.85,
        }
    }
}

/// The cascade orchestrator. Composes layers and runs them in order.
pub struct Cascade<'a> {
    layers: Vec<Box<dyn Layer + 'a>>,
    config: CascadeConfig,
}

impl<'a> Cascade<'a> {
    pub fn new(layers: Vec<Box<dyn Layer + 'a>>, config: CascadeConfig) -> Self {
        Self { layers, config }
    }

    /// Run the cascade. Layers execute in registration order.
    pub fn query(
        &self,
        query: &str,
    ) -> Result<CascadeResult, Box<dyn std::error::Error + Send + Sync>> {
        let mut layer_outcomes = Vec::new();
        let mut all_hits = Vec::new();
        let mut seen_ids = HashSet::new();
        let mut tokens_remaining = self.config.total_budget;
        let mut stopped_at = None;

        for layer in &self.layers {
            if tokens_remaining == 0 {
                break;
            }

            let result = layer.query(query, tokens_remaining)?;
            let tokens_used = result.tokens_used();

            // Collect unique hits
            for hit in result.hits() {
                if seen_ids.insert(hit.id.clone()) {
                    all_hits.push(hit.clone());
                }
            }

            let should_stop = self.config.stop_on_sufficient
                && matches!(
                    &result,
                    LayerResult::Sufficient { confidence, .. }
                        if *confidence >= self.config.confidence_threshold
                );
            let layer_id = layer.id();

            tokens_remaining = tokens_remaining.saturating_sub(tokens_used);
            layer_outcomes.push((layer_id, result));

            if should_stop {
                stopped_at = Some(layer_id);
                break;
            }
        }

        let total_tokens_used = self.config.total_budget - tokens_remaining;
        let max_confidence = layer_outcomes
            .iter()
            .map(|(_, r)| r.confidence())
            .fold(0.0_f64, f64::max);

        Ok(CascadeResult {
            layer_outcomes,
            merged_hits: all_hits,
            total_tokens_used,
            stopped_at,
            max_confidence,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Layer, LayerId, LayerResult};
    use spectral_ingest::MemoryHit;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn make_hit(id: &str) -> MemoryHit {
        MemoryHit {
            id: id.into(),
            key: id.into(),
            content: format!("content for {id}"),
            wing: None,
            hall: None,
            signal_score: 0.5,
            visibility: "private".into(),
            hits: 1,
            source: None,
            device_id: None,
            confidence: 1.0,
            created_at: None,
            last_reinforced_at: None,
        }
    }

    struct MockLayer {
        layer_id: LayerId,
        result_fn: Box<dyn Fn(usize) -> LayerResult + Send + Sync>,
        call_count: AtomicUsize,
    }

    impl MockLayer {
        fn new(id: LayerId, f: impl Fn(usize) -> LayerResult + Send + Sync + 'static) -> Self {
            Self {
                layer_id: id,
                result_fn: Box::new(f),
                call_count: AtomicUsize::new(0),
            }
        }
    }

    impl Layer for MockLayer {
        fn id(&self) -> LayerId {
            self.layer_id
        }

        fn query(
            &self,
            _query: &str,
            budget: usize,
        ) -> Result<LayerResult, Box<dyn std::error::Error + Send + Sync>> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            Ok((self.result_fn)(budget))
        }
    }

    #[test]
    fn cascade_runs_layers_in_order() {
        use std::sync::{Arc, Mutex};

        let order = Arc::new(Mutex::new(Vec::new()));
        let layers: Vec<Box<dyn Layer>> = vec![
            {
                let o = order.clone();
                Box::new(MockLayer::new(LayerId::L1, move |_| {
                    o.lock().unwrap().push(LayerId::L1);
                    LayerResult::Partial {
                        hits: vec![],
                        tokens_used: 10,
                        confidence: 0.3,
                    }
                }))
            },
            {
                let o = order.clone();
                Box::new(MockLayer::new(LayerId::L2, move |_| {
                    o.lock().unwrap().push(LayerId::L2);
                    LayerResult::Partial {
                        hits: vec![],
                        tokens_used: 10,
                        confidence: 0.3,
                    }
                }))
            },
            {
                let o = order.clone();
                Box::new(MockLayer::new(LayerId::L3, move |_| {
                    o.lock().unwrap().push(LayerId::L3);
                    LayerResult::Partial {
                        hits: vec![],
                        tokens_used: 10,
                        confidence: 0.3,
                    }
                }))
            },
        ];

        let cascade = Cascade::new(layers, CascadeConfig::default());
        let result = cascade.query("test").unwrap();
        assert_eq!(
            *order.lock().unwrap(),
            vec![LayerId::L1, LayerId::L2, LayerId::L3]
        );
        assert_eq!(result.layer_outcomes.len(), 3);
    }

    #[test]
    fn cascade_stops_on_sufficient_when_configured() {
        use std::sync::Arc;

        let l3_calls = Arc::new(AtomicUsize::new(0));
        let l3_calls_clone = l3_calls.clone();

        let cascade = Cascade::new(
            vec![
                Box::new(MockLayer::new(LayerId::L1, |_| LayerResult::Sufficient {
                    hits: vec![make_hit("m1")],
                    tokens_used: 50,
                    confidence: 0.95,
                })),
                Box::new(MockLayer::new(LayerId::L3, move |_| {
                    l3_calls_clone.fetch_add(1, Ordering::SeqCst);
                    LayerResult::Partial {
                        hits: vec![make_hit("m2")],
                        tokens_used: 50,
                        confidence: 0.5,
                    }
                })),
            ],
            CascadeConfig {
                stop_on_sufficient: true,
                ..Default::default()
            },
        );
        let result = cascade.query("test").unwrap();

        assert_eq!(result.stopped_at, Some(LayerId::L1));
        assert_eq!(result.merged_hits.len(), 1);
        // L3 should not have been called — L1 was Sufficient
        assert_eq!(l3_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn cascade_continues_on_partial() {
        let cascade = Cascade::new(
            vec![
                Box::new(MockLayer::new(LayerId::L1, |_| LayerResult::Partial {
                    hits: vec![make_hit("m1")],
                    tokens_used: 50,
                    confidence: 0.4,
                })),
                Box::new(MockLayer::new(LayerId::L3, |_| LayerResult::Partial {
                    hits: vec![make_hit("m2")],
                    tokens_used: 50,
                    confidence: 0.6,
                })),
            ],
            CascadeConfig::default(),
        );
        let result = cascade.query("test").unwrap();

        assert!(result.stopped_at.is_none());
        assert_eq!(result.merged_hits.len(), 2);
        assert_eq!(result.layer_outcomes.len(), 2);
    }

    #[test]
    fn cascade_continues_on_skipped() {
        let cascade = Cascade::new(
            vec![
                Box::new(MockLayer::new(LayerId::L1, |_| LayerResult::Skipped {
                    reason: "no facts".into(),
                    confidence: 0.0,
                })),
                Box::new(MockLayer::new(LayerId::L3, |budget| {
                    // Budget should be full since skip consumes 0 tokens
                    assert_eq!(budget, 4096);
                    LayerResult::Partial {
                        hits: vec![make_hit("m1")],
                        tokens_used: 100,
                        confidence: 0.7,
                    }
                })),
            ],
            CascadeConfig::default(),
        );
        let result = cascade.query("test").unwrap();
        assert_eq!(result.merged_hits.len(), 1);
        assert_eq!(result.total_tokens_used, 100);
    }

    #[test]
    fn cascade_respects_total_budget() {
        let cascade = Cascade::new(
            vec![
                Box::new(MockLayer::new(LayerId::L1, |_| LayerResult::Partial {
                    hits: vec![make_hit("m1")],
                    tokens_used: 80,
                    confidence: 0.3,
                })),
                Box::new(MockLayer::new(LayerId::L3, |budget| {
                    assert_eq!(budget, 20);
                    LayerResult::Partial {
                        hits: vec![make_hit("m2")],
                        tokens_used: 20,
                        confidence: 0.5,
                    }
                })),
                // Third layer should not be called — budget exhausted
                Box::new(MockLayer::new(LayerId::L5, |_| {
                    panic!("should not be called");
                })),
            ],
            CascadeConfig {
                total_budget: 100,
                ..Default::default()
            },
        );
        let result = cascade.query("test").unwrap();
        assert_eq!(result.total_tokens_used, 100);
        assert_eq!(result.merged_hits.len(), 2);
    }

    #[test]
    fn cascade_merged_hits_deduplicated() {
        let cascade = Cascade::new(
            vec![
                Box::new(MockLayer::new(LayerId::L1, |_| LayerResult::Partial {
                    hits: vec![make_hit("shared"), make_hit("only-l1")],
                    tokens_used: 50,
                    confidence: 0.4,
                })),
                Box::new(MockLayer::new(LayerId::L3, |_| LayerResult::Partial {
                    hits: vec![make_hit("shared"), make_hit("only-l3")],
                    tokens_used: 50,
                    confidence: 0.6,
                })),
            ],
            CascadeConfig::default(),
        );
        let result = cascade.query("test").unwrap();
        // "shared" appears in both but should be deduplicated
        assert_eq!(result.merged_hits.len(), 3);
        let ids: Vec<&str> = result.merged_hits.iter().map(|h| h.id.as_str()).collect();
        assert!(ids.contains(&"shared"));
        assert!(ids.contains(&"only-l1"));
        assert!(ids.contains(&"only-l3"));
    }

    #[test]
    fn cascade_stops_on_high_confidence_sufficient() {
        use std::sync::Arc;

        let l3_calls = Arc::new(AtomicUsize::new(0));
        let l3_clone = l3_calls.clone();

        let cascade = Cascade::new(
            vec![
                Box::new(MockLayer::new(LayerId::L1, |_| LayerResult::Sufficient {
                    hits: vec![make_hit("m1")],
                    tokens_used: 50,
                    confidence: 0.95,
                })),
                Box::new(MockLayer::new(LayerId::L3, move |_| {
                    l3_clone.fetch_add(1, Ordering::SeqCst);
                    LayerResult::Partial {
                        hits: vec![make_hit("m2")],
                        tokens_used: 50,
                        confidence: 0.5,
                    }
                })),
            ],
            CascadeConfig {
                confidence_threshold: 0.85,
                ..Default::default()
            },
        );
        let result = cascade.query("test").unwrap();
        assert_eq!(result.stopped_at, Some(LayerId::L1));
        assert_eq!(l3_calls.load(Ordering::SeqCst), 0);
        assert!((result.max_confidence - 0.95).abs() < f64::EPSILON);
    }

    #[test]
    fn cascade_continues_on_low_confidence_sufficient() {
        let cascade = Cascade::new(
            vec![
                Box::new(MockLayer::new(LayerId::L1, |_| LayerResult::Sufficient {
                    hits: vec![make_hit("m1")],
                    tokens_used: 50,
                    confidence: 0.5, // Below threshold
                })),
                Box::new(MockLayer::new(LayerId::L3, |_| LayerResult::Partial {
                    hits: vec![make_hit("m2")],
                    tokens_used: 50,
                    confidence: 0.7,
                })),
            ],
            CascadeConfig {
                confidence_threshold: 0.85,
                ..Default::default()
            },
        );
        let result = cascade.query("test").unwrap();
        // Should NOT have stopped — confidence 0.5 < threshold 0.85
        assert!(result.stopped_at.is_none());
        assert_eq!(result.merged_hits.len(), 2);
        assert_eq!(result.layer_outcomes.len(), 2);
    }

    #[test]
    fn cascade_result_includes_max_confidence() {
        let cascade = Cascade::new(
            vec![
                Box::new(MockLayer::new(LayerId::L1, |_| LayerResult::Partial {
                    hits: vec![make_hit("m1")],
                    tokens_used: 30,
                    confidence: 0.3,
                })),
                Box::new(MockLayer::new(LayerId::L2, |_| LayerResult::Partial {
                    hits: vec![make_hit("m2")],
                    tokens_used: 30,
                    confidence: 0.7,
                })),
                Box::new(MockLayer::new(LayerId::L3, |_| LayerResult::Partial {
                    hits: vec![make_hit("m3")],
                    tokens_used: 30,
                    confidence: 0.5,
                })),
            ],
            CascadeConfig::default(),
        );
        let result = cascade.query("test").unwrap();
        assert!((result.max_confidence - 0.7).abs() < f64::EPSILON);
    }
}
