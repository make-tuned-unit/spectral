#!/usr/bin/env python3
"""R33 pricing: is the answer-shape signal discriminative BEFORE building it?

R22 queued "answer-shape matching" — 'how many' preferring quantity-bearing
turns — as the one remaining query-conditioned $0 idea. Before implementing a
boost, measure the signal itself, offline, from an archived baseline arm:

For each SHAPE-CLASS question (count / date-time), over its haystack:
  - evidence rate:   P(turn carries the shape | turn is evidence)
  - distractor rate: P(turn carries the shape | turn is not evidence)
  - lift = evidence rate / distractor rate

A boost can only help if lift is well above 1 for the turns we MISS — a
signal that fires on evidence and distractors alike reranks noise (that is
what killed declarative density as a static prior: it was a corpus prior,
not a question signal). Also reports how much of the miss population the
addressable classes even cover, which caps any possible gain.
"""
import argparse
import json
import re

COUNT_RE = re.compile(r"\bhow (many|much|often|long)\b", re.I)
DATE_RE = re.compile(r"\b(when|what date|which day|what day|what time|what year)\b", re.I)

MONTHS = ("january february march april may june july august september october "
          "november december jan feb mar apr jun jul aug sep sept oct nov dec").split()
NUMWORDS = ("one two three four five six seven eight nine ten eleven twelve "
            "first second third fourth fifth couple few several twice once").split()
TIMEWORDS = ("yesterday today tomorrow week weekend month year morning evening "
             "night monday tuesday wednesday thursday friday saturday sunday "
             "spring summer autumn fall winter ago last next").split()


def has_digit(t):
    return bool(re.search(r"\d", t))


def has_any(t, words):
    tl = re.findall(r"[a-z']+", t.lower())
    s = set(tl)
    return any(w in s for w in words)


def count_shape(t):
    return has_digit(t) or has_any(t, NUMWORDS)


def date_shape(t):
    return has_digit(t) or has_any(t, MONTHS) or has_any(t, TIMEWORDS)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--rows", required=True)
    ap.add_argument("--dataset", required=True)
    args = ap.parse_args()

    ds = {q["question_id"]: q for q in json.load(open(args.dataset))}

    classes = {
        "count": (COUNT_RE, count_shape),
        "date-time": (DATE_RE, date_shape),
    }

    stats = {k: {"q": 0, "ev": 0, "ev_shape": 0, "non": 0, "non_shape": 0,
                 "missed": 0, "missed_shape": 0} for k in classes}
    total_q = 0
    total_missed_all = 0

    for line in open(args.rows):
        line = line.strip()
        if not line:
            continue
        r = json.loads(line)
        if r.get("evidence_turns_total") is None:
            continue
        total_q += 1
        total_missed_all += len(r.get("evidence_keys_missed") or [])
        q = ds.get(r["question_id"])
        if not q:
            continue
        qtext = q.get("question", "")
        cls = next((k for k, (qre, _) in classes.items() if qre.search(qtext)), None)
        if cls is None:
            continue
        _, shape = classes[cls]
        st = stats[cls]
        st["q"] += 1
        missed = set(r.get("evidence_keys_missed") or [])
        for si, sess in enumerate(q["haystack_sessions"]):
            sid = q["haystack_session_ids"][si]
            for ti, t in enumerate(sess):
                txt = t.get("content") or t.get("text") or ""
                if not txt:
                    continue
                is_ev = bool(t.get("has_answer"))
                s = shape(txt)
                if is_ev:
                    st["ev"] += 1
                    st["ev_shape"] += s
                else:
                    st["non"] += 1
                    st["non_shape"] += s
        for k in missed:
            sid, _, idx, _role = k.split(":", 3)
            try:
                si = q["haystack_session_ids"].index(sid)
                txt = (q["haystack_sessions"][si][int(idx)].get("content") or "")
            except (ValueError, IndexError):
                continue
            st["missed"] += 1
            st["missed_shape"] += shape(txt)

    print(f"questions total {total_q}, missed evidence turns total {total_missed_all}\n")
    for k, st in stats.items():
        if not st["q"]:
            continue
        er = st["ev_shape"] / max(st["ev"], 1)
        nr = st["non_shape"] / max(st["non"], 1)
        mr = st["missed_shape"] / max(st["missed"], 1)
        print(f"== {k}: {st['q']} questions "
              f"({st['q']/total_q*100:.1f}% of corpus), "
              f"{st['missed']} missed evidence turns "
              f"({st['missed']/max(total_missed_all,1)*100:.1f}% of all misses)")
        print(f"   shape rate on evidence turns:   {er*100:5.1f}%  ({st['ev_shape']}/{st['ev']})")
        print(f"   shape rate on non-evidence:     {nr*100:5.1f}%  ({st['non_shape']}/{st['non']})")
        print(f"   shape rate on MISSED evidence:  {mr*100:5.1f}%  ({st['missed_shape']}/{st['missed']})")
        print(f"   lift (evidence vs distractor):  {er/max(nr,1e-9):.2f}x\n")


if __name__ == "__main__":
    main()
