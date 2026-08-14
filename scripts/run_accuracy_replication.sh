#!/usr/bin/env bash
# R31 -- the conversion question on an instrument that can see. $0, on-device.
#
# R30 was null but its baseline sat at 11.79%, barely above its own
# UNINFORMATIVE floor, and it could not have detected the effect it found. This
# replaces the instrument: single-session-user, where a weak reader lands far
# off the floor, and where adjacency's retrieval gain was VERIFIED equivalent
# (+19.44pp vs multi-session's +19.77pp) before the slice was chosen.
#
# See docs/internal/adjacency-accuracy-replication-prereg-2026-08-11.md.
# This is the LAST slice -- the prereg fixes that stopping rule.
set -euo pipefail

BIN="${BIN:-$(git rev-parse --show-toplevel)/target/release/spectral-bench-accuracy}"
BENCH="${BENCH:-$HOME/spectral-local-bench}"
DS="${DS:-$BENCH/locomo_full_answerable_labelled.json}"
IDS="${IDS:-$(git rev-parse --show-toplevel)/docs/internal/r31-sample-ids.txt}"
OUT="${OUT:-$BENCH/accuracy-repl-2026-08-11}"
MODEL="${MODEL:-qwen25-16k}"

mkdir -p "$OUT"; cd "$OUT"

disk_guard () {
  local free_gi
  free_gi=$(df -g / | awk 'NR==2 {print $4}')
  if [ "$free_gi" -lt 5 ]; then
    echo "!! disk guard: ${free_gi}Gi free (<5Gi) -- halting" >&2
    exit 1
  fi
}

run () {
  local label="$1"; shift
  if [ -s "${label}.json" ]; then
    echo "== ${label}: report exists, skipping"; return
  fi
  disk_guard
  echo "== ${label}: starting $(date +%H:%M) -- $*"
  # The 384-token cap is preregistered and identical on both arms; varying it
  # would compare two actors rather than two retrieval configs.
  env -u SPECTRAL_RRF -u SPECTRAL_TOPK_PROXIMITY -u SPECTRAL_SPEAKER_FIELD \
      -u SPECTRAL_ADJACENCY -u SPECTRAL_CASCADE_K -u SPECTRAL_CASCADE_K_MULT \
      OPENAI_API_KEY=ollama ANTHROPIC_API_KEY=ollama \
      SPECTRAL_ACTOR_MAX_TOKENS=384 \
      "$@" \
      "$BIN" run --dataset "$DS" --work-dir "./work-${label}" \
        --output "${label}.json" --question-id "@${IDS}" \
        --retrieval-path cascade --no-expand-queries \
        --actor-api openai --base-url http://localhost:11434 \
        --actor-model "$MODEL" --judge-model "$MODEL" 2>&1 | tail -12
  rm -rf "./work-${label}"
  echo "== ${label}: done $(date +%H:%M)"
}

run b0                                   # cascade defaults
run b_adj  SPECTRAL_ADJACENCY=1          # the lever
echo "== R31 arms complete"
