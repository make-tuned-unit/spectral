# Context rendering moved into the library — 2026-08-02

## The gap this closes

PR #237 (2026-07-31) moved the retrieval *policy* — `QuestionShape`,
`RetrievalRoute`, per-shape cascade profiles — from `spectral-bench-accuracy`
into `spectral::policy`, on the argument that

> a result is only meaningful if the configuration that produced it ships.

That argument applies with equal force to **rendering**, and rendering did not
move. Retrieval decides *what* the actor sees; rendering decides *how*. Both
are load-bearing on accuracy.

What the published LongMemEval-S run actually rendered
(`retrieve_cascade` → `format_hits_grouped_capped_dated`):

```
--- Session s1 (2023/02/15) ---
[user] I switched my main laptop to the framework 13 for repairability
[asst] Noted — the Framework 13 is now your main laptop, chosen for repairability.
```

- sessions grouped by episode, ordered by earliest `created_at`
- turns within a session ordered by key (turn sequence)
- an absolute session date on every header
- role tags (`[user]` / `[asst]` / `[turn]`)
- **short assistant filler suppressed** (`< 40` chars — "Hi!", "Sure!")

What the library's only public formatter emitted
(`spectral_tact::format_context_block`):

```
[wing/hall] key: content
```

No dates, no grouping, no role tags, no filler suppression. A consumer calling
`recall_local` and injecting the result got a materially different prompt from
the benchmarked one **even with byte-identical retrieval**.

## What moved

New module `spectral::render`:

| item | what it does |
|---|---|
| `RenderOptions` | explicit config: `cap_frac`, `question_date`, `relative_offsets`, `show_descriptions` |
| `session_grouped` | the published format above |
| `flat` / `flat_hit` | `[date] [wing/hall] key: content` — the top-k FTS route's format |
| `relative_offset` / `extract_ymd` | date arithmetic for `"4 months ago"` annotations |
| `cap_content` | char-boundary-safe assistant-turn truncation |
| `with_description` | the provenanced `[librarian: …]` gloss |

## The seam: library owns the algorithm, harness owns the levers

`spectral::render` **never reads the environment**. Every knob is an explicit
field on `RenderOptions`; `spectral-bench-accuracy` maps its `SPECTRAL_*`
variables onto them in one place (`retrieval::render_options`). This is the
same split #237 established for policy, and it is enforced by a test
(`library_render_does_not_read_the_environment`) that sets the harness's env
vars and asserts the library's output is unchanged.

Consequence: a consumer's rendering is a function of the options they passed
and nothing else.

## Behaviour preservation

The harness's formatting functions are now thin delegations. Its **pre-existing**
test suite — which covers session headers, chronological ordering, turn
ordering, filler suppression, capping, char boundaries, description injection
and the relative-offset buckets — passes unchanged: **130/130**.

That suite was written against the harness implementation, so it is the
byte-identity check: nothing about it was adjusted for the move.

New library-side coverage: 9 unit tests in `render.rs`, plus 4 end-to-end
consumer-path tests in `crates/spectral/tests/render_contract.rs` that go
`Brain::open` → `remember` → `recall_topk_fts` → `render` through the public
API only.

Full workspace suite green.

## Corrections to earlier framing

Two things I had wrong before checking:

1. **The relative-offset feature was NOT part of the published run.**
   `SPECTRAL_DATED_CONTEXT` defaults to off and appears in no run config. The
   published run rendered absolute session dates without offsets. The offsets
   remain available and off by default (`RenderOptions::relative_offsets`).

2. **The published config was cascade + expansion-ON**, per `docs/RESULTS.md`
   (run `cdd793e`, #172) — not a porter-only pipeline. The rendering gap is
   independent of that and holds either way.

## Not done here

`spectral_tact::format_context_block` is unchanged and still what
`recall_at` puts in `tact.context_block`. Redirecting it to
`render::session_grouped` would change output for existing consumers and is a
behaviour change, not a packaging one — it needs its own decision. The new
module is additive.

## Follow-on

The `< 40` char assistant-filler rule is the one piece of rendering that is
really a **retrieval** judgement wearing a formatting hat — it discards
retrieved evidence after the fact. DMF (arXiv 2606.03463) reaches the same
conclusion from the other direction and handles phatic turns with an explicit
scored signal rather than a length heuristic. Candidate for the Phase 3
answerability rerank, where an "acknowledgement-like" penalty belongs.
