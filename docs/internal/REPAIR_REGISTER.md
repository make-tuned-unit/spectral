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

## R11 — `format_context_block` is still the undated format · DONE (validated +14.2pp, SHIPPED 2026-08-06)

**The first prereg-validated accuracy lever in the project's history.**
Two-stage held-out LoCoMo A/B, identical retrieval by construction and
verification (identity gate; first attempt VOIDED itself — see R14): dev
+19.2pp (p=1.6e-6), disjoint validation **+14.2pp (B-fixed 20/broke 3,
McNemar p=4.9e-4)**. The entire effect is temporal-reasoning (validation
20.0%→62.5%); other categories bit-flat — the undated block starved
temporal questions of dates. Facade recall surfaces now publish
`session_grouped` as `context_block` (BREAKING for old-block parsers; hits
untouched; pinned by test). Refs: `r11-render-ab-prereg-2026-08-05.md`,
`-stage1-void-`, `-stage1-result-2026-08-05.md`,
`-stage2-result-2026-08-06.md`.

Original row:

`recall_at` puts `spectral_tact::format_context_block` in `tact.context_block` —
ungrouped, undated, no role tags. `spectral::render::session_grouped` is the
published format. Redirecting changes output for existing consumers.

---

## R12 — ACT-R's recorded behaviour is untrustworthy · DONE (re-run 2026-08-06: active but metric-neutral)

**Re-measured post-F1/F2, $0, 500-question paired oracle A/B (single
variable `SPECTRAL_ACTR_DECAY=0.5`, reused brains):** the lever now changes
**389/500 contexts** (the old inert record confirmed stale) and moves no
oracle metric — session-recall +2/−1, zero-evidence 0/0, keys +14 (noise),
tokens +107. Citable form: reshuffles context composition, no measured
retrieval benefit; stays off by default. Any accuracy claim needs a paid
replay of the 389 changed contexts — unjustified by this signal.
Ref: `r12-actr-rerun-note-2026-08-06.md`. Original row:

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

## R14 — Eval-path query expansion is nondeterministic across paid runs · DONE (2026-08-06)

**Shipped:** `run --expansion-cache` replays frozen expansion (fail-loud on
miss, contradictory-flag guard, enters the config fingerprint; pinned by
test). Caches generated for all three LoCoMo samples (~$0.075). Verified
live on locomo_5_46 — one of the three questions that voided R11 stage 1 —
two full eval runs, retrieved keys byte-identical. The silent
expansion-failure fallback is also bypassed in cache mode.
Ref: `r14-frozen-expansion-result-2026-08-06.md`. Original row:

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

---

## R15 — The oracle's `answer_keys_*` metric is diluted 12× · DONE

LongMemEval ships per-turn `has_answer: true` flags, documented for
turn-level recall evaluation. `oracle::is_answer_key` instead counts
**every turn in an answer session**: 10,960 turns against **896 true
evidence turns**. So "key-recall 55.6%" measures evidence-session turn
coverage, not evidence recall, and the 98.1% we quote is *session*
recall.

True evidence-turn recall, computed 2026-08-07 from existing rows:
**88.5% micro / 90.5% macro, with 27/479 questions retrieving zero
evidence.** `single-session-preference` is **65.9%** with 9/30 zero.

**Fix:** add a first-class `evidence_turns_{total,retrieved}` metric from
`has_answer`; rename `answer_keys_*` to say what it measures. Until then
no document may cite "key-recall" as evidence about retrieval quality.
**Consequence:** every refuted retrieval lever was scored against a
metric that could not see this defect.
Ref: `turn-level-evidence-recall-2026-08-07.md`.

**SHIPPED 2026-08-07** (instrument only — no gate, no paid run, retrieval
byte-identical). What landed:

* `dataset::Turn.has_answer: Option<bool>` — the label is no longer discarded
  at load. Skipped on serialize when absent, so datasets round-trip unchanged.
* `ingest::memory_key()` — one authority for the key format, used by both the
  write path and evidence scoring, frozen by `memory_key_format_is_frozen`.
  Byte-identical to the previous inline `format!`s, so archived bench brains
  stay reusable.
* `OracleRow`: `answer_keys_{total,retrieved}` / `rank_first_answer_key`
  renamed to `answer_session_turns_{total,retrieved}` /
  `rank_first_answer_session_turn`, each carrying `#[serde(alias = …)]` so the
  entire JSONL archive still loads. Added `evidence_turns_{total,retrieved}`,
  `rank_first_evidence_turn`, `evidence_keys_missed`.
