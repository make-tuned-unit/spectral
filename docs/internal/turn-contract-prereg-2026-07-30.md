# Preregistration — agent-turn contract (`Brain::turn`)

**Written 2026-07-30, BEFORE any measurement.** Per project discipline
(prereg → invariant tests → public doc → claims-gate), this file states the
claims, expectations, and decision rules in advance so the result cannot be
retrofitted to whatever the numbers turn out to be.

Origin: holistic architecture review (Codex, 2026-07-30). The review's
diagnosis was that Spectral's structural gap is not another retrieval lever —
the retrieval-lever family is exhausted and documented as such — but the
absence of a typed, outcome-bearing turn contract on the public API.

---

## The defect being fixed

Every recall entry point auto-reinforces **every returned hit** at retrieval
time and logs the full returned set as one co-access event
(`crates/spectral-graph/src/cascade_layers.rs`, the `write_back` block).

That credits **exposure, not usefulness**. All `k` hits are strengthened
before the consumer has filtered them. Two measured consequences already in
the record:

- 728/744 real events returned roughly the same ~40 memories
  (`docs/internal/LAST_LOOK.md`).
- Co-retrieval edges built from those events made real-query top-5 relevance
  ~3–4.5:1 **worse** (p≈0), which is why `co_retrieval_weight` defaults to 0.0
  (`docs/internal/tickets/coretrieval-regression.md`).

The co-retrieval weight was turned off, but the *mechanism that poisons the
signal* — exposure-credited feedback — was left in place. This contract fixes
the label at its source rather than muting its downstream consumer.

Second defect, same root: recall queries and recognition stimuli were not
distinguished as types. Feeding questions to a content re-encounter engine is
the documented cause of 0.9% real-query wing precision and 10.9% cascade
agreement (`docs/internal/RECOGNITION_BASELINE.md`).

## What was built

- `CascadePipelineConfig::write_back` (default **true** — legacy `recall_*`
  semantics are byte-for-byte unchanged).
- `spectral::Brain::turn(&TurnRequest) -> TurnResult` — read-only retrieval,
  recognition over `observations` only, returns a `TurnReceipt`.
- `spectral::Brain::record_turn_outcome(&TurnReceipt, &[(key, MemoryOutcome)])`
  — reinforces only `Used`; logs one event whose member set is the used set.
- `spectral_graph::brain::Brain::commit_outcome` — the synchronous,
  error-returning deferred counterpart to the best-effort `write_back`.
- `spectral::Brain::vacuum()` — the facade wrapper that
  `docs/DELETION_GUARANTEES.md` already promised but which did not exist on
  the public surface.

## Claims

- **T1 — Retrieval parity.** `turn` delivers the same hit ids in the same
  order as `recall_cascade_scoped` under default config.
- **T2 — Read-only retrieval.** A turn that is never committed leaves signal
  scores and ranking unchanged, for any number of repeats.
- **T3 — Outcome asymmetry.** Only `Used` reinforces. `Wrong` and `Ignored`
  never strengthen a memory and never build a positive association.
- **T4 — Attribution.** An outcome naming a key the turn did not deliver is
  rejected, not silently absorbed.
- **T5 — Recognition separation.** Recognition runs over `observations` and
  never over `query`; verdicts are identical to standalone `recognize`.
- **T6 — No legacy behavior change.** `CascadePipelineConfig::default()
  .write_back == true`; existing consumers are unaffected.
- **T7 — Deletion reachability.** The documented `forget` → `vacuum` erasure
  path is callable on `spectral::Brain` and physically erases.

## Gates (all $0, all must pass before any spend)

| Claim | Test | Status |
|---|---|---|
| T1 | `turn_contract.rs::turn_delivers_same_hits_as_legacy_cascade_path` | PASS |
| T2 | `turn_contract.rs::uncommitted_turns_do_not_change_signal_scores` | PASS |
| T3 | `turn_contract.rs::only_used_outcomes_reinforce` | PASS |
| T4 | `turn_contract.rs::outcome_for_undelivered_key_is_rejected` | PASS |
| T5 | `turn_contract.rs::recognition_runs_only_over_observations` | PASS |
| T6 | `turn_contract.rs::legacy_recall_still_writes_back_by_default` | PASS |
| T7 | `deletion_via_public_api.rs::public_forget_then_vacuum_physically_erases` | PASS |

