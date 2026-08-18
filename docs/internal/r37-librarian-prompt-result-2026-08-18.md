# R37 result — the Librarian prompt rewrite, and why the question was wrong

Measured 2026-08-18 against `r37-librarian-prompt-prereg-2026-08-18.md`.
Real brain `~/.permagent/brain` (2,807 memories, 2,712 enriched), n = 300
paired (seed 37), `qwen2.5:7b` local, $0, read-only on the brain.

**Preregistered verdict: FAIL** (2 of 4 gates). Then a direct end-to-end
measurement showed the gates were measuring a wire that does not exist, and
that building it would make recognition worse under either prompt.

## 1. The preregistered gates

Same model, same run, same 300 memories; only the prompt differs.

| gate | old prompt (regen) | new prompt (regen) | rule | result |
|---|---:|---:|---|---|
| landmark density change, content → content+desc | −28.1% | **−32.0%** | new > old | **FAIL** |
| spectrogram separation change | −0.7% | **+1.2%** | positive | PASS |
| memories losing a verbatim anchor | 0 | 0 | = 0 | PASS |
| raw-fallback rate | 0.3% | **10.0%** | ≤ old + 5pp | **FAIL** |
| description length, mean chars | 270 | 224 | — | −17% |

`stored` (the descriptions currently in the brain) behaves like `old_regen`
on every line (−29.4% density, −1.1% separation): the brain was written by
this prompt and this model tier.

Why density got *worse* under a prompt written to raise it: recognition stems
and counts each unique stem once. The old prompt "adds landmarks" by emitting
genre words the content lacks (`software`, `development`, `monitoring`) — new
stems, low rarity. The new prompt copies the memory's own identifiers, which
dedupe against the content to zero new landmarks while still adding
characters. The proxy penalises verbatim reuse — but see §2: it was
directionally right anyway.

Why fallback rose 0.3% → 10.0%: the parser floor is `MIN_TERMS = 4`. The old
prompt always clears it by padding with inflections; the new prompt forbids
padding, and roughly one memory in ten (`project_selected`,
`decision_resolved`, one-line events) honestly has only 2–3 distinguishing
terms. That is a parser floor problem, reported as the prereg required, not
absorbed. It is moot given §2.

