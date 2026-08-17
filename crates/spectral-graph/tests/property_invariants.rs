//! Property tests over generated corpora.
//!
//! Everything else in this suite asserts behaviour on hand-built fixtures,
//! which only ever exercises the shapes someone thought to write down. These
//! generate corpora instead and assert invariants that must hold for *every*
//! corpus — the ones where a single counterexample is a defect rather than a
//! surprise.
//!
//! The generator is a hand-rolled xorshift seeded by a constant, not a
//! dependency. That is deliberate: this repo avoids `rand` (see the workspace
//! manifest), and a property failure has to be *reproducible* — a fixed seed
//! means the corpus that broke the invariant can be rebuilt exactly. Each
//! test prints its seed so a failure names the case.

use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig, EntityPolicy, RecallTopKConfig, RememberOpts};
use tempfile::TempDir;

// ── deterministic generator ────────────────────────────────────────

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        // Avoid the zero state, which xorshift cannot leave.
        Self(seed | 1)
    }
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len())]
    }
}

const WORDS: &[&str] = &[
    "deploy",
    "runbook",
    "notion",
    "rollback",
    "staging",
    "zephyr",
    "incident",
    "escalation",
    "budget",
    "review",
    "calendar",
    "migration",
    "cluster",
    "pipeline",
    "cutover",
    "postgres",
    "benchmark",
    "retry",
    "cache",
    "origin",
];

const VISIBILITIES: &[Visibility] = &[
    Visibility::Private,
    Visibility::Team,
    Visibility::Org,
    Visibility::Public,
];

/// Mirror of the crate-private `str_to_vis`: anything unrecognised is
/// Private, matching its `_ =>` arm. Duplicated here rather than widening the
/// source's visibility for a test.
fn parse_vis(s: &str) -> Visibility {
    match s {
        "team" => Visibility::Team,
        "org" => Visibility::Org,
        "public" => Visibility::Public,
        _ => Visibility::Private,
    }
}

/// Admissibility, decided **independently** of `Visibility::allows`.
///
/// The sovereignty property below previously asserted with `allows` — the same
/// predicate production filters with. Inverting `allows` therefore inverted
/// both sides and the property stayed green; confirmed by mutation, where 24
/// other tests caught it and this one did not. An oracle that shares its
/// implementation with the subject tests only that production is
/// self-consistent, never that the rule is right.
///
/// This rank is written out by hand for that reason. Do not refactor it to
/// call `allows`, `Ord`, or anything derived from them.
fn admissible_independently(content: &str, context: Visibility) -> bool {
    let rank = |v: &str| match v {
        "team" => 1,
        "org" => 2,
        "public" => 3,
        _ => 0, // private and anything unrecognised
    };
    let ctx = match context {
        Visibility::Private => 0,
        Visibility::Team => 1,
        Visibility::Org => 2,
        Visibility::Public => 3,
    };
    rank(content) >= ctx
}

/// One generated memory.
struct Gen {
    key: String,
    content: String,
    visibility: Visibility,
}

fn gen_corpus(rng: &mut Rng, n: usize) -> Vec<Gen> {
    (0..n)
        .map(|i| {
            let len = 4 + rng.below(8);
            let content = (0..len)
                .map(|_| *rng.pick(WORDS))
                .collect::<Vec<_>>()
                .join(" ");
            Gen {
                key: format!("k{i}"),
                content,
                visibility: *rng.pick(VISIBILITIES),
            }
        })
        .collect()
}

fn config(tmp: &TempDir) -> BrainConfig {
    let o = tmp.path().join("ontology.toml");
    std::fs::write(&o, "version = 1\n").unwrap();
    BrainConfig {
        data_dir: tmp.path().to_path_buf(),
        ontology_path: o,
        entity_policy: EntityPolicy::Strict,
        activity_wing: "activity".into(),
        ..Default::default()
    }
}

