# Federation fundamentals — audit, fixes, and the research-grounded v2 roadmap

**Date:** 2026-07-10
**Inputs:** full-code federation audit (this session, against main + porter-default working tree); adversarially-verified deep research sweep on 2025–2026 shared/federated agent memory (105-agent harness; claims below carry their verification votes).

## 1. What the audit found (and what is now fixed)

Federation v1 (PR #178) is a correct **single-user, in-process merge utility**. For
multi-user federation ("multiple Permagent users contributing to a federated brain"),
four load-bearing gaps were found. Two are fixed in this tree; two are design work.

| Gap | Status |
|---|---|
| **(a) Fan-out skipped visibility entirely** — `recall_cascade` applies no filter; children's Private memories crossed the boundary (v1's own tests exercised this) | **FIXED**: `fan_out_recall` now takes a `Visibility` context and filters coordinator-side. Multi-user → `Team`+; own-brains → `Private`. |
| **(b) Open and recall both WROTE to child brains** — open ran migrations (incl. whole-index FTS rebuild on tokenizer mismatch → rebuild ping-pong between differently-configured processes); every cascade recall bumped members' `signal_score` (+0.01/hit, decay-clock reset) and logged the caller's `query_hash`/`session_id` into the member's `retrieval_events` (a query side-channel written into the victim's DB) | **FIXED**: `read_only` mode — driver-level RO (SQLite flags + `query_only`, Kuzu engine RO), zero migrations/backfills on open, ambient writes skipped, 19 write APIs → `Error::ReadOnly`. Coordinators over foreign brains must open children read-only. |
| **(c) Deletion is a soft filter** — `consolidate_into` hides sources from only 2 read paths (FTS/fingerprint); rows stay reachable via AAAK, `get_memory`, wing listings, the recognition index (no unenroll exists), spectrograms. No `Brain::forget`. | OPEN — backlog item exists; API shape to be coordinated with the Permagent collaborator. See §3.2. |
| **(d) No trust/anti-poisoning layer** — ranking is `signal_score × weight` where a member fully controls its own scores/timestamps/content; a malicious peer writing score-1.0 keyword-stuffed memories dominates every fan-out. ed25519 identity exists but signs nothing. | OPEN — see §3.1; this is where the research is most decisive. |

Also fixed: registry/children drift (duplicate `add_brain` of one BrainId double-counted
hits), and the spectrogram wing-corpus query is now bounded (top-256 via cached
`wing_search`) instead of materializing whole wings per ingest.

Still-standing structural constraints (documented, not fixed):
- **Kuzu is single-process** — the coordinator opening a member's live brain dir is
  UB; v1 is only coherent when the coordinator is the sole process on all N dirs.
  Multi-user federation therefore implies **replica dirs** (sync'd copies opened
  read-only), not shared live dirs — which the read-only mode now makes safe.
- **Issue #153** (Linux `Brain::open` abort) remains internally contradictory across
  docs and must be resolved before any Linux federation claim.
- Visibility labels are **self-asserted** by the data owner and enforced
  coordinator-side: honest-participant privacy, not mandatory access control.
- `AAAK`, `probe`, raw accessors, and the recognition sidecar still have no
  visibility notion (single-brain concern once fan-out uses the boundary; recognition
  `Evidence.feature` carries verbatim content runs — never federate `recognize()`
  output without a policy).

## 2. What the research says (verified claims, votes in parentheses)

1. **Memory poisoning is the defining threat of shared agent memory, and it is
   cheap.** Query-only injection by an ordinary user (MINJA, NeurIPS/ICML 2025):
   98.2% injection / 76.8% attack success; ≥80% ASR at <0.1% corpus poisoning
   (AgentPoison, NeurIPS 2024); poisoned memories persist across sessions ~50% ASR
   without further attacker involvement (MPBench 2026). OWASP formalized this as
   ASI06 "Memory and Context Poisoning". (3-0)
2. **Detection/content-based defenses fail.** Fluent "weak-signal" payloads evade
   prompt-injection detectors (84%→42% detection drop); keyword/entropy/anomaly
   heuristics are 100% bypassed by fluent enterprise-style text; trust-scoring and
   lineage tracking are laundered through the agent's own summarization at 47–68%
   ASR. (3-0)
3. **The convergent 2026 position: security must be anchored at WRITE time** —
   cryptographic provenance + origin-bound authority. HMAC/signature-bound writes
   reduce unsigned-injection ASR from 93–100% to 0% in benchmarks; a (finite-model
   machine-checked) separation result argues content- and lineage-based defenses are
   unsound under laundering. Caveats: single-author preprints, self-designed
   benchmarks, insider (key-holding) adversaries NOT solved — SMSR's certified bound
   collapses at t=3 colluding insiders. (mostly 3-0; one overreach claim killed 0-3)
