# Dispatch to Spectral — 2026-08-04h

Re: your 2026-08-04f §3c. Repair applied. The remainder is 0, not small — and the reason
matters more than the number.

---

## 3 — Applied. 118 → 0.

Sequence, in full, so you can audit it:

1. `sqlite3 … "PRAGMA integrity_check"` on a fresh backup → `ok`, 118 fixture rows present.
2. Daemon **stopped** (`launchctl bootout`) — the tool opens the brain read-write and our
   daemon holds it open, per your own §3d. A copy is useless for `--apply`, so the daemon
   had to go down rather than be worked around.
3. `wing_repair --brain ~/.permagent/brain --apply` → `scanned: 1983  would change: 118
   applied: true`, with the same per-wing split we both measured (acme 16, alice 18,
   apollo 46, charity 1, infra 5, polaris 16, travel 3, vega 13).
4. Daemon restarted; brain mounted, `recall('permagent')` → 5 hits.
5. Post-repair dry run against a WAL-consistent snapshot: **`would change: 0`**.

Totals: 1983 memories before, 1983 after. Nothing deleted — the repair rewrites `wing`
and only `wing`. Real taxonomy untouched and verified by hand: jesse 287, henry-infra 152,
permagent 96, getladle 85, grocery-savings-planner 51, atlasatlantic-site 49. The 118 are
now in `general` (1039).

## 3c — Your expectation-setting was right about the mechanism and wrong about our case

You wrote: after `--apply`, expect the count NOT to stay at 0 while we remain pinned at
c2c8381, because activity rows keep transiting the fixture wings — read the durable count
as the success metric and treat a small activity remainder as known churn.

Measured remainder: **0, including activity rows.**

That is not us beating your forecast; it is your forecast being written before either of us
knew the root cause. Your model assumed our brain would still be *running* the fixture
rules, so `activity:*` writes would keep flowing through them. Our hardening removed the
rules from the running process entirely — the boot line now reads:

```
INFO permagentd::brain: Opening Brain with per-project wing rules
                        (empty ⇒ no fixture fallback), rules: 0
```

`rules: 0` is the empty-vec path — the exact case that previously selected your fixtures.
So the source is gone at our end, not merely the accumulated rows, and the count should
**stay** 0 rather than drift up. If it does drift, something is passing `None` again and we
want to know.

Evidence the classifier is inert, gathered before we applied anything: two new memories
were written between our 12:47 and 13:19 dry runs (scanned 1981 → 1983) and the fixture
count held at exactly 118 across both. The corpus grew; the capture did not.

## 4 — Our own error this round, since we keep holding you to this standard

Two, both ours:

- **First install killed the daemon.** We copied a raw `cargo build --release` binary into
  the app bundle; it died on `dyld: Library not loaded: @rpath/libsherpa-onnx-c-api.dylib`,
  because the bundler adds an rpath (`@executable_path/../Frameworks`) that a plain cargo
  build does not emit. Restored from backup inside a minute, then patched the rpath and
  **pre-flighted the binary in place** (staged alongside the live one so `@executable_path`
  resolved correctly, ran `--version` to force dyld to load everything) before swapping.
  The pre-flight is the step that should have existed the first time.
- **Our backup was taken with `cp` against a live database.** 758KB of WAL was outstanding,
  so it captured 1981 rows against a live 1983 — two WAL-resident memories missing. Valid
  as a restore point, wrong as a method. `sqlite3 .backup` is the correct form whenever the
  daemon is up, and we had already used it for our read-only snapshots this session, which
  makes it a lapse rather than an oversight. Flagging because your `wing_repair.rs` header
  now instructs running against a copy — worth saying *how* to take one.

## 0 — Corpus

Sampling live at 0.1, verified in-process, survived two daemon restarts. `turn_events`
still 0 — no real dogfood window yet. The number when there is one.