fn seeded_brain(tmp: &TempDir, corpus: &[Gen]) -> Brain {
    let b = Brain::open(config(tmp)).unwrap();
    for g in corpus {
        b.remember_with(
            &g.key,
            &g.content,
            RememberOpts {
                visibility: g.visibility,
                ..Default::default()
            },
        )
        .unwrap();
    }
    b
}

/// Every query word, so a property is checked against terms that actually hit.
fn queries() -> Vec<String> {
    WORDS.iter().take(6).map(|w| w.to_string()).collect()
}

// ── the sovereignty invariant ──────────────────────────────────────

/// **For every corpus and every scope, a scoped recall returns only hits whose
/// own label admits that scope.** This is the security invariant the whole
/// visibility system exists for, and one counterexample is a data leak.
///
/// Checked across generated corpora with mixed visibilities rather than a
/// hand-built pair, because the leak that matters is the one nobody thought
/// to write a fixture for.
///
/// **Defence in depth, verified by mutation:** visibility is enforced twice —
/// as a SQL predicate before `LIMIT` (`fts_search_scoped`) and again as a
/// Rust `retain` afterwards. Removing *either* layer alone leaves this
/// property holding; removing *both* makes it fail. So the invariant survives
/// a single-point regression, and this test does detect a total one.
#[test]
fn no_scoped_recall_ever_returns_an_inadmissible_hit() {
    // Counts hits actually inspected. Without it this test passes having
    // asserted nothing if recall returns empty for every query — the loop
    // body simply never runs. The floor keys on something the test controls
    // (its own corpus and queries), not on ranking or page composition.
    let mut examined = 0usize;
    for seed in 1..=12u64 {
        let mut rng = Rng::new(seed * 0x9E37_79B9);
        let corpus = gen_corpus(&mut rng, 12);
        let tmp = TempDir::new().unwrap();
        let b = seeded_brain(&tmp, &corpus);

        for scope in VISIBILITIES {
            for q in queries() {
                let hits = b
                    .recall_topk_fts(&q, &RecallTopKConfig::default(), *scope)
                    .unwrap();
                for h in &hits {
                    examined += 1;
                    let label = parse_vis(&h.visibility);
                    assert!(
                        admissible_independently(&h.visibility, *scope),
                        "seed {seed}: a {:?} memory ({}) was returned to a {scope:?} \
                         scoped recall for query {q:?}",
                        label,
                        h.key
                    );
                }
            }
        }
    }
    assert!(
        examined > 0,
        "the invariant held over ZERO hits — recall returned nothing across \
         every seed, scope and query, so this test proved nothing"
    );
}

/// The complement, so the scoping is not vacuously satisfied by returning
/// nothing: a Private scope admits every label, so for any corpus it must see
/// at least as many distinct keys as any stricter scope.
#[test]
fn a_private_scope_is_never_narrower_than_a_stricter_one() {
    let mut nonempty_private = 0usize;
    for seed in 1..=12u64 {
        let mut rng = Rng::new(seed * 0x85EB_CA6B);
        let corpus = gen_corpus(&mut rng, 12);
        let tmp = TempDir::new().unwrap();
        let b = seeded_brain(&tmp, &corpus);

        for q in queries() {
            let cfg = RecallTopKConfig::default();
            let private = b
                .recall_topk_fts(&q, &cfg, Visibility::Private)
                .unwrap()
                .len();
            if private > 0 {
                nonempty_private += 1;
            }
            for scope in [Visibility::Team, Visibility::Org, Visibility::Public] {
                let scoped = b.recall_topk_fts(&q, &cfg, scope).unwrap().len();
                assert!(
                    private >= scoped,
                    "seed {seed}, query {q:?}: a {scope:?} scope returned {scoped} \
                     hits while Private returned only {private}"
                );
            }
        }
    }
    assert!(
        nonempty_private > 0,
        "every Private recall was empty, so `private >= scoped` held only \
         because both sides were zero"
    );
}

// ── determinism ────────────────────────────────────────────────────

