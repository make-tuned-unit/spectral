# Shared Project Brain — product spec for Permagent + Spectral

**Date:** 2026-07-10
**Status:** design input for the Permagent collaborator (the "tell Permagent what to build" doc).
**Grounds on:** the federation fundamentals shipped this session (read-only mode, visibility boundary, trust-aware merge, verified `forget`) and the roadmap in [`federation-fundamentals-2026-07-10.md`](federation-fundamentals-2026-07-10.md).

## The user story

> Two people using the Permagent app work on a project together. They connect
> via a **project code**. Both **contribute to a shared federated project
> brain**. When the project concludes, that brain can be **archived, deleted,
> or kept**.

Three verbs to build: **connect**, **contribute**, **conclude** (archive/delete/keep).

## Architecture decision: server-owned project brain

Kuzu is single-writer / single-process (`operational-considerations.md`): two
users cannot co-open one brain directory's *live* files. So the shared project
brain must have exactly one owner process. The clean model:

**The Permagent backend owns the project brain.** Each user's Permagent client
submits **signed contributions**; the backend applies them to the one project
brain (single writer) with the contributor's brain-id as provenance, and serves
recall over it. This is a genuine shared brain, matches the user's words ("*a*
federated project brain" that can be archived/deleted as a unit), and sidesteps
the single-writer constraint.

Contrast (rejected as the primary model): federating each user's *personal*
brain read-only — that's the fan-out we hardened this session, but there's no
single artifact to archive/delete, and "the project brain" would only ever be a
merged view. Keep read-only fan-out as the **offline/local fallback** (a user
working offline recalls over a synced read-only replica), not the main path.

```
  User A (Permagent client)                 User B (Permagent client)
        │ signs contribution                      │ signs contribution
        └──────────────┐                ┌─────────┘
                       ▼                ▼
              ┌───────────────────────────────┐
              │   Permagent backend (owner)    │
              │   project Brain (single writer)│  ← Spectral
              │   verifies sig, applies write  │
              │   serves recall (vis + trust)  │
              └───────────────────────────────┘
```

## 1. Connect — the project code

**Permagent builds:** project creation, the code, redemption UI/flow.
**Spectral provides / adds:** identity + a contributor-grant concept.

- On "New shared project", the backend creates a project brain with its own
  `BrainIdentity` (ed25519, already exists) → the **project brain-id**.
- A **project code** is a signed invite capability: `{project_brain_id, role,
  expiry}` signed by the project brain key. Format and transport (link, QR,
  6-word code) are Permagent's; Spectral just needs to **mint and verify** it.
- User B redeems the code → the backend adds B's Permagent brain-id to the
  project's **contributor set** with a role (`reader` | `contributor` |
  `owner`). This is the access-grant primitive from roadmap §3.3.

**Spectral additions (new, small):**
- `ProjectGrant { brain_id, role, granted_at }` and a contributor set on the
  project brain (persisted table `project_grants`).
- `BrainIdentity::mint_invite(role, expiry) -> InviteToken` and
  `verify_invite(token) -> Result<Grant>` (thin wrappers over existing
  `sign`/`verify`).

## 2. Contribute — signed provenance (roadmap §3.1(i))

**Permagent builds:** client-side signing of each contribution with the user's
Permagent identity; submit to backend.
**Spectral adds:** signature columns + verify-on-apply + attribution in recall.

This is the one piece the fundamentals flagged as the next Spectral PR — and the
shared project brain is exactly why it matters (attribution + anti-poisoning
across contributors):

- Each contributed memory carries `source_brain_id` + `signature =
  sign(user_key, content_hash ‖ created_at ‖ visibility)`. New columns on
  `memories`; `MemoryHit` gains `source_brain_id` (the audit flagged its absence).
- On apply, the backend verifies the signature against the contributor's
  registered pubkey and that the contributor is in the project grant set with a
  writing role. Unsigned / unauthorized → reject (not silently store).
- Recall results are **attributed**: each hit shows who contributed it. The
  trust-aware merge shipped this session (`MergePolicy`: rank-normalization +
  corroboration) already ranks a shared brain's hits so no single contributor
  dominates by self-asserting scores, and cross-contributor agreement floats up.

