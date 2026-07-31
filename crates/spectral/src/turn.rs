//! The agent-turn contract: typed inputs, a delivery receipt, and deferred
//! outcome-bearing feedback.
//!
//! # Why this exists
//!
//! Every other recall entry point auto-reinforces **every returned hit** at
//! retrieval time (`cascade_layers.rs`, gated on the new `write_back` flag).
//! That credits *exposure*, not usefulness: all `k` hits are strengthened
//! before the consumer has filtered them, and the co-access log records the
//! full returned set. Measured consequence on Permagent's real workload:
//! 728/744 events returned roughly the same 40 memories, and the co-retrieval
//! edges built from those events made top-5 relevance ~3–4.5:1 *worse*
//! (`docs/internal/LAST_LOOK.md`, `docs/internal/tickets/coretrieval-regression.md`).
//!
//! A turn separates the two halves:
//!
//! 1. [`Brain::turn`](crate::Brain::turn) retrieves **read-only** and returns a
//!    [`TurnReceipt`] naming exactly what was delivered, in rank order.
//! 2. [`Brain::record_turn_outcome`](crate::Brain::record_turn_outcome) is
//!    called after the actor has finished, and reinforces **only** the
//!    memories reported [`MemoryOutcome::Used`]. `Wrong` and `Ignored` never
//!    strengthen anything.
//!
//! It also stops conflating two different questions. Recall answers "what is
//! relevant to this query"; recognition answers "have I encountered this
//! before". Feeding questions to a content re-encounter engine is why real-query
//! recognition measured 0.9% wing precision / 10.9% cascade agreement
//! (`docs/internal/RECOGNITION_BASELINE.md`). Here they are separate typed
//! fields: recognition runs over [`TurnRequest::observations`] and **never**
//! implicitly over the query.
//!
//! # Scope of this version
//!
//! [`TurnPolicyVersion::V1`] pins retrieval to the same cascade pipeline the
//! existing `recall_cascade_scoped` uses, with write-back disabled — so hit
//! sets are identical to today's and only the *feedback timing* changes.
//!
//! # The outcome ledger
//!
//! Every delivery is persisted to `turn_events` / `turn_members` — each member
//! with its rank and its outcome (`used` / `wrong` / `ignored` /
//! `unreported`). This was initially deferred as "auditability, not behavior",
//! which was wrong: without it `Wrong` and `Ignored` live only in a returned
//! struct and vanish, so *"delivered repeatedly and never used"* — the one
//! question negative evidence exists to answer — is structurally
//! unanswerable. `Unreported` is a first-class state: a turn whose outcome is
//! never committed still recorded an exposure.
//!
//! Two properties are load-bearing:
//!
//! * **Occurrence identity ≠ delivery digest.** Two identical deliveries are
//!   two real turns. Collapsing them would undercount exposure, which is the
//!   quantity the ledger exists to measure.
//! * **The ledger write and the `Used` reinforcement are one transaction**, and
//!   replaying a commit is a no-op. Reinforcement is additive, so a retry that
//!   re-applied it would silently double-count the same evidence.
//!
//! The ledger is **evidence, not policy**. Nothing here decays, merges, or
//! forgets anything automatically — see [`Brain::memory_outcome_evidence`].

use std::sync::atomic::{AtomicU64, Ordering};

use spectral_graph::cascade_layers::CascadePipelineConfig;
use spectral_graph::RecognitionContext;
use spectral_ingest::MemoryHit;
use spectral_recognition::RecognitionResult;

use crate::{Brain, Error, Visibility};

/// Versioned retrieval policy. The version is part of the receipt so a result
/// can name the exact executable configuration that produced it — the
/// distinction between "a benchmark number" and "a product number".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum TurnPolicyVersion {
    /// Today's cascade pipeline defaults, with retrieval-time write-back
    /// disabled and reinforcement deferred to the outcome commit.
    #[default]
    V1,
}

impl TurnPolicyVersion {
    /// Stable string form for receipts, logs, and published results.
    pub fn as_str(&self) -> &'static str {
        match self {
            TurnPolicyVersion::V1 => "v1",
        }
    }

    fn retrieval_config(&self) -> CascadePipelineConfig {
        match self {
            TurnPolicyVersion::V1 => CascadePipelineConfig {
                // The whole point: no reinforcement, no event log, at read time.
                write_back: false,
                ..Default::default()
            },
        }
    }
}

