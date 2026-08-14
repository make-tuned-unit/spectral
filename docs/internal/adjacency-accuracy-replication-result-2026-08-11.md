# R31 — RESULT: retrieval DOES convert to answers, on an instrument that can see

**PASS on the preregistered primary.** $0, fully on-device, ollama
`qwen25-16k` (actor + judge), temp 0, no cloud calls. Prereg:
`adjacency-accuracy-replication-prereg-2026-08-11.md`, registered
2026-08-11 before any arm ran; sample IDs committed at `r31-sample-ids.txt`
(seed 20260811). Scored with `scripts/score_containment.py`, unchanged.

## Result — all three registered metrics, n = 300

| metric | B0 (cascade) | B_ADJ (`SPECTRAL_ADJACENCY=1`) | Δ | test | p |
|---|---:|---:|---:|---|---:|
| **Containment (PRIMARY)** | **42.00%** (126/300) | **48.00%** (144/300) | **+6.00pp** | exact McNemar, 35/17 discordant | **0.0175** |
| Local LLM judge (secondary) | 51.33% (154/300) | 64.00% (192/300) | +12.67pp | exact McNemar, 52/14 | <0.0001 (×3 = <0.0003) |
| Item-level recall (secondary) | 43.33% | 49.31% | +5.97pp | Wilcoxon, 57 changed | 0.0286 (**×3 = 0.0858, n.s.**) |

Wilson 95% CI, primary: B0 [36.55%, 47.65%], B_ADJ [42.41%, 53.64%].

**Instrument check (the whole point of this run):** B0 containment 42.00%
and judge 51.33% both sit mid-band, far from the prereg's 20% floor and 80%
ceiling. **This instrument could see.** R30's could not (11.79% baseline,
barely above a 10% line set too low).

**Cost:** mean actor context 1,437 → 3,266 tokens (2.27×), matching the
prereg's design table (1,438 → 3,264) almost exactly.

## Verdict

Per the fixed rule — *B_ADJ > B0 with p < 0.05 on the primary* → **PASS:
retrieval improvement converts to answers on this slice.** This does not
retract R30's null; it explains it. R30 measured on a floored instrument
and its own writeup said so in advance.

**Multiplicity, as registered:** three metrics were tested. The judge
secondary survives Bonferroni ×3 comfortably; **item-level recall does
not** (0.0286 → 0.0858) and is reported as not significant after
correction. The primary needs no correction.

**Prediction, from the prereg: HIT.** "I expect a small positive, +3 to
+7pp, and I am genuinely unsure whether it clears p < 0.05." Measured
+6.00pp, p = 0.0175 — inside the predicted band, and the stated uncertainty
was the right posture.

## Deviation from the registered protocol — disclosed

**The treatment arm did not run entirely on one host.** The 2026-08-14
crash killed b_adj after 50 questions; 46 clean answers were kept and the
remaining 254 were re-run with the actor/judge served from the *other* Mac
mini over the tailnet, to free RAM on this one. B0 ran entirely on this
host. Same model, byte-identical weights blob
(`sha256-2bada8a745…`, Q4_K_M, `num_ctx` 16384, temp 0).

This is a real deviation from a paired single-host design, and it is not
free: a probe of 5 fixed prompts found **4/5 byte-identical across the two
hosts, 1 divergent** on a longer generation (llama.cpp floating-point
non-determinism). Since containment rewards verbosity, a systematically
more verbose host would inflate the treatment arm. Two checks say it did
not:

| subset | n | B0 | B_ADJ | Δ | p | mean answer chars |
|---|---:|---:|---:|---:|---:|---|
| answered on **this** host | 46 | 45.65% | 50.00% | +4.35pp | 0.7266 (8 discordant, underpowered) | 92 → 110 (1.20×) |
| answered on **mini-2** | 254 | 41.34% | 47.64% | +6.30pp | 0.0226 | 123 → 123 (**0.99×**) |

The effect has the same sign and comparable magnitude on both hosts, and on
the mini-2 subset the treatment arm is **not** more verbose than its own
baseline (0.99×) — so the metric is not being inflated by the host swap.
The 1.20× on the local subset is a same-host comparison and is therefore a
genuine treatment effect (more retrieved context → longer answers), not a
host artifact.

**Residual risk, stated plainly:** a single-host replication would be
strictly cleaner, and this result should be read as PASS-with-a-disclosed-
deviation rather than a pristine paired run. The direction is not in doubt;
the exact magnitude carries more uncertainty than the p-value alone implies.

Also: 1 of 300 b_adj answers hit a judge parse failure (b0 had 3); these
affect only the judge secondary, not the deterministic primary.

## What this does and does not license

**Does:** the retrieval programme's central assumption — that better
evidence retrieval produces better answers — is **supported at $0 on
single-session-user with a local reader.** The conversion question, which
the prereg declared closed at $0 if this run was null, is answered instead.

**Does not:** no default change. `SPECTRAL_ADJACENCY` stays off — this is
one slice, one reader, in-sample, and it costs 2.27× context. It does not
generalise to multi-session (R30's slice, where the instrument was blind)
without a run that can see there. It says nothing about a cloud actor.

**Refs:** `adjacency-accuracy-replication-prereg-2026-08-11.md`,
`adjacency-accuracy-result-2026-08-11.md` (R30),
`handoff-2026-08-12.md`. Arms: `b0.json`, `b_adj.json` (merged from
`b_adj_partial_50.json` + `b_adj_resume.json`) in
`~/spectral-local-bench/accuracy-repl-2026-08-11/`.
