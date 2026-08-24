//! R37 end-to-end recognition probe.
//!
//! The R36 landmark probe measures a *proxy* (unique-stem density) for what an
//! enrichment style does to recognition. This measures recognition itself:
//! enrol every memory in a brain, then probe with degraded re-encounters of a
//! sample and with foreign text, and report the verdicts.
//!
//! Enrolment modes over the SAME database (plus `parts`: content and
//! description fingerprinted separately and indexed as the UNION under one id
//! via `enroll_parts`; `dual`: content under the id
//! AND the description as a second trace `{id}#d` mapped back at verdict time,
//! so the description cannot displace content peaks; and env
//! R37_PARA_DB=<other variant db> adds a paraphrase probe — that db's
//! description of the same memory as the stimulus):
//!   content   — enrol `content` only (what production does today)
//!   enriched  — enrol `content\n{description}` where a description exists
//!
//! Probes, per sample memory (the ones that HAVE a description):
//!   exact     — the raw content (sanity: should be Recognized)
//!   head      — first 50% of whitespace tokens (fragment re-encounter)
//!   dropout   — 30% of tokens removed, deterministic per memory
//! Foreign probes: utterances from a LoCoMo file that is not in the brain;
//! any Recognized verdict there is a false positive.
//!
//! Read-only on the database. In-memory index. $0.
//!
//! usage: recognition_e2e <memory.db> <content|enriched> [locomo.json] [n_foreign]

use spectral_recognition::{
    InMemoryRecognitionStore, RecognitionConfig, RecognitionEngine, Verdict,
};

fn degrade_head(s: &str) -> String {
    let toks: Vec<&str> = s.split_whitespace().collect();
    let keep = (toks.len() / 2).max(1);
    toks[..keep].join(" ")
}

fn degrade_dropout(s: &str, seed: u64) -> String {
    // xorshift, deterministic per memory
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut out = Vec::new();
    for t in s.split_whitespace() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        if (x % 100) >= 30 {
            out.push(t);
        }
    }
    out.join(" ")
}

fn base_id(id: &str) -> &str {
    id.split('#').next().unwrap_or(id)
}

fn seed_of(id: &str) -> u64 {
    id.bytes().fold(1469598103934665603u64, |h, b| {
        (h ^ b as u64).wrapping_mul(1099511628211)
    })
}