**Visibility inside a project:** a contributor can mark a memory `Private` (only
me), `Team` (this project), or `Public`. The fan-out/recall visibility boundary
shipped this session enforces it; in the server model the backend passes the
requesting user's context so `Private` contributions never surface to the other
member.

## 3. Conclude — archive / delete / keep (the lifecycle)

**Permagent builds:** the three buttons and archive storage/retention policy.
**Spectral adds:** brain-level export and verified destroy primitives (per-key
`forget` shipped this session is the building block; brain-level does not exist yet).

- **Keep** (default): the project brain persists. Nothing to build.
- **Archive**: snapshot the project brain to a portable, checksummed artifact
  (the SQLite `memory.db`, `graph.kz`, `recognition.db`, identity), then close
  it. Restorable read-only later.
  - **Spectral add:** `Brain::export(dest) -> ArchiveManifest` (consistent
    snapshot + per-file blake3 + a manifest) and `Brain::open_archive(path,
    read_only)`.
- **Delete**: verified destruction (right-to-be-forgotten for the whole
  project).
  - **Spectral add:** `Brain::destroy() -> DestroyReceipt` — iterate all keys
    through the `forget` path shipped this session (so recognition index,
    fingerprints, spectrograms, FTS, co-retrieval, retrieval events are all
    purged, not just rows), then remove the directory, and return a receipt
    with per-substrate counts + a post-destroy emptiness probe. In the server
    model there's one copy, so this is complete; if any read-only replicas were
    distributed (offline fallback), emit a **tombstone** they honor on next sync.

`Brain::forget(key)` + `ForgetReport` shipped today are the per-item version of
this; `destroy` is the same guarantee applied brain-wide and is a
straightforward follow-on.

## Build split (summary)

| Piece | Permagent app/backend | Spectral library |
|---|---|---|
| Project code / invite | create, transport, redeem UI | `mint_invite` / `verify_invite` (wrap existing sign/verify) |
| Contributor set / roles | manage membership UI | `project_grants` table + `ProjectGrant`, grant checks in write/read |
| Signed contributions | client-side signing, submit | signature columns, verify-on-apply, `source_brain_id` on `MemoryHit` — **roadmap §3.1(i)** |
| Shared recall | request with user context | ✅ shipped: visibility boundary + `MergePolicy` trust merge |
| Read-only offline replica | sync/distribute replicas | ✅ shipped: `read_only` open (no mutation, no ambient writes) |
| Keep | default | nothing |
| Archive | storage + retention | `export` / `open_archive` (new) |
| Delete | button + tombstone distribution | `destroy` / `forget_all` receipt (new; per-key `forget` shipped) |

## What's already true after this session (nothing new needed)

- A project brain can be opened **read-only** by anyone without mutating it
  (safe replicas, safe recall over a concluded/archived project).
- Shared recall **filters by visibility** and **resists contributor poisoning**
  via rank-normalized, corroboration-boosted merge (a member cannot dominate by
  self-asserting scores).
- Individual project memories can be **hard-deleted and verified gone** across
  every substrate including the recognition index.

## What Spectral still needs (scoped follow-on PRs, in order)

1. **Signed contributions** (§3.1(i)) — the trust anchor for multi-contributor
   attribution and anti-poisoning. Biggest and first.
2. **Contributor grants** (`project_grants` + role checks) — the "join" target.
3. **Lifecycle primitives** — `export`/`open_archive` (archive) and
   `destroy`/`forget_all` receipt (delete).
4. **Invite tokens** — thin sign/verify wrappers, once (1) lands.

Each is independently reviewable; none blocks the others except (4) wanting (1).
Coordinate the exact grant roles and the archive format with the Permagent
collaborator before building — those are the two interface decisions.

---

## Addendum (2026-07-11): multi-device sync for a single user