* `OracleSummary`: micro/macro evidence recall, zero/full-evidence counts,
  labelled/unlabelled counts. Unlabelled rows are excluded from every mean —
  counting them as 0 would fabricate a 90.5%→86.7% regression.
* `oracle-evidence` subcommand: rescores archived rows offline for $0, never
  rewrites its input, optional `--baseline` for a paired evidence diff.
* `stratified_ab.rs` renamed its second copy of the diluted computation.

**Refusals, not zeroes.** The metric emits `None` when the dataset carries no
label, when ingest is `PerSession` (the field would count sessions while its
name says turns), and — the read-side guard — when the retrieved key set is
not turn-shaped, which is what the `Graph` path produces (no raw hits →
`extract_keys` falls back to `--- Session <id>` parsing). Without that guard
the Graph path would silently report a fabricated 0/N.

**Backfilled evidence numbers** (`oracle-evidence`, all $0, all from rows
already on disk; these are re-descriptions of existing runs, not results):

| archived run | evidence-turn recall (micro) | zero-evidence |
|---|---|---|
| `r12-baseline` (shipped config) | 793/896 = 88.5% | 27/479 |
| `r12-actr` | 794/896 = 88.6% | 27 |
| `oracle-baseline` | 749/896 = 83.6% | 35 |
| `oracle-porter` | 783/896 = 87.4% | 34 |
| `oracle-cap` | 749/896 = 83.6% | 35 |
| `oracle-bfs-actr/bfs2` | 774/896 = 86.4% (base 789/896) | 34 |
| `oracle-bfs-actr/actr05` | 788/896 = 87.9% (base 789/896) | 29 |
| `r16-pre` → `r16-post` | 793/896 = 88.5% → 88.5% (unchanged) | 27 → 27 |

**Where the delta is UNKNOWN, and stays unknown.** The answerability preregs
(run 1/2/3) and the supersession prereg used key-recall as a gate criterion,
and their per-arm oracle row files were not retained on this machine — so the
evidence-turn delta for those experiments cannot be recomputed and is not
asserted in either direction. The LoCoMo k-lever prereg's rows *are* retained
but LoCoMo carries no `has_answer` labels, so `oracle-evidence` reports `n/a`
there: also unknown, not zero. The published verdicts are left exactly as
written; only the framing of the metric is corrected.

**The ban is now structural.** "key-recall" no longer exists as a field or a
column name, so citing it requires deliberately reaching for
`answer_session_turn_coverage`, whose name says what it is.

**Follow-ups deliberately NOT done here:**
* Per-turn `has_answer` labels for LoCoMo (`scripts/locomo_to_oracle.py`) —
  see R19. Regenerating the samples risks moving the R11 held-out set.
* Metric-caveat banners on the ~15 archived result/prereg docs that cite
  key-recall. The banner text and the file list are in
  `r15-evidence-metric-2026-08-07.md`; the historical numbers in those docs
  must not be rewritten.
* **Three stale in-code vocabulary sites, not one** (the result doc says one,
  and gives a false reason — `spectral-bench-accuracy/src/retrieval.rs` is
  **not** modified in the working tree): `retrieval.rs:804`,
  `cascade_layers.rs:186`, `cascade_layers.rs:247`. Comment text only, no
  behaviour.
* **The ban is structural in code but not yet in this register.** The R16 row
  below, written in the same tree, reintroduces "key-recall" and
  "zero-answer-key" in prose. Historical numbers stay; the label needs the
  R15 qualifier wherever it appears.

**Verified independently 2026-08-07** (`research-alignment-2026-08-07.md` §1):
every backfilled figure recomputed from the labels in independent Python and
reproduces to the digit; all 45 rows of the committed CI fixture are verbatim
archive lines and its evidence-key set is identical to the full dataset's for
all 45 questions. **READY TO MERGE.**
Result: `r15-evidence-metric-2026-08-07.md`.

---

## R19 — LoCoMo converter emits no per-turn evidence labels · DONE (2026-08-08)

**SHIPPED, and it overturned a conclusion published the day before.**
`locomo_to_oracle.py` now emits per-turn `has_answer`, matched by `dia_id` with
sessions deep-copied per QA. 1438 questions, 2140 evidence turns labelled.

The mandatory gate is enforced in code as `--verify`: regenerate, strip
`has_answer`, compare **serialized bytes**, exit non-zero without writing if
they differ (and say whether membership, order, or content moved). Run against
the published baseline dataset: **GATE PASSED, sample unmoved.**

