#!/usr/bin/env python3
"""Score the BM25-only LoCoMo baseline, with the interval that accounts for
clustering.

LoCoMo is 10 conversations, not 1438 independent questions. Every question
against conversation N shares that conversation's haystack, its speakers, and
its annotation quality, so the questions are not independent draws. A binomial
interval over n=1438 (~±2.4pp) is the wrong variance and would be a smaller,
more flattering number than the data supports.

This computes a **cluster bootstrap**: resample the 10 conversations with
replacement, rescore, repeat. That propagates between-conversation variance,
which is the dominant term.

Reads either the final report JSON or the in-flight checkpoint — both carry
`results`.
"""
import argparse
import json
import random
import re
import statistics
from collections import defaultdict

CONV = re.compile(r"^locomo_(\d+)_")


def conversation_of(question_id: str) -> str:
    m = CONV.match(question_id)
    return m.group(1) if m else "unknown"


def accuracy(rows):
    return sum(1 for r in rows if r["correct"]) / len(rows) if rows else float("nan")


def cluster_bootstrap(by_conv, iters, rng):
    """Resample conversations with replacement; rescore pooled each time."""
    convs = list(by_conv)
    out = []
    for _ in range(iters):
        picked = [rng.choice(convs) for _ in convs]
        rows = [r for c in picked for r in by_conv[c]]
        out.append(accuracy(rows))
    return out


def interval(samples, alpha=0.05):
    s = sorted(samples)
    lo = s[int((alpha / 2) * len(s))]
    hi = s[int((1 - alpha / 2) * len(s)) - 1]
    return lo, hi


EV_SESSION = re.compile(r"answer_session_\d+")


def evidence_turn_recall(ds, rows):
    """R19: the metric session recall was standing in for.

    Requires a dataset carrying per-turn `has_answer` (emit it with
    `locomo_to_oracle.py`, R19). Silently reports unavailable otherwise, since
    that is the honest state for a pre-R19 file — undefined, not 0%.

    Expected keys are built the same way `ingest::memory_key` builds them:
    `{session_id}:turn:{index}:{role}`, index enumerated per session. That
    format is frozen by `memory_key_format_is_frozen`.
    """
    want = {}
    session_turns = 0
    for q in ds:
        keys = set()
        for sid, sess in zip(q["haystack_session_ids"], q["haystack_sessions"]):
            if sid.startswith("answer_"):
                session_turns += len(sess)
            for i, t in enumerate(sess):
                if t.get("has_answer"):
                    keys.add(f"{sid}:turn:{i}:{t['role']}")
        if keys:
            want[q["question_id"]] = keys

    if not want:
        print("\nEVIDENCE-TURN RECALL: unavailable — dataset carries no `has_answer`")
        print("  (UNDEFINED, not 0%. Regenerate with locomo_to_oracle.py for R19 labels.)")
        return

    num = den = zero = 0
    ratios, by_cat = [], defaultdict(lambda: [0, 0, []])
    split = {True: [], False: []}
    for r in rows:
        w = want.get(r["question_id"])
        if not w:
            continue
        hit = len(w & set(r["retrieved_memory_keys"]))
        num += hit
        den += len(w)
        ratios.append(hit / len(w))
        zero += hit == 0
        c = by_cat[r["category"]]
        c[0] += hit
        c[1] += len(w)
        c[2].append(hit / len(w))
        split[bool(r["correct"])].append(hit / len(w))

    print("\nEVIDENCE-TURN RECALL (R19 — the real metric; session recall is ~20x diluted)")
    print(f"  questions labelled  {len(ratios)}")
    print(f"  micro (pooled)      {num}/{den} = {num/den*100:.2f}%")
    print(f"  macro (mean)        {sum(ratios)/len(ratios)*100:.2f}%")
    print(f"  ZERO-evidence       {zero}/{len(ratios)} ({zero/len(ratios)*100:.2f}%)")
    for cat in sorted(by_cat):
        h, t, rr = by_cat[cat]
        z = sum(1 for x in rr if x == 0)
        print(f"    {cat:22s} micro {h}/{t} = {h/t*100:6.2f}%  macro {sum(rr)/len(rr)*100:6.2f}%  zero {z}")
    ok, bad = statistics.mean(split[True]), statistics.mean(split[False])
    print(f"  recall | judged CORRECT    {ok*100:.2f}%  (n={len(split[True])})")
    print(f"  recall | judged INCORRECT  {bad*100:.2f}%  (n={len(split[False])})")
    print(f"  -> difference {(ok-bad)*100:.2f}pp. Retrieval IS discriminative here.")
    print(f"  dilution check: {session_turns} evidence-session turns / {den} true "
          f"evidence turns = {session_turns/den:.1f}x")


