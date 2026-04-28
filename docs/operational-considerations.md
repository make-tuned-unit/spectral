# Operational Considerations

This document covers crash recovery, concurrency, and known limitations
discovered through testing. Read this if you're running Spectral in a
long-lived process (an agent, a daemon, a server).

## Crash recovery

### What survives a crash

Spectral uses two storage engines. Both handle crashes differently:

**SQLite (memory store — `memory.db`)**

SQLite with WAL (Write-Ahead Logging) mode ensures that completed SQL
statements are durable. If your process crashes, every `remember()` call
that returned `Ok(...)` will be there when you reopen the brain.

**Kuzu (graph store — `graph.kz`)**

Kuzu operations (entity upserts, triple inserts) are individually durable.
If `assert()` returned `Ok(...)`, the data survived.

### What doesn't survive

**Partial fingerprint sets (SQLite)**

`Brain::remember()` calls `MemoryStore::write()`, which does one INSERT
for the memory and then one INSERT per fingerprint — without wrapping
them in an explicit transaction. If the process crashes between the memory
INSERT and the last fingerprint INSERT:

- The memory exists and is retrievable via FTS search.
- Some fingerprints are missing, so fingerprint-based retrieval may not
  find it until the next `remember()` call pairs it again.

This is a durability gap, not corruption. No data is invalid; some
retrieval paths are temporarily degraded.

**Partial graph assertions (Kuzu)**

`Brain::assert()` does three separate operations: upsert subject entity,
upsert object entity, insert triple. If the process crashes between them:

- Entities may exist without their connecting triple ("dangling entities").
- Since `upsert_entity` is idempotent, re-running the same `assert()`
  after recovery will complete the operation.

### Recommended recovery pattern

After an unclean shutdown, simply reopen the brain and continue. There
is no need for a repair step. If you track which operations were in
flight, re-running them is safe — all write operations are idempotent.

## Concurrency

### Single process, multiple threads

**Safe.** A single `Brain` instance can be shared across threads
(wrapped in `Arc<Brain>`). All operations take `&self`, not `&mut self`.

- **SQLite**: Serialized via `Arc<Mutex<Connection>>`. Threads take
  turns; no parallel writes.
- **Kuzu**: Creates a fresh `Connection` per operation. Kuzu serializes
  writes internally.

This means concurrent writes are *correct but sequential*. If throughput
matters, batch your writes rather than spawning many threads.

### Multiple processes, same brain

**Not recommended.** Two separate processes opening the same `data_dir`
create two separate Kuzu database handles and two separate SQLite
connections.

- **SQLite**: Handles multi-process access correctly via WAL file locks.
  Both processes can read and write safely.
- **Kuzu**: Does not reliably support multiple processes opening the
  same database directory simultaneously. The second open may succeed,
  fail, or produce undefined behavior depending on the Kuzu version and
  platform.

**Recommendation:** Use a single writer process per brain. If you need
multi-process access, have one process own the brain and expose it via
IPC (socket, gRPC, etc.).

### Last-write-wins semantics

When multiple threads `remember()` the same key, the SQLite `ON CONFLICT`
clause applies: the last write wins. The final state will be the content
from whichever thread wrote last. This is correct but non-deterministic
if ordering matters to you.

## Known limitations

| Limitation | Impact | Workaround |
|---|---|---|
| No explicit transactions in `MemoryStore::write()` | Partial fingerprint sets after crash | Re-run `remember()` to regenerate |
| No transaction wrapping in `Brain::assert()` | Dangling entities after crash | Re-run the `assert()` |
| Single `Mutex` serializes all SQLite access | No write parallelism | Batch writes, accept serialization |
| Kuzu doesn't support multi-process access | Second process may fail or corrupt | One writer process per brain |
| No read replicas or snapshots | Can't serve reads during heavy writes | Accept serialized access |

## Production deployment recommendations

1. **One writer per brain.** Don't open the same `data_dir` from multiple
   processes. Use IPC if you need multi-process access.

2. **Handle errors, retry operations.** All Brain write methods are
   idempotent. If a write fails or the process crashes, retry on next
   startup.

3. **Back up before upgrades.** The SQLite and Kuzu files are the brain's
   state. Copy `data_dir` before upgrading Spectral versions.

4. **Monitor disk space.** Both SQLite WAL files and Kuzu data directories
   can grow. Kuzu in particular produces large intermediate files during
   schema operations.

5. **Use `recall_local()` for internal queries.** Only use visibility-
   filtered `recall(query, context)` when serving external/federated
   requests.
