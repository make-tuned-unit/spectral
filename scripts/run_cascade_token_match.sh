#!/usr/bin/env bash
# R29 -- the token-matched control on the production cascade path. $0.
#
# Two stages, in order:
#   CAL  -- N=100 equivalence check + k-multiplier calibration sweep
#   FULL -- N=1438 run of the single calibrated arm
#
# The equivalence arm (cal_eq) exists because target/ has vanished mid-session
# four times and retrieval.rs was edited to add SPECTRAL_CASCADE_K_MULT. If the
# rebuilt binary does not reproduce c0's context_hash on the same questions,
# every comparison against the reused R28 arms is invalid and the run must stop.
#
# See docs/internal/cascade-token-match-prereg-2026-08-11.md.
set -euo pipefail

BIN="${BIN:-/Users/jessesharratt/dev/spectral/target/release/spectral-bench-accuracy}"
BENCH="${BENCH:-$HOME/spectral-local-bench}"
DS="${DS:-$BENCH/locomo_full_answerable_labelled.json}"
OUT="${OUT:-$BENCH/token-match-2026-08-11}"
STAGE="${1:-cal}"

mkdir -p "$OUT"; cd "$OUT"

# Halt at 5Gi, not 2Gi: the 2Gi alarm fired once and the very next tick was 0Gi.
disk_guard () {
  local free_gi
  free_gi=$(df -g / | awk 'NR==2 {print $4}')
  if [ "$free_gi" -lt 5 ]; then
    echo "!! disk guard: ${free_gi}Gi free (<5Gi) -- halting" >&2
    exit 1
  fi
}

run () {
  local label="$1" n="$2"; shift 2
  if [ -s "${label}.jsonl" ] && [ "$(wc -l < "${label}.jsonl")" -eq "$n" ]; then
    echo "== ${label}: already complete, skipping"; return
  fi
  disk_guard
  echo "== ${label}: starting $(date +%H:%M) -- N=${n} $*"
  rm -f "${label}.jsonl"
  env -u SPECTRAL_RRF -u SPECTRAL_TOPK_DECLARATIVE -u SPECTRAL_TOPK_PROXIMITY \
      -u SPECTRAL_SPEAKER_FIELD -u SPECTRAL_ADJACENCY \
      -u SPECTRAL_CASCADE_K -u SPECTRAL_CASCADE_K_MULT \
      "$@" \
      "$BIN" oracle --dataset "$DS" --work-dir "./brains-${label}" \
        --output "${label}.jsonl" --label "$label" --max-questions "$n" \
        --retrieval-path cascade \
        --fresh-brains --no-keep-brains 2>&1 | tail -2
  rm -rf "./brains-${label}"
  echo "== ${label}: done $(date +%H:%M)"
}

case "$STAGE" in
  cal)
    run cal_eq  100
    run cal_m15 100 SPECTRAL_CASCADE_K_MULT=1.5
    run cal_m20 100 SPECTRAL_CASCADE_K_MULT=2.0
    run cal_m25 100 SPECTRAL_CASCADE_K_MULT=2.5
    run cal_m30 100 SPECTRAL_CASCADE_K_MULT=3.0
    echo "== calibration arms complete"; wc -l ./cal_*.jsonl
    ;;
  full)
    # M is chosen by scripts/calibrate_token_match.py, per the prereg rule.
    : "${M:?set M to the calibrated multiplier, e.g. M=2.5}"
    run c_kmult 1438 "SPECTRAL_CASCADE_K_MULT=$M"
    echo "== R29 control arm complete"; wc -l ./c_kmult.jsonl
    ;;
  *)
    echo "usage: $0 [cal|full]" >&2; exit 2 ;;
esac