Spectrogram secondary signals moved the way the brief predicted —
`decision_polarity` variance ×1.59, `entity_density` spread 0.97 vs 0.82,
peak-set change 42.4% (crossing R35's 40%) — but separation is +1.2% against
R35's +25% bar. Enrichment still does not separate the space, and see §2.

## 2. The wire does not exist — and should not

**In production the description never reaches recognition.** All three
enrolment sites in `spectral-graph/src/brain.rs` (`remember` L1975,
`repair_derivations` L3513, the pending-queue drain L3606) enrol
`memory.content` only. `set_description` writes the column and nothing else,
and `enroll` is idempotent per id. So the Librarian, under any prompt, changes
recognition by exactly nothing — the same shape as R35's finding that
`SpectrogramAnalyzer::analyze` never reads `description`. R36 measured
"content+desc" as a hypothetical.

So the real question is whether to *build* the wire. Measured directly with a
new instrument, `spectral-recognition/examples/recognition_e2e` — enrol all
2,807 memories in-memory (content-only, or content+desc where a description
exists), probe with the 300 sample memories' raw content and two degraded
re-encounters (first 50% of tokens; 30% deterministic token dropout), plus 300
foreign LoCoMo utterances. 1.5 s per run.

| enrolment | exact: Recognized / top‑1 | head50: Recognized / top‑1 | drop30: Recognized / top‑1 | foreign false‑Recognized |
|---|---:|---:|---:|---:|
| **content only (production today)** | **83.7% / 95.3%** | **54.0% / 67.7%** | **59.3% / 85.0%** | 0.0% |
| content + stored desc | 82.7 / 92.3 | 45.0 / 65.7 | 50.0 / 82.0 | 0.0% |
| content + old-prompt desc | 82.3 / 91.7 | 47.0 / 66.0 | 50.0 / 82.0 | 0.0% |
| content + new-prompt desc | 83.3 / 93.3 | 50.3 / 66.3 | 55.3 / 82.7 | 0.0% |

Every enrichment arm is worse than raw content on every probe. The current
descriptions cost **−9.0pp** on fragment re-encounters and **−9.3pp** on
dropout re-encounters. The R36-brief prompt roughly halves the damage
(−3.7pp / −4.0pp) — the brief's direction was right — but does not reach
zero. Mechanism: `max_peaks` is 32 per memory; description tokens displace
content peaks, so a re-encounter of the *content* shares fewer pairs with the
enrolled trace. Any text that is not the content dilutes the trace against
re-encounters of the content, and no prompt style escapes that.

The 16.3% Familiar on *exact* content is not enrichment's doing — it is the
baseline: near-duplicate memories (`Automation '…' completed in Nms`) fail the
lead-margin rule against each other. Separate matter.

Caveat stated, not tested: the probes are degraded *content*. A stimulus
phrased in the description's vocabulary (a paraphrase re-encounter) is a
different model and could favour enrichment; that is not how Permagent uses
recognition today, and R1's paraphrase set exists if it ever becomes so.

## Decisions

1. **Recognition stays content-only. Do not build the wire.** This is the
   optimum on this brain, and it is what ships today.
2. **The R36 brief is withdrawn as a recognition/spectrogram matter.** Neither
   engine reads the description, and feeding it to recognition hurts under
   every style measured. The brief's style advice may still be right for the
   surfaces the description *does* feed — FTS (`porter unicode61`, content +
   description) and the `term:`/`cat:` entity annotations — but that needs
   its own measurement with its own ground truth, and this run does not
   supply one. **The prompt PR is not shipped.** The rewritten prompt is
   preserved below for whoever measures FTS.
3. **`recognition_e2e` is the gate for recognition** from here on, not the
   density proxy — it measures verdicts, not stems, and it is $0 and seconds.
4. **Answer to "is recognition working": yes, at the raw-content optimum.**
   On the real brain: exact re-encounter 83.7% Recognized / 95.3% top‑1;
   50% fragment 54.0% / 67.7%; 30% dropout 59.3% / 85.0%; 0/300 false
   Recognized on foreign text. Enrolment coverage is 100% after #293/#294.
   The remaining headroom is the near-duplicate lead-margin failure on exact
   probes, not enrichment.

## Reproduce

```
# arms: scratch harness (Python port of librarian.rs describe path, same
# prompt/options/parser), then:
target/release/examples/enrichment_landmarks <variant.db>
target/release/examples/enrichment_probe     <variant.db>
target/release/examples/recognition_e2e      <variant.db> content|enriched ~/spectral-local-bench/locomo10.json 300
```

Variant DBs are copies of `memory.db` with descriptions NULLed except on the
300 sample ids, so every instrument sees the same memory set.

Operational note: mid-run, a concurrent llama.cpp benchmark on this 16 GB
host made Ollama return empty responses for ~350 consecutive calls; those
records were discarded and regenerated, and the harness now treats an empty
response as a retryable error. Every number above is from non-empty output.

## Appendix — the rewritten prompt (not shipped)

Kept the three-field wire format (`FACTS … Related terms: …. Categories: ….`)
so `annotate_memory` still parses it; changed only what goes in the fields.

```
FACTS: <what this memory settles — outcome, decision, verdict, blocker or
        change — with names, numbers, dates, versions, paths and error codes
        copied verbatim>  (≤25 words, fragments fine, lead with the decision,
        no filler)
TERMS: <4–10 specific terms that set this memory apart: proper nouns,
        names, identifiers, versions, error codes, dates, specific technical
        nouns; verbatim spelling; NO inflected forms — the FTS index is
        porter unicode61; NO generic words>
CATEGORIES: <2–4 concrete subjects — the project, system, person, tool or
        topic — never a genre>
```
Full text with neutral-name examples: scratch `r37/prompt_new.txt`, and the
unshipped branch diff in this session's record.