struct Tally {
    n: usize,
    recognized_correct: usize,
    recognized_wrong: usize,
    familiar: usize,
    novel: usize,
    top1_correct: usize,
    margins: Vec<f64>,
}
impl Tally {
    fn new() -> Self {
        Self {
            n: 0,
            recognized_correct: 0,
            recognized_wrong: 0,
            familiar: 0,
            novel: 0,
            top1_correct: 0,
            margins: Vec::new(),
        }
    }
    fn add(&mut self, r: &spectral_recognition::RecognitionResult, truth: &str) {
        self.n += 1;
        match &r.verdict {
            Verdict::Recognized { memory_id } => {
                if base_id(memory_id) == truth {
                    self.recognized_correct += 1
                } else {
                    self.recognized_wrong += 1
                }
            }
            Verdict::Familiar => self.familiar += 1,
            Verdict::Novel => self.novel += 1,
        }
        if let Some(t) = r.traces.first() {
            if base_id(&t.memory_id) == truth {
                self.top1_correct += 1;
                // runner-up = first trace that is NOT the same base memory
                let second = r
                    .traces
                    .iter()
                    .find(|s| base_id(&s.memory_id) != truth)
                    .map(|s| s.score)
                    .unwrap_or(0.0);
                if t.score > 0.0 {
                    self.margins.push((t.score - second) / t.score);
                }
            }
        }
    }
    fn print(&self, name: &str) {
        let pct = |k: usize| 100.0 * k as f64 / self.n.max(1) as f64;
        let mut m = self.margins.clone();
        m.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let med = if m.is_empty() { 0.0 } else { m[m.len() / 2] };
        let mean = if m.is_empty() {
            0.0
        } else {
            m.iter().sum::<f64>() / m.len() as f64
        };
        println!(
            "  {name:<8} n={:<4} Recognized(correct) {:5.1}%  Recognized(WRONG) {:4.1}%  Familiar {:5.1}%  Novel {:5.1}%  top1 {:5.1}%  margin mean/med {:.3}/{:.3}",
            self.n, pct(self.recognized_correct), pct(self.recognized_wrong), pct(self.familiar), pct(self.novel), pct(self.top1_correct), mean, med
        );
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let db = args
        .get(1)
        .ok_or("usage: recognition_e2e <memory.db> <content|enriched> [locomo.json] [n_foreign]")?;
    let mode = args.get(2).map(|s| s.as_str()).unwrap_or("content");
    let locomo = args.get(3);
    let n_foreign: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(300);

    let conn = rusqlite::Connection::open_with_flags(
        db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )?;
    let mut stmt = conn.prepare(
        "SELECT id, content, description FROM memories
         WHERE content IS NOT NULL AND TRIM(content) <> '' ORDER BY id",
    )?;
    let rows: Vec<(String, String, Option<String>)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<Result<_, _>>()?;

    let cfg = RecognitionConfig::default();
    let mut engine = RecognitionEngine::new(InMemoryRecognitionStore::default(), cfg);

    let mut sample: Vec<(String, String)> = Vec::new();
    for (id, content, desc) in &rows {
        let has_desc = desc
            .as_deref()
            .map(|d| !d.trim().is_empty())
            .unwrap_or(false);
        let text = match (mode, has_desc) {
            ("enriched", true) => format!("{content}\n{}", desc.as_deref().unwrap()),
            _ => content.clone(),
        };
        if mode == "parts" && has_desc {
            engine.enroll_parts(id, &[content.as_str(), desc.as_deref().unwrap()])?;
        } else if mode == "parts_clean" && has_desc {
            // Strip the Librarian's wire-format wrapper: identical boilerplate
            // on every memory is a feature every memory shares.
            let cleaned = desc
                .as_deref()
                .unwrap()
                .replace("Related terms:", " ")
                .replace("Categories:", " ")
                .replace("FACTS:", " ")
                .replace("TERMS:", " ")
                .replace("CATEGORIES:", " ");
            engine.enroll_parts(id, &[content.as_str(), cleaned.as_str()])?;
        } else {
            engine.enroll(id, &text)?;
        }
        if mode == "dual" && has_desc {
            engine.enroll(&format!("{id}#d"), desc.as_deref().unwrap())?;
        }
        if has_desc {
            sample.push((id.clone(), content.clone()));
        }
    }
    println!("R37 e2e recognition probe — mode={mode}");
    println!(
        "  enrolled {} memories; probing {} sample memories (those with a description)",
        rows.len(),
        sample.len()
    );

    let mut exact = Tally::new();
    let mut head = Tally::new();
    let mut drop = Tally::new();
    for (id, content) in &sample {
        exact.add(&engine.recognize(content)?, id);
        head.add(&engine.recognize(&degrade_head(content))?, id);
        drop.add(
            &engine.recognize(&degrade_dropout(content, seed_of(id)))?,
            id,
        );
    }
    exact.print("exact");
    head.print("head50");
    drop.print("drop30");

    if let Ok(para_db) = std::env::var("R37_PARA_DB") {
        let pconn = rusqlite::Connection::open_with_flags(
            &para_db,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
        )?;
        let mut pstmt = pconn.prepare(
            "SELECT id, description FROM memories WHERE description IS NOT NULL AND TRIM(description) <> ''",
        )?;
        let paras: std::collections::HashMap<String, String> = pstmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        let mut para = Tally::new();
        for (id, _content) in &sample {
            if let Some(p) = paras.get(id) {
                para.add(&engine.recognize(p)?, id);
            }
        }
        para.print("para");
    }

    if let Some(path) = locomo {
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path)?)?;
        let mut probes = Vec::new();
        'outer: for sampl in v.as_array().into_iter().flatten() {
            let conv = &sampl["conversation"];
            let mut keys: Vec<&String> = conv
                .as_object()
                .into_iter()
                .flatten()
                .map(|(k, _)| k)
                .collect();
            keys.sort();
            for k in keys {
                if let Some(turns) = conv[k].as_array() {
                    for t in turns {
                        if let Some(text) = t["text"].as_str() {
                            if text.split_whitespace().count() >= 12 {
                                probes.push(text.to_string());
                                if probes.len() >= n_foreign {
                                    break 'outer;
                                }
                            }
                        }
                    }
                }
            }
        }
        let (mut rec, mut fam, mut nov) = (0, 0, 0);
        for p in &probes {
            match engine.recognize(p)?.verdict {
                Verdict::Recognized { .. } => rec += 1,
                Verdict::Familiar => fam += 1,
                Verdict::Novel => nov += 1,
            }
        }
        let n = probes.len().max(1) as f64;
        println!(
            "  foreign  n={:<4} Recognized(FALSE) {:5.1}%  Familiar {:5.1}%  Novel {:5.1}%",
            probes.len(),
            100.0 * rec as f64 / n,
            100.0 * fam as f64 / n,
            100.0 * nov as f64 / n
        );
    }
    Ok(())
}
