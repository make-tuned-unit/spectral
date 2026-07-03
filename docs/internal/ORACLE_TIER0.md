# Tier-0 Retrieval Oracle — methodology and first results

**Date:** 2026-07-02
**Cost of everything below:** $0.00 (zero LLM calls)

## What this is

A retrieval-only evaluation gate: every retrieval-side change must show a
paired, per-question improvement here before any paid actor/judge run.
Implemented as two subcommands in `spectral-bench-accuracy`:

```bash
# Run (zero LLM calls; ~10-15 min for 500 questions, release build)
spectral-bench-accuracy oracle \
  --dataset ~/spectral-local-bench/longmemeval/longmemeval_s.json \
  --output oracle-rows.jsonl --label baseline

# Paired diff between two configs
spectral-bench-accuracy oracle-diff --baseline a.jsonl --candidate b.jsonl
```

Per question it records: answer-key recall (keys `{sid}:turn:{i}:{role}` in
sessions whose id starts with `answer_`), answer-session recall, 1-based rank
of the first answer key, retrieved-context token estimate (chars/4), and a
blake3 hash of the exact actor context. **Equal context hashes between two
configs mean the actor outcome distribution is identical** — such questions
need no paid replay. The Tier-1 replay set is exactly the changed-context
questions (further narrowed to recall-changed + a control sample).

Retrieval mirrors the published run's routing (shape-routed cascade with
temporal→topk_fts, per-shape K profiles). **Caveat:** the oracle runs
expansion-OFF (no Haiku call); the published 81.5% run was expansion-ON. The
frozen expansion terms from that run were not available on this machine. A
~$0.12 Haiku pass can produce a frozen expansion cache (`--expansion-cache`)
when budget allows.

## Baseline (label=baseline, expansion-OFF, published routing)

| category | n | sess-rec | key-rec | zero-evidence | rank1 | tok mean | tok p95 |
|---|---|---|---|---|---|---|---|
| knowledge-update | 78 | 99.4% | 54.3% | 0 | 1.4 | 13,485 | 21,934 |
| multi-session | 133 | 96.9% | 50.5% | 1 | 2.2 | 17,928 | 25,654 |
| single-session-assistant | 56 | 98.2% | 74.1% | 1 | 2.0 | 11,893 | 18,993 |
| single-session-preference | 30 | 90.0% | 35.9% | 3 | 6.1 | 9,240 | 12,734 |
| single-session-user | 70 | 100.0% | 46.7% | 0 | 1.7 | 10,930 | 20,003 |
| temporal-reasoning | 133 | 94.8% | 48.4% | 3 | 2.2 | 12,729 | 17,557 |
| **TOTAL** | **500** | **96.9%** | **51.8%** | **8** | **2.2** | **13,675** | **23,111** |

Sanity: 96.9% session recall replicates the published 93–97% claim; the 8
zero-evidence questions align with the documented retrieval-starved floor;
SSP shows the worst rank exactly where the published run failed hardest.

Frozen artifacts: `~/spectral-local-bench/oracle-{baseline,porter,spectrogram,cap}.jsonl`.

## Lever verdicts

### 1. FTS5 porter stemming — PASS, promote to Tier 1

`SPECTRAL_FTS_TOKENIZER="porter unicode61"` (or
`SqliteStoreConfig::fts_tokenizer`). Applies when the FTS table is created;
requires re-ingest (`--fresh-brains`).

- **Zero-evidence questions 8 → 4** (fixed: gpt4_f2262a51, 09d032c9,
  gpt4_e061b84g, gpt4_1e4a8aec; introduced: none). Includes documented
  vocabulary-wall case ba358f49 among the 12 session-recall improvements.
- Session recall 96.9% → 97.6% (multi-session 96.9% → 98.9%).
- Net +388 answer keys; key recall 51.8% → 54.7%.
- 12 improved vs 5 regressed. All 5 regressions are temporal-reasoning and
  mild (lost one marginal session of several, first-evidence rank held at 1–2)
  except 9a707b82 (rank 9 → 16). Mechanism: broader matching displaces
  borderline sessions under temporal's fixed K.
- Cost: +591 mean context tokens (+4.3%).
- Unit test: `porter_tokenizer_bridges_plural_queries` ("doctors" → "doctor";
  control shows default tokenizer misses).

**Tier-1 ask:** actor+judge on the 17 recall-changed questions + ~20 controls,
n=3 → ≈ $6–8.

#### 1a. Widen the re-rank candidate pool for porter — PASS Tier 0 (2026-07-02)

`SPECTRAL_TOPK_FETCH_MULT=N` (topk_fts path only; default 1). Fetches N× the
output size as the re-rank candidate pool, then truncates output to the same
size. **Query-time only — no re-ingest.** Rationale: porter's stemming widens
the FTS match set, so true evidence gets pushed below the fixed bm25 `LIMIT`
and evicted before re-ranking ever sees it (verified on 9a707b82: porter
surfaced generic `ultrachat_*` distractors that evicted a true answer turn,
dropping session recall 2/2 → 1/2). A wider fetch pool lets the deterministic
signal-blend re-ranker recover buried high-signal turns.

