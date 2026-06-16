# Constellation / Retrieval Subsystem Reality Audit

**Scope:** READ-ONLY audit of the constellation/retrieval subsystem against current `main`
(commit `97f40f0`, worktree branch `worktree-agent-a18a9209141feb144`).

**Method:** Every claim is backed by `file:line` from the *actual implementation*, not
comments or prior docs. Where a citation could not be found, this is stated explicitly.

**Crates examined:** `spectral-core`, `spectral-ingest`, `spectral-graph`, `spectral-cascade`,
`spectral-spectrogram`, `spectral-tact`, `spectral-bench-accuracy` (plus `spectral`,
`spectral-bench-real`, `spectral-archivist` for caller/actor tracing).

Paths below are relative to the repo root
`/Users/jessesharratt/projects/spectral/.claude/worktrees/agent-a18a9209141feb144`.

---

## 1. Inventory Table

"WIRED" = reachable from a real recall path (`recall_cascade` / `recall_local` / `aaak`),
not merely defined. "CONSUMED" = the actor (bench harness / any consumer binary) actually
puts the data in front of the LLM, as opposed to it being used only internally for ranking.

| Component | EXISTS | WIRED (live recall path) | ENABLED (by default) | CONSUMED (by actor) |
|---|---|---|---|---|
| **TACT fingerprint search** | YES — `crates/spectral-tact/src/extractor.rs:36` (`store.fingerprint_search`), Tier 1 of `search()` | YES (reachable) — `recall_cascade` → `cascade_retrieve` → `tact_retrieve_with_k` → `spectral_tact::retrieve` (`brain.rs:1099`, `brain.rs:1080`) → `extractor::search` (`spectral-tact/src/lib.rs:129`) | CONDITIONAL — only fires if query classifies into **both** a wing AND a hall (`extractor.rs:34`). Default Brain wing rules exist but are narrow (see §4); most queries skip Tier 1 | NO direct — feeds candidate set; method label not shown to LLM. Cascade actor formatter emits only `content`/`role`/`date` (`retrieval.rs:301`) |
| **Wing search** | YES — `crates/spectral-tact/src/extractor.rs:49` (`store.wing_search`), Tier 2 | YES (reachable) via same `retrieve()` path | CONDITIONAL — fires only if query classifies into a wing (`extractor.rs:47`); query-side `detect_wing` returns `None` (not "general") on no match (`classifier.rs:6-16`) | PARTIAL — `wing` reaches the prompt only on non-cascade flat formatter `format_hit` (`retrieval.rs:311-320`); the **default cascade path does not emit wing** (`retrieval.rs:301`) |
| **Hall** | YES — assigned at ingest (`spectral-ingest/src/ingest.rs:140`, `classifier.rs:29-37`); detected at query (`spectral-tact/src/classifier.rs:19-29`) | YES — used to gate fingerprint Tier 1 (`extractor.rs:34`) and to filter AAAK (`brain.rs:2276-2284`) | YES — 4 default hall rules always present (`spectral-ingest/src/classifier.rs:82-98`; TACT default `lib.rs:52-70`) | PARTIAL — `hall` emitted only by flat `format_hit` (`retrieval.rs:311-320`), not by cascade path |
| **Spectrogram** | YES — `spectral-spectrogram` crate; writer `sqlite_store.rs:1298`; reader `sqlite_store.rs:1348/1388`; analyzer wired in `brain.rs:510` | **NO** — read path (`recall_cross_wing`, `brain.rs:1528`) is **not** called by `recall_cascade`/`recall_local`/`aaak`; `cascade_layers.rs` has zero spectrogram references | NO — `enable_spectrogram` defaults `false` (`brain.rs:71-72`; no `BrainConfig::Default`, builder derives `false`) | NO — actor never reads spectrogram/fingerprint fields; bench opens brains with `enable_spectrogram: false` (`retrieval.rs:535,585,648,732,1084`) |
| **AAAK** | YES — `Brain::aaak()` `brain.rs:2241`; opts `brain.rs:274-302` | **NO** — `aaak()` is reachable as an API but is **not** invoked by any non-test caller; all `.aaak(` call sites are in `tests/aaak_tests.rs` | N/A — standalone API, not part of cascade default | NO — actor builds its prompt only from `memories` (`actor.rs:101-106`); AAAK is never injected |

