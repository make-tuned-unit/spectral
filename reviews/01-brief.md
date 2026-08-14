# Independent production-readiness review — brief (identical for all reviewers)

You are one of several independent reviewers. You cannot see the others' work;
do not attempt to. Work alone from the repo and this brief.

## Scope

- Repo: `/Users/j/Documents/dev/spectral`, branch `fix/r17-r18-order-by-tiebreaks` (checked out at HEAD `0422094`)
- IN scope: `crates/`, `scripts/`
- OUT of scope: `target/`, `reviews/` (do NOT read this directory), `docs/` (context-only, not review target), `benches/`, datasets, lockfiles, generated files
- READ-ONLY review: do not modify any file; do not run build/test/write commands. Baseline results are supplied below. You may propose commands as repro steps without running them.
- Client promise doc (context only): `docs/pitch.md`

## Claims to verify (C1..C14)

- C1 Recall is deterministic / byte-reproducible for a given brain state.
- C2 Zero model calls on recall + recognition; `recognition_token_cost == 0` is structural.
- C3 One `Brain` handle over one SQLite file; embedded library, no service.
- C4 Recall = FTS5 + BM25.
- C5 Recognition = landmark fingerprinting + winnowed k-grams + scoring, returning a familiarity/novelty verdict with the exact features behind it (near-duplicate/verbatim scope, not paraphrase).
- C6 Typed knowledge graph, 2-hop, ontology-validated.
- C7 Episodic/temporal recall exists.
- C8 Adaptive feedback loop: used memories strengthen, unused decay.
- C9 Read-time federation across brains: provenance-ranked, visibility-scoped.
- C10 Federation is "poisoning-resistant" — AMBIGUOUS (no attack model stated). Assess what the code actually defends against; do not invent a threat model.
- C11 98.6% session-recall on LongMemEval-S — dataset not on this host; verify only internal consistency of recorded artifacts if you cite anything.
- C12 81.5% end-to-end accuracy (401/492) on LongMemEval-S — same status as C11.
- C13 $0 per query on all six library query paths (no paid/model call on recall, recognition, relational, episodic, adaptive, federated reads).
- C14 Apache-2.0 licensing is coherent (LICENSE vs manifests).

## Baseline facts (already measured; trust these, do not re-run)

- `cargo build --workspace --all-targets` → exit 0
- `cargo test --workspace` → 71 passed / 0 failed across 20 test binaries; 11 of those binaries contain 0 tests
- `cargo clippy --workspace --all-targets -- -D warnings` → exit 0
- Coverage: not measured (no tool installed)
- `cargo audit`: RUSTSEC-2026-0185 HIGH quinn-proto 0.11.14 (lockfile-only, NOT in default feature graph — identify which optional feature pulls it if you can); RUSTSEC-2026-0204 crossbeam-epoch 0.9.18 (dev-deps only via criterion); paste 1.0.15 unmaintained.

## Required output — EXACTLY this structure, markdown, nothing else

## Verdict per claim
For each C1..C14: `MET` | `PARTIAL` | `NOT MET` | `CANNOT VERIFY`
plus ONE sentence of evidence with file:line.

## Findings (max 15, ranked most severe first)
Each finding:
- ID (e.g. X-01), Title
- Severity: P0 blocks deploy / P1 ship-blocker for this client / P2 fix within 30 days / P3 nice-to-have
- Location: file:line
- Evidence: what in the code causes this (quote max 3 lines)
- Repro or proof: command, test, or trace; if none, write "static reasoning only"
- Proposed fix: concrete, scoped
- Confidence: high/med/low

## Production readiness
One of: SHIP / SHIP WITH FIXES / DO NOT SHIP — plus one paragraph why.

Hard requirements: every claim verdict and every finding must carry a real
file:line. No vague items ("consider refactoring", "improve error handling")
without a specific location and failure mode. Max 15 findings. Responses
violating these will be rejected and re-requested.
