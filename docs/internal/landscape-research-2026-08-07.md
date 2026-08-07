# Landscape research — what we're missing (2026-08-07)

Four parallel research sweeps (production systems, academic literature,
deterministic IR, benchmarks/methodology) plus local verification. **Every
claim below that could be checked against this repo or our data WAS
checked** — three agent claims died that way, and one produced a measured
finding bigger than anything the sweep returned.

---

## 1. The finding: retrieval is NOT saturated, and we couldn't see it

Full write-up: `turn-level-evidence-recall-2026-08-07.md`.

LongMemEval ships per-turn `has_answer: true` flags, documented as "used
for turn-level memory recall accuracy evaluation." We never used them.
`oracle::is_answer_key` counts **every turn in an answer session** as an
answer key — 10,960 turns against **896 true evidence turns**, a 12.2×
dilution. So "key-recall 55.6%" measured nothing meaningful, and the
98.1% we quoted is *session* recall, which a 40-turn session satisfies
with the evidence turn absent.

True evidence-turn recall, computed from existing shipped-config oracle
rows: **88.5% micro / 90.5% macro**, with **27/479 questions (5.6%)
retrieving zero evidence**.

| category | recall | zero-evidence Qs |
|---|---:|---:|
| single-session-user | 98.5% | 1 |
| knowledge-update | 97.2% | 0 |
| single-session-assistant | 96.4% | 2 |
| temporal-reasoning | 88.4% | 11 |
| multi-session | 84.4% | 4 |
| **single-session-preference** | **65.9%** | **9 / 30** |

**Consequences.** (a) "Failures are synthesis-bound" survives as a
majority claim but "retrieval is saturated" does not. (b) Every retrieval
lever we refuted — porter, widening, spreading, ACT-R, cascade-k — was
scored on session-recall (saturated by construction) or the diluted
key-recall; a lever fixing preference retrieval would have read as noise
on both. (c) There is now a specific, quantified target: **a third of
preference evidence never reaches the actor.**

---

## 2. Ranked gaps worth acting on

### G0 — A determinism gap under the ranking layer · VERIFIED HERE · FIXED + CORRECTED 2026-08-07

> **Correction notice (2026-08-07).** The gap was real and is now fixed
> (R16 · `r16-baseline-shift-2026-08-07.md`), but **two claims in this
> section were wrong and are struck below**: the "one large tie block"
> story from the IDF clamp, and "identical scores, removes a full sort"
> as a free latency win. Read the strikethroughs, not the original text.

The default FTS path has **no tiebreak**
(`sqlite_store.rs:2018`, `fusion` defaults false):

```sql
ORDER BY bm25(memories_fts, 1.0, 1.0, 0.5) LIMIT ?2
```

The fusion path **does** tiebreak, lexically by id
(`sqlite_store.rs:2069`, `.then(a.0.cmp(&b.0))`). So the two paths
disagree on tie handling, and on the default path *which memories enter
the candidate pool at the LIMIT boundary* is decided by SQLite's chosen
query plan rather than by our code.

~~Ties are not rare: FTS5 clamps non-positive IDF to `1e-6`
(`fts5_aux.c`), so any term in >~40% of documents contributes almost
nothing and documents matching only common terms collapse into one large
tie block — and our query is a pure OR bag of up to 64 terms.~~

**STRUCK — measured false (2026-08-07).** The clamp is real; the
conclusion is not. The tf/doclen factor still varies, so a pure-`"the"`
query returns ~2585 near-zero but **distinct** scores (-2.105206e-06,
-2.105152e-06, -2.103540e-06, …). Across 150 LongMemEval brains, the
LIMIT boundary sits inside a tie block for **0/120** full-question
queries and **0/40** common-term-only queries; single-term queries
straddle in 36/150 (24%) with median block 2, max 5. **Ties are rare and
small.** The measured shift when the tiebreak landed — 10/500 contexts,
9 of them reordering only — is consistent with that, not with large tie
blocks. Do not cite the IDF-clamp story; it will otherwise propagate as
fact. The fix is justified by the paragraph below instead.

