---
name: verify
description: Build/launch/drive recipe for verifying spectral library changes end-to-end on this machine
---

# Verifying spectral changes

Spectral is a library; its surface is the public `spectral` umbrella crate.
Verify by driving sample code through `use spectral::...`, never internal
`spectral-*` imports.

## Machine constraints

This machine (since 2026-08-13): **Apple M4 Mac mini, 16 GB** — note the
default shell may report `x86_64` under Rosetta; the hardware is arm64.
The old Intel machine's `ort-sys` prohibition does NOT apply here:
`cargo test --workspace` was run green on 2026-08-13 (verified, all crates).

- `cargo` needs `export PATH="$HOME/.cargo/bin:$PATH"`.
- Still set `CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0` for dev builds —
  kuzu debug artifacts are huge (a debug libkuzu rlib is ~2 GB; the cmake
  build dir is ~4 GB). If ENOSPC hits, check for duplicate
  `target/debug/build/kuzu-*` dirs from profile changes and delete stale
  ones (by mtime) — but never while a build is running.
- Ollama serves local models on :11434 (`brew services start ollama`);
  `qwen25-16k` = qwen2.5:7b + `num_ctx 16384, temperature 0` via Modelfile.

## Test what you changed

```bash
cargo test -p spectral-ingest -p spectral-graph -p spectral-bench-accuracy
cargo check -p spectral --lib        # umbrella lib compiles without dev-deps
cargo clippy -p spectral-ingest -p spectral-graph --tests
```

## Drive the public surface (the actual verification)

Use the pre-built consumer harness pattern: a scratch crate outside the
workspace depending on spectral by path. Critical tricks:

1. `cp <repo>/Cargo.lock <consumer>/` — without this the consumer resolves
   different dep versions → different kuzu fingerprint → 30+ min C++
   rebuild (and possible link errors against half-shared artifacts).
2. `export CARGO_TARGET_DIR=<repo>/target` — reuses the already-built kuzu.

```toml
[package]
name = "spectral-consumer"
version = "0.1.0"
edition = "2021"
[dependencies]
spectral = { path = "<repo>/crates/spectral" }
[workspace]
```

Gotchas when writing scenarios:
- `Brain::open(path)` auto-creates an ontology; `Brain::builder()` requires
  `.ontology_path(...)` — write a `version = 1` ontology.toml first.
- Wing/hall classification is regex-driven (`spectral-ingest/src/classifier.rs`);
  to hit TACT Tier-1 the query must match BOTH a wing regex (e.g. "apollo",
  "strategy") and a hall regex (e.g. "decided"). Check the regexes before
  crafting scenario text.
- `HybridRecallResult.tact.method` tells you which TACT tier actually fired
  (`Fingerprint` / `WingOnly` / `Fts`) — assert on it.
