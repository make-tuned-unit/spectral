#!/usr/bin/env bash
# R25 — turn adjacency emission. $0, retrieval-only, full N.
#
# Baseline is R24's existing full-N A0" arm (precondition already passed against
# R19's published corpus figures), so it is NOT re-run.
#
# PRIMARY is ADJ1 vs KMATCH — both spend ~2.62x the baseline token budget. The
# question is whether dialogue adjacency beats simply retrieving more, NOT
# whether more context helps. ADJ1 vs A0" is the flattering secondary.
#
# See docs/internal/turn-adjacency-prereg-2026-08-09.md.
set -euo pipefail

BIN="${BIN:-$(git rev-parse --show-toplevel)/target/release/spectral-bench-accuracy}"
BENCH="${BENCH:-$HOME/spectral-local-bench}"
OUT="${OUT:-$BENCH/adjacency-2026-08-09}"
DS="${DS:-$BENCH/locomo_full_answerable_labelled.json}"
N="${N:-1438}"

mkdir -p "$OUT"; cd "$OUT"

run () {
  local label="$1" k="$2"; shift 2
  if [ -s "${label}.jsonl" ] && [ "$(wc -l < "${label}.jsonl")" -eq "$N" ]; then
    echo "== ${label}: already complete, skipping"; return
  fi
  echo "== ${label}: starting $(date +%H:%M) — k=$k $*"
  rm -f "${label}.jsonl"
  env -u SPECTRAL_RRF -u SPECTRAL_TOPK_DECLARATIVE -u SPECTRAL_TOPK_PROXIMITY \
      -u SPECTRAL_SPEAKER_FIELD -u SPECTRAL_ADJACENCY \
      "$@" \
      "$BIN" oracle --dataset "$DS" --work-dir "./brains-${label}" \
        --output "${label}.jsonl" --label "$label" --max-questions "$N" \
        --retrieval-path topk_fts --max-results "$k" \
        --fresh-brains --no-keep-brains 2>&1 | tail -2
  rm -rf "./brains-${label}"
  echo "== ${label}: done $(date +%H:%M)"
}

# KMATCH first: it is the control the primary comparison depends on, so if
# anything goes wrong it surfaces before the treatment arms are spent.
run kmatch 105
run adj1    40 SPECTRAL_ADJACENCY=1
run adj2    40 SPECTRAL_ADJACENCY=2

echo "== R25 arms complete"; wc -l ./*.jsonl
