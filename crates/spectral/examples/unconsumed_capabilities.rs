//! What is Spectral already able to do on this brain that nothing is asking it
//! for?
//!
//! Permagent consumes `remember`, the `recall*` family, `get_memory` and
//! `set_description`. This exercises, read-only, the capabilities that have
//! **zero production call sites** in the consumer — on the consumer's own real
//! data — so the choice to wire them or not is made against output rather than
//! against a description.
//!
//! usage: unconsumed_capabilities <brain_dir>

use spectral::Brain;
use spectral_graph::activity::ProbeOpts;
use spectral_graph::brain::AaakOpts;

fn line() {
    println!("{}", "─".repeat(78));
}

fn snip(s: &str, n: usize) -> String {
    let one: String = s.chars().take(n).collect();
    one.replace('\n', " ")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::args()
        .nth(1)
        .ok_or("usage: unconsumed_capabilities <brain_dir>")?;
    let path = std::path::Path::new(&dir);
    let brain = Brain::builder()
        .data_dir(path)
        .ontology_path(path.join("ontology.toml"))
        .read_only(true)
        .build()?;

    let conn = rusqlite::Connection::open_with_flags(
        path.join("memory.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let memories: i64 = conn.query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0))?;
    let pairs: i64 = conn
        .query_row("SELECT COUNT(*) FROM co_retrieval_pairs", [], |r| r.get(0))
        .unwrap_or(0);
    println!("brain: {memories} memories, {pairs} co-retrieval pairs\n");

    // A seed with associations, so the recommender has something to work with.
    let seed: Option<(String, String)> = conn
        .query_row(
            "SELECT m.id, m.content FROM co_retrieval_pairs p
             JOIN memories m ON m.id = p.memory_id_a
             GROUP BY p.memory_id_a ORDER BY SUM(p.co_count) DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok();

    line();
    println!("aaak() — foundational facts as a token-budgeted system-prompt block");
    line();
    match brain.aaak(AaakOpts {
        max_tokens: 300,
        ..Default::default()
    }) {
        Ok(a) => {
            let text = format!("{a:?}");
            println!("{}\n", snip(&text, 700));
        }
        Err(e) => println!("error: {e}\n"),
    }

    if let Some((id, content)) = &seed {
        line();
        println!("related_memories() — co-retrieval associations of a real memory");
        line();
        println!("seed: {}\n", snip(content, 110));
        match brain.related_memories(id, 5) {
            Ok(rs) => {
                for r in &rs {
                    println!("  {}", snip(&format!("{r:?}"), 150));
                }
                if rs.is_empty() {
                    println!("  (none)");
                }
            }
            Err(e) => println!("  error: {e}"),
        }
        println!();

        line();
        println!("recommend() — anticipatory recall, ranked by lift not raw count");
        line();
        match brain.recommend(id, 5, 1) {
            Ok(rs) => {
                for r in &rs {
                    println!("  {}", snip(&format!("{r:?}"), 150));
                }
                if rs.is_empty() {
                    println!("  (none)");
                }
            }
            Err(e) => println!("  error: {e}"),
        }
        println!();
    }

    line();
    println!("probe() — memories relevant to a current cognitive state");
    line();
    let context = "debugging a failing deploy, checking why the build broke and what changed";
    println!("context: \"{context}\"\n");
    match brain.probe(
        context,
        ProbeOpts {
            max_results: 5,
            ..Default::default()
        },
    ) {
        Ok(rs) => {
            for r in &rs {
                println!("  {}", snip(&format!("{r:?}"), 160));
            }
            if rs.is_empty() {
                println!("  (none)");
            }
        }
        Err(e) => println!("  error: {e}"),
    }
    Ok(())
}
