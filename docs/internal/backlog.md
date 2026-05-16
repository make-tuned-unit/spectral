# Spectral product backlog

Living document. Append-only at the top (newest first). Each item is a candidate, not a commitment. Promote to a PR draft when prioritized.

Format per item: title, source, effort estimate, dependencies, why it matters, explicit out-of-scope. Effort is rough Spectral-CC hours, not calendar time.

---

## 2026-05-15 reconciliation — read-side closed, recognition substrate live

Eight investigations closed, three wrapper PRs shipped, Permagent enrichment activation live. Project has phase-changed from bench-tuning to recognition-substrate accumulation. This reconciliation folds in all evidence and updates the strategic frame accordingly.

### Closed investigations (2026-05-15)

**GENUINE_MISS reconstructive read pre-validation.** 1/4 flipped (case #8 tanks only). Multi-pass reconstructive pipeline rejected — below the >= 2 threshold. Structural floor confirmed: failure mechanisms span three granularities (sentence-level partial extraction, session-level topic filtering, unknown incomplete enumeration), and session-level isolation only addresses one. Analysis: `spectral-local-bench/analysis/genuine-miss-reconstructive-prevalidation.md`.

**Cross-extraction entity dedup defect scope check.** Affects exactly 1 question (#9 weddings) in a rejected pipeline (reconstructive read). Two prompt-level fix attempts failed; model treats relationship-descriptor variation as meaningful identity signal. Not worth a PR. Analysis: `spectral-local-bench/analysis/dedup-defect-scope-check.md`.

**Shipped components active/dormant audit.** 7 of 18 components ACTIVE (FTS, description-enriched FTS, TACT FTS fallback, signal score weighting, declarative density, recency decay, episode diversity). 11 DORMANT or UNREACHABLE — all correctly so (bench-limited or config-disabled). **0 real defects found.** The bench number (79.2%) is an honest measure of the active subset. Analysis: `spectral-local-bench/analysis/shipped-components-active-dormant-audit.md`.

**Librarian FTS format alignment.** The Librarian's description format is well-aligned with Spectral's FTS5 indexing. The "Related terms:" suffix compensates for FTS5's lack of stemming — the single most valuable structural feature. 99.4% description coverage on Henry's brain. No action needed. Analysis: `spectral-local-bench/analysis/librarian-fts-format-alignment.md`.

**Recall API / ambient boost contract spec.** Full contract produced. v1 fits: Permagent can activate ambient boost with `focus_wing` = project, `recent_activity` with per-episode wings, `persona` = mode, `session_id` = thread. No Spectral-side extension needed for v1. Analysis: `spectral-local-bench/analysis/recall-ambient-contract-spec.md`.

### Shipped wrapper PRs

**PR #123 — `annotate()` and `list_annotations()` delegation.** Wrapper gap closed. Permagent can use `spectral::Brain::annotate()` instead of direct inner-type access.

**PR #124 — `recall_cascade()` delegation.** Wrapper gap closed. `recall_cascade()` — the entry point for ambient-boost-aware retrieval — is now accessible through `spectral::Brain`.

**PR #125 — `rebuild_co_retrieval_index()` delegation.** Wrapper gap closed. Doc comment documents full-recompute semantics, atomic replace (concurrent read-safe), idempotency, and O(E) cost. Permagent can schedule periodic rebuilds through the wrapper.

### Permagent enrichment activation (2026-05-15)

Permagent's enrichment layer is live on Henry's brain. Retrieval events are streaming into `retrieval_events`. Ambient boost is active in production via `recall_cascade()` with populated `RecognitionContext`. This is the phase change — the recognition substrate is now accumulating real usage data for the first time.

---

## Strategic frame (read this first)

The read-side solution space is closed. Six investigations — session signal (#11), L2 episode summaries (#12), cascade max_confidence (#19), actor-level interventions, reconstructive read pre-validation, and the dedup defect scope check — systematically explored every remaining lever on the bench retrieval and actor paths. Each produced evidence, not opinion: session signal found no failure case it addresses; L2 summaries duplicate description-enriched FTS with zero incremental value; actor prompt interventions hit a structural ceiling (3 interventions tried, all failed); reconstructive multi-pass flipped 1 of 4 cases (below threshold); the dedup defect is 1 question in a dead pipeline. The shipped-components audit confirmed 0 defects in the 7 active components. No stone left unturned, no investigation left open.

The bench number is 79.2% on LongMemEval-S. The components audit confirmed this is honest and complete for the bench-measurable layer. The 10.6pp gap to Memanto (89.8%) decomposes into: the actor-level GENUINE_MISS structural ceiling (4 cases), the exhausted judge rubric delta (2-3 cases), and the limits of BM25 FTS + re-ranking without co-retrieval or ambient context. Critically, the last component — co-retrieval and ambient signals — cannot be measured on bench by design, because LongMemEval uses ephemeral brains with no usage history and no runtime context.

The recognition layer is built. Ambient boost, co-retrieval ranking, auto-reinforce, retrieval event logging, and session signal are all shipped and wired into the cascade pipeline. As of 2026-05-15, this layer is live in production: Permagent's enrichment layer is streaming retrieval events from Henry's real usage, `RecognitionContext` is populated with focus wing and recent activity, and ambient boost is active. The substrate — the accumulated retrieval events, co-retrieval pairs, and reinforcement signal that make the recognition layer's ranking meaningful — is now accumulating for the first time on a real brain.

The measurement instrument for the recognition layer is Henry's brain, not LongMemEval. LongMemEval structurally cannot test the recognition loop because the bench has no usage history, no ambient context, no cumulative reinforcement. The dormant components (co-retrieval boost, ambient boost, auto-reinforce, context chain dedup, retrieval event logging) are dormant on bench for structural reasons, not bugs. Their production value is real but measurable only against real usage patterns.

The next program is substrate accumulation (~1-2 weeks), then measurement against a pre-committed framework (4 claims: ambient boost reranking utility, co-retrieval clustering signal, signal_score modulation via auto-reinforce, subjective-utility journal from Henry), then evidence-driven decisions on the post-substrate follow-ups (Canonicalizer migration, persona-as-ranking-signal, signal_score activation timing) and the constellation revisit. No new engineering work is queued until the substrate has cooked enough to measure. The project waits on data, not code.

Spectral's architectural bet remains unchanged: **deterministic recognition without LLM-in-loop**. The bench proves the retrieval and re-ranking core works (79.2%, 0 defects in active components). The recognition layer extends that core with signals that compound over usage time. Whether those signals earn their weight is the question the next measurement pass will answer.

---

## Tier 1 — High leverage, queue first

### 2. Co-retrieval signal in cascade ranking (audit-P4 minimum behavioral)

- **Source:** PR #76 foundation laid; behavioral use deliberately deferred.
- **Effort:** 4-6h
- **Depends on:** PR #76 (merged), bench checkpoint (done — baseline 73.3%).
- **Why it matters:** First behavioral payoff of the recognition loop. Queries `co_retrieval_pairs` during ranking and boosts memories frequently co-retrieved with current top hits. The deterministic mechanism that makes Path B real. Estimated +0.5-2pp on LongMemEval. Composes with item 17 (shape-routed actors) — preference and general-recall strategies benefit most. **Update (2026-05-15):** The shipped-components audit confirmed co-retrieval boost is DORMANT on bench (0 pairs — fresh ephemeral brains can't accumulate co-occurrence data). The boost is wired and active in production via Permagent; measurement requires Henry's brain, not LongMemEval.
- **Out of scope:** Time-decay on co_count (item 14), per-session weighting (item 11).

### 5. `spectral doctor` CLI command  [gbrain idea #7]

- **Source:** garrytan/gbrain — `gbrain doctor`, `gbrain skillpack-check`, structured JSON output with `actions[]` array.
- **Effort:** 3-4h
- **Depends on:** Nothing.
- **Why it matters:** Health probe with structured output and exit codes (0/1/2 for CI gating). Checks: schema migrations up to date, embeddings present for all memories, `retrieval_events` not corrupted, `co_retrieval_pairs` index built and fresh, `content_hash` backfilled. Becomes a primitive Permagent, CI, and the bench harness all reuse. The "observable behavior" hard requirement.
- **Out of scope:** Auto-fixing (`--fix` flag) — print actions but don't execute in v1.

### ~~17. Shape-routed actor strategies~~ — SHIPPED (PR #86)

- **Source:** Bench failure analysis 2026-05-11.
- **Status:** Shipped. Classifier extended to 8 variants, per-shape actor prompts via 8 markdown templates, retrieval path lifted from run-level to per-question (Temporal → topk_fts recovered the −15pp regression).

### 18. Strategy effectiveness telemetry analysis per bench run

- **Source:** Bench failure analysis 2026-05-11; enables items #15 and #16.
- **Effort:** 2-3h
- **Depends on:** Item 17 (shape-routed strategies — **shipped**). Needs `strategy_telemetry` field populated on results.
- **Why it matters:** Builds the analysis script that consumes per-question `strategy_telemetry` to produce a per-strategy accuracy report per bench run. Becomes the input to item #15 (per-PR ablation reporting) and item #16 (fail-improve loop). Without this, we ship strategies but can't measure which earn their weight.
- **Out of scope:** Automated regression detection. This is reporting, not gating.

### 19. Cascade `max_confidence=0.85` plateau investigation (INVESTIGATED — confirmed bug, not accuracy lever)

- **Source:** Bench failure analysis 2026-05-11.
- **Investigation (2026-05-14, PR #112):** Confirmed as a real bug — `.min(0.85)` clamp in `brain.rs:1189-1192` makes `max_confidence` non-functional (166/166 runs produce identical 0.85). Orchestrator with correct computation is bypassed. Two fix options: Option A (fix the shim — minimal) or Option B (wire through orchestrator — structural). Fix is telemetry-only; does not affect retrieval accuracy. The Temporal->topk_fts routing decision was based on accuracy, not confidence, so fixing this doesn't change routing. Enables future confidence-driven routing. Full analysis: `docs/internal/cascade-max-confidence-investigation.md`.

---

## Tier 2 — Useful, queue after Tier 1

### 6. Backfill orchestration: `spectral backfill --all`

- **Source:** Captured during PR #76 review. Three backfill methods now exist: declarative density, co-retrieval index, and content_hash (added in #85). Still no canonical entry point.
- **Effort:** 1-2h
- **Depends on:** Nothing.
- **Why it matters:** Every Permagent deployment needs a single command to bring an old brain forward. Pairs well with item 5 (doctor) — doctor reports gap, backfill closes it. Item #85 added a third backfill method, so this is more useful than when originally captured.
- **Out of scope:** Scheduling/automation — that's the consumer's job.

### 7. Auto-rebuild co-retrieval index on schedule

- **Source:** PR #76 followup. Currently manual via `Brain::rebuild_co_retrieval_index`. **Update (2026-05-15):** PR #125 delegated `rebuild_co_retrieval_index()` through the wrapper with full behavior documentation (full-recompute, atomic, idempotent, O(E)). Permagent can now call it through the public API; the remaining work is Permagent-side scheduler wiring.
- **Effort:** ~2h
- **Depends on:** Nothing.
- **Why it matters:** As `retrieval_events` accumulates, `co_retrieval_pairs` drifts unless rebuilt. Either Permagent's Librarian schedules it or Spectral exposes a "rebuild if stale" primitive. Cleaner if Spectral owns the staleness check; consumers shouldn't need to reason about index freshness.
- **Out of scope:** Background tokio task — provide the primitive, let consumer decide cadence.

### ~~8. Compiled-truth boost in cascade ranking  [gbrain idea #2]~~ — SHIPPED + VALIDATED (PR #117)

Foundation shipped (PRs #104/#108), bench validation complete (PR #117). Isolated description-content lift: **+2.5pp** (75.0% -> 77.5%), within the predicted +1-3pp range. Lift concentrated in temporal-reasoning (+10pp); multi-session showed zero lift because that bottleneck is actor synthesis, not retrieval. RETRIEVAL_MISS cases showed the vocabulary-bridging mechanism working (previously-missing answer sessions surfaced) but the actor ceiling absorbed the retrieval gain — questions did not flip. Full analysis: `docs/internal/item-8-bench-validation.md`.

### 9. Filtered `list_undescribed` (by wing/age/source)

- **Source:** Permagent CC mentioned as future Spectral work in their integration report.
- **Effort:** ~2h
- **Depends on:** Nothing.
- **Why it matters:** Permagent's Librarian wants to describe memories selectively — newest first, only certain wings, etc. Currently `list_undescribed` returns the full bag. Adding filter params makes the Librarian's scheduling logic 10x simpler.
- **Out of scope:** Full query DSL — just the three filters Permagent has asked for.

### 10. `MemoryHit` carries `description` consistently

- **Source:** PR #75 review. `description` is on `Memory` but only on `MemoryHit` from certain code paths.
- **Effort:** 1h investigation, 1-2h fix.
- **Depends on:** Nothing.
- **Why it matters:** Consumers expect `MemoryHit.description` to be populated everywhere it's available. Today the ranking pipeline doesn't propagate it through every path. Trivial bug, easy to verify, no risk.
- **Out of scope:** Description propagation in non-`MemoryHit` types.

### 20. Judge rubric per-category revision (deferred Change D from PR #84) — first attempt REVERTED

- **Source:** Deferred from PR #84 to avoid attribution muddling.
- **Effort:** 2-3h
- **Depends on:** Bench checkpoint (done) and ideally item 17 shipping first (so we can attribute lift cleanly between actor changes and judge changes).
- **Why it matters:** 3 DEFINITION_DISAGREEMENT failures (#1 clothing, #2 projects, #6 citrus) are judge-side, not actor-side. Category-specific rubrics could distinguish defensible-different-count from genuine-miss.
- **First attempt (PR #102, reverted PR #103):** Reasoning-aware +-1 tolerance for counting questions. Zero lift on target cases — all 3 DEFINITION_DISAGREEMENT cases remained incorrect. The "deliberation" bar was too strict: the judge required explicit meta-reasoning, not just exhaustive evidence documentation. Post-mortem: `docs/internal/item-20-reasoning-aware-judge-proposal.md` Section 0. Needs a different approach — either two-call judge or structural signal detection.
- **Out of scope:** Multi-judge ensemble, model swapping for judge — keep Sonnet 4.5 as judge.

### ~~21. Retrieval telemetry in bench reports (`memory_keys` population)~~ — SHIPPED via PR #107

Shipped 2026-05-14. `retrieve_cascade()` and `retrieve_topk_fts()` now return raw `MemoryHit` vectors alongside formatted strings. `memory_keys` reliably populated for Cascade, TopkFts, and Tact paths. Graph path still falls back to string parsing. **AMBIGUOUS cases resolved (PR #117):** Item #8 bench validation confirmed cases #8 (tanks) and #9 (weddings) as GENUINE_MISS — all answer sessions retrieved in both conditions; actor fails to count. Candidate C no longer gated on this resolution.

### 22. Enable spectrograms in bench ingest

- **Source:** Deferred from PR #82 (split into preflight-only PR #110 + this item). Preflight subcommand shipped on main via PR #110.
- **Effort:** 1h code change + 1 bench run for attribution.
- **Depends on:** Item #8 bench validation complete — **now satisfied** (PR #117). Spectrogram impact can now be measured independently.
- **Why it matters:** Spectrograms feed `signal_score` via the spectrogram analyzer, which feeds re-ranking. Enabling them changes bench behavior. Must run as an isolated experiment to measure impact on accuracy — don't bundle with other retrieval changes or attribution is confounded. PR #82's `enable_spectrogram: true` change to `ingest_question()` is the implementation; the preflight subcommand (now on main via PR #110) can verify coverage. **Baseline for this measurement is now 77.5%** (descriptions-enabled main), not the old 73.3%. **Update (2026-05-15):** Shipped-components audit confirmed spectrogram analysis is UNREACHABLE on bench (config-disabled, item #22 is the gate). TACT fingerprint search (tier 1) is also UNREACHABLE for the same reason. This is the only UNREACHABLE component with a clear one-flag path to activation.
- **Out of scope:** Spectrogram-conditioned retrieval (that's further architectural work). This item is just flipping the flag and measuring.

---

## Tier 3 — Architectural / longer horizon

### 11. Audit-P4 full scope: session signal in ranking (DEFERRED — investigated 2026-05-13)

- **Source:** Audit P4. PR #79 shipped the data capture; behavioral use deferred.
- **Effort:** 6-8h
- **Depends on:** Bench checkpoint (done), some usage data accumulated in `retrieval_events`.
- **Why it matters:** Closes the ambient state loop. Recency-weighted ranking via active session. The biggest architectural change still pending. Deferred deliberately until we have real usage data to inform what session signal *should* weight — guessing now means redesigning later. **Composes with item 17:** GeneralRecall strategy benefits most from session-priority retrieval.
- **Investigation (2026-05-13, PR #106):** Evaluated 5 candidate session signals against documented failures. None addresses existing failure cases — GENUINE_MISS is actor-level (embedded references), not ranking-level. Existing per-memory signals subsume session-level approximations. Candidate topic-density is actively counterproductive. Defer until: (1) a failure is found where retrieved sessions rank below K cutoff, (2) Permagent production usage data, (3) item #12 creates session-level metadata, or (4) actor improvements shift bottleneck to ranking. PR #106 also corrected PR #99's failure classification: cases #8 (tanks) and #9 (weddings) reclassified from GENUINE_MISS to AMBIGUOUS — retrieval status unverified (memory_keys was empty). Full analysis: `docs/internal/item-11-investigation.md`.
- **Out of scope:** Multi-device session reconciliation, session lifecycle management.

### 12. L2 cascade layer: episode summaries (DEFERRED — investigated 2026-05-14)

- **Source:** TACT whitepaper, queued as PR 2 of the cascade trilogy.
- **Effort:** 1 week (single largest item in the backlog).
- **Depends on:** Episode data model (partially exists), consumer-provided `episode_id` API (RememberOpts already has it).
- **Why it matters:** The layer between AAAK (L1) and constellation/TACT (L3).
- **Investigation (2026-05-14, PR #108):** L2 summaries duplicate item #8's FTS-bridging mechanism at episode granularity with zero incremental value for documented failures. The compression-vs-embedded-reference tension is structural: summaries lose the detail that the hardest failures require, and the LLM summarizer faces the same attention problem as the actor. The backlog's original "+3-4pp" estimate was written before PRs #93, #99, and #70 reframed the multi-session bottleneck as actor attention, not retrieval grouping. Defer until: (1) item #8 ships and demonstrates a ceiling, (2) structured summary format validated for embedded-reference recall, (3) production usage data, or (4) actor attention problem solved. Full analysis: `docs/internal/item-12-investigation.md`.
- **Out of scope:** L4 vector layer (deferred), L0 filesystem layer (PR 3 of cascade trilogy).

### 13. Per-session summaries dual-index retrieval (audit Proposal 5)

- **Source:** Spectral architecture audit, Proposal 5.
- **Effort:** 3-4h
- **Depends on:** Nothing — independent of L2 work.
- **Why it matters:** Targets complete-miss failures. Pure retrieval tuning, independent of recognition work. Estimated +1-2pp on LongMemEval. Complements item 12 rather than replacing it.
- **Out of scope:** LLM-mediated summary generation (that's Librarian's job).

### 14. Time-decay on `co_count`

- **Source:** PR #76 review, deferred deliberately.
- **Effort:** 2-3h once design is settled (longer with design).
- **Depends on:** Audit-P4 full scope (item 11).
- **Why it matters:** A pair retrieved 100 times last year vs 100 times this week look identical right now. For v1 "related memories" this is fine. As session-aware ranking lands, recency starts mattering more.
- **Out of scope:** Anything until item 11 ships.

### 15. Benchmarked ablation reporting per PR  [gbrain idea #3]

- **Source:** garrytan/gbrain — "v0.11->v0.12 moved P@5 from 22.1% -> 49.1% on identical inputs."
- **Effort:** 2-3h to scaffold per-PR delta reporting in `spectral-bench-accuracy`.
- **Depends on:** Item 18 (strategy telemetry analysis) — the analysis script becomes input to this reporting layer.
- **Why it matters:** Per-PR attribution. "+X from co-retrieval boost, +Y from compiled-truth boost, +Z from L2 episodes." Tonight's bench established the discipline (per-category before/after, locked in `bench-failure-analysis-2026-05-11.md`). This item formalizes it for future PRs.
- **Out of scope:** Continuous benchmarking infrastructure — keep it manual and reproducible.

### 16. Fail-improve loop for deterministic classifiers  [gbrain idea #5]

- **Source:** garrytan/gbrain — "Deterministic classifiers improve over time via a fail-improve loop that logs every LLM fallback and generates better regex patterns from the failures."
- **Effort:** 4-6h.
- **Depends on:** Item 18 (strategy telemetry analysis) — provides the misclassification log.
- **Why it matters:** Generalizes the recall->recognition loop (#73) to classifier improvement. Every time a deterministic classifier produces "General" (the catch-all), log the input. Periodically mine the logs for missing rules. The pattern matters because "deterministic where possible, LLM as escape hatch" is a Spectral architectural commitment — making the deterministic layer self-improving turns the commitment into a moat.
- **Out of scope:** Auto-generating regex from logs (research project). Start with logging + manual review.

---

## Tracked — post-substrate follow-ups

These items surfaced during the 2026-05-15 investigations and are tracked with explicit trigger conditions. None is active. All wait on substrate accumulation and measurement.

### 23. Canonicalizer migration (post-substrate)

- **Source:** Permagent enrichment activation analysis, 2026-05-15.
- **Effort:** 4-6h investigation + implementation TBD.
- **Why it matters:** Permagent shipped synthetic `canonical_id` values (term:, cat: prefixes) instead of Canonicalizer-resolved entity-level IDs, because Canonicalizer fuzzy-match on common words ("model", "platform") produced unreliable matches. String-level matching is fine for graph view but limits recognition-layer signal quality — entity-level resolution would let co-retrieval and ambient boost operate on semantic entities rather than string tokens.
- **Trigger:** Revisit once retrieval events accumulate enough to measure whether string-level matching produces sufficient signal for recognition-layer quality. If string-level matching is adequate, this stays deferred.
- **Out of scope:** Canonicalizer algorithm changes — this is about the migration decision, not the fuzzy-match quality.

### 24. Persona-as-ranking-signal (post-substrate)

- **Source:** Henry's introspection during ambient boost design review, 2026-05-15.
- **Effort:** 2-3h to prototype, bench measurement TBD.
- **Why it matters:** Ambient boost currently reads `persona` only as a gate (prevents `is_empty()` short-circuit) but doesn't weight by it. The same query feels different across Henry's 5 operating modes — there's a real discrimination signal sitting unused. The recognition layer should surface different memories for "developer-mode Jesse" vs "fitness-mode Jesse" given the same query.
- **Trigger:** Activate once retrieval events show enough variation across personas to tune the weighting. Requires persona field populated consistently on retrieval events.
- **Out of scope:** Persona taxonomy — Permagent defines the modes, Spectral consumes them.

### 25. signal_score activation timing (post-substrate)

- **Source:** signal_score architecture review, 2026-05-15.
- **Effort:** 1h decision + possible UI/API changes.
- **Why it matters:** signal_score currently reflects initial ingest values; auto-reinforce will modulate it as retrieval events accumulate. The "rank Brain view by signal_score" decision was deferred because the distribution doesn't meaningfully diverge from initial values yet. Once auto-reinforce has cooked enough events, signal_score becomes a genuine relevance signal worth exposing.
- **Trigger:** Revisit once auto-reinforce has processed enough events that the signal_score distribution meaningfully diverges from initial values. Measurement: compare signal_score histogram before vs after reinforcement on Henry's brain.
- **Out of scope:** Changing signal_score computation — this is about when to surface it, not how to compute it.

### 26. Un-delegated Brain wrapper methods

- **Source:** PR #123 follow-up (24+ un-delegated methods identified), PR #125 (5 co-retrieval-adjacent methods identified: `related_memories`, `count_retrieval_events`, `count_retrieval_events_by_method`, `events_for_session`, `memories_for_session`).
- **Effort:** ~1h per method (mechanical delegation + tests).
- **Why it matters:** The `spectral::Brain` wrapper exists so consumers don't couple to the inner type. Every un-delegated method forces consumers to either bypass the wrapper or write their own logic. PRs #123, #124, and #125 closed the three gaps Permagent actively hit (annotate, recall_cascade, rebuild_co_retrieval_index).
- **Policy:** Close-when-consumer-asks. Don't add proactively — batch when Permagent or another consumer hits a specific gap. The remaining ~24 methods are not blocking any current consumer.
- **Out of scope:** Proactive delegation of all methods.

### 27. Constellation revisit point

- **Source:** Shipped-components audit (2026-05-15): TACT fingerprint, spectrogram, and AAAK remain UNREACHABLE per the components audit.
- **Effort:** Investigation — the build decision follows the measurement.
- **Why it matters:** The strategic deferral ("can't measure on bench") was correct — confirmed by the components audit. But the deferral can't be indefinite. Once recognition-layer signal on Henry's brain is measurable, the constellation question (peak-pair fingerprinting, spectrogram-conditioned retrieval, L3 cascade) becomes evaluable on the right instrument.
- **Revisit:** 2026-05-29 or first substrate-measurement review, whichever comes first.
- **Out of scope:** Building anything before the revisit. This is a measurement-then-decide checkpoint, not a build item.

### 28. Substrate measurement framework

- **Source:** Recognition substrate design conversations, 2026-05-15.
- **Effort:** 2-3h to formalize if not yet captured as a doc.
- **Why it matters:** A pre-committed measurement framework was designed with 4 claims: (1) ambient boost reranking utility, (2) co-retrieval clustering signal, (3) signal_score modulation via auto-reinforce, (4) subjective-utility journal from Henry. This framework guides the first real evaluation of the recognition layer once substrate matures. Without pre-committed claims, measurement devolves into cherry-picking.
- **Status:** Framework lives in conversation history — **needs to be captured as a standalone doc** before the measurement review. This is owed work.
- **Out of scope:** Running the measurement — this item is capturing the framework, not executing it.

---

## Closed (since backlog established 2026-05-11)

- **Item 1 — Synthesis prompt revisions** -> PR #84 shipped. Bench lift attributed.
- **Item 3 — Bench checkpoint** -> 73.3% baseline locked at commit `e9a80d8`. Failure analysis in `docs/internal/bench-failure-analysis-2026-05-11.md`.
- **Item 4 — Content-hash dedup** -> PR #85 shipped, broader scope than originally captured (non-destructive write semantics + WriteOutcome + content hash + backfill).
- **Item 8 — Description-enriched FTS** -> Foundation shipped (PRs #104/#108), bench validation complete (PR #117). Isolated lift: +2.5pp (75.0% -> 77.5%), within predicted +1-3pp. Lift concentrated in temporal-reasoning; multi-session zero (actor ceiling). AMBIGUOUS cases #8/#9 resolved as GENUINE_MISS. Full analysis: `docs/internal/item-8-bench-validation.md`.
- **Item 21 — Retrieval telemetry** -> PR #107 shipped 2026-05-14. `memory_keys` now populated for Cascade, TopkFts, and Tact paths. AMBIGUOUS cases #8/#9 resolved via item #8 bench validation (PR #117).
- **GENUINE_MISS reconstructive read** -> Closed 2026-05-15. 1/4 flipped (below threshold). Structural floor confirmed across three failure granularities. Analysis: `spectral-local-bench/analysis/genuine-miss-reconstructive-prevalidation.md`.
- **Cross-extraction dedup defect** -> Closed 2026-05-15. #9-only, dead pipeline (rejected reconstructive read). Two fix attempts failed. Analysis: `spectral-local-bench/analysis/dedup-defect-scope-check.md`.
- **Shipped-components dormancy** -> Closed 2026-05-15. 0 defects found. 7 active, 11 dormant/unreachable (all correctly so — bench-limited or config-disabled). 79.2% is honest. Analysis: `spectral-local-bench/analysis/shipped-components-active-dormant-audit.md`.
- **Librarian FTS format alignment** -> Closed 2026-05-15. Aligned, no action needed. 99.4% description coverage, "Related terms:" suffix compensates for FTS5 no-stemming. Analysis: `spectral-local-bench/analysis/librarian-fts-format-alignment.md`.
- **Recall API / ambient boost contract** -> Closed 2026-05-15. Contract spec produced. v1 fits — no Spectral extension needed. Analysis: `spectral-local-bench/analysis/recall-ambient-contract-spec.md`.
- **Brain wrapper delegation (annotate, recall_cascade, rebuild_co_retrieval_index)** -> Closed 2026-05-15 via PRs #123, #124, #125. All three gaps Permagent actively hit are now closed.

---

## Deferred indefinitely (here for visibility, not action)

- **L4 vector layer in cascade.** Adding semantic search to FTS inverts the cascade.
- **Auto-running the Librarian on a schedule.** Permagent's task scheduler is the right place.
- **Re-description / refresh logic for stale descriptions.** Needs usage data first.
- **Wing/method filtering on co-retrieval queries.** Reintroduces path-dependence.
- **Background tokio tasks inside Spectral.** Consumers manage their own runtimes.
- **Config-file-driven gate registry for shape routing.** Inline regex is consistent style; promote only if gate set exceeds ~15 entries.
- **LLM-side classifier for question shape.** Deterministic classification is the architectural commitment.
- **`Brain::classify_question` as Spectral primitive.** Classifier lives in bench until a production consumer needs it.
- **Candidate C (per-session context isolation).** Reconstructive read pre-validation flipped 1/4 (below threshold). Structural floor confirmed. Not worth pursuing unless a new failure pattern emerges that session-level isolation specifically addresses.

---

## Track 2 — Topology (recognition substrate)

Topology features give memory the adjacency structure that
recognition-based retrieval traverses. Audit 2026-05-14 found
two distinct implementations at different maturity. This section
tracks the gap between them and the architectural work above.

### T1. Kuzu graph neighborhood — wire into cascade, or retire

- **Source:** Neighbours status audit 2026-05-14.
- **State:** Built (PRs #2/#3, 2026-04-27), inert. `neighborhood()`
  BFS over the Kuzu entity graph exists and is queryable, but only
  consumed by the experimental `--retrieval-path graph` bench flag.
  `recall_cascade()` never calls `recall_graph()`.
- **Effort:** Investigation first (is entity-graph adjacency
  additive over co-retrieval pairs already in ranking?), then
  either wiring work or a deliberate retirement.
- **Depends on:** Four preconditions (T2 investigation, PR #116):
  1. Item #8 bench validation complete — **now satisfied** (PR #117).
  2. Entity->memory bridging gap fix — `recall_graph()` currently
     detours through FTS on entity canonical names; this must be
     closed before the graph signal is meaningful.
  3. Ontology cleanup — ~385 auto-created phrase entities dilute
     graph signal; curate or improve extraction quality first.
  4. Kuzu neighborhood size measurement — deferred Q1 from T2;
     measure on Permagent brain before wiring.
- **Update (2026-05-15):** Shipped-components audit confirmed Kuzu graph is UNREACHABLE on the cascade path (not on the recall path, ontology empty on bench). This is correctly deferred — T1 preconditions are real gates, not busywork.
- **Why it matters:** Either this is an unused signal worth
  activating, or it's six-week-old dead code that should be named
  as such. The current ambiguous state is the worst option.
- **Out of scope:** Spectrogram-conditioned or peak-pair retrieval
  (see T3).

### T2. Topology design questions (Kuzu graph) — RESOLVED (PR #116)

All three design questions answered. Full analysis:
`docs/internal/t2-topology-design-decisions.md`.

- **Q1 Degree cap:** Deferred to T1. Measurement needed first —
  graph is inert so capping dead code is speculative. When T1
  begins, measure neighborhood sizes on Permagent brain, then
  decide.
- **Q2 Adjacency basis:** Keep pure explicit-predicate.
  Co-retrieval pairs already cover behavioral adjacency. No
  failure case shows blended adjacency would help.
- **Q3 Co-retrieval relationship:** Complementary in theory, but
  co-retrieval subsumes practical value today. Critical finding:
  the entity->memory bridging gap (`recall_graph()` detours
  through FTS on canonical names) plus ~385 auto-created
  phrase-entity ontology noise make the graph path low-signal.
- **Implication for T1:** Not a simple wiring task, not a clear
  retirement. Three additional preconditions established (see
  T1's updated "Depends on").

### T3. Peak-pair fingerprinting (recognition layer)

- **Source:** TACT constellation whitepaper; cascade trilogy
  planning. Formerly tracked informally as "PR 2."
- **State:** Not built. The current `find_resonant()` in
  spectral-spectrogram does dimensional similarity matching
  (nearest-neighbor in feature space), not Shazam-style
  combinatorial peak-pair hashing.
- **Effort:** 1-2 weeks. Largest item in this section.
- **Depends on:** The schema-synthesis-vs-raw-memory question
  resolved first (whether peaks are detected in raw memory or
  constructed at synthesis time). Dispatching peak-pair work
  before that decision risks building the matcher on the wrong
  substrate.
- **Why it matters:** This is the Track 2 recognition core — the
  relational structure no vector system captures. But the #113
  structural-ceiling finding means its *bench* payoff is
  uncertain; its real payoff is production recognition quality.
  Sequence honestly: this is product work, not bench work.
- **Update (2026-05-15):** See item #27 (constellation revisit point). Revisit at 2026-05-29 or first substrate-measurement review.
- **Out of scope:** L3 cascade wiring + diagonal alignment
  matching (follows peak-pair as a separate item).

---

## Current state — honest accounting (2026-05-15)

### Active (7 components contributing to bench number)

FTS retrieval, description-enriched FTS (99.4% coverage on Henry's brain), TACT FTS fallback, signal score weighting, declarative density boost, recency decay, episode diversity. These are the core retrieval + re-ranking pipeline. 79.2% on LongMemEval-S, 0 defects.

### Accumulating (recognition substrate in production)

- **Retrieval events:** Streaming into `retrieval_events` via Permagent's enrichment layer on Henry's brain since 2026-05-15.
- **Ambient boost:** Active in production — `RecognitionContext` populated with focus wing and recent activity.
- **Auto-reinforce:** Running on each cascade retrieval (+0.01 signal_score per hit). Cumulative effect will emerge as retrievals accumulate.
- **Co-retrieval pairs:** Will populate once Permagent's scheduler calls `rebuild_co_retrieval_index()` (wrapper delegation shipped in PR #125).

### Tracked but not active (6 items with trigger conditions)

- Item #23: Canonicalizer migration — trigger: measure string-level matching sufficiency
- Item #24: Persona-as-ranking-signal — trigger: persona variation in retrieval events
- Item #25: signal_score activation timing — trigger: signal_score distribution divergence
- Item #26: Un-delegated wrapper methods — trigger: consumer hits a specific gap
- Item #27: Constellation revisit — trigger: 2026-05-29 or first substrate-measurement review
- Item #28: Substrate measurement framework — **owed work**: needs capture as standalone doc

### Explicitly deferred (with rationale)

- **Constellation / peak-pair fingerprinting (T3):** Can't measure on bench (confirmed by components audit). Revisit at substrate-measurement review (item #27).
- **Canonicalizer migration (#23):** String-level matching may be sufficient. Wait for evidence.
- **Persona-as-ranking-signal (#24):** No data yet to tune weighting. Wait for persona variation in events.
- **signal_score activation (#25):** Distribution hasn't diverged from initial values yet. Wait for auto-reinforce to cook.
- **L2 episode summaries (#12):** Duplicates description-enriched FTS with zero incremental value for documented failures.
- **Session signal (#11):** No failure case where it helps. Wait for production data or new failure pattern.
- **Candidate C (per-session context isolation):** Reconstructive pre-validation flipped 1/4. Below threshold. Structural floor.

### Owed on Permagent side

- **Scheduler wiring:** Call `rebuild_co_retrieval_index()` periodically (hourly or after N events) via the wrapper (PR #125).
- **Real usage time:** ~1-2 weeks of Henry's normal usage to accumulate meaningful substrate.
- **Measurement framework capture:** The 4-claim framework needs to be written as a standalone doc before the measurement review (item #28).

---

## How to use this backlog

When picking the next Spectral-CC task, scan Tier 1 top-to-bottom. If everything in Tier 1 is in flight or shipped, move to Tier 2. Tier 3 items are for "what's the next big architectural piece" conversations. Post-substrate tracked items (#23-#28) activate on their trigger conditions, not on priority order.

Before promoting an item to a PR draft, re-check the **Depends on** field. Items marked "Nothing" can ship anytime.

**Standard PR discipline (post-2026-05-11):** For any non-trivial bench-side or architecturally-significant PR, use the propose-then-implement pattern. CC writes a proposal doc first, opens as a draft PR, waits for review, implements only after approval. Caught design issues twice on PRs #84 and #86 that would have required rework otherwise.

---

## Source notes

Original backlog assembled 2026-05-11 from PRs #71-#79, the Spectral architecture audit, the Permagent integration audit, the cascade trilogy planning, and gbrain review.

Updated same day after PR #84, PR #85, bench checkpoint, failure analysis (`docs/internal/bench-failure-analysis-2026-05-11.md`), and PR #86 dispatch. Added items 17-20.

Reconciled 2026-05-14 to reflect six merged investigation docs (PRs #106, #107, #108, #109, #110, #112) and the actor-level interventions investigation. Items #11, #12, #19 updated with investigation outcomes. Item #21 marked shipped. Item #8 status updated with PR #104/#108 progress. Item #20 updated with revert. Strategic frame updated with actor-level ceiling finding and PR #99 correction (#8/#9 AMBIGUOUS).

Topology section added 2026-05-14: Track 2 — Topology (T1 Kuzu graph wire-or-retire, T2 design questions, T3 peak-pair fingerprinting), plus docs/internal/topology-lineage.md. Reflects neighbours status audit 2026-05-14. No existing items modified.

Reconciled 2026-05-14 (item #8 + T2): item #8 validation complete (+2.5pp isolated lift, PR #117), T2 design questions resolved (PR #116), T1 dependencies expanded to 4 preconditions, item #22 baseline updated to 77.5%, multi-session classification corrected (4 confirmed GENUINE_MISS), strategic frame updated — actor synthesis confirmed as binding constraint.

Reconciled 2026-05-15: Phase change from bench-tuning to recognition-substrate accumulation. 5 investigations closed (reconstructive read, dedup defect, components audit, FTS alignment, recall API contract). 3 wrapper PRs shipped (#123 annotate, #124 recall_cascade, #125 rebuild_co_retrieval_index). 6 new tracked items added (#23-#28: Canonicalizer migration, persona-as-ranking-signal, signal_score activation, un-delegated wrapper methods, constellation revisit, substrate measurement framework). Permagent enrichment live on Henry's brain. Strategic frame rewritten to reflect read-side closure and substrate accumulation phase. Candidate C moved to deferred indefinitely (1/4 reconstructive pre-validation, below threshold). "Current state — honest accounting" section added.
