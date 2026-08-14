#!/usr/bin/env bash
# R24 arms at FULL N (1,438 questions). $0 — retrieval-only, zero model calls.
#
# Each arm is a separate ingest by construction (the arms differ in what gets
# written), so brains are not shared and every arm pays a full ingest. Peak disk
# is ONE brain (~20MB): oracle.rs deletes each brain immediately after its row
# is written, so --no-keep-brains streams cleanup. N was never disk-bound.
#
# See docs/internal/speaker-field-prereg-2026-08-09.md.
set -euo pipefail

BIN="${BIN:-$(git rev-parse --show-toplevel)/target/release/spectral-bench-accuracy}"
BENCH="${BENCH:-$HOME/spectral-local-bench}"
OUT="${OUT:-$BENCH/speaker-field-2026-08-09}"
N="${N:-1438}"

mkdir -p "$OUT"; cd "$OUT"

run () {
  local label="$1" ds="$2"; shift 2
  if [ -s "${label}.jsonl" ] && [ "$(wc -l < "${label}.jsonl")" -eq "$N" ]; then
    echo "== ${label}: already complete, skipping"; return
  fi
  echo "== ${label}: starting $(date +%H:%M) — $* $(basename "$ds")"
  rm -f "${label}.jsonl"
  env -u SPECTRAL_RRF -u SPECTRAL_TOPK_DECLARATIVE -u SPECTRAL_TOPK_PROXIMITY \
      -u SPECTRAL_SPEAKER_FIELD \
      "$@" \
      "$BIN" oracle \
        --dataset "$ds" \
        --work-dir "./brains-${label}" \
        --output "${label}.jsonl" \
        --label "$label" \
        --max-questions "$N" \
        --retrieval-path topk_fts \
        --fresh-brains --no-keep-brains 2>&1 | tail -2
  rm -rf "./brains-${label}"
  echo "== ${label}: done $(date +%H:%M)"
}

# A0" first: its first 250 rows must reproduce the R22/R23 baseline.
run a0pp "$BENCH/locomo_full_answerable_labelled.json"
run c    "$BENCH/locomo_speaker_field.json" SPECTRAL_SPEAKER_FIELD=1
run bp   "$BENCH/locomo_speaker_prefixed_full.json"

echo "== R24 arms complete"; wc -l ./*.jsonl
