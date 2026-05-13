# Item #11 Investigation: Session-Level Signal in Ranking

**Date**: 2026-05-13
**Branch**: `investigate/item-11-session-signal`
**Status**: Investigation complete. Recommendation: **defer**.

---

## Section 1 — Current Ranking Pipeline Inventory

The unified re-ranking pipeline is `apply_reranking_pipeline()` at `ranking.rs:282`. It receives FTS candidates (ordered by BM25 rank) and applies signals compositely before a single sort.

| # | Signal | File:Line | Type | Description |
|---|--------|-----------|------|-------------|
| 1 | FTS rank position | `ranking.rs:294` | Base score | `1.0 - (position / n)` — normalized rank from BM25 |
| 2 | Signal score blending | `ranking.rs:299-303` | Additive | `(1-w)*fts_rank + w*signal_score`, default w=0.3 |
| 3 | Ambient boost | `ranking.rs:307-311` | Multiplicative | Wing alignment + recency via `ambient_boost_for_hit()` at `cascade_layers.rs:17` |
| 4 | Declarative density | `ranking.rs:317-325` | Additive | `+w*density` where density is ratio of first-person declarative sentences, default w=0.10 |
| 5 | Co-retrieval boost | `ranking.rs:328-335` | Additive | `+w*affinity` from pre-computed `co_retrieval_pairs`, default w=0.10 |
| 6 | Recency decay | `ranking.rs:338-351` | Multiplicative | `0.5^(age_days / half_life)`, default half_life=365 days |
| 7 | Entity boost | `ranking.rs:354-375` | Additive | Top member of each wing cluster gets +0.05 |
| 8 | Episode diversity | `cascade_layers.rs:66-98` | Post-rank reorder | Cap per-episode memories to `max_per_episode`, push overflow to tail |
| 9 | Context chain dedup | `ranking.rs:152-187` | Post-rank filter | Collapse near-duplicate `[Memory context]` references |

`RerankingConfig` at `ranking.rs:238-272` controls which signals are active. The cascade path (`cascade_layers.rs:160-175`) enables ambient boost, declarative density, episode diversity, and context dedup. The topk_fts path (`brain.rs:1134-1148`) enables signal score, recency, entity boost, and context dedup.

**What exists for sessions today**: `MemoryHit.episode_id` (`lib.rs:171`) carries the session/episode identifier. Episode diversity (`cascade_layers.rs:66`) uses it for post-rank reordering. No signal currently computes a session-level aggregate (average density, session memory count, etc.) and uses it to boost or penalize individual memories.

---

## Section 2 — Candidate Session-Level Signals

### Candidate A: Session Memory Count

**What it would measure**: Sessions with more memories about a topic rank higher than sessions with single mentions.

**Computation**: For each memory in the candidate set, count how many other candidates share the same `episode_id`. Boost memories from sessions with more representation in the FTS results.