A second, distinct topology the product needs: **one user, multiple devices** —
the Permagent macOS desktop app plus a mobile app, sharing that user's brain,
with the mobile app able to control the desktop. This is *not* multi-user
federation (no trust boundary — every device is the same person), so the
poisoning/visibility machinery is not the point here; **sync consistency and
remote control** are.

### Trust model: all-trusted replicas

Every device is the same principal, so all memories are mutually visible
(`Visibility::Private` fan-out — "see everything I own"). Signed provenance is
still useful, but as **attribution** (which device authored a memory), not
authorization. Two workable identity choices:

- **Shared brain identity** across devices (copy `brain.key` to each device):
  every device signs as the same brain — simplest; a memory is "mine" regardless
  of origin device. `device_id` (already on every memory) records which device.
- **Per-device identity** (each device its own `BrainId`): memories are signed
  per device, and the sync layer treats the set of the user's device-brains as a
  trusted group. More auditable; needs a "my devices" trusted set (a degenerate,
  all-`owner` contributor grant set from §1).

Recommend **per-device identity + a trusted "my devices" set** — it reuses the
grant machinery from the multi-user design, gives honest per-device attribution,
and means losing one device doesn't leak the shared signing key.

### Sync: ship the substrates, merge idempotently

A brain is three files (`memory.db`, `graph.kz`, `recognition.db`) + identity.
Sync = replicate memories across devices. Two levels:

- **Read replica (have today):** a device opens a **read-only** copy of another
  device's brain (the `read_only` mode shipped this session — never mutates the
  source, safe to sync a snapshot and open it). Good for "recall on mobile over a
  synced snapshot of desktop's brain."
- **Bidirectional sync (needs a small primitive):** each device writes locally
  and periodically exchanges *new* memories. The idempotent-merge foundation
  already exists — **content-hash dedup** (same content ⇒ `WriteOutcome::NoOp`,
  no clobber) and **signed provenance** (authenticated origin). What Spectral
  should add is an **export-since / import primitive**: `Brain::export_since(ts)
  -> Vec<SignedMemory>` and `Brain::import(signed_memories)` that verifies each
  signature against the device's grant set and applies with content-hash dedup.
  Conflict on the same key across devices resolves last-writer-wins by default
  (the non-destructive write semantics already prevent field clobbering);
  contradiction-aware updates are the §3.4 bi-temporal item.

CRDTs are unnecessary here: single-user multi-device with content-hash identity
+ signed provenance + LWW-per-key is sufficient and matches what production
agent-memory systems do (the research found no CRDT use; semantic/temporal merge
is the norm). Kuzu's single-writer constraint still holds — sync operates on
snapshots/deltas, never two live processes on one dir.

### Mobile controls desktop: RPC, not brain-in-the-app

"Mobile controls desktop" is a **remote-control** channel, largely Permagent's to
build, with two Spectral-supported shapes:

- **Thin client (recommended):** the desktop owns the live brain; the mobile app
  issues commands (`remember` / `recall` / `forget` / `recognize`) to the desktop
  over Permagent's transport, and renders results. Spectral needs nothing new —
  its API is already the command surface. Offline, the mobile app falls back to a
  read-only synced replica for recall.
- **Peer devices:** mobile holds its own brain, writes offline, and
  bidirectionally syncs (above) when the desktop is reachable.

### Build split (multi-device)

| Piece | Permagent app | Spectral library |
|---|---|---|
| Device pairing / "my devices" | pairing UI, transport | reuse grant set (`ProjectGrant` with `owner` role) |
| Sync transport | file/delta sync, scheduling | `export_since` / `import` (verify + content-hash dedup) — new |
| Read-only mobile replica | distribute snapshot | ✅ `read_only` open (shipped) |
| Remote control (mobile→desktop) | RPC transport + UI | ✅ existing `Brain` API is the command surface |
| Update conflicts | — | LWW today; §3.4 bi-temporal for contradiction-aware |

**Net:** the substrate shipped this session (read-only replicas, signed
provenance, content-hash dedup) already covers read-sync and remote control. The
one new library primitive for full bidirectional device sync is
`export_since`/`import` — small, and it composes with the signing + grant work
rather than duplicating it.
