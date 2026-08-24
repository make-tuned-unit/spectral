# R38/R39 result — enrichment, the spectrogram, TACT and recognition, measured end to end

Measured 2026-08-18/19 against `r38-resonance-enriched-prereg-2026-08-18.md`.
LoCoMo conv‑42 + conv‑48, **369 answerable questions**, per‑turn ingest,
`topk_fts`, all arms 48 memories, $0, same host, same binary. Descriptions
generated locally with `qwen2.5:7b` through the R37 describe pipeline, one map
per (sample, prompt); all generation on the M4.

## 1. The preregistered primary passes — for the wrong reason

| arm | Δ micro evidence‑turn recall vs C | q up/down | Wilcoxon p |
|---|---:|---:|---:|
| S_new (40 FTS + 8 resonant, enriched fp) vs C (48 FTS) | **+3.58pp** | 26/4 | 0.0003 |

By the prereg gate (p < 0.05 and ≥ +2.0pp) that is a PASS. It is
**confounded**: `--descriptions` also puts the description into the FTS index
(`memories_fts` indexes `content, description`), so S_new's ranked 40 are not
C's ranked 40. The prereg did not include a descriptions‑without‑resonance
control. I added one (D arms) as soon as sample 3 showed the gain, and the
decomposition below is the honest reading. The prereg'd primary is reported
as **confounded, not as a spectrogram result**.

## 2. Decomposition (pooled n = 369)

| comparison | Δ | up/down | p | reading |
|---|---:|---:|---:|---|
| **D_old vs C** — current Librarian prompt in FTS, no resonance | **+6.44pp** | 37/1 | <0.0001 | descriptions help retrieval, through FTS |
| **D_new vs C** — R36‑brief prompt in FTS | **+5.72pp** | 33/2 | <0.0001 | …so does the new one |
| **D_old vs D_new** — prompt style, FTS only | +0.72pp | 14/10 | 0.41 | **style does not matter for FTS** |
| **S_new vs D_new** — resonance's 8 vs FTS's next 8, enriched fp | **−2.15pp** | 3/14 | 0.007 | resonance is worse than FTS's tail |
| **S_old vs D_old** — same, old‑prompt fp | −1.43pp | 2/10 | 0.021 | same |
| S0 vs C — resonance, content‑only fp | −2.50pp | 2/14 | 0.003 | worse still without enrichment |

Enrichment makes resonance *less bad* (−2.50 → −2.15/−1.43) — R37's
"separation went positive" was real and this is what it bought — but resonance
never beats simply taking more FTS. **The spectrogram‑as‑retrieval gate closes
on an end‑to‑end test at N = 369, with the enrichment hypothesis actually
tested. R35's retirement stands.**

New‑prompt raw‑fallback on LoCoMo: 36–45% (short conversational turns cannot
honestly yield four terms; the parser floor is `MIN_TERMS = 4`), vs 0.0–0.3%
for the old prompt. Old‑prompt descriptions are longer (mean 224–231 vs
137–144 chars). Neither difference moved FTS recall.

## 3. Why no spectrogram consumer can win here — the representation probe

`scripts/spec_auc.py`: one kept brain per (sample, fingerprint arm); for each
question take its **actual FTS seed** (rank‑1 key of the C row) and ask whether
Euclidean distance in the 6 numeric dimensions ranks gold evidence turns above
non‑gold. Also with a gold turn as seed.

| sample | fingerprints | AUC (FTS seed → gold vs non‑gold) | AUC (gold seed → other gold vs non‑gold) |
|---|---|---:|---:|
| conv‑42 | content | 0.495 (n=173) | 0.523 (n=45) |
| conv‑42 | content+desc (new) | 0.506 | 0.550 |
| conv‑48 | content | 0.423 (n=165) | 0.569 (n=48) |
| conv‑48 | content+desc (new) | 0.457 | 0.549 |

Chance is 0.50. The fingerprint carries essentially no information about which
turns are evidence, whether the seed is FTS's best guess or a gold turn itself,
and enrichment barely moves it. That is a statement about the *representation*:
no re‑rank, expansion or resonance rule built on the seven dimensions can lift
retrieval on this data. This closes the question at a level below the consumer.

## 4. TACT — what enrichment does to the hall (`hall_enrichment_probe`)

`classify_hall` with the seven default rules, three texts per described
memory:

| corpus | content | content+desc | desc only |
|---|---:|---:|---:|
| real brain, 2,712 memories (old‑style descriptions) — fallback `event` | 79.8% | 75.7% | 90.7% |
| LoCoMo conv‑42, 629 turns (new prompt) — fallback `event` | 88.2% | 77.3% | 80.9% |
| … memories that gain a hall from the description | | 4.1% / 11.0% | |
| … content and description disagree (both non‑event) | | 1.8% / 0.6% | |

Prose descriptions recover a hall for 4–11% of memories and almost never
contradict the content. The bottleneck is the rule set, not the text: seven
regexes for personal statements. **The TACT lever is an explicit hall label
from the Librarian (a `HALL:` field or Permagent‑supplied `hall_rules`), not
richer prose for the regexes to fish in.** Whether real halls then lift
retrieval through tier‑1 needs an arm of its own; not run here.

