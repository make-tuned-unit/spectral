# Contributing to Spectral

Spectral is early (v0.0.1). We welcome contributions and will respond to issues
and PRs promptly.

## Before opening a PR

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace --lib --tests
```

All three must pass. CI enforces the same checks.

## What we are looking for

- Bug reports with reproduction steps
- Performance improvements with benchmark evidence
- New tests, especially edge cases
- Documentation fixes

## Issues

Open an issue at https://github.com/make-tuned-unit/spectral/issues. Include
the Spectral version (`Cargo.toml` version), Rust version, and OS.
