//! Generate search-indexing descriptions for bench memories via LLM API.
//!
//! Produces a JSON file mapping `memory_key -> description` that can be
//! loaded during bench runs to enrich FTS indexing.

use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

/// Description map: memory key → generated description.
pub type DescriptionMap = HashMap<String, String>;

/// Load an existing description map from a JSON file.
/// Returns an empty map if the file doesn't exist.
pub fn load_descriptions(path: &Path) -> Result<DescriptionMap> {
    if !path.exists() {
        return Ok(DescriptionMap::new());
    }
    let contents = std::fs::read_to_string(path)?;
    let map: DescriptionMap = serde_json::from_str(&contents)?;
    Ok(map)
}

/// Save a description map to a JSON file.
pub fn save_descriptions(map: &DescriptionMap, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(map)?;
    std::fs::write(path, json)?;
    Ok(())
}

/// Prompt template for generating search-indexing descriptions.
pub const DESCRIBE_PROMPT: &str = r#"Write a concise description (50-100 tokens) of this memory for search indexing.

Requirements:
- Include category-level nouns that generalize the specific items mentioned
  (e.g., "coffee table" → also say "furniture"; "Dr. Patel" → also say "doctors")
- Include BOTH singular and plural forms of key nouns
  (e.g., "doctor/doctors", "wedding/weddings", "project/projects")
- Include the specific names and details from the content
- Do NOT add category terms the content doesn't support
- Write in third person ("User..." not "I...")

Memory content:
{content}

Description:"#;

/// Build the prompt for a single memory.
pub fn build_prompt(content: &str) -> String {
    DESCRIBE_PROMPT.replace("{content}", content)
}

/// Trait for description generation, enabling mock/real implementations.
pub trait DescriptionGenerator: Send + Sync {
    fn generate(&self, content: &str) -> Result<String>;
}

/// Generator that calls the Anthropic Messages API.
pub struct AnthropicDescriber {
    api_key: String,
    model: String,
    base_url: String,
    client: reqwest::blocking::Client,
}

impl AnthropicDescriber {
    pub fn new(api_key: String, model: String, base_url: String) -> Self {
        Self {
            api_key,
            model,
            base_url,
            client: reqwest::blocking::Client::new(),
        }
    }

    pub fn from_env() -> Result<Self> {
        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .map_err(|_| anyhow::anyhow!("ANTHROPIC_API_KEY not set"))?;
        Ok(Self::new(
            api_key,
            "claude-haiku-4-5-20251001".into(),
            "https://api.anthropic.com".into(),
        ))
    }
}

impl DescriptionGenerator for AnthropicDescriber {
    fn generate(&self, content: &str) -> Result<String> {
        let prompt = build_prompt(content);
        let body = serde_json::json!({
            "model": self.model,
            "max_tokens": 256,
            "messages": [{"role": "user", "content": prompt}]
        });

        let resp = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().unwrap_or_default();
            anyhow::bail!(
                "Describe API returned {status}: {}",
                &body[..body.len().min(500)]
            );
        }

        let json: serde_json::Value = resp.json()?;
        let text = json["content"][0]["text"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing content[0].text in describe response"))?
            .trim()
            .to_string();
        Ok(text)
    }
}

/// Write the description map atomically: write to `.tmp`, then rename.
fn save_descriptions_atomic(map: &DescriptionMap, path: &Path) -> Result<()> {
    let tmp_path = path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(map)?;
    std::fs::write(&tmp_path, json)?;
    std::fs::rename(&tmp_path, path)?;
    Ok(())
}

/// Generate descriptions for all memory keys, respecting idempotence.
///
/// - `memory_keys_and_content`: list of (key, content) pairs to describe
/// - `existing`: previously generated descriptions (skip these unless regenerate=true)
/// - `regenerate`: if true, regenerate all descriptions regardless of existing
/// - `generator`: the LLM client to call
///
/// Returns the merged description map (existing + newly generated).
pub fn generate_descriptions(
    memory_keys_and_content: &[(String, String)],
    existing: &DescriptionMap,
    regenerate: bool,
    generator: &dyn DescriptionGenerator,
) -> Result<DescriptionMap> {
    generate_descriptions_incremental(
        memory_keys_and_content,
        existing,
        regenerate,
        generator,
        None,
        100,
    )
}

