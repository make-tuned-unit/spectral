# Repair register

Everything found and **not yet applied**, with what blocks each. Live document —
update status in place; do not delete rows.

Status key: `BLOCKED` (needs someone else) · `NEEDS-PREREG` (behaviour change,
needs measurement) · `READY` (safe, implementable now) · `DONE`.

---

## R1 — Live brain: memories in retired fixture wings · DONE (applied 2026-08-04, 118→0)

**CLOSED.** Permagent applied the repair (dispatch 04h): integrity-checked
backup, daemon stopped (`bootout`), `--apply` → `scanned 1983, changed 118`,
daemon restarted, post-repair WAL-consistent dry run **would-change: 0** —
including activity rows, because their hardening (04g) removed the fixture
rules from the running process entirely. Real taxonomy hand-verified
untouched; the 118 now in `general` (uninformative beats wrong). Root cause
was worse than the fallthrough: `spectral-recognition` was not a default
feature in their shipping build, so project rules were ALWAYS empty and our
`unwrap_or_else(default_wing_rule_strings)` made fixtures their permanent
normal path. Library side: `default_wing_rule_pairs()` is now deliberately
empty in the working tree — lands with the merge, at which point their
`absent_rules_fall_through_to_spectral_fixture_wings` test retires (they
asked to be told; told in dispatch 04i). If their durable fixture count ever
drifts off 0 again, something is passing `None` — alarm, not churn.

Original record follows for history:

The library used to ship demo wing rules; they captured real content in the live
Permagent brain: `apollo` 46, `alice` 18, `acme` 17, `polaris` 16, `vega` 13,
`infra` 5, `travel` 3, `charity` 1.

Fix exists and is verified targeted (119 changes, consumer taxonomy untouched;
an unrestricted run would have hit 1,053/1,979 including `permagent`,
`polybot`, `atlasatlantic-site`).

**Blocked on:** writing to production data — denied by the permission
classifier. **Also (Permagent, 2026-08-04): the tool exists ONLY as
uncommitted work in this working tree** — zero commits on any branch, so
Permagent cannot reach it. Two paths: Jesse runs it locally from this tree
(works today, command below), or it becomes reachable to them when the
branch lands. Jesse runs it:

```bash
cp ~/.permagent/brain/memory.db /tmp/permagent-brain-backup.db
cargo run -p spectral-bench-real --release --bin wing_repair -- \
  --brain ~/.permagent/brain --apply
```

Idempotent (pinned by test). Dry run is the default.

**2026-08-04, resolved after a false drift alarm:** dry-run counts moved
119→121→118 across the day; Permagent correctly refused the monotonic-leak
reading. Row-level diff against a morning snapshot explains everything: the
movers are ephemeral `activity:*:browser_navigated:*` rows churning through
fixture wings (created by browsing — one landed in `acme` 14:25 same day,
proving capture is live at their pin — then deleted by retention). Final
decomposition: **118 = 110 durable (the stable repair target, unchanged all
day) + 8 churning activity rows.** Both sides measure 118 on the same state;
no divergence. The fixture rules ARE still live in `classifier.rs` at
c2c8381 (removal is uncommitted) and their zero-project fallthrough is real
— sequencing stands: they harden fallthrough first, then `--apply` (backup
taken), rule removal lands with the merge. Their restart-reconcile
hypothesis was unnecessary. `wing_repair.rs` header now documents copy-first
inspection and the activity-churn caveat.

---

## R2 — `recall_at` computes a decayed score it never uses · NEEDS-PREREG

`decayed_signal_score` is applied in a `map` and the result is never sorted on,
so the decay changes a *reported* number and nothing else. Either dead
computation or a missing sort.

**Blocked on:** adding the sort changes ordering for every
`recall`/`recall_local`/`recall_at` caller — a behaviour change on the default
path. Needs prereg + oracle A/B.
Ref: `decay-time-invariance-2026-08-03.md`.

---

## R3 — `FactualCurrentState` never matches bare "current" · NEEDS-PREREG

