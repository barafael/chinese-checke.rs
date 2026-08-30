//! Renders the specification from [`checkers_core`].
//!
//! The output is **build output**, not a source file: edit the chapter prose in
//! `checkers-core/src/spec.rs` and the `Law` impls in `checkers-core/src/laws/`.
//! Because the document and the test suite read the same registry, a law cannot
//! be documented without being checked, or checked without being documented.
//!
//! Why generate a document at all when rustdoc exists: rustdoc sorts items
//! alphabetically and offers no stable way to override that, so it cannot present
//! chapters in reading order. rustdoc is the right view for reading a claim
//! beside its code; this is the right view for reading the specification as a
//! specification.
//!
//! It also generates the wasm law registry, for the same reason: the registry
//! and the document are both views of one link-time-collected truth. See
//! [`registry`].
//!
//! ```text
//! cargo run -p checkers-spec-gen -- specs/specification.md
//! cargo run -p checkers-spec-gen -- --check specs/specification.md
//! cargo run -p checkers-spec-gen -- --emit-registry
//! cargo run -p checkers-spec-gen -- --check-registry
//! ```

mod registry;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use checkers_core::law::{Evidence, LawInfo, all_in_reading_order, for_chapter};
use checkers_core::spec::Chapter;

const PREAMBLE: &str = "\
This document is generated from `checkers-core`. Do not edit it; edit the chapter \
prose in `checkers-core/src/spec.rs` or the law impls in \
`checkers-core/src/laws/`, then regenerate.

Each numbered chapter states the rules in prose and mathematics. The **laws** \
listed under a chapter are the machine-checked formalisation of its claims: every \
law is a Rust type whose statement, provenance, and executable check live in one \
place, and which is registered at link time so it cannot be documented without \
being verified.
";

fn evidence_note() -> String {
    let mut s = String::new();
    s.push_str("Each law records how strongly it is established:\n\n");
    s.push_str("| Evidence | Meaning |\n|---|---|\n");
    s.push_str(
        "| proof (Kani) | Proven for the whole domain by bounded model checking. |\n\
         | exhaustive | Checked over a finite domain by enumeration. |\n\
         | property test | Checked over inputs from a generated strategy. |\n\
         | example | Checked against fixed examples only. |\n\n",
    );
    s.push_str(
        "`proof (Kani)` laws additionally re-check themselves in ordinary Rust, so \
         `cargo test` exercises them on every platform; the proofs themselves need \
         Linux or WSL, since Kani does not build on Windows.\n",
    );
    s
}

fn render_law(out: &mut String, law: &LawInfo) {
    writeln!(out, "##### `{}`\n", law.id).unwrap();
    writeln!(out, "{}\n", law.summary).unwrap();
    writeln!(out, "$$\n{}\n$$\n", law.statement).unwrap();
    writeln!(out, "*Evidence: {}*\n", law.evidence.label()).unwrap();
}

fn render_contents(out: &mut String) {
    writeln!(out, "## Contents\n").unwrap();
    for chapter in Chapter::ALL {
        let n = for_chapter(chapter).len();
        let laws = match n {
            0 => String::new(),
            1 => " — 1 law".to_string(),
            n => format!(" — {n} laws"),
        };
        writeln!(
            out,
            "{}. [{}](#{}){}",
            chapter.number(),
            chapter.title(),
            chapter.slug(),
            laws
        )
        .unwrap();
    }
    out.push('\n');
}