/// Generate descriptions with incremental persistence.
///
/// Every `flush_interval` newly generated descriptions, the accumulated map
/// is written atomically to `output_path` (write to `.tmp`, rename to final).
/// On Ctrl-C the current state is flushed before exit.
///
/// Combined with skip-existing default behavior, a killed run resumes by
/// re-invoking the same command.
pub fn generate_descriptions_incremental(
    memory_keys_and_content: &[(String, String)],
    existing: &DescriptionMap,
    regenerate: bool,
    generator: &dyn DescriptionGenerator,
    output_path: Option<&Path>,
    flush_interval: usize,
) -> Result<DescriptionMap> {
    let mut map = if regenerate {
        DescriptionMap::new()
    } else {
        existing.clone()
    };

    // Ctrl-C flag: set by signal handler, checked each iteration
    let interrupted = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let interrupted_clone = interrupted.clone();
    let _ = ctrlc::set_handler(move || {
        interrupted_clone.store(true, std::sync::atomic::Ordering::SeqCst);
        eprintln!("\n  Ctrl-C received, flushing descriptions to disk...");
    });

    let total = memory_keys_and_content.len();
    let mut generated = 0;
    let mut skipped = 0;
    let mut since_last_flush = 0;

    for (key, content) in memory_keys_and_content {
        // Check for Ctrl-C before each generation
        if interrupted.load(std::sync::atomic::Ordering::SeqCst) {
            eprintln!("  Interrupted after {generated} generations, {skipped} skipped");
            break;
        }

        if !regenerate && map.contains_key(key) {
            skipped += 1;
            continue;
        }

        match generator.generate(content) {
            Ok(desc) => {
                map.insert(key.clone(), desc);
                generated += 1;
                since_last_flush += 1;
                if generated % 100 == 0 {
                    eprintln!(
                        "  Generated {generated}/{total} descriptions ({skipped} skipped)..."
                    );
                }
                // Incremental flush
                if since_last_flush >= flush_interval {
                    if let Some(path) = output_path {
                        save_descriptions_atomic(&map, path)?;
                    }
                    since_last_flush = 0;
                }
            }
            Err(e) => {
                eprintln!("  warn: failed to describe {key}: {e}");
            }
        }
    }

    // Final flush (covers remaining items + Ctrl-C)
    if let Some(path) = output_path {
        save_descriptions_atomic(&map, path)?;
    }

    eprintln!(
        "Description generation complete: {generated} generated, {skipped} skipped, {total} total"
    );
    Ok(map)
}

/// Compute the memory ID from a key (same deterministic hash as Brain::remember_with).
pub fn memory_id_from_key(key: &str) -> String {
    format!(
        "{:016x}",
        u64::from_be_bytes(
            blake3::hash(key.as_bytes()).as_bytes()[..8]
                .try_into()
                .unwrap()
        )
    )
}

