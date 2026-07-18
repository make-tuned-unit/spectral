# Federation sync — "git for federated brains" (Spectral's memory-layer half)

Status: design + v1 core. Scope: the plaintext, local, crypto-agnostic memory
layer. Identity, E2E encryption of exported packs, and transport are Permagent's;
Spectral never sees keys, identity, or the network.

## The shape

A Spectral brain becomes syncable like a git repo:

| git | Spectral |
|---|---|
| blob (immutable, content-addressed) | **memory object** (a memory-version, content-addressed) |
| tree / index | **shared-wing manifest** (the set of member object-hashes) |
| `fetch` (have/want by hash) | **enumerate → pack → import** |
| merge | **OR-Set union of content-addressed objects** (CRDT — converges automatically) |
| `.gitignore` / never-committed | **`Local` realm** — structurally unexportable (the sovereignty invariant) |

A user's recall transparently spans **their private store + the merged shared
wings**, with per-result provenance.

## 1. Realm — scope is its own axis (not `wing`, not `visibility`)

Three orthogonal axes:

| axis | meaning | guarantee |
|---|---|---|
| `wing` | topic (health, work…) | semantic; classifier-derived; re-derived on import |
| `visibility` (exists) | who-may-read | soft, honest-participant read filter |
| **`realm` (new)** | replication boundary: `Local` \| `Shared(wing_id)` | **hard, structural** |

`wing` must not carry sync scope: it would force "one topic == one replication
unit," but you'll want to share *some* health memories, not the topic wholesale.

**The sovereignty invariant lives in `realm`:** a `Local` object is *structurally
excluded from every export enumeration* — it can never be serialized into a pack.
Enforced twice (enumerate filter + pack serializer re-check) and asserted by a
property test.

v1 represents realm as a **manifest table** rather than a column on `memories`
(git-native; a shared wing *references* member objects, exactly like a tree
references blobs). "Share memory K into wing W" = add its object-hash to W's
manifest; "Local" = referenced by no manifest (the default). This keeps the
memory object identical whether shared or not, and makes the export gate a single
join: you can only pack objects a manifest references.

## 2. Content-addressed memory object

`object_hash = blake3(canonical(author_id, key, created_at, content, visibility, supersedes))`

- Covers **source fields only**. Excludes `wing`, `hall`, `signal_score`,
  `content_hash`(legacy), and all **derived indexes** (BM25 postings, MinHash
  sigs, ACR edges) — those are re-derived locally on import.
- Rationale: (a) portability — derived data depends on the importing brain's
  ingest config (stemmer, wing rules, IDF corpus); hashing it would make identical
  content hash differently across brains. (b) smaller on the wire.
- Reuses the existing signing discipline: Spectral already signs
  `(source_brain_id, content_hash, created_at, visibility)`
  (`spectral_core::identity::memory_signing_payload`); the object hash is the
  content-address of that same canonical form, plus `key` and `supersedes`.
- **Identity is (key, author_id)**, not key. Two authors' `deploy-process` are
  different objects (different `author_id` → different hash) → both survive.

## 3. Sync primitives (shape, not signatures)

- `enumerate(wing_id) -> Set<ObjectHash>` — object-hashes the shared wing
  references. **Only manifest-referenced objects; Local memories are unreachable.**
- `export(hashes) -> Pack` — serialize the requested objects' **source fields**
  (a git-pack analog). Re-checks realm; refuses any hash not in a shared manifest.
- `import(Pack) -> merge + re-index` — OR-Set merge into the local store, then
  re-run the ingest pipeline on imported content to rebuild BM25 / MinHash / ACR
  locally.

have/want negotiation (`enumerate` diff) is the caller's loop; Permagent moves
the encrypted pack over the wire.

## 4. Deterministic cross-author merge — OR-Set of immutable objects

Convergence is automatic because objects are immutable and content-addressed
(union is commutative/associative/idempotent; same content = same hash = dedup):

- **Cross-author same key → accumulate.** Both objects survive, provenance-tagged.
  No cross-author destructive merge ever runs — that is the convergence guarantee.
- **Within-author update → supersede.** Author A's v2 carries `supersedes:
  <v1-hash>`; among one author's own chain, latest wins (LWW *within a single
  author's timeline only* — conflict-free, since one author owns their sequence).
- **Deletion → tombstone** (OR-Set). A tombstone is itself a content-addressed
  object `(author, target_hash, ts)`, replicated like any blob. Presence =
  added-set − tombstoned-set. Retained so a later re-import can't resurrect.

**Corpus-relative ranking (bank this now):** BM25/IDF and ACR co-occurrence are
computed against each brain's *own merged corpus*, so **content converges
deterministically but ranked output does not** — each user's ranking reflects
their own merged knowledge. Convergence is a property of the object set, not the
result list.

## 5. Recall provenance & scope-spanning

Recall spans a set of realms (private + chosen shared wings). This reuses the
existing **`FederationCoordinator`**: read-time fan-out already tags each hit with
its origin (`LabeledHit.origin`), merges with RRF (poisoning-resistant), caps
per-origin volume, and degrades gracefully. Extend "N brains" → "private store +
N shared wings"; results carry `origin = (wing_id, author_id)` so the agent/UI
distinguish team knowledge from mine.

**This merge step is load-bearing, not polish.** A shared wing where five
teammates each wrote "the deploy process" hands the actor five memories, and the
hard lesson of Spectral's eval arc is that *more context distracts the actor*. The
RRF + per-origin-cap + provenance merge is what keeps accumulation from *lowering*
answer quality.

## ACR across the private↔shared boundary

Activation may cross private↔shared **locally, for ranking power** — but two
invariants close the privacy subtlety:

1. **Returned results are realm-filtered.** A shared/team-scoped recall lets a
   shared memory *activate* a related private one to improve ranking, then drops
   the private mate from the output. (Already shipped: `associative_spread` takes
   the scope and filters — the visibility-leak fix.)
2. **ACR edges are never exported** — free, because indexes are re-derived on
   import, never shipped. A private↔shared edge encodes a private memory's
   existence; since only source content of *shared* objects crosses the wire, that
   edge is computed locally and never leaves.

## Deletion / revocation — the honest boundary

- Spectral provides the **mechanism**: content-addressed tombstones + OR-Set
  semantics + local purge (hard-delete already cascades to FTS + fingerprints via
  AFTER-DELETE triggers).
- **Who may tombstone whom is Permagent's policy** (identity/authorization). An
  author retracting their own memory is always safe.
- **You cannot un-share what replicas already hold.** A tombstone means "logically
  retracted, purge locally." True revocation of already-distributed *content* is
  Permagent's crypto layer (revoke the decryption key). Tombstone + key-revocation
  together give real revocation; neither alone does.

## Boundary

- **Spectral:** object model, content-addressing, realm + export gate, sync/merge
  primitives (OR-Set), tombstones, scope-spanning recall + provenance — all on
  plaintext local objects.
- **Permagent:** identity (who's a teammate), E2E crypto of exported packs,
  transport, and the who-may-tombstone-whom policy. Crypto-agnostic by
  construction: Spectral emits plaintext packs to encrypt and re-indexes whatever
  plaintext is handed back.

## v1 build status

Implemented + tested in `crates/spectral-graph/src/federation_sync.rs`:
canonical object hash, shared-wing manifest, `export_pack`/`import_pack`, OR-Set
union, tombstones, and the two load-bearing proofs — **sovereignty** (a `Local`
object never appears in any pack) and **convergence** (two brains importing the
same packs reach the identical shared object set). Scope-spanning recall via the
coordinator and have/want negotiation are the next slice.