The variant is documented as *"What is my current X"* and the sub-gate lists
`currently`, not `current`. Repaired in `RetrievalPolicyVersion::V2Fixed`;
default remains `V1`.

**Blocked on:** V2 measured **inconclusive (n=1)** — the whole effect was one
question, and the +2.0pp gate was miscalibrated for n=30. Needs a corpus where
the effect is measurable.
Ref: `policy-v2-result-2026-08-02.md`.

---

## R4 — `what should i` is dead code in the GeneralPreference gate · NEEDS-PREREG

Checked after `^(?:what|where|who|which)`, so unreachable for any question
starting with "what". Repaired in `V2Fixed`; default `V1`. Same blocker as R3.

---

## R5 — `*CurrentState` sub-shapes are dead weight · NEEDS-PREREG

`FactualCurrentState` shares a cascade profile *and* route with `Factual`
(k=30, mpe=8); `CountingCurrentState` with `Counting`. They were introduced for
recency priority that no profile ever applied — confirmed: 2 questions
reclassified, zero retrieval effect.

**Real fix:** give the CurrentState shapes a profile that differs (short
`recency_half_life_days`). Not attempted.
**Prior:** low — knowledge-update is already 99.4% session-recall.

---

## R6 — Constellation fingerprints cost 39% of writes for a 12% path · NEEDS-PREREG

`IngestConfig::fingerprints` exists, default `true`. Off gives **7.0–7.8x
ingest, 14.7x storage**, byte-identical retrieval over 361 questions.

**Not a deletion decision.** Tier 1 now reaches 12% of real queries once
ungated, and whether those 12% are *better* is unmeasurable without ground
truth (see R10). Flipping the default is a product call, not a benchmark one.
Ref: `fingerprint-retirement-2026-08-03.md`, `wing-taxonomy-2026-08-03.md`.

---

## R7 — No batched write API · DONE (2026-08-05, 4.8–5.1× at the store layer)

**Shipped:** `MemoryStore::write_batch` (default sequential impl; SQLite
override = one transaction, shared `write_memory_in_tx` body so paths cannot
drift) + `ingest::ingest_batch_with` (shared `prepare_ingest`). Explicit API,
never a default — a crash loses the batch, not one event. Measured on the
shipped API, disk-backed: sequential 7.8k ev/s → batched 37–39k ev/s
(**4.77×/5.05×**, two passes). One documented+pinned divergence: no
intra-batch fingerprint pairing. No `Brain::remember_batch` (deliberate —
see result doc). Ref: `batched-write-api-result-2026-08-05.md`.

Original row:

`SqliteStore::write` opens a transaction per memory. Per-event commit is **21%**
of ingest cost; batched raw SQLite runs **60,489 ev/s vs MinHash+BM25's
22,688** — durability is not the bottleneck.

**Note:** batching changes durability semantics (a crash loses the batch, not
one event), so it must be an explicit API, never a silent default.
Ref: `ingest-gap-decomposition-2026-08-03.md`.

---

## R8 — Turn latency gate FAILED; `turn` is not the default path · DONE (deferred mode, gate PASSED)

Recall-only p95 +87–100% against a +5% kill line. Diagnosed: the synchronous
delivery-write commit, not retrieval.

**Repaired 2026-08-04, preregistered and measured.** Opt-in
`set_async_turn_delivery` spawns the ledger write off the read path with
per-occurrence ordering (`commit_turn_outcomes` awaits its own delivery —
closing a silent outcome-loss race pinned by test) and
`flush_turn_deliveries()` for shutdown. Gate: deferred p95 **−56.8% / −64.8%**
vs legacy across two runs (kill line +5%) — faster than legacy recall, which
still write-backs inline. Sync mode unchanged and still failing; `turn` stays
non-default, the mode opt-in. This was Permagent's stated condition for going
to sample 1.0 and making `turn` primary.
Ref: `deferred-delivery-prereg-2026-08-04.md`,
`deferred-delivery-result-2026-08-04.md`, `turn-latency-gate-2026-07-31.md`.

---

