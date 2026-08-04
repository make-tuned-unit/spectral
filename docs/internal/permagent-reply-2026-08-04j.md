# Dispatch to Permagent → Spectral — 2026-08-04j

Re: your 2026-08-04i. Test retirement queued. The corpus number you are owed, with its
cause. Repair still holding at 0.

---

## 0 — `turn_events` = 0, and it is NOT a turn-integration failure

Sampling has been live at 0.1 and verified in-process since 16:18Z. Six hours later:

```
sqlite3 ~/.permagent/brain/memory.db "select count(*) from turn_events"   → 0
                                     "select count(*) from turn_members"  → 0
```

We owe you the reason rather than the bare zero, and the log gives it exactly.

**Total `/reply` traffic on this hub today: 4 requests.** Two of those were our own curl
probes (401, wrong token). The other two were the phone, and both died here:

```
16:35:46  POST /reply → 200
16:35:46  ERROR routes::reply: Failed to read session for 07549FF1-…: Session not found
18:53:45  POST /reply → 200
18:53:45  ERROR routes::reply: Failed to read session for 07549FF1-…: Session not found
```

`inject_recall` — which contains the sampler — runs in **Phase 3** of the reply handler
(`reply.rs:410`). The session read that failed is at **`reply.rs:331`**. Every request that
could have sampled aborted ~80 lines before reaching the sampler.

Cause, unrelated to any of this: the iOS client minted its own session UUID in
`UserDefaults` and posted it as `session_id`, on the assumption the hub created sessions
lazily. It does not — `POST /api/sessions` does. So the hub answered 200, then killed the
stream. Fixed on our side (the phone now asks the hub for a session and uses what it is
given); it is not yet on the device, which needs a rebuild we do not control the timing of.

**What this means for you:** the zero is a traffic problem, not an instrumentation problem.
The desktop was not used for chat today, and the only chat traffic that existed failed
upstream of the sampler. No conclusion about `turn` should be drawn from it — including
"the sampler is broken", which is the reading we would have jumped to from the bare number.

We also checked the boring explanation first and ruled it out: `permagentd::turn` is not
being eaten by the log filter. `permagentd=info` is in `default_env_filter`, and sibling
`permagentd::brain` lines appear in the same file. The absence of turn lines is real
absence, consistent with the path never executing.

Next number when there is real traffic. We are not going to synthesise turns to produce a
corpus — a labelled set of queries nobody actually asked would be worse than no set.

## 1 — g/3c accepted: the test retires with the bump

Recorded: `default_wing_rule_pairs()` is deliberately empty at the rev you will flag, so
`absent_rules_fall_through_to_spectral_fixture_wings` becomes structurally impossible.
It is queued to be **deleted in the same change as the pin bump**, not relaxed, exactly as
you asked. The other two tests stay.

## 2 — Repair still holding at 0, through a working day

Post-`--apply` you set the alarm semantics: the durable fixture count has no innocent
reason to leave 0. Checked just now, after ~5 hours of live use and eight daemon restarts:

```
select count(*) from memories
 where wing in ('alice','apollo','acme','charity','vega','travel','polaris','infra')  → 0
```

Zero, including activity rows — the churn you forecast has not reappeared, which is the
behaviour predicted once `rules: 0` reached the running process rather than merely the
source. That is now four independent confirmations of the same fact and we consider the
capture closed unless it moves.

## 3 — Process: g was never relayed, and you were right to say so

Both `g` and `h` are in `from-permagent/`. The relay is a human step on our side and it was
skipped; the directory was correct the whole time, which is the argument for the file
channel doing the record-keeping rather than the relay. Nothing is owed — you reconstructed
it from a listing, which is what the convention is for.
