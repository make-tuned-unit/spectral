# The consumer gap — Spectral's capabilities are built, benchmarked, populated, and not called

Measured 2026-08-19 against permagent-runtime `main` (pinned spectral rev
`c2c8381`, 2026-07-31) and the real brain at `~/.permagent/brain`
(2,818 memories). Every claim below is verified against source or measured
output; commands to reproduce are at the end.

## The finding

**Recognition has never been called in production.** Not degraded, not
misconfigured — never invoked. And it is not alone: the entire associative and
anticipatory surface of Spectral has zero production call sites in the
consumer.

This reframes the week. R37–R42 measured, tuned and gated an engine that
nothing asks a question of. Those measurements are still correct and were
worth having — but the ordering was wrong. **Wiring beats tuning, and the
wiring is already possible today, at the current pin, with no dependency
upgrade.**

## Recognition is blocked on a premise that stopped being true on 2026-07-12

`crates/goose/src/recognition_sink.rs:5`:

> This module is the SEAM ONLY. Spectral's `Brain::recognize()` (query mode)
> and the session-level stream tracker … **are not in the pinned Spectral rev
> yet**, so nothing here computes a verdict — the call sites are wired and a
> debug-log sink is installed by default, so the day the dep lands the only
> work is conversion + forwarding.

The premise is false, and has been for over a month:

| fact | evidence |
|---|---|
| `spectral::Brain::recognize()` landed on the public facade | commit `f1692f0`, **2026-07-12** |
| Permagent's pin was set to `c2c8381` | **2026-07-31** — nineteen days later |
| `recognize()` is present in the pinned rev's facade | `git show c2c8381:crates/spectral/src/lib.rs` line **452** |
| It is **not** feature-gated | pinned rev's features are only `http-llm`, `neural-bench`, `spectrogram-legacy` |
| The stream tracker is present too | `StreamEvent`, `StreamTracker`, `StreamTracker::observe()` in `c2c8381:crates/spectral-recognition/src/stream.rs` |
| `SafeBrain` has no `recognize` wrapper | zero matches in `brain_handle.rs` |
| The only `.recognize(` in the tree is a comment | `recognition_sink.rs:151` — "run `brain.recognize(stimulus)` here" |

Permagent's own `Cargo.toml` says as much — `spectral-recognition = []` is
annotated *"Enabling requires NO Spectral dep upgrade"* — while the module
docstring says the opposite. The Cargo comment was right; the module comment
governed behaviour.

## The inventory: built, populated, zero call sites

Verified by grepping the whole `crates/` tree for real call sites (comments
and unrelated same-name methods excluded). **All are present in the current
pin** — none requires a dependency upgrade:

| capability | what it does | production call sites |
|---|---|---:|
| `recognize` | "have I encountered this before, and what happened last time?" | **0** |
| `probe` | memories relevant to a *current cognitive state* (ambient text) | **0** |
| `recommend` | anticipatory recall ranked by **lift**, not raw count | **0** |
| `related_memories` | co-retrieval associations | **0** |
| `reinforce` | strengthen memories the caller found useful | **0** |
| `aaak` | foundational facts as a token-budgeted prompt block | **0** |
| `repair_derivations` / `derivation_health` | the one-call fix for the enrolment/signature gap | **0** |
| `verify_hit` | federation provenance verification | **0** |

What *is* consumed: `remember*`, the `recall*` family, `get_memory`,
`set_description`, `list_undescribed`, `consolidation_candidates`. In other
words the product uses Spectral as **a search index with a description
column**, and none of the memory-system behaviour it was built for.

## They work — today, on the real brain

`cargo run --release -p spectral --example unconsumed_capabilities -- ~/.permagent/brain`
(read-only). Live output, abridged:

- **`probe("debugging a failing deploy, checking why the build broke…")`** →
  `permagent_deployment_workflow`, a `DIAGNO…` decision note, a "Debug why…"
  task. On-topic, no LLM, no query written by anyone.
- **`related_memories(seed)`** → associations with `co_count` up to **443**.
- **`recommend(seed)`** → lift-ranked (1.93) anticipatory set.
- **`aaak(max_tokens: 300)`** → a formatted foundational-facts block ready for
  system-prompt injection.

The substrate is not empty and waiting to be populated: **296,526
co-retrieval pairs** already exist. The recommender has been ready to run for
as long as the table has been filling.

## Cost, in production shape

Every published recognition figure, and every probe in R37–R42, used the
in-memory store. Production uses the SQLite sidecar, so a consumer deciding
whether to call `recognize()` on *every* recall needs that number. Measured
with a new instrument (`recognition_latency`) on the real brain, 200 probes:

```
mean 25.3 ms   median 12.9 ms   p90 73.3 ms   p99 86.1 ms   max 90.9 ms
verdicts: Recognized 155 / Familiar 45 / Novel 0
```

Affordable alongside a recall that already costs tens of ms. Zero false-Novel
on enrolled content, consistent with the published `pos_novel = 0`.

## Why this happened — and the process fix

This is the **fourth** instance of the same defect found in one week:

1. `SpectrogramAnalyzer::analyze` never read `Memory::description` (R35).
2. Recognition enrolment never read it either — R36 measured a hypothetical.
3. The graph's producer exists; the consumer was never built (9 triples, zero
   `assert` calls).
4. **Recognition itself is never called, because a comment said it could not
   be.**

The pattern: Spectral ships a capability, the consumer writes a seam in
anticipation, and nobody re-checks when the capability actually lands. Every
instance was invisible to tests, because a seam that is never exercised
cannot fail.

**The fix is a guard, not vigilance.** permagent-runtime already uses exactly
this pattern twice — the phantom-tool guard in `agents::self_knowledge` and
`config::identity_name_guard` — scan the real artefact, fail loudly, make
every exemption a written decision. The analogue here: for each seam that
claims a dependency is unavailable, a test that **attempts the call**. If it
compiles and returns, the seam's premise is false and the test fails, naming
the seam to wire. Written once, it would have caught this on 2026-07-12 and
will catch the next one.

## Recommendation, ordered by value per unit of work

1. **Wire `recognize()`.** `SafeBrain::recognize` wrapper (mirrors the
   existing `spawn_blocking` wrappers, ~15 lines), enable the
   `spectral-recognition` feature, convert `Verdict` at the seam boundary as
   the module already prescribes, forward to `RecognitionSink::on_verdict`.
   The v22 verdict columns already exist. No pin bump.
2. **Add the seam guard** described above, so premise rot is a failing test.
3. **Wire `probe()` / `recommend()`** — the associative surface, against 296k
   already-populated pairs. This is the "living memory" behaviour the product
   markets and does not currently execute.
4. **Bump the pin** (161 commits). Independently justified: the self-healing
   enrolment (#294) and orphan pruning (#293) are *not* in the current build,
   so the enrolment coverage repaired to 100% will decay again exactly as it
   decayed to 43.2% before — the old build records enrolment failures only in
   memory, so they evaporate on process exit.
5. Then, and only then, revisit engine tuning (R42b and friends).

## Reproduce

```
cargo run --release -p spectral --example unconsumed_capabilities -- ~/.permagent/brain
cargo run --release -p spectral --example recognition_latency      -- ~/.permagent/brain 200
git show c2c8381:crates/spectral/src/lib.rs | grep -n "pub fn recognize"
```