Temporal-only oracle sweep (133 Q, porter brains, paired vs frozen default
baseline):

| config | sess-rec | regressions | improvements | Δkeys | ctx tok |
|---|---|---|---|---|---|
| porter, mult=1 (control) | 94.8% | 5 | 5 | +40 | 13,276 |
| **porter, mult=3** | **95.6%** | **4** | **6** | **+42** | 13,785 |
| porter, mult=3 + declarative | 95.1% | 6 | 5 | +51 | — |
| porter, mult=5 | 95.6% | 5 | 6 | +40 | 14,138 |

- **Attribution:** default tokenizer + mult=3 is an *exact no-op* (94.8% →
  94.8%, 0 regressions, +3 keys) — narrow default matches already fit the
  top-40. The gain exists *only with porter*, confirming the mechanism.
- mult=3 is the knee: recovers gpt4_7abb270c, adds eac54add, deepens the
  gpt4_e061b84g fix (0.33 → 0.67 session recall); mult=5 over-widens (new
  regression 4dfccbf8). Declarative boost is net-negative (reshuffles rankings,
  re-introduces regressions) — kept off by default.
- **Not free:** output count is fixed at 40 but composition shifts toward the
  promoted (longer) high-signal turns → +509 ctx tokens (+3.8%) on temporal.
- 9a707b82 itself is *not* fully recovered (its evicted turn is a mid-signal
  assistant turn indistinguishable from distractors at query time) — a genuine
  porter-stemming loss deterministic re-ranking can't reach without oracle
  knowledge of answer-session-ness.
- Unit test: `wider_fetch_pool_recovers_buried_high_signal_memory`.

**Tier-1 implication:** ship porter with `mult=3` as one bundle; the widening
changes context composition (like porter and the cap), so it must clear the
same paid actor-replay gate. Fold the mult=3 arm into the porter recall-changed
replay set — no extra questions, the changed-context set already covers it.

### 2. Spectrogram enable — RETIRE from bench path

`SPECTRAL_BENCH_SPECTROGRAM=1` (backlog item 22). **0/500 contexts changed.**
Write-time spectrograms have zero effect on any live retrieval path,
confirming the code audit (sole reader `recall_cross_wing` has no production
callers; `peak_dimensions` loaded and discarded even there). The
wire-or-retire question is settled with data: retire, or wire a reader first
and re-gate here before any paid run.

### 3. Assistant-turn cap (shape-gated ROLE_TOKEN_PROBE) — PASS Tier 0, Tier 1 MANDATORY

`SPECTRAL_ASSISTANT_CAP_FRAC=0.36`, with a 120-char floor and
GeneralRecall-shaped questions exempt (text-classified, not dataset labels —
the original probe regressed assistant-recall ~5pp, hence the gate).

- **Mean context 13,675 → 6,970 tokens (−49%)**; p95 23,111 → 12,487.
- Answer-key sets byte-identical across all 500 (recall held by construction).
- This is the only lever that attacks the dominant real cost (actor input
  tokens, ~$0.05/query) rather than the 169-token memory overhead.
- **Known Tier-0 blind spot applies in full:** truncation changes context
  composition; the original probe lost 5pp SSA at held recall. Tier-1 actor
  replay is mandatory before adopting: sample ~40 of the 461 changed contexts
  weighted toward SSA/KU, n=3 → ≈ $8–10.

### Structurally untestable in this benchmark (documented, no run)

Co-retrieval ranking and ambient-boost context (`focus_wing`,
`recent_activity`) are always zero here: each question gets a fresh brain and
a single query, so co-access counts and recent activity cannot exist. These
are Permagent-live levers. The right instruments are (a) a shared-brain oracle
mode — all 500 haystacks in one brain (~247k memories), queries replayed
sequentially so retrieval events accrue — and (b) replay of real Permagent
session traces. Neither is built yet.

## Combined Tier-1 proposal (when budget lands)

One batched replay: porter recall-changed set + cap sample + overlap controls
≈ 70–80 actor+judge calls × n=3 ≈ **$15**. Pre-registered expectations:
porter flips 2–4 of the 4 zero-evidence-fixed questions to correct with no
temporal losses; cap holds category accuracy within the ±2pp noise band.
If both hold, ship porter default-on (with FTS rebuild migration), cap as an
opt-in cost profile, and re-run the full n=500 bench once (~$28) with both.

## Reproducibility notes

- Brains are deterministic per (dataset, ingest code); `--fresh-brains` after
  any ingest-affecting change, reuse otherwise. ~24 MB/brain, 500 ≈ 9 GB —
  run candidates sequentially with `--no-keep-brains` on disk-constrained
  machines (two concurrent fresh-brain runs exhausted this machine's disk).
- Dataset: HuggingFace `xiaowu0162/longmemeval` → `longmemeval_s` (the repo
  also hosts `longmemeval-cleaned`, a candidate quasi-held-out variant).
