#!/usr/bin/env python3
"""Price turn adjacency on a SECOND corpus, offline, before spending a run.

The mechanism diagnostic (2026-08-11) found adjacency is indifferent to lexical
overlap: it does not attack the coreference inversion, it is simply orthogonal
to the lexical channel. That makes its value depend on **dialogue geometry** —
question and answer landing in adjacent turns — and LoCoMo's two-party strictly
alternating chat is the best possible case for it.

R24 is the cautionary precedent: it PASSED on LoCoMo and provably does not
transfer to LongMemEval, because the structure it needs is not there. So before
any LongMemEval adjacency run is scheduled, this asks the cheap version of the
question from archived rows plus the corpus labels:

    of the labelled evidence turns the baseline MISSED, how many sit within
    +/-1 of a turn it retrieved?

That is adjacency's **ceiling** on this corpus — the most it could recover if
every neighbour it pulled in were free and none displaced anything. A ceiling
near zero kills the run for $0. A high ceiling does not prove a gain.

$0: no brains, no model calls, no retrieval. Archived JSONL + dataset labels.
"""
import argparse
import json
import re
from pathlib import Path

KEY_RE = re.compile(r"^(?P<sess>.+):turn:(?P<idx>\d+):(?P<role>user|assistant)$")
MAX_WINDOW = 6


def neighbours(key, dist):
    m = KEY_RE.match(key)
    if not m:
        return []
    i = int(m.group("idx"))
    return [f"{m.group('sess')}:turn:{j}:{r}"
            for j in (i - dist, i + dist) if j >= 0
            for r in ("user", "assistant")]


def evidence_keys(dataset_path):
    """question_id -> set of keys for turns the corpus labels has_answer."""
    out = {}
    for q in json.loads(Path(dataset_path).read_text()):
        keys = set()
        for sid, sess in zip(q["haystack_session_ids"], q["haystack_sessions"]):
            for i, t in enumerate(sess):
                if t.get("has_answer"):
                    keys.add(f"{sid}:turn:{i}:{t['role']}")
        if keys:
            out[q["question_id"]] = keys
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--arm", required=True, help="archived oracle jsonl with retrieved_keys")
    ap.add_argument("--dataset", required=True, help="longmemeval_s.json (carries has_answer)")
    ap.add_argument("--label", default="LongMemEval")
    args = ap.parse_args()

    ev = evidence_keys(args.dataset)
    rows = {}
    for line in Path(args.arm).read_text().splitlines():
        if line.strip():
            r = json.loads(line)
            rows[r["question_id"]] = r

    ids = sorted(set(ev) & set(rows))
    total_ev = retrieved = 0
    reach = []          # per missed turn: smallest window that reaches it, or None
    key_miss = 0

    for qid in ids:
        got = set(rows[qid]["retrieved_keys"])
        for k in ev[qid]:
            total_ev += 1
            if k in got:
                retrieved += 1
                continue
            if not KEY_RE.match(k):
                key_miss += 1
                continue
            reach.append(min((d for d in range(1, MAX_WINDOW + 1)
                              if any(n in got for n in neighbours(k, d))), default=None))

    missed = len(reach)
    print(f"=== adjacency ceiling on {args.label} ({len(ids)} questions) ===\n")
    print(f"  labelled evidence turns   {total_ev}")
    print(f"  retrieved by the baseline {retrieved} ({retrieved / total_ev:.2%})")
    print(f"  missed                    {missed}")
    if key_miss:
        print(f"  unparseable keys          {key_miss}")
    if not missed:
        return

    print(f"\n  distance from the nearest RETRIEVED turn:")
    cum = 0
    for d in range(1, MAX_WINDOW + 1):
        hit = sum(1 for x in reach if x == d)
        cum += hit
        if hit:
            gain = hit if d == 1 else cum
            print(f"    +/-{d}   +{hit:<4} cumulative {cum:>4}/{missed} "
                  f"({cum / missed:5.1%} of misses)  "
                  f"=> ceiling +{gain / total_ev:5.2%} micro recall")
    unreachable = sum(1 for x in reach if x is None)
    print(f"    unreachable at any window <= {MAX_WINDOW}: "
          f"{unreachable}/{missed} ({unreachable / missed:.1%})")
    print("\n  Ceiling, not forecast: neighbours also displace and cost context.")


if __name__ == "__main__":
    main()