/// **For every corpus, repeating the same query against an unchanged brain
/// yields the identical ordered key sequence.** Determinism is the product's
/// headline claim; a corpus-shaped tie-break failure would show here and not
/// in a fixture with three memories.
///
/// Uses a read-only handle so the auto-reinforce write-back cannot perturb
/// ranking between the two calls — the same scoping the pitch guardrail uses.
///
/// **What this does NOT catch, verified by mutation:** removing the `m.id`
/// tiebreak from the FTS `ORDER BY` leaves this property passing, because
/// SQLite is incidentally stable for a fixed database and query plan. The
/// tiebreak exists for reproducibility across *plans and machines*, which an
/// in-process test cannot induce. That guarantee is covered instead by
/// `spectral-ingest`'s `tied_orderings_are_deterministic`, which builds two
/// independent stores and compares — and which does fail when the tiebreak is
/// removed. Recorded here so this test is not mistaken for the whole
/// determinism story.
#[test]
fn repeated_recall_is_byte_stable_on_an_unchanged_brain() {
    // Comparing two empty results is trivially equal, so stability would hold
    // over a brain that recalls nothing. Count the non-empty comparisons.
    let mut compared_nonempty = 0usize;
    for seed in 1..=10u64 {
        let mut rng = Rng::new(seed * 0xC2B2_AE35);
        let corpus = gen_corpus(&mut rng, 16);
        let tmp = TempDir::new().unwrap();
        drop(seeded_brain(&tmp, &corpus));

        let ro = Brain::open(BrainConfig {
            read_only: true,
            ..config(&tmp)
        })
        .unwrap();

        for q in queries() {
            let cfg = RecallTopKConfig::default();
            let first: Vec<String> = ro
                .recall_topk_fts(&q, &cfg, Visibility::Private)
                .unwrap()
                .into_iter()
                .map(|h| h.key)
                .collect();
            if !first.is_empty() {
                compared_nonempty += 1;
            }
            for attempt in 0..3 {
                let again: Vec<String> = ro
                    .recall_topk_fts(&q, &cfg, Visibility::Private)
                    .unwrap()
                    .into_iter()
                    .map(|h| h.key)
                    .collect();
                assert_eq!(
                    first, again,
                    "seed {seed}, query {q:?}, attempt {attempt}: recall order \
                     changed on an unchanged brain"
                );
            }
        }
    }
    assert!(
        compared_nonempty > 0,
        "every comparison was between two EMPTY results, so stability held \
         trivially and nothing was actually shown to be stable"
    );
}

/// **Two brains built from the same corpus in the same order return the same
/// results.** This is the "byte-reproducible across machines" half of the
/// claim: identical content must produce identical ranking, not merely stable
/// ranking within one process.
#[test]
fn two_brains_from_the_same_corpus_agree() {
    let mut compared_nonempty = 0usize;
    for seed in 1..=8u64 {
        let mut rng = Rng::new(seed * 0x27D4_EB2F);
        let corpus = gen_corpus(&mut rng, 14);

        let tmp_a = TempDir::new().unwrap();
        let tmp_b = TempDir::new().unwrap();
        drop(seeded_brain(&tmp_a, &corpus));
        drop(seeded_brain(&tmp_b, &corpus));

        let a = Brain::open(BrainConfig {
            read_only: true,
            ..config(&tmp_a)
        })
        .unwrap();
        let b = Brain::open(BrainConfig {
            read_only: true,
            ..config(&tmp_b)
        })
        .unwrap();

        for q in queries() {
            let cfg = RecallTopKConfig::default();
            let ka: Vec<String> = a
                .recall_topk_fts(&q, &cfg, Visibility::Private)
                .unwrap()
                .into_iter()
                .map(|h| h.key)
                .collect();
            let kb: Vec<String> = b
                .recall_topk_fts(&q, &cfg, Visibility::Private)
                .unwrap()
                .into_iter()
                .map(|h| h.key)
                .collect();
            if !ka.is_empty() {
                compared_nonempty += 1;
            }
            assert_eq!(
                ka, kb,
                "seed {seed}, query {q:?}: two brains built from an identical \
                 corpus disagreed on recall order"
            );
        }
    }
    assert!(
        compared_nonempty > 0,
        "both brains returned nothing for every query, so agreement was \
         trivial"
    );
}

