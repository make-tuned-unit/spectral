# Spectral product backlog

Living document. Append-only at the top (newest first). Each item is a candidate, not a commitment. Promote to a PR draft when prioritized.

Format per item: title, source, effort estimate, dependencies, why it matters, explicit out-of-scope. Effort is rough Spectral-CC hours, not calendar time.

---

## Strategic frame (read this first)

Current state on `main` at `5b9c457`. Recognition architecture v1 is shipped: every retrieval is logged, events are mined for co-occurrence, events are attributable to sessions, descriptions can be written and read on memories. Permagent's Librarian — and any future consumer — has the surface they need. What's NOT yet shipped: behavioral changes to ranking that *use* the captured signal.

Spectral is making a deliberate bet: **deterministic recognition without LLM-in-loop**, closer to how a brain actually works, cheaper at inference, but architecturally distant from what Memanto and other RAG-style systems optimize for. The backlog below reflects two parallel tracks:

- **Track A — Path B (recognition wins):** Use the recognition loop we just shipped to make ranking better without LLM filtering. Co-retrieval signal in ranking, session-aware recency, description-text in FTS, AAAK context priming. Bench outcome uncertain but every point is defended by deterministic mechanisms no other system has.
- **Track B — robustness and consumer DX:** Fix the footguns Permagent (and future consumers) currently have to defend against externally. Idempotency, health probes, backfill orchestration.

Both tracks matter. Track A moves the differentiator; Track B reduces the cost of every future integration.

---

## Tier 1 — High leverage, queue first

### 1. Synthesis prompt revisions

- **Source:** Drafted yesterday in bench thread, never shipped.
- **Effort:** 1–2h
- **Depends on:** Nothing.
- **Why it matters:** Cheapest, highest-confidence retrieval lift on LongMemEval. Pure prompt engineering, no architectural risk. Estimated +1–2pp on LongMemEval-S. Worth landing before any behavioral cascade change so we have a clean baseline.
- **Out of scope:** Cascade changes, ranking changes, schema changes.

### 2. Co-retrieval signal in cascade ranking (audit-P4 minimum behavioral)

- **Source:** PR #76 foundation laid; behavioral use deliberately deferred.
- **Effort:** 4–6h
- **Depends on:** PR #76 (merged), bench checkpoint as baseline.
- **Why it matters:** First behavioral payoff of the recognition loop. Queries `co_retrieval_pairs` during ranking and boosts memories frequently co-retrieved with current top hits. The deterministic mechanism that makes Path B real. Estimated +0.5–2pp on LongMemEval.
- **Out of scope:** Time-decay on co_count (separate item), per-session weighting (audit-P4 full scope).

### 3. Bench checkpoint against current main

- **Source:** Deferred explicitly post-#79 merge.
- **Effort:** ~15 min wall time, ~$40, no Spectral-CC work — dispatch only.
- **Depends on:** Nothing. Optionally wait for items 1+2 to land first if we want one bench run instead of three.
- **Why it matters:** Locks in the v1 baseline for recognition architecture. All future behavioral work compares against this number. Audit projected 83–87% post-loop-closure; needs empirical confirmation before building on top. Catches regressions early.
- **Out of scope:** Tuning. The point is measuring, not improving.

### 4. Content-hash dedup in `remember_with`  [gbrain idea #1]

- **Source:** garrytan/gbrain — "Idempotent imports (content-hash dedup)."
- **Effort:** 1–2h
- **Depends on:** Nothing.
- **Why it matters:** Direct answer to bite #4 in the Permagent integration audit: `remember_with` is not idempotent today. Same key called twice creates two memories. Permagent has to defend against this externally; every future consumer will too. Right fix is inside Spectral: hash content, check on insert, dedup. Removes a footgun the API currently exposes.
- **Out of scope:** Updating embeddings on content-hash-change (separate concern; current behavior is correct: new content → new memory → new embedding).

### 5. `spectral doctor` CLI command  [gbrain idea #7]

