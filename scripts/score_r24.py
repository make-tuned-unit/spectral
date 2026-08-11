#!/usr/bin/env python3
"""R24 primary scorer — Wilcoxon signed-rank on per-question evidence-turn counts.

R23 used exact McNemar on the all-or-nothing "all evidence turns retrieved"
indicator. With 3 discordant pairs the smallest attainable two-sided p is
2*0.5^3 = 0.25, so it could not have passed at any effect size. This applies the
statistic R24 preregistered instead, and — the part that actually matters —
**always reports the number of nonzero pairs**, because that is the quantity
that made R23 uninterpretable and it was invisible in the old output.

The power rule is fixed in the prereg, not here: fewer than ~15 nonzero pairs is
reported as STILL UNDERPOWERED rather than as a null.

No SciPy dependency: the signed-rank statistic is computed directly and its
p-value by exact enumeration of sign flips for n <= 20, normal approximation
with continuity correction above that.
"""
import argparse
import json
from itertools import product
from math import sqrt, erfc

MIN_NONZERO_PAIRS = 15  # prereg power rule


def load(path):
    rows = {}
    for line in open(path):
        line = line.strip()
        if line:
            r = json.loads(line)
            rows[r["question_id"]] = r
    return {q: r for q, r in rows.items() if (r.get("evidence_turns_total") or 0) > 0}


def ranks(vals):
    """Average ranks, 1-based, ties averaged."""
    order = sorted(range(len(vals)), key=lambda i: vals[i])
    out = [0.0] * len(vals)
    i = 0
    while i < len(order):
        j = i
        while j + 1 < len(order) and vals[order[j + 1]] == vals[order[i]]:
            j += 1
        avg = (i + j) / 2.0 + 1.0
        for k in range(i, j + 1):
            out[order[k]] = avg
        i = j + 1
    return out


def wilcoxon(diffs):
    """Two-sided Wilcoxon signed-rank. Returns (W, p, n_nonzero)."""
    nz = [d for d in diffs if d != 0]
    n = len(nz)
    if n == 0:
        return 0.0, 1.0, 0
    r = ranks([abs(d) for d in nz])
    w_pos = sum(r[i] for i, d in enumerate(nz) if d > 0)
    w_neg = sum(r[i] for i, d in enumerate(nz) if d < 0)
    w = min(w_pos, w_neg)

    if n <= 20:
        # Exact: enumerate every assignment of signs to the observed |ranks|.
        total = 0
        hits = 0
        for signs in product((0, 1), repeat=n):
            s = sum(r[i] for i in range(n) if signs[i])
            total += 1
            if min(s, sum(r) - s) <= w:
                hits += 1
        return w, min(1.0, hits / total), n

    mean = n * (n + 1) / 4.0
    var = n * (n + 1) * (2 * n + 1) / 24.0
    z = (w - mean + 0.5) / sqrt(var)
    return w, min(1.0, erfc(abs(z) / sqrt(2))), n


def summarize(rows):
    tot = sum(r["evidence_turns_total"] for r in rows.values())
    got = sum(r["evidence_turns_retrieved"] for r in rows.values())
    zero = sum(1 for r in rows.values() if r["evidence_turns_retrieved"] == 0)
    return {"n": len(rows), "micro": got / tot, "got": got, "tot": tot, "zero": zero}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--baseline", required=True)
    ap.add_argument("--arm", required=True)
    ap.add_argument("--label", default="arm")
    args = ap.parse_args()

    a, b = load(args.baseline), load(args.arm)
    shared = sorted(set(a) & set(b))
    diffs = [
        b[q]["evidence_turns_retrieved"] - a[q]["evidence_turns_retrieved"] for q in shared
    ]
    sa, sb = summarize({q: a[q] for q in shared}), summarize({q: b[q] for q in shared})
    d_micro = (sb["micro"] - sa["micro"]) * 100
    w, p, n_nz = wilcoxon(diffs)
    pos = sum(1 for d in diffs if d > 0)
    neg = sum(1 for d in diffs if d < 0)

    print(f"\n=== R24 primary: {args.label} vs baseline ===")
    print(f"  paired questions          {len(shared)}")
    print(f"  baseline micro            {sa['micro']*100:.2f}% ({sa['got']}/{sa['tot']})"
          f"  zero-ev {sa['zero']}")
    print(f"  {args.label:<25} {sb['micro']*100:.2f}% ({sb['got']}/{sb['tot']})"
          f"  zero-ev {sb['zero']}")
    print(f"  delta micro               {d_micro:+.2f}pp   ({sb['got']-sa['got']:+d} turns)")
    print(f"  NONZERO PAIRS             {n_nz}   [+{pos} / -{neg}]")
    print(f"  Wilcoxon W                {w:.1f}")
    print(f"  two-sided p               {p:.4f}")

    if n_nz < MIN_NONZERO_PAIRS:
        print(f"\n  VERDICT: STILL UNDERPOWERED — {n_nz} nonzero pairs is below the "
              f"preregistered floor of {MIN_NONZERO_PAIRS}.")
        print("  Reported as underpowered, NOT as a null. This call was fixed in the")
        print("  prereg before the run, not chosen after seeing p.")
    elif p < 0.05 and d_micro >= 2.0:
        print("\n  VERDICT: PASS — p < 0.05 and >= +2.0pp, both prespecified clauses met.")
    elif p < 0.05 and d_micro <= -2.0:
        print("\n  VERDICT: REFUTED — significant decrease.")
    else:
        why = []
        if p >= 0.05:
            why.append(f"p={p:.4f} >= 0.05")
        if abs(d_micro) < 2.0:
            why.append(f"|{d_micro:+.2f}pp| < 2.0pp")
        print(f"\n  VERDICT: NULL — {', '.join(why)}.")


if __name__ == "__main__":
    main()
