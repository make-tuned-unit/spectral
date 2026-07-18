//! Brain-level federation-sync surface: share/export/import/have-want over shared
//! wings, plus **scope-spanning recall** — recall that spans the private store and
//! merged shared wings, tags each result with provenance, and enforces the
//! view-scoping filter.
//!
//! Two layers of the sovereignty story (see `docs/internal/federation-sync-design.md`):
//! - **Structural export gate** (hard): a `Local` memory can never appear in any
//!   pack — enforced in `spectral_ingest::federation_sync`.
//! - **View-scoping recall filter** (honest-participant): a shared-scope recall
//!   never *surfaces* a private memory — enforced here, as a single chokepoint
//!   applied to the final hit list of whatever recall path produced it (FTS,
//!   associative spreading, future paths), so no path can silently bypass it.
//!
//! Caveat, banked: the recall filter is not a defense against a malicious local
//! process reading its own SQLite — that is disk/OS encryption, a separate layer.
//! Structural sovereignty is the export gate.

use crate::brain::Brain;
use crate::cascade_layers::CascadePipelineConfig;
use crate::error::Error;
use crate::spreading::AssocSpreadConfig;
use spectral_cascade::RecognitionContext;
use spectral_ingest::federation_sync::{self, Origin, Pack};
use spectral_ingest::MemoryHit;

/// Which realms a scoped recall spans.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RealmScope {
    /// The user's full view: private memories + every merged shared wing.
    All,
    /// Only the named shared wings — **private memories are excluded** (a
    /// "team view"). This is the view-scoping filter's sovereign case.
    Shared(Vec<String>),
    /// Only the user's own private memories.
    Private,
}

impl RealmScope {
    /// Whether a result of the given provenance is admitted by this scope.
    fn admits(&self, origin: &Origin) -> bool {
        match (self, origin) {
            (RealmScope::All, _) => true,
            (RealmScope::Private, Origin::Private) => true,
            (RealmScope::Private, Origin::Shared { .. }) => false,
            (RealmScope::Shared(wings), Origin::Shared { wing_id, .. }) => {
                wings.iter().any(|w| w == wing_id)
            }
            (RealmScope::Shared(_), Origin::Private) => false,
        }
    }
}

impl Brain {
    /// Share a local memory (by key) into a shared wing. Returns its object hash.
    pub fn share_memory(&self, key: &str, wing_id: &str) -> Result<String, Error> {
        federation_sync::share(self.sqlite_store(), key, wing_id)
            .map_err(|e| Error::Schema(e.to_string()))
    }

    /// Export a shared wing as a pack (source objects + tombstones) for the caller
    /// to encrypt and ship. A never-shared `Local` memory can never appear here.
    pub fn export_shared_wing(&self, wing_id: &str) -> Result<Pack, Error> {
        federation_sync::export_pack(self.sqlite_store(), wing_id)
            .map_err(|e| Error::Schema(e.to_string()))
    }

    /// Merge a received pack into the local store (OR-Set union) and re-index for
    /// recall. Returns the number of new objects merged.
    pub fn import_shared_wing(&self, pack: &Pack) -> Result<usize, Error> {
        federation_sync::import_pack(self.sqlite_store(), pack)
            .map_err(|e| Error::Schema(e.to_string()))
    }

    /// The object hashes currently live in a shared wing (for advertising in a
    /// have/want exchange).
    pub fn shared_wing_hashes(&self, wing_id: &str) -> Result<Vec<String>, Error> {
        federation_sync::enumerate(self.sqlite_store(), wing_id)
            .map_err(|e| Error::Schema(e.to_string()))
    }

    /// Have/want: given a remote's advertised hashes for a wing, the subset we
    /// lack and should request.
    pub fn shared_wing_want(
        &self,
        wing_id: &str,
        remote_hashes: &[String],
    ) -> Result<Vec<String>, Error> {
        let local = self.shared_wing_hashes(wing_id)?;
        Ok(federation_sync::missing_locally(&local, remote_hashes))
    }

    /// Retract an object from a shared wing (tombstone; propagates via the next
    /// export, cannot be resurrected by re-import).
    pub fn tombstone_shared(&self, wing_id: &str, object_hash: &str) -> Result<(), Error> {
        federation_sync::tombstone(self.sqlite_store(), wing_id, object_hash)
            .map_err(|e| Error::Schema(e.to_string()))
    }

