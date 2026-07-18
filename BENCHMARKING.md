# Benchmarking Spectral

This document is the reproducible harness behind every number in the README. The
goal is credibility: pinned inputs, exact commands, and an honest split between
**in-sample** results (retrieval developed against the data) and **held-out**
results (a benchmark the retrieval never saw).

> **The one rule:** in-sample numbers demonstrate the ceiling; held-out numbers
> earn trust. We label every result as one or the other, and we publish the
> held-out numbers even though they are lower.

## 0. Environment

- **Toolchain:** stable Rust (see CI); no nightly features.
- **Determinism:** recall and recognition are deterministic given brain state.
  Note recall auto-reinforces on read (a small `signal_score` nudge), so a brain
  that has been queried is not byte-identical to a fresh one — for reproducible
  retrieval metrics, always pass `--fresh-brains` (below) or open read-only.
- **Known limitation:** the ONNX-based vector-comparison bench links `ort-sys`,
  which has no prebuilt binary for some targets (e.g. Intel macOS). Those two
  crates build on Linux CI; everything else builds everywhere. Test per-crate
  rather than `--workspace` if `ort-sys` fails locally.

Record with every run: `git rev-parse HEAD`, the dataset SHA256, and — for
end-to-end accuracy — the exact actor/judge model IDs.

## 1. Correctness & stability (no dataset, no API key)

```bash
# Per-crate (ort-sys blocks --workspace on some hosts)
cargo test -p spectral-graph  --lib
cargo test -p spectral-ingest --lib
cargo test -p spectral-core --lib && cargo test -p spectral-cascade --lib \
  && cargo test -p spectral-tact --lib

# The end-to-end proof suite — prints observed behavior as evidence
cargo test -p spectral-graph --test e2e_verification -- --nocapture
```

`e2e_verification` drives the public API and captures: deterministic recall,
recognition (familiarity vs novelty), the visibility boundary under spreading,
async-runtime safety, federation provenance/boundary, a concurrency stress test
(8 threads × 40 interleaved read/write ops — no deadlock/corruption), and an
adversarial-query fuzz test (FTS metacharacters, injection, unicode, a
100k-char megatoken — all return cleanly).

## 2. Speed micro-benchmarks (criterion)

```bash
cargo bench --bench retrieval          -p spectral   # recall latency / ingest throughput
cargo bench --bench vector_comparison  -p spectral   # vs BGE-small-en; downloads ~330 MB on first run
```

These measure *speed*, not answer quality — do not cite them as a memory-quality
comparison. Full results: [benches/RESULTS.md](benches/RESULTS.md).

## 3. Retrieval quality — the $0 oracle

The Tier-0 oracle measures retrieval only (no LLM, no API key). Metrics:

- **session-recall** — of the sessions that contain the answer, the fraction
  whose memories the retrieval surfaced. The comparable headline metric.
- **key-recall** — of the answer session's turns (each an "answer key"), the
  fraction retrieved. Stricter, and dataset-dependent (see §4).

```bash
# LongMemEval-S — retrieval was DEVELOPED against this, so results are IN-SAMPLE.
cargo run -p spectral-bench-accuracy --release -- oracle \
  --dataset path/to/longmemeval_s.json \
  --output oracle-rows.jsonl --label lme_s --fresh-brains
# Sample a subset with --max-questions N and/or --categories <comma-list>.
```

**In-sample result (LongMemEval-S, all six memory types, published routing):**

| memory type | session-recall | key-recall |
|---|:-:|:-:|
| single-session-user | 100.0% | 46.3% |
| single-session-assistant | 100.0% | 68.6% |
| single-session-preference | 93.3% | 32.4% |
| knowledge-update | 100.0% | 56.1% |
| multi-session | 98.3% | 51.8% |
| temporal-reasoning | 100.0% | 51.4% |
| **overall** | **98.6%** | **51.1%** |

*In-sample — the retrieval config was tuned against this dataset. It shows the
ceiling, not generalization. For generalization, see §4.*

## 4. Held-out evaluation — LoCoMo

To earn credibility, evaluate on a benchmark the retrieval **never saw**. LoCoMo
(snap-research/locomo) is a separate long-term-conversation QA benchmark; nothing
in Spectral's retrieval was tuned on it.

