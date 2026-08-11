#!/usr/bin/env bash
# R30 -- does adjacency improve ANSWERS? Fully on-device, $0, no cloud calls.
#
# The question every result in this programme has deferred. See
# docs/internal/adjacency-accuracy-prereg-2026-08-11.md.
#
# Arms are sequential by design: ollama serves one model and two concurrent
# arms would contend for the same GPU, making per-question latency (and the
# machine) meaningless.
set -euo pipefail

BIN="${BIN:-/Users/jessesharratt/dev/spectral/target/release/spectral-bench-accuracy}"
BENCH="${BENCH:-$HOME/spectral-local-bench}"
DS="${DS:-$BENCH/locomo_full_answerable_labelled.json}"
OUT="${OUT:-$BENCH/accuracy-2026-08-11}"
MODEL="${MODEL:-qwen25-16k}"
CAT="${CAT:-multi-session}"

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
  # SPECTRAL_ACTOR_MAX_TOKENS is preregistered and identical on every arm:
  # varying it would compare two different actors, not two retrieval configs.
  env -u SPECTRAL_RRF -u SPECTRAL_TOPK_PROXIMITY -u SPECTRAL_SPEAKER_FIELD \
      -u SPECTRAL_ADJACENCY -u SPECTRAL_CASCADE_K -u SPECTRAL_CASCADE_K_MULT \
      OPENAI_API_KEY=ollama ANTHROPIC_API_KEY=ollama \
      SPECTRAL_ACTOR_MAX_TOKENS=384 \
      "$@" \
      "$BIN" run --dataset "$DS" --work-dir "./work-${label}" \
        --output "${label}.json" --categories "$CAT" \
        --retrieval-path cascade --no-expand-queries \
        --actor-api openai --base-url http://localhost:11434 \
        --actor-model "$MODEL" --judge-model "$MODEL" 2>&1 | tail -12
  rm -rf "./work-${label}"
  echo "== ${label}: done $(date +%H:%M)"
}

run a0                                   # cascade defaults
run a_adj  SPECTRAL_ADJACENCY=1          # the lever
echo "== R30 primary arms complete"
