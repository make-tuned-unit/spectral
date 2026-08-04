# Result — R9 step 1: true-IDF landmark salience vs length proxy (2026-08-04)

Prereg: `r9-idf-prereg-2026-08-04.md` (committed before measurement).

## Verdict: GATE FAILED — the length proxy stands; fts5vocab plumbing does NOT proceed

| regime | baseline AUC | IDF-arm AUC | Δ | gate |
|---|---|---|---|---|
| R1 lexical | 0.9945699 | 0.9945813 | **+0.0000114** | needed ≥ +0.0010 → **FAIL** |
| R2 semantic | 0.9787598 | 0.9787598 | **exactly 0** | (≥ −0.0010: pass, vacuously) |
| R3 adversarial | 0.4874625 | 0.4874625 | **exactly 0** | not gating |

Verdict flips: R1 negatives non-Novel 940→939 (one flip in the right
direction); zero flips anywhere else. Cost: IDF arm within noise of baseline
(enroll 1107 vs 1189 ms, query 2.04 vs 2.09 ms/probe on R1).

Discipline: run twice, byte-identical across runs (all AUCs). No parameter
search performed.

## Why the effect is structurally tiny — the real finding

`extract_landmarks` ranks candidates by salience, **truncates to
`max_peaks` (default 32), then restores document order**. Ranking therefore
changes the landmark SET only when a text has more than 32 candidate tokens
— otherwise it changes nothing at all, because the final order is positional
regardless.

- R2 (MRPC) and R3 (PAWS) are single sentences: never >32 candidates → the
  IDF arm produced **bit-identical scores** (Δ exactly 0.0).
- R1 turns are ≥60 chars but mostly still <32 candidates; only the longest
  turns truncate → Δ = +1.14e-05.

So the R9 concern — "length picks the wrong half of the landmarks" (51%
top-8 overlap vs IDF on the real brain) — is real as a *ranking* statement
but nearly inert as a *behaviour* statement at the shipped config: the
ranking is only load-bearing for long texts, and even there the pair/gram/
minhash channels dominate the score. The 51% overlap number was measured at
top-8; at max_peaks=32 the selected sets mostly coincide.

## STOP-condition audit (prereg required baseline reproduction)

Today's baseline arm did NOT byte-match the published 2026-07-28
`public-bench-results.json` (6th-decimal drift, e.g. R1 0.99456992 vs
0.99457377). Diagnosed and cleared:

- The published JSON was written **Jul 28 21:33**; commit `3183efc`
  (scale-robust verdict calibration, modifies `score.rs`) landed **Jul 29
  08:09** — after that run. The published file predates committed history.
- A clean-HEAD build (git worktree at `9f794fa`, separate target dir)
  reproduces today's working-tree baseline **exactly** on all three regimes
  (0.9945699164636133 / 0.9787597507697176 / 0.4874624509905285).

So the comparison inside this run is valid: both arms ran in one binary on
identical pre-registered splits, and the default path is proven unchanged by
the seam (clean-HEAD parity + the pinned byte-identical test).

## Disposition

- The `TermIdf` seam stays (tested, default `None` proven inert; an engine
  consumer with a real corpus can still inject it).
- `RecognitionEngine::set_term_idf` and `public_bench --idf-arm` stay as the
  measurement apparatus.
- **No fts5vocab production plumbing.** Register row R9 → measurement DONE,
  refuted at shipped config. Re-opening requires new evidence (e.g. a config
  with much smaller max_peaks, or a corpus of long documents where
  truncation is the norm) and a fresh prereg.

Artifacts: `~/spectral-local-bench/recognition-public/r9-idf-run{1,2}.json`,
prereg baselines from `public-bench-results.json` (2026-07-28).
