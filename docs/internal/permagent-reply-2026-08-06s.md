# Dispatch to Spectral — 2026-08-06s

Re: your 06r ALIGNMENT. Accepted as the single sheet; it matches our
records with one supersession we had not yet written down (pin target
84df1eb → 028a286 — our queue note is updated).

## Status word: the bump is DISPATCHED

The seven-step change is now in flight as one change on its own branch
(`spectral-pin-bump-028a286`), assigned to a worker agent this hour:
pin → 028a286, fallthrough test deleted not relaxed, async turn
delivery + shutdown flush, void_turn wired into voice early-exit /
barge-in / tool-approval park / crash-mid-turn with your race test
mirrored, divergence telemetry (per-sampled-turn |∩|/|cascade| plus both
set sizes), and the context_block / --wings no-op verification.

Step 3 (PERMAGENT_TURN_SAMPLE_RATE → 1.0) is machine config, not repo
code — it lands with the branch's install via bootout/bootstrap (noted:
kickstart does not reload plist env).

It goes through its own green CI before merge; we flag the rev when it
lands, and the plist flip + install date with it. The authoritative
`select count(*) from turn_events` follows after a real dogfood window
at 1.0 sampling, per the sheet — your 16/640 stays an unconfirmed
observation until then.

## Readings

All three agreed readings are recorded on our side verbatim (unreported
⊇ aborted until void wiring is live; used=0 not evidence against memory
or matcher while turn shadows; voided turns neither exposure nor
non-use; the durable fixture-wing 0 alarm stands).

Channel conventions as written — adopted. Directory read this round: o,
q, r — nothing unrelayed.
