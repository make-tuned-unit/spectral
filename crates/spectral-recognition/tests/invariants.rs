//! Tier-A invariant tests for the pre-registered public-benchmark claims
//! (docs/internal/recognition-public-benchmark-prereg-2026-07-28.md).
//!
//! - **C1 — Determinism.** Same content → byte-identical verdict and scalar,
//!   regardless of run count or store insertion order; golden fixture pins
//!   exact scalar values against silent drift.
//! - **C2 — Auditability.** Every non-Novel verdict carries machine-checkable
//!   evidence: cited landmarks literally occur (post-normalization) in BOTH
//!   the probe and the matched enrolled content; no evidence → Novel.
//! - **C3 — Zero inference.** The default feature surface carries no network
//!   stack and no ML runtime (the real gate is Cargo.toml; the test here
//!   documents it on the CI default-features build).

use spectral_recognition::{
    minhash, normalized_tokens, InMemoryRecognitionStore, MinHashConfig, RecognitionConfig,
    RecognitionEngine, RecognitionResult, Verdict,
};

/// A small but structurally diverse corpus: ops incidents (anchors), plain
/// prose, near-neighbours of each other, numbers/identifiers.
const CORPUS: &[(&str, &str)] = &[
    (
        "m-deploy",
        "The staging deploy failed with exit code 137 because the pod was OOMKilled during the migration step",
    ),
    (
        "m-auth",
        "Decided to use Clerk for authentication instead of rolling our own session management",
    ),
    (
        "m-grocery",
        "Planned the weekly grocery run: Costco for bulk items, saved about forty dollars splitting with neighbors",
    ),
    (
        "m-report",
        "Started the weekly status report for the Wealthie project covering bond structure progress",
    ),
    (
        "m-gpu",
        "Provisioned a new GPU node group g5-xlarge in us-east-1 for the training cluster",
    ),
    (
        "m-invoice",
        "Invoice INV-2214 from Fastly for $312.40 was approved and scheduled for payment on the 15th",
    ),
];

/// Probe panel spanning the verdict space: exact re-encounters, degraded
/// fragments, paraphrases, topical near-misses, and true novelty.
const PROBES: &[&str] = &[
    "The staging deploy failed with exit code 137 because the pod was OOMKilled during the migration step",
    "deploy failed exit code 137 pod OOMKilled",
    "our pods got OOMKilled again — exit 137 on the deploy",
    "Decided to use Clerk for authentication instead of rolling our own session management",
    "went with Clerk for auth rather than building session management ourselves",
    "weekly grocery run Costco bulk items forty dollars",
    "Invoice INV-2214 Fastly $312.40 approved",
    "Kubernetes upgrade to 1.31 finished without downtime on the api cluster",
    "Booked a dentist appointment for Tuesday morning near the office",
    "the",
    "",
];

fn engine_with(order: &[usize]) -> RecognitionEngine<InMemoryRecognitionStore> {
    let mut e = RecognitionEngine::new(
        InMemoryRecognitionStore::default(),
        RecognitionConfig::default(),
    );
    for &i in order {
        let (id, content) = CORPUS[i];
        e.enroll(id, content).unwrap();
    }
    e
}

fn scalar_bits(r: &RecognitionResult) -> (u64, u64, u64) {
    (
        r.familiarity.to_bits(),
        r.novelty.to_bits(),
        r.odds_of_old.to_bits(),
    )
}

// ── C1(a): repeated calls are byte-identical ────────────────────────────

#[test]
fn c1_repeat_determinism_byte_identical() {
    let e = engine_with(&[0, 1, 2, 3, 4, 5]);
    for probe in PROBES {
        let first = e.recognize(probe).unwrap();
        for _ in 0..2 {
            let again = e.recognize(probe).unwrap();
            assert_eq!(again.verdict, first.verdict, "verdict drift on {probe:?}");
            // Bit-level equality, not epsilon: C1 claims byte-identical scalars.
            assert_eq!(
                scalar_bits(&again),
                scalar_bits(&first),
                "scalar bits drift on {probe:?}"
            );
        }
    }
}

// ── C1(b): insertion-order independence ─────────────────────────────────