## R9 — Landmark salience uses token LENGTH as an IDF proxy · DONE (refuted at shipped config)

**My original framing of this row was wrong and is corrected here.** I claimed
recognition duplicates FTS5's IDF corpus. It does not: recognition's scoring
`doc_frequency` is over **composite features** (landmark pair-hashes, winnowed
k-gram hashes), which are not terms and which FTS5 cannot supply. There is no
duplicated corpus to unify.

The real defect is one layer up, and `extract.rs` names it outright:

> *"rarer-looking tokens **by length** (a cheap monotone proxy for IDF that
> keeps extraction free of store reads; corpus rarity enters at SCORING time
> via document frequencies)"*

Measured on the real brain (1,981 docs, 14,255 terms):

| | |
|---|---|
| Spearman(token length, true IDF) | **0.275** |
| top-8 landmark overlap, length-ranked vs IDF-ranked | **51%** |

Length picks a different half of the landmarks than rarity would. For an engine
whose thesis is "statistically salient features — the text analog of spectral
peaks above the noise floor", the salience measure is wrong half the time.

**True term DF is already available at zero extra storage** via SQLite's
`fts5vocab` module over `memories_fts` — verified working on the real brain.

**The trade the code names is real:** using it adds a store read at extraction,
which the current design deliberately avoids. Mitigable with a cached snapshot
rather than a per-token query.

**Implemented 2026-08-04 (seam only, default unchanged):**
`extract::TermIdf` trait, `MapIdf::from_corpus`, and
`extract_landmarks_with(content, config, Option<&dyn TermIdf>)`. Passing `None`
reproduces the length ranking byte for byte (pinned by test); with a corpus,
anchors still rank first, then true rarity, then length for terms the corpus
has never seen. 4 tests, including the exact failure case — a long common word
("characteristically") losing to a short rare one ("kafka").

**Measured 2026-08-04 — gate FAILED, fts5vocab plumbing does not proceed.**
Pre-registered two-arm run (`r9-idf-prereg-2026-08-04.md`), engine wired via
`RecognitionEngine::set_term_idf`, `public_bench --idf-arm`: R1 ΔAUC
**+0.0000114** against a +0.0010 gate; R2/R3 **bit-identical** (Δ exactly 0).
Structural cause: ranking is truncate-then-restore-position, so it only
changes the landmark SET for texts with >`max_peaks` (32) candidates —
sentences never truncate, and most R1 turns don't either. The 51% top-8
overlap is a true ranking defect that is nearly inert as behaviour at the
shipped config. Seam and measurement apparatus stay; re-opening needs a
long-document corpus or small-max_peaks config plus a fresh prereg.
Ref: `r9-idf-result-2026-08-04.md`.

---

## R10 — No labelled corpus with wing structure · BLOCKED (product)

**The bottleneck under R6, and under every tier-1 verdict ever recorded.**
LongMemEval and LoCoMo have no within-brain topic areas; the real brain has no
answer keys. Every judgement about the constellation tier — including the
0-wins/2-losses/9-ties that nearly justified deleting it — was measured on
corpora where wings cannot exist.

**Unblock IN MOTION (2026-08-04) — but corpus still ZERO ROWS.** Permagent
shipped `SafeBrain::turn` + outcome reporting, sampled and shadowed, verified
by e2e — but their second reply corrected themselves: `turn_events = 0` live.
One cause (their 04e correction retracting the build-date theory):
`PERMAGENT_TURN_SAMPLE_RATE` never set in the daemon's launchd env (default
0.0, deliberately opt-out-proof). As of 2026-08-04e sampling is LIVE —
`PERMAGENT_TURN_SAMPLE_RATE=0.1` verified inside the running process (`ps
eww`), daemon restarted; they send the row count after a real window.

