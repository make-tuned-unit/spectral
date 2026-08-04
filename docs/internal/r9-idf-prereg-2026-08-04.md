# Prereg — R9 step 1: true-IDF landmark salience vs length proxy (2026-08-04)

Committed BEFORE any measurement. Register row: R9 (`REPAIR_REGISTER.md`).

## Question

Landmark extraction ranks non-anchor tokens by **length** as an IDF proxy.
Measured on the real brain: Spearman(length, true IDF) = 0.275; top-8 landmark
overlap vs IDF-ranked = 51%. The `TermIdf` seam exists
(`extract_landmarks_with`, `MapIdf`, implemented 2026-08-04, `None` pinned
byte-identical). Does ranking by **true corpus rarity** improve recognition
AUC at all?

## Design

Two arms in one `public_bench` binary run, identical pre-registered splits
(prereg 2026-07-28), identical `eval::roc_auc`:

- **Baseline** — engine as shipped (`idf = None`, length proxy).
- **IDF arm** — `MapIdf::from_corpus(<regime's enrolled texts>)` supplied to
  BOTH enroll-time and probe-time extraction. Corpus statistics come from
  enrolled texts only — no probe text enters the map, so no label leakage.
  This mirrors production, where `fts5vocab` over `memories_fts` reflects the
  enrolled store.

Datasets (already local, $0):
- R1: `~/spectral-local-bench/longmemeval/longmemeval_s.json` (r1-limit 10000)
- R2: `~/spectral-local-bench/recognition-public/mrpc_test.jsonl`
- R3: `~/spectral-local-bench/recognition-public/paws_test.jsonl`

## Pre-specified baselines (from `public-bench-results.json`, 2026-07-28 run)

| regime | engine AUC | minhash128 AUC |
|---|---|---|
| R1 lexical | 0.99457 | 0.99883 |
| R2 semantic | 0.97876 | 0.96681 |
| R3 adversarial | 0.48746 | 0.49173 |

The baseline arm in this run must reproduce these engine numbers exactly
(deterministic engine, identical splits). If it does not, STOP — the working
tree has drifted and the comparison is invalid.

## Gate — decides whether step 2 (fts5vocab production plumbing) proceeds

The comparison is paired and deterministic (same instances, same scorer), so
any nonzero delta is real for THIS corpus; the gate guards practical
significance, not noise:

- **PROCEED** iff R1 ΔAUC (IDF − baseline) ≥ **+0.0010** (≈19% of the 0.0054
  remaining R1 headroom) AND R2 ΔAUC ≥ **−0.0010** (no material semantic
  regression).
- R3 is reported, not gating — the adversarial regime is a known failure for
  every lexical system in the matrix.
- Secondary observables (reported, not gating): verdict flips
  (`pos_novel`, `neg_familiar`), enroll/query cost per arm.
- Any other outcome → R9 stays open at "seam only"; the length proxy stands
  and the register records the refutation.

## Discipline

- Run warm (second consecutive run of the same binary), run twice; the two
  runs must be byte-identical on all AUCs (engine is deterministic).
- No parameter search. One IDF construction (`MapIdf::from_corpus`), one run,
  gate decides. If the gate fails there is no "try a blend" follow-up without
  a new prereg.
