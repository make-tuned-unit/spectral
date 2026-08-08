//! ForgetEval's prediction about us, tested against us.
//!
//! *Control-Plane Placement Shapes Forgetting* (arXiv 2606.15903, MIT) is the
//! one public benchmark whose research question is our thesis: deterministic
//! primitives versus LLM control planes. It reports that deterministic stores
//! hold the lexical and temporal categories and then fail a specific one —
//! **canonicalization** — at **5% on identifier-obfuscation and 0% on
//! cross-lingual**, recoverable only by an inscribe-time LLM.
//!
//! That is a falsifiable prediction about a system built exactly like ours, so
//! we test it on ours rather than assuming it transfers. These tests
//! **document the gap; they do not assert it away.** Where we fail, the test
//! asserts the failure, so that any future canonicalization work turns them red
//! and forces the claim to be re-stated deliberately.
//!
//! Nothing here needs the ForgetEval harness or dataset — it needs its
//! hypothesis, which is free.

use spectral::{Brain, RecallTopKConfig, RememberOpts, Visibility};
use tempfile::TempDir;

fn brain_with(facts: &[(&str, &str)]) -> (TempDir, Brain) {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(tmp.path()).unwrap();
    for (k, v) in facts {
        brain
            .remember_with(
                k,
                v,
                RememberOpts {
                    visibility: Visibility::Private,
                    ..Default::default()
                },
            )
            .unwrap();
    }
    (tmp, brain)
}

fn retrieves(brain: &Brain, query: &str, expect_key: &str) -> bool {
    let cfg = RecallTopKConfig {
        k: 20,
        ..RecallTopKConfig::default()
    };
    brain
        .recall_topk_fts(query, &cfg, Visibility::Private)
        .unwrap()
        .iter()
        .any(|h| h.key == expect_key)
}

/// Baseline: the categories ForgetEval says deterministic stores hold.
/// If this fails, the tests below are measuring something other than
/// canonicalization.
#[test]
fn lexical_retrieval_works_which_is_the_control() {
    let (_t, brain) = brain_with(&[
        (
            "f:db",
            "the production database migration is scheduled for Tuesday",
        ),
        (
            "f:cat",
            "my cat is called Mackerel and sleeps on the router",
        ),
    ]);
    assert!(
        retrieves(&brain, "when is the database migration", "f:db"),
        "plain lexical overlap must retrieve — otherwise this file proves nothing"
    );
    assert!(retrieves(&brain, "what is my cat called", "f:cat"));
}

/// **Identifier obfuscation.** ForgetEval: ~5% for deterministic stores.
///
/// The same identifier written with different separators or casing is the same
/// identifier to a human and a different token to a tokenizer.
#[test]
fn identifier_obfuscation_is_a_measured_gap() {
    let (_t, brain) = brain_with(&[(
        "f:key",
        "my deploy token is sk-ABC123XYZ for the staging cluster",
    )]);

    // Exact form: retrievable.
    assert!(
        retrieves(&brain, "sk-ABC123XYZ", "f:key"),
        "the exact identifier must match"
    );

    // Obfuscated forms a person would consider identical. Which of these
    // survive is a property of the TOKENIZER, and the answer was not what the
    // paper's 5% led us to expect — measured, not assumed:
    //
    //   sk_ABC123XYZ  RETRIEVES  — unicode61 treats `_` as a separator too, so
    //   SK-ABC123XYZ  RETRIEVES  — it case-folds, and
    //   sk-abc123xyz  RETRIEVES  — both forms tokenize to ["sk","abc123xyz"].
    //   skABC123XYZ   MISSES     — concatenation removes the boundary entirely,
    //                              producing one token that shares no term.
    //
    // So Spectral is *better* than ForgetEval's 5% on the separator and casing
    // variants, and fails exactly where a term index must: when the token
    // boundary itself is gone.
    for v in ["sk_ABC123XYZ", "SK-ABC123XYZ", "sk-abc123xyz"] {
        assert!(
            retrieves(&brain, v, "f:key"),
            "{v} should retrieve: unicode61 folds case and splits on both \
             separators, so it tokenizes identically to the stored form"
        );
    }
    assert!(
        !retrieves(&brain, "skABC123XYZ", "f:key"),
        "concatenation is the real gap: with no separator there is no shared \
         token, and no ranking signal can recover a term that was never indexed"
    );
}

/// **Cross-lingual.** ForgetEval: 0% for deterministic stores.
///
/// This is the cleanest statement of the lexical ceiling. A term index cannot
/// match words it has never seen, and translation is not a ranking problem.
#[test]
fn cross_lingual_retrieval_is_zero_as_predicted() {
    let (_t, brain) = brain_with(&[
        ("f:allergy", "I am allergic to peanuts and shellfish"),
        ("f:job", "I work as a civil engineer in Porto"),
    ]);

    // English: fine.
    assert!(retrieves(&brain, "what am I allergic to", "f:allergy"));

    // Same questions, other languages. Every one of these should miss.
    for (q, key) in [
        ("à quoi suis-je allergique", "f:allergy"), // French
        ("¿a qué soy alérgico?", "f:allergy"),      // Spanish
        ("was ist meine Arbeit", "f:job"),          // German
        ("qual é o meu trabalho", "f:job"),         // Portuguese
    ] {
        assert!(
            !retrieves(&brain, q, key),
            "cross-lingual retrieval unexpectedly SUCCEEDED for {q:?}. That would \
             contradict ForgetEval's 0% and means something semantic entered the \
             read path — re-state the claim, do not delete this test."
        );
    }
}

/// The gap is **not** a ranking failure, which is what makes it interesting.
///
/// A ranking failure means the right memory was in the candidate set and lost.
/// Here it never enters: no query term matches, so no k, no rerank, and no
/// proximity signal (G4) can recover it. This is why the fix ForgetEval finds
/// effective is *inscribe-time*, not read-time.
#[test]
fn the_canonicalization_gap_is_admission_not_ranking() {
    let (_t, brain) = brain_with(&[("f:allergy", "I am allergic to peanuts and shellfish")]);

    let cfg = RecallTopKConfig {
        // Absurdly generous k: if this were a ranking problem, depth would fix it.
        k: 500,
        fetch_mult: 12,
        ..RecallTopKConfig::default()
    };
    let hits = brain
        .recall_topk_fts("à quoi suis-je allergique", &cfg, Visibility::Private)
        .unwrap();

    assert!(
        !hits.iter().any(|h| h.key == "f:allergy"),
        "k=500 with a 12x pool still cannot admit it — depth is not the fix"
    );
}