4. **Access control that works is per-fragment provenance + two-tier
   private/shared with time-evolving grants** (Collaborative Memory, ICML 2025):
   private tier visible only to originator; shared tier selectively; every fragment
   carries immutable provenance (contributor, resources, timestamps) enabling
   retrospective permission checks. (3-0) — Spectral's Private/Team/Org/Public single
   axis has no principal; this is the model to converge toward.
5. **Consistency in production is LLM-mediated semantic merge + temporal
   validity, not CRDTs/LWW**: Mem0 (ADD/UPDATE/DELETE/NOOP decisions, edge
   invalidation), Zep (bi-temporal valid-from/expiry windows; contradiction closes a
   validity window instead of overwriting). No verified production use of CRDTs for
   agent memory. (3-0)
6. **Deletion is incomplete without multi-substrate verification**: one memory
   leaves derivatives in logs, summaries, vector/FTS indexes, reflections, shared
   stores; "Verified Forgetting" = post-deletion membership tests across ALL
   substrates. (3-0) — maps exactly to Spectral's substrate list: memories row, FTS,
   constellation fingerprints, recognition pairs/grams, spectrogram, consolidation
   edges, retrieval_events, kuzu triples, and any federated replicas.
7. Multi-agent shared memory synchronization/access-control is still an **open
   research problem** (PAKDD 2026 survey; 2-1 on the strongest wording) — Spectral is
   not behind a settled art; it is early on a frontier.

## 3. The v2 fundamentals roadmap (ranked)

**Progress (2026-07-10, later same day):** §3.1 (signing + merge-trust) and §3.2
are **landed**. Remaining: §3.3 visibility principals, §3.4 bi-temporal
invalidation, §3.5 ops.