#[test]
fn c1_insertion_order_independence() {
    let forward = engine_with(&[0, 1, 2, 3, 4, 5]);
    let shuffled = engine_with(&[3, 5, 1, 4, 0, 2]);
    for probe in PROBES {
        let a = forward.recognize(probe).unwrap();
        let b = shuffled.recognize(probe).unwrap();
        assert_eq!(
            a.verdict, b.verdict,
            "verdict depends on enrollment order for {probe:?}"
        );
        assert_eq!(
            scalar_bits(&a),
            scalar_bits(&b),
            "scalar bits depend on enrollment order for {probe:?}"
        );
        // The audit trail itself must be order-independent too (evidence is
        // sorted by weight then feature; traces by score then id).
        let feats = |r: &RecognitionResult| {
            r.evidence
                .iter()
                .map(|e| (e.feature.clone(), e.memory_id.clone(), e.weight.to_bits()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            feats(&a),
            feats(&b),
            "evidence order/content differs for {probe:?}"
        );
    }
}

// ── C1(c): golden fixture — exact scalar values pinned ──────────────────

/// Pinned once from a verified run (2026-07-28, this commit) and asserted
/// with EXACT bit equality thereafter. Why: an epsilon test would let the
/// scalar drift silently under refactors (stemmer tweaks, weight changes,
/// float re-association); C1 promises byte-identical output for identical
/// input across runs AND platforms, so the fixture is the cross-platform
/// tripwire. If this fails, either the change intentionally altered scoring
/// (re-pin WITH a changelog note) or determinism broke (a bug).
#[test]
// The literals carry every digit of the pinned f64 on purpose — the fixture
// asserts bit equality, so truncating "excessive" precision would change it.
#[allow(clippy::excessive_precision)]
fn c1_golden_fixture_scalars() {
    let e = engine_with(&[0, 1, 2, 3, 4, 5]);
    let golden: &[(&str, &str, f64)] = &[
        (
            "deploy failed exit code 137 pod OOMKilled",
            "Recognized(m-deploy)",
            0.978_260_869_565_217_29, // shingle containment 45/46
        ),
        (
            "our pods got OOMKilled again — exit 137 on the deploy",
            "Familiar",
            0.588_235_294_117_647_08, // shingle containment 10/17
        ),
        (
            "Kubernetes upgrade to 1.31 finished without downtime on the api cluster",
            "Novel",
            0.0,
        ),
    ];
    for (probe, want_verdict, want_familiarity) in golden {
        let r = e.recognize(probe).unwrap();
        let got_verdict = match &r.verdict {
            Verdict::Recognized { memory_id } => format!("Recognized({memory_id})"),
            Verdict::Familiar => "Familiar".to_string(),
            Verdict::Novel => "Novel".to_string(),
        };
        assert_eq!(&got_verdict, want_verdict, "verdict for {probe:?}");
        assert_eq!(
            r.familiarity.to_bits(),
            want_familiarity.to_bits(),
            "familiarity for {probe:?}: got {:.17}, pinned {:.17}",
            r.familiarity,
            want_familiarity
        );
    }
}

// ── C2: every non-Novel verdict is machine-auditable ────────────────────

/// True iff `needle_run` (space-joined normalized keys) occurs as a
/// CONTIGUOUS subsequence of `tokens`.
fn contains_run(tokens: &[String], needle_run: &str) -> bool {
    let needle: Vec<&str> = needle_run.split(' ').collect();
    if needle.is_empty() || tokens.len() < needle.len() {
        return false;
    }
    tokens
        .windows(needle.len())
        .any(|w| w.iter().map(String::as_str).eq(needle.iter().copied()))
}

/// Machine-verify one evidence row against the probe and the enrolled text
/// it cites. Panics with a precise message on any audit failure.
fn audit_evidence_row(feature: &str, probe: &str, enrolled: &str, shingle: usize) {
    let probe_toks = normalized_tokens(probe);
    let enrolled_toks = normalized_tokens(enrolled);
    if let Some(pair) = feature.strip_prefix("pair: ") {
        let (lo, hi) = pair
            .split_once('~')
            .unwrap_or_else(|| panic!("unparseable pair evidence {feature:?}"));
        for key in [lo, hi] {
            assert!(
                probe_toks.iter().any(|t| t == key),
                "cited landmark {key:?} absent from normalized probe {probe:?}"
            );
            assert!(
                enrolled_toks.iter().any(|t| t == key),
                "cited landmark {key:?} absent from normalized enrolled text {enrolled:?}"
            );
        }
    } else if let Some(run) = feature
        .strip_prefix("run: '")
        .and_then(|r| r.strip_suffix('\''))
    {
        assert!(
            contains_run(&probe_toks, run),
            "cited run {run:?} not a contiguous token run of the probe {probe:?}"
        );
        assert!(
            contains_run(&enrolled_toks, run),
            "cited run {run:?} not a contiguous token run of the enrolled text {enrolled:?}"
        );
    } else if let Some(sim) = feature.strip_prefix("minhash: containment ") {
        // The containment citation is auditable by recomputation: the stated
        // similarity must equal the containment of the probe's shingle set in
        // the enrolled text's, and the overlap must be real (non-empty).
        let probe_set = minhash::shingle_set(probe, shingle);
        let doc_set = minhash::shingle_set(enrolled, shingle);
        let recomputed = minhash::containment(&probe_set, &doc_set);
        assert!(
            recomputed > 0.0,
            "minhash evidence cites {enrolled:?} but probe {probe:?} shares no shingle with it"
        );
        assert_eq!(
            sim,
            format!("{recomputed:.2}"),
            "stated containment does not recompute for probe {probe:?}"
        );
    } else {
        panic!("unknown evidence kind {feature:?} — audit rules must cover every kind");
    }
}

#[test]
fn c2_non_novel_verdicts_carry_verifiable_evidence() {
    let cfg = RecognitionConfig::default();
    let shingle = MinHashConfig::default().shingle;
    let e = RecognitionEngine::new(InMemoryRecognitionStore::default(), cfg);
    let mut e = e;
    for (id, content) in CORPUS {
        e.enroll(id, content).unwrap();
    }
    let enrolled_by_id: std::collections::HashMap<&str, &str> = CORPUS.iter().copied().collect();

    let mut non_novel = 0usize;
    for probe in PROBES {
        let r = e.recognize(probe).unwrap();
        if r.verdict == Verdict::Novel {
            continue;
        }
        non_novel += 1;
        assert!(
            !r.evidence.is_empty(),
            "non-Novel verdict {:?} for {probe:?} carries no evidence",
            r.verdict
        );
        for ev in &r.evidence {
            assert!(ev.weight > 0.0, "evidence weight must be positive");
            let enrolled = enrolled_by_id
                .get(ev.memory_id.as_str())
                .unwrap_or_else(|| panic!("evidence cites unknown memory {:?}", ev.memory_id));
            audit_evidence_row(&ev.feature, probe, enrolled, shingle);
        }
        // A Recognized verdict must be backed by evidence citing that memory.
        if let Verdict::Recognized { memory_id } = &r.verdict {
            assert!(
                r.evidence.iter().any(|ev| &ev.memory_id == memory_id),
                "Recognized({memory_id}) has no evidence row citing it"
            );
        }
    }
    assert!(
        non_novel >= 5,
        "panel must exercise the audit path; only {non_novel} non-Novel verdicts"
    );
}

#[test]
fn c2_no_evidence_implies_novel() {
    let e = engine_with(&[0, 1, 2, 3, 4, 5]);
    for probe in PROBES {
        let r = e.recognize(probe).unwrap();
        if r.evidence.is_empty() {
            assert_eq!(
                r.verdict,
                Verdict::Novel,
                "verdict without evidence must be Novel; {probe:?} got {:?}",
                r.verdict
            );
        }
    }
    // And on an empty store, everything is evidence-free and Novel.
    let empty = RecognitionEngine::new(
        InMemoryRecognitionStore::default(),
        RecognitionConfig::default(),
    );
    for probe in PROBES {
        let r = empty.recognize(probe).unwrap();
        assert!(r.evidence.is_empty());
        assert_eq!(r.verdict, Verdict::Novel);
    }
}

// ── C3: default feature surface is inference-free ───────────────────────

/// The real gate is Cargo.toml: `reqwest` is optional behind `paraphrase-gen`
/// and `fastembed` behind `neural-baseline`, both off by default — verified
/// externally by `cargo tree -p spectral-recognition` showing neither. This
/// test documents the surface on the build CI actually runs (default
/// features): if someone re-promotes either to a default dependency-feature,
/// the cfg flips on here and the test names the violated claim.
#[test]
// Constant by design: `cfg!` resolves at compile time, which is the point —
// the assertion documents the compiled feature surface of the CI build.
#[allow(clippy::assertions_on_constants)]
fn c3_default_features_are_inference_free() {
    assert!(
        !cfg!(feature = "paraphrase-gen"),
        "C3 violated: network stack (reqwest) present in the default build"
    );
    assert!(
        !cfg!(feature = "neural-baseline"),
        "C3 violated: ML runtime (fastembed/ort) present in the default build"
    );
}