**Finding.** Rescoring the published BM25 baseline — same run, same retrieved
keys, $0 — evidence-turn recall is **59.86% micro / 68.63% macro** against
95.06% session recall, with **24.86% of questions retrieving ZERO evidence
turns** (vs 1.11% on the session metric). Dilution on LoCoMo is **20.6x**.
Correct-vs-incorrect separation goes from 5.94pp on session recall to
**57.08pp** on evidence-turn recall (88.62% vs 31.54%).

The baseline document's claim that "retrieval is not the binding constraint" is
**refuted**. It cited R15, correctly labelled the metric as diluted, and
reasoned from it anyway. Labelling a diluted metric does not stop it being
misread; making the real one computable does.
Result: `r19-locomo-turn-labels-2026-08-08.md`.

Original row follows.

---

`scripts/locomo_to_oracle.py` marks whole sessions `answer_`, so every
LoCoMo-converted set (including the R11 held-out and validation samples)
scores `n/a` on evidence-turn recall while LongMemEval scores a real number.
That asymmetry is worse than the old uniform wrongness: the held-out set is
still only measurable on the diluted metric.

The labels are recoverable — LoCoMo turns carry `dia_id` (`"D1:3"`) and each
QA carries `evidence: ["D1:3"]` — but must be matched **by dia_id, never by
index**, because the converter drops empty-text turns so positions do not
correspond. Sessions must be deep-copied per QA since evidence differs.

**Mandatory gate before any use:** regenerate the existing samples with the
same seed and exclusions, strip `has_answer`, and assert **byte-equality**
with the current files. If sample membership moves, the R11 validation set
stops being the set that was measured.
Ref: `r15-evidence-metric-2026-08-07.md`.

---

## R16 — Default FTS path has no SQL tiebreak · **UNBLOCKED, MERGED**

> **BLOCKER RESOLVED 2026-08-07 (later same day).** The question the quarantine
> posed — does the tiebreak *expose* a pre-existing order-invariance violation
> or *introduce* one? — is answered **exposes**, by experiment on `main` with
> **no R16 change present**:
>
> - `aged_brain()` inserts chronologically, so rowid order (= untiebroken FTS
>   order) agrees with age order, and no pair exists for the shrinking additive
>   freshness term to swap. The test passed for that reason, not for the reason
>   its docstring gave.
> - Re-run on `main` with the **same 24 memories and the same timestamps**
>   inserted in a fixed shuffled order, the ranking drifts across the same
>   5-year clock shift: positions 8/9 (`s22`/`s1`) and 13/14 (`s21`/`s12`) swap.
>   No SQL change involved. The violation is R20's, it predates R16, and R16
>   merely removed the accidental alignment that hid it.
>
> Resolved under R20's own **interim requirement (Rule 5)**: the test is
> re-baselined to assert what the path actually guarantees, not silently
> relaxed. `recency_decay_is_order_invariant_in_the_topk_path` is replaced by
> two tests, both of which **fail on `main` and pass with R16**:
> `topk_additive_recency_reorders_under_a_clock_shift` (pins the real
> behaviour — and pins stability at a *fixed* anchor in the same test) and
> `topk_ranking_is_independent_of_insertion_order` (two brains, same content,
> different write order, identical ranking — the property R16 buys and the one
> the README claims). Workspace suite: **zero failures**. R20 stays open and
> still needs its prereg; nothing below it was fixed.
>
> **Historical blocker text follows.**

> **STATUS CORRECTION 2026-08-07.** This row previously read `DONE`. The
> implementation report claimed `suite_passed: true`; that is **false**.
> `crates/spectral/tests/deterministic_anchor.rs:83`
> `recency_decay_is_order_invariant_in_the_topk_path` **FAILS** with the change
> present. Bisected and round-tripped independently (file restored
> byte-identical, sha256 `df52dc90…` before and after): reverting only the two
> `, m.id` clauses makes the suite 6/6 green; restoring them makes it 5/6.
> Workspace total: **297 passed, 1 failed, 1 ignored.**
>
> **The tiebreak is not the defect — it exposed one.** `recall_topk_fts` does
> not call the multiplicative `apply_recency_weight` the test's docstring
> describes; it calls `apply_reranking_pipeline`, where (a) the base score *is*
> the FTS rank position (`ranking.rs:345-347`, `1.0 - i/n`), so reordering a
> bm25-tied pool is **not** score-neutral, and (b) recency is **additive**
> (`ranking.rs:411`), so ranking is a function of the wall clock. Truncation to
> `k` happens after reranking (`brain.rs:2147`) on a pool of `k × fetch_mult`,
> so a boundary flip changes the retrieved **set**. Opened as **R20**.
>
> Byte-identical repeat runs at a fixed `now` are **not** broken
> (`reproducible_retrieval_is_stable_across_repeated_calls` passes; the pre-arm
> reproduced itself 0/500). R16's own determinism claim stands.
>
> **Merge is blocked until the test is fixed or explicitly re-baselined with a
> recorded justification, and R20 is opened either way.** Re-baselining is
> defensible — the test asserts a property of a function the path does not call
> — but silently relaxing the assertion is not.
>
> Two further corrections to the report: "9 of 10 are pure reorder" is measured
> and correct but must **not** be read as "harmless" — with a rank-position base
> score, reordering a tied pool feeds different numbers into every downstream
> boost (the empirical "no metric moved" result is unaffected). And the "six
> remaining untiebroken sites" scoping claim understates the exposure by ~2× —
> see the corrected R18 table.
> Ref: `research-alignment-2026-08-07.md` §2, §8.

