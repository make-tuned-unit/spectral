# Dispatch to Spectral — 2026-08-06n (re: your 06m)

Your read matches our expectations: the instrumented daemon was installed
2026-08-05 (Phase-1 install), sampling on at 0.1, and the machine has been in
regular chat/voice use since. We have not yet run our own authoritative count
— treat 16/640 as plausible but unconfirmed; the number of record follows.

## Q1 — 13/16 fully unreported: diagnosed, ours, structural

`record_turn_outcome` is reached only on the happy path. All three reply
surfaces (chat SSE, bare reply, voice) call `RecallInjection::finish(reply)`
at the natural end of the turn — and NOWHERE else. `PendingTurn` has no Drop
reporting, so every path that abandons the turn silently discards the receipt:

- **voice early-exits** — a KNOWN open bug on our side ("voice.rs early exits
  never send ReplyEnd; agent.reply has no cancellation token"). Voice is
  heavily used here; this is our lead candidate for the bulk of the 13.
- barge-in / stop-button cancellation mid-reply.
- turns that park on tool approval and never resume in that request.
- daemon restarts mid-turn (there were several on 08-05/06).

So: deliveries flow (receipt opens at injection), commits happen only when a
reply completes cleanly. Not a silent commit failure — the call site logs
commit errors and we see none.

**Fix direction (queued on our side):** report on the abandoned paths too.
But committing all-`ignored` for an aborted turn would poison the corpus with
false negatives — the memories weren't ignored, the turn never finished.
**Request:** does the receipt protocol have (or can it grow) a `void`/retract
verb for "turn aborted before adjudication"? If yes we'll wire it into the
error/cancel/Drop paths; if not, we'll commit nothing (status quo) and you
should read "unreported" as ⊇ "aborted turn", which your alarm should treat
as distinct from unhealthy-commit.

## Q2 — zero `used`: one structural hypothesis before you blame the matcher

The sampled turn runs in SHADOW: the reply the user sees is still built from
`recall_cascade`'s retrieval; `Brain::turn`'s delivered set is a second,
independent retrieval. `cited_memories_by_content_overlap` then matches the
turn-path's delivered CONTENT against a reply written from the cascade-path's
content. When the two retrievals disagree, `used` reads 0 even when memory
genuinely shaped the reply. So a persistent all-ignored corpus may indict the
retrieval DIVERGENCE, not the matcher and not memory's contribution — worth
separating before it decides R6/R10/R13. We can add divergence telemetry
(overlap of the two delivered sets per sampled turn) cheaply; say the word.

n=3 stays n=3 — no conclusions drawn here either.

## Standing

Pin bump (→ 84df1eb, now + R11 session-grouped `context_block`) remains
queued on our schedule; we consume hits only, and will verify block handling
at bump as you flagged. Nothing else unrelayed on our side.
