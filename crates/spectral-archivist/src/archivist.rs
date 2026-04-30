//! Main Archivist struct and orchestration.

use crate::candidates::{self, ConsolidationCandidate};
use crate::decay::{self, DecayStats};
use crate::duplicates::{self, DuplicatePair};
use crate::gaps::{self, GapReport};
use crate::reclassify::{self, ReclassificationSuggestion};
use crate::traits::{Consolidator, Indexer, NoOpConsolidator, NoOpIndexer};
use chrono::{DateTime, Utc};
use rusqlite::Connection;
use std::path::Path;

/// Configuration for the Archivist.
#[derive(Debug, Clone)]
pub struct ArchivistConfig {
    /// Jaccard threshold for duplicate detection. Default 0.6.
    pub duplicate_threshold: f64,
    /// Minimum Jaccard overlap for consolidation candidates. Default 0.45.
    pub consolidation_overlap_min: f64,
    /// Maximum Jaccard overlap for consolidation candidates. Default 0.58.
    pub consolidation_overlap_max: f64,
    /// Days without reinforcement before decay applies. Default 30.
    pub decay_threshold_days: i64,
    /// Amount to subtract per decay pass. Default 0.05.
    pub decay_amount: f64,
    /// Days since last reinforcement to qualify for boost. Default 7.
    pub boost_threshold_days: i64,
    /// Amount to add per boost pass. Default 0.02.
    pub boost_amount: f64,
    /// Floor for signal score after decay. Default 0.1.
    pub min_signal_score: f64,
    /// Ceiling for signal score after boost. Default 1.0.
    pub max_signal_score: f64,
    /// Minimum memories in a wing before gap detection applies. Default 3.
    pub gap_min_memories: usize,
    /// Key prefixes to skip in consolidation candidate search.
    pub consolidation_skip_prefixes: Vec<String>,
    /// Key substrings to skip in consolidation candidate search.
    pub consolidation_skip_contains: Vec<String>,
    /// Wing names that are too generic for reclassification matching.
    pub weak_wings: Vec<String>,
    /// Known project names for unmapped-project gap detection.
    pub known_projects: Option<Vec<String>>,
}

impl Default for ArchivistConfig {
    fn default() -> Self {
        Self {
            duplicate_threshold: 0.6,
            consolidation_overlap_min: 0.45,
            consolidation_overlap_max: 0.58,
            decay_threshold_days: 30,
            decay_amount: 0.05,
            boost_threshold_days: 7,
            boost_amount: 0.02,
            min_signal_score: 0.1,
            max_signal_score: 1.0,
            gap_min_memories: 3,
            consolidation_skip_prefixes: vec![
                "slack_".into(),
                "openbird:".into(),
                "user_msg_".into(),
                "task_".into(),
            ],
            consolidation_skip_contains: vec![
                "status_".into(),
                "_status_".into(),
                "cron_".into(),
                "_cron".into(),
                "activity_".into(),
                "_activity".into(),
                "sync".into(),
                "db_".into(),
                "_db".into(),
                "location".into(),
            ],
            weak_wings: vec!["general".into()],
            known_projects: None,
        }
    }
}

/// Combined report from all dry-run passes.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ArchivistReport {
    pub duplicates: Vec<DuplicatePair>,
    pub gaps: GapReport,
    pub reclassifications: Vec<ReclassificationSuggestion>,
    pub consolidation_candidates: Vec<ConsolidationCandidate>,
    pub memory_count: usize,
    pub timestamp: DateTime<Utc>,
}

/// Report from a full run (includes mutation stats).
#[derive(Debug, Clone, serde::Serialize)]
pub struct ArchivistRunReport {
    pub report: ArchivistReport,
    pub decay_stats: DecayStats,
}

/// Memory-quality maintenance engine for Spectral brains.
///
/// Runs five algorithmic passes over the memory database:
/// 1. **Duplicates** — Jaccard similarity on word sets
/// 2. **Gaps** — wings missing summaries, facts, or people
/// 3. **Reclassification** — general-wing memories that belong elsewhere
/// 4. **Decay** — stale memories lose signal, recently-used gain it
/// 5. **Consolidation candidates** — pairs eligible for LLM-mediated merge
pub struct Archivist {
    conn: Connection,
    config: ArchivistConfig,
    consolidator: Box<dyn Consolidator>,
    indexer: Box<dyn Indexer>,
}