**Status 2026-08-07: FIXED.** `, m.id` landed at this site *and* at the
two fusion channel subqueries this section missed (their rank positions
feed RRF). Measured on the merge commit, $0: **10/500 contexts changed**,
all `TopkFts`, 0/333 `Cascade`; 9 reorder-only, 1 one-document swap; every
metric unmoved including R15's evidence-turn recall (793/896 = 88.50%,
zero per-question change). Recorded as a baseline shift, no accuracy
claim. Ref: `r16-baseline-shift-2026-08-07.md`.

**Original honest status: latent, not active.** Our determinism tests pass, which
means SQLite's plan is stable for fixed schema + data + version. But
#238 added memory-id tiebreaks to all five sorts in `ranking.rs` and
**missed the SQL layer beneath them** — so the byte-identical invariant
currently rests on an external, undocumented guarantee that one schema
change, one `ANALYZE`, or one SQLite upgrade can move. Fix is one clause
(`, m.id`); it will shift the current baseline, so it lands behind an
oracle diff and is recorded as a baseline shift, not a null.

~~**Bonus, same check:** `ORDER BY bm25(...)` forces `USE TEMP B-TREE FOR
ORDER BY` — every matching row is scored and sorted before `LIMIT`.
`ORDER BY rank` with weights set via the table's rank config gets FTS5's
internal ordered scan instead. Identical scores, removes a full sort.~~

**STRUCK — REJECTED 2026-08-07.** The scores *are* bit-identical
(verified: `-9.62855697443987` on both forms, equal on every sampled
row). It is not a free win. (i) `ORDER BY rank, m.id` **reintroduces**
the temp B-tree — measured p50 5.29 ms vs 5.06 ms for today's form — so
the latency win and the determinism fix are **mutually exclusive**.
(ii) Untiebroken, `ORDER BY rank` moves the LIMIT boundary into FTS5's
*undocumented internal* result ordering, which is strictly weaker than
SQLite's temp B-tree: a Rule 3 regression sold as a latency win.
(iii) The prize is ~1.3 ms against ~9 ms/question end-to-end. The
persistent rank-config variant is separately unusable: it writes
`memories_fts_config`, so it fails on `read_only` brains and silently
falls back to unweighted `bm25(1,1,1)` on every already-built brain that
lacks the config row.

### G1 — Bi-temporal validity on `memories` (convergent, 3 sources)
Zep/Graphiti, TOKI (arXiv 2606.06240, with soundness theorems), and our
own inspection all land here. Four timestamps: `t_created`/`t_expired`
(system) and `t_valid`/`t_invalid` (world). **Invalidate by writing
`t_invalid`; never delete.** We have `valid_to` on `triple` only;
`memories` has just `created_at`/`updated_at`, so `render.rs` dates
context from ingestion time, conflating "when said" with "when true."

Use the half-open `[from, to)` convention with a
`'9999-12-31T23:59:59.999Z'` **sentinel rather than NULL**, so every
predicate stays an indexable range comparison.

Payoff is not storage — it is that `known_at` in the past answers *"what
would we have answered then,"* making an eval reproducible against a
mutating store. That is the audit moat, made provable. Pure schema, no
model. **Caveat: no public benchmark tests bi-temporal modelling at all
— Zep's claim is currently unfalsifiable — so ship it for auditability,
not for an accuracy claim.**

### G2 — Wire `temporal.rs`, then render valid time
`resolve_relative_dates` exists, is exported, and is wired to nothing.
Resolving at ingest *is* a valid-time field obtainable deterministically.
And `RenderOptions::relative_offsets` is **off by default** — R11's
+14.2pp came from bare dates alone, so the "4 months ago" annotation is
built, untested, and sits on the exact axis that already paid.

Literature agrees on direction: timestamp-marked verbatim chunks score
**50.2%** on temporal questions vs **31.2%** for extracted artifacts
(arXiv 2601.00821). Extend the resolver along **SCATE**'s compositional
operators (interval/offset/period/repeating-interval) rather than
ad-hoc regex — it composes; regex doesn't.