Plus the existing workspace suite must stay green (no regression to the
integration gate).

## Decision rules — stated in advance

1. **Any T1 mismatch that is not fully explained kills the migration.** The
   turn path is not allowed to quietly change what gets retrieved. If hits
   differ, the contract is wrong, not the benchmark.
2. **T3 is non-negotiable.** If `Wrong`/`Ignored` are ever observed to
   strengthen a memory, the outcome path is reverted entirely.
3. **Systems kill line.** Recall-only p95 may regress by at most 5% versus the
   legacy path; combined recall+recognition p95 must not exceed today's two
   sequential calls. If exceeded, keep the APIs typed but do **not** fuse
   execution. *(Not yet measured — see Open below.)*
4. **No adaptive or recognition-value claim ships** until production outcomes
   include **both polarities**. A corpus of only-positive outcomes cannot
   distinguish a working feedback loop from a popularity loop, which is the
   exact failure this contract exists to prevent.
5. Held-out end-to-end results are published **regardless of direction**.

## Explicitly NOT claimed

- No accuracy improvement is claimed. This changes *when and on what* learning
  happens, not what is retrieved (T1 pins that). Any accuracy claim requires a
  held-out run under rule 5.
- No claim that outcome-credited co-retrieval beats exposure-credited
  co-retrieval. That is the *hypothesis* this unblocks; it is untested until
  real outcome data exists, and `co_retrieval_weight` stays at 0.0 until then.
- No exactly-once guarantee on outcome commits. Reinforcement is additive and
  clamped, so replaying a commit is safe but not a no-op. Callers needing
  exactly-once dedupe on `TurnReceipt::id`, which is content-addressed over
  the delivery.

## Open / deferred

- **Latency (rule 3) is not yet measured.** Must be run before the contract is
  recommended to Permagent as the default path.
- ~~**Durable receipts deferred.**~~ **BUILT 2026-07-30 (same day).** The
  deferral reasoning — "auditability, not behavior" — was **wrong**. Without
  the ledger, `Wrong` and `Ignored` exist only in a returned `OutcomeReceipt`
  and are discarded; only `Used` ids reach the database. So Spectral could not
  answer *"delivered repeatedly and never used"* at all — the single question
  negative evidence exists to answer. See the ledger claims T8–T13 below.
- **Bench harness not yet rewired.** The review's credibility argument is that
  `spectral-bench-accuracy` should retrieve through `spectral::Brain` +
  `TurnPolicyVersion` instead of its own private implementation. `turn` is the
  surface that makes this possible; the migration itself is separate work and
  is where the in-sample→product-configuration credibility gain is actually
  realized.
- **The co-retrieval index will be a BLEND, and this matters.**
  `rebuild_co_retrieval_index` reads `SELECT memory_ids_json FROM
  retrieval_events` with **no filter on `method`**
  (`crates/spectral-ingest/src/sqlite_store.rs:3214`). So once both paths are
  in use, the index is built from a mixture of exposure-credited legacy
  `cascade` events (full returned set) and outcome-credited `turn:v1` events
  (used set only). Any future evaluation of outcome-credited co-retrieval
  **must** either filter by method or rebuild from a turn-only event corpus —
  otherwise the legacy events dilute the very effect being measured, and a
  null result would be uninterpretable. Turn events are deliberately tagged
  `turn:<policy>` so this filtering is possible; the filter itself is not yet
  built because there is no turn-event corpus to filter yet.
- **Blocked on Permagent.** The outcome polarities this contract collects are
  exactly what the 2026-07-29 Tier-C dispatch asked for
  (`docs/internal/DISPATCH-permagent-production-replay-2026-07-29.md`). This
  work makes Spectral able to *receive* that data; it does not substitute
  for it.

---

# Addendum — the outcome ledger (2026-07-30, later same day)

Origin: an adversarial debate between me and Codex over whether "harmony" in
this codebase is a read-path or write-path property. Codex proposed a read-path
sensor-fusion estimator (FlowGuard) and **withdrew it**, expecting its own
ablation gate to fail. I proposed an automatic lifecycle (outcome-driven
consolidation, usage-based decay) and **it was defeated on actionability**.
Both of us converged on this instead.

## Why the lifecycle proposal was rejected (record it so it is not reopened)

Codex's objections, accepted:

- `Used` means "the caller consumed this hit", **not** "this improved the answer".
- Co-used memories may be **complementary facts that must stay separate**, not
  redundant ones safe to merge.
