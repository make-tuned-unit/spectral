# R15 — first-class evidence-turn recall in the oracle (2026-08-07)

**Claim type: instrument.** No accuracy claim, no prereg, no gate, no paid
run. Retrieval code is untouched; this changes what the oracle *reports*, not
what it retrieves.

## The defect

`oracle::is_answer_key` matched on the session-id prefix alone, so every turn
of every `answer_`-prefixed haystack session counted as an "answer key".
LongMemEval-S has **10,960 such turns** against the **896 turns it actually
labels `has_answer: true`** — a **12.2× dilution**. "key-recall 55.6%" was
therefore evidence-*session* turn coverage, and the 98.1% headline was
*session* recall, which a 40-turn session satisfies even when its one evidence
turn is missing. Full analysis:
`turn-level-evidence-recall-2026-08-07.md`.

## What shipped

| area | change |
|---|---|
| `dataset.rs` | `Turn.has_answer: Option<bool>`, `#[serde(default, skip_serializing_if)]` so datasets round-trip byte-identically |
| `ingest.rs` | `memory_key(strategy, sid, turn_idx, role)` — one authority for the key format, used by both the write path and evidence scoring |
| `oracle.rs` | `evidence_keys()`, `score_evidence()`, new row + summary fields, renames with serde aliases, evidence lines in the paired diff, `backfill_evidence()` |
| `main.rs` | `oracle-evidence` subcommand (`--rows`, optional `--baseline`, optional `--out` sidecar) |
| `bin/stratified_ab.rs` | renamed its independent second copy of the diluted computation (`key_recall` → `answer_session_turn_coverage`, column `key-rec` → `as-cov`) |

Field renames, all carrying `#[serde(alias = …)]`:

| was | is |
|---|---|
| `answer_keys_total` | `answer_session_turns_total` |
| `answer_keys_retrieved` | `answer_session_turns_retrieved` |
| `rank_first_answer_key` | `rank_first_answer_session_turn` |

Added: `evidence_turns_total`, `evidence_turns_retrieved`,
`rank_first_evidence_turn`, `evidence_keys_missed`.

## Three places the metric refuses rather than reporting zero

This is the part that matters most, because every one of these would
otherwise fabricate a retrieval catastrophe out of nothing.

1. **Unlabelled questions.** The 21 LongMemEval `_abs` abstention questions
   carry no `has_answer` flag; every LoCoMo-converted set carries none at all.
   `Option<usize>`, and `summarize` excludes them from the micro denominator,
   the macro mean, and the zero-evidence count. Counting them as 0 would drag
   macro recall 90.5% → 86.7% and inflate zero-evidence 27 → 48.
2. **`IngestStrategy::PerSession`.** Every turn collapses to `{sid}:session`,
   so the quantity would be evidence *sessions* while the field name says
   turns. It emits `None`. Shipped config is `PerTurn`.