/// What one agent turn asks of memory.
///
/// `query` and `observations` are distinct on purpose — see the module docs.
#[derive(Debug, Clone)]
pub struct TurnRequest<'a> {
    /// The recall query, if this turn retrieves. `None` runs recognition only.
    pub query: Option<&'a str>,
    /// Recognition stimuli: things the agent just *encountered* (user message,
    /// tool output, file content). Never inferred from `query`.
    pub observations: &'a [&'a str],
    /// Visibility boundary for retrieval.
    pub visibility: Visibility,
    /// Ambient context (recent activity, focus wing, session id, `now`).
    pub context: RecognitionContext,
    /// Retrieval policy version.
    pub policy: TurnPolicyVersion,
}

impl<'a> TurnRequest<'a> {
    /// A recall-only turn with default policy and an empty ambient context.
    pub fn query(query: &'a str, visibility: Visibility) -> Self {
        Self {
            query: Some(query),
            observations: &[],
            visibility,
            context: RecognitionContext::empty(),
            policy: TurnPolicyVersion::default(),
        }
    }

    /// Attach recognition stimuli.
    pub fn with_observations(mut self, observations: &'a [&'a str]) -> Self {
        self.observations = observations;
        self
    }

    /// Attach an ambient context.
    pub fn with_context(mut self, context: RecognitionContext) -> Self {
        self.context = context;
        self
    }
}

/// One delivered hit, in the rank position the caller received it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveredHit {
    /// 0-based rank as delivered.
    pub rank: usize,
    /// Memory id.
    pub id: String,
    /// Memory key — the handle `reinforce` and outcome commits use.
    pub key: String,
}

/// Identifies exactly what a turn delivered, so a later outcome can be
/// attributed to it. Hold this between `turn` and `record_turn_outcome`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnReceipt {
    /// Identifies THIS turn occurrence. Unique per call, even when two turns
    /// deliver byte-identical results.
    ///
    /// Deliberately distinct from [`Self::delivery_digest`]: two identical
    /// deliveries are two separate real-world turns, and collapsing them would
    /// undercount exposure — the exact quantity the outcome ledger exists to
    /// measure. Use this to commit and to dedupe *retries*, not to dedupe
    /// repeat deliveries.
    pub id: String,
    /// Content-addressed digest of what was delivered — policy, query hash, and
    /// ordered member ids. Two turns returning the same thing share this, which
    /// is how repeat deliveries stay detectable.
    pub delivery_digest: String,
    /// Blake3 of the query text; `None` for recognition-only turns.
    pub query_hash: Option<String>,
    /// Session id carried from the request context, if any.
    pub session_id: Option<String>,
    /// Wing of the top hit, mirroring what retrieval-time logging recorded.
    pub wing: Option<String>,
    /// Everything delivered, in rank order.
    pub delivered: Vec<DeliveredHit>,
    /// Policy that produced this delivery.
    pub policy: TurnPolicyVersion,
}

impl TurnReceipt {
    /// Whether `key` was actually delivered by this turn. Outcome commits are
    /// rejected for keys that were not — an outcome about an undelivered
    /// memory is a caller bug, and silently accepting it would reintroduce
    /// unattributed reinforcement.
    pub fn delivered_key(&self, key: &str) -> bool {
        self.delivered.iter().any(|d| d.key == key)
    }
}

/// What the caller did with a delivered memory.
///
/// Only `Used` reinforces. `Wrong` and `Ignored` are recorded as *negative
/// evidence* and must never strengthen a memory or build an association —
/// that asymmetry is the correctness core of this contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryOutcome {
    /// The agent used this memory in its response.
    Used,
    /// The memory was retrieved and was actively misleading or incorrect.
    Wrong,
    /// Delivered but not used.
    Ignored,
}

impl MemoryOutcome {
    /// Whether this outcome earns reinforcement.
    pub fn reinforces(&self) -> bool {
        matches!(self, MemoryOutcome::Used)
    }
}

