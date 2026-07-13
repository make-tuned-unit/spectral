//! Read-time context consolidation pre-pass (A/B for the layered-memory idea).
//!
//! Gated by `SPECTRAL_CONSOLIDATE_CONTEXT=1`. Before the (expensive) actor runs,
//! one **sparse, cheap** model call (haiku) deduplicates cross-session mentions
//! of the same real-world item into a compact entity-keyed atom list, which is
//! prepended to the context. This mirrors, at read time, Spectral's
//! `consolidate_*` + `recall_with_provenance` structure: give the actor a
//! deduplicated candidate set plus the raw sessions for grounding. Tests whether
//! that reduces the cross-session-dedup counting errors the in-prompt two-pass
//! only partly fixes. Cheap by construction: haiku, one call per question.

const CONSOLIDATE_MODEL: &str = "claude-haiku-4-5-20251001";

fn prompt(question: &str, memories: &str) -> String {
    format!(
        "You are preprocessing retrieved conversation memories for a COUNTING question, \
so a downstream agent can count accurately across many sessions.\n\n\
Question: {question}\n\n\
Below are memories from MULTIPLE sessions (headers like \"--- Session <id> ---\"). \
Produce a DEDUPLICATED candidate list of the distinct real-world items relevant to \
the question. Rules:\n\
- Merge mentions of the SAME item across different sessions/dates into ONE entry \
(same person/couple, same event, same project, same object = one item). Key each \
item by its most distinctive identifier (a name, participants, a title).\n\
- Include only items the user actually did/attended/bought/owns — exclude \
hypotheticals, planned-but-not-done, and assistant suggestions.\n\
- One line per distinct item: `<identifier> — <sessions it appears in>`.\n\
- Do NOT state a final count; just the deduplicated list.\n\n\
Memories:\n{memories}\n\nDeduplicated candidate list:"
    )
}

/// Make one sparse haiku call to consolidate the retrieved memories into a
/// deduplicated atom list. Returns `None` (caller falls back to the flat
/// context) on any error — the pre-pass must never break the run.
pub fn consolidate_context(question: &str, memories: &[String]) -> Option<String> {
    let api_key = std::env::var("ANTHROPIC_API_KEY").ok()?;
    let body = serde_json::json!({
        "model": CONSOLIDATE_MODEL,
        "max_tokens": 1024,
        "messages": [{"role": "user", "content": prompt(question, &memories.join("\n"))}]
    });
    let client = reqwest::blocking::Client::new();
    let resp = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let json: serde_json::Value = resp.json().ok()?;
    let atoms = crate::actor::extract_text(&json)?;
    let trimmed = atoms.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}
