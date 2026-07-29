# PRE-REGISTRATION — deletion guarantees proof suite (2026-07-29)

Committed before the test suite is built. Addenda only, no rewrites. Same
discipline as the recognition proof suite (PR #229): claims first, tests
enforce them, a claims-gate pins any public statement to evidence.

## Why deletion

Verified deletion is Spectral's least-contested differentiator: Phase-0 named
it a winning axis vs MinHash+BM25, the 2026 research sweep found no published
agent-memory system that proves it (vector stores structurally cannot — 
embeddings of deleted content persist in index geometry), and it is the axis
consumers can least afford to take on faith. It deserves proof, not prose.

## Claims under test

- **D1 — Completeness.** `forget(key)` removes the memory from EVERY
  substrate: primary store, FTS index (including FTS5 shadow tables),
  recognition sidecar (landmarks/pairs/grams/minhash), graph triples and
  entity links derived from it, constellation fingerprints, descriptions,
  wing caches. Enumerated, not sampled — the test derives the substrate list
  from the schema (any table containing the memory id or content column),
  so a NEW substrate added later that isn't wired into forget FAILS the
  test by construction.
- **D2 — Verification, not assumption.** The `ForgetReport` probes
  (recall_clear, recognize_clear, VerificationStatus) must be load-bearing:
  a deliberately sabotaged deletion (test re-inserts a residue into one
  substrate mid-forget) must be DETECTED and reported as failed
  verification, not reported clean.
- **D3 — Residue resistance (adversarial).** After forget, the deleted
  content must be unreachable through every side door we can think of:
  (a) FTS prefix/phrase search on distinctive tokens of the deleted content;
  (b) recognition probe with the deleted content verbatim AND with a 30%
  degraded copy (the re-encounter condition) — no Recognized verdict naming
  it, no evidence rows citing its features;
  (c) graph queries for triples/aliases minted from the deleted memory;
  (d) recall through co-retrieval/associative paths seeded by neighbors.
- **D4 — Physical residue boundary (the honest hard part).** SQLite retains
  logically-deleted bytes in free pages and WAL until checkpoint/vacuum.
  Claim precisely: logical unreachability is immediate; PHYSICAL erasure
  requires `forget_hard`/vacuum. Test: enroll a memory containing a unique
  sentinel string, forget it, run `wal_checkpoint(TRUNCATE)` + `VACUUM`,
  then byte-scan the raw .db/.wal files — the sentinel must be ABSENT. If
  no vacuum-path API exists, that is a FINDING: the gap ships as a
  documented boundary plus an issue, never as an implied guarantee.
- **D5 — Federation tombstones (scoped).** A forgotten shared-wing object
  does not resurface via have/want replication after deletion (tombstone
  honored by the replicated-set primitive, PR #210/#207 semantics). If the
  current primitive cannot express this, record the finding and scope the
  public claim to single-brain deletion.

## Pre-registered expectations

- D1, D2, D3(a–c) pass against current main — the machinery exists
  (ForgetReceipt per-substrate counts, verification probes). Failures are
  findings to fix, not reasons to soften claims.
- D3(d) unknown — co-retrieval/spreading paths were never deletion-tested.
- D4: predicted PARTIAL — we expect no explicit vacuum API today; the
  sentinel byte-scan after manual VACUUM should pass, and the missing
  `forget_hard`-style API becomes a scoped follow-up.
- D5: predicted PASS for tombstone non-resurrection on the generic
  replicated set; unknown for relay paths.

## Deliverables

1. `crates/spectral-graph/tests/deletion_guarantees.rs` (+ helpers) —
   invariant tests D1–D3 wired to run in CI; D4 byte-scan test; D5 if
   expressible in-repo.
2. Public `docs/DELETION_GUARANTEES.md` — exact claims, exact boundaries
   (WAL/vacuum, federation scope), reproduce instructions.
3. Claims-gate entry: statements in the public doc cross-checked against a
   committed test-inventory JSON (same mechanism as recognition).

Decision rules: any D1–D3 failure is fixed before the public doc ships (or
the claim is narrowed to what passes, with the failure documented). D4/D5
findings ship as documented boundaries. No claim without a test enforcing it.