### G3 — Outcome-ledger statistics (we have the only ledger; we score it naively)
Nobody else in the field has a delivered-vs-used ledger. We reinforce
with a flat nudge, which is unusable at small n and rewards exposure.
Deterministic, offline, $0 fix:
- **Wilson lower bound** so 1-of-1 doesn't outrank 90-of-100.
- **Position-bias correction**: estimate empirical use-rate at delivered
  rank `r` and divide it out, else you learn "rank-1 items are good" —
  circular.
- Saturate rich-get-richer (`log10(max(used,1))`).

This plausibly **explains the measured co-retrieval regression** (728/744
events returning the same ~40 memories; top-5 relevance 3–4.5:1 worse) as
textbook position bias plus rich-get-richer — a known defect with a known
deterministic repair. Related: **Rocchio PRF** over the ledger
(`D_relevant` = Used, `D_nonrelevant` = delivered-but-Ignored) is the one
query-side lever we've never tested; the query-conditioned family was
closed on *rerank*, not on *expansion*.

### G4 — Term proximity (the one classic IR signal never tested)
On 10–50-token memories BM25 degenerates: `tf` is almost always 1, so
saturation does nothing and ranking collapses toward `Σ IDF`. Proximity
is what BM25 discards, and FTS5 already stores positions
(`fts5vocab(tbl,'instance')`). `NEAR(a b, N)` is the zero-work crude
version. This is **admission-changing, not just rerank**, so the $0
oracle can measure it — and it sits *beneath* every lever we refuted.

### G5 — Bounded typed memory units (the only untested *axis*)
Memobase's profile slots: `(topic, sub_topic) → content`, bounded count,
read path = recency-ordered SQL + deterministic greedy token-fill.
Retrieval as a problem disappears; cost moves to write time. Given our
own record says recall is high and synthesis is the bottleneck, changing
the *unit* is the axis never tried — the same class of insight that made
R11 our only validated accuracy lever. Strategic decision, not a sprint.

---

## 3. Verified NON-gaps (agent claims that died here)

- **"Spectral uses random UUIDs for `memories.id`"** — FALSE.
  `key_to_id` is `blake3(key)[..8]`; IDs are already content-addressed
  and idempotent across runs.
- **"BM25 `b=0.75` is wrong; if CV < 0.3, `b→0` is a strict win"** — the
  agent's own criterion **fails on our data**: measured CV is **2.44**
  (real brain, n=2585, mean 112.8, sd 275.8) and **1.04** (LongMemEval
  turns, n=20,069). Both far above 0.3, so the cheap win is closed. The
  weaker form survives: high variance means `b` is load-bearing, and
  SQLite hard-codes k1=1.2/b=0.75 with no pragma — tuning it needs a
  custom auxiliary function via `fts5_api` (~80 lines of unsafe). Not
  free, not obviously worth it, no longer a "just do it."
- **Deletion byte-erasure** — already handled more thoroughly than the
  finding: `Brain::vacuum` runs `'optimize'` + truncating WAL checkpoint
  + `VACUUM` + a second checkpoint, with a byte-scan test.
- Bi-temporal 4-timestamp design, sleep-time compute, n-hop BFS, PPR,
  ACT-R, dated-observation formatting — all already in the 2026-07-28
  sweep or closed with measured nulls.

---

## 4. Our design choices, now evidence-backed rather than merely principled

- **BM25 48% vs Mem0 18% vs Zep/Graphiti 7%** on MemoryAgentBench
  FactConsolidation (arXiv 2606.01435). Plain lexical retrieval beats
  both flagship commercial memory products by 3–7× on supersession.
- **Never add a summarization tier**: summarized representations score
  **7.5%** on temporal questions vs 50.2% for timestamped verbatim.
- **Never add LLM-driven consolidation**: it made a frontier model fail
  **54% of problems it had previously solved**, *even consolidating from
  ground truth*; append-only doubled accuracy (arXiv 2605.12978). This is
  the core loop of Mem0/LangMem/most commercial products.
