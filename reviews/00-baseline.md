# Phase 0 — Ground truth baseline (2026-08-14)

Repo: `/Users/j/Documents/dev/spectral` @ `fix/r17-r18-order-by-tiebreaks` (HEAD `0422094`)
In scope: `crates/`, `scripts/` · Out of scope: `target/`, datasets, docs artifacts, generated files
Promise doc: `docs/pitch.md`

## 0.1 Reviewer reachability (smoke calls)

| Reviewer | Channel | Result |
|---|---|---|
| Codex | codex-cli 0.147.0, ChatGPT login | `CODEX-SMOKE-OK` |
| Opus 5 | Agent tool, model=opus | `OPUS-SMOKE-OK` |
| cursor | cursor-agent 2026.08.11 (`-p -f` required non-interactively) | `CURSOR-SMOKE-OK` |

All three reachable. Proceeding with three.

## 0.2 Claim list extracted from docs/pitch.md

Testable claims, numbered. AMBIGUOUS = not testable as written without a
definition the doc does not give.

- **C1 — Determinism of recall.** Recall results are deterministic /
  byte-reproducible for a given brain state ("all deterministically",
  "byte-reproducible"). Testable: repeated-query identity, ORDER BY tiebreak
  audit (this branch's R17/R18 work is exactly this surface).
- **C2 — Zero model calls on recall + recognition.** "Recall and recognition
  make zero model calls (`recognition_token_cost == 0` is structural)."
  Testable: no network/LLM dependency reachable from those code paths; the
  structural token-cost invariant exists in code.
- **C3 — Single-file SQLite storage.** One `Brain` handle over one SQLite
  file; embedded library, no service. Testable: storage layer inspection.
- **C4 — Recall = FTS5 + BM25.** Testable: retrieval implementation.
- **C5 — Recognition mechanism as described.** Landmark fingerprinting +
  winnowed k-grams + scoring, returning a familiarity/novelty verdict with
  the exact features behind it. Testable: recognition module + its outputs.
  (Guardrail scope: near-duplicate/verbatim, NOT paraphrase.)
- **C6 — Typed knowledge graph, 2-hop, ontology-validated.** Testable.
- **C7 — Episodic/temporal recall exists.** Testable.
- **C8 — Adaptive feedback loop.** Used memories strengthen, unused decay.
  Testable: decay/strengthen logic + covering tests.
- **C9 — Read-time federation: provenance-ranked, visibility-scoped.**
  Testable: federation module.
- **C10 — "Poisoning-resistant" federation.** **AMBIGUOUS** as written: no
  attack model stated in the pitch; the guardrails themselves concede sybil
  resistance is a deployment-trust property, not a code guarantee. Need: a
  stated threat model (which attacks are in scope) to verify beyond "some
  mitigation code exists".
- **C11 — 98.6% session-recall on LongMemEval-S.** CANNOT VERIFY by re-run on
  this host (LongMemEval dataset not present; prior-host artifacts gone).
  Verifiable only against recorded artifacts (`docs/RESULTS.md`,
  `benches/RESULTS.md`) for internal consistency + the guardrail that it is
  never quoted without C12.
- **C12 — 81.5% end-to-end accuracy (401/492) on LongMemEval-S.** Same status
  as C11; additionally the guardrail requires disclosing the optional Haiku
  query-expansion call behind it.
- **C13 — "$0 per query" for all six memory kinds.** Testable as "no paid
  call on any of the six query paths" (recall, recognition, relational,
  episodic, adaptive, federated). The pitch's own guardrail carves out the
  benchmark's optional query expansion — the claim is about the library path.
- **C14 — Apache-2.0.** Testable: LICENSE.

## 0.3 Current state (build / test / lint / typecheck / audit)

Raw log: `reviews/baseline-raw.log`. Run at HEAD `0422094`, 2026-08-14.

| Check | Command | Result | Exit |
|---|---|---|---|
| Build (also typecheck) | `cargo build --workspace --all-targets` | Finished dev profile, 45.79s | 0 |
| Tests | `cargo test --workspace` | **914 passed, 0 failed, 0 ignored** across 106 test binaries (57 contain 0 tests — bin targets and integration files without unit tests) | 0 |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` | clean | 0 |
| Coverage | — | **NOT MEASURED** — no tool installed (tarpaulin/llvm-cov absent); recorded as a baseline gap, not silently skipped | n/a |
| Dependency audit | `cargo audit` (cargo-audit v0.22.2, installed for this baseline) | **2 vulnerabilities, 1 unmaintained warning** — details below | 1 |

### Dependency audit detail

- **RUSTSEC-2026-0185 (HIGH 7.5)** — `quinn-proto` 0.11.14, remote memory
  exhaustion; fix ≥0.11.15. Present in `Cargo.lock` but **not in the default
  feature graph** (`cargo tree -i quinn-proto` is empty without
  `--all-features`) — reachable only via an optional feature. Reviewers:
  confirm which feature pulls it and whether any shipped configuration does.
- **RUSTSEC-2026-0204** — `crossbeam-epoch` 0.9.18, invalid pointer deref in
  `fmt::Pointer`; fix ≥0.9.20. Dev-dependency-only chain:
  criterion → rayon → crossbeam-deque → crossbeam-epoch. Not shipped.
- **RUSTSEC-2024-0436 (warning)** — `paste` 1.0.15 unmaintained.

### CORRECTION (recorded after Phase 3, before Phase 5 item 2)

**The test figure originally recorded here was wrong.** The Phase 0 capture
piped `cargo test --workspace` through `tail -40`, which silently truncated
the per-binary results; I recorded the visible tail (71 passed / 20 binaries /
11 empty) as if it were the whole run. The true baseline is **914 passing
tests across 106 test binaries** (verified untruncated at `d5ce897`, which
includes item 1's 4 new tests: 918 − 4 = 914; full capture in
`reviews/baseline-tests-full.log`).

Consequences, stated plainly:
- **All three reviewers were given the wrong number in the Phase 1 brief.**
  They were told the suite was thin. None of them contradicted it, and none
  filed a finding that depends on the count either way.
- The register's "missed by all three" bullet about 11 empty binaries is
  **retracted** — it was an artifact of my truncation, not a property of the
  repo.
- What survives unchanged: coverage is still not measured (no tool), and
  OP-14's finding that two specific invariant tests cannot fail is about
  test *quality*, not test count, and is unaffected.

Notes for reviewers:
- A long-running benchmark process (R31) is active on this machine; it touches
  only `~/spectral-local-bench`, not the repo.
