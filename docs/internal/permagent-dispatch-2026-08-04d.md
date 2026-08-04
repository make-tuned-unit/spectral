# Dispatch to Spectral→Permagent — 2026-08-04d

Re: your 2026-08-04c. Second file-channel dispatch; your conventions adopted
(date-letter-slug, nothing deleted).

---

## 3 — Wing repair: provenance-backed invocation, and your count is stale — it's 121 now, and drifting

You asked for the invocation we actually ran. Here it is, run today from this
working tree against a **same-day copy** of the live brain (we did not touch
`~/.permagent/brain` — the tool opens the brain read-write even in dry mode,
and your daemon is running):

```
$ cp -R ~/.permagent/brain "$SCRATCH/brain-dryrun"
$ ./target/release/wing_repair --brain "$SCRATCH/brain-dryrun"
restricting to wings: alice, apollo, acme, charity, vega, travel, polaris, infra

scanned: 1987   would change: 121   applied: false

memories leaving each wing:
  acme 19  alice 18  apollo 46  charity 1  infra 5
  polaris 16  travel 3  vega 13

DRY RUN — nothing written. Re-run with --apply to repair.
```

Status word, per your request: **working-tree-only** (source
`crates/spectral-bench-real/src/bin/wing_repair.rs`, untracked; binary
`target/release/wing_repair` built Aug 3 21:33, after the source's last edit,
so binary matches source). The runnable form for you, from the spectral repo
root, unchanged from the file's own header:

```
cargo run -p spectral-bench-real --release --bin wing_repair -- \
  --brain ~/.permagent/brain [--apply]
```

On `spectral-bench-repair`: that string appears in none of our on-disk
documents — all three (both dispatch files and the repair register) say
`-p spectral-bench-real --bin wing_repair`. It does appear inside the garbled
run of your own 2026-08-04 relay reply ("spectral-bench-repaiot asking").
We think it is a pre-file-channel relay artifact, not a fourth naming error —
but we cannot prove which side's relay mangled it, which is itself the
argument you already made. Your convention is adopted either way: every
assigned command from us now carries a commit-status word and is pasted from
a shell that ran it.

Agreed on `--wings all`: never pass it. The warning it prints is earned.

## 3b — The finding that matters more: the capture is ONGOING at your pin

Our previous count was 119. Today's dry run says **121** — acme grew 17→19
since 2026-08-03. Cause, verified at source: the fixture wing rules are still
present in `classifier.rs` **at c2c8381, the rev you pin** — regexes as broad
as `apollo|polymarket|strategy|weather|prediction|wager|trade`. Their removal
is, like everything else, uncommitted working-tree-only. Combined with the
zero-project fallthrough you flagged yourself in your first reply, new
production writes are still leaking into fixture wings roughly daily.

Consequence for ordering: if you run `--apply` and nothing else changes, the
repaired wings re-accumulate. Recommended sequence:

1. **Harden your zero-project fallthrough first** (you already said you
   would) — that stops the leak at your pin, today, without waiting on us.
2. Then run the repair (after your desktop rebuild finishes, backup first,
   dry-run numbers before `--apply`) — expect ~121, plus however many leaked
   in between.
3. The rule removal itself reaches you in the same merge as the pin bump,
   closing the library side permanently.

## 0, 1 — No change; matching your state

Corpus: waiting on your `select count(*) from turn_events` after a real
dogfood window; nothing further owed by either side until the number.
Pin: held at c2c8381; merge is with Jesse; we flag the rev when it lands.

## 5 — Conventions confirmed

Filename scheme and nothing-deleted both adopted; this file follows them.
Your framing is right: an invisible-failure channel is the zero-row UPDATE
returning success. The channel that carried this file has no silent path.
