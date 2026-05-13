# Ollama Compatibility for Describe Subcommand

**Date**: 2026-05-13
**Branch**: `investigate/ollama-describe-compat`
**Status**: Implemented and verified.

---

## Verdict

Ollama integration works. The describe subcommand now supports `--api-format openai` for Ollama and other OpenAI-compatible endpoints (vLLM, text-generation-inference, etc.).

## Evidence

### Environment
- Ollama 0.23.2 installed
- qwen2.5:7b model available (4.7 GB, pulled today)
- Ollama running at `http://localhost:11434`

### API compatibility test
```bash
curl -s http://localhost:11434/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model":"qwen2.5:7b","messages":[{"role":"user","content":"Hello"}],"max_tokens":50}'
```
Response: valid OpenAI-shaped JSON with `choices[0].message.content`.

### Three incompatibilities found and resolved

| Aspect | Anthropic API | OpenAI/Ollama API |
|--------|--------------|-------------------|
| Endpoint | `/v1/messages` | `/v1/chat/completions` |
| Auth | `x-api-key` + `anthropic-version` headers | `Authorization: Bearer` (optional for local) |
| Response | `content[0].text` | `choices[0].message.content` |

### Smoke test
```bash
./target/release/spectral-bench-accuracy describe \
  --dataset ~/spectral-local-bench/longmemeval/longmemeval_s.json \
  --output /tmp/ollama-describe-test.json \
  --max-questions 1 \
  --api-format openai \
  --model qwen2.5:7b \
  --base-url http://localhost:11434
```
Result: 100+ descriptions generated and written incrementally. Content quality looks reasonable — descriptions contain category-level vocabulary and specific details.

## Changes made

- Added `OpenAIDescriber` struct to `describe.rs` (~40 lines). Implements `DescriptionGenerator` trait with OpenAI-shaped request/response handling.
- Added `--api-format <anthropic|openai>` flag to `describe` subcommand. Default: `anthropic`. Use `openai` for Ollama/vLLM.
- No API key required for OpenAI format (Ollama doesn't need one).

## Usage

```bash
# Full bench corpus description generation via Ollama
./target/release/spectral-bench-accuracy describe \
  --dataset ~/spectral-local-bench/longmemeval/longmemeval_s.json \
  --output bench_descriptions.json \
  --api-format openai \
  --model qwen2.5:7b \
  --base-url http://localhost:11434

# Resume after interruption (skip-existing default)
./target/release/spectral-bench-accuracy describe \
  --dataset ~/spectral-local-bench/longmemeval/longmemeval_s.json \
  --output bench_descriptions.json \
  --api-format openai \
  --model qwen2.5:7b \
  --base-url http://localhost:11434
```

Estimated time: ~2-4 hours for full LongMemEval corpus (~12,000 memories) on qwen2.5:7b locally. No API cost.