## 5. Recognition — is there ANY enrolment shape enrichment helps?

`recognition_e2e` on the real brain (2,807 enrolled, 300 probed), R37 variant
DBs, plus a **paraphrase probe**: the *other* prompt's description of the same
memory as the stimulus (a genuine paraphrase from a different generation).

| enrolment | exact Rec / top‑1 | head50 | drop30 | **para** Rec / top‑1 | foreign false |
|---|---:|---:|---:|---:|---:|
| content only (production) | 83.7 / 95.3 | 54.0 / 67.7 | 59.3 / 85.0 | 0.0 / **42.3** | 0.0 |
| content+desc concatenated | 83.3 / 93.3 | 50.3 / 66.3 | 55.3 / 82.7 | 0.0 / 47.0 | 0.0 |
| description as 2nd trace (`dual`) | **68.0** / 95.3 | 50.0 / 67.7 | 46.3 / 85.0 | 0.0 / 65.7 | 0.0 |
| union under one id (`enroll_parts`) | 82.0 / 90.3 | 42.0 / 60.7 | **31.7** / 80.0 | 0.0 / **67.0** | 0.0 |
| union, wrapper stripped | 81.7 / 90.3 | 41.0 / 60.0 | 33.7 / 80.7 | 0.0 / 69.0 | 0.0 |
| union, old‑prompt desc; new‑prompt para | 82.0 / 90.3 | 40.3 / 60.0 | 32.7 / 80.0 | 1.7 / **81.0** | 0.0 |
| union, stored desc; new‑prompt para | 82.0 / 90.3 | 41.7 / 60.3 | 32.7 / 79.3 | 3.0 / **88.3** | 0.0 |

Two things are true at once:

- **For content‑shaped re‑encounters every enriched shape is worse**, and the
  union is much worse (drop30 59.3 → 31.7). Not the wire‑format wrapper
  (stripping it changes nothing). It is the same mechanism R35 saw in the
  spectrogram: descriptions *summarise*, so same‑template memories converge on
  shared vocabulary; the union hands each of them the others' shingles,
  containment rises across the cluster and the lead‑margin rule turns
  Recognized into Familiar. Enrichment homogenises; recognition needs
  distinctiveness.
- **For paraphrase‑shaped re‑encounters the description is the only thing that
  helps** — top‑1 42% → 67–88% — but it never clears the identity gate
  (≤ 3% Recognized; 97–100% Familiar). The signal is there; the verdict
  machinery is tuned for the content shape.

Naive `dual` also shows a scoring fact: a second trace becomes its own
memory's runner‑up and trips the margin rule (exact 83.7 → 68.0 with top‑1
unchanged). `enroll_parts` (new, tested) fixes that specific failure and is the
right primitive if a description channel is ever built.

## Decisions

1. **Spectrogram:** closed for retrieval on an end‑to‑end test — resonance is
   worse than FTS's tail under every fingerprint, and the representation has
   no discriminative power for evidence (AUC ≈ chance). The wire stays behind
   `spectrogram-legacy` for experiments; nothing ships to production.
2. **Librarian style:** descriptions matter for FTS (+6pp), style does not
   (0.72pp, p = 0.41). **No change to the Permagent prompt.** The R36 brief is
   withdrawn for retrieval too; the held Permagent PR is dropped.
3. **TACT:** the enrichment lever is an explicit hall label, not prose. Cheap,
   Permagent‑side, and testable with a `--hall-map` arm on the `tact` path.
   Proposed as R40, not run.
4. **Recognition:** production stays content‑only. The path to lifting
   recognition *with* enrichment is a **separate description channel** —
   description features indexed apart from the identity trace, consulted for
   paraphrase‑shaped stimuli, with its own verdict semantics — never merged
   into the content trace. `enroll_parts` is the primitive; the verdict work
   is the open piece. Proposed as R41, not run.

## Errata and operations, stated

- My generation script clobbered its `ARM` argument (an `exec` of harness
  code re‑read `sys.argv`), so the first "old‑prompt" maps were silently the
  new maps and the first D_old/S_old arms ran on an *empty* description map
  (`load_descriptions` returns `{}` for a missing file) — they were identical
  to C/S0, which is how it was caught. Those rows were discarded, the maps
  regenerated, and every old‑prompt number above is from the regenerated maps.
- Local Ollama collided twice with a concurrent llama.cpp split on this 16 GB
  host (empties → runs killed and resumed from checkpoint). All recorded
  descriptions are non‑empty. Overnight local generation on this host must
  avoid 01:50–06:10.

## Reproduce

```
scripts/analyze.py <arm>…                                # in the arm dir
scripts/spec_auc.py                                      # kept probe brains
target/release/examples/hall_enrichment_probe <memory.db>
R37_PARA_DB=<other.db> target/release/examples/recognition_e2e <db> content|enriched|dual|parts|parts_clean <locomo.json> 300
```
Arms, maps, kept brains and logs: `~/spectral-local-bench/r38-resonance-2026-08-18/`,
maps also in `~/spectral-local-bench/r37-librarian-2026-08-18/`.