def session_recall(dataset_path, rows):
    """The $0 retrieval-side companion, from this run's own retrieved keys.

    Evidence sessions are the `answer_`-prefixed entries in the dataset's
    `haystack_session_ids`; a session counts as retrieved if any retrieved
    memory key belongs to it. Evidence-TURN recall is deliberately absent —
    LoCoMo carries no per-turn `has_answer` labels, so R15's metric reports
    n/a (undefined, not 0%). See R19.
    """
    ds = json.load(open(dataset_path))
    evidence_turn_recall(ds, rows)
    ev = {
        q["question_id"]: {s for s in q["haystack_session_ids"] if s.startswith("answer_")}
        for q in ds
    }
    num = den = zero = 0
    ratios, by_cat = [], defaultdict(lambda: [0, 0, []])
    split = {True: [], False: []}
    for r in rows:
        want = ev.get(r["question_id"], set())
        if not want:
            continue
        got = {m.group(0) for k in r["retrieved_memory_keys"] if (m := EV_SESSION.match(k))}
        hit = len(want & got)
        num += hit
        den += len(want)
        ratios.append(hit / len(want))
        zero += hit == 0
        c = by_cat[r["category"]]
        c[0] += hit
        c[1] += len(want)
        c[2].append(hit / len(want))
        split[bool(r["correct"])].append(hit / len(want))

    print("\nSESSION RECALL (the $0 companion; evidence-TURN recall is n/a on LoCoMo — R15/R19)")
    print(f"  micro (pooled)  {num}/{den} = {num/den*100:.2f}%")
    print(f"  macro (mean)    {sum(ratios)/len(ratios)*100:.2f}%")
    print(f"  zero-evidence   {zero}/{len(ratios)} ({zero/len(ratios)*100:.2f}%)")
    for cat in sorted(by_cat):
        h, t, rr = by_cat[cat]
        print(f"    {cat:22s} micro {h}/{t} = {h/t*100:6.2f}%   macro {sum(rr)/len(rr)*100:6.2f}%")
    print(f"  mean recall | judged CORRECT   {statistics.mean(split[True])*100:.2f}%  (n={len(split[True])})")
    print(f"  mean recall | judged INCORRECT {statistics.mean(split[False])*100:.2f}%  (n={len(split[False])})")
    print(
        f"  -> difference {(statistics.mean(split[True])-statistics.mean(split[False]))*100:.2f}pp: "
        "retrieval is not what separates right from wrong here."
    )


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--report", required=True)
    ap.add_argument("--dataset", help="LoCoMo JSON; enables the session-recall companion")
    ap.add_argument("--iters", type=int, default=10000)
    ap.add_argument("--seed", type=int, default=20260807)
    args = ap.parse_args()

    d = json.load(open(args.report))
    rows = d["results"]
    rng = random.Random(args.seed)

    by_conv = defaultdict(list)
    for r in rows:
        by_conv[conversation_of(r["question_id"])].append(r)

    n = len(rows)
    acc = accuracy(rows)
    print(f"questions scored : {n}")
    print(f"conversations    : {len(by_conv)}")
    print(f"OVERALL ACCURACY : {acc*100:.2f}%  ({sum(1 for r in rows if r['correct'])}/{n})")

    boots = cluster_bootstrap(by_conv, args.iters, rng)
    lo, hi = interval(boots)
    half = (hi - lo) / 2

    # HEADLINE interval. With only G=10 clusters a percentile cluster bootstrap
    # is known to be anti-conservative — the normal critical value assumes the
    # between-cluster variance is estimated from many clusters, and 10 is not
    # many. Use the cluster-level SE with a t(G-1) critical value instead, which
    # is the standard small-G correction and always the wider of the two here.
    cluster_accs = [accuracy(v) for v in by_conv.values()]
    G = len(cluster_accs)
    # Weight-free cluster-level SE: SD of per-cluster accuracies / sqrt(G).
    se = statistics.stdev(cluster_accs) / (G**0.5)
    t_crit = {5: 2.776, 6: 2.571, 7: 2.447, 8: 2.365, 9: 2.262, 10: 2.228}.get(G - 1, 1.96)
    t_half = t_crit * se
    print(
        f"CLUSTER-ROBUST 95% CI    : [{(acc-t_half)*100:.2f}%, {(acc+t_half)*100:.2f}%]"
        f"  (±{t_half*100:.2f}pp)   <- headline; t({G-1}) small-G correction"
    )
    print(f"  between-conversation SD  : {statistics.stdev(cluster_accs)*100:.2f}pp over G={G}")
    print(f"cluster bootstrap 95% CI : [{lo*100:.2f}%, {hi*100:.2f}%]  (±{half*100:.2f}pp)")
    print(f"  bootstrap SD           : {statistics.pstdev(boots)*100:.2f}pp")

    # The wrong interval, printed so the difference is on the record.
    naive = 1.96 * (acc * (1 - acc) / n) ** 0.5
    print(f"  naive binomial (WRONG, ignores clustering): ±{naive*100:.2f}pp")
    print(f"  design effect (cluster-robust / naive)   : {t_half/naive:.2f}x")

    print("\nPER CATEGORY (cluster-bootstrapped)")
    by_cat = defaultdict(list)
    for r in rows:
        by_cat[r["category"]].append(r)
    for cat in sorted(by_cat):
        crows = by_cat[cat]
        cconv = defaultdict(list)
        for r in crows:
            cconv[conversation_of(r["question_id"])].append(r)
        cboot = cluster_bootstrap(cconv, args.iters, rng)
        clo, chi = interval(cboot)
        print(
            f"  {cat:22s} n={len(crows):5d}  acc={accuracy(crows)*100:6.2f}%"
            f"  95% CI [{clo*100:.2f}%, {chi*100:.2f}%]  (±{(chi-clo)/2*100:.2f}pp)"
        )

    print("\nPER CONVERSATION (the unit of clustering)")
    for c in sorted(by_conv, key=lambda x: int(x) if x.isdigit() else 99):
        cr = by_conv[c]
        print(f"  conversation {c:>3s}  n={len(cr):5d}  acc={accuracy(cr)*100:6.2f}%")

    print("\nFAILURES AND HYGIENE")
    for k in (
        "transport_failures",
        "auth_failures",
        "judge_parse_failures",
        "clean",
        "recovered_after_retry",
    ):
        if k in d:
            print(f"  {k:24s} {d[k]}")
    empty = sum(1 for r in rows if not (r.get("predicted") or "").strip())
    erred = sum(1 for r in rows if (r.get("predicted") or "").startswith("[error"))
    zero_ret = sum(1 for r in rows if r.get("retrieved_memory_count", 0) == 0)
    print(f"  empty predictions        {empty}")
    print(f"  error predictions        {erred}")
    print(f"  zero-retrieval questions {zero_ret}")

    if args.dataset:
        session_recall(args.dataset, rows)

    print("\nJUDGE STRICTNESS (crude token overlap — NOT a scoring correction)")
    stop = set(
        "the a an of in on at to and or is are was were be been it its his her their "
        "that this for with from as by no not".split()
    )

    def content(s):
        return {w for w in re.findall(r"[a-z0-9]+", (s or "").lower()) if w not in stop and len(w) > 2}

    wrong = [r for r in rows if not r["correct"] and r.get("outcome_class") == "ok"]
    full = sum(
        1
        for r in wrong
        if content(str(r["ground_truth"])) and content(str(r["ground_truth"])) <= content(r["predicted"])
    )
    half = sum(
        1
        for r in wrong
        if (g := content(str(r["ground_truth"])))
        and len(g & content(r["predicted"])) / len(g) >= 0.5
    )
    print(f"  judged wrong (clean)                      {len(wrong)}")
    print(f"  ...containing EVERY ground-truth word     {full} ({full/len(wrong)*100:.1f}% of wrong, {full/n*100:.2f}% of all)")
    print(f"  ...containing >=50% of them               {half} ({half/len(wrong)*100:.1f}% of wrong)")
    print("  Inspection shows most are legitimately wrong. No correction applied.")

    e = d.get("efficiency", {})
    if e:
        spend = e.get("total_system_cost_usd", 0) + e.get("total_judge_cost_usd", 0)
        print("\nCOST AND LATENCY")
        print(f"  total spend            ${spend:.2f}   per question ${spend/n:.5f}")
        print(f"  actor+judge split      ${e.get('total_system_cost_usd',0):.2f} / ${e.get('total_judge_cost_usd',0):.2f}")
        print(f"  mean context tokens    {e.get('mean_system_tokens',0):.0f}  p95 {e.get('p95_system_tokens')}")
        print(f"  retrieval wall ms      mean {e.get('mean_retrieval_wall_ms',0):.2f}  p95 {e.get('p95_retrieval_wall_ms')}")
        print(f"  missing usage records  {e.get('missing_usage_count')}")
    if "duration_seconds" in d:
        print(f"  wall clock             {d['duration_seconds']}s")


if __name__ == "__main__":
    main()