**Failure mode addressed**: None documented. In all GENUINE_MISS cases (#3, #7, #8, #9), answer sessions had 3/3 or 4/4 turns retrieved. The issue is actor attention on retrieved content, not session count in results. In RETRIEVAL_MISS cases (#4, #10), sessions had 0 turns retrieved — session count doesn't help if no turns surface.

**Complexity**: Small — loop over candidates, count per episode, additive boost.

**Risk**: Could penalize single-turn sessions that happen to contain critical information. E.g., a one-turn session where the user states a key fact ("I moved to Denver") would rank below a verbose session that mentions Denver incidentally across 5 turns.

### Candidate B: Session Recency

**What it would measure**: Most recent session about X outranks older sessions about X.

**Computation**: For each memory, use the session's `created_at` (earliest or latest turn timestamp) as a recency anchor. Boost memories from more recent sessions.

**Failure mode addressed**: Potentially knowledge-update questions where the latest state matters. But the existing per-memory recency decay (signal #6) already does this at memory granularity. Session-level recency would smooth over turn-level timestamp variation within a session, which is negligible for LongMemEval (sessions are ingested with a single date).

**Complexity**: Small — group by episode, compute session date, apply boost.

**Risk**: Duplicates existing recency signal. The per-memory recency decay already handles this, since all turns in a session share the same `created_at`. Adding a session-level recency signal on top would double-count.

### Candidate C: Session Declarative Density

**What it would measure**: Average declarative density across a session's memories. Sessions where the user made many personal statements rank higher.

**Computation**: Group candidates by `episode_id`, average their `declarative_density`, boost memories from high-density sessions.

**Failure mode addressed**: Potentially cases where answer sessions are user-heavy (lots of personal statements) but compete with assistant-heavy sessions. However, individual memory-level declarative density (signal #4) already boosts user turns over assistant turns. Averaging at session level would smooth this, potentially boosting assistant turns in high-density sessions they don't deserve.

**Complexity**: Small — aggregate density per episode, additive boost.

**Risk**: Could boost irrelevant assistant turns just because they're in a session with many user declarations. The per-memory signal is already more precise.

### Candidate D: Session Co-Retrieval Coherence

**What it would measure**: How often this session's memories are co-retrieved together as a cluster.

**Computation**: For each session in the candidate set, query `co_retrieval_pairs` for intra-session pairs. Sessions whose memories frequently co-occur in retrievals are coherent — they're about a consistent topic. Boost coherent sessions.

**Failure mode addressed**: None clearly documented. Coherent sessions might indicate answer-bearing sessions for multi-session questions, but this is speculative. The existing co-retrieval boost (signal #5) already uses inter-memory affinity.

**Complexity**: Medium — requires multiple `related_memories` queries per session, aggregation logic, new normalization.

**Risk**: Self-reinforcing feedback loop. Sessions that were retrieved together before get boosted more, making them more likely to be retrieved together in the future. Could lock in early retrieval patterns.

### Candidate E: Session Topic Density (Count of Unique Query-Term Hits)

**What it would measure**: How many of the session's total memories match the query terms. Sessions where the topic pervades multiple turns rank higher than sessions with a single passing mention.

**Computation**: Count how many memories from each session appear in the FTS candidate set (as a fraction of total session memories). Boost memories from sessions with higher topic density.

**Failure mode addressed**: Potentially cases where an answer session mentions the topic in multiple turns (high density) but competes with sessions that mention it once. However, looking at the GENUINE_MISS cases: in #9 (weddings), the wedding planning sessions DO mention weddings in multiple turns — they just mention the user's OWN wedding (primary topic) while the ATTENDED weddings are embedded references. High topic density would actually HURT here, because the wedding-planning sessions would get boosted, pushing attended-wedding mentions lower.

**Complexity**: Small — count per-episode candidate fraction.

**Risk**: Actively counterproductive for the embedded-reference failure mode. Sessions with high topic density are typically about the PRIMARY topic, while the answer to a counting question often comes from embedded references in sessions about DIFFERENT primary topics.

---

## Section 3 — Recommendation: Defer

**None of the five candidates clearly addresses documented failure cases.**

The critical evidence:

1. **GENUINE_MISS is the largest single failure mode** (4 of 10 multi-session failures), with DEFINITION_DISAGREEMENT (3) and RETRIEVAL_MISS (2) making up the rest. Verification of retrieval status for the 4 GENUINE_MISS cases:

   **Verification methodology**: PR #99's failure table claims "3/3 retrieved" for cases #7, #8, #9, citing cross-referencing of `memory_keys` from `report.json` against answer session IDs. However, `memory_keys` is **empty for all results** in the post-PR-#98 bench run — the field was not populated. PR #99's retrieval status was inferred from actor output, not retrieval telemetry.

   Independent verification from actor output:
   - **#3 Bike expenses**: Actor quotes bike lights ($40) from `answer_2880eb6c_2`. Misses $25 chain from the SAME turn. 4/4 answer sessions confirmed in actor context — actor attention miss, not ranking.
   - **#7 Festivals**: Actor explicitly references `answer_cf9e3940_1`, `answer_cf9e3940_2`, `answer_cf9e3940_3` by session ID. 3/3 confirmed retrieved. Actor found 3 festivals, missed the 4th. GENUINE_MISS confirmed.
   - **#8 Tanks**: Actor found "Amazonia" (from `answer_c65042d7_3`) and "1-gallon tank for friend's kid" (from `answer_c65042d7_1`). But the missed 5-gallon betta tank from `answer_c65042d7_2` has **no evidence of retrieval** — neither the session ID nor any unique content from that session appears in actor output. PR #99 classified this as GENUINE_MISS, stating the betta tank is "in the first turn of the session," but retrieval of that session is not confirmed. **This case is ambiguous: could be partial RETRIEVAL_MISS.**
   - **#9 Weddings**: Actor explicitly references `answer_e7b0637e_1` and quotes from non-answer sessions. Emily+Sarah (`answer_e7b0637e_2`) and Jen+Tom (`answer_e7b0637e_3`) have **no evidence of retrieval** — neither session IDs nor distinctive content (Emily, Sarah, Jen, Tom, bohemian) appear in actor output. PR #99 states "All 3 answer sessions were retrieved" but this is not confirmed by the data. **This case is ambiguous: could be partial RETRIEVAL_MISS for 2 of 3 answer sessions.**

   **Impact on recommendation**: Even if #8 and #9 are partially retrieval-related, the missing sessions contain embedded references (betta tank in a nitrite-levels session, attended weddings in wedding-planning sessions). These are the same vocabulary-gap pattern that item #8 addresses. Session-level ranking would not help surface sessions that FTS doesn't match — descriptions would.

2. **RETRIEVAL_MISS is addressed by item #8** (description-enriched FTS), not session-level signals. Cases #4 and #10 failed because FTS vocabulary didn't match — descriptions bridge that gap. Session-level ranking can't help if the session's memories aren't in the FTS result set at all. The ambiguous portions of #8 and #9 (if retrieval-related) are the same vocabulary-gap pattern.

3. **Existing per-memory signals already cover what session signals would approximate.** Per-memory recency ≈ session recency (same timestamps). Per-memory declarative density ≈ session declarative density (more precise). Per-memory co-retrieval ≈ session co-retrieval coherence (avoids feedback loops).

4. **Candidate E (topic density) is actively counterproductive** for the embedded-reference failure mode. Boosting sessions with high topic density would penalize sessions where the answer appears as a subordinate reference — exactly the cases that are hardest.

5. **The backlog's own framing was correct**: "Deferred deliberately until we have real usage data to inform what session signal should weight." There is no usage data yet. The bench uses synthetic LongMemEval data with single-date sessions, which means session-level temporal signals are degenerate (same date for all turns in a session).

---

## Section 4 — Implementation Sketch

N/A — recommending defer.

---

## Section 5 — Validation Strategy

N/A — recommending defer.

If item #11 is revisited in the future, the validation would need:

1. **Identify at least 2 specific bench questions** where answer sessions are retrieved but rank below irrelevant sessions, AND where the rank ordering change would cause the actor to produce a different (correct) answer. No such cases are documented in PR #99 or PR #100's failure analyses.

2. **Pre-validation experiment** (analogous to PR #101 for item #8): manually inject the proposed session signal into a scored candidate list, show that answer sessions move up and irrelevant sessions move down, re-run the actor on the reordered context, and verify the answer changes.

---

## Section 6 — Risks and Dependencies

### If we were to implement (hypothetical)

- **Schema**: Would need session-level aggregate table or on-the-fly computation during retrieval. On-the-fly is feasible since episode groups are small (typically 2-20 memories per session).
- **Permagent coordination**: Not required. Session/episode structure already exists.
- **Pre-validation cost**: Would need to identify failing cases attributable to ranking, which we haven't found.

### Conditions that would change the assessment

Item #11 becomes worthwhile if:

1. **A bench failure is identified where answer sessions are retrieved but ranked below K cutoff.** This would mean retrieval found the evidence but ranking discarded it. No confirmed case exists in the current multi-session failure classification (PR #99), though cases #8 and #9 are ambiguous — their retrieval status was not independently verified (see Section 3 verification). A follow-up with retrieval telemetry populated (`memory_keys` in `report.json`) would resolve the ambiguity. If those cases are partially retrieval-related, they're vocabulary-gap failures addressable by item #8's descriptions, not session-level ranking.

2. **Production usage data from Permagent shows session-level patterns** that LongMemEval doesn't capture. Real conversations have multi-day sessions, variable turn counts, and temporal patterns absent in the benchmark. Session-level signals might matter more in production than in synthetic data.

3. **Item #12 (L2 episode summaries) ships and creates session-level metadata.** If episodes get summaries, a "session summary relevance" signal becomes possible — ranking sessions by how well their summary matches the query. This is more targeted than the generic candidates analyzed above and would depend on summary quality.

4. **The GENUINE_MISS bottleneck is addressed by actor improvements** (per-session extraction, two-call patterns), shifting the limiting factor back to ranking. If the actor can reliably extract embedded references, ranking becomes the next bottleneck. Currently it's not.
