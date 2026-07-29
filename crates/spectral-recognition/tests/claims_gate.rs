//! Claims-drift gate (prereg C4 enforcement).
//!
//! Every AUC cited in the public `docs/RECOGNITION_RESULTS.md` table must
//! match the canonical measured results in
//! `docs/recognition-benchmark-results.json` to 4 decimal places. If a doc
//! edit drifts a claim away from its measurement — or cites a number with no
//! measurement behind it — this test fails the build. Nobody has to trust
//! the prose; the repo structurally cannot overclaim.

use std::path::PathBuf;

fn docs_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs")
        .join(name)
}

#[test]
fn public_results_doc_matches_canonical_measurements() {
    let json: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(docs_path("recognition-benchmark-results.json"))
            .expect("canonical results JSON must exist alongside the public doc"),
    )
    .expect("canonical results JSON must parse");

    let md = std::fs::read_to_string(docs_path("RECOGNITION_RESULTS.md"))
        .expect("public results doc must exist");

    // Canonical per-regime AUCs, rounded exactly as the doc cites them.
    let mut expected: Vec<(String, [f64; 3])> = Vec::new();
    for regime in json["regimes"].as_array().expect("regimes array") {
        let name = regime["name"].as_str().unwrap().to_uppercase();
        let round4 = |v: &serde_json::Value| (v.as_f64().unwrap() * 10_000.0).round() / 10_000.0;
        expected.push((
            name,
            [
                round4(&regime["engine"]["auc"]),
                round4(&regime["minhash128"]["auc"]),
                round4(&regime["bge_small"]["auc"]),
            ],
        ));
    }
    assert_eq!(expected.len(), 3, "three regimes are pre-registered");

    // Parse the doc's table rows: "| R1 — ... | a | b | c |"
    for (name, aucs) in &expected {
        let row = md
            .lines()
            .find(|l| l.starts_with(&format!("| {name} ")) || l.starts_with(&format!("| {name}—")))
            .unwrap_or_else(|| panic!("results doc must contain a table row for {name}"));
        let cells: Vec<&str> = row.split('|').map(str::trim).collect();
        // cells[0] = "", cells[1] = regime label, cells[2..5] = the three AUCs
        let cited: Vec<f64> = cells[2..5]
            .iter()
            .map(|c| {
                c.trim_matches('*')
                    .trim()
                    .parse::<f64>()
                    .unwrap_or_else(|_| panic!("unparseable AUC cell {c:?} in {name} row"))
            })
            .collect();
        for (i, (cited_v, canon_v)) in cited.iter().zip(aucs.iter()).enumerate() {
            assert!(
                (cited_v - canon_v).abs() < 5e-5,
                "{name} column {i}: doc cites {cited_v}, canonical measurement is {canon_v} — \
                 claims may not drift from evidence"
            );
        }
    }
}
