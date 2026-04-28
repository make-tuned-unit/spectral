# Changelog

All notable changes to Spectral will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Breaking changes
- **`Brain::recall()`** now takes `context_visibility: Visibility` to filter
  results. Use `Visibility::Private` for the previous see-everything behavior.
- **`Brain::recall_graph()`** — same change.
- **`Brain::remember()`** now takes `visibility: Visibility` to control who can
  see the stored memory. Default to `Visibility::Private` for fail-safe.
- **`Memory`** and **`MemoryHit`** structs gain a `visibility: String` field.

### Added
- Visibility enforcement in recall paths — entities, triples, and memory hits
  are filtered by the caller's visibility context.
- **`Brain::recall_local()`** — convenience for `recall(query, Visibility::Private)`.
- SQLite memory schema: `visibility TEXT NOT NULL DEFAULT 'private'` column.
- Initial workspace scaffolding for spectral-core, spectral-graph,
  spectral-tact, spectral-spectrogram, and the umbrella `spectral` crate.