- `Ignored` is **censored** evidence: a good memory can be ignored for ranking
  40th, for being duplicated elsewhere in context, or because the actor failed.
- **Repeated delivery is partly a property of the retriever.** Penalising it
  would let the write path erase evidence of a read-path defect.
- Automatic hard forgetting would convert a noisy behavioural proxy into
  irreversible data loss. Keeping `forget` explicit is correct.

**Therefore: no automatic consolidation, decay, or forgetting is built, and
none may be built on this evidence until real bipolar production outcomes
exist and a separate prereg is written.**

## Supporting measurement (mine, real corpus, $0)

On the production brain (1,867 memories): the co-retrieval graph derived from
exposure-credited `retrieval_events` has 178,376 edges = **10.2% of all
possible memory pairs**. `consolidation_candidates` runs union-find over it, so
it percolates — largest connected component **99.0% of the graph (56.3% of the
corpus) at `co_count>=5`**, and still 91.4% at `>=20`. Consolidation candidacy
is therefore structurally degenerate today. This is *motivation* for better
evidence; it is **not** validation that outcome-credited evidence fixes it.

## Claims (ledger)

- **T8 — Exposure survives.** Two identical deliveries record two distinct
  occurrences; delivery digest shared, occurrence ids distinct.
- **T9 — Durability.** Rank and outcome survive close/reopen.
- **T10 — Idempotence.** Replaying a commit changes neither counts nor scores.
- **T11 — Asymmetry + completeness.** `Used` reinforces exactly once;
  `Wrong`/`Ignored`/`Unreported` never reinforce; an unreported member persists
  as `unreported` rather than vanishing.
- **T12 — Atomicity of rejection.** A commit naming an undelivered key leaves
  ledger and scores untouched.
- **T13 — Deletion.** `forget` cascades to ledger rows (the tables hold memory
  id *and* key, so without this the D1 substrate sweep would find residue).

| Claim | Test (`crates/spectral/tests/turn_ledger.rs`) | Status |
|---|---|---|
| T8 | `identical_deliveries_are_two_ledger_occurrences` | PASS |
| T9 | `ledger_survives_reopen_with_exact_rank_and_outcome` | PASS |
| T10 | `replaying_an_outcome_commit_is_a_no_op` | PASS |
| T11 | `only_used_reinforces_and_all_outcomes_are_recorded` | PASS |
| T12 | `rejected_outcome_leaves_ledger_and_scores_unchanged` | PASS |
| T13 | `forget_erases_ledger_rows` | PASS |
| — | `delivered_never_used_is_answerable` | PASS |

## Explicitly NOT claimed by the ledger

This validates **evidence integrity only**. It does not claim better memory,
better retrieval, or better accuracy. It makes a previously unanswerable
question answerable, and nothing acts on the answer.

## Corrected design defect

`TurnReceipt::id` was originally content-addressed over the delivery, and an
earlier test asserted two identical deliveries share an id — as a *feature*.
That was wrong for a ledger: collapsing repeat deliveries undercounts exposure.
`id` is now a unique occurrence id and `delivery_digest` carries the
content-addressed shape.

## Still open

- `rebuild_co_retrieval_index` (`sqlite_store.rs:3213`) still reads all
  `retrieval_events` with **no `method` filter**, so `turn:v1` used-set events
  are diluted by legacy exposure events. Independently flagged by both
  reviewers. Must be filtered before any outcome-credited co-retrieval is
  evaluated, or a null result is uninterpretable.
- Latency kill-line (recall p95 ≤ +5%) still unmeasured.
- Bench harness still not rewired through `spectral::Brain`.

## Anti-recommendations recorded (from the same review)

- **Do not** transplant the bench's shape router into `Brain` merely to claim
  benchmark/product parity. It moves tuned code without fixing feedback labels.
  Retrieval is near ceiling and repeatedly fails to convert into accuracy
  (`docs/MEASURED_RECORD.md`); only 5 of 94 headline failures retrieved zero
  answer keys (`docs/internal/N500_FAILURE_ANALYSIS.md`).
- **Do not** optimize recognition enrollment with LSH yet. MinHash already
  wins the lexical recognition regime and LSH trades recall for scale;
  without negative production outcomes this optimizes an internal proxy
  before proving recognition changes behavior usefully.