---

## 2. The Gap — what would activate each dormant component

### TACT fingerprint search (Tier 1) — *partly firing, under-fed*
- **State:** Wired and reachable. Fires only when a query matches both a wing regex AND a
  hall regex (`extractor.rs:34`). With default Brain config the wing rules are present
  (`brain.rs:457-468` injects `default_wing_rule_strings()`), so it is **not** dead — but it
  is rarely reached because the wing regexes are narrow proper-noun lists (§4).
- **What activates it more:** broader/selective query-side wing classification (regex or a
  real classifier) so more queries resolve to a wing+hall. Also note even on a hit, Tier 1
  immediately merges in FTS results and returns `RetrievalMethod::Fingerprint`
  (`extractor.rs:38-42`) — fingerprint contributes, FTS still dominates recall.
- **Classification:** **Spectral-side.** No external data required; broadening the rules or
  the classifier is within our control.

### Wing search (Tier 2) — *reachable, narrowly triggered*
- **State:** Fires only when `detect_wing` returns `Some` (`extractor.rs:47`). On the query
  side, no-match returns `None` (`classifier.rs:15`), so the query falls straight to FTS.
- **What activates it:** same as above — query classification breadth. Optionally setting
  `RecognitionContext.focus_wing` would let the *ambient boost* (a different mechanism, see
  below) reward wing-aligned hits, but that does not change which TACT tier fires.
- **Classification:** **Spectral-side.**

### Ambient boost / `focus_wing` (related, not a search tier)
- **State:** `ambient_boost_for_hit` is identity (1.0) whenever context is empty
  (`cascade_layers.rs:18-20`). The bench always passes `RecognitionContext::empty()`
  (`retrieval.rs:457-458`, `inspect.rs:118-119`); `focus_wing` is never set anywhere in the
  repo (no `with_focus_wing`/`focus_wing(` call sites found).
- **What activates it:** an actor-side change to populate `focus_wing` / `recent_activity`
  in `RecognitionContext`.
- **Classification:** **Spectral-side** (actor change) — but only meaningful once memories
  carry discriminating wings (gated on §4 wing selectivity, which is data-shaped).

### Spectrogram (read path) — *dead, not merely disabled*
- **State:** See §3. The only reader, `recall_cross_wing` (`brain.rs:1528`), is not wired
  into any live retrieval pipeline.
- **What activates it:** an actual code change to call `recall_cross_wing` (or load
  spectrograms inside `run_cascade_pipeline`/`ranking.rs`) **and** flipping
  `enable_spectrogram=true` so rows exist to read.
- **Classification:** **Spectral-side** (it is a wiring + flag job), but it is a genuine
  *build* because no live path consumes it today — see verdict.

### AAAK — *built, tested, unconsumed*
- **State:** `aaak()` works (`brain.rs:2241-2319`), reads from `list_memories_by_signal` /
  `list_wing_memories` (`brain.rs:2254/2270`), independent of cascade and spectrogram. No
  production caller; only `tests/aaak_tests.rs`.
- **What activates it:** an **actor-side change** — the actor must call `aaak()` and inject
  the formatted string into its system prompt. Today `actor.rs:101-106` builds the prompt
  only from per-query `memories`.
- **Classification:** **Spectral-side** (actor wiring).

---

## 3. Spectrogram Specifically

**Writer path (`memory_spectrogram` table):**
- Trait decl `spectral-ingest/src/lib.rs:331`; impl `INSERT INTO memory_spectrogram` at
  `spectral-ingest/src/sqlite_store.rs:1298` (SQL at `sqlite_store.rs:1318`).
- Caller 1: `brain.rs:956`, inside `remember_with`, gated behind `if self.enable_spectrogram`
  (`brain.rs:952`).