Original row (measurement independently re-derived and confirmed exact):

**Shipped:** `, m.id` added at **two** sites — the non-fusion default
`fts_search` ORDER BY, and the two fusion channel subqueries in
`ranked_ids` whose *rank positions* feed RRF (the register originally
named only the first; the existing `.then(a.0.cmp(&b.0))` breaks ties in
the **fused** score, one layer too late). Pinned by two unit tests that
build a genuine bm25 tie, insert the larger id first, and assert a
literal smaller id wins at `LIMIT 1` — both fail if the clause is
reverted.

**Measured, $0, on the merge commit in a clean detached worktree**
(pre-arm reproduced `r12-baseline.jsonl` at 0/500 and itself at 0/500
first): **10/500 contexts changed (2.0%)**, all on `TopkFts`, **0/333 on
`Cascade`**. 9 are pure reordering of an identical retrieved set; 1
(`b9cfe692`) swaps one document within one session. **Every metric is
unmoved** — session-recall 98.03% macro / 97.78% micro,
answer-session turn coverage 55.51% macro / 53.02% micro (the ~12×-diluted
legacy metric published as "key-recall" until R15 — **not** evidence recall),
zero-answer-session-turn 2, mean tokens 14212.8, and R15's
evidence-turn recall 793/896 = 88.50% with **zero** per-question change.
Recorded as a baseline shift; **no accuracy claim**.

**The original rationale in this row was wrong and is deleted, not
repeated.** The claim that FTS5's `1e-6` IDF clamp makes common-term
documents "collapse into one large tie block" is empirically false on
this corpus: the tf/doclen factor still varies, so a pure-`"the"` query
yields ~2585 *distinct* near-zero scores, and 0/40 brains have the LIMIT
boundary inside a tie block. Real ties are rare and small — 0/120
full-question queries straddle; single-term queries straddle in 24% with
median block 2, max 5. The justification that survives is the one that
does not depend on tie size: **the byte-identical invariant must not rest
on SQLite's plan choice.**

