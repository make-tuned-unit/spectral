#!/usr/bin/env python3
"""Score oracle arms on evidence-turn recall and diff them pairwise.

The metric this reports is R19's evidence-turn recall, not the diluted
answer-session recall R15 named. Those differ by 35pp on the same rows, and
using the wrong one is the specific mistake that made six retrieval levers read
as noise -- see `docs/internal/r19-locomo-turn-labels-2026-08-08.md`.

Primary statistic is the exact two-sided McNemar test on the paired per-question
`all evidence turns retrieved` indicator. Effect size without significance is
not a result here, and neither is significance on three discordant pairs, so
both are printed and the discordant counts are always shown.
"""
import argparse
import json
from math import comb
from pathlib import Path


def load(path):
    """Load oracle rows, tolerating a torn final line.

    Arms are read while they are still being written, and a half-flushed last
    line is not a corrupt arm. Anything *other* than an unterminated tail is
    still an error — silently dropping bad rows mid-file would quietly shrink a
    denominator, which is the one failure mode that would not be visible in the
    output.
    """
    rows, lines = {}, open(path).read().splitlines()
    for i, line in enumerate(lines):
        line = line.strip()
        if not line:
            continue
        try:
            r = json.loads(line)
        except json.JSONDecodeError:
            if i == len(lines) - 1:
                continue  # torn tail of an in-flight arm
            raise
        rows[r["question_id"]] = r
    return rows


def exact_two_sided_p(b, c):
    """Exact binomial McNemar on the discordant pairs."""
    n = b + c
    if n == 0:
        return 1.0

    def pmf(k):
        return comb(n, k) * 0.5**n

    obs = pmf(min(b, c))
    return min(1.0, sum(pmf(k) for k in range(n + 1) if pmf(k) <= obs + 1e-12))


def labelled(rows):
    """Rows carrying R19 evidence labels. Unlabelled rows are excluded from
    every metric rather than silently counted as misses."""
    return {q: r for q, r in rows.items() if r.get("evidence_turns_total", 0) > 0}


def summarize(rows, key=None):
    ev = labelled(rows)
    if key:
        ev = {q: r for q, r in ev.items() if key(r)}
    if not ev:
        return None
    tot = sum(r["evidence_turns_total"] for r in ev.values())
    got = sum(r["evidence_turns_retrieved"] for r in ev.values())
    macro = sum(
        r["evidence_turns_retrieved"] / r["evidence_turns_total"] for r in ev.values()
    ) / len(ev)
    zero = sum(1 for r in ev.values() if r["evidence_turns_retrieved"] == 0)
    full = sum(
        1
        for r in ev.values()
        if r["evidence_turns_retrieved"] == r["evidence_turns_total"]
    )
    toks = sum(r.get("context_tokens_est", 0) for r in ev.values()) / len(ev)
    return {
        "n": len(ev),
        "micro": got / tot,
        "got": got,
        "tot": tot,
        "macro": macro,
        "zero": zero,
        "full": full,
        "tokens": toks,
    }


def fmt(s):
    if s is None:
        return "      --"
    return (
        f"{s['micro']*100:6.2f}% ({s['got']:>3}/{s['tot']}) "
        f"macro {s['macro']*100:5.2f}%  zero {s['zero']:>3}  "
        f"full {s['full']:>3}  tok {s['tokens']:>6.0f}"
    )


def diff(base, cand, label):
    """Paired McNemar on the full-evidence-retrieved indicator."""
    shared = sorted(set(labelled(base)) & set(labelled(cand)))
    b = c = 0  # b: base yes / cand no, c: base no / cand yes
    for q in shared:
        bf = base[q]["evidence_turns_retrieved"] == base[q]["evidence_turns_total"]
        cf = cand[q]["evidence_turns_retrieved"] == cand[q]["evidence_turns_total"]
        if bf and not cf:
            b += 1
        elif cf and not bf:
            c += 1
    p = exact_two_sided_p(b, c)
    bs, cs = summarize(base), summarize(cand)
    d_micro = (cs["micro"] - bs["micro"]) * 100
    d_turns = cs["got"] - bs["got"]
    print(
        f"  {label:<28} d_micro {d_micro:+6.2f}pp  d_turns {d_turns:+4}  "
        f"discordant {b}/{c}  p={p:.4f}"
    )
    return {"delta_micro_pp": d_micro, "delta_turns": d_turns, "b": b, "c": c, "p": p}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--arms", nargs="+", required=True, help="label=path.jsonl")
    ap.add_argument("--baseline", default="a0", help="arm label to diff against")
    ap.add_argument("--json-out", type=Path)
    args = ap.parse_args()

    arms = {}
    for spec in args.arms:
        label, _, path = spec.partition("=")
        arms[label] = load(path)

    print("\n=== Evidence-turn recall (R19 labels) ===")
    for label, rows in arms.items():
        print(f"  {label:<8} {fmt(summarize(rows))}")

    print("\n=== multi-session slice ===")
    ms = lambda r: r.get("category") == "multi-session"
    for label, rows in arms.items():
        print(f"  {label:<8} {fmt(summarize(rows, ms))}")

    print(f"\n=== Rank of first evidence turn, vs {args.baseline} ===")
    # The mechanism, not the verdict. RRF's entire claim is that it can promote
    # a deep evidence turn that additive boosts cannot reach. That claim is
    # about RANK, and it is testable even when recall does not move: a
    # composition that promotes nothing has no mechanism, while one that
    # promotes turns without improving recall is displacing something else.
    basline_rows = arms[args.baseline]
    for label, rows in arms.items():
        if label == args.baseline:
            continue
        moved_up = moved_down = same = 0
        deltas = []
        for q in set(labelled(basline_rows)) & set(labelled(rows)):
            rb = basline_rows[q].get("rank_first_evidence_turn")
            rc = rows[q].get("rank_first_evidence_turn")
            if rb is None or rc is None:
                continue  # no evidence retrieved in one arm; recall covers that
            deltas.append(rb - rc)
            if rc < rb:
                moved_up += 1
            elif rc > rb:
                moved_down += 1
            else:
                same += 1
        med = sorted(deltas)[len(deltas) // 2] if deltas else 0
        print(
            f"  {label:<8} promoted {moved_up:>3}  demoted {moved_down:>3}  "
            f"unchanged {same:>3}  median move {med:+d} ranks"
        )

    print(f"\n=== Paired McNemar vs {args.baseline} (full-evidence indicator) ===")
    base = arms[args.baseline]
    out = {}
    for label, rows in arms.items():
        if label == args.baseline:
            continue
        out[label] = diff(base, rows, f"{label} vs {args.baseline}")

    if args.json_out:
        args.json_out.write_text(
            json.dumps(
                {
                    "arms": {k: summarize(v) for k, v in arms.items()},
                    "vs_baseline": out,
                },
                indent=2,
            )
        )
        print(f"\nwrote {args.json_out}")


if __name__ == "__main__":
    main()
