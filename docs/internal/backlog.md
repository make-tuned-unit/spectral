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

**Item 19 — Cascade `max_confidence=0.85` plateau investigation.**
- **Source:** Bench failure analysis 2026-05-11. All 6 temporal failures showed identical `max_confidence=0.85` and `stopped_at=None`.
- **Effort:** 4-6h investigation + 2-4h likely fix.
- **Depends on:** Nothing.
- **Why it matters:** Either a hardcoded ceiling or a real cascade confidence-scoring bug. If real, fixing it could lift cascade-on-temporal from 70% back toward 85% without needing the topk_fts routing escape hatch (which PR #86 puts in as the fast fix). The uniformity is suspicious; the kind of pattern that often hides a real bug. Worth investigating regardless of bench impact.
- **Out of scope:** Refactoring cascade confidence semantics — investigate scope first.

### Strategic frame update

**Empirical baseline locked:** 73.3% overall on LongMemEval-S at K=40 cascade. Path to 90%+ exists but requires composing this baseline with 4-6 weeks of disciplined engineering: shape-routed actors (item 17), structural backlog items (item 2 co-retrieval ranking, item 8 compiled-truth boost, item 12 L2 episodes), plus multi-step actor patterns for the multi-session counting bottleneck. Per-step contribution stacks to +15-20pp; bench checkpoints between each PR keep attribution clean.

**Bottleneck identified:** Actor synthesis, not retrieval. 75% of remaining failures show the actor receiving relevant memories but failing to count, synthesize, or apply preference signal correctly. Retrieval is doing more work than the prompt-eng and counting-discipline of the actor can keep up with. This recontextualizes future Spectral-side work: ranking changes (items 2, 8) matter for the 15% of failures that ARE retrieval-bound; actor-side work in the bench harness (items 17, 18) addresses the 75% that aren't.

**Process discipline:** The propose-then-implement pattern (used on PR #84 and #86) caught design issues at proposal stage twice in a row that would have required rework if implementation had proceeded. Make this the standard for any non-trivial bench-side or architecturally-significant PR going forward.

### Items moved to Deferred indefinitely

- **Config-file-driven gate registry.** Considered for shape routing in PR #86; rejected. Inline regex in `retrieval.rs` is consistent with existing code style; config files add deployment complexity for no current benefit. Promote to file-based config only when the gate set exceeds ~15 entries.
- **LLM-side classifier for question shape.** Considered; rejected. Deterministic classification is the architectural commitment. Adding LLM in the classifier hot path inverts the "deterministic where possible" stance. Only revisit if deterministic patterns plateau materially below their ceiling.
- **`Brain::classify_question` Spectral primitive.** Considered for the routing work; rejected for now. Classifier lives in `spectral-bench-accuracy`. Promote to Spectral only if (a) Permagent or another consumer needs shape-aware routing in production, and (b) the bench's iteration has stabilized the shape vocabulary. Speculative generality otherwise.

---

## Strategic frame (read this first)

Current state on `main` at `e9a80d8`. Recognition architecture v1 is shipped: every retrieval is logged, events are mined for co-occurrence, events are attributable to sessions, descriptions can be written and read on memories. Non-destructive write semantics shipped via PR #85. Synthesis prompt revisions shipped via PR #84. Empirical bench baseline established at 73.3% on LongMemEval-S (commit `e9a80d8`, K=40 cascade). Shape-routed actor strategies in flight as PR #86; on landing, expected +10-15pp lift breaks the 80% mark.

Spectral is making a deliberate bet: **deterministic recognition without LLM-in-loop**, closer to how a brain actually works, cheaper at inference, but architecturally distant from what Memanto and other RAG-style systems optimize for. The backlog reflects two parallel tracks:

- **Track A — Path B (recognition wins):** Use the recognition loop shipped in #71-#79 to make ranking better without LLM filtering. Co-retrieval signal in ranking (item 2), session-aware recency (item 11), description-text in FTS (item 8), AAAK context priming. Every point is defended by deterministic mechanisms no other system has.
- **Track B — robustness and consumer DX:** Fix the footguns consumers have to defend against externally. Idempotency (closed via #85), health probes (item 5), backfill orchestration (item 6).
- **Track C — bench engineering (new track post 2026-05-11):** Actor synthesis is the limiting factor for ~75% of bench failures. Shape-routed strategies (item 17), strategy telemetry (item 18), and category-specific judge rubrics (deferred Change D from PR #84) materially move the bench number independent of retrieval-side work.

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

### 19. Cascade `max_confidence=0.85` plateau investigation

See "New Tier 1 items" above.

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

### 8. Compiled-truth boost in cascade ranking  [gbrain idea #2]

- **Source:** garrytan/gbrain — "Compiled-truth boost (assessments outrank timeline noise)."
- **Effort:** 1–2h
- **Depends on:** Description-writing path populated (Librarian shipping in Permagent).
- **Why it matters:** Memories with `description.is_some()` rank higher than raw timeline events. Cheap, deterministic, measurable. The ranking lift compounds with description coverage: as Librarian writes more descriptions, retrieval quality improves automatically. Estimated +1–3pp once description coverage is meaningful. **Composes with item 17:** GeneralPreference strategy benefits most when descriptions exist.
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

### 20. Judge rubric per-category revision (deferred Change D from PR #84)

- **Source:** Deferred from PR #84 to avoid attribution muddling.
- **Effort:** 2–3h
- **Depends on:** Bench checkpoint (done) and ideally item 17 shipping first (so we can attribute lift cleanly between actor changes and judge changes).
- **Why it matters:** Single-session-preference is at 45%; some failures are arguably correct under a different rubric. Category-specific rubrics distinguish "preference match" from "factual match," reducing judge-side false negatives. Estimated +0.5-1pp overall, concentrated in single-session-preference.
- **Out of scope:** Multi-judge ensemble, model swapping for judge — keep Sonnet 4.5 as judge.

### 21. Retrieval telemetry in bench reports (`memory_keys` population)

- **Source:** Item #11 investigation (2026-05-13). `memory_keys` is empty for all results in the post-PR-#98 bench run. PR #99's failure classification was based on actor output inference, not retrieval telemetry. Cases #8 and #9 were reclassified to AMBIGUOUS as a result.
- **Effort:** 2-3h
- **Depends on:** Nothing — the `memory_keys` field already exists on `QuestionResult` in `report.rs`, and `SingleResult` in `eval.rs` computes `memory_keys` at `eval.rs:338-358`. The gap is that cascade retrieval via `format_hits_grouped` returns formatted strings, not `MemoryHit` structs, so key extraction falls back to string parsing which may silently produce empty results.
- **Why it matters:** Evidence-based failure classification. Without per-question retrieval data (which memories were retrieved, at what rank, with what score), every failure analysis requires reasoning backward from actor output. This is fragile — it produced an incorrect classification for 2 of 10 multi-session failures. Future bench runs should produce attributable retrieval telemetry so classification is mechanical, not inferential.
- **Out of scope:** Per-memory score records in main reports (the `--dump-scores` flag already handles this for detailed analysis). This item is about making `memory_keys` reliably populated in the standard report.

---

## Tier 3 — Architectural / longer horizon

### 11. Audit-P4 full scope: session signal in ranking (DEFERRED — investigated 2026-05-13)

- **Source:** Audit P4. PR #79 shipped the data capture; behavioral use deferred.
- **Effort:** 6–8h
- **Depends on:** Bench checkpoint (done), some usage data accumulated in `retrieval_events`.
- **Why it matters:** Closes the ambient state loop. Recency-weighted ranking via active session. The biggest architectural change still pending. Deferred deliberately until we have real usage data to inform what session signal *should* weight — guessing now means redesigning later. **Composes with item 17:** GeneralRecall strategy benefits most from session-priority retrieval.
- **Investigation (2026-05-13):** Evaluated 5 candidate session signals against documented failures. None addresses existing failure cases — GENUINE_MISS is actor-level (embedded references), not ranking-level. Existing per-memory signals subsume session-level approximations. Candidate topic-density is actively counterproductive. Defer until: (1) a failure is found where retrieved sessions rank below K cutoff, (2) Permagent production usage data, (3) item #12 creates session-level metadata, or (4) actor improvements shift bottleneck to ranking. Full analysis: `docs/internal/item-11-investigation.md`.
- **Out of scope:** Multi-device session reconciliation, session lifecycle management.

### 12. L2 cascade layer: episode summaries

- **Source:** TACT whitepaper, queued as PR 2 of the cascade trilogy.
- **Effort:** 1 week (single largest item in the backlog).
- **Depends on:** Episode data model (partially exists), consumer-provided `episode_id` API (RememberOpts already has it).
- **Why it matters:** The layer between AAAK (L1) and constellation/TACT (L3). **Bench analysis confirmed this is the highest-leverage individual item:** multi-session counting (9 failures, biggest absolute failure count) needs L2 episode-grouped retrieval to let the actor scan over discrete session blocks rather than 80+ interleaved hits. Composes with item 17's `counting_enumerate` strategy — they become genuinely powerful together. Estimated +3-4pp overall.
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

## How to use this backlog

When picking the next Spectral-CC task, scan Tier 1 top-to-bottom. If everything in Tier 1 is in flight or shipped, move to Tier 2. Tier 3 items are for "what's the next big architectural piece" conversations.

Before promoting an item to a PR draft, re-check the **Depends on** field. Items marked "Nothing" can ship anytime.

**Standard PR discipline (post-2026-05-11):** For any non-trivial bench-side or architecturally-significant PR, use the propose-then-implement pattern. CC writes a proposal doc first, opens as a draft PR, waits for review, implements only after approval. Caught design issues twice on PRs #84 and #86 that would have required rework otherwise.

---

## Source notes

Original backlog assembled 2026-05-11 from PRs #71-#79, the Spectral architecture audit, the Permagent integration audit, the cascade trilogy planning, and gbrain review.

Updated same day after PR #84, PR #85, bench checkpoint, failure analysis (`docs/internal/bench-failure-analysis-2026-05-11.md`), and PR #86 dispatch. Added items 17-20.
