#!/usr/bin/env python3
"""Does turn adjacency work for the reason we claim it does?

R28 measured that adjacency helps on the production cascade path. It did not
measure *why*. The proposed mechanism is the coreference inversion documented
in `speaker-attribution-diagnostic-2026-08-09.md`:

    BM25 spends its top-k on turns that MENTION a person (a name is the
    highest-IDF query term), but the evidence is that person's OWN reply,
    where they say "I". Measured inversion: 4.3% vs 36.6%, 8.5x.

If that is the mechanism, the turns adjacency recovers should be
**disproportionately the ones BM25 structurally could not reach** — zero
lexical overlap with the question — and they should sit next to a turn the
baseline already had. If instead the recovered turns look like the ones the
baseline already retrieves, adjacency is just a cheap way of raising k and the
mechanism story is decoration.

An effect without a mechanism is a corpus fit waiting to be discovered. This is
$0, offline, on archived JSONL -- no brains, no model calls.
"""
import argparse
import json
import re
from collections import Counter
from pathlib import Path

KEY_RE = re.compile(r"^(?P<sess>.+):turn:(?P<idx>\d+):(?P<role>user|assistant)$")

# Deliberately crude and fixed in advance: the point is a like-for-like
# comparison between two sets of turns under ONE tokenizer, not a tuned
# overlap score. Tuning this after seeing the result would be fitting.
STOP = set("""a an the and or but if then than that this these those of to in on
at by for with from as is are was were be been being it its it's i you he she
they we me him her them my your his their our what when where who whom which
how why did do does done have has had will would can could should may might
about into over under again more most some any all no not so such own same too
very just now there here up out off down".,?!;:'""".split())


def words(s):
    return {w for w in re.findall(r"[a-z0-9']+", s.lower()) if w not in STOP and len(w) > 2}


def load(path):
    rows = {}
    for line in Path(path).read_text().splitlines():
        if line.strip():
            r = json.loads(line)
            rows[r["question_id"]] = r
    return rows


def build_turn_index(dataset_path):
    """question_id -> {key: turn_text}, using the harness's key format."""
    idx = {}
    for q in json.loads(Path(dataset_path).read_text()):
        turns = {}
        for sid, sess in zip(q["haystack_session_ids"], q["haystack_sessions"]):
            # 0-based: verified against locomo_6_49, whose missed evidence key
            # `answer_session_23:turn:9:assistant` is the 10th turn.
            for i, t in enumerate(sess):
                turns[f"{sid}:turn:{i}:{t['role']}"] = t["content"]
        idx[q["question_id"]] = (turns, q["question"])
    return idx


# How wide a window to price offline. Beyond this the "neighbour" framing
# stops meaning anything -- it is just a session dump.
MAX_WINDOW = 6