// ── forget ─────────────────────────────────────────────────────────

/// **For every corpus and every subset forgotten, none of the forgotten keys
/// recall and every surviving key still does.** Both halves matter: the first
/// is the deletion guarantee, the second is that deletion has bounded blast
/// radius.
#[test]
fn forgetting_a_subset_removes_exactly_that_subset() {
    // Both halves must actually occur across the run: something forgotten and
    // something kept. A run that forgot nothing, or everything, would satisfy
    // the per-key assertion while testing only one direction.
    let mut total_forgotten = 0usize;
    let mut total_kept = 0usize;
    for seed in 1..=10u64 {
        let mut rng = Rng::new(seed * 0x1656_67B1);
        let corpus = gen_corpus(&mut rng, 12);
        let tmp = TempDir::new().unwrap();
        let b = seeded_brain(&tmp, &corpus);

        // Forget a pseudo-random subset (never all of it).
        let doomed: Vec<&Gen> = corpus.iter().filter(|_| rng.below(2) == 0).collect();
        let doomed_keys: Vec<String> = doomed.iter().map(|g| g.key.clone()).collect();
        for k in &doomed_keys {
            b.forget(k).unwrap();
        }

        total_forgotten += doomed_keys.len();
        total_kept += corpus.len() - doomed_keys.len();

        for g in &corpus {
            let present = b.get_memory_by_key(&g.key).unwrap().is_some();
            let expected = !doomed_keys.contains(&g.key);
            assert_eq!(
                present,
                expected,
                "seed {seed}: key {} presence is {present}, expected {expected} \
                 after forgetting {} of {} memories",
                g.key,
                doomed_keys.len(),
                corpus.len()
            );
        }

        // And nothing forgotten comes back through recall.
        for q in queries() {
            let hits = b
                .recall_topk_fts(&q, &RecallTopKConfig::default(), Visibility::Private)
                .unwrap();
            for h in &hits {
                assert!(
                    !doomed_keys.contains(&h.key),
                    "seed {seed}: forgotten key {} was returned by recall for {q:?}",
                    h.key
                );
            }
        }
    }
    assert!(
        total_forgotten > 0 && total_kept > 0,
        "the run forgot {total_forgotten} and kept {total_kept}; both must be \
         non-zero or only one direction of the property was exercised"
    );
}

// ── recall shape ───────────────────────────────────────────────────

/// **Recall never returns more than `k`, never returns duplicates, and never
/// returns a key that was not written.** Cheap structural invariants that a
/// ranking change could violate without any fixture noticing.
#[test]
fn recall_results_are_bounded_unique_and_real() {
    let mut examined = 0usize;
    for seed in 1..=10u64 {
        let mut rng = Rng::new(seed * 0x4F1B_BCDD);
        let corpus = gen_corpus(&mut rng, 20);
        let known: std::collections::HashSet<&str> =
            corpus.iter().map(|g| g.key.as_str()).collect();
        let tmp = TempDir::new().unwrap();
        let b = seeded_brain(&tmp, &corpus);

        for k in [1usize, 3, 5, 10] {
            let cfg = RecallTopKConfig {
                k,
                ..Default::default()
            };
            for q in queries() {
                let hits = b.recall_topk_fts(&q, &cfg, Visibility::Private).unwrap();
                assert!(
                    hits.len() <= k,
                    "seed {seed}: k={k} but recall returned {} hits",
                    hits.len()
                );

                let mut seen = std::collections::HashSet::new();
                for h in &hits {
                    examined += 1;
                    assert!(
                        seen.insert(h.key.clone()),
                        "seed {seed}: duplicate key {} in one recall result",
                        h.key
                    );
                    assert!(
                        known.contains(h.key.as_str()),
                        "seed {seed}: recall returned key {} that was never written",
                        h.key
                    );
                }
            }
        }
    }
    assert!(
        examined > 0,
        "no hit was ever inspected, so uniqueness and provenance were never \
         actually checked"
    );
}
