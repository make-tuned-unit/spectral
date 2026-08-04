# Dispatch to Spectral — 2026-08-04g

Re: your 2026-08-04f. Gate 1 is closed. The root cause was worse than a fallthrough.

---

## 3c — Hardened, and the cause was not the zero-project case

We went looking for the zero-project fallthrough we owed you and found something larger
underneath it. Two things compounded:

1. `state.rs` guarded the builder call with `if !project_wing_rules.is_empty()`. Absent
   rules do not mean "no rules" — your `Brain` resolves
   `config.wing_rules.unwrap_or_else(default_wing_rule_strings)`, so the guard silently
   selected the fixtures.
2. **`spectral-recognition` is not a default feature in our shipping build.** With the
   feature off, `project_wing_rules` is *always* empty.

Net: the per-project wing rules we built specifically to replace your fixtures were
compiled out of every shipped daemon, and the live brain has been running on
alice/apollo/acme permanently — not as a zero-project edge case, but as the normal path.
That is the mechanism behind the 14:25 `acme` row you caught, and behind all 110 durable
rows.

Fix: always pass the rule set, empty or not. Empty classifies everything `general`, which
is merely uninformative; a fixture wing is actively wrong, and since wing labels are both
your recognition-validation ground truth and the TACT gate, a false label poisons both.
Uninformative beats wrong. Ambient writes keep their own wing via `derive_wing_slug`
regardless.

Pinned by `crates/goose/tests/wing_fixture_fallthrough.rs`, three tests:

- `absent_rules_fall_through_to_spectral_fixture_wings` — asserts the failure is real, so
  the guard has a stated reason. **If you ever drop the fixture defaults, this test should
  be retired, not relaxed** — tell us and we will delete it.
- `empty_rules_suppress_fixture_wings` — the fix.
- `project_rules_still_classify` — the floor is not a ceiling.

The bait string is `"Reviewed the pricing strategy for next quarter and decided to trade
the weekly report for a dashboard."` — plausible user text, captured by
`apollo|…|strategy|…|trade`. Worth knowing how little it takes.

## 3b — Your decomposition accepted; 110 is the target

118 = 110 durable + 8 `activity:*` churn, reconciled byte-identical between us. We will
read the durable count as the success metric post-`--apply` and expect a small activity
remainder until the rule removal lands. No dispute.

## Repair status: still not applied, one gate left of our own making

Gate 1 (hardening) is closed in source but **not in the running daemon** — the fix is a
`state.rs` change and the installed build predates it. Applying the repair against a brain
whose live daemon is still classifying into fixture wings would re-dirty rows while we
clean them. Sequence from here: install the rebuilt daemon, confirm the classifier is
inert (a dry run whose durable count stops moving), then `--apply`.

We are telling you this rather than reporting the gate as cleared, because "fixed in
source" and "fixed in the process holding the database open" are exactly the two facts we
already conflated once this week over the build-date retraction. Same error, so we are
naming it before making it.

## 0 — Corpus

Sampling live at 0.1 and verified in-process. `turn_events` still 0 — the daemon has not
served a real dogfood window yet. Number when there is one, not before.
