//! Staleness detection for undeclared multi-valued facts.
//!
//! A predicate the ontology marks `single_valued` supersedes deterministically
//! at write time — no judgement needed. This module covers the rest: a
//! `(subject, predicate)` holding several live objects where the store cannot
//! know whether they accumulate (`attended` three cities) or the newest
//! replaced the others (`lives_in` moved twice while nobody declared the
//! predicate functional).
//!
//! Deciding that needs judgement, so the split mirrors
//! [`spectral_archivist`]'s consolidation pass: **detection is deterministic
//! and lives here; adjudication is a pluggable trait with a no-op default.**
//! A consumer wires its own model — the reference implementation in the
//! literature runs a 7B locally.
//!
//! Two properties keep an automated adjudicator safe to run:
//!
//! * **It never touches the read path.** Adjudication happens during
//!   maintenance; recall stays deterministic and LLM-free.
//! * **Every retirement is reversible.** Supersession retires rather than
//!   deletes, records which assertion caused it and which agent decided, and
//!   [`GraphStore::undo_supersession`](crate::graph_store::GraphStore::undo_supersession)
//!   reverses one event.
//!
//! Adjudicators are asked a *closed* question about an already-detected
//! conflict, never to extract facts from prose. That distinction matters: in
//! the published evaluation, open-ended extraction is the accuracy bottleneck
//! (~44% on messy multi-value sentences) while the supersession rule itself is
//! exact.

use chrono::{DateTime, Utc};
use spectral_core::entity_id::EntityId;

use crate::brain::Brain;
use crate::Error;

/// One live object competing for a `(subject, predicate)` slot.
#[derive(Debug, Clone)]
pub struct CandidateObject {
    /// Row identity of the assertion, for undo and provenance.
    pub rowid: i64,
    pub object: EntityId,
    /// Canonical name, resolved for display. Empty if the entity is missing.
    pub object_canonical: String,
    pub asserted_at: DateTime<Utc>,
}

/// A `(subject, predicate)` with more than one live object.
#[derive(Debug, Clone)]
pub struct SupersessionCandidate {
    pub subject: EntityId,
    pub subject_canonical: String,
    /// Subject's entity type. Cardinality is domain-scoped, so an adjudicator
    /// needs this to reason about the slot (`person.location` is a different
    /// question from `org.location`).
    pub subject_type: String,
    pub predicate: String,
    /// Competing objects, oldest assertion first.
    pub objects: Vec<CandidateObject>,
}

/// What an adjudicator concluded about one candidate.
#[derive(Debug, Clone, PartialEq)]
pub enum Adjudication {
    /// The values genuinely accumulate. Nothing is retired.
    AllHold,
    /// The predicate is functional here and this object is the current value;
    /// every other live object for the slot is retired.
    Supersedes {
        /// Must be one of the candidate's objects.
        keep: EntityId,
        /// 0.0–1.0. Compared against the caller's threshold.
        confidence: f64,
    },
    /// Not enough information. Nothing is retired.
    Unknown,
}

/// The closed question an [`Adjudicator`] should put to its model, and the
/// only shape [`Adjudication`] can express.
///
/// Shipped as a constant so both sides bind to one contract instead of the
/// prompt drifting on the consumer side. `{subject}`, `{subject_type}`,
/// `{predicate}` and `{objects}` are substituted from the candidate; render
/// `{objects}` one per line as `- <canonical> (asserted <rfc3339>)`.
///
/// Three properties are deliberate. It is closed — the model chooses among
/// listed values and cannot introduce one, matching the `invalid_verdicts`
/// rejection in [`apply_adjudications`]. It never shows prose, so the model is
/// never asked to extract facts, which is the published accuracy bottleneck.
/// And abstention is a first-class answer, because `UNKNOWN` costs nothing
/// while a wrong `REPLACED` retires a true fact.
pub const ADJUDICATION_PROMPT: &str = "\
A memory system recorded several values for the same slot and cannot tell \
whether they accumulate or whether newer ones replaced older ones.

Subject: {subject} (type: {subject_type})
Relation: {predicate}
Values, oldest first:
{objects}

Answer with exactly one line:
  ALL_HOLD                      - all values are simultaneously true
  REPLACED <value> <0.0-1.0>    - only <value> is true now; the rest are stale
  UNKNOWN                       - cannot tell from the values alone

<value> must be copied exactly from the list. Judge only whether one value \
supersedes the others for this subject; do not infer new facts. Prefer UNKNOWN \
over a guess.";