- Caller 2: `brain.rs:1681`, inside `backfill_spectrograms()` (`brain.rs:1659`) — **not**
  gated by the flag, but `backfill_spectrograms` itself has **zero callers** in the
  workspace (grep returns only its definition). So it is manual/dead API.
- No other writers (no audit-tool writer, no cron/job writer). The bench only reads a coverage
  count (`spectral-bench-accuracy/src/main.rs:721,728,799-841`), never writes.

**Reader path:**
- `load_spectrogram` `sqlite_store.rs:1348` (SELECT `:1361`); `load_spectrograms`
  `sqlite_store.rs:1388` (SELECT `:1405/:1432`).
- Sole consumer: `recall_cross_wing` (`brain.rs:1528`), calling `load_spectrogram`
  (`brain.rs:1552`) and `load_spectrograms` (`brain.rs:1587`).
- `recall_cross_wing` callers: public re-export wrapper `spectral/src/lib.rs:267` (calls at
  `:274`), whose only callers are tests `tests/spectrogram_tests.rs:133,168,208`.

**Is the read path DEAD or DISABLED?**
**DEAD.** `recall_cross_wing` — the only function reading `memory_spectrogram` — is never
invoked from any non-test code. It is not wired into `run_cascade_pipeline`
(`cascade_layers.rs:143`, which has zero `memory_store.`/spectrogram references), nor into
`recall_cascade`/`recall_local`/`aaak`/`recall`, nor any bench/actor binary. Even with
`enable_spectrogram=true`, no production retrieval path would call the reader. This is a
*reachability* problem (dead code), distinct from a feature flag.

Separately, the **write path is DISABLED by default**: `enable_spectrogram` is a plain `bool`
documented "Default false" (`brain.rs:71-72`); there is no `impl Default for BrainConfig`; the
`spectral::BrainBuilder` is `#[derive(Default)]` (`spectral/src/lib.rs:548-549`) so the flag
defaults to `false`; every non-test caller sets it `false` explicitly. Only
`tests/spectrogram_tests.rs:16` sets it `true`.

**Does the spectrogram feed cascade RANKING?**
No. `spectral-graph/src/ranking.rs` has zero references to `spectro`/`fingerprint`/
`spectral_spectrogram`. Cascade ranking uses `declarative_density` (`brain.rs:945`),
co-retrieval, signal score, recency, episode diversity (`cascade_layers.rs:160-183`) — none
read spectrogram data. `SpectrogramAnalyzer` output is used **only** inside the dead
`recall_cross_wing`.

**Verdict on the prior "built and dormant" claim:** PARTIALLY CORRECT, IMPRECISE. The *writer*
is built and dormant (disabled by flag). The *reader* is not merely dormant — it is **dead**:
unreachable from any live path regardless of the flag. "Built and dormant" undersells how far
the read side is from production; flipping the flag alone does nothing.

---

## 4. Wing Discrimination

**What the wing rules are / where they come from:**
HARDCODED in source as `&'static str` literals, not loaded from config or data.
`default_wing_rule_pairs()` `spectral-ingest/src/classifier.rs:57-80` — **8 wings**, each a
single alternation regex matched against `lowercase("{key} {content} {category}")`:

```
classifier.rs:59-60  alice|coffee|anniversary|colou?r|favourit|favorit|sons|noah|leo|carol-doe  -> "alice"
classifier.rs:63-64  apollo|polymarket|strategy|weather|prediction|wager|trade                  -> "apollo"
classifier.rs:67     acme|widget|bob|recipe|cook|feast                                          -> "acme"
classifier.rs:68     charity|advocacy|grant|nonprofit|fundrais                                  -> "charity"
classifier.rs:69     vega|sales|purchase|commerce                                               -> "vega"
classifier.rs:70     travel|immigration|visa|permit                                             -> "travel"
classifier.rs:71-72  polaris|volunteer|plogging|litter|marathon|summit                         -> "polaris"
classifier.rs:75-76  task.runner|litellm|infrastructure|ollama|gemma|model.ladder              -> "infra"
```

Halls: `default_hall_rule_pairs()` `classifier.rs:82-98` — 4 halls (`fact`, `preference`,
`discovery`, `advice`); fallback `event`.

