//! Runs every registered law, and checks the registry's own integrity.
//!
//! This file names no individual law. Laws are collected at link time, so adding
//! one to `src/laws/` is enough to have it checked here — the failure mode
//! "declared a law but forgot to test it" does not exist.

use checkers_core::law::{Evidence, LAWS, all_in_reading_order, for_chapter, verify_all};
use checkers_core::spec::Chapter;

#[test]
fn every_registered_law_holds() {
    if let Err(violation) = verify_all() {
        panic!("{violation}");
    }
}

#[test]
fn laws_are_actually_registered() {
    // Guards against the linker dropping the distributed slice, which would make
    // the suite above vacuously pass.
    assert!(
        LAWS.len() >= 12,
        "expected the geometry laws to be registered, found {}",
        LAWS.len()
    );
}

#[test]
fn law_ids_are_unique() {
    let mut ids: Vec<&str> = LAWS.iter().map(|l| l.id).collect();
    let total = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), total, "duplicate law IDs found");
}

#[test]
fn law_metadata_is_populated() {
    for law in LAWS {
        assert!(!law.id.is_empty(), "law with empty ID");
        assert!(
            law.id.starts_with("CC-"),
            "law {} should use the CC- prefix",
            law.id
        );
        assert!(!law.statement.is_empty(), "law {} has no statement", law.id);
        assert!(!law.summary.is_empty(), "law {} has no summary", law.id);
        assert!(
            law.summary.ends_with('.'),
            "law {} summary should be a sentence",
            law.id
        );
    }
}

/// LaTeX in `STATEMENT` is rendered by rustdoc + KaTeX and embedded in the
/// generated markdown, so unbalanced delimiters would break both.
#[test]
fn law_statements_have_balanced_delimiters() {
    for law in LAWS {
        let s = law.statement;
        for (open, close) in [('{', '}'), ('[', ']'), ('(', ')')] {
            let opens = s.matches(open).count();
            let closes = s.matches(close).count();
            assert_eq!(
                opens, closes,
                "law {} has unbalanced {open}{close} in: {s}",
                law.id
            );
        }
        assert!(
            !s.contains("$$"),
            "law {} should carry bare LaTeX without delimiters",
            law.id
        );
    }
}

#[test]
fn proof_backed_laws_are_also_runtime_checked() {
    let proven: Vec<&str> = LAWS
        .iter()
        .filter(|l| l.evidence == Evidence::Proof)
        .map(|l| l.id)
        .collect();

    assert!(
        !proven.is_empty(),
        "expected some laws to be backed by Kani proofs"
    );

    for law in LAWS.iter().filter(|l| l.evidence == Evidence::Proof) {
        (law.verify)().unwrap_or_else(|e| panic!("proof-backed law failed its runtime check: {e}"));
    }
}

// ---------------------------------------------------------------------------
// Chapter structure: the reading order that rustdoc cannot provide.
// ---------------------------------------------------------------------------

#[test]
fn reading_order_is_by_chapter_then_id() {
    let laws = all_in_reading_order();
    for pair in laws.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let ord = (a.chapter.number(), a.id).cmp(&(b.chapter.number(), b.id));
        assert!(
            ord.is_le(),
            "reading order violated: {} (ch. {}) before {} (ch. {})",
            a.id,
            a.chapter.number(),
            b.id,
            b.chapter.number()
        );
    }
}

#[test]
fn reading_order_includes_every_law() {
    assert_eq!(all_in_reading_order().len(), LAWS.len());
}

#[test]
fn chapter_partition_is_exact() {
    // Every law belongs to exactly one chapter, and the per-chapter lists
    // reconstruct the whole registry.
    let total: usize = Chapter::ALL.iter().map(|c| for_chapter(*c).len()).sum();
    assert_eq!(
        total,
        LAWS.len(),
        "laws are missing from, or duplicated across, chapters"
    );
}

/// A chapter with prose but no laws is not an error — some chapters are purely
/// narrative — but it is worth surfacing, since it usually means the formalisation
/// is incomplete.
#[test]
fn report_chapters_without_laws() {
    let empty: Vec<&str> = Chapter::ALL
        .iter()
        .filter(|c| for_chapter(**c).is_empty())
        .map(|c| c.title())
        .collect();

    println!("\n{} chapters have no laws yet:", empty.len());
    for title in &empty {
        println!("  - {title}");
    }
}

/// Prints the law inventory in reading order. Run with `--nocapture`.
#[test]
fn law_inventory() {
    println!(
        "\n{} registered laws, in specification order:\n",
        LAWS.len()
    );
    let mut current = None;
    for law in all_in_reading_order() {
        if current != Some(law.chapter) {
            println!("  {}. {}", law.chapter.number(), law.chapter.title());
            current = Some(law.chapter);
        }
        println!(
            "       {:<24} {:<16} {}",
            law.id,
            law.evidence.label(),
            law.summary
        );
    }
}