- **Source:** garrytan/gbrain — `gbrain doctor`, `gbrain skillpack-check`, structured JSON output with `actions[]` array.
- **Effort:** 3–4h
- **Depends on:** Nothing.
- **Why it matters:** Health probe with structured output and exit codes (0/1/2 for CI gating). Checks: schema migrations up to date, embeddings present for all memories, `retrieval_events` not corrupted, `co_retrieval_pairs` index built and fresh. Becomes a primitive Permagent, CI, and the bench harness all reuse. The "observable behavior" hard requirement from the user-preferences document.
- **Out of scope:** Auto-fixing (`--fix` flag) — print actions but don't execute in v1.

---

## Tier 2 — Useful, queue after Tier 1

### 6. Backfill orchestration: `spectral backfill --all`

- **Source:** Captured during PR #76 review. Two backfill methods exist (declarative density, co-retrieval index) but no canonical entry point.
- **Effort:** 1–2h
- **Depends on:** Nothing.
- **Why it matters:** Henry's brain and every Permagent deployment needs a single command to bring an old brain forward. Without it, each consumer has to know which backfills exist and what order to run them. Pairs well with `spectral doctor` (doctor reports gap, backfill closes it).
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
- **Depends on:** Description-writing path populated (Librarian shipping).
- **Why it matters:** Memories with `description.is_some()` rank higher than raw timeline events. This is exactly what the `description` field exists for. Cheap, deterministic, measurable. The ranking lift compounds with description coverage: as Librarian writes more descriptions, retrieval quality improves automatically. Estimated +1–3pp once description coverage is meaningful.
- **Out of scope:** Description quality scoring, freshness-weighted boost (separate items).

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

---

## Tier 3 — Architectural / longer horizon

### 11. Audit-P4 full scope: session signal in ranking

- **Source:** Audit P4. PR #79 shipped the data capture; behavioral use deferred.
- **Effort:** 6–8h
- **Depends on:** Bench checkpoint (item 3), some usage data accumulated in `retrieval_events`.
- **Why it matters:** Closes the ambient state loop. Recency-weighted ranking via active session. The biggest architectural change still pending. Deferred deliberately until we have real usage data to inform what session signal *should* weight — guessing now means redesigning later.
- **Out of scope:** Multi-device session reconciliation, session lifecycle management.

### 12. L2 cascade layer: episode summaries

- **Source:** TACT whitepaper, queued as PR 2 of the cascade trilogy.
- **Effort:** 1 week (single largest item in the backlog).
- **Depends on:** Episode data model (partially exists), consumer-provided `episode_id` API (RememberOpts already has it).
- **Why it matters:** The layer between AAAK (L1) and constellation/TACT (L3). Targets multi-session synthesis (currently the weakest LongMemEval category at 40%). When L1 has no foundational fact and the query is about a coherent past activity, L2 should surface the *episode* — its constituent memories grouped together — rather than fragmenting into turn-level recall at L3.
- **Out of scope:** L4 vector layer (deferred indefinitely per recognition-not-retrieval architecture), L0 filesystem layer (PR 3 of cascade trilogy).

### 13. Per-session summaries dual-index retrieval (audit Proposal 5)

- **Source:** Spectral architecture audit, Proposal 5.
- **Effort:** 3–4h
- **Depends on:** Nothing — independent of L2 work.
- **Why it matters:** Targets complete-miss failures. Pure retrieval tuning, independent of recognition work. Estimated +1–2pp on LongMemEval.
- **Out of scope:** LLM-mediated summary generation (that's Librarian's job).

### 14. Time-decay on `co_count`

- **Source:** PR #76 review, deferred deliberately.
- **Effort:** 2–3h once design is settled (longer with design).
- **Depends on:** Audit-P4 full scope (item 11) — the natural place for recency-aware co-retrieval scoring.
- **Why it matters:** A pair retrieved 100 times last year vs 100 times this week look identical right now. For v1 "related memories" this is fine — co-retrieval is a stable historical signal. As session-aware ranking lands, recency starts mattering more. Design questions (decay function, weighted window) need real usage data first.
- **Out of scope:** Anything until item 11 ships.

### 15. Benchmarked ablation reporting per PR  [gbrain idea #3]

