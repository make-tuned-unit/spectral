#!/usr/bin/env python3
"""Price the vocabulary-bridging lever BEFORE building an alias table.

`query_aliases` is a consumer-curated table, so "testing" it means authoring
one. Authoring it by looking at the questions it will be scored on is fitting
to the evaluation set, and the resulting number would mean nothing. This
diagnoses the *shape* of the failures instead, which is a property of the
corpus and does not depend on any table we might write.

The question this answers: when we retrieve no evidence at all, is the missed
evidence turn something a synonym table could have reached?

- **0 shared content words** -> no lexical bridge exists. An alias table maps
  word to word; it cannot connect a question and an answer that share no words.
- **1 shared content word** -> the turn IS admitted by FTS but ranks low
  (G4 measured exactly this: 88.8% of deep misses carry at most one query
  term). That is a ranking problem, and the ranking family is closed.
- **2+ shared** -> squarely a ranking problem.

Only the first bucket is addressable by aliases, and only if the missing link
is a *synonym* rather than an inference.
"""
import argparse
import json
import re
from collections import Counter

# Deliberately broad: a word that is a stopword for FTS is still a word an
# alias table could bridge, so being generous here can only make the
# addressable bucket look BIGGER, never smaller. The conclusion is safe in the
# direction that matters.
STOP = set(
    """a an the and or but if then than that this these those there here of in on at to for
with without from by as is are was were be been being am do does did doing have has had
having i you he she it we they me him her them my your his its our their what which who
whom whose when where why how all any both each few more most other some such no nor not
only own same so too very can will just should now about into over after before under
again further once during above below between out up down off""".split()
)


def content_words(text):
    return {w for w in re.findall(r"[a-z0-9']+", text.lower()) if len(w) > 2 and w not in STOP}


def stem(w):
    for suf in ("ing", "ed", "es", "ly", "s"):
        if w.endswith(suf) and len(w) - len(suf) >= 3:
            return w[: -len(suf)]
    return w


def stems(ws):
    return {stem(w) for w in ws}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--rows", required=True, help="oracle arm JSONL")
    ap.add_argument("--dataset", required=True)
    args = ap.parse_args()

    ds = {q["question_id"]: q for q in json.load(open(args.dataset))}

    # key -> content, per question
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

    buckets = Counter()
    examples = []
    total_missed = 0
    for line in open(args.rows):
        line = line.strip()
        if not line:
            continue
        r = json.loads(line)
        if r.get("evidence_turns_total", 0) == 0:
            continue
        # Only questions where we retrieved NO evidence at all: the population
        # the lever is meant to rescue.
        if r["evidence_turns_retrieved"] != 0:
            continue
        q = ds.get(r["question_id"])
        if not q:
            continue
        qw = stems(content_words(q.get("question", "")))
        for key in r.get("evidence_keys_missed", []):
            txt = turn_text(q, key)
            if txt is None:
                continue
            total_missed += 1
            shared = qw & stems(content_words(txt))
            n = len(shared)
            buckets["0 shared (no lexical bridge)" if n == 0 else
                    "1 shared (thin: admitted, ranked low)" if n == 1 else
                    "2+ shared (ranking problem)"] += 1
            if n == 0 and len(examples) < 5:
                examples.append((q.get("question", "")[:90], txt[:110]))

    print(f"\nMissed evidence turns in zero-evidence questions: {total_missed}\n")
    for k, v in sorted(buckets.items()):
        print(f"  {k:<40} {v:>4}  ({v/max(total_missed,1)*100:5.1f}%)")

    print("\nExamples with ZERO shared content words "
          "(the only bucket an alias table could address):")
    for qq, tt in examples:
        print(f"\n  Q: {qq}")
        print(f"  A: {tt}")


if __name__ == "__main__":
    main()
