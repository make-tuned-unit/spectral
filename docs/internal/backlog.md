# Spectral product backlog

Living document. Append-only at the top (newest first). Each item is a candidate, not a commitment. Promote to a PR draft when prioritized.

Format per item: title, source, effort estimate, dependencies, why it matters, explicit out-of-scope. Effort is rough Spectral-CC hours, not calendar time.

---

## 2026-05-11 update — bench checkpoint complete, three PRs shipped

Today's shipping landed three PRs and an empirical baseline. State of `main` is now `e9a80d8` (post-#85) plus PR #86 in flight (proposal approved, implementation underway).

### Closed items

**Item 1 — Synthesis prompt revisions.** Closed via PR #84. Shipped Changes A (preference-priority), B (concise answers), C (temporal calculation as new instruction 9 rather than replacing #6). Change D (judge rubrics) deferred to a separate PR pending judge calibration evidence. Bench attribution: single-session-preference moved 35% → 45% (+10pp), single-session-user 85% → 90% (+5pp), single-session-assistant 80% → 90% (+10pp). Confirmed +1-2pp overall projection landed.

**Item 3 — Bench checkpoint against current main.** Closed. Run at commit `e9a80d8`, K=40, cascade enabled, top-K=20 ingest, 24000 max context chars, 20 questions × 6 categories. Result: **73.3% overall (88/120)**, +7.5pp over May 1 baseline of 65.8%. Per-category breakdown locked in `docs/internal/bench-failure-analysis-2026-05-11.md`. New tag candidate: `baseline-2026-05-11-recognition-v1-cascade-k40` (uncalibrated until temporal regression addressed).

**Item 4 — Content-hash dedup in `remember_with`.** Closed via PR #85, broader than originally scoped. The PR became *non-destructive write semantics with content-hash dedup* — fixed the underlying destructive upsert behavior (signal_score, wing, source, etc. were being clobbered on same-key writes), added `WriteOutcome::{Inserted, NoOp, ContentUpdated}` return type, exposed `content_hash` column with idempotent migration, added `Brain::backfill_content_hashes()`. The original "remember_with creates duplicates" framing was wrong — the actual bug was destructive overwrite. Real fix removed a footgun every consumer had to defend against externally. Permagent confirmed they don't need their `(source, content_hash)` external guard after pin bump; will keep it as defense-in-depth.

### New Tier 1 items (queue immediately)