- **Our ACT-R null is corroborated**: an independent architecture got
  +1.6pp and admitted decay wasn't the cause (arXiv 2604.00131). The
  whole cognitive-mechanism family (spreading, Hebbian, spacing) has no
  clean published win; our n=78 p=0.81 refutation is better controlled
  than the papers claiming success.
- **Prereg + held-out is a differentiator**: the survey found *no* vendor
  preregistering or holding out. The only held-out number in the field
  came from a third-party audit.

---

## 5. Benchmark credibility — affects our claims too

- **Our LongMemEval dataset is DEPRECATED.** HF `xiaowu0162/longmemeval`
  is superseded by **`longmemeval-cleaned`** because noisy history
  sessions contained content affecting answer correctness. Every number
  we've published used the noisy version. Re-baselining is cheap and the
  delta is itself publishable.
- **LoCoMo's answer key is 6.4% wrong** (99 errors in 1,540 questions,
  including temporal-reasoning errors), giving a ~93.6% ceiling that
  several vendors already claim to have passed. The standard judge
  **accepts 62.8% of deliberately-wrong-but-topically-adjacent answers**
  — it rewards vagueness, the signature of weak retrieval.
  → **R11 (+14.2pp) likely survives** (well outside the 6.4% band, and
  judge bias favours vague answers while dates make answers *more*
  specific) **but must carry the caveat, and nothing under ~7pp on LoCoMo
  is worth chasing.**
- **Harness dominates system**: the same system scored **58.44 / 65.99 /
  75.14 / 84** on LoCoMo depending who ran it — a 25.6-point spread wider
  than any published gap between systems.
- **N is 10 conversations, not 1,540 questions.** Cluster-robust CIs are
  ~±5.5pp; vendors report ±0.31 (judge stochasticity on fixed items —
  the wrong variance, understating uncertainty ~8×).
- **Full-context often beats the memory systems** in their own papers
  (Mem0: 72.9 vs 68.4; MIRIX: 87.52 vs 85.38).

**Better targets than LoCoMo/LongMemEval:**
1. **ForgetEval / Lethe** (arXiv 2606.15903, MIT, **$0 deterministic
   substring scoring**, 1,385 cases incl. 385 adversarial, six-method
   adapter protocol). Its research question — deterministic primitives vs
   LLM control planes — *is* our thesis. Report the 385 adversarial cases
   separately; the templated 1,000 flatter.
2. **MemoryAgentBench FactConsolidation** — MQuAKE-derived, so stale and
   current facts are **lexically identical** and BM25 has no tiebreaker.
   Isolates supersession from retrieval quality.
3. **MEME deletion+cascade** — cascade sits at **3% across all systems**;
   deletion-dependency reasoning is the clearest unsolved problem
   adjacent to our design.
4. **MemLeak** finding to design against: after "successful" deletion,
   direct probing recovers a fact <1% of the time but **retained
   correlated text recovers it 18.3%**.

**Three open construction opportunities:** no benchmark tests bi-temporal
modelling; **no standalone relative-date-resolution benchmark exists at
all** (and we just shipped a deterministic resolver); nothing scores
audit-log or lineage *correctness*.

---

## 6. Suggested sequence

1. **Add true evidence-turn recall to the oracle** and rename the
   misleading `answer_keys_*`. Free, and every future retrieval claim
   depends on it.
2. **Preference-retrieval prereg** — 65.9% recall, 9/30 zero-evidence, a
   real target for the first time in months.
3. **G3 (ledger statistics)** — offline, $0, and has a prior failure
   (co-retrieval regression) to explain and repair.
4. **G4 (proximity)** via `NEAR`/`fts5vocab`, measured on the $0 oracle.
5. **G1+G2 (bi-temporal + wire temporal.rs)** — ship for auditability and
   reproducibility, not for an accuracy claim.
6. **Re-baseline on `longmemeval-cleaned`**; add ForgetEval as the
   on-thesis public benchmark.
