#!/usr/bin/env python3
"""R32 scorer: split-half paired comparison, gate on the TEST half only.

Reuses score_r24's Wilcoxon (exact <=20 nonzero pairs, normal approx above).
Split fixed in the prereg: derivation = locomo_{0,2,4,6,8}, test = the rest.
"""
import argparse
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from score_r24 import wilcoxon  # noqa: E402

DERIVATION_CONVS = {"locomo_0", "locomo_2", "locomo_4", "locomo_6", "locomo_8"}


def load(path):
    rows = {}
    for line in open(path):
        line = line.strip()
        if line:
            r = json.loads(line)
            rows[r["question_id"]] = r
    return rows


def half(rows, derivation):
    # Unlabelled rows (no `has_answer` in the source) are excluded from every
    # mean, matching the oracle's own refusal semantics (R15).
    return {
        q: r for q, r in rows.items()
        if r["evidence_turns_total"] is not None
        and (q.rsplit("_", 1)[0] in DERIVATION_CONVS) == derivation
    }


def report(name, base, arm):
    shared = sorted(set(base) & set(arm))
    diffs = [arm[q]["evidence_turns_retrieved"] - base[q]["evidence_turns_retrieved"]
             for q in shared]
    bt = sum(base[q]["evidence_turns_total"] for q in shared)
    bg = sum(base[q]["evidence_turns_retrieved"] for q in shared)
    ag = sum(arm[q]["evidence_turns_retrieved"] for q in shared)
    bz = sum(1 for q in shared if base[q]["evidence_turns_retrieved"] == 0)
    az = sum(1 for q in shared if arm[q]["evidence_turns_retrieved"] == 0)
    d_pp = (ag - bg) / bt * 100
    w, p, n_nz = wilcoxon(diffs)
    pos = sum(1 for d in diffs if d > 0)
    neg = sum(1 for d in diffs if d < 0)
    print(f"\n== {name} ({len(shared)} questions)")
    print(f"   base {bg}/{bt} = {bg/bt*100:.2f}%   arm {ag}/{bt} = {ag/bt*100:.2f}%"
          f"   delta {d_pp:+.2f}pp ({ag-bg:+d} turns)")
    print(f"   zero-evidence {bz} -> {az}")
    print(f"   Wilcoxon: {n_nz} nonzero pairs [{pos} up / {neg} down], p = {p:.4g}")
    return d_pp, p


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--baseline", required=True)
    ap.add_argument("--arm", required=True)
    args = ap.parse_args()

    base, arm = load(args.baseline), load(args.arm)
    report("DERIVATION half (fitted — ceiling-flavoured, no verdict)",
           half(base, True), half(arm, True))
    d, p = report("TEST half (PRIMARY — the gate)", half(base, False), half(arm, False))
    ok = p < 0.05 and d >= 2.0
    print(f"\n   GATE (test half, prereg): p<0.05 AND >=+2.0pp -> "
          f"{'PASS' if ok else 'FAIL'}")


if __name__ == "__main__":
    main()
