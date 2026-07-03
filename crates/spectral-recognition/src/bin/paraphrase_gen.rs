//! Generate a frozen paraphrase set from real brain memories (Haiku, ~$1
//! for 200). Paraphrases are the HARD family for deterministic recognition:
//! same facts, different words. The output JSON {memory_id: paraphrase} is
//! a pay-once asset consumed by `paraphrase_replay` at $0.
//!
//! Usage: paraphrase_gen --db <memory.db> [--n 200] [--output paraphrases.json]

use anyhow::{Context, Result};

const PROMPT: &str = "Rewrite this note in completely different words and sentence structure. \
Preserve every specific fact — names, numbers, dates, identifiers — but change all other \
vocabulary and phrasing as much as possible. Output ONLY the rewrite, no preamble.\n\nNote:\n";

fn paraphrase(
    client: &reqwest::blocking::Client,
    api_key: &str,
    content: &str,
) -> Result<(String, u64, u64)> {
    let body = serde_json::json!({
        "model": "claude-haiku-4-5-20251001",
        "max_tokens": 1024,
        "messages": [{"role": "user", "content": format!("{PROMPT}{content}")}],
    });
    let resp: serde_json::Value = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()?
        .error_for_status()?
        .json()?;
    let text = resp["content"][0]["text"]
        .as_str()
        .context("no text in response")?
        .trim()
        .to_string();
    let in_tok = resp["usage"]["input_tokens"].as_u64().unwrap_or(0);
    let out_tok = resp["usage"]["output_tokens"].as_u64().unwrap_or(0);
    Ok((text, in_tok, out_tok))
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let db = args
        .iter()
        .position(|a| a == "--db")
        .and_then(|i| args.get(i + 1))
        .context("--db <path> required")?;
    let n: usize = args
        .iter()
        .position(|a| a == "--n")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(200);
    let output = args
        .iter()
        .position(|a| a == "--output")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| "paraphrases.json".to_string());
    let api_key = std::env::var("ANTHROPIC_API_KEY").context("ANTHROPIC_API_KEY not set")?;

    let conn =
        rusqlite::Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    // Deterministic sample: longest-content memories first give the
    // paraphraser room to actually rewrite; order by id for determinism.
    let mut stmt = conn.prepare(
        "SELECT id, content FROM memories WHERE LENGTH(content) BETWEEN 150 AND 2500
         ORDER BY id LIMIT ?1",
    )?;
    let memories: Vec<(String, String)> = stmt
        .query_map([n as i64], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<std::result::Result<_, _>>()?;

    let mut cache: std::collections::BTreeMap<String, String> = std::fs::read_to_string(&output)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    eprintln!(
        "{} memories, {} already paraphrased",
        memories.len(),
        cache.len()
    );

    let client = reqwest::blocking::Client::new();
    let (mut in_tok, mut out_tok, mut failures) = (0u64, 0u64, 0usize);
    for (i, (id, content)) in memories.iter().enumerate() {
        if cache.contains_key(id) {
            continue;
        }
        match paraphrase(&client, &api_key, content) {
            Ok((text, it, ot)) => {
                in_tok += it;
                out_tok += ot;
                cache.insert(id.clone(), text);
            }
            Err(e) => {
                eprintln!("  [{i}] {id} failed: {e}");
                failures += 1;
                if failures > 15 {
                    anyhow::bail!("too many failures; aborting (cache saved)");
                }
            }
        }
        if i % 20 == 0 {
            std::fs::write(&output, serde_json::to_string_pretty(&cache)?)?;
            eprint!("\r{}/{}", cache.len(), memories.len());
        }
    }
    std::fs::write(&output, serde_json::to_string_pretty(&cache)?)?;
    let cost = in_tok as f64 * 0.80 / 1e6 + out_tok as f64 * 4.00 / 1e6;
    eprintln!(
        "\ndone: {} paraphrases, {failures} failures, cost ${cost:.3}",
        cache.len()
    );
    Ok(())
}
