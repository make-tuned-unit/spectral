#!/usr/bin/env bash
# R23 arms. $0 — retrieval-only oracle, zero model calls.
#
# A0' and B need SEPARATE brain sets (arm B changes turn content, so it changes
# ingest). Both use --no-keep-brains and run sequentially, so peak disk is one
# brain set (~5GB) rather than two — this machine has repeatedly hit
# "database or disk is full" running two at once.
#
# See docs/internal/speaker-attribution-prereg-2026-08-09.md.
set -euo pipefail

BIN="${BIN:-/Users/jessesharratt/dev/spectral/target/release/spectral-bench-accuracy}"
BENCH="${BENCH:-$HOME/spectral-local-bench}"
OUT="${OUT:-$BENCH/speaker-2026-08-09}"
N="${N:-250}"

mkdir -p "$OUT"; cd "$OUT"

run () {
  local label="$1" ds="$2"
  if [ -s "${label}.jsonl" ] && [ "$(wc -l < "${label}.jsonl")" -eq "$N" ]; then
    echo "== ${label}: already complete, skipping"; return
  fi
  echo "== ${label}: fresh ingest from $(basename "$ds")"
  rm -f "${label}.jsonl"
  env -u SPECTRAL_RRF -u SPECTRAL_TOPK_DECLARATIVE -u SPECTRAL_TOPK_PROXIMITY \
    "$BIN" oracle \
      --dataset "$ds" \
      --work-dir "./brains-${label}" \
      --output "${label}.jsonl" \
      --label "$label" \
      --max-questions "$N" \
      --retrieval-path topk_fts \
      --fresh-brains --no-keep-brains 2>&1 | tail -3
  rm -rf "./brains-${label}"
}

# A0' first: it is the precondition. If it does not reproduce the R22 baseline
# (231/356, 53 zero-evidence) the run is void and B means nothing.
run a0p "$BENCH/locomo_full_answerable_labelled.json"
run b   "$BENCH/locomo_speaker_prefixed.json"

echo "== R23 arms complete"; wc -l ./*.jsonl