/// Pluggable staleness judgement. Default is a no-op, matching
/// [`spectral_archivist::traits::Consolidator`]'s shape.
///
/// Implementations must be side-effect free — they are asked to *judge*, and
/// the caller decides whether the verdict clears the confidence gate.
pub trait Adjudicator: Send + Sync {
    fn adjudicate(&self, candidate: &SupersessionCandidate) -> anyhow::Result<Adjudication>;
}

/// Always returns [`Adjudication::Unknown`]. The default, so the library ships
/// with no model dependency and no automated retirement.
pub struct NoOpAdjudicator;

impl Adjudicator for NoOpAdjudicator {
    fn adjudicate(&self, _candidate: &SupersessionCandidate) -> anyhow::Result<Adjudication> {
        Ok(Adjudication::Unknown)
    }
}

/// Outcome of an [`apply_adjudications`] pass.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SupersessionReport {
    /// Candidates examined.
    pub considered: usize,
    /// Verdicts that cleared the confidence gate and were applied.
    pub applied: usize,
    /// Assertions retired across all applied verdicts.
    pub retired: usize,
    /// Verdicts of `Supersedes` rejected for falling below the threshold.
    pub below_threshold: usize,
    /// Candidates judged `AllHold` or `Unknown`.
    pub left_alone: usize,
    /// Verdicts naming an object that was not in the candidate — a
    /// hallucinating adjudicator. Counted and skipped, never applied.
    pub invalid_verdicts: usize,
    /// Adjudicator errors. The pass continues; the candidate is untouched.
    pub errors: Vec<String>,
}

/// Deterministically find `(subject, predicate)` slots holding several live
/// objects. No model involved; this only narrows the field.
pub fn detect_candidates(brain: &Brain, limit: usize) -> Result<Vec<SupersessionCandidate>, Error> {
    let groups = brain.store().multi_valued_live_groups(limit)?;
    let mut out = Vec::with_capacity(groups.len());
    for (subject, predicate, objects) in groups {
        // A declared-functional predicate cannot reach here: its writes
        // supersede at assert time. Skip defensively so a mid-flight ontology
        // change cannot hand an adjudicator a slot it does not own.
        let subject_entity = brain.store().get_entity(&subject)?;
        let subject_type = subject_entity
            .as_ref()
            .map(|e| e.entity_type.clone())
            .unwrap_or_default();
        if brain.predicate_is_single_valued_pub(&predicate, &subject_type) {
            continue;
        }
        let subject_canonical = subject_entity.map(|e| e.canonical).unwrap_or_default();
        let mut resolved = Vec::with_capacity(objects.len());
        for (rowid, object, asserted_at) in objects {
            let object_canonical = brain
                .store()
                .get_entity(&object)?
                .map(|e| e.canonical)
                .unwrap_or_default();
            resolved.push(CandidateObject {
                rowid,
                object,
                object_canonical,
                asserted_at,
            });
        }
        out.push(SupersessionCandidate {
            subject,
            subject_canonical,
            subject_type,
            predicate,
            objects: resolved,
        });
    }
    Ok(out)
}

/// Run `adjudicator` over detected candidates and apply the verdicts that
/// clear `min_confidence`.
///
/// `agent` is recorded on every retirement, so an automated pass can be
/// audited and selectively undone later. Pass something identifying, e.g.
/// `"librarian-7b"`.
///
/// A verdict naming an object outside its candidate is counted as invalid and
/// skipped — an adjudicator cannot introduce a fact, only choose among ones
/// already asserted.
pub fn apply_adjudications(
    brain: &Brain,
    adjudicator: &dyn Adjudicator,
    limit: usize,
    min_confidence: f64,
    agent: &str,
) -> Result<SupersessionReport, Error> {
    let candidates = detect_candidates(brain, limit)?;
    let mut report = SupersessionReport {
        considered: candidates.len(),
        ..Default::default()
    };

    for candidate in &candidates {
        let verdict = match adjudicator.adjudicate(candidate) {
            Ok(v) => v,
            Err(e) => {
                report.errors.push(format!(
                    "{}/{}: {e}",
                    candidate.subject_canonical, candidate.predicate
                ));
                continue;
            }
        };

        match verdict {
            Adjudication::AllHold | Adjudication::Unknown => report.left_alone += 1,
            Adjudication::Supersedes { keep, confidence } => {
                if !candidate.objects.iter().any(|o| o.object == keep) {
                    report.invalid_verdicts += 1;
                    continue;
                }
                if confidence < min_confidence {
                    report.below_threshold += 1;
                    continue;
                }
                let retired = brain.retire_conflicting_objects(
                    &candidate.subject,
                    &candidate.predicate,
                    &keep,
                    agent,
                )?;
                report.applied += 1;
                report.retired += retired;
            }
        }
    }
    Ok(report)
}