**Wing assignment / classification code:**
- Ingest-time: `classify_wing()` `classifier.rs:11-24` — regex first-match-wins; on no match
  returns `"general"` (`classifier.rs:23`). Wired in `ingest.rs:137-139`:
  `opts.wing.unwrap_or_else(|| classify_wing(...))` — explicit override wins, else regex, else
  `"general"`.
- Query-time (retrieval): `detect_wing()` `spectral-tact/src/classifier.rs:6-16` — same regex
  approach, but on no match returns **`None`** (not `"general"`). Asymmetry worth noting:
  ingest defaults to `"general"`, query defaults to no-wing.

**Override path:** A caller *can* override via `BrainConfig.wing_rules` (`brain.rs:65`),
resolved at `brain.rs:450-455` (`unwrap_or_else(default_wing_rule_strings)`) and fed into both
`tact_config` (`brain.rs:457-468`) and `ingest_config` (`brain.rs:470-480`). The bench does
**not** override — every `BrainConfig` passes `wing_rules: None`
(`spectral-bench-accuracy/src/ingest.rs:52-53`; `retrieval.rs:532-533,582-583,645-646,
729-730,1081-1082`; `bin/audit.rs:56-57`; `describe.rs:596-597`). So the bench runs exactly
the 8 hardcoded rules.

**Is the ~76% "general" lack-of-selectivity still true on `main`?**
**YES.** The wing rules are narrow project-specific keyword sets (proper nouns `alice`,
`apollo`, `acme`, `vega`, `polaris`; domain tokens `polymarket`, `plogging`, `litellm`,
`ollama`). A typical conversational memory that mentions none of these matches nothing and
falls to `"general"` (`classifier.rs:23`); the test `wing_general_fallback`
(`classifier.rs:146-152`) confirms "hello world" → "general". There is **no LLM-based wing
classification** anywhere — `classify_wing` is purely regex; `BrainConfig.llm_client`
(`brain.rs:62`) is documented for "TACT classification" but is **not** wired into wing
assignment (and TACT retrieval itself is regex-only — `spectral-tact/src/lib.rs:17-18`, and the
`LlmClient` trait at `lib.rs:19-24` is unused dead code). The selectivity ceiling is fixed by
the 8 hardcoded regexes; lack of discrimination persists.

---

## Verdicts

| Component | Verdict | Rationale |
|---|---|---|
| **TACT fingerprint** | **WIRE** | Reachable and partly firing today; activating it more broadly is Spectral-side (broaden/replace the regex query classifier so more queries hit a wing+hall). No external data needed. Note its win is diluted: on a hit it merges FTS and returns immediately (`extractor.rs:38-42`). |
| **Wing search** | **WIRE** | Reachable; gated only by query-side wing classification, which we control. Broadening rules or adding a classifier is buildable now. Caveat: real *value* from wing discrimination is **GATED** on memories carrying non-"general" wings (§4) — the mechanism is Spectral-side, the payoff is data-shaped. |
| **Hall** | **WIRE** | Already enabled and used (Tier-1 gating + AAAK filter). To surface hall to the actor in the *default cascade* path is a small Spectral-side formatter change (`retrieval.rs:301` vs `:311-320`). |
| **Spectrogram** | **BUILD** | Writer is disabled-by-flag, but the reader (`recall_cross_wing`) is genuinely **dead** — no live path calls it and it feeds no ranking signal. Making it real requires actual implementation work: wire a reader into the cascade/ranking path, then enable writes. Not a flag flip. |
| **AAAK** | **WIRE** | Fully implemented and tested; the only missing piece is an **actor-side call** that injects `aaak()` output into the system prompt. Purely Spectral-side wiring, buildable now, no external data. |

**Cross-cutting note (skeptical):** The most impactful single gap is wing *selectivity* (§4).
Several components (fingerprint Tier 1, wing Tier 2, ambient `focus_wing` boost) are
mechanically wired but starved because real-world memories collapse to `"general"`. Improving
the classifier is Spectral-side; whether that yields better retrieval is contingent on
production data distribution, which this audit cannot measure from source alone.
