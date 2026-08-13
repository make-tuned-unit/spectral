#!/usr/bin/env python3
"""R30: dump the DERIVATION-half misses the alias table may be authored from.

Split fixed in `query-aliases-prereg-2026-08-13.md`: derivation =
conversations locomo_{0,2,4,6,8}. This script refuses to emit anything from
the test half, so table authoring physically cannot consult it.

Output: one JSON list of {question_id, question, missed_turns: [text, ...],
shared_words: n} restricted to missed evidence turns of ALL derivation-half
questions (not only zero-evidence ones — a thin 1-shared miss can also
motivate a generic pair, and excluding it would only hide data the prereg
allows).
"""
import argparse
import json

DERIVATION_CONVS = {"locomo_0", "locomo_2", "locomo_4", "locomo_6", "locomo_8"}


def conv_of(question_id):
    return question_id.rsplit("_", 1)[0]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--rows", required=True)
    ap.add_argument("--dataset", required=True)
    ap.add_argument("--out", required=True)
    args = ap.parse_args()

    ds = {q["question_id"]: q for q in json.load(open(args.dataset))}

    def turn_text(q, key):
        sid, _, idx, _role = key.split(":", 3)
        try:
            si = q["haystack_session_ids"].index(sid)
        except ValueError:
            return None
        sess = q["haystack_sessions"][si]
        i = int(idx)
        if i >= len(sess):
            return None
        t = sess[i]
        return t.get("content") or t.get("text") or ""

    out = []
    skipped_test_half = 0
    for line in open(args.rows):
        line = line.strip()
        if not line:
            continue
        r = json.loads(line)
        if conv_of(r["question_id"]) not in DERIVATION_CONVS:
            skipped_test_half += 1
            continue
        missed = r.get("evidence_keys_missed") or []
        if not missed:
            continue
        q = ds.get(r["question_id"])
        if not q:
            continue
        turns = [t for t in (turn_text(q, k) for k in missed) if t]
        if turns:
            out.append({
                "question_id": r["question_id"],
                "question": q.get("question", ""),
                "missed_turns": turns,
            })

    json.dump(out, open(args.out, "w"), indent=1)
    print(f"derivation-half questions with misses: {len(out)} "
          f"(test-half rows withheld: {skipped_test_half})")


if __name__ == "__main__":
    main()
