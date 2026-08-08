#!/usr/bin/env python3
"""Paired McNemar test for a two-arm accuracy A/B.

The gate for any accuracy claim in this project is significance, not effect
size — the distinction PR #239 got wrong in the other direction. This computes
the exact binomial McNemar test on the discordant pairs, which is the right
test for paired binary outcomes and does not rely on a normal approximation
that a handful of discordant pairs would not support.

Reports the concordant cells too. A large accuracy delta built on three
discordant pairs is not a result, and printing only the delta hides that.
"""
import argparse
import json
from math import comb


def load(path):
    d = json.load(open(path))
    rows = d.get("results", [])
    return d, {r["question_id"]: bool(r["correct"]) for r in rows}


def exact_two_sided_p(b, c):
    """Exact binomial McNemar. b, c are the discordant counts."""
    n = b + c
    if n == 0:
        return 1.0
    # P(X = k) under H0: p = 0.5
    def pmf(k):
        return comb(n, k) * 0.5**n
    obs = pmf(min(b, c))
    # Two-sided: sum all outcomes at least as extreme as observed.
    return min(1.0, sum(pmf(k) for k in range(n + 1) if pmf(k) <= obs + 1e-12))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--arm-a", required=True, help="control report JSON")
    ap.add_argument("--arm-b", required=True, help="treatment report JSON")
    ap.add_argument("--label-a", default="A")
    ap.add_argument("--label-b", default="B")
    args = ap.parse_args()

    da, a = load(args.arm_a)
    db, b = load(args.arm_b)

    shared = sorted(set(a) & set(b))
    only_a, only_b = set(a) - set(b), set(b) - set(a)
    if only_a or only_b:
        print(f"WARNING: unpaired questions — {len(only_a)} only in {args.label_a}, "
              f"{len(only_b)} only in {args.label_b}. Scoring the {len(shared)} paired ones.")
    if not shared:
        raise SystemExit("no paired questions")

    both = sum(1 for q in shared if a[q] and b[q])
    neither = sum(1 for q in shared if not a[q] and not b[q])
    a_only = sum(1 for q in shared if a[q] and not b[q])   # B broke it
    b_only = sum(1 for q in shared if b[q] and not a[q])   # B fixed it

    acc_a = sum(1 for q in shared if a[q]) / len(shared)
    acc_b = sum(1 for q in shared if b[q]) / len(shared)

    print(f"paired questions      {len(shared)}")
    print(f"{args.label_a} accuracy           {acc_a*100:.2f}%  ({sum(1 for q in shared if a[q])}/{len(shared)})")
    print(f"{args.label_b} accuracy           {acc_b*100:.2f}%  ({sum(1 for q in shared if b[q])}/{len(shared)})")
    print(f"delta                 {(acc_b-acc_a)*100:+.2f}pp")
    print()
    print("CONTINGENCY (the numbers the p-value actually rests on)")
    print(f"  both correct        {both}")
    print(f"  both wrong          {neither}")
    print(f"  {args.label_b} FIXED (A wrong, B right)   {b_only}")
    print(f"  {args.label_b} BROKE (A right, B wrong)   {a_only}")
    print(f"  discordant pairs    {b_only + a_only}")

    p = exact_two_sided_p(b_only, a_only)
    print()
    print(f"McNemar exact two-sided p = {p:.4f}")
    if p < 0.05 and b_only > a_only:
        print(f"VERDICT: PASS — {args.label_b} significantly better (p < 0.05)")
    elif p < 0.05 and a_only > b_only:
        print(f"VERDICT: REGRESSION — {args.label_b} significantly WORSE (p < 0.05)")
    else:
        print("VERDICT: NULL — not significant at p < 0.05; lever stays off")

    for d, lbl in ((da, args.label_a), (db, args.label_b)):
        e = d.get("efficiency", {})
        sp = e.get("total_system_cost_usd", 0) + e.get("total_judge_cost_usd", 0)
        print(f"\n{lbl}: spend ${sp:.2f}  mean ctx tokens {e.get('mean_system_tokens', 0):.0f}  "
              f"transport {d.get('transport_failures')}  auth {d.get('auth_failures')}  "
              f"judge-parse {d.get('judge_parse_failures')}")


if __name__ == "__main__":
    main()
