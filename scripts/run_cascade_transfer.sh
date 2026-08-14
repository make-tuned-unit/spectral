#!/usr/bin/env bash
# R28 — do the topk_fts findings transfer to recall_cascade? $0, retrieval-only.
#
# recall_cascade is the ONLY path Permagent calls. Every result in this series
# was measured on topk_fts, so none of it is known to transfer.
#
# NOTE: no token-matched control. Cascade k is NOT set by --max-results (it
# comes from the question-type profile), so an equal-budget control needs a
# SPECTRAL_CASCADE_K sweep -- a separate prereg. C-ADJ vs C0 is therefore
# COST-UNMATCHED and is reported as such.
#
# See docs/internal/cascade-transfer-prereg-2026-08-10.md.
set -euo pipefail

BIN="${BIN:-$(git rev-parse --show-toplevel)/target/release/spectral-bench-accuracy}"
BENCH="${BENCH:-$HOME/spectral-local-bench}"
OUT="${OUT:-$BENCH/cascade-2026-08-10}"
N="${N:-1438}"

mkdir -p "$OUT"; cd "$OUT"

run () {
  local label="$1" ds="$2"; shift 2
  if [ -s "${label}.jsonl" ] && [ "$(wc -l < "${label}.jsonl")" -eq "$N" ]; then
    echo "== ${label}: already complete, skipping"; return
  fi
  echo "== ${label}: starting $(date +%H:%M) — $*"
  rm -f "${label}.jsonl"
  env -u SPECTRAL_RRF -u SPECTRAL_TOPK_DECLARATIVE -u SPECTRAL_TOPK_PROXIMITY \
      -u SPECTRAL_SPEAKER_FIELD -u SPECTRAL_ADJACENCY -u SPECTRAL_CASCADE_K \
      "$@" \
      "$BIN" oracle --dataset "$ds" --work-dir "./brains-${label}" \
        --output "${label}.jsonl" --label "$label" --max-questions "$N" \
        --retrieval-path cascade \
        --fresh-brains --no-keep-brains 2>&1 | tail -2
  rm -rf "./brains-${label}"
  echo "== ${label}: done $(date +%H:%M)"
}

run c0     "$BENCH/locomo_full_answerable_labelled.json"
run c_adj  "$BENCH/locomo_full_answerable_labelled.json" SPECTRAL_ADJACENCY=1
run c_spk  "$BENCH/locomo_speaker_field.json"            SPECTRAL_SPEAKER_FIELD=1

echo "== R28 arms complete"; wc -l ./*.jsonl