fn render_coverage(out: &mut String) {
    let laws = all_in_reading_order();
    let mut by_evidence: BTreeMap<&str, usize> = BTreeMap::new();
    for law in &laws {
        *by_evidence.entry(law.evidence.label()).or_default() += 1;
    }

    writeln!(out, "## Coverage\n").unwrap();
    writeln!(out, "| Evidence | Laws |\n|---|---|").unwrap();
    for (label, count) in &by_evidence {
        writeln!(out, "| {label} | {count} |").unwrap();
    }
    writeln!(out, "| **total** | **{}** |\n", laws.len()).unwrap();

    let unformalised: Vec<Chapter> = Chapter::ALL
        .into_iter()
        .filter(|c| for_chapter(*c).is_empty())
        .collect();

    if !unformalised.is_empty() {
        writeln!(
            out,
            "The following chapters are stated in prose but not yet formalised as \
             laws, so their claims are **not** machine-checked:\n"
        )
        .unwrap();
        for c in unformalised {
            writeln!(out, "- {}. {}", c.number(), c.title()).unwrap();
        }
        out.push('\n');
    }

    out.push_str(&evidence_note());
    out.push('\n');
}

fn render() -> String {
    let mut out = String::new();

    out.push_str("# Chinese Checkers — specification\n\n");
    out.push_str(
        "<!-- GENERATED FILE. Do not edit.\n     \
         Source: checkers-core/src/spec.rs and checkers-core/src/laws/\n     \
         Regenerate: cargo run -p checkers-spec-gen -- specs/specification.md -->\n\n",
    );
    out.push_str(PREAMBLE);
    out.push('\n');

    render_contents(&mut out);
    render_coverage(&mut out);

    out.push_str("---\n\n");

    for chapter in Chapter::ALL {
        writeln!(
            out,
            "## {}. {} <a id=\"{}\"></a>\n",
            chapter.number(),
            chapter.title(),
            chapter.slug()
        )
        .unwrap();
        writeln!(out, "{}\n", chapter.prose()).unwrap();

        let laws = for_chapter(chapter);
        if !laws.is_empty() {
            writeln!(out, "#### Laws\n").unwrap();
            for law in laws {
                render_law(&mut out, law);
            }
        }
    }

    out
}

/// The workspace root, derived from this crate's manifest directory so the tool
/// works regardless of the caller's working directory.
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the crate directory has a parent")
        .to_path_buf()
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // Registry modes take no path: the destination is fixed, because the file is
    // `include!`d from a known location in checkers-core.
    match args.as_slice() {
        [flag] if flag == "--emit-registry" => {
            return match registry::emit(&workspace_root()) {
                Ok(n) => {
                    println!("wrote {}: {n} laws", registry::GENERATED_PATH);
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::FAILURE
                }
            };
        }
        [flag] if flag == "--check-registry" => {
            return match registry::check(&workspace_root()) {
                Ok(n) => {
                    println!("{} is up to date: {n} laws", registry::GENERATED_PATH);
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::FAILURE
                }
            };
        }
        _ => {}
    }

    let (check_only, path) = match args.as_slice() {
        [flag, path] if flag == "--check" => (true, path.clone()),
        [path] => (false, path.clone()),
        _ => {
            eprintln!(
                "usage: checkers-spec-gen [--check] <output.md>\n       \
                 checkers-spec-gen --emit-registry | --check-registry"
            );
            return ExitCode::from(2);
        }
    };

    let rendered = render();
    let laws = all_in_reading_order();
    let proven = laws
        .iter()
        .filter(|l| l.evidence == Evidence::Proof)
        .count();

    if check_only {
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        if existing.replace("\r\n", "\n") == rendered {
            println!(
                "{path} is up to date: {} chapters, {} laws",
                Chapter::ALL.len(),
                laws.len()
            );
            return ExitCode::SUCCESS;
        }
        eprintln!("{path} is stale. Regenerate with:\n  cargo run -p checkers-spec-gen -- {path}");
        return ExitCode::FAILURE;
    }

    if let Some(parent) = std::path::Path::new(&path).parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        eprintln!("failed to create {}: {e}", parent.display());
        return ExitCode::FAILURE;
    }

    match std::fs::write(&path, &rendered) {
        Ok(()) => {
            println!(
                "wrote {path}: {} chapters, {} laws ({proven} Kani-proven)",
                Chapter::ALL.len(),
                laws.len()
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("failed to write {path}: {e}");
            ExitCode::FAILURE
        }
    }
}
