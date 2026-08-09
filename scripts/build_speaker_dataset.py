#!/usr/bin/env python3
"""Derive a speaker-attributed LoCoMo dataset for R23 arm B.

Restores metadata the converter dropped. Raw LoCoMo carries `speaker_a` /
`speaker_b` per conversation and a `speaker` on every turn; the answerable
export kept only `role`. This rejoins them by turn text within the
conversation named by `question_id` (`locomo_<conv>_<qid>`), which was
validated at 149,456/149,456 turns matched, 0 unmatched.

Nothing here reads a question, an answer, or an evidence label — only turn
text and corpus speaker metadata. That is what keeps R23 from being fitted to
the set it is scored on.

Turn ORDER, session ids, roles and counts are untouched, so `memory_key`
(`{session_id}:turn:{index}:{role}`) is byte-identical to the baseline and the
R19 evidence labels still line up. Only `content` changes.
"""
import argparse
import json


def speaker_maps(raw):
    """conversation index -> {turn text: speaker}"""
    out = []
    for x in raw:
        c, m = x["conversation"], {}
        for k, v in c.items():
            if not k.startswith("session_") or k.endswith("date_time"):
                continue
            if not isinstance(v, list):
                continue
            for t in v:
                txt = (t.get("text") or "").strip()
                if txt:
                    m.setdefault(txt, t.get("speaker"))
        out.append(m)
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--labelled", required=True)
    ap.add_argument("--raw", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--max-questions", type=int)
    args = ap.parse_args()

    lab = json.load(open(args.labelled))
    maps = speaker_maps(json.load(open(args.raw)))
    if args.max_questions:
        lab = lab[: args.max_questions]

    matched = unmatched = 0
    for q in lab:
        ci = int(q["question_id"].split("_")[1])
        m = maps[ci]
        for sess in q["haystack_sessions"]:
            for t in sess:
                c = (t.get("content") or "").strip()
                sp = m.get(c)
                if sp is None:
                    unmatched += 1
                    continue  # leave untouched rather than guess
                matched += 1
                t["content"] = f"{sp}: {t['content']}"

    json.dump(lab, open(args.out, "w"))
    total = matched + unmatched
    print(f"turns speaker-prefixed: {matched}/{total} "
          f"({matched/max(total,1)*100:.2f}%), unmatched left as-is: {unmatched}")
    print(f"wrote {args.out}")


if __name__ == "__main__":
    main()