**Item 17 — Shape-routed actor strategies (PR #86 in flight).**
- **Source:** Bench failure analysis 2026-05-11.
- **Effort:** ~8-12h (in flight as PR #86)
- **Depends on:** Nothing.
- **Why it matters:** Existing `QuestionType` classifier in bench harness was doing meaningful retrieval-side routing but actor was generic. PR #86 extends classifier to 8 variants (sub-gates for recency, preference, recall), routes per-shape actor prompts via 8 markdown templates, lifts retrieval path from run-level to per-question (Temporal → topk_fts recovers the −15pp regression). Target lift: +10-15pp overall (73.3% → 80%+). When this lands, it becomes the foundational primitive for all future bench-side routing work.
- **Out of scope:** L2/co-retrieval/compiled-truth integration (separate items, plug in after this lands).

**Item 18 — Strategy effectiveness telemetry analysis per bench run.**
- **Source:** Bench failure analysis 2026-05-11; enables items #15 and #16.
- **Effort:** 2-3h
- **Depends on:** Item 17 (shape-routed strategies) — needs `strategy_telemetry` field populated on results.
- **Why it matters:** PR #86 ships per-question `strategy_telemetry` (chosen prompt template, retrieval path, classification path). This item builds the analysis script that consumes that telemetry to produce a per-strategy accuracy report per bench run. Becomes the input to backlog item #15 (per-PR ablation reporting) and item #16 (fail-improve loop on misclassifications). Without this, we ship #17 but can't measure which strategies earn their weight.
- **Out of scope:** Automated regression detection. This is reporting, not gating.

**Item 19 — Cascade `max_confidence=0.85` plateau investigation.** INVESTIGATED (PR #112). Bug confirmed: `.min(0.85)` clamp makes `max_confidence` non-functional. Telemetry-only — not an accuracy lever. Fix options: Option A (fix shim) or Option B (wire through orchestrator). See `docs/internal/cascade-max-confidence-investigation.md`.

### Strategic frame update

**Empirical baseline locked:** 73.3% overall on LongMemEval-S at K=40 cascade. Path to 90%+ exists but requires composing this baseline with 4-6 weeks of disciplined engineering: shape-routed actors (item 17), structural backlog items (item 2 co-retrieval ranking, item 8 compiled-truth boost, item 12 L2 episodes), plus multi-step actor patterns for the multi-session counting bottleneck. Per-step contribution stacks to +15-20pp; bench checkpoints between each PR keep attribution clean.

**Bottleneck identified (2026-05-11):** Actor synthesis, not retrieval. 75% of remaining failures show the actor receiving relevant memories but failing to count, synthesize, or apply preference signal correctly. **Update (2026-05-14):** Deeper investigation refined this picture. Multi-session failures decompose into DEFINITION_DISAGREEMENT (3, judge-side), GENUINE_MISS (2 confirmed + 2 AMBIGUOUS), RETRIEVAL_MISS (2), and TEMPORAL (1). Actor-level prompt interventions exhausted for GENUINE_MISS (3 interventions tried, all failed; structural output analysis confirms ceiling). Remaining path: item #8 bench validation (retrieval), item #21 telemetry resolving AMBIGUOUS cases, and judge refinement (item #20). See `docs/internal/actor-level-interventions-investigation.md`.

**Process discipline:** The propose-then-implement pattern (used on PR #84 and #86) caught design issues at proposal stage twice in a row that would have required rework if implementation had proceeded. Make this the standard for any non-trivial bench-side or architecturally-significant PR going forward.

### Items moved to Deferred indefinitely

- **Config-file-driven gate registry.** Considered for shape routing in PR #86; rejected. Inline regex in `retrieval.rs` is consistent with existing code style; config files add deployment complexity for no current benefit. Promote to file-based config only when the gate set exceeds ~15 entries.
- **LLM-side classifier for question shape.** Considered; rejected. Deterministic classification is the architectural commitment. Adding LLM in the classifier hot path inverts the "deterministic where possible" stance. Only revisit if deterministic patterns plateau materially below their ceiling.
- **`Brain::classify_question` Spectral primitive.** Considered for the routing work; rejected for now. Classifier lives in `spectral-bench-accuracy`. Promote to Spectral only if (a) Permagent or another consumer needs shape-aware routing in production, and (b) the bench's iteration has stabilized the shape vocabulary. Speculative generality otherwise.

---

## Strategic frame (read this first)

Current state on `main` at `f258e2d` (2026-05-14). Recognition architecture v1 shipped. Description-enriched FTS shipped (PR #104). Describe subcommand with Ollama support and structural template shipped (PR #108). Retrieval telemetry in bench reports shipped (PR #107 — resolves the AMBIGUOUS classification gap on next bench run). Shape-routed actor strategies shipped (PR #86). Preflight spot-check subcommand shipped (PR #110). Three investigations completed and deferred: item #11 (session signal), item #12 (L2 episodes), item #19 (cascade max_confidence — confirmed bug, telemetry-only).

**Actor-level intervention ceiling established (2026-05-14):** The actor-level interventions investigation (`docs/internal/actor-level-interventions-investigation.md`) concluded that the GENUINE_MISS ceiling is real for single-context-window actor approaches. Three prompt interventions tried (PRs #93, #97, #98) and two structural-output candidates analyzed — all face the same input-attention limitation. The only structurally different lever (per-session context isolation, Candidate C) is expensive and addresses at most the 2 AMBIGUOUS cases (#8, #9), contingent on item #21's telemetry resolving them as actor failures. Multi-session GENUINE_MISS cases #8/#9 are AMBIGUOUS (may be retrieval), not confirmed actor failures (PR #106 correction).

Spectral is making a deliberate bet: **deterministic recognition without LLM-in-loop**, closer to how a brain actually works, cheaper at inference, but architecturally distant from what Memanto and other RAG-style systems optimize for. The backlog reflects two parallel tracks:

- **Track A — Path B (recognition wins):** Use the recognition loop shipped in #71-#79 to make ranking better without LLM filtering. Co-retrieval signal in ranking (item 2), description-text in FTS (item 8 — foundation shipped, bench validation pending), AAAK context priming. Session-aware recency (item 11) deferred — no documented failure case. Every point is defended by deterministic mechanisms no other system has.
- **Track B — robustness and consumer DX:** Fix the footguns consumers have to defend against externally. Idempotency (closed via #85), health probes (item 5), backfill orchestration (item 6).
- **Track C — bench engineering:** Shape-routed strategies (item 17 — shipped), strategy telemetry (item 18), judge rubrics (item 20 — first attempt reverted, needs different approach). Actor-level prompt interventions exhausted for multi-session GENUINE_MISS; remaining path is retrieval improvement (item #8 bench validation) and accepting the multi-session floor where neither retrieval nor actor fixes apply.

All three tracks matter. Track A moves the differentiator; Track B reduces integration friction; Track C closes the bench number gap that funders, partners, and credibility analysts read first.

---

## Tier 1 — High leverage, queue first

### 2. Co-retrieval signal in cascade ranking (audit-P4 minimum behavioral)

- **Source:** PR #76 foundation laid; behavioral use deliberately deferred.
- **Effort:** 4–6h
- **Depends on:** PR #76 (merged), bench checkpoint (done — baseline 73.3%).
- **Why it matters:** First behavioral payoff of the recognition loop. Queries `co_retrieval_pairs` during ranking and boosts memories frequently co-retrieved with current top hits. The deterministic mechanism that makes Path B real. Estimated +0.5–2pp on LongMemEval. Composes with item 17 (shape-routed actors) — preference and general-recall strategies benefit most.
- **Out of scope:** Time-decay on co_count (item 14), per-session weighting (item 11).

### 5. `spectral doctor` CLI command  [gbrain idea #7]

- **Source:** garrytan/gbrain — `gbrain doctor`, `gbrain skillpack-check`, structured JSON output with `actions[]` array.
- **Effort:** 3–4h
- **Depends on:** Nothing.
- **Why it matters:** Health probe with structured output and exit codes (0/1/2 for CI gating). Checks: schema migrations up to date, embeddings present for all memories, `retrieval_events` not corrupted, `co_retrieval_pairs` index built and fresh, `content_hash` backfilled. Becomes a primitive Permagent, CI, and the bench harness all reuse. The "observable behavior" hard requirement.
- **Out of scope:** Auto-fixing (`--fix` flag) — print actions but don't execute in v1.

### 17. Shape-routed actor strategies *(PR #86 in flight)*

See "New Tier 1 items" above.

### 18. Strategy effectiveness telemetry analysis per bench run

See "New Tier 1 items" above.

### 19. Cascade `max_confidence=0.85` plateau investigation (INVESTIGATED — confirmed bug, not accuracy lever)

See "New Tier 1 items" above. **Investigation (2026-05-14, PR #112):** Confirmed as a real bug — `.min(0.85)` clamp in `brain.rs:1189-1192` makes `max_confidence` non-functional (166/166 runs produce identical 0.85). Orchestrator with correct computation is bypassed. Two fix options: Option A (fix the shim — minimal) or Option B (wire through orchestrator — structural). Fix is telemetry-only; does not affect retrieval accuracy. The Temporal→topk_fts routing decision was based on accuracy, not confidence, so fixing this doesn't change routing. Enables future confidence-driven routing. Full analysis: `docs/internal/cascade-max-confidence-investigation.md`.

---

## Tier 2 — Useful, queue after Tier 1

### 6. Backfill orchestration: `spectral backfill --all`

- **Source:** Captured during PR #76 review. Three backfill methods now exist: declarative density, co-retrieval index, and content_hash (added in #85). Still no canonical entry point.
- **Effort:** 1–2h
- **Depends on:** Nothing.
- **Why it matters:** Every Permagent deployment needs a single command to bring an old brain forward. Pairs well with item 5 (doctor) — doctor reports gap, backfill closes it. Item #85 added a third backfill method, so this is more useful than when originally captured.
- **Out of scope:** Scheduling/automation — that's the consumer's job.

### 7. Auto-rebuild co-retrieval index on schedule

- **Source:** PR #76 followup. Currently manual via `Brain::rebuild_co_retrieval_index`.
- **Effort:** ~2h
- **Depends on:** Nothing.
- **Why it matters:** As `retrieval_events` accumulates, `co_retrieval_pairs` drifts unless rebuilt. Either Permagent's Librarian schedules it or Spectral exposes a "rebuild if stale" primitive. Cleaner if Spectral owns the staleness check; consumers shouldn't need to reason about index freshness.
- **Out of scope:** Background tokio task — provide the primitive, let consumer decide cadence.

### 8. Compiled-truth boost in cascade ranking  [gbrain idea #2] — foundation SHIPPED, bench validation pending

- **Source:** garrytan/gbrain — "Compiled-truth boost (assessments outrank timeline noise)."
- **Effort:** 1–2h (original estimate). Foundation shipped; bench validation remaining.
- **Depends on:** Description-writing path populated (Librarian shipping in Permagent).
- **Why it matters:** Memories with `description.is_some()` rank higher than raw timeline events. Cheap, deterministic, measurable. The ranking lift compounds with description coverage: as Librarian writes more descriptions, retrieval quality improves automatically. Estimated +1–3pp once description coverage is meaningful. **Composes with item 17:** GeneralPreference strategy benefits most when descriptions exist.
- **Shipped so far:** (1) PR #104 — description-enriched FTS (descriptions indexed in BM25, vocabulary-gap bridging confirmed on 3/3 RETRIEVAL_MISS cases). Pre-validation: `docs/internal/item-8-prevalidation-vocabulary-gap.md`. (2) PR #108 — `describe` subcommand with Ollama/OpenAI-compatible API (`--api-format openai`), structural template prompt (0% hallucination on 35 tested memories vs 23% with prose prompt). Docs: `docs/internal/ollama-describe-compat.md`, `docs/internal/describe-structural-template-smoke.md`.
- **Remaining:** Bench validation — qwen2.5:7b description regen on full LongMemEval corpus, targeted bench run to measure actual lift on RETRIEVAL_MISS and AMBIGUOUS cases.
- **Out of scope:** Description quality scoring, freshness-weighted boost.

### 9. Filtered `list_undescribed` (by wing/age/source)

- **Source:** Permagent CC mentioned as future Spectral work in their integration report.
- **Effort:** ~2h
- **Depends on:** Nothing.
- **Why it matters:** Permagent's Librarian wants to describe memories selectively — newest first, only certain wings, etc. Currently `list_undescribed` returns the full bag. Adding filter params makes the Librarian's scheduling logic 10x simpler.
- **Out of scope:** Full query DSL — just the three filters Permagent has asked for.

### 10. `MemoryHit` carries `description` consistently

- **Source:** PR #75 review. `description` is on `Memory` but only on `MemoryHit` from certain code paths.
- **Effort:** 1h investigation, 1–2h fix.
- **Depends on:** Nothing.
- **Why it matters:** Consumers expect `MemoryHit.description` to be populated everywhere it's available. Today the ranking pipeline doesn't propagate it through every path. Trivial bug, easy to verify, no risk.
- **Out of scope:** Description propagation in non-`MemoryHit` types.

### 20. Judge rubric per-category revision (deferred Change D from PR #84) — first attempt REVERTED

- **Source:** Deferred from PR #84 to avoid attribution muddling.
- **Effort:** 2–3h
- **Depends on:** Bench checkpoint (done) and ideally item 17 shipping first (so we can attribute lift cleanly between actor changes and judge changes).
- **Why it matters:** 3 DEFINITION_DISAGREEMENT failures (#1 clothing, #2 projects, #6 citrus) are judge-side, not actor-side. Category-specific rubrics could distinguish defensible-different-count from genuine-miss.
- **First attempt (PR #102, reverted PR #103):** Reasoning-aware +-1 tolerance for counting questions. Zero lift on target cases — all 3 DEFINITION_DISAGREEMENT cases remained incorrect. The "deliberation" bar was too strict: the judge required explicit meta-reasoning, not just exhaustive evidence documentation. Post-mortem: `docs/internal/item-20-reasoning-aware-judge-proposal.md` Section 0. Needs a different approach — either two-call judge or structural signal detection.
- **Out of scope:** Multi-judge ensemble, model swapping for judge — keep Sonnet 4.5 as judge.

### ~~21. Retrieval telemetry in bench reports (`memory_keys` population)~~ — SHIPPED via PR #107

Shipped 2026-05-14. `retrieve_cascade()` and `retrieve_topk_fts()` now return raw `MemoryHit` vectors alongside formatted strings. `memory_keys` reliably populated for Cascade, TopkFts, and Tact paths. Graph path still falls back to string parsing. The next multi-session bench run with telemetry will resolve the AMBIGUOUS classification of cases #8 (tanks) and #9 (weddings) — determining whether these are actor failures or retrieval failures. This resolution gates whether actor-level Candidate C (per-session context isolation) is worth pre-validating. See `docs/internal/actor-level-interventions-investigation.md` Section 5.

### 22. Enable spectrograms in bench ingest

- **Source:** Deferred from PR #82 (split into preflight-only PR #110 + this item). Preflight subcommand shipped on main via PR #110.
- **Effort:** 1h code change + 1 bench run for attribution.
- **Depends on:** Item #8 bench validation complete (so spectrogram impact is measured independently, not confounded with description-enriched FTS).
- **Why it matters:** Spectrograms feed `signal_score` via the spectrogram analyzer, which feeds re-ranking. Enabling them changes bench behavior. Must run as an isolated experiment to measure impact on accuracy — don't bundle with other retrieval changes or attribution is confounded. PR #82's `enable_spectrogram: true` change to `ingest_question()` is the implementation; the preflight subcommand (now on main via PR #110) can verify coverage.
- **Out of scope:** Spectrogram-conditioned retrieval (that's further architectural work). This item is just flipping the flag and measuring.

---

## Tier 3 — Architectural / longer horizon

### 11. Audit-P4 full scope: session signal in ranking (DEFERRED — investigated 2026-05-13)

- **Source:** Audit P4. PR #79 shipped the data capture; behavioral use deferred.
- **Effort:** 6–8h
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
- **Effort:** 3–4h
- **Depends on:** Nothing — independent of L2 work.
- **Why it matters:** Targets complete-miss failures. Pure retrieval tuning, independent of recognition work. Estimated +1–2pp on LongMemEval. Complements item 12 rather than replacing it.
- **Out of scope:** LLM-mediated summary generation (that's Librarian's job).

### 14. Time-decay on `co_count`

- **Source:** PR #76 review, deferred deliberately.
- **Effort:** 2–3h once design is settled (longer with design).
- **Depends on:** Audit-P4 full scope (item 11).
- **Why it matters:** A pair retrieved 100 times last year vs 100 times this week look identical right now. For v1 "related memories" this is fine. As session-aware ranking lands, recency starts mattering more.
- **Out of scope:** Anything until item 11 ships.

### 15. Benchmarked ablation reporting per PR  [gbrain idea #3]

- **Source:** garrytan/gbrain — "v0.11→v0.12 moved P@5 from 22.1% → 49.1% on identical inputs."
- **Effort:** 2–3h to scaffold per-PR delta reporting in `spectral-bench-accuracy`.
- **Depends on:** Item 18 (strategy telemetry analysis) — the analysis script becomes input to this reporting layer.
- **Why it matters:** Per-PR attribution. "+X from co-retrieval boost, +Y from compiled-truth boost, +Z from L2 episodes." Tonight's bench established the discipline (per-category before/after, locked in `bench-failure-analysis-2026-05-11.md`). This item formalizes it for future PRs.
- **Out of scope:** Continuous benchmarking infrastructure — keep it manual and reproducible.

### 16. Fail-improve loop for deterministic classifiers  [gbrain idea #5]

- **Source:** garrytan/gbrain — "Deterministic classifiers improve over time via a fail-improve loop that logs every LLM fallback and generates better regex patterns from the failures."
- **Effort:** 4–6h.
- **Depends on:** Item 18 (strategy telemetry analysis) — provides the misclassification log.
- **Why it matters:** Generalizes the recall→recognition loop (#73) to classifier improvement. Every time a deterministic classifier produces "General" (the catch-all), log the input. Periodically mine the logs for missing rules. The pattern matters because "deterministic where possible, LLM as escape hatch" is a Spectral architectural commitment — making the deterministic layer self-improving turns the commitment into a moat.
- **Out of scope:** Auto-generating regex from logs (research project). Start with logging + manual review.

---

## Closed (since backlog established 2026-05-11)

- **Item 1 — Synthesis prompt revisions** → PR #84 shipped. Bench lift attributed.
- **Item 3 — Bench checkpoint** → 73.3% baseline locked at commit `e9a80d8`. Failure analysis in `docs/internal/bench-failure-analysis-2026-05-11.md`.
- **Item 4 — Content-hash dedup** → PR #85 shipped, broader scope than originally captured (non-destructive write semantics + WriteOutcome + content hash + backfill).
- **Item 21 — Retrieval telemetry** → PR #107 shipped 2026-05-14. `memory_keys` now populated for Cascade, TopkFts, and Tact paths. Next bench run resolves AMBIGUOUS cases #8/#9.

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
- **Depends on:** Item #8 bench validation complete — same
  attribution-confounding rule as item #22. Don't measure graph
  neighborhood lift while description-enriched FTS lift is
  un-isolated.
- **Why it matters:** Either this is an unused signal worth
  activating, or it's six-week-old dead code that should be named
  as such. The current ambiguous state is the worst option.
- **Out of scope:** Spectrogram-conditioned or peak-pair retrieval
  (see T3).

### T2. Topology design questions (Kuzu graph)

- **Source:** Neighbours status audit + brain-topology brief
  Section 3.3.
- **State:** Design decisions, not build work. The co-retrieval
  half is already resolved (session co-occurrence, symmetric,
  query-side limit, feeds re-ranking at weight 0.10 — shipped #90).
  These questions are open for the Kuzu graph only.
- **Effort:** Decision doc, no code. Cheap. NOT gated on item #8.
- **Open questions:**
  - **Degree cap.** The Kuzu neighborhood has no degree cap —
    bounded only by `max_hops`. If T1 wires it into the cascade,
    an uncapped graph drifts toward fully-connected and stops
    discriminating. Decide a cap before wiring, not after.
  - **Adjacency basis.** Entity-graph edges only today. Should it
    blend co-access or semantic similarity, or stay pure
    explicit-predicate?
  - **Relationship to co-retrieval pairs.** Two adjacency signals
    now exist. Are they complementary or redundant? T1's
    investigation should answer this.
- **Why it matters:** Resolving these is the precondition for T1
  being a real wiring task instead of a guess.

### T3. Peak-pair fingerprinting (recognition layer)

- **Source:** TACT constellation whitepaper; cascade trilogy
  planning. Formerly tracked informally as "PR 2."
- **State:** Not built. The current `find_resonant()` in
  spectral-spectrogram does dimensional similarity matching
  (nearest-neighbor in feature space), not Shazam-style
  combinatorial peak-pair hashing.
- **Effort:** 1–2 weeks. Largest item in this section.
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
- **Out of scope:** L3 cascade wiring + diagonal alignment
  matching (follows peak-pair as a separate item).

---

## How to use this backlog

When picking the next Spectral-CC task, scan Tier 1 top-to-bottom. If everything in Tier 1 is in flight or shipped, move to Tier 2. Tier 3 items are for "what's the next big architectural piece" conversations.

Before promoting an item to a PR draft, re-check the **Depends on** field. Items marked "Nothing" can ship anytime.

**Standard PR discipline (post-2026-05-11):** For any non-trivial bench-side or architecturally-significant PR, use the propose-then-implement pattern. CC writes a proposal doc first, opens as a draft PR, waits for review, implements only after approval. Caught design issues twice on PRs #84 and #86 that would have required rework otherwise.

---

## Source notes

Original backlog assembled 2026-05-11 from PRs #71-#79, the Spectral architecture audit, the Permagent integration audit, the cascade trilogy planning, and gbrain review.

Updated same day after PR #84, PR #85, bench checkpoint, failure analysis (`docs/internal/bench-failure-analysis-2026-05-11.md`), and PR #86 dispatch. Added items 17-20.

Reconciled 2026-05-14 to reflect six merged investigation docs (PRs #106, #107, #108, #109, #110, #112) and the actor-level interventions investigation. Items #11, #12, #19 updated with investigation outcomes. Item #21 marked shipped. Item #8 status updated with PR #104/#108 progress. Item #20 updated with revert. Strategic frame updated with actor-level ceiling finding and PR #99 correction (#8/#9 AMBIGUOUS).

Topology section added 2026-05-14: Track 2 — Topology (T1 Kuzu graph wire-or-retire, T2 design questions, T3 peak-pair fingerprinting), plus docs/internal/topology-lineage.md. Reflects neighbours status audit 2026-05-14. No existing items modified.
