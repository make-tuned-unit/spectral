# Track C — clean re-run: the shares replicate to the decimal (2026-08-13)

**$0. `SPECTRAL_INGEST_PROFILE=1`, LoCoMo ingest, 120 questions, 71,379
profiled writes, `topk_fts`, binary `fa5763d`.** This is the clean re-run the
2026-08-10 result demanded before its numbers could be entered as measured.

## Preconditions — held this time, and recorded

| | 2026-08-10 (violated) | this run |
|---|---|---|
| disk | **99% full** | 149 Gi free (8% used) |
| swap | **~14 GB active** | 2.9 GB |
| load (1-min) at start | competing arms same day | **2.93**, no competing work |

Run sandwiched between two `uptime` calls (2.93 before, 3.93 after — the run
itself is the load). Machine is a desktop with a GUI session; "idle" here means
no competing batch work, which is the condition the hypotheses doc meant.

## Result — per-stage profile

| stage | ms/event | share | 08-10 share |
|---|---:|---:|---:|
| **`ingest_call`** (classify + score + hash + episode) | **0.9960** | **85.5%** | 85.6% |
| `sig_write` | 0.0724 | 6.2% | 6.5% |
| `density_write` | 0.0606 | 5.2% | 5.4% |
| `readback` | 0.0254 | 2.2% | 1.7% |
| `sign` (Ed25519) | 0.0092 | 0.8% | 0.7% |
| `density_compute` | 0.0010 | 0.1% | 0.1% |
| TOTAL (measured stages) | **1.1645** | — | (1.9357) |

**Every share replicates within 0.5pp.** The qualitative verdict now stands on
a run that satisfied its preconditions: **H4 confirmed — the write path is
~85% classification/scoring/hashing; H1 refuted — Ed25519 is under 1%.**
Removing every extra round trip still buys ~13-14%, not a majority.
`session_assoc` is again absent from the samples — the R10-adjacent coverage
blind spot (bench sets `episode_id`, never `session_id`) is unchanged.

## Absolutes — better, still host-bound

Total drops 1.94 → **1.16 ms/event**, but this is a **different host** than
every prior register run (Apple M4 Mac mini (16 GB; the shell reports x86_64 under Rosetta, which earlier misled this doc), `/Users/j`), so two things follow:

1. The clean absolute is a valid number **for this host** and is the one to
   cite going forward from here.
2. The 2026-08-03 decomposition's 0.233 ms/event was measured on the old host,
   and 1.16 vs 0.233 (~5×) on different hardware reconciles nothing. **The 8×
   discrepancy stays open** and can only be closed by a clean run on the
   original machine — which no longer exists in this environment. It may
   simply stay open.

## An unreconciled count, flagged

This run profiled **71,379 writes over 120 questions** (~595/question —
consistent with per-question full-haystack ingest, which is what
`oracle.rs` demonstrably does: brains are keyed `brain_{question_id}` and
`--fresh-brains` forces re-ingest). The 08-10 doc says **"14,900 memory
writes"** for nominally the same command. Neither the old dataset file nor
the old tree is on this machine, so the discrepancy cannot be resolved here;
it is recorded rather than explained. It does not touch the shares, which are
per-event.

**Refs:** `ingest-profile-result-2026-08-10.md`,
`ingest-per-event-hypotheses-2026-08-09.md`,
`ingest-gap-decomposition-2026-08-03.md`.
