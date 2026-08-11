#!/usr/bin/env bash
# Track C — per-stage write-path profile. $0, no model calls.
#
# MUST run on an otherwise-idle machine: timing an ingest while another ingest
# is running produces numbers that mean nothing. This script therefore waits on
# the cascade arms by FILE COUNT, not pgrep -- a pgrep whose pattern appears in
# its own command line matches itself and blocks forever, which already cost
# this session one silent stall.
#
# Rebuilds the release binary first: the instrumentation was deliberately
# committed unbuilt so it could not perturb the queued arms.
#
# See docs/internal/ingest-per-event-hypotheses-2026-08-09.md.
set -euo pipefail

REPO=/Users/jessesharratt/dev/spectral
BENCH="$HOME/spectral-local-bench"
LAST="$BENCH/cascade-2026-08-10/c_spk.jsonl"
N=1438

echo "== waiting for cascade arms to finish ($(date +%H:%M))"
while [ "$(wc -l < "$LAST" 2>/dev/null || echo 0)" -lt "$N" ]; do sleep 60; done
echo "== cascade complete, settling 60s for an idle machine"
sleep 60

echo "== rebuilding release binary WITH profiling instrumentation"
cd "$REPO"
cargo build --release -p spectral-bench-accuracy --bin spectral-bench-accuracy 2>&1 | tail -2

OUT="$BENCH/ingest-profile-2026-08-10"
mkdir -p "$OUT"; cd "$OUT"

# 120 questions is ample: each ingests a full haystack (hundreds of turns), so
# the per-stage sample count is in the tens of thousands.
echo "== profiling ingest over 120 questions ($(date +%H:%M))"
SPECTRAL_INGEST_PROFILE=1 "$REPO/target/release/spectral-bench-accuracy" oracle \
  --dataset "$BENCH/locomo_full_answerable_labelled.json" \
  --work-dir ./brains --output profile.jsonl --label profile \
  --max-questions 120 --retrieval-path topk_fts \
  --fresh-brains --no-keep-brains 2>&1 | tail -25
rm -rf ./brains
echo "== profile complete $(date +%H:%M)"
