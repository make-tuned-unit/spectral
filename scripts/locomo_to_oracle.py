#!/usr/bin/env python3
"""Convert LoCoMo (snap-research/locomo) into the Spectral oracle's LongMemEval-
style schema, so Spectral can be evaluated on a benchmark its retrieval was never
tuned on (a genuine held-out set).

LoCoMo is a *different* long-term-conversation benchmark. Each sample is a
multi-session dialogue plus QA pairs; each QA carries `evidence` dia_ids
("D<session>:<turn>") pointing at the supporting turns. We mark the SESSIONS
containing those turns with an `answer_` prefix, so session-recall is computable.

**R19 (2026-08-08): per-turn `has_answer` labels are now emitted.** LoCoMo
turns carry `dia_id` ("D1:3") and each QA carries `evidence: ["D1:3"]`, so the
evidence *turns* are recoverable, not just the evidence sessions. Matching is
**by dia_id, never by turn index** — this script drops empty-text turns, so
positions do not correspond — and sessions are **deep-copied per QA**, because
which turns are evidence differs from question to question.

This makes evidence-turn recall (R15's real metric) computable on LoCoMo, where
it previously reported `n/a`. Session recall alone is saturated by
construction: a 30-turn evidence session counts as recalled when the one turn
that answers the question is absent from the context.

**The regeneration gate is enforced in code, not by convention.** `--verify`
regenerates a sample with the same seed and exclusions, strips `has_answer`,
and asserts byte-equality against the existing file. If sample membership
moves, the held-out set stops being the set that was measured, and the script
exits non-zero rather than writing.

Notes / honest caveats:
  * Adversarial questions (LoCoMo category 5) are excluded — they test refusal,
    not retrieval, so there is no evidence session to recall.
  * Open-domain questions (category 3) are excluded — their answers come from
    world knowledge, not the conversation.
  * We keep categories 1 (multi-hop), 2 (temporal), 4 (single-hop), mapped to the
    oracle's category labels for reporting.
  * LoCoMo evidence sessions are long (often 15-30 turns), and ALL their turns
    land in the answer-session-turn-coverage denominator. This was previously
    described as merely "a stricter measure than on LongMemEval"; it is in
    fact the same ~12x dilution defect R15 names, and the resulting number is
    NOT evidence recall. SESSION-recall is the comparable headline metric.
    See docs/internal/turn-level-evidence-recall-2026-08-07.md.
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


def convert(locomo, turn_labels=True):
    """Convert LoCoMo to the oracle schema.

    `turn_labels=False` reproduces the pre-R19 output exactly, which is what
    `--verify` diffs against to prove sample membership did not move.
    """
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
                    # Carry dia_id alongside the turn so evidence can be matched
                    # by id. Index matching would be WRONG: empty-text turns are
                    # dropped above, so position i here is not position i in the
                    # source session.
                    turns.append(({"role": role, "content": txt}, str(t.get("dia_id") or "")))
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
            ev_ids = {str(e).strip() for e in ev}

            # Deep-copy per QA: which turns are evidence differs per question,
            # so sharing turn dicts across questions would let one question's
            # labels leak into another's.
            haystack = []
            for n in nums:
                sess = []
                for turn, dia in sessions[n]:
                    t = dict(turn)
                    if turn_labels and dia and dia in ev_ids:
                        t["has_answer"] = True
                    sess.append(t)
                haystack.append(sess)

            out.append({
                "question_id": f"locomo_{ci}_{qi}",
                "question_type": CATEGORY[cat],
                "question": qa["question"],
                "answer": ans,
                "question_date": dates.get(max(nums), ""),
                "haystack_sessions": haystack,
                "haystack_session_ids": [
                    f"answer_session_{n}" if n in ans_sess else f"session_{n}" for n in nums
                ],
                "haystack_dates": [dates[n] for n in nums],
            })
    return out


def strip_labels(questions):
    """Remove `has_answer` everywhere, for the byte-equality gate."""
    out = json.loads(json.dumps(questions))
    for q in out:
        for sess in q["haystack_sessions"]:
            for t in sess:
                t.pop("has_answer", None)
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("locomo_json")
    ap.add_argument("out_json")
    ap.add_argument("--per-cat", type=int, default=40, help="questions sampled per category")
    ap.add_argument("--seed", type=int, default=42)
    ap.add_argument(
        "--exclude",
        action="append",
        default=[],
        help="prior sample JSON whose question_ids must not reappear "
        "(repeatable); guarantees a disjoint validation sample",
    )
    ap.add_argument(
        "--all",
        action="store_true",
        help="take EVERY answerable question rather than sampling per category "
        "(what a full published baseline needs; --per-cat is ignored)",
    )
    ap.add_argument(
        "--verify",
        metavar="EXISTING_JSON",
        help="R19 GATE: regenerate with these exact flags, strip `has_answer`, "
        "and assert byte-equality with EXISTING_JSON. Proves the labels are "
        "additive and sample membership did not move. Writes nothing.",
    )
    ap.add_argument(
        "--no-turn-labels",
        action="store_true",
        help="emit the pre-R19 output (no per-turn `has_answer`)",
    )
    args = ap.parse_args()

    burned = set()
    for path in args.exclude:
        burned |= {q["question_id"] for q in json.load(open(path))}

    random.seed(args.seed)
    everything = convert(json.load(open(args.locomo_json)), turn_labels=not args.no_turn_labels)
    if burned:
        before = len(everything)
        everything = [q for q in everything if q["question_id"] not in burned]
        print(f"excluded {before - len(everything)} burned questions "
              f"({len(burned)} ids supplied)")
    by_cat = {}
    for q in everything:
        by_cat.setdefault(q["question_type"], []).append(q)
    # The pool guard protects DISJOINT sampling (a short category would
    # silently yield a smaller sample than asked for). It must not fire for
    # --all, where taking everything is the point.
    if not args.all:
        for cat, qs in sorted(by_cat.items()):
            if len(qs) < args.per_cat:
                raise SystemExit(
                    f"pool exhausted: {cat} has {len(qs)} < {args.per_cat} after exclusion"
                )
    sample = []
    for qs in by_cat.values():
        random.shuffle(qs)
        sample += qs if args.all else qs[: args.per_cat]
    random.shuffle(sample)

    from collections import Counter

    if args.verify:
        # R19 gate. Byte-equality on the SERIALIZED form, not a structural
        # compare: the point is that a downstream reader loading the old file
        # and the stripped new one cannot tell them apart.
        existing = open(args.verify, "rb").read()
        regenerated = json.dumps(strip_labels(sample)).encode()
        if existing != regenerated:
            print(f"GATE FAILED: stripped regeneration differs from {args.verify}")
            print(f"  existing    {len(existing)} bytes")
            print(f"  regenerated {len(regenerated)} bytes")
            try:
                old_ids = [q["question_id"] for q in json.loads(existing)]
                new_ids = [q["question_id"] for q in sample]
                if old_ids != new_ids:
                    print(f"  SAMPLE MEMBERSHIP MOVED: {len(set(old_ids) ^ set(new_ids))} ids differ"
                          if set(old_ids) != set(new_ids)
                          else "  same ids, different ORDER")
                else:
                    print("  ids and order identical — difference is in turn content")
            except Exception as e:  # noqa: BLE001 - diagnostics only
                print(f"  (could not diff ids: {e})")
            raise SystemExit(1)
        labelled = sum(
            1 for q in sample for s in q["haystack_sessions"] for t in s if t.get("has_answer")
        )
        print(f"GATE PASSED: {args.verify} is byte-identical after stripping `has_answer`.")
        print(f"  {len(sample)} questions, {labelled} evidence turns labelled, sample unmoved.")
        return

    json.dump(sample, open(args.out_json, "w"))

    labelled = sum(
        1 for q in sample for s in q["haystack_sessions"] for t in s if t.get("has_answer")
    )
    zero = sum(
        1
        for q in sample
        if not any(t.get("has_answer") for s in q["haystack_sessions"] for t in s)
    )
    print(f"answerable: {len(everything)}  sampled: {len(sample)}  "
          f"{dict(Counter(q['question_type'] for q in sample))}")
    print(f"evidence turns labelled: {labelled}  "
          f"questions with NO labelled turn: {zero}")


if __name__ == "__main__":
    main()
