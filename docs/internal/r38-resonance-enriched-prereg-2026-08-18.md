# R38 prereg — does an R36-style description, fed to the spectrogram, help retrieval?

Preregistered 2026-08-18, before any arm was run. Follows R35's stated rule
("separation going positive is the gate to a preregistered retrieval arm"),
which the R36-brief prompt passed in R37 (+1.2%, old prompt −0.7%).

## The claim under test

Spectrogram-as-recall was retired on ORACLE_TIER0 (0/500 contexts changed)
because the default recall path never reads fingerprints; the only consumer,
`recall_cross_wing`, was never in the bench. R35 then found the analyzer never
read `description` either. So "does Librarian enrichment help the spectrogram
help retrieval" has never been measured end to end. This measures it, with the
spectrogram as an explicit *expansion* mechanism and the fingerprint computed
from content+description.

## Mechanism (new, behind `spectrogram-legacy`, this branch)

- `Brain::refingerprint_from_descriptions` — recompute each described memory's
  fingerprint over `content + "\n" + description`. The wire R35 found missing,
  called explicitly by the experiment, not by `set_description`.
- `Brain::resonant_memory_ids(seeds, max, tolerances)` — fingerprints
  resonant with the seeds' fingerprints, all wings, best-first, deterministic.
- Bench `SPECTRAL_RESONANCE_EXPAND=E`: after the `topk_fts` pipeline chooses
  its 40, append up to E resonant memories of the top‑3 ranked hits that
  ranking did not choose. Appends only, never ranks. Default tolerances,
  `min_matching_dimensions = 3`.
- Oracle `--descriptions <map>`: apply the map after fresh ingest, then
  refingerprint when `SPECTRAL_BENCH_SPECTROGRAM` is set.

## Corpus and arms

LoCoMo samples **conv‑42 (`locomo_3_*`, 188 answerable questions, 1,258
role‑split turns)** and **conv‑48 (`locomo_7_*`, 181 questions)**, run as two
per‑sample datasets because session ids collide across samples. Descriptions
generated locally with `qwen2.5:7b` through the same describe pipeline as R37
(mask → `/api/generate` T=0.2/top_p 0.9/num_predict 150 → parser → one retry →
raw fallback), one map per (sample, prompt).

All arms: `--retrieval-path topk_fts`, per_turn, `--fresh-brains`, same host,
same binary. **E = 8.**

| arm | fingerprints | output |
|---|---|---|
| **C** control | none | `--max-results 48` (FTS's next 8) |
| **S0** | content only (`SPECTRAL_BENCH_SPECTROGRAM`) | 40 + 8 resonant |
| **S_new** | content + R36‑brief description | 40 + 8 resonant |
| **S_old** | content + current‑prompt description | 40 + 8 resonant (run if generation time allows; reported either way) |

Every arm hands 48 memories to the metric, so the comparison is which 8 are
worth more: FTS's next 8, or resonance's 8 under each fingerprint.

## Metric and gate (the series' standard)

R15 micro **evidence‑turn recall** (`evidence_turns_retrieved / evidence_turns_total`)
over the labelled questions, plus per‑question paired **Wilcoxon** on
evidence‑turn counts.

- **Primary — "the description helps retrieval through the spectrogram":**
  S_new vs C. PASS requires two‑sided Wilcoxon p < 0.05 **and** ≥ +2.0pp
  micro evidence‑turn recall, on the pooled 369 questions.
- **Secondary — "it is the description doing it":** S_new vs S0, same rule.
- **Tertiary — "the prompt matters":** S_new vs S_old, same rule, if S_old runs.

If S_new fails the primary, the spectrogram gate closes on an end‑to‑end
test at N≈370, and R35's retirement stands with the enrichment hypothesis
now actually tested. If it passes, the next step is full N (all ten samples,
~6.5 h of local generation per prompt) before anything ships — the series'
N=250 → full‑N pattern.

## Fixed in advance

- E = 8, seeds = 3, min_dims = 3, default tolerances. No sweep before the
  primary is read; a sweep afterwards is exploratory and labelled so.
- No prompt edits after reading any arm.
- Reported regardless of outcome. $0, ~1 h generation per (sample, prompt),
  ~10 min per arm.