/// The result of a turn: what was delivered, plus the receipt to report on.
#[derive(Debug, Clone)]
pub struct TurnResult {
    /// Receipt naming the delivery. Pass to `record_turn_outcome`.
    pub receipt: TurnReceipt,
    /// Retrieved hits in rank order. Empty for recognition-only turns.
    pub hits: Vec<MemoryHit>,
    /// One verdict per entry in `observations`, positionally aligned.
    pub recognition: Vec<RecognitionResult>,
}

/// Receipt for a committed outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutcomeReceipt {
    /// The turn this outcome was attributed to.
    pub turn_id: String,
    /// Keys reinforced (the `Used` set).
    pub reinforced: Vec<String>,
    /// Keys recorded as delivered-but-not-reinforced (`Wrong` / `Ignored`).
    pub not_reinforced: Vec<String>,
}

/// Reinforcement applied to a memory the actor actually used.
///
/// Ten times the retrieval-time auto-reinforce nudge (0.01), because it is
/// paid out on roughly the fraction of hits that get used rather than on all
/// of them — the signal is far sparser and far better attributed.
const USED_REINFORCE_STRENGTH: f64 = 0.1;

impl Brain {
    /// Run one agent turn: read-only retrieval plus recognition over the
    /// turn's observations, returning a [`TurnReceipt`] for later attribution.
    ///
    /// Nothing is reinforced and no retrieval event is logged here. Call
    /// [`Brain::record_turn_outcome`] once the actor has finished to commit
    /// what was actually used. A turn that is never committed leaves memory
    /// state completely unchanged.
    pub fn turn(&self, request: &TurnRequest<'_>) -> Result<TurnResult, Error> {
        let config = request.policy.retrieval_config();

        let hits = match request.query {
            Some(query) => {
                self.inner
                    .recall_cascade_scoped(query, &request.context, &config, request.visibility)?
                    .merged_hits
            }
            None => Vec::new(),
        };

        // Recognition runs ONLY over observations — never over the query.
        let recognition = request
            .observations
            .iter()
            .map(|stimulus| self.inner.recognize(stimulus))
            .collect::<Result<Vec<_>, _>>()?;

        let delivered: Vec<DeliveredHit> = hits
            .iter()
            .enumerate()
            .map(|(rank, hit)| DeliveredHit {
                rank,
                id: hit.id.clone(),
                key: hit.key.clone(),
            })
            .collect();

        let query_hash = request.query.map(spectral_ingest::hash_query);

        // Content-addressed over everything that defines the delivery. Two
        // turns returning the same thing share this digest.
        let delivery_digest = spectral_ingest::hash_query(&format!(
            "{}\u{1}{}\u{1}{}",
            request.policy.as_str(),
            query_hash.as_deref().unwrap_or(""),
            delivered
                .iter()
                .map(|d| d.id.as_str())
                .collect::<Vec<_>>()
                .join(",")
        ));

        // Occurrence id: unique per call. Derived from the digest plus a
        // monotonic process-local counter and the wall clock, so two identical
        // deliveries are still two distinct ledger rows.
        static TURN_SEQ: AtomicU64 = AtomicU64::new(0);
        let seq = TURN_SEQ.fetch_add(1, Ordering::Relaxed);
        let delivered_at = chrono::Utc::now();
        let id = spectral_ingest::hash_query(&format!(
            "{delivery_digest}\u{1}{}\u{1}{seq}",
            delivered_at.timestamp_nanos_opt().unwrap_or_default(),
        ));

        let receipt = TurnReceipt {
            id,
            delivery_digest,
            query_hash,
            session_id: request.context.session_id.clone(),
            wing: hits.first().and_then(|h| h.wing.clone()),
            delivered,
            policy: request.policy,
        };

        // Persist the delivery so exposure survives the call. Without this,
        // `Wrong`/`Ignored`/never-adjudicated members exist only in memory and
        // "delivered repeatedly and never used" is unanswerable.
        let delivery = spectral_ingest::TurnDelivery {
            occurrence_id: receipt.id.clone(),
            delivery_digest: receipt.delivery_digest.clone(),
            query_hash: receipt.query_hash.clone(),
            session_id: receipt.session_id.clone(),
            policy: receipt.policy.as_str().to_string(),
            delivered_at: delivered_at.to_rfc3339(),
            members: receipt
                .delivered
                .iter()
                .map(|d| (d.rank, d.id.clone(), d.key.clone()))
                .collect(),
        };
        self.inner.record_turn_delivery(&delivery)?;

        Ok(TurnResult {
            receipt,
            hits,
            recognition,
        })
    }

