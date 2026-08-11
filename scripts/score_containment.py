#!/usr/bin/env python3
"""R30 primary metric: deterministic normalized containment. No model in loop.

Written BEFORE either arm finished, against the rule fixed in
`docs/internal/adjacency-accuracy-prereg-2026-08-11.md`, so the scorer cannot be
tuned to the result it produces.

The rule, verbatim from the prereg: lowercase, strip punctuation and articles;
correct iff **every** comma-separated ground-truth item appears as a substring
of the prediction.

Why deterministic is primary and the LLM judge secondary: the judge here is a
7B local model, and a judge that drifts between arms silently manufactures or
destroys an effect. Containment is reproducible by anyone holding the report.
It is also *harsh* -- it will mark a correct-but-reworded answer wrong -- but it
is harsh **identically on both arms**, which is what a paired test needs.

Known harshness, found by self-test before either arm finished and deliberately
NOT patched: morphological variants fail. "trophies" does not contain "trophy",
so a correct plural is scored wrong. Adding a stemmer after seeing that case
would be tuning the instrument mid-experiment, even though the case was picked
blind. The cost is **power, not bias** -- it lands on both arms equally -- and
the LLM judge is the lenient counterpart already registered as the secondary
metric.
"""
import argparse
import json
import re
from math import comb
from pathlib import Path

ARTICLES = {"a", "an", "the"}


def normalize(s):
    s = str(s).lower()
    s = re.sub(r"[^a-z0-9\s]", " ", s)
    return " ".join(w for w in s.split() if w not in ARTICLES)


def items(ground_truth):
    """Ground truth is a comma-separated list of required items."""
    parts = [normalize(p) for p in str(ground_truth).split(",")]
    return [p for p in parts if p]


def contained(pred, gt):
    p = normalize(pred)
    reqs = items(gt)
    if not reqs:
        return False
    return all(r in p for r in reqs)


def load(path):
    d = json.loads(Path(path).read_text())
    return {r["question_id"]: r for r in d.get("results", [])}


def exact_two_sided_p(b, c):
    n = b + c
    if n == 0:
        return 1.0
    pmf = lambda k: comb(n, k) * 0.5**n
    obs = pmf(min(b, c))
    return min(1.0, sum(pmf(k) for k in range(n + 1) if pmf(k) <= obs + 1e-12))


def wilcoxon_p(diffs):
    """Two-sided Wilcoxon signed-rank, normal approximation with tie handling.

    Non-zero paired differences only, per the standard definition. Falls back to
    1.0 below n=10, where the normal approximation is not trustworthy and the
    honest answer is 'no evidence' rather than a fabricated p.
    """
    # Zero differences are dropped by definition. Guarding here as well as at
    # the call site: passing an all-zero vector must mean "no evidence of a
    # difference", and without this it reported p ~ 9e-05 for exactly that.
    diffs = [d for d in diffs if d != 0]
    n = len(diffs)
    if n < 10:
        return 1.0
    order = sorted(range(n), key=lambda i: abs(diffs[i]))
    ranks = [0.0] * n
    i = 0
    while i < n:                       # average ranks within ties
        j = i
        while j + 1 < n and abs(diffs[order[j + 1]]) == abs(diffs[order[i]]):
            j += 1
        avg = (i + j) / 2 + 1
        for k in range(i, j + 1):
            ranks[order[k]] = avg
        i = j + 1
    w_plus = sum(r for r, d in zip(ranks, diffs) if d > 0)
    mu = n * (n + 1) / 4
    sigma = (n * (n + 1) * (2 * n + 1) / 24) ** 0.5
    if sigma == 0:
        return 1.0
    z = (w_plus - mu) / sigma
    # Two-sided normal tail via erfc.
    from math import erfc
    return min(1.0, erfc(abs(z) / (2 ** 0.5)))


def wilson(k, n):
    if n == 0:
        return (0.0, 0.0)
    z, p = 1.96, k / n
    d = 1 + z * z / n
    c = (p + z * z / (2 * n)) / d
    h = z * ((p * (1 - p) / n + z * z / (4 * n * n)) ** 0.5) / d
    return (max(0.0, c - h), min(1.0, c + h))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--arm-a", required=True, help="baseline report JSON")
    ap.add_argument("--arm-b", required=True, help="treatment report JSON")
    ap.add_argument("--label-a", default="a0")
    ap.add_argument("--label-b", default="a_adj")
    args = ap.parse_args()

    a, b = load(args.arm_a), load(args.arm_b)
    ids = sorted(set(a) & set(b))
    if not ids:
        raise SystemExit("no shared question_ids")

    # Graded metric, added by the prereg amendment while A0 was still running
    # and before A_ADJ existed. The binary metrics sit at ~12% because
    # multi-session answers are multi-item lists and all-or-nothing scoring
    # gives 4-of-8 the same credit as 0-of-8; this uses those partial answers.
    def item_recall(r):
        reqs = items(r["ground_truth"])
        if not reqs:
            return None
        p = normalize(r["predicted"])
        return sum(1 for q in reqs if q in p) / len(reqs)

    graded = [(item_recall(a[q]), item_recall(b[q])) for q in ids]
    graded = [(x, y) for x, y in graded if x is not None and y is not None]
    if graded:
        ma = sum(x for x, _ in graded) / len(graded)
        mb = sum(y for _, y in graded) / len(graded)
        diffs = [y - x for x, y in graded if y != x]
        w_p = wilcoxon_p(diffs)
        print(f"\n=== item-level recall (graded secondary)  (n={len(graded)}) ===")
        print(f"  {args.label_a:<8} {ma:6.2%}")
        print(f"  {args.label_b:<8} {mb:6.2%}")
        print(f"  delta {mb - ma:+.2%}   changed on {len(diffs)} questions   "
              f"Wilcoxon p={w_p:.4f}")

    for metric, get in (("containment (PRIMARY, deterministic)",
                         lambda r: contained(r["predicted"], r["ground_truth"])),
                        ("local LLM judge (secondary)",
                         lambda r: bool(r["correct"]))):
        ka = sum(1 for q in ids if get(a[q]))
        kb = sum(1 for q in ids if get(b[q]))
        n = len(ids)
        lo_a, hi_a = wilson(ka, n)
        lo_b, hi_b = wilson(kb, n)
        d01 = sum(1 for q in ids if get(a[q]) and not get(b[q]))   # a only
        d10 = sum(1 for q in ids if get(b[q]) and not get(a[q]))   # b only
        p = exact_two_sided_p(d01, d10)
        print(f"\n=== {metric}  (n={n}) ===")
        print(f"  {args.label_a:<8} {ka:>4}/{n}  {ka / n:6.2%}  "
              f"95% CI [{lo_a:.2%}, {hi_a:.2%}]")
        print(f"  {args.label_b:<8} {kb:>4}/{n}  {kb / n:6.2%}  "
              f"95% CI [{lo_b:.2%}, {hi_b:.2%}]")
        print(f"  delta {(kb - ka) / n:+.2%}   discordant "
              f"{d10} for {args.label_b} / {d01} for {args.label_a}   "
              f"exact McNemar p={p:.4f}")
        if ka / n < 0.10:
            print(f"  !! {args.label_a} below the 10% floor -- prereg says "
                  f"UNINFORMATIVE, not a null")

    # Context cost actually paid, so the delta is never read without its price.
    for lbl, arm in ((args.label_a, a), (args.label_b, b)):
        toks = [len(arm[q].get("actor_context") or "") // 4 for q in ids]
        print(f"\n  {lbl:<8} mean context ~{sum(toks) / len(toks):.0f} tok")


if __name__ == "__main__":
    main()
