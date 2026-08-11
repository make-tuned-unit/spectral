#!/usr/bin/env python3
"""R29 calibration: pick the k-multiplier that matches c_adj's token spend.

Two jobs, both preregistered in
`docs/internal/cascade-token-match-prereg-2026-08-11.md`:

1. **Equivalence check.** `cal_eq` (defaults, rebuilt binary) must reproduce
   `c0`'s `context_hash` on the shared question_ids. `target/` has vanished
   mid-session four times; if the binary drifted, every comparison against the
   reused R28 arms is invalid and nothing downstream can be trusted.

2. **Calibration.** Pick the multiplier whose mean `context_tokens_est` is
   closest to `c_adj`'s mean **over the same question_ids** — subsets differ
   from the full-N mean, so the target must be recomputed on the subset.

Selection is by token distance only. It never looks at recall, so the pick
cannot be steered by the outcome.
"""
import argparse
import json
import sys
from pathlib import Path


def load(path):
    rows = {}
    for line in Path(path).read_text().splitlines():
        if not line.strip():
            continue
        try:
            r = json.loads(line)
        except json.JSONDecodeError:
            # Tolerate only an unterminated tail; anything else shrinks a
            # denominator invisibly.
            if line is Path(path).read_text().splitlines()[-1]:
                continue
            raise
        rows[r["question_id"]] = r
    return rows


def mean_tokens(rows, ids):
    return sum(rows[q]["context_tokens_est"] for q in ids) / len(ids)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--cal-dir", required=True, type=Path)
    ap.add_argument("--c0", required=True, type=Path)
    ap.add_argument("--c-adj", required=True, type=Path)
    args = ap.parse_args()

    c0, c_adj = load(args.c0), load(args.c_adj)
    eq = load(args.cal_dir / "cal_eq.jsonl")
    ids = sorted(set(eq) & set(c0) & set(c_adj))
    if not ids:
        sys.exit("no shared question_ids -- wrong dataset or wrong arms")

    # --- 1. equivalence ---
    diffs = [q for q in ids if eq[q]["context_hash"] != c0[q]["context_hash"]]
    print(f"=== binary equivalence (cal_eq vs c0, n={len(ids)}) ===")
    print(f"  context_hash diffs: {len(diffs)}/{len(ids)}")
    if diffs:
        print("  !! BINARY DRIFTED -- reused R28 arms are NOT comparable. STOP.")
        for q in diffs[:5]:
            print(f"     {q}")
        sys.exit(1)
    print("  OK -- default path is bit-identical, R28 arms are reusable")

    # --- 2. calibration ---
    target = mean_tokens(c_adj, ids)
    base = mean_tokens(c0, ids)
    print(f"\n=== calibration target (over the same {len(ids)} questions) ===")
    print(f"  c0     mean tokens {base:8.1f}")
    print(f"  c_adj  mean tokens {target:8.1f}   ({target / base:.2f}x)")

    print("\n=== sweep ===")
    best = None
    for path in sorted(args.cal_dir.glob("cal_m*.jsonl")):
        mult = float(path.stem.removeprefix("cal_m")) / 10.0
        rows = load(path)
        shared = [q for q in ids if q in rows]
        if len(shared) < len(ids):
            print(f"  m={mult:<4} INCOMPLETE ({len(shared)}/{len(ids)}) -- skipped")
            continue
        mt = mean_tokens(rows, shared)
        dist = abs(mt - target)
        print(f"  m={mult:<4} mean tokens {mt:8.1f}  ({mt / base:.2f}x)  "
              f"off target {mt / target - 1:+6.1%}")
        # Ties go to the lower m, and glob is sorted ascending.
        if best is None or dist < best[2]:
            best = (mult, mt, dist)

    if best is None:
        sys.exit("no complete calibration arm")
    mult, mt, _ = best
    print(f"\n=== PICK: M={mult} (mean {mt:.1f} tok, {mt / target - 1:+.1%} vs target) ===")

    mults = sorted(float(p.stem.removeprefix("cal_m")) / 10.0
                   for p in args.cal_dir.glob("cal_m*.jsonl"))
    if mults and mult in (mults[0], mults[-1]):
        print("  !! endpoint fit -- prereg says EXTEND the sweep, do not accept this")
        sys.exit(2)
    if abs(mt / target - 1) > 0.10:
        print("  !! outside the +/-10% band -- INCONCLUSIVE per prereg")
        sys.exit(3)
    print(f"  run:  M={mult} scripts/run_cascade_token_match.sh full")


if __name__ == "__main__":
    main()