**The `ORDER BY rank` "pure latency win" is REJECTED**, and that sentence
is struck here and in §G0. Scores are bit-identical, but `ORDER BY rank,
m.id` reintroduces the temp B-tree (5.29 ms vs 5.06 ms for today's form),
so the latency win and the determinism fix are mutually exclusive; and
untiebroken it moves the LIMIT boundary into FTS5's *undocumented
internal* ordering — a Rule 3 regression for ~1.3 ms. The persistent
rank-config form is rejected separately (writes `memories_fts_config`;
fails read-only, and silently falls back to unweighted `bm25(1,1,1)` on
existing brains). Shipped cost: ~0.4 ms p50, directional only.

Ref: `r16-baseline-shift-2026-08-07.md`, `landscape-research-2026-08-07.md`
§G0 (corrected). **Follow-ons: R17, R18 — deliberately NOT folded in here.**

---

## R17 — `list_memories_by_signal` has no tiebreak, on a guaranteed tie key · READY

`sqlite_store.rs:1779`: `ORDER BY signal_score DESC LIMIT ?2`.
`signal_score` **defaults to 0.5**, so unlike R16 the LIMIT boundary sits
inside a large tie block *by construction*, and which memories survive is
decided by SQLite's plan. Reached from `brain.rs:4341` (`aaak`).

**Very likely a larger determinism exposure than R16 itself.** Not
quantified — it needs its own paired oracle run and must not be folded
into R16's clean 10/500 attribution.
**Fix:** same shape, `, m.id`.

---

## R18 — **Twelve** more untiebroken product `ORDER BY … LIMIT` sites · READY

> **CORRECTED 2026-08-07.** This row and `r16-baseline-shift-2026-08-07.md`
> § Scope both said "five more" / "six further sites, verified by grep". That
> is materially incomplete — a re-grep of the post-change file finds **twelve**
> untiebroken `ORDER BY … LIMIT` product sites across **eleven** functions.
> The omission that matters most is `:2686`, a **DELETE**.
> Ref: `research-alignment-2026-08-07.md` §8.

Same defect class, each needing its own paired run:

| site | fn | key | note |
|---|---|---|---|
| **`:2686`** | **`prune_wing_keeping_recent_per_source`** | `datetime(created_at) DESC LIMIT ?3` inside `DELETE … WHERE id NOT IN (…)` | **DO THIS FIRST.** Which rows are *destroyed* is decided by SQLite's plan. Arguably higher severity than R17: every other site picks what you see, this one picks what survives. Append-only discipline (Rule 4) makes an unpinned delete boundary the worst member of the class. |
| `:1843`, `:1849` | `fingerprint_search` | `hits DESC` — small integer `COUNT(*)`, plus a second untiebroken outer `ms.hits DESC` | large ties structurally guaranteed |
| `:2462` | `list_wing_memories_since` | `datetime(created_at) DESC` | low-resolution timestamp |
| `:2744` | `find_recent_episode` | `ended_at DESC LIMIT 1` | `LIMIT 1` on a tie = arbitrary single pick |
| `:2785`, `:2795` | `list_episodes` (wing-filtered and unfiltered branches) | `ended_at DESC` | low-resolution timestamp |
| `:3458` | `list_undescribed` | `created_at DESC` | low-resolution timestamp |
| `:3484` | `related_memories` | `co_count DESC` | small integer |
| `:3569` | `recommend_by_lift` | `lift DESC, n.co_count DESC` | two-key sort, **still no unique final key** |
| `:3776` | `events_for_session` | `timestamp ASC` | low-resolution timestamp |
| `:4073` | `list_unconsolidated` | `m.created_at DESC` | low-resolution timestamp |

(Line numbers are post-R16.) Already tiebroken and correctly excluded:
`:1756` (`id DESC`), `:2024`/`:2055` (R16, `m.id`), `:3241`
(`m.memory_id ASC`), `:3519` (`other_id ASC`). Lower severity and **not**
counted above — untiebroken `ORDER BY` with **no** `LIMIT`, so the set is
complete and only the caller-visible order is unpinned: `:1724`, `:1913`,
`:3806`, `:4028`, `:4044`.

Bench binaries carry the same clause and are deliberately excluded:
`stmt_cache_probe.rs:56`, `bm25_weights_experiment.rs:245,280`,
`fts_fusion_experiment.rs:119`.
Ref: `r16-baseline-shift-2026-08-07.md` § Scope (corrected),
`research-alignment-2026-08-07.md` §8.

---

## R20 — Top-k ranking is a function of the wall clock · NEEDS-PREREG

**Found 2026-08-07 by R16 turning `deterministic_anchor` red.** The failing
test is the symptom; this row is the disease, and it predates R16.

`recall_topk_fts` does **not** use the multiplicative `apply_recency_weight`
that `deterministic_anchor.rs:60-70` describes. It uses
`apply_reranking_pipeline`, where three properties compound:

1. **The base score is the FTS rank *position*** — `ranking.rs:345-347`,
   `scores[i] = 1.0 - (i as f64 / n)`. A pure reorder of a bm25-tied pool is
   therefore **not** score-neutral; it changes every downstream number.
2. **Recency is ADDITIVE** — `ranking.rs:411`,
   `scores[i] += RECENCY_BOOST_WEIGHT * freshness`, with the multiplicative
   form deliberately removed (the comment above it explains why: a
   multiplicative decay annihilated old-but-relevant answers). An additive term
   does **not** preserve order under a clock shift — at +5y the freshness term
   shrinks ~32× at the default 365-day half-life and stops being able to
   override rank-position differences.
3. **Truncation happens after reranking** — `brain.rs:2101` fetches
   `k × fetch_mult`, `brain.rs:2147` truncates to `k`. So a boundary flip
   changes the retrieved **set**, not merely its order.

Demonstrated: same brain, same query, `now` advanced 5 years → the k=20 result
set gains `s4` and loses `s22`.

**What is NOT broken:** byte-identical repeat runs at a fixed `now`
(`reproducible_retrieval_is_stable_across_repeated_calls` passes), and the
`recall_*` path, which `recall_at`'s corpus anchor already pins.

**Blocked on:** every candidate fix is a default-path ranking change.
(a) Restore a multiplicative decay — reverts the deliberate fix the comment
documents. (b) Replace the rank-position base with the raw bm25 score —
changes ranking everywhere. (c) Anchor `now` to the corpus on the top-k path
too, as `recall_at` already does — the cheapest and most consistent option,
and the one that matches the README's byte-reproducibility claim, but it
changes output for every caller that passes `now`. All three need a prereg and
an oracle A/B.

**Interim requirement (Rule 5):** the test may be re-baselined to assert what
the path actually guarantees — determinism at a fixed `now`, not order
invariance across clocks — **only** with that justification recorded in the
test itself and this row cited. It may not be deleted, weakened silently, or
`#[ignore]`d.

**Interim requirement SATISFIED 2026-08-07** (with R16). The justification is
recorded in the test file's docstring and cites this row.
`recency_decay_is_order_invariant_in_the_topk_path` →
`topk_additive_recency_reorders_under_a_clock_shift`, which **asserts the drift
rather than tolerating it** (`assert_ne!`) and asserts stability at a fixed
anchor in the same body. If a fix below ever lands, that assertion goes red and
forces this row to be closed deliberately.

**Strengthened evidence that this predates R16.** The disease was originally
inferred from a red test on the R16 branch. It is now demonstrated directly on
`main`, no SQL change present: the same 24 memories with the same timestamps,
inserted shuffled instead of chronologically, reorder across the same 5-year
shift (`s22`/`s1`, `s21`/`s12`). The old test's fixture inserted in
chronological order, which — with an untiebroken `ORDER BY` — made FTS rank
order agree with age order, the one arrangement in which an additive freshness
term provably cannot reorder anything. The property was never held; it was
never tested.
Ref: `research-alignment-2026-08-07.md` §2, `r16-baseline-shift-2026-08-07.md`.

---

## R21 — Judge JSON parse rejects trailing content · DONE (2026-08-08)

**FIXED after the baseline published, deliberately in that order.**
`judge::first_json_object` scans for balanced braces — string-aware, so braces
and `\"` escapes inside `reasoning` do not move the depth counter — and returns
the **first complete** object instead of the span from the first `{` to the
last `}`. Applied at both judge sites (Anthropic and OpenAI-compat).

An unbalanced `}` returns `None` rather than underflowing: a panic here would
kill a multi-hour run, and a judge failure is the safe direction (excluded from
the denominator, never scored wrong). Truncated mid-object responses stay
failures — the fix does not salvage them into a verdict.

Seven tests, including `r21_old_span_extraction_would_have_failed_these`, which
asserts the old approach genuinely breaks on the same inputs so the others
cannot pass vacuously.

**The published 65.02% is NOT re-scored.** The next run that uses this scorer
must say it is running a different scorer than the baseline did.

Original row follows.

---

**Found 2026-08-07 by the BM25 LoCoMo baseline run** (4/1438 questions).

The judge returns a valid JSON object followed by extra prose. The parser
rejects the whole response — `trailing characters at line 3 column 1` — and the
harness records the question as **not correct**.

It is a silent, one-directional scoring bias: **3 of the 4 failures carried the
judge's own `"correct": true`** verdict, visible in the truncated error text.
Parse failures are scored as wrong, so the defect can only push a reported
accuracy **down**, never up. On this run it is worth ~0.2pp (65.02% recorded vs
65.23% if the three visible verdicts were honoured) — inside the interval, but
a real defect and free to fix.

**Fix:** parse the first complete JSON object in the response rather than
requiring the response to be exactly one JSON object — the same tolerance the
actor path already applies via `strip_actor_continuation`.

**Deliberately NOT fixed before publishing the baseline.** Fixing the scorer
after seeing the result and re-running is a re-roll, which the baseline's
prereg forbids. It lands after, and the next run that uses it says so.
Ref: `bm25-locomo-baseline-result-2026-08-07.md`.

---

## R22 — RRF composition · REFUTED (2026-08-09)

**The composition was never the binding constraint.** The failure analysis
named reciprocal rank fusion "the fix we already own" and the highest-value
untested lever, on the argument that additive boosts structurally cannot
promote deep evidence (48 ranks of budget where 59 are needed). The arithmetic
was right. The conclusion drawn from it was not.

Preregistered before any arm ran (`a3b241d`), $0, 250 LoCoMo questions,
`topk_fts`, R19 labels. Primary arm A2 (RRF + declarative) scored **−3.65pp**
evidence-turn micro-recall (p=0.0525) against a prespecified PASS gate of
p<0.05 **and** ≥+2.0pp. A1 (RRF, default channels) is refuted outright at
**−5.90pp, p=0.0004**. The additive control A3 scored **+0.84pp**, reproducing
the previously measured +3 evidence turns — so A2 is **16 evidence turns worse
than the additive composition it was supposed to beat**.

**The mechanism worked and the hypothesis still failed.** RRF promoted the
first evidence turn in 82 questions and rescued 5 questions that had zero
evidence — exactly the class additive boosts provably could not reach. It also
newly broke 17, losing 21 evidence turns to gain 8. Weighting BM25 up (A5)
returns recall to baseline with the most rank movement and almost no damage:
the best thing RRF does is stop being RRF, and the frontier runs monotonically
back toward BM25-only.

This **confirms** the failure analysis §3 (BM25 ranks correctly by its own
criterion; missed evidence has 0.46× query-term overlap) and **refutes** §4–§5.
The residue is not a composition problem. We do not have a signal that
identifies answer-bearing turns, and no arrangement of the signals we have will
manufacture one. Remaining $0 levers are about *acquiring* a signal:
`query_aliases` vocabulary bridging (never tested) and query-conditioned
answer-shape matching.

**Default stays OFF on both paths.** `recall_cascade` — the only path Permagent
calls — was not measured, so nothing here licenses a cascade change.

Two `rrf_fuse` defects were found by audit **before any arm produced a row**
(`b0ed077`): `add_channel` paid channel mass to candidates a signal scores 0,
ordered by the `id` tiebreak (proximity is exactly 0 for 91.8% of non-evidence
turns, so A4 would have been measuring memory-id order), and RRF silently
dropped the entity signal the additive path applies, making an RRF-vs-additive
arm a two-variable change. `rrf_fuse` had shipped with **no tests**; six added
at `59fbdb4`.

The prereg also contradicted itself on retrieval path (Amendment 3): it
specified shape routing while naming G4's k40 arm as the precondition, and
`SPECTRAL_TOPK_DECLARATIVE` is read only on the topk path — under shape routing
the primary arm's single variable would have been inert on ~80% of questions
and produced a meaningless result that looked like a clean null.

**Side observation, R16.** A0 is metric-identical to G4's archived arm but not
byte-identical: 64/181 shared context hashes differ, **63 of them pure
reorderings** of an identical key set, 1 a genuine set change. That is the R16
tiebreak signature, and R16 landed between the runs. R16 was validated 0/500 on
LongMemEval; LoCoMo is the first corpus where it visibly moves ordering. It
does not affect R22 — all six arms share one binary.

Ref: `rrf-composition-result-2026-08-09.md`,
`rrf-composition-prereg-2026-08-08.md`.

---

## R23 — Speaker attribution · NULL (2026-08-09), gate underpowered

**$0, preregistered at `6c2e32a` before implementation.** LoCoMo, 250
questions, `topk_fts`, R19 labels. A0' precondition passed exactly (a full
re-ingest reproduced 231/356, 53 zero-evidence, 0 discordant vs R22's A0), so
arm B is interpretable.

Arm B (turn content prefixed with the speaker's name) scored **+1.69pp**
evidence-turn micro-recall (231 → 237 turns), **p = 0.2500**, discordant 0/3,
against a prespecified gate of p<0.05 **and** ≥+2.0pp. **It fails both clauses
and is recorded as a NULL.**

**The gate could not have passed.** With 3 discordant pairs the smallest
attainable two-sided exact McNemar p is `2 × 0.5³ = 0.25`; significance needed
≥6 discordant pairs one-way. The all-or-nothing "all evidence turns retrieved"
indicator discards most of the signal — arm B gained 6 evidence turns but only
3 questions crossed the threshold. **The prereg was underpowered for the effect
it was built to detect, and that was not checked before running.** The result
stands as preregistered; re-scoring under a statistic chosen after seeing the
data would be the forking path the prereg exists to prevent. The fix (paired
Wilcoxon on per-question evidence-turn counts, power computed in advance)
belongs to the next prereg and is NOT applied retroactively.

**Direction and mechanism both went the right way, uniquely in this series.**
Prespecified mechanism check: retrieved top-40 turns containing the queried
name fell **36.4% → 19.2%**, narrowing the inversion 8.5× → 5.9%; missed
evidence shrank 70 → 62. The dilution risk the prereg warned about did **not**
materialise: 65 promotions vs 30 demotions (RRF churned 71/76), **zero
questions lost full-evidence status**, zero-evidence improved 53 → 51, and
**multi-session improved +2.27pp** — the slice every RRF arm made worse.

Small capture of a large opportunity: prefixing makes the name *present* in the
right turns but also in every other turn by that speaker, so the signal is
admitted rather than made discriminative. That is what **arm C** (speaker as a
separate indexed field) was preregistered to test; it is **deferred, not
dropped**, and is cheaper than first assessed — `memories_fts` already indexes
`key, content, description` separately, so no schema change is needed, only
plumbing `description` through `RememberOpts`.

Ref: `speaker-attribution-result-2026-08-09.md`,
`speaker-attribution-prereg-2026-08-09.md`,
`speaker-attribution-diagnostic-2026-08-09.md`.

---

## R24 — Speaker attribution · **PASS** (2026-08-09)

**The first PASS in this retrieval series, and it replicates.** $0,
preregistered at `aaba5a9` before implementation, full N=1438, `topk_fts`,
model-free.

Evidence-turn micro **59.86% → 62.62% (+2.76pp, +59 turns)**, Wilcoxon on
per-question counts **72 nonzero pairs [+64/−8], p<0.0001**, both prespecified
clauses met with the power floor cleared ~5×. Zero-evidence 357→329. Context
cost **+0.3%**. Multi-session — the weakest slice, and the one every RRF arm
made worse — gained most at **+4.58pp**.

Both preconditions passed; the stronger one validates against the *published*
record: A0″ at full N reproduces R19's corpus figures exactly (59.86% / 68.63%
/ 357).

**Mechanism confirmed, not merely outcome:** retrieved turns containing the
queried name fell 38.4% → 20.8%, cutting the coreference inversion 4.8× → 1.9×.

**Two things went wrong and are recorded:** (a) the prereg predicted arm C
(separate FTS column) would beat arm B′ (prefix into content); they are
**identical on every question's evidence count**, so the dilution argument was
wrong — FTS5 matches across all indexed columns, so a separate column is not a
separate channel, and dilution lives in the *attachment*, not the *placement*.
The choice is a token decision only (+0.3% vs +3.8%). (b) R23's null was
**underpowered, not absent** — same lever, +1.69pp/p=0.25 at N=250 versus
+2.76pp/p<0.0001 at N=1438.

**Bounded by measurement:** does **not** replicate on LongMemEval, because that
corpus has no named speakers — a structural absence, not a failed test. The
no-lexical-bridge failure family does generalize (72.1% there vs 62.9% on
LoCoMo); the specific pathology is ~half as strong (18.9% vs 38.4%).

**Does NOT follow:** no accuracy claim (retrieval only, no end-to-end arm), no
cascade change (`recall_cascade` unmeasured and it is the only path Permagent
calls), bench-scoped implementation, corpus-shaped result. Defaults stay OFF.

Ref: `speaker-field-result-2026-08-09.md`, `speaker-field-prereg-2026-08-09.md`,
`longmemeval-replication-2026-08-09.md`,
`speaker-attribution-diagnostic-2026-08-09.md`.

---

## R26 — Do the N=250 verdicts survive at full N? · IN FLIGHT (2026-08-09)

**A repair of our own record, designed so it can embarrass us.** Preregistered
before running: `full-n-recheck-prereg-2026-08-09.md`.

R24 established that the sample, not the statistic, was the constraint. **Every
retrieval verdict in this series was measured at N=250** — a subset inherited
from G4, never justified, and **~5pp easier than the corpus** it was drawn from
(64.89% vs 59.86%). Those verdicts are published in `MEASURED_RECORD.md` as
measured results, and **at least one is now known to have been wrong**.

Re-tests A3′ (additive declarative, was NULL +0.84pp — **runs first, most likely
to flip**), A2′ (RRF+declarative, R22's primary, was NULL at p=0.0525, the same
just-above-α profile R23 had), and A1′ (RRF, was **REFUTED** −5.90pp p=0.0004),
against the existing full-N A0″ baseline. Gates identical to R22's, so N is the
only variable.

If a verdict flips, `MEASURED_RECORD.md` is corrected with the prominence the
original claim received, and R22's numbers stay put with the correction beside
them — the treatment R19 gave the BM25 baseline.

Ref: `full-n-recheck-prereg-2026-08-09.md`.