```bash
# 1. Get LoCoMo and convert it to the oracle's schema (deterministic sample).
curl -sL https://raw.githubusercontent.com/snap-research/locomo/main/data/locomo10.json -o locomo10.json
python scripts/locomo_to_oracle.py locomo10.json locomo_heldout.json --per-cat 40

# 2. Run the same $0 oracle on it.
cargo run -p spectral-bench-accuracy --release -- oracle \
  --dataset locomo_heldout.json --output locomo-heldout.jsonl \
  --label locomo_heldout --fresh-brains
```

The converter (`scripts/locomo_to_oracle.py`) marks the sessions holding each
question's `evidence` turns with the oracle's `answer_` prefix, excludes
adversarial and open-domain questions, and samples deterministically. **Caveat:**
LoCoMo evidence sessions are long (15–30 turns) and every turn counts as an
answer key, so **key-recall is a stricter measure here than on LongMemEval** —
compare **session-recall** across the two.

**Held-out result (LoCoMo, 120 questions, same code & routing, deterministic seed):**

| memory type | session-recall | key-recall (stricter — see caveat) |
|---|:-:|:-:|
| single-session-user | 100.0% | 17.4% |
| temporal-reasoning | 100.0% | 14.7% |
| multi-session | 78.6% | 9.4% |
| **overall (held-out)** | **92.9%** | **13.8%** |

**Session-recall generalizes: 98.6% in-sample → 92.9% held-out** on a benchmark
the retrieval was never tuned on. A ~6pp drop across an entirely different
dataset is strong evidence the deterministic recall isn't overfit to
LongMemEval. The honest weak spot is **multi-session (78.6%)** — multi-hop
questions whose evidence spans several sessions, where recovering *every* answer
session is harder (all 4 zero-recall cases are multi-session).

Key-recall is *not* comparable to §3: LoCoMo's evidence sessions are long and
every turn counts as an answer key, so surfacing the right session recovers only
a fraction of its turns. Read session-recall for the cross-benchmark comparison.

## 5. End-to-end accuracy (needs an actor API key)

Retrieval recall is necessary but not sufficient; the arbiter is whether an actor
answers correctly given the retrieved context. The accuracy harness runs an
actor + judge (temp=0), excluding transport/auth failures.

```bash
export SPECTRAL_ACTOR_API_KEY=...   # your provider key
cargo run -p spectral-bench-accuracy --release -- <eval subcommand> \
  --dataset path/to/dataset.json --actor <model-id> --judge <model-id>
```

Pin and report the **actor and judge model IDs** — accuracy is meaningless
without them. The published 81.5% (LongMemEval-S, Sonnet actor) is **in-sample**
and front-runs an optional Haiku query-expansion call (≈$0.25/1k); recall itself
is LLM-free.

**Held-out end-to-end accuracy (LoCoMo, n=120, claude-sonnet-4-6 actor + judge,
temp 0, 0 quarantined, total eval cost ≈ $1.14 + $0.29 judge):**

| category | accuracy |
|---|:-:|
| single-session-user (single-hop) | 85.0% (34/40) |
| temporal-reasoning | 72.5% (29/40) |
| multi-session (multi-hop) | 40.0% (16/40) |
| **overall (held-out)** | **65.8% (79/120)** |

Same converted subset as §4 (adversarial + open-domain excluded, so not
comparable to full-protocol LoCoMo leaderboard numbers). Failure decomposition:
of the 41 misses, **29 are synthesis** (evidence retrieved, actor answered
wrong) and **12 are retrieval** (evidence incomplete — concentrated in
multi-hop). The honest weak spot is multi-hop; single-hop and temporal are
strong. Run it yourself: `federation_ab`'s sibling flow in §4 plus the `run`
subcommand — the entire held-out accuracy eval reproduces for about **the price
of a coffee (~$1.50)**.

## 5b. Federation accuracy A/B (gates shipping federation)

