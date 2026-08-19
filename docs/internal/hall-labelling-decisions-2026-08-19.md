# Hall labelling — ground truth from the data owner, and what it corrects

Session with Jesse, 2026-08-19. Replaces the 50-memory labelling sheet: rather
than label instances, he ruled on the **content classes** that dominate the
brain, which generalises to thousands of memories and yields a deterministic
oracle to score any classifier against.

Corpus: `~/.permagent/brain`, 2,846 memories.

## The rulings

| class | n | hall **today** | ruling |
|---|---:|---|---|
| ambient — automation runs, browser navigation, project switches | 754 | 100% `event` | **Should not be memories.** Prune or consolidate into one daily summary of what happened in the app. |
| task records — `Task [completed] via claude-code` + description | 245 | 100% `event` | **`event` — already correct.** |
| approvals — `X was asked: … He answered: approve` | 82 | 100% `event` | **`fact`.** All 82 are misfiled. |
| chat turns — `User: … Assistant: …` | 954 | 705 `event`, 118 advice, 70 fact, 51 preference, 10 discovery | Judge **per turn**; but measure the 27B's self-consistency first. |
| other | 782 | mixed (429 `event`) | not ruled |

## What this corrects in our own record

`brain-substrate-audit-2026-08-17.md` and the neurotopology memo both treat
"77.8% of memories matched no hall rule" as a coverage failure. **That framing
is too strong.** Decomposed against the owner's ground truth, the 2,215
`event` memories are:

- **999 arguably correct** — ambient (754) and task records (245) genuinely are
  events;
- **82 outright wrong** — the approvals, and cheaply fixable;
- **705 undifferentiated** — chat turns, the only class where per-instance
  judgement is actually needed;
- 429 other, unruled.

So the honest statement is not "77.8% is a coverage failure" but "**45% of the
`event` bucket is correct, 3.7% is wrong, and 32% is the one class that needs
judgement**". R40's value is real but narrower and differently shaped than the
audit implied, and the cheap part of it needs no model at all.

## The approvals fix — measured, not assumed

The `fact` rule matches `decided|chose|switching to|using|will use|agreed|
locked in`. It does not match `approve`, which is why 82 explicit decisions —
the highest-value memories in the brain — sit on the fallback.

The obvious fix is wrong, and measuring it showed why:

| candidate rule | memories moved to `fact` | verdict |
|---|---:|---|
| word match `\bapprove[ds]?\b|\brejected\b|\bdeclined\b` | **165** | **over-matches by 106** — mostly `Task [completed]` records whose long descriptions happen to contain "approved". That is the class the owner ruled must stay `event`. |
| structural `was asked:.*answered:` | **82** | exact. Zero over-match. |

Outcomes in those 82: 54 `approve`, 25 `reject`, 3 routing decisions. A
rejection is a decision too, so `fact` is right for all of them.

**Three ways to apply it, best first:**

1. **Set the hall at the write site.** `decision_inbox/learn.rs` already knows
   it is writing a decision and already passes `wing: Some(...)`. With
   `RememberOpts.hall` (PR #298) it passes `hall: Some("fact")` — exact, no
   regex, no model, no inference. This is what that API was built for.
2. **A domain `hall_rule`** on the structural pattern, supplied by the consumer
   via `BrainConfig::hall_rules`.
3. Not a Spectral default. The phrasing is Permagent's decision-inbox wording;
   fitting library defaults to one consumer's phrasing is exactly the mistake
   the retired wing fixtures made.

Existing rows need `Brain::set_hall` (PR #298) to be corrected; the hall is
computed once at insert and, note, **is not recomputed when content is
updated** (`ContentUpdated` preserves every field but content) — a smaller
effect worth knowing.

## R46 — ambient daily rollup (queued)

Owner's ruling: ambient records should collapse into **one daily summary of
what happened inside the app**, and **the raw records are then deleted** —
summary only, not kept alongside.

Machinery already exists in Spectral and has zero consumers:
`CompactionTier` (`Raw`/`HourlyRollup`/`DailyRollup`/`WeeklyRollup`),
`consolidate_as`, `consolidate_extractive`, `list_unconsolidated`.

**Load-bearing constraint:** at 754 records per period this is deletion at
scale, and deletion at scale is exactly where the orphan bug lives. It must go
through `Brain::forget()`, never a raw `DELETE FROM memories` — the FK cascade
cannot reach `recognition.db`, so a raw delete would orphan ~24 recognition
rows per record (measured: 2 pruned memories left 49). `Brain::recognition_residue`
(PR #302) is the postcondition to assert afterwards.

Second constraint: the summary must be **written before** the raw records are
deleted, and the deletion gated on the summary existing. This is irreversible —
the owner accepted that trade explicitly, choosing summary-only over any
retention window.

Priority: **after** the recognition wiring and the seam guard. Not urgent.

## Chat turns — blocked on a measurement, deliberately

705 of 954 sit undifferentiated on `event`. Per-turn judgement needs an LLM
call per memory, and the local `qwen2.5:7b` agreed **with itself** only 51%
across two prompt variants (measured 2026-08-19, n=100). The owner's ruling:
**measure the Qwen3.8-27B's self-consistency first**, in the nightly window,
and decide after. If the 27B is not markedly more consistent, per-turn hall
judgement is not viable at any price and should be reported as such rather
than shipped noisy.
