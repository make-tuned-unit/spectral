# Turn-level evidence recall — the metric we never computed (2026-08-07)

$0, from data already on disk. Prompted by a literature sweep: multiple
2026 papers attribute 41–79% of memory-system errors to retrieval, which
contradicted our standing "retrieval is saturated, failures are
synthesis-bound" conclusion. The contradiction is a **metric artifact,
and the artifact is ours.**

## What we were measuring

LongMemEval-S ships a per-turn `has_answer: true` flag, documented as
"used for turn-level memory recall accuracy evaluation." We never used
it. `oracle::is_answer_key` counts **every turn in an answer session** as
an answer key:

```rust
fn is_answer_key(key: &str) -> bool {
    key.split(':').next().map(|sid| sid.starts_with("answer_")).unwrap_or(false)
}
```

Measured over all 500 questions: there are **896 true evidence turns**
(mean 1.79/question) but **10,960 turns in answer sessions**. Our
"key-recall" denominator is therefore **12.2× too large** — true evidence
is 8.2% of what we counted. `key-recall ≈ 55.6%` was never turn-level
evidence recall; it was "fraction of all evidence-session turns
retrieved", a diluted proxy that is not the quantity anyone cares about.

Meanwhile the headline we cited — **98.1% session recall** — only asks
whether the right *session* appeared, which a 40-turn session satisfies
even if the one evidence turn is absent.

## What the real number is

Computed against `has_answer` turns, using the existing shipped-config
oracle rows (`r12-baseline.jsonl`, cascade + shape routing, porter):

| metric | value |
|---|---|
| evidence-turn recall (micro, 793/896) | **88.5%** |
| evidence-turn recall (macro, per-question mean) | **90.5%** |
| questions with ALL evidence turns retrieved | 409/479 = 85.4% |
| **questions with ZERO evidence turns retrieved** | **27/479 = 5.6%** |

(479 questions scored; 21 carry no `has_answer` flag at all.)

Per category — this is where it gets actionable:

| category | n | ev-turns | recall | questions w/ zero evidence |
|---|---:|---:|---:|---:|
| single-session-user | 64 | 66 | 98.5% | 1 |
| knowledge-update | 72 | 144 | 97.2% | 0 |
| single-session-assistant | 56 | 56 | 96.4% | 2 |
| **temporal-reasoning** | 132 | 259 | 88.4% | **11** |
| multi-session | 125 | 327 | 84.4% | 4 |
| **single-session-preference** | 30 | 44 | **65.9%** | **9** |

## What this does and does not overturn

**Does not overturn:** the failure analysis that attributed ~62/91
LongMemEval failures to actor-side synthesis still stands on its own
evidence. 88.5% is high, and most questions do get their evidence.

**Does overturn:** the claim that retrieval is *saturated* and therefore
has no headroom. There is a measured **11.5% evidence-turn miss rate**,
and **27 questions where the answer was never retrievable at all** —
those are retrieval failures presenting as synthesis failures, and no
actor-side or rendering work can fix them.

**The concentrated defect: `single-session-preference` at 65.9%, with
9 of 30 questions retrieving zero evidence.** A third of preference
evidence never reaches the actor. This is the single most localized
retrieval gap the project has ever measured, and it was invisible at
session level (preference sessions are short, so hitting the session was
easy and looked like success).

`temporal-reasoning` has the most zero-evidence questions in absolute
terms (11), which is notable given R11's temporal win came from
rendering: for those 11, rendering cannot help — the evidence isn't there.

## Immediate consequences

1. **`answer_keys_*` in the oracle is misnamed and misleading.** It should
   be renamed to reflect what it measures (evidence-session turn
   coverage) and a true `evidence_turns_*` metric added alongside. Until
   then, no document should cite "key-recall" as evidence about retrieval
   quality.
2. **Retrieval-lever experiments were evaluated against the wrong
   target.** Porter, widening, spreading, ACT-R, cascade-k were all
   scored on session-recall (saturated by construction) or the diluted
   key-recall. A lever that improves preference-question retrieval would
   have looked like noise on both.
3. **A preference-retrieval prereg is now justified** on measured
   evidence rather than intuition — the first retrieval work in months
   with a specific, quantified target.

## Reproduce

```
python3 - <<'EOF'   # against ~/spectral-local-bench/
# build {question_id: {f"{sid}:turn:{i}:{role}"}} from has_answer flags in
# longmemeval/longmemeval_s.json, intersect with retrieved_keys in
# r12-baseline.jsonl
EOF
```
Full script in this session's transcript; to be folded into the oracle as
a first-class metric.
