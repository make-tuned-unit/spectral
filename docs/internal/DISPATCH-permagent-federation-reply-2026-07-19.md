# Reply → Permagent: federation seam (Rev 2 dispatch) — rulings + guarantee status

**Date:** 2026-07-19 · **Spectral pin for this reply:** `main` at PR #207 merge candidate
(federation object-identity round-trip). **Supersedes the Permagent pin `bd68467b`**, which
predates the federation-sync surface entirely (`export_pack`/`import_pack`/realm landed in
#199 `0939122`, after that pin).

**TL;DR:** §1 ruled **(A) — control objects live in a Permagent-owned parallel namespace;
Spectral's memory-object substrate stays pure.** Guarantees A and B hold in the v1 build; the
authorship invariant is enforced by *Permagent's verify-before-import*, not by Spectral's
crypto-agnostic `import_pack` — one seam detail to confirm. §3b confirmed by construction.
Relay (your control-plane's replication assumption) was broken and is now fixed (#207).

---

## §1 — control-object placement: ruled **(A)**

The three signed control objects (`genesis`, `admin_chain_link`, `realm_keyring`) replicate as
a **parallel grow-only set keyed by `realm_id`, owned and merged by Permagent**, riding the same
have/want + relay shape. They never enter Spectral's memory tables.

Rationale (why not the Spectral control-object kind, option B):

- **Recall exclusion becomes structural, not a flag.** Spectral's invariant is "everything in a
  pack is a memory that flows through recall → ranking → view-scoping." A control-object *kind*
  (B) forces every one of those paths — `recall_scoped`, associative spreading, `merge_and_rank`,
  the view-scoping chokepoint — to special-case "is this control? exclude it." That is exactly
  the shape of the spreading-reinjection leak we already had to fix (Guarantee B's history). In
  (A) control objects are excluded *because they are not in the memory tables* — no per-kind
  flag can be forgotten by a future path.
- **Cross-author merge stays yours.** `admin_chain_link` is authored-by-admin-about-subject —
  inherently cross-author, like a tombstone. In (A) that merge rule lives in your grow-only set.
  In (B) Spectral's merge would need a per-kind authorship exemption threaded through the
  plaintext layer.
- **The negotiation shape already generalizes.** `enumerate` / `missing_locally` / relay are
  generic over "content-addressed hashes." You can reuse the *pattern* for the realm-control set
  without importing Spectral's memory semantics. If it helps, we can factor the have/want
  primitive out generically — say the word; not needed for you to proceed.

**Envelope decision this unblocks (§4):** control objects are **beside** Spectral's pack, not
inside it. Spectral's `Pack` stays `{ wing_id, objects: [MemoryObject], tombstones: [Tombstone] }`.

## §2 — author_id: current contract (ratify against your §3.2/§10.1)

Spectral treats `author_id` as **`Option<[u8; 32]>` — 32 opaque bytes**, never interpreted:

- Hashed into `object_hash` with a presence tag (`1` + 32 bytes if present, `0` if absent), so
  two authors never collide and signed/unsigned are distinct pre-images.
- `None` = unsigned/legacy; hashed with the `0` tag; **untouched** (your "legacy-row no-touch"
  holds).
- Post-#207, the 4-byte `author_short` is **only** a cosmetic local-storage-key fragment and is
  **no longer load-bearing for identity** — uniqueness now rides the object hash. (Pre-#207 it
  silently defeated cross-author accumulation on a 4-byte collision; fixed — see below.)

We can ratify the exact encoding once you paste §3.2/§10.1 (it was truncated in the dispatch).
Spectral's only requirement: the identity is the **full 32 bytes**, opaque to us.

## §3a — have/want manifest transport

`enumerate(wing_id)` returns a **sorted plaintext `Vec<String>` of object hashes**;
`missing_locally(local, remote)` computes the want set. **Spectral defines no transport** — it
hands you the hash list. Where/how it moves, and any size-bucketing/padding, is your layer.

Honest-metadata disclosure to set for a buyer: the list length is the **exact** live object
count for the wing, and the entries are the object hashes themselves. If you need to hide count
or cadence, pad/bucket in your transport wrap — Spectral emits the true list.

## §3b — epoch opaque / convergence on content-hash: **confirmed by construction**

`import_pack(&Pack)` consumes **plaintext only** — no envelope, no `epoch`, no `keyring_hash`
fields exist in the type. Convergence keys on `object_hash` = blake3 over source fields
(`author_id, key, created_at, content, visibility, supersedes`) with domain separation and
length-prefixing — no Permagent metadata in the pre-image. Two packs sealed under different
epochs carrying the same plaintext object converge (same hash → `id` dedup). Ranking/indexes are
re-derived locally and are corpus-relative. So "shared content converges, ranking stays local"
holds regardless of which epoch sealed a given pack. ✅

## §4 — restated guarantees: status in the v1 build

- **A — structural export-gate.** ✅ Holds. `export_pack` packs only manifest-referenced objects;
  a never-`share()`d memory is in no manifest and is unexportable. Property test
  `local_memory_is_never_exportable`.
- **B — view-scoping recall, spreading ON.** ✅ Holds. `recall_scoped` filters at the final-output
  chokepoint, after spreading. Test `shared_scope_recall_never_surfaces_a_private_spread_mate`.
- **Authorship invariant at merge.** ⚠️ **Seam detail — confirm.** Spectral's `import_pack` is
  crypto-agnostic and has **no signer concept**; it does not (and cannot, without keys) verify
  `embedded-author == pack-signer`. That check must live in **Permagent's verify-before-import**
  (you "verify + strip the envelope before handing us plaintext," §3b). So the invariant holds
  **iff Permagent enforces it pre-`import_pack`**. If you want Spectral to *co-enforce* on the
  plaintext, the verified signer identity must be passed **into** `import_pack` — a small,
  well-defined API addition we can make on request. Flagging so neither side assumes the other
  owns it.
- **Federation passes the accuracy eval.** ✅ Done — private-only vs private+shared-merged A/B is
  +7pp (BENCHMARKING §5b); per-child cap defaults `Some(20)`; the one temporal regression is
  attributable to displacement (a knob-turn), not a redesign.

## Correctness fixes since your pin (relevant to the control plane)

Your §1 assumes control objects "ride the same have/want + **relay**." At your pin, relay was
**broken**: an imported object could never be re-exported (rebuilt from a synthetic local key with
`supersedes: None`, so its integrity re-hash never matched → dropped from re-export). A→B→C
replication silently failed. Fixed in **PR #207**, which also fixes two adjacent data-integrity
bugs. Bump past `bd68467b` to get:

- **Relay round-trips** — imported objects re-export with their original `object_hash` (persist
  `orig_key`/`supersedes`, reconstruct the wire object from stored identity). *This is the
  guarantee your parallel control-set relay will lean on, whether you reuse our primitive or
  mirror it.*
- **No silent drops** — object-scoped injective local key; a same-author update or a 4-byte
  `author_short` collision no longer overwrites-and-loses under the UNIQUE-key `INSERT OR IGNORE`.
- **Wing-scoped tombstones** — retracting a multi-wing object from one wing no longer destroys the
  copy the other wings serve.

## Terminology map (one drift to note)

Spectral's sync layer names the shared-set identifier **`wing_id`** (`share(mem_key, wing_id)`,
`shared_wing_members.wing_id`); the recall layer exposes it as **`RealmScope`**. Your `realm_id`
maps to this `wing_id`. Not a blocker; noting it so "keyed by `realm_id`" in your parallel set
lines up with our `wing_id` on the negotiation surface. We may unify the naming to `realm_id` in
a later pass.

## Sequencing (your §5, updated)

1. ✅ Spectral realm/pack/OR-Set/tombstone surface — landed (#199) and hardened (#207).
2. **Ruled here:** §1 = (A). **Pending you:** ratify §2 encoding; confirm §4 authorship seam.
3. Permagent builds identity + seal/open against (A) — control objects beside the pack, in your
   grow-only realm-control set.
4. **Pin bump** past `bd68467b` (→ #207) so relay + identity round-trip are present, then replace
   any direct paths with the agreed trait surface.

## Open questions back to you

- §2: paste the exact `author_id` encoding (§3.2/§10.1) so we can ratify; confirm 32-byte opaque
  identity is the contract.
- §4: confirm Permagent owns the authorship-invariant check pre-`import_pack` (or request the
  signer-into-`import_pack` API).
- §1: do you want Spectral to expose the have/want primitive generically for your control-set to
  reuse, or will you mirror it? Either is fine by us.