- **Source:** garrytan/gbrain — "v0.11→v0.12 moved P@5 from 22.1% → 49.1% on identical inputs."
- **Effort:** 2–3h to scaffold per-PR delta reporting in `spectral-bench-accuracy`.
- **Depends on:** Bench checkpoint (item 3) for the baseline.
- **Why it matters:** Spectral already has the bench harness. What's missing is the *discipline* of reporting deltas per PR and isolating which layer earned the gain. gbrain explicitly attributes "+28.8 F1 to typed-link extract quality." Spectral should be able to say "+X from co-retrieval boost, +Y from description boost, +Z from L2 episode layer." Otherwise we can't tell which audit work is actually paying off.
- **Out of scope:** Continuous benchmarking infrastructure — keep it manual and reproducible.

### 16. Fail-improve loop for deterministic classifiers  [gbrain idea #5]

- **Source:** garrytan/gbrain — "Deterministic classifiers improve over time via a fail-improve loop that logs every LLM fallback and generates better regex patterns from the failures."
- **Effort:** 4–6h.
- **Depends on:** A deterministic classifier in Spectral that has a meaningful LLM fallback path (declarative density signal is the candidate today; question-type routing is another).
- **Why it matters:** Generalizes the recall→recognition loop (#73) to classifier improvement. Every time a deterministic classifier escalates to LLM, log the input. Periodically mine the logs for missing rules. The pattern matters because "deterministic where possible, LLM as escape hatch" is a Spectral architectural commitment — making the deterministic layer self-improving turns the commitment into a moat.
- **Out of scope:** Auto-generating regex from logs (that's a research project). Start with logging + manual review.

---

## Deferred indefinitely (here for visibility, not action)

These were considered and consciously kept off the roadmap. Listed so future "should we add X?" conversations can short-circuit.

- **L4 vector layer in cascade.** Deferred per recognition-not-retrieval architectural vision. Adding semantic search to FTS *is* what everyone else does; making it the primary path inverts the cascade.
- **Auto-running the Librarian on a schedule.** Permagent's task scheduler is the right place. Spectral exposes the primitives; Permagent decides when to run them.
- **Re-description / refresh logic for stale descriptions.** Future work. Descriptions get re-evaluated when signal_score, hits, or related-memory set has drifted materially. Needs usage data first.
- **Wing/method filtering on co-retrieval queries.** Related memories regardless of how they were retrieved. Filtering would re-introduce path-dependence the index is designed to abstract away.
- **Background tokio tasks inside Spectral.** Consumers (Permagent, future deployments) manage their own runtimes. Spectral exposes synchronous primitives; consumers schedule them.

---

## How to use this backlog

When picking the next Spectral-CC task, scan Tier 1 top-to-bottom. If everything in Tier 1 is in flight or shipped, move to Tier 2. Tier 3 items are for "what's the next big architectural piece" conversations, not "what do we ship this week."

Before promoting an item to a PR draft, re-check the **Depends on** field — some items depend on others being in flight, and the dependency graph isn't always linear. Items marked "Nothing" can ship anytime.

Bench checkpoint (item 3) is the gating event for several Tier 2/3 items. Don't defer it indefinitely; the recognition loop architecture deserves a real baseline number before further behavioral work lands.

---

## Source notes

This backlog was assembled on 2026-05-11 from:

- Spectral chat threads covering PRs #71–#79 (recognition architecture v1)
- The Spectral architecture audit (`spectral-architecture-audit.md`)
- The cascade trilogy planning (TACT whitepaper alignment)
- The Permagent integration audit (`permagent-integration-audit-2026-05-11.md`)
- Bench thread context (LongMemEval baseline at 65.8% top-K=20)
- Review of `garrytan/gbrain` (top 5 transferable ideas integrated as items 4, 5, 8, 15, 16)

Five gbrain ideas integrated. Three others considered and excluded: "fat skills architecture" (Spectral is a library, not an agent harness), "Postgres/PGLite backend" (SQLite fits embedded use case), "typed-link entity graph" (different model from Spectral's behavioral co-retrieval graph — both valid, doing different jobs).