def neighbours(key, dist=1):
    """Keys exactly `dist` turns either side of `key`."""
    m = KEY_RE.match(key)
    if not m:
        return []
    i = int(m.group("idx"))
    # Role alternates, and we do not know the neighbour's role from the key
    # alone, so emit both candidates per offset and let membership decide.
    return [f"{m.group('sess')}:turn:{j}:{r}"
            for j in (i - dist, i + dist) if j >= 0
            for r in ("user", "assistant")]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--baseline", required=True, help="c0.jsonl")
    ap.add_argument("--treatment", required=True, help="c_adj.jsonl")
    ap.add_argument("--dataset", required=True)
    args = ap.parse_args()

    base, treat = load(args.baseline), load(args.treatment)
    turns = build_turn_index(args.dataset)
    ids = sorted(set(base) & set(treat) & set(turns))

    recovered, still_missed, residual_reach = [], [], []
    adj_explained = 0
    key_parse_fail = 0

    for qid in ids:
        b, t = base[qid], treat[qid]
        turn_text, question = turns[qid]
        qw = words(question)
        b_keys, t_keys = set(b["retrieved_keys"]), set(t["retrieved_keys"])
        # The field is omitted when nothing was missed, not emitted empty.
        b_missed = set(b.get("evidence_keys_missed") or [])
        t_missed = set(t.get("evidence_keys_missed") or [])

        # Only the MISSED evidence keys are enumerable from the rows, so the
        # comparison is between two classes of miss: the ones adjacency
        # recovered and the ones nobody reached.
        for k in b_missed - t_missed:            # adjacency recovered it
            txt = turn_text.get(k)
            if txt is None:
                key_parse_fail += 1
                continue
            recovered.append(len(qw & words(txt)))
            if any(n in b_keys for n in neighbours(k)):
                adj_explained += 1
        for k in b_missed & t_missed:            # nobody got it
            txt = turn_text.get(k)
            if txt is None:
                continue
            still_missed.append(len(qw & words(txt)))
            # Price a wider window offline, before anyone builds it: how far
            # from a turn the BASELINE already had does this residual turn sit?
            # Unreachable at any window means no ±N rule will ever get it.
            residual_reach.append(min(
                (d for d in range(2, MAX_WINDOW + 1)
                 if any(n in b_keys for n in neighbours(k, d))),
                default=None))

    def dist(v, name):
        if not v:
            print(f"  {name}: none")
            return
        c = Counter(min(x, 3) for x in v)
        n = len(v)
        z = c[0] / n
        print(f"  {name:<34} n={n:<5} 0-overlap {c[0]:>4} ({z:5.1%})  "
              f"1 {c[1]:>4}  2 {c[2]:>4}  3+ {c[3]:>4}")

    print(f"=== adjacency mechanism, {len(ids)} questions ===\n")
    print("Lexical overlap between the question and the EVIDENCE TURN itself.")
    print("0-overlap = no lexical bridge; BM25 could not have ranked it.\n")
    dist(recovered, "recovered by adjacency")
    dist(still_missed, "still missed by both")
    if key_parse_fail:
        print(f"\n  (unmatched keys: {key_parse_fail})")

    # The distributions above are easy to misread as "adjacency targets the
    # coreference class". The recovery RATE per class is the claim-bearing
    # number: it says whether adjacency reaches unreachable turns at a higher
    # rate than reachable ones, or is simply indifferent to lexical overlap.
    rec_c, miss_c = Counter(min(x, 3) for x in recovered), Counter(min(x, 3) for x in still_missed)
    print("\n=== recovery rate by overlap class (recovered / all missed) ===")
    for cls in (0, 1, 2, 3):
        tot = rec_c[cls] + miss_c[cls]
        if tot:
            name = "0 (no lexical bridge)" if cls == 0 else ("3+" if cls == 3 else str(cls))
            print(f"  overlap {name:<22} {rec_c[cls]:>4}/{tot:<5} {rec_c[cls] / tot:6.1%}")

    # What a wider window could buy, priced from archived rows -- no run.
    if residual_reach:
        print(f"\n=== the residual: {len(residual_reach)} evidence turns neither arm reached ===")
        print("  distance from the nearest turn the BASELINE already retrieved:")
        cum = 0
        for d in range(2, MAX_WINDOW + 1):
            hit = sum(1 for x in residual_reach if x == d)
            cum += hit
            if hit:
                print(f"    within +/-{d}   +{hit:<4} cumulative {cum:>4}/{len(residual_reach)} "
                      f"({cum / len(residual_reach):5.1%} of the residual)")
        unreachable = sum(1 for x in residual_reach if x is None)
        print(f"    unreachable at any window <= {MAX_WINDOW}: {unreachable}/{len(residual_reach)} "
              f"({unreachable / len(residual_reach):.1%})")
        print("  NB: a ceiling, not a forecast -- widening also adds distractors,")
        print("  and every extra turn is paid for in context.")

    n = len(recovered)
    if n:
        print(f"\n=== is it actually adjacency? ===")
        print(f"  recovered turns adjacent to a turn the baseline already had: "
              f"{adj_explained}/{n} ({adj_explained / n:.1%})")
        print("  (the rest arrived via the wider candidate pool, not the ±1 rule)")


if __name__ == "__main__":
    main()