3. **Read-side shape mismatch (the reviewer's catch).** `RetrievalPath::Graph`
   returns no raw hits, so `extract_keys` falls back to parsing the formatted
   context, whose `--- Session <id>` blocks yield bare session ids.
   Intersecting those with turn-shaped evidence keys gives a silent, total,
   fabricated 0/N. `score_evidence` refuses when the retrieved key set is
   non-empty and contains no `:turn:` key, and `run_oracle` warns on stderr
   with a count. An *empty* retrieved set is a genuine zero and is still
   scored.

Key-format drift is the other silent-zero risk; it is closed by routing both
ingest and evidence scoring through `ingest::memory_key()`, frozen by
`memory_key_format_is_frozen` and checked against a real ingested brain by
`ingested_keys_match_memory_key_helper` and `evidence_keys_are_ingestable_keys`.

## Backfilled numbers — instrument correction, not a result

Produced by `oracle-evidence` from rows already on disk. **Each is the same
retrieval it always was, re-described against a denominator 12.2× smaller and
correctly chosen.** None of these is an improvement over any previously
published coverage figure: no gate, no actor, no comparison was run, and they
must never be printed next to the old 55.6% as if something moved.

| archived run | evidence-turn recall (micro) | zero-evidence |
|---|---|---|
| `r12-baseline` (shipped config) | 793/896 = 88.5%, macro 90.5% over 479 labelled | 27 |
| `r12-actr` | 794/896 = 88.6% | 27 |
| `oracle-baseline` | 749/896 = 83.6% | 35 |
| `oracle-porter` | 783/896 = 87.4% | 34 |
| `oracle-cap` | 749/896 = 83.6% | 35 |
| `oracle-bfs-actr/bfs2` (base 789/896) | 774/896 = 86.4% | 34 |
| `oracle-bfs-actr/actr05` (base 789/896) | 788/896 = 87.9% | 29 |
| `r16-pre` → `r16-post` | 793/896 = 88.5% → 88.5%, per-question delta 0 | 27 → 27 |

Per category on `r12-baseline` (the shipped config):

| category | ev-recall (micro) | macro | zero-evidence |
|---|---|---|---|
| single-session-user | 65/66 | 98.4% | 1 |
| knowledge-update | 140/144 | 97.2% | 0 |
| single-session-assistant | 54/56 | 96.4% | 2 |
| temporal-reasoning | 229/259 | 88.5% | 11 |
| multi-session | 276/327 | 88.1% | 4 |
| **single-session-preference** | **29/44 = 65.9%** | 65.6% | **9 of 30** |

The preference row is the actionable one and the reason R15 was worth doing.

## Where the delta is UNKNOWN — and is left unknown

The reviewer required that no prereg be told what the right metric "would
have shown". Accordingly:

* **Answerability preregs (run 1 / run 2 / run 3)** and the **supersession
  prereg** used key-recall as a gate criterion. Their per-arm oracle row files
  are **not retained** on this machine, so the evidence-turn delta cannot be
  recomputed. It is **unknown**. The published verdicts are not restated,
  defended, or overturned on this basis.
* **LoCoMo k-lever prereg**: the rows *are* retained, but LoCoMo carries no
  `has_answer` labels, so `oracle-evidence` reports `n/a` for both arms and
  prints `evidence metric: UNAVAILABLE on both arms … Delta unknown — not
  zero.` The doc's sentence "key-recall 13.8% was the real signal" names
  precisely the quantity R15 says is not a signal; what the real signal was
  there is not established by this work.

## Determinism

Measured, not asserted:

* Two `oracle` runs over the same 6 questions (fresh brains, then reused
  brains) differ in **exactly one field: `retrieval_wall_ms`**, a wall-clock
  timing that was already non-deterministic by construction. `context_hash`,
  `retrieved_keys` and every evidence field are identical.
* Two `oracle-evidence` runs over `r12-baseline.jsonl` produce a
  **byte-identical** sidecar (sha256
  `919a4e533815c2e227882b2984059e8150fae37ba54ed7f3d209939fa23e5a52`). The
  backfill is a pure function of (labels, `retrieved_keys`);
  `evidence_keys` builds a `BTreeSet` and `evidence_keys_missed` is emitted
  sorted.
* `reuse_brains_produces_identical_context_hash` unchanged and passing.

### Recorded schema shift (rule 6)

Default *behaviour* does not move; the oracle's JSONL *schema* does, and it is
one-directional:

```
removed: answer_keys_total, answer_keys_retrieved, rank_first_answer_key
added:   answer_session_turns_total, answer_session_turns_retrieved,
         rank_first_answer_session_turn, evidence_turns_total,
         evidence_turns_retrieved, rank_first_evidence_turn
         (+ evidence_keys_missed, omitted when empty)
13 fields unchanged
```

`#[serde(alias)]` covers the **read** direction only: every archived row file
(`r12-baseline.jsonl`, `r11-*`, `oracle-porter.jsonl`, `oracle-bfs-actr/*`,
`r16-*`, `locomo-*`) still loads. Rows written by the new binary are **not**
readable by an older checkout, and any local analysis script keyed on
`answer_keys_retrieved` must move to `answer_session_turns_retrieved`. There
are no in-repo consumers outside `oracle.rs`.

## Tests

All $0. Full crate suite: **143 passed, 0 failed, 1 ignored** (the ignored one
is the machine-local full-corpus check, run separately and passing).

| test | pins |
|---|---|
| `ingest::memory_key_format_is_frozen` | the two literal key strings |
| `ingest::ingested_keys_match_memory_key_helper` | helper reproduces what ingest wrote, against a real brain |
| `oracle::evidence_keys_are_ingestable_keys` | anti-drift: every evidence key exists in an ingested brain |
| `oracle::has_answer_false_and_absent_are_not_evidence` | `Some(false)` and `None` both excluded |
| `oracle::unlabeled_question_yields_none` | undefined ≠ `Some(0)` |
| `oracle::per_session_strategy_refuses_the_turn_metric` | `PerSession` → `None` |
| `oracle::score_evidence_refuses_session_shaped_retrieved_keys` | the Graph-path silent-zero guard; empty retrieval still scores as a real 0 |
| `oracle::score_evidence_ranks_and_sorts_deterministically` | rank + sorted `missed` |
| `oracle::summarize_excludes_unlabeled_from_both_means` | macro divides by labelled, not `n` |
| `oracle::archived_rows_deserialize_via_alias` | a verbatim pre-rename line |
| `oracle::archived_row_file_still_loads` | the whole committed archive fixture |
| `oracle::evidence_recall_pinned_on_committed_fixture` | **CI acceptance**: 45 questions, 42 labelled / 3 unlabelled, micro 68/78, macro 91.746%, 1 zero, 35 full |
| `oracle::evidence_recall_reproduces_r15_note` (`#[ignore]`) | **full corpus**: 793/896 micro, 90.5% macro, 27 zero, 409 full, 479/21 labelled, preference 29/44 with 9 zero |
| `oracle::reuse_brains_produces_identical_context_hash` | unchanged — retrieval did not move |

The CI fixture is committed under
`crates/spectral-bench-accuracy/tests/fixtures/r15/` with a README explaining
the deterministic selection rule, the trimming, and why the trimmed dataset
must never be used to run retrieval.

## Deliberately not done

1. **Phase 7 — per-turn labels for the LoCoMo converter.** Deferred to
   register item **R19**. It regenerates the sample files; if the seed,
   `--exclude` lists or the empty-turn filter shift, the R11 held-out set
   stops being the set that was measured. A strip-`has_answer`-and-diff
   byte-equality check is mandatory, not optional, before it lands. The
   converter's docstring and BENCHMARKING §4 are corrected in the meantime so
   nobody reads the LoCoMo coverage number as recall.
2. **Metric-caveat banners on the ~15 archived result/prereg docs.** Not
   applied here: this change was scoped to a file allowlist that covers only
   `BENCHMARKING.md`, `ORACLE_TIER0.md`, `MEASURED_RECORD.md`,
   `REPAIR_REGISTER.md` and `scripts/locomo_to_oracle.py`. The banner below is
   ready to apply verbatim; the historical numbers in those documents **must
   not be rewritten** (rule 5).

```
METRIC CAVEAT (R15, 2026-08-07): "key-recall" in this document is
evidence-SESSION turn coverage — every turn of every `answer_` session, a
~12x-diluted denominator — not evidence-turn recall. See
turn-level-evidence-recall-2026-08-07.md. This note does not assert what the
correct metric would have shown here.
```

Files still needing it: `answerability-prereg-2026-08-02.md`,
`answerability-result-run1/2/3-2026-08-02.md`,
`answerability-prereg-run2/run3-2026-08-02.md`,
`supersession-prereg-2026-08-03.md`, `supersession-result-2026-08-03.md`,
`locomo-k-lever-prereg-2026-08-01.md`, `oracle-bfs-actr-2026-07-28.md` (its
`zero-evid` column is *not* zero-evidence),
`r12-actr-rerun-note-2026-08-06.md`,
`graph-vs-cascade-retrieval-2026-07-14.md`,
`read-path-regex-cache-2026-07-25.md`, `ROLE_TOKEN_PROBE.md`,
`post-hardening-benchmark-2026-07-24.md`, `TIER1_PORTER_WIDEN.md` (+ the
stale comment at `eval.rs:482`), and the ACR family
(`acr-lift-all-memory-types-2026-07-15.md`,
`integrated-recall-architecture-2026-07-15.md`,
`DISPATCH-permagent-associative-recall-2026-07-15.md` — the "+18–40pp
answer-key recall" headline is a diluted-metric number).
`cascade-fetch-mult-lever-2026-07-14.md` and `k-admission-test-2026-07-20.md`
already call the metric "bloated" and should be annotated as the first
sighting of R15 rather than caveated.

3. **Splitting into four commits.** The reviewer asked for
   (1) `Turn.has_answer` + literals, (2) `memory_key` refactor,
   (3) evidence scoring/renames/backfill, (4) docs. The implementing task
   forbade committing, so the work is left in the working tree undivided. The
   boundaries above are clean and the split is mechanical if wanted.

## What this does and does not claim

**Claims:** the oracle now reports the quantity LongMemEval labels; archived
runs can be rescored for $0; the reported figures are reproducible to the
digit and pinned by test.

**Does not claim:** that retrieval improved, that any past verdict was wrong,
or that 88.5% is comparable to any previously published percentage on this
project. The only inference drawn is the one in the R15 note — **retrieval is
not saturated**: 11.5% of evidence turns are missed and 27 questions retrieve
zero evidence, concentrated in `single-session-preference`. Acting on that
requires a preregistered gate, which this change does not contain.