    /// Scope-spanning recall: recall across the private store + merged shared
    /// wings (with associative spreading), tag each result with its provenance,
    /// and apply the view-scoping filter for `scope`.
    ///
    /// The filter is applied to the **final** hit list — after FTS, reranking, and
    /// spreading — so a shared-scope recall never surfaces a private memory,
    /// including a private spread-mate of a shared seed. That is the chokepoint:
    /// enforcement is a property of the output, independent of the path.
    pub fn recall_scoped(
        &self,
        query: &str,
        scope: RealmScope,
    ) -> Result<Vec<(MemoryHit, Origin)>, Error> {
        let cfg = CascadePipelineConfig {
            spread: AssocSpreadConfig::completeness(),
            ..CascadePipelineConfig::default()
        };
        let ctx = RecognitionContext::empty();
        let result = self.recall_cascade_with_pipeline(query, &ctx, &cfg)?;

        let mut out = Vec::with_capacity(result.merged_hits.len());
        for hit in result.merged_hits {
            let origin = federation_sync::provenance(self.sqlite_store(), &hit.key)
                .map_err(|e| Error::Schema(e.to_string()))?;
            if scope.admits(&origin) {
                out.push((hit, origin));
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::{Brain, BrainConfig, EntityPolicy, RememberOpts};
    use spectral_core::visibility::Visibility;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn brain(tmp: &TempDir) -> Brain {
        Brain::open(BrainConfig {
            data_dir: tmp.path().to_path_buf(),
            ontology_path: PathBuf::from("tests/fixtures/brain_ontology.toml"),
            memory_db_path: None,
            llm_client: None,
            wing_rules: None,
            hall_rules: None,
            device_id: None,
            enable_spectrogram: false,
            entity_policy: EntityPolicy::Strict,
            sqlite_mmap_size: None,
            fts_tokenizer: None,
            read_only: false,
            activity_wing: "activity".into(),
            redaction_policy: None,
            tact_config: None,
        })
        .unwrap()
    }

    /// End-to-end: brain A shares a memory, brain B imports the pack, and B's
    /// scope-spanning recall surfaces it — tagged with the shared wing + A's
    /// author — while B's own private memory is tagged Private.
    #[test]
    fn sync_then_scoped_recall_carries_provenance() {
        let ta = TempDir::new().unwrap();
        let a = brain(&ta);
        let a_id = *a.brain_id();
        a.remember(
            "runbook",
            "the deploy runbook lives in the ops notion",
            Visibility::Team,
        )
        .unwrap();
        a.share_memory("runbook", "team").unwrap();
        let pack = a.export_shared_wing("team").unwrap();

        let tb = TempDir::new().unwrap();
        let b = brain(&tb);
        b.remember(
            "mine",
            "my private grocery list for the weekend",
            Visibility::Private,
        )
        .unwrap();
        b.import_shared_wing(&pack).unwrap();

        // Full view: sees both, correctly tagged.
        let all = b
            .recall_scoped("deploy runbook ops", RealmScope::All)
            .unwrap();
        let shared = all
            .iter()
            .find(|(h, _)| h.content.contains("deploy runbook"))
            .expect("imported shared memory should be recalled");
        match &shared.1 {
            Origin::Shared { wing_id, author_id } => {
                assert_eq!(wing_id, "team");
                assert_eq!(*author_id, Some(*a_id.as_bytes()));
            }
            Origin::Private => panic!("imported shared memory tagged Private"),
        }

        // Team view: the shared memory is present; nothing private leaks in.
        let team = b
            .recall_scoped("deploy runbook", RealmScope::Shared(vec!["team".into()]))
            .unwrap();
        assert!(team.iter().all(|(_, o)| matches!(o, Origin::Shared { .. })));
        assert!(team
            .iter()
            .any(|(h, _)| h.content.contains("deploy runbook")));
    }

    /// The view-scoping property, adversarial shape: with associative spreading
    /// ON, a Private episode-mate of a shared seed must NOT surface in a
    /// shared-scope recall — the filter catches spread-pulled private memories.
    #[test]
    fn shared_scope_recall_never_surfaces_a_private_spread_mate() {
        let tmp = TempDir::new().unwrap();
        let br = brain(&tmp);
        // A query-matching shared seed and a lexically-disjoint PRIVATE mate in the
        // same episode — spreading will try to pull the mate in.
        br.remember_with(
            "seed",
            "planning the quarterly offsite logistics and the venue booking",
            RememberOpts {
                visibility: Visibility::Team,
                episode_id: Some("ep-off".into()),
                ..Default::default()
            },
        )
        .unwrap();
        br.remember_with(
            "priv-mate",
            "personal reminder refill the beta blocker prescription friday",
            RememberOpts {
                visibility: Visibility::Private,
                episode_id: Some("ep-off".into()),
                ..Default::default()
            },
        )
        .unwrap();
        br.share_memory("seed", "team").unwrap();

        let team = br
            .recall_scoped(
                "quarterly offsite venue",
                RealmScope::Shared(vec!["team".into()]),
            )
            .unwrap();
        assert!(
            !team.iter().any(|(h, _)| h.content.contains("beta blocker")),
            "a Private episode-mate leaked into a shared-scope recall via spreading: {:?}",
            team.iter().map(|(h, _)| h.key.clone()).collect::<Vec<_>>()
        );
        // And the shared seed itself is recalled and tagged shared.
        assert!(
            team.iter()
                .any(|(h, o)| h.content.contains("offsite") && matches!(o, Origin::Shared { .. })),
            "the shared seed should be recalled under the team scope"
        );
    }
}
