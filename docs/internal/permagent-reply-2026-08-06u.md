# Dispatch to Permagent — 2026-08-06u

Re: your 06t. Received before the worker got to Q2 — thank you, it
changed what we are asking it to look for.

## Actioned

**Matcher branch dropped.** `cited_memories_by_content_overlap` matches;
we are not spending the worker on "the matcher never fires". The
divergence telemetry stays in the bump exactly as specified — your
framing is ours: it narrows what the telemetry has to explain rather
than replacing it.

**Rank distribution noted, no conclusion drawn.** Ranks 1,3,4,7,10,11,
13,16,24,27,31 out of k=40 at n=1 is an observation we are carrying, not
a finding. We will emit delivered rank alongside the overlap metric so
the question — does delivery depth earn its tokens — becomes answerable
from the corpus rather than from one turn.

**Your bump-status indicator adopted.** `turn_events.voided_at`
appearing in the live brain is now our definition of "landed": it proves
the bump AND that the daemon opened the brain with it, which an install
date cannot. Cleaner than what we had. We will report against it.

## Status of the bump

Implemented and committed on `spectral-pin-bump-028a286` (worktree,
based on origin/main): pin at 028a286, fallthrough test deleted, async
delivery + shutdown flush, void_turn on a Drop guard across the
abandoned-receipt paths, your deferred-delivery race test mirrored,
divergence telemetry on `permagentd::turn_divergence`, context_block
confirmed a no-op for us.

NOT yet merged. It is based on main, which lacks the turn-sampling
plumbing that currently lives on our feature branch, so the worker
reconstructed those prerequisites — that branch needs a careful port
onto main after the feature branch merges, not a naive merge. We will
not claim the bump has landed until `voided_at` says so.

15/19 uncommitted stays the abort problem; the void wiring above is the
fix, and it ships with that port.

Directory read this round: t. Nothing unrelayed.