Does merging a teammate's shared wing change end-to-end accuracy? Same actor,
judge, and questions; the only variable is the merged wing. Harness:
`cargo run -p spectral-bench-accuracy --release --bin federation_ab -- <converted_locomo.json>`.
Each LoCoMo conversation's two speakers become two brains: the user's turns are
private; the teammate's turns are shared into a wing, exported as a pack, and
imported mid-run. Both arms use the real federation recall path
(`recall_scoped`, provenance-tagged, spreading on) — so the A/B also end-to-end
exercises the sync wiring.

**Result (n=30, 10/category, claude-sonnet-4-6 actor+judge, temp 0):**

| category | private-only | federated | net |
|---|:-:|:-:|:-:|
| multi-session (multi-hop) | 40% | **60%** | +2 |
| single-session-user | 50% | **70%** | +2 |
| temporal-reasoning | 70% | 50% | −2 |
| **overall** | **53%** | **60%** | **+2** (fixed 5 / broke 3) |

Reading (n=30 is directional, ±3.3pp/question — not definitive):
- **Federation helps most where solo memory is weakest** — multi-hop, the
  held-out weak spot, gains the most (40→60%): the teammate's turns carry the
  evidence the user's own memories lack.
- **The regression attributes cleanly to displacement, not a design flaw**: all
  3 broken questions are temporal-reasoning, where an avg of 43–61 private
  (dated, own-timeline) memories were displaced from the context by shared ones.
  That is a knob-turn (per-origin context cap / scope routing for temporal
  shapes), exactly what the instrumentation (`f_shared_in_context`,
  `f_private_displaced` per row) was built to isolate.

## 5c. Internal federation / stratified retrieval (single-user, content constant)

Does partitioning ONE user's memory (multi-DB fan-out, or in-DB per-session
stratification) improve multi-session recall? Harness:
`cargo run -p spectral-bench-accuracy --release --bin stratified_ab -- <converted_locomo.json> [--accuracy]`
— three arms over the same corpus and context budget (K=40), LoCoMo
multi-session slice (n=40):

| arm | session-recall | key-recall | zero-recall | accuracy |
|---|:-:|:-:|:-:|:-:|
| monolith (today) | 85.2% | 11.8% | 2 | **35%** |
| sharded (one brain/session + fan-out) | **100.0%** | 7.0% | 0 | — |
| stratified (in-DB round-robin) | **100.0%** | 7.8% | 0 | 28% |

Two findings, one honest verdict:
- **The coverage mechanism works, and multi-DB is unnecessary for it.** Both
  partitioned arms reach perfect session coverage (11 questions improved, 0
  regressed, both total-miss questions recovered), and the in-DB stratified
  variant matches multi-DB sharding — same guarantee, none of the N-brains
  overhead.
- **Blanket stratification does NOT convert to accuracy** (35%→28%, fixed 2 /
  broke 5). The decomposition is clean: both fixes are exactly the
  coverage-deficient questions (session-recall 0.60/0.00 → 1.00); all five
  breaks had *full* monolith coverage already, and thinner per-session depth
  (11.8%→7.8% key-recall) removed supporting detail the actor needed.

Coverage and depth trade against each other at fixed K. The implied lever —
apply stratification **conditionally**, only when the ranked pool is
session-concentrated — would in principle keep both fixes and avoid all five
breaks, but that is a hypothesis derived from this data; it must be validated
on a fresh split before being claimed (see the honesty ledger).

## 6. Honesty ledger (do not delete)

- **In-sample vs held-out** is labeled on every result. Held-out is the number
  that counts externally.
- **Speed ≠ quality.** The 6.8× figure is a latency micro-benchmark.
- **Recognition** is strong at near-duplicate/verbatim, **not** paraphrase
  (measured coverage AUC ≈ 0.55 on paraphrases).
- **What didn't work** is published too — `fetch_mult` null, associative-recall
  did not convert to accuracy on LongMemEval, TACT tiers were retrieval-inferior.
  See `docs/internal/`.

## 7. One-command reproduction

```bash
git rev-parse HEAD                                   # pin the code
sha256sum longmemeval_s.json locomo_heldout.json     # pin the data
cargo test -p spectral-graph --test e2e_verification # correctness
# then §3 (in-sample) and §4 (held-out) as above.
```
