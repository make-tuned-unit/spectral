#!/usr/bin/env python3
"""Convert LoCoMo (snap-research/locomo) into the Spectral oracle's LongMemEval-
style schema, so Spectral can be evaluated on a benchmark its retrieval was never
tuned on (a genuine held-out set).

LoCoMo is a *different* long-term-conversation benchmark. Each sample is a
multi-session dialogue plus QA pairs; each QA carries `evidence` dia_ids
("D<session>:<turn>") pointing at the supporting turns. We mark the sessions
containing those turns with an `answer_` prefix — the oracle's convention for an
evidence session (every turn in it becomes an answer key) — so both session-recall
and key-recall are computable.

Notes / honest caveats:
  * Adversarial questions (LoCoMo category 5) are excluded — they test refusal,
    not retrieval, so there is no evidence session to recall.
  * Open-domain questions (category 3) are excluded — their answers come from
    world knowledge, not the conversation.
  * We keep categories 1 (multi-hop), 2 (temporal), 4 (single-hop), mapped to the
    oracle's category labels for reporting.
  * LoCoMo evidence sessions are long (often 15-30 turns), and ALL their turns
    count as answer keys, so key-recall is a stricter measure than on
    LongMemEval — SESSION-recall is the comparable headline metric.
  * Sampling is deterministic (seed 42) so the held-out set is reproducible.

Usage:
    python scripts/locomo_to_oracle.py locomo10.json locomo_heldout.json [--per-cat 40]

Get LoCoMo:
    curl -sL https://raw.githubusercontent.com/snap-research/locomo/main/data/locomo10.json -o locomo10.json
"""
import argparse
import json
import random
import re
from datetime import datetime

MONTHS = {m: i + 1 for i, m in enumerate(
    ["january", "february", "march", "april", "may", "june", "july",
     "august", "september", "october", "november", "december"])}

# LoCoMo category -> oracle question_type (for report grouping only; retrieval
# routing is inferred from the question text, not this field).
CATEGORY = {1: "multi-session", 2: "temporal-reasoning", 4: "single-session-user"}


def to_lme_date(s: str) -> str:
    """'1:56 pm on 8 May, 2023' -> '2023/05/08 (Mon) 13:56'.

    The oracle ingests haystack dates with the LongMemEval format
    `%Y/%m/%d (%a) %H:%M` — the day-of-week token is required.
    """
    if not s:
        return ""
    m = re.match(r"\s*(\d+):(\d+)\s*(am|pm)\s+on\s+(\d+)\s+([A-Za-z]+),?\s+(\d+)", s, re.I)
    if not m:
        return ""
    hh, mm, ap, day, mon, yr = m.groups()
    hh, mm = int(hh), int(mm)
    if ap.lower() == "pm" and hh != 12:
        hh += 12
    if ap.lower() == "am" and hh == 12:
        hh = 0
    mo = MONTHS.get(mon.lower())
    if not mo:
        return ""
    return datetime(int(yr), mo, int(day), hh, mm).strftime("%Y/%m/%d (%a) %H:%M")


def convert(locomo):
    out = []
    for ci, conv in enumerate(locomo):
        c = conv["conversation"]
        nums = sorted(int(k.split("_")[1]) for k in c if re.fullmatch(r"session_\d+", k))
        sessions, dates = {}, {}
        for n in nums:
            turns = []
            for t in c.get(f"session_{n}", []):
                role = "user" if t.get("speaker") == c.get("speaker_a") else "assistant"
                txt = (t.get("text") or "").strip()
                if txt:
                    turns.append({"role": role, "content": txt})
            sessions[n] = turns
            dates[n] = to_lme_date(c.get(f"session_{n}_date_time", ""))
        for qi, qa in enumerate(conv.get("qa", [])):
            cat = qa.get("category")
            ev = qa.get("evidence") or []
            ans = qa.get("answer")
            if cat not in CATEGORY or not ev or not isinstance(ans, str) or not ans.strip():
                continue
            ans_sess = {int(m.group(1)) for e in ev if (m := re.match(r"D(\d+):", str(e)))}
            if not ans_sess:
                continue
            out.append({
                "question_id": f"locomo_{ci}_{qi}",
                "question_type": CATEGORY[cat],
                "question": qa["question"],
                "answer": ans,
                "question_date": dates.get(max(nums), ""),
                "haystack_sessions": [sessions[n] for n in nums],
                "haystack_session_ids": [
                    f"answer_session_{n}" if n in ans_sess else f"session_{n}" for n in nums
                ],
                "haystack_dates": [dates[n] for n in nums],
            })
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("locomo_json")
    ap.add_argument("out_json")
    ap.add_argument("--per-cat", type=int, default=40, help="questions sampled per category")
    ap.add_argument("--seed", type=int, default=42)
    args = ap.parse_args()

    random.seed(args.seed)
    everything = convert(json.load(open(args.locomo_json)))
    by_cat = {}
    for q in everything:
        by_cat.setdefault(q["question_type"], []).append(q)
    sample = []
    for qs in by_cat.values():
        random.shuffle(qs)
        sample += qs[: args.per_cat]
    random.shuffle(sample)
    json.dump(sample, open(args.out_json, "w"))

    from collections import Counter
    print(f"answerable: {len(everything)}  sampled: {len(sample)}  "
          f"{dict(Counter(q['question_type'] for q in sample))}")


if __name__ == "__main__":
    main()