    /// Commit what the actor did with a turn's hits.
    ///
    /// Reinforces exactly the [`MemoryOutcome::Used`] keys and logs one
    /// retrieval event whose member set is that used set — so co-access mining
    /// learns from usefulness rather than exposure.
    ///
    /// Returns [`Error::Schema`] if an outcome names a key this turn did not
    /// deliver: an unattributable outcome is a caller bug, and accepting it
    /// would reopen exactly the unearned-credit path this contract closes.
    pub fn record_turn_outcome(
        &self,
        receipt: &TurnReceipt,
        outcomes: &[(&str, MemoryOutcome)],
    ) -> Result<OutcomeReceipt, Error> {
        for (key, _) in outcomes {
            if !receipt.delivered_key(key) {
                return Err(Error::Schema(format!(
                    "outcome for key {key:?} which turn {} did not deliver",
                    receipt.id
                )));
            }
        }

        let mut reinforced = Vec::new();
        let mut not_reinforced = Vec::new();
        let mut ledger = Vec::with_capacity(outcomes.len());
        for (key, outcome) in outcomes {
            ledger.push((
                (*key).to_string(),
                match outcome {
                    MemoryOutcome::Used => spectral_ingest::LedgerOutcome::Used,
                    MemoryOutcome::Wrong => spectral_ingest::LedgerOutcome::Wrong,
                    MemoryOutcome::Ignored => spectral_ingest::LedgerOutcome::Ignored,
                },
            ));
            if outcome.reinforces() {
                reinforced.push((*key).to_string());
            } else {
                not_reinforced.push((*key).to_string());
            }
        }

        // Ledger write + reinforcement of `Used`, in ONE transaction and
        // idempotent on the occurrence id. Replaying a commit is a no-op, so a
        // retrying caller cannot double-count reinforcement — the defect the
        // previous best-effort path explicitly admitted to.
        self.inner
            .commit_turn_outcomes(&receipt.id, &ledger, USED_REINFORCE_STRENGTH)?;

        // The event carries ONLY the used set. An empty used set still logs an
        // event: "this query helped with nothing" is real evidence, and
        // dropping it would bias the log toward successful turns.
        let used_ids: Vec<&str> = receipt
            .delivered
            .iter()
            .filter(|d| reinforced.contains(&d.key))
            .map(|d| d.id.as_str())
            .collect();

        let event = spectral_ingest::RetrievalEvent {
            query_hash: receipt.query_hash.clone().unwrap_or_default(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            memory_ids_json: serde_json::to_string(&used_ids).unwrap_or_default(),
            method: format!("turn:{}", receipt.policy.as_str()),
            wing: receipt.wing.clone(),
            question_type: None,
            session_id: receipt.session_id.clone(),
        };

        self.inner
            // Reinforcement already happened atomically with the ledger write
            // above; pass no keys so this only logs the used-set event for
            // co-access mining. Passing them again would double-count.
            .commit_outcome(Vec::new(), event, USED_REINFORCE_STRENGTH)?;

        Ok(OutcomeReceipt {
            turn_id: receipt.id.clone(),
            reinforced,
            not_reinforced,
        })
    }
}

impl Brain {
    /// Aggregated delivery-and-use evidence per memory, most-delivered first.
    ///
    /// Answers the question the flat retrieval-event log structurally cannot:
    /// *which memories are delivered repeatedly and never used?* Exposure,
    /// use, rejection, and never-adjudicated are all distinguishable here.
    ///
    /// This is deliberately **evidence, not policy**. A memory can go unused
    /// because it ranked 40th, because the same fact appeared elsewhere in the
    /// context, or because the actor failed — so repeated non-use is partly a
    /// property of the retriever. Wiring this to automatic decay or forgetting
    /// would let the write path erase evidence of a read-path defect, and must
    /// not be done without separate validation.
    pub fn memory_outcome_evidence(
        &self,
        limit: usize,
    ) -> Result<Vec<spectral_ingest::MemoryOutcomeEvidence>, Error> {
        self.inner.memory_outcome_evidence(limit)
    }
}