**04j: still zero six hours later, and diagnosed — a TRAFFIC problem, not
instrumentation.** Total real chat traffic that day: 2 requests, both from
the phone, both aborted at the session read (their reply.rs:331) ~80 lines
upstream of the Phase-3 sampler (reply.rs:410) — an iOS client minted its
own session UUID assuming lazy session creation. Fixed their side, pending a
device rebuild they don't control the timing of. Desktop saw no chat that
day. No conclusion about `turn` is drawable from the zero, including "the
sampler is broken". They explicitly refuse to synthesize turns for the
corpus — right call: labelled queries nobody asked would be worse than no
set (the R10 principle). Wing repair also confirmed holding at 0 through a
working day + 8 restarts (fourth independent confirmation). Do not build anything on
this corpus until a nonzero count is reported. Going primary (sample 1.0 +
async delivery) additionally waits on OUR branch landing so they can bump
their pin past c2c8381. Non-cited hits are `Ignored`, never `Wrong` (content
overlap cannot distinguish unhelpful from unused — correct call).
See `permagent-dispatch-2026-08-03/04.md` and their two responses.

---

## R11 — `format_context_block` is still the undated format · NEEDS-PREREG

`recall_at` puts `spectral_tact::format_context_block` in `tact.context_block` —
ungrouped, undated, no role tags. `spectral::render::session_grouped` is the
published format. Redirecting changes output for existing consumers.

---

## R12 — ACT-R's recorded behaviour is untrustworthy · READY (re-run)

`ACTR_POOL_WIDEN` used the same post-hoc `take()` that made widening a no-op on
the cascade route, so ACT-R there was doubly inert (couldn't change membership;
its reordering never reached the actor). Fixed by F1/F2, but any ACT-R
measurement predating that fix was diluted by whatever fraction was
cascade-routed — 70% on the held-out set.

ACT-R is an off-by-default env lever and no published number depends on it, but
its record should not be cited until re-run.

---

## R13 — Wing scope comes only from query text · DONE (plumbing) / BLOCKED (value)

A wing fires when the query *names* the project — 12.4% of real queries.
`RecognitionContext::focus_wing` exists to supply scope ambiently (the agent
knows its project even when the user doesn't say so) and is **unexercised** on
the tier-1 path.

**Implemented 2026-08-03.** `spectral_tact::retrieve_memories_scoped`,
`Brain::tact_retrieve_with_k_scoped` / `cascade_retrieve_scoped`, and
`run_cascade_pipeline_scoped` now passes `context.focus_wing` through to TACT's
tier selection. A query-named wing still wins; the hint is a fallback, not an
override. `None` is byte-identical to the previous path (pinned by test).

**Premise corrected 2026-08-04 (Permagent dispatch response):** `focus_wing`
is NOT unused on the consumer side — Permagent derives it from the active
project (their brain_ops.rs:229–244 → RecognitionContext, state.rs:969).
Recomputed against the live event log (`rc_focus_wing`, snapshot 2026-08-04):
**157 of 261** queries ≥30 chars carry an ambient focus wing (**60.2%** vs
12.4% by query text), and **all 157** point at wings with content in the
brain. Ambient scoping reaches ~5x more queries than the query-text figure
suggested. Note their pin (c2c8381) predates this plumbing — it reaches them
on the next pin bump after merge.

**Value still BLOCKED on R10.** Whether ambient scoping produces *better*
results cannot be measured without outcome labels — which Permagent's sampled
turn corpus is now accumulating. First real unblock in the register's history.
Ref: `tier1-ungating-result-2026-08-03.md`.

---

## R14 — Eval-path query expansion is nondeterministic across paid runs · READY (fix known)

Found 2026-08-05 by R11's identity gate: with expansion on (the default),
the pre-retrieval LLM expansion samples differently across runs — 3/120
LoCoMo questions retrieved different SETS in two same-day runs of identical
code on brains proven byte-identical. Every prior paid A/B that left
expansion on carries this (unmeasured, probably small) noise floor; the
#238 "retrieval is deterministic" result is true only GIVEN the query.

**Fix options:** freeze expansion via a cache input on the eval path (the
oracle already supports `--expansion-cache`), or run paired comparisons
with `--no-expand-queries` (what R11 does now, prereg amendment 5).
Ref: `r11-render-ab-stage1-void-2026-08-05.md`.
