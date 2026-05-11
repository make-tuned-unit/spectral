# Phase 2a Bench Result

**Date:** 2026-05-10
**Code SHA:** `7c4fd76` (branch `fix/bench-phase2a-spectrogram-preflight`)
**Dataset:** `longmemeval_s.json` (SHA-256: `08d8dad4...7e117894`, 278 MB)
**Retrieval path:** cascade
**Actor/Judge model:** claude-sonnet-4-6
**Ingestion strategy:** per_turn

## Changes from Phase 1.5 baseline

- `enable_spectrogram: true` — every memory gets a 7-dimension cognitive
  spectrogram computed at ingest time
- time_delta_bucket fix (PR #65) already in ancestry chain — fingerprints
  use computed buckets instead of hardcoded "unknown"

## Pre-flight spot-check

Ingested question `0a995998` (45 sessions, 484 turns):

```
Wing distribution:
  general: 323, alice: 90, apollo: 22, vega: 18, acme: 15,
  travel: 9, charity: 5, polaris: 1, infra: 1

Hall distribution:
  event: 241, preference: 96, fact: 76, advice: 68, discovery: 3

Spectrograms: 484/484 (100%)
Fingerprints: 56543, all same_day (expected — turns share session date)
time_delta_bucket NULL: 0, unknown: 0
```

All pre-flight checks PASS.

## Bench result

**Status:** PENDING — requires ANTHROPIC_API_KEY

Run command:
```bash
ANTHROPIC_API_KEY=<key> cargo run --release --bin spectral-bench-accuracy -- run \
  --dataset /Users/jessesharratt/spectral-local-bench/longmemeval/longmemeval_s.json \
  --retrieval-path cascade \
  --confirm-cost \
  --output docs/internal/phase2a-report.json
```

### Overall accuracy

| Metric | Phase 1.5 | Phase 2a | Delta |
|--------|-----------|----------|-------|
| Overall | 78.0% | **TBD** | TBD |

### Per-category accuracy

| Category | Count | Phase 1.5 | Phase 2a |
|----------|-------|-----------|----------|
| multi-session | 133 | TBD | TBD |
| temporal-reasoning | 133 | TBD | TBD |
| knowledge-update | 78 | TBD | TBD |
| single-session-user | 70 | TBD | TBD |
| single-session-assistant | 56 | TBD | TBD |
| single-session-preference | 30 | TBD | TBD |

## Notes

- time_delta_bucket is 100% `same_day` because all turns within a session
  share the same `created_at` from `haystack_dates`. This is a property of
  the LongMemEval dataset format, not a bug — inter-session fingerprints
  would show varied buckets.
- Wing classification: 67% lands in "general" because LongMemEval content
  doesn't contain domain-specific trigger words. This is expected and
  means TACT fingerprint search mostly falls through to FTS.
