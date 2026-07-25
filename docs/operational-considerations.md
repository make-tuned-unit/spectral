# Operational considerations

Spectral stores relational data, memories, full-text indexes, fingerprints, and
derived ranking state in one SQLite database (`memory.db`). Recognition uses a
second SQLite sidecar (`recognition.db`) because it has an independent index
lifecycle. Identity and ontology metadata are files in the same brain directory.

## Durability and recovery

Primary memory writes and their FTS/fingerprint rows are transactional. A
`remember()` call that returns successfully has committed its primary row.
Density, signed provenance, recognition enrollment, and optional spectrograms
are derived immediately afterward. An interruption in that phase can leave a
valid, recallable memory with incomplete derived state.

Use the bounded health probe and idempotent repair API after an unclean shutdown
or upgrade:

```rust,no_run
# use spectral::Brain;
# let brain = Brain::open("./my-brain")?;
let health = brain.derivation_health(10_000)?;
if !health.is_healthy() {
    let repaired = brain.repair_derivations(10_000)?;
    eprintln!("repaired: {repaired:?}");
}
# Ok::<(), spectral::Error>(())
```

Ontology auto-creation is persisted through validated TOML serialization and an
atomic file replacement. A crash sees either the old or new complete ontology.

`Brain::forget` deletes all indexed traces and verifies recall and recognition
afterward. Verification failures are reported as `VerificationFailed`; they are
never counted as successful forgetting by `fully_forgotten()`.

## Concurrency

A `Brain` may be shared across threads. SQLite access is serialized through the
store connection, so operations are correct but write throughput is bounded by
one connection. Batch APIs are preferable to large numbers of tiny concurrent
writes.

SQLite WAL supports multiple processes, but Spectral does not coordinate
application-level read/modify/write operations between independent `Brain`
instances. Use one writer process per brain. Open peer brains with `read_only`
for federation fan-out; this prevents migrations, feedback writes, and query
telemetry from mutating a brain you do not own.

## Visibility

Use `Brain::recall_with` for new integrations. It requires a `RecallOptions`
value with an explicit visibility boundary and runs the integrated cascade.
The older `recall` and `recall_local` methods remain compatibility paths using
basic TACT retrieval. `recall_cascade` is intentionally local/unrestricted;
external and federated callers must use `recall_cascade_scoped`.

## Backups and upgrades

Stop the writer or use SQLite's backup facilities before copying a live brain.
Back up the whole directory so `memory.db`, `recognition.db`, identity keys, and
the ontology stay together. After upgrading, open the brain normally, run
`derivation_health`, and repair any reported gaps.

Monitor database and WAL size. Retrieval events are adaptive-state input and
can grow continuously in long-lived deployments; consumers should establish a
retention policy appropriate to their audit requirements.