/// Apply descriptions from a map to a brain via set_description().
///
/// Keys in the map are memory keys (e.g., "session_id:turn:0:user").
/// The memory ID is computed deterministically from the key.
pub fn apply_descriptions(
    brain: &spectral_graph::brain::Brain,
    descriptions: &DescriptionMap,
) -> Result<usize> {
    let mut applied = 0;
    for (key, desc) in descriptions {
        let id = memory_id_from_key(key);
        if brain.set_description(&id, desc).is_ok() {
            applied += 1;
        }
    }
    Ok(applied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Mock generator that returns deterministic descriptions.
    struct MockDescriber {
        call_count: AtomicUsize,
    }

    impl MockDescriber {
        fn new() -> Self {
            Self {
                call_count: AtomicUsize::new(0),
            }
        }

        fn calls(&self) -> usize {
            self.call_count.load(Ordering::SeqCst)
        }
    }

    impl DescriptionGenerator for MockDescriber {
        fn generate(&self, content: &str) -> Result<String> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            Ok(format!(
                "Description of: {}",
                &content[..content.len().min(50)]
            ))
        }
    }

    #[test]
    fn load_nonexistent_returns_empty() {
        let map = load_descriptions(Path::new("/nonexistent/path.json")).unwrap();
        assert!(map.is_empty());
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("descriptions.json");

        let mut map = DescriptionMap::new();
        map.insert("key1".into(), "desc1".into());
        map.insert("key2".into(), "desc2".into());

        save_descriptions(&map, &path).unwrap();
        let loaded = load_descriptions(&path).unwrap();

        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded.get("key1").unwrap(), "desc1");
        assert_eq!(loaded.get("key2").unwrap(), "desc2");
    }

    /// Mock generator that tracks calls and optionally records flush events.
    struct FlushTrackingDescriber {
        call_count: AtomicUsize,
    }

    impl FlushTrackingDescriber {
        fn new() -> Self {
            Self {
                call_count: AtomicUsize::new(0),
            }
        }

        fn calls(&self) -> usize {
            self.call_count.load(Ordering::SeqCst)
        }
    }

    impl DescriptionGenerator for FlushTrackingDescriber {
        fn generate(&self, content: &str) -> Result<String> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            Ok(format!("desc: {}", &content[..content.len().min(20)]))
        }
    }

    #[test]
    fn incremental_flush_writes_periodically() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("incr.json");
        let mock = FlushTrackingDescriber::new();

        let items: Vec<(String, String)> = (0..5)
            .map(|i| (format!("key{i}"), format!("content{i}")))
            .collect();

        let result = generate_descriptions_incremental(
            &items,
            &DescriptionMap::new(),
            false,
            &mock,
            Some(&path),
            2, // flush every 2
        )
        .unwrap();

        assert_eq!(mock.calls(), 5);
        assert_eq!(result.len(), 5);

        // File should exist with all 5 descriptions (final flush)
        let loaded = load_descriptions(&path).unwrap();
        assert_eq!(loaded.len(), 5);
    }

    #[test]
    fn incremental_resume_skips_existing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("resume.json");
        let mock = FlushTrackingDescriber::new();

        // Pre-populate with 3 existing descriptions
        let mut existing = DescriptionMap::new();
        for i in 0..3 {
            existing.insert(format!("key{i}"), format!("existing{i}"));
        }
        save_descriptions(&existing, &path).unwrap();

        let items: Vec<(String, String)> = (0..5)
            .map(|i| (format!("key{i}"), format!("content{i}")))
            .collect();

        let loaded_existing = load_descriptions(&path).unwrap();
        let result = generate_descriptions_incremental(
            &items,
            &loaded_existing,
            false,
            &mock,
            Some(&path),
            100,
        )
        .unwrap();

        // Should only generate 2 new descriptions (key3, key4)
        assert_eq!(mock.calls(), 2, "should only generate for key3 and key4");
        assert_eq!(result.len(), 5);
        // Existing entries preserved
        assert_eq!(result.get("key0").unwrap(), "existing0");
        // New entries generated
        assert!(result.get("key3").unwrap().starts_with("desc:"));
    }

    #[test]
    fn incremental_no_output_path_still_works() {
        let mock = FlushTrackingDescriber::new();
        let items: Vec<(String, String)> = (0..3)
            .map(|i| (format!("key{i}"), format!("content{i}")))
            .collect();

        let result = generate_descriptions_incremental(
            &items,
            &DescriptionMap::new(),
            false,
            &mock,
            None, // no output path
            1,
        )
        .unwrap();

        assert_eq!(mock.calls(), 3);
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn atomic_write_no_partial_file_on_success() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("atomic.json");
        let tmp_path = path.with_extension("json.tmp");

        let mut map = DescriptionMap::new();
        map.insert("k1".into(), "v1".into());
        save_descriptions_atomic(&map, &path).unwrap();

        // Final file exists, tmp does not
        assert!(path.exists());
        assert!(!tmp_path.exists());
        let loaded = load_descriptions(&path).unwrap();
        assert_eq!(loaded.len(), 1);
    }

    #[test]
    fn generate_skips_existing_by_default() {
        let mock = MockDescriber::new();
        let mut existing = DescriptionMap::new();
        existing.insert("key1".into(), "already described".into());

        let items = vec![
            ("key1".into(), "content1".into()),
            ("key2".into(), "content2".into()),
        ];

        let result = generate_descriptions(&items, &existing, false, &mock).unwrap();

        assert_eq!(mock.calls(), 1, "should only call generator for key2");
        assert_eq!(result.get("key1").unwrap(), "already described");
        assert!(result.get("key2").unwrap().starts_with("Description of:"));
    }

    #[test]
    fn generate_regenerates_all_with_flag() {
        let mock = MockDescriber::new();
        let mut existing = DescriptionMap::new();
        existing.insert("key1".into(), "old description".into());

        let items = vec![
            ("key1".into(), "content1".into()),
            ("key2".into(), "content2".into()),
        ];

        let result = generate_descriptions(&items, &existing, true, &mock).unwrap();

        assert_eq!(mock.calls(), 2, "should call generator for both keys");
        assert!(
            result.get("key1").unwrap().starts_with("Description of:"),
            "key1 should be regenerated"
        );
        assert!(result.get("key2").unwrap().starts_with("Description of:"));
    }

    #[test]
    fn generate_empty_input_returns_existing() {
        let mock = MockDescriber::new();
        let mut existing = DescriptionMap::new();
        existing.insert("key1".into(), "existing".into());

        let items: Vec<(String, String)> = vec![];
        let result = generate_descriptions(&items, &existing, false, &mock).unwrap();

        assert_eq!(mock.calls(), 0);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn build_prompt_includes_content() {
        let prompt = build_prompt("I saw Dr. Patel for sinusitis");
        assert!(prompt.contains("I saw Dr. Patel for sinusitis"));
        assert!(prompt.contains("category-level nouns"));
        assert!(prompt.contains("singular and plural"));
    }

    #[test]
    fn memory_id_from_key_is_deterministic() {
        let id1 = memory_id_from_key("session_abc:turn:0:user");
        let id2 = memory_id_from_key("session_abc:turn:0:user");
        assert_eq!(id1, id2);
        assert_eq!(id1.len(), 16, "should be 16 hex chars");

        // Different keys produce different IDs
        let id3 = memory_id_from_key("session_abc:turn:1:assistant");
        assert_ne!(id1, id3);
    }

    #[test]
    fn apply_descriptions_sets_on_brain() {
        use spectral_core::visibility::Visibility;
        use spectral_graph::brain::{Brain, BrainConfig, EntityPolicy, RememberOpts};

        let dir = tempfile::tempdir().unwrap();
        let ontology_path = dir.path().join("ontology.toml");
        std::fs::write(&ontology_path, "version = 1\n").unwrap();

        let brain = Brain::open(BrainConfig {
            data_dir: dir.path().to_path_buf(),
            ontology_path,
            memory_db_path: None,
            llm_client: None,
            wing_rules: None,
            hall_rules: None,
            device_id: None,
            enable_spectrogram: false,
            entity_policy: EntityPolicy::Strict,
            sqlite_mmap_size: None,
            activity_wing: "activity".into(),
            redaction_policy: None,
            tact_config: None,
        })
        .unwrap();

        let key = "test_session:turn:0:user";
        brain
            .remember_with(
                key,
                "I saw Dr. Patel for sinusitis",
                RememberOpts {
                    visibility: Visibility::Private,
                    ..Default::default()
                },
            )
            .unwrap();

        let mut descs = DescriptionMap::new();
        descs.insert(key.into(), "User visits doctors including Dr. Patel".into());

        let applied = apply_descriptions(&brain, &descs).unwrap();
        assert_eq!(applied, 1);

        // Verify description was set
        let id = memory_id_from_key(key);
        let mem = brain.get_memory(&id).unwrap().unwrap();
        assert_eq!(
            mem.description.as_deref(),
            Some("User visits doctors including Dr. Patel")
        );
    }
}
