#!/usr/bin/env bash
# R26 — do the N=250 verdicts survive at full N? $0, retrieval-only.
#
# Baseline is the EXISTING A0" arm from R24 (full N, precondition already
# passed against R19's published corpus figures). These arms are retrieval-time
# levers, so they differ from A0" only in the named env var — but each still
# pays a full ingest because brains are streamed away (--no-keep-brains) rather
# than kept, which is what makes full N affordable on a 99%-full disk.
#
# See docs/internal/full-n-recheck-prereg-2026-08-09.md.
set -euo pipefail

BIN="${BIN:-/Users/jessesharratt/dev/spectral/target/release/spectral-bench-accuracy}"
BENCH="${BENCH:-$HOME/spectral-local-bench}"
OUT="${OUT:-$BENCH/full-n-recheck-2026-08-09}"
DS="${DS:-$BENCH/locomo_full_answerable_labelled.json}"
N="${N:-1438}"

mkdir -p "$OUT"; cd "$OUT"

run () {
  local label="$1"; shift
  if [ -s "${label}.jsonl" ] && [ "$(wc -l < "${label}.jsonl")" -eq "$N" ]; then
    echo "== ${label}: already complete, skipping"; return
  fi
  echo "== ${label}: starting $(date +%H:%M) — $*"
  rm -f "${label}.jsonl"
  env -u SPECTRAL_RRF -u SPECTRAL_TOPK_DECLARATIVE -u SPECTRAL_TOPK_PROXIMITY \
      -u SPECTRAL_SPEAKER_FIELD -u SPECTRAL_ADJACENCY \
      "$@" \
      "$BIN" oracle --dataset "$DS" --work-dir "./brains-${label}" \
        --output "${label}.jsonl" --label "$label" --max-questions "$N" \
        --retrieval-path topk_fts --fresh-brains --no-keep-brains 2>&1 | tail -2
  rm -rf "./brains-${label}"
  echo "== ${label}: done $(date +%H:%M)"
}

# A3' first: the arm most likely to flip, so a surprise surfaces early.
run a3p SPECTRAL_TOPK_DECLARATIVE=1
run a2p SPECTRAL_RRF=1 SPECTRAL_TOPK_DECLARATIVE=1
run a1p SPECTRAL_RRF=1

echo "== R26 arms complete"; wc -l ./*.jsonl
