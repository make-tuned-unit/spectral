#!/usr/bin/env bash
# R27 — k-admission frontier at full N on LoCoMo. $0, retrieval-only.
#
# k=40 (R24 A0") and k=105 (R25 KMATCH) already exist at full N and are NOT
# re-run. This adds the points that turn two measurements into a curve.
#
# NOT a hypothesis test: evidence recall rises with k almost by construction, so
# there is deliberately no PASS gate. The output is a priced frontier.
# See docs/internal/k-admission-frontier-prereg-2026-08-10.md.
set -euo pipefail

BIN="${BIN:-$(git rev-parse --show-toplevel)/target/release/spectral-bench-accuracy}"
BENCH="${BENCH:-$HOME/spectral-local-bench}"
OUT="${OUT:-$BENCH/k-frontier-2026-08-10}"
DS="${DS:-$BENCH/locomo_full_answerable_labelled.json}"
N="${N:-1438}"

mkdir -p "$OUT"; cd "$OUT"

run () {
  local k="$1" label="k$1"
  if [ -s "${label}.jsonl" ] && [ "$(wc -l < "${label}.jsonl")" -eq "$N" ]; then
    echo "== ${label}: already complete, skipping"; return
  fi
  echo "== ${label}: starting $(date +%H:%M)"
  rm -f "${label}.jsonl"
  env -u SPECTRAL_RRF -u SPECTRAL_TOPK_DECLARATIVE -u SPECTRAL_TOPK_PROXIMITY \
      -u SPECTRAL_SPEAKER_FIELD -u SPECTRAL_ADJACENCY \
      "$BIN" oracle --dataset "$DS" --work-dir "./brains-${label}" \
        --output "${label}.jsonl" --label "$label" --max-questions "$N" \
        --retrieval-path topk_fts --max-results "$k" \
        --fresh-brains --no-keep-brains 2>&1 | tail -2
  rm -rf "./brains-${label}"
  echo "== ${label}: done $(date +%H:%M)"
}

# k=80 first: it is the exact point rejected on LongMemEval, so the falsifiable
# prediction in the prereg (it should NOT reproduce +1.00pp here) resolves early.
run 80
run 60
run 150
run 200

echo "== R27 arms complete"; wc -l ./*.jsonl
