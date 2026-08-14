#!/usr/bin/env bash
# R22 arms. $0 — retrieval-only oracle, zero model calls.
#
# Every arm reuses the SAME brains (built once by A0 with --fresh-brains) and
# differs from every other arm in exactly the one prespecified variable. See
# docs/internal/rrf-composition-prereg-2026-08-08.md.
set -euo pipefail

BIN="${BIN:-$(git rev-parse --show-toplevel)/target/release/spectral-bench-accuracy}"
DS="${DS:-$HOME/spectral-local-bench/locomo_full_answerable_labelled.json}"
OUT="${OUT:-$HOME/spectral-local-bench/rrf-2026-08-08}"
N="${N:-250}"

cd "$OUT"

run () {
  local label="$1"; shift
  if [ -s "${label}.jsonl" ] && [ "$(wc -l < "${label}.jsonl")" -eq "$N" ]; then
    echo "== ${label}: already complete, skipping"
    return
  fi
  echo "== ${label}: $*"
  # `env -i`-style isolation is not used deliberately: PATH/HOME are needed.
  # Instead every lever is unset first, so a stale export cannot leak into an
  # arm and silently make it a two-variable change.
  env -u SPECTRAL_RRF \
      -u SPECTRAL_TOPK_DECLARATIVE \
      -u SPECTRAL_TOPK_PROXIMITY \
      -u SPECTRAL_RRF_BM25_W \
      -u SPECTRAL_RRF_DECLARATIVE_W \
      -u SPECTRAL_RRF_PROXIMITY_W \
      -u SPECTRAL_RRF_RECENCY_W \
      -u SPECTRAL_RRF_SIGNAL_W \
      -u SPECTRAL_RRF_ENTITY_W \
      "$@" \
      "$BIN" oracle \
        --dataset "$DS" \
        --work-dir ./brains \
        --output "${label}.jsonl" \
        --label "$label" \
        --max-questions "$N" \
        --retrieval-path topk_fts \
      2>&1 | tail -3
}

# Brains are ingest artefacts and do not depend on the retrieval path, so a
# partial set is reused and only the missing ones are built (oracle.rs:
# `reused = reuse_brains && brain_dir/memory.db exists`). No --fresh-brains:
# the ingest-affecting config has not changed, and forcing a rebuild here would
# discard a correct brain set for no reason.

run a0
run a1 SPECTRAL_RRF=1
run a2 SPECTRAL_RRF=1 SPECTRAL_TOPK_DECLARATIVE=1
run a3 SPECTRAL_TOPK_DECLARATIVE=1
run a4 SPECTRAL_RRF=1 SPECTRAL_TOPK_DECLARATIVE=1 SPECTRAL_TOPK_PROXIMITY=0.15
run a5 SPECTRAL_RRF=1 SPECTRAL_TOPK_DECLARATIVE=1 SPECTRAL_RRF_BM25_W=3

echo "== all arms complete"
wc -l ./*.jsonl