- **§3.1 (i) signed provenance — DONE (PR #1).** Memories are Ed25519-signed at
  write over `(source_brain_id, content_hash, created_at, visibility)`;
  `Brain::verify_hit(hit, pubkey)` authenticates. Core primitives
  (`sign_memory` / `verify_memory_signature` / `memory_signing_payload`),
  `source_brain_id`+`signature` columns + `MemoryHit` fields, post-write
  `set_signature`. Tamper / visibility-escalation / foreign-key-impersonation
  all fail verification (tested at core and Brain level, and end-to-end through
  the public API). The trust anchor a determined signed insider requires.
- **§3.1 (ii)+(iii) merge trust — DONE.** `MergePolicy` with rank-normalization
  (defeats the cheap self-asserted-score dominance/flooding attack: a max-signal
  flood collapses from a wall-at-top to a decaying ramp), corroboration boost
  (independent agreement outranks lone assertion; no self-corroboration), and
  optional per-child cap. Pure `merge_and_rank`, unit-tested. Default-safe.
- **§3.2 forget — DONE.** `Brain::forget(key)` → verified multi-substrate hard
  delete with a `ForgetReport` (per-substrate counts + post-delete recall/
  recognize probes). Built the missing recognition-index unenroll
  (`RecognitionStore::forget_memory`). This closes the "recognition index has no
  unenroll" gap the audit flagged.

**Open finding (surfaced building PR #1):** `FederationCoordinator` operates on
the inner `spectral_graph::brain::Brain`, **not** the umbrella `spectral::Brain`
wrapper, so federation is not reachable through the umbrella API a normal
consumer uses. Either expose federation on the wrapper or document the
graph-crate entry point. Fold into PR #2 (contributor grants) since that also
touches the coordinator.

**Verification tooling:** the signed contribution + trust merge + visibility
boundary flow is validated end-to-end through the public package boundary
(consumer-crate demo): signed hits verify, tampering/foreign-key rejected,
Team-context fan-out blocks private memories, corroborated content surfaces from
both contributors.

**Merge upgraded to RRF + scored (2026-07-10, later):** the custom
rank-normalization + corroboration boost was replaced by **Reciprocal Rank
Fusion** (Cormack et al. 2009) — the field-standard rank-fusion primitive, which
subsumes both anti-flooding (ranks on position, not self-asserted score) and
corroboration (sums across *distinct* origins; deduped per origin so a member
can't self-corroborate by flooding identical copies). `MergePolicy` is now
`{ fusion: FusionMethod::{Rrf{k}, RawScore}, per_child_cap }`. This is the
"adopt a widely-accepted standard to lift ours while keeping differentiation"
move — RRF is the standard, signed provenance + caps are the differentiator.

**Scored credibility (two benchmarks, `spectral-bench-real`):**
- **Poisoning ASR** (`poison_bench`, accepted MPBench/AgentPoison metric):
  **87.5% → 0.0%** (undefended raw merge → shipped RRF), 87.5pp reduction, on a
  shared-project-brain with 2 honest members + 1 flooding attacker. The
  benchmark *caught a real self-corroboration bug* in the first RRF cut (fixed +
  regression-tested).
- **Recognition re-encounter AUC** (`recognition_bench`): familiar-vs-novel
  ROC-AUC **1.000** on a clean ops corpus, all 40%-degraded re-encounters still
  Familiar, all novels flagged Novel. Honestly scoped to the lexical/re-encounter
  regime (the engine's strong suit); the at-scale ~0.94 and the MinHash
  comparison live in RECOGNITION_BASELINE.md; semantic paraphrase not claimed.

**Next widely-accepted integration (recognition):** RECOGNITION_BASELINE shows
MinHash wins the lexical regime (0.998 vs peak-pair 0.941) and notes "the
verdict/trace wrapper could sit on top of MinHash." Adding a MinHash feature
channel to the recognition engine — keeping the auditable verdict/evidence
layer — would lift the recognition score to competitive-or-best while preserving
the differentiated auditability. This is the recognition analog of the RRF move.

### 3.1 Signed provenance at write time (the anti-poisoning fundamental)
The ed25519 `BrainIdentity` exists and signs nothing. The research-aligned move:
- Sign each memory at `remember` time: `sig = sign(brain_key, content_hash ||
  created_at || visibility)`; store alongside provenance columns (memories already
  carry `content_hash`; add `source_brain_id`, `signature`).
- At fan-out merge, verify signatures against the member's registered pubkey;
  unsigned/invalid → drop or quarantine-label. This makes provenance authenticated
  rather than "trusted because the coordinator opened the directory."
- **Rank on coordinator-side trust, not member-side self-assertion**: a member fully
  controls its own `signal_score`. Merge should (i) normalize per-child ranks instead
  of trusting raw scores, (ii) cap per-child contribution (top-k per member), and
  (iii) weight by contributor reputation earned from corroboration (multiple
  independent members asserting compatible facts), never by self-asserted scores.
  Insider poisoning (a validly-signed malicious member) is NOT solved by any
  verified defense — cap + corroborate + audit is the honest posture.

### 3.2 `Brain::forget(key)` as Verified Forgetting (not row delete)
The backlog item is scoped as row + FTS + graph + consolidation traces. The research
says that under-scopes it. The contract worth building (coordinate API with the
Permagent collaborator first, per backlog):
- Hard-delete across ALL substrates: memories row, FTS (trigger handles), constellation
  fingerprints, **recognition pairs/grams (no unenroll exists today — must be built)**,
  spectrogram row, consolidation edges, annotations; scrub `retrieval_events`
  references.
- Return a **receipt**: per-substrate deletion counts + a post-delete membership
  probe (recall + recognize the deleted content; assert zero hits) — the "verified"
  half.
- Tombstone table for federation: replicas propagate deletion by tombstone on next
  sync; fan-out's read-time semantics then honor retraction without a protocol.

### 3.3 Visibility with principals (from clearance axis to access graph)
`Team`/`Org` of *what* is undefined. Minimum viable evolution, following the
Collaborative Memory model: a memory's visibility record gains an optional grant set
(brain-ids or group ids); the coordinator resolves the querying principal against it.
Keep the current axis as the default; add grants only where needed. Enforce in ALL
read paths (cascade path first — it feeds fan-out; then AAAK/probe if they ever
federate).

### 3.4 Merge semantics: adopt bi-temporal invalidation, skip CRDTs
Spectral already refuses destructive upserts (`WriteOutcome`) and soft-invalidates
via consolidation. The production-validated pattern to converge on is Zep-style
validity windows: `valid_from`/`invalid_from` on memories; contradiction (e.g.
knowledge-update class) closes a window rather than deleting. This also gives
`recall_at` honest historical semantics and dovetails with the temporal-reasoning
bench category. CRDTs: no verified production precedent for agent memory — do not
spend here.

### 3.5 Operational prerequisites
- Resolve #153 (file the upstream kuzu bug with the Linux backtrace) before any
  Linux federation claim.
- Federation replica model: define "member replica dir" (rsync/litestream of
  SQLite + kuzu dir + recognition.db) opened `read_only` by the coordinator — never
  the live dir (kuzu single-process).
- Record the FTS tokenizer in-band (already implicit in `sqlite_master`) and treat
  member tokenizer mismatch as a coordinator-side WARN, never a rebuild (read-only
  mode already guarantees this).

## 4. What was deliberately NOT done
- No re-enable of co-retrieval (measured harmful; needs query-conditioned pairs +
  the shared-brain oracle instrument).
- No `forget()` implementation (backlog says coordinate the API with the deciding
  consumer first).
- No signature scheme implementation (design above; it touches the write path of
  every member and deserves its own reviewed PR).