impl Archivist {
    /// Open an archivist on the given memory database with default config.
    pub fn open(memory_db_path: &Path) -> anyhow::Result<Self> {
        Self::open_with_config(memory_db_path, ArchivistConfig::default())
    }

    /// Open an archivist with custom configuration.
    pub fn open_with_config(
        memory_db_path: &Path,
        config: ArchivistConfig,
    ) -> anyhow::Result<Self> {
        let conn = Connection::open(memory_db_path)?;
        Ok(Self {
            conn,
            config,
            consolidator: Box::new(NoOpConsolidator),
            indexer: Box::new(NoOpIndexer),
        })
    }

    /// Set a custom consolidator (for Phase 2 LLM-mediated consolidation).
    pub fn with_consolidator(mut self, consolidator: Box<dyn Consolidator>) -> Self {
        self.consolidator = consolidator;
        self
    }

    /// Set a custom indexer (for Phase 2 LLM-mediated index generation).
    pub fn with_indexer(mut self, indexer: Box<dyn Indexer>) -> Self {
        self.indexer = indexer;
        self
    }

    /// Run all passes in dry-run mode (no mutations).
    pub fn report(&self) -> anyhow::Result<ArchivistReport> {
        let memory_count: usize =
            self.conn
                .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?;

        Ok(ArchivistReport {
            duplicates: self.find_duplicates()?,
            gaps: self.find_gaps()?,
            reclassifications: self.suggest_reclassifications()?,
            consolidation_candidates: self.find_consolidation_candidates()?,
            memory_count,
            timestamp: Utc::now(),
        })
    }

    /// Apply signal score decay and boost (the only mutation pass).
    pub fn apply_decay(&self) -> anyhow::Result<DecayStats> {
        decay::apply_decay(
            &self.conn,
            self.config.decay_threshold_days,
            self.config.decay_amount,
            self.config.boost_threshold_days,
            self.config.boost_amount,
            self.config.min_signal_score,
            self.config.max_signal_score,
        )
    }

    /// Run all passes including mutations (decay).
    pub fn run(&self) -> anyhow::Result<ArchivistRunReport> {
        let report = self.report()?;
        let decay_stats = self.apply_decay()?;
        Ok(ArchivistRunReport {
            report,
            decay_stats,
        })
    }

    /// Find duplicate memory pairs by Jaccard similarity.
    pub fn find_duplicates(&self) -> anyhow::Result<Vec<DuplicatePair>> {
        duplicates::find_duplicates(&self.conn, self.config.duplicate_threshold)
    }

    /// Detect coverage gaps across wings.
    pub fn find_gaps(&self) -> anyhow::Result<GapReport> {
        gaps::find_gaps(
            &self.conn,
            self.config.gap_min_memories,
            self.config.known_projects.as_deref(),
        )
    }

    /// Suggest reclassifications for general-wing memories.
    pub fn suggest_reclassifications(&self) -> anyhow::Result<Vec<ReclassificationSuggestion>> {
        reclassify::suggest_reclassifications(&self.conn, &self.config.weak_wings)
    }

    /// Find pairs eligible for LLM-mediated consolidation.
    pub fn find_consolidation_candidates(&self) -> anyhow::Result<Vec<ConsolidationCandidate>> {
        candidates::find_consolidation_candidates(
            &self.conn,
            self.config.consolidation_overlap_min,
            self.config.consolidation_overlap_max,
            &self.config.consolidation_skip_prefixes,
            &self.config.consolidation_skip_contains,
        )
    }

    /// Access the underlying consolidator.
    pub fn consolidator(&self) -> &dyn Consolidator {
        self.consolidator.as_ref()
    }

    /// Access the underlying indexer.
    pub fn indexer(&self) -> &dyn Indexer {
        self.indexer.as_ref()
    }

    /// Access the underlying connection. Intended for testing.
    #[doc(hidden)]
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Create an archivist from an existing connection. Intended for testing.
    #[doc(hidden)]
    pub fn from_conn(conn: Connection, config: ArchivistConfig) -> Self {
        Self {
            conn,
            config,
            consolidator: Box::new(NoOpConsolidator),
            indexer: Box::new(NoOpIndexer),
        }
    }
}
