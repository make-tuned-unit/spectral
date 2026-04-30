# spectral-bench-real

Benchmark harness for measuring Spectral recall latency and accuracy against real brains.

## Purpose

This crate runs a curated set of queries against an existing Spectral brain and reports per-query latency percentiles, accuracy spot-checks, and aggregate statistics. It exists to make performance claims measurable and regressions detectable.

This is measurement infrastructure, not a tuning tool. It doesn't modify the brain or the library.

## Usage

```bash
# Basic run against a brain
cargo run --release -p spectral-bench-real -- --brain /path/to/brain

# JSON output for CI
cargo run --release -p spectral-bench-real -- --brain /path/to/brain --format json

# Fewer iterations for a quick smoke test
cargo run --release -p spectral-bench-real -- --brain /path/to/brain --iterations 10

# Filter to specific query patterns
cargo run --release -p spectral-bench-real -- --brain /path/to/brain --filter adversarial
```

### Options

| Flag | Default | Description |
|---|---|---|
| `--brain <path>` | required | Path to an existing Spectral brain directory |
| `--queries <path>` | `crates/spectral-bench-real/queries.toml` | Path to queries TOML file |
| `--iterations <n>` | 100 | Iterations per query for warm-cache measurement |
| `--format <text\|json>` | text | Output format |
| `--filter <substring>` | none | Only run queries whose name contains this substring |

## Measurement approach

The benchmark runs in two phases:

1. **Cold-cache pass** — opens the brain fresh and runs each query once. This measures worst-case latency including SQLite page cache misses and Kuzu cold start.

2. **Warm-cache pass** — opens the brain once, runs a discard warmup pass, then loops each query for `--iterations` repetitions. Reports p50/p95/p99 from these runs.

Latency is measured with `std::time::Instant` (low overhead, no external framework).

## Query file format

Queries live in `queries.toml`:

```toml
[[queries]]
name = "single_word_agent"
text = "agent"
description = "Single-word lookup for agent-related memories"
expected_keywords = ["agent"]
expected_top_n = 5
latency_budget_p95_ms = 5.0
latency_budget_p99_ms = 10.0
visibility = "private"
```

| Field | Description |
|---|---|
| `name` | Unique identifier, prefixed by pattern (single_word, multi_word, etc.) |
| `text` | The query string passed to `brain.recall()` |
| `description` | Human-readable description |
| `expected_keywords` | At least one must appear (case-insensitive) in top-N results |
| `expected_top_n` | How many results to check for keyword matches |
| `latency_budget_p95_ms` | P95 budget in milliseconds |
| `latency_budget_p99_ms` | P99 budget in milliseconds |
| `visibility` | Visibility level for the query (private, team, org, public) |

### Accuracy check

For non-adversarial queries: at least one `expected_keywords` entry must appear as a case-insensitive substring in the content of any of the top-N results. This is a loose smoke test, not a precision benchmark.

For adversarial queries (empty `expected_keywords`): always passes. These exist to measure latency on miss paths and detect false positives visually.

## Output interpretation

### Text output

```
Query                               Cold    P50    P95    P99 Score Hits  OK
--------------------------------------------------------------------------------
single_word_agent                    4523    234    567    891  0.69    5   Y
adversarial_gibberish                 312     98    145    201  0.00    0   Y
```

OK column: `Y`=pass, `B`=budget miss, `A`=accuracy miss, `N`=both miss.

### JSON output

Machine-readable format with `queries[]`, `aggregate`, and `per_pattern[]` sections. Suitable for CI diffing.

## Adding new queries

1. Add a `[[queries]]` entry to `queries.toml`
2. Prefix the name with the pattern category (single_word, multi_word, concept, temporal, cross_domain, adversarial)
3. Choose `expected_keywords` that are likely to appear in memories matching the query — keep it loose
4. Run the benchmark and verify the query behaves as expected

## CI integration (planned)

Not yet wired up. The intended design:

1. A GitHub Action runs the benchmark against a small reference brain (committed or fetched from a release artifact)
2. JSON output is compared against baseline from the target branch
3. PR comments show latency deltas: "p95 changed from 567us to 612us (+8%)"
4. Regressions above a threshold (e.g., 10%) fail the workflow
5. Baseline JSON is updated on merge to main

## Caveats

- Results are specific to the brain being measured. A 931-memory brain behaves differently from a 10-memory brain.
- Cold-cache latency depends on OS page cache state and disk speed.
- The accuracy check is deliberately loose — it catches gross regressions (recall returning nothing), not subtle ranking changes.
- This measures `brain.recall()` end-to-end. It doesn't isolate individual subsystems (fingerprint search, FTS, graph traversal).
