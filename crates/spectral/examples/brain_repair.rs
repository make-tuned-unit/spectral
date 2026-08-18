//! Repair a brain's derived state, with health reported before and after.
//!
//! `remember` writes the memory row first and derives the rest afterwards —
//! content hash, declarative density, signature, recognition enrolment — and
//! those steps are **non-fatal**: a failure logs a warning and carries on, so
//! the row survives while its derived state does not. Over a long-lived brain
//! those failures accumulate silently.
//!
//! [`Brain::repair_derivations`] is idempotent and never changes or deletes a
//! primary memory row. It re-enrols every scanned memory unconditionally rather
//! than diffing first, so it also repairs a torn recognition index.
//!
//! Run:
//! ```text
//! cargo run -p spectral --example brain_repair -- <brain-dir> [--apply]
//! ```
//!
//! Defaults to a **dry run**: it reports what is missing and changes nothing.
//! Pass `--apply` to actually repair. Take a backup first — the three databases
//! in a brain are not written atomically with respect to each other, so this
//! should be run with no other process holding the brain open.

use spectral::Brain;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let dir = args
        .get(1)
        .ok_or("usage: brain_repair <brain-dir> [--apply]")?;
    let apply = args.iter().any(|a| a == "--apply");

    // A limit large enough to cover the whole corpus; `derivation_health` and
    // `repair_derivations` both scan up to this many memories.
    const SCAN: usize = 1_000_000;

    let brain = Brain::open(dir)?;

    let before = brain.derivation_health(SCAN)?;
    println!("Derivation health — {dir}");
    println!("  scanned                        : {}", before.scanned);
    println!(
        "  missing content_hash           : {}",
        before.missing_content_hash
    );
    println!(
        "  missing declarative_density    : {}",
        before.missing_declarative_density
    );
    println!(
        "  missing signature              : {}",
        before.missing_signature
    );
    println!(
        "  missing recognition enrolment  : {}",
        before.missing_recognition_enrollment
    );
    println!(
        "  orphaned recognition entries   : {}",
        before.orphaned_recognition_entries
    );
    println!("  healthy                        : {}", before.is_healthy());

    if before.is_healthy() {
        println!("\nNothing to repair.");
        return Ok(());
    }

    if !apply {
        println!("\nDry run — nothing was changed. Re-run with --apply to repair.");
        return Ok(());
    }

    println!("\nRepairing…");
    let rep = brain.repair_derivations(SCAN)?;
    println!("  scanned                        : {}", rep.scanned);
    println!(
        "  content hashes repaired        : {}",
        rep.content_hashes_repaired
    );
    println!(
        "  densities repaired             : {}",
        rep.densities_repaired
    );
    println!(
        "  signatures repaired            : {}",
        rep.signatures_repaired
    );
    println!(
        "  recognition enrolments refreshed: {}",
        rep.recognition_enrollments_refreshed
    );
    println!(
        "  orphaned enrolments pruned     : {}",
        rep.orphaned_enrollments_pruned
    );

    // Re-read health rather than trusting the repair's own account of itself.
    // A repair that reported success while leaving the gap open is exactly the
    // failure this whole exercise has been about.
    let after = brain.derivation_health(SCAN)?;
    println!("\nHealth after repair");
    println!(
        "  missing content_hash           : {}",
        after.missing_content_hash
    );
    println!(
        "  missing declarative_density    : {}",
        after.missing_declarative_density
    );
    println!(
        "  missing signature              : {}",
        after.missing_signature
    );
    println!(
        "  missing recognition enrolment  : {}",
        after.missing_recognition_enrollment
    );
    println!(
        "  orphaned recognition entries   : {}",
        after.orphaned_recognition_entries
    );
    println!("  healthy                        : {}", after.is_healthy());

    if !after.is_healthy() {
        println!(
            "\nStill not healthy. Remaining gaps are not repairable by this pass — \
             check the warnings above and the brain's logs."
        );
    }
    Ok(())
}
