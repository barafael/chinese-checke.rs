//! Renders the specification from [`checkers_core`].
//!
//! The output is **build output**: edit `checkers-core/src/spec.rs` and the
//! `Law` impls, then regenerate. It exists because rustdoc sorts items
//! alphabetically and cannot present chapters in reading order. It also
//! generates the wasm law registry — both are views of one link-time-collected
//! truth. See [`registry`].
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
use std::path::{Path, PathBuf};
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

const EVIDENCE_NOTE: &str = "\
Each law records how strongly it is established:

| Evidence | Meaning |
|---|---|
| proof (Kani) | Proven for the whole domain by bounded model checking. |
| exhaustive | Checked over a finite domain by enumeration. |
| property test | Checked over inputs from a generated strategy. |

`proof (Kani)` laws additionally re-check themselves in ordinary Rust, so \
`cargo test` exercises them on every platform; the proofs themselves need \
Linux or WSL, since Kani does not build on Windows.
";

fn render_law(out: &mut String, law: &LawInfo) {
    writeln!(out, "##### `{}`\n", law.id).unwrap();
    // The plain-language statement leads: it is what a first-time reader
    // needs, and what the mathematics below is held accountable to.
    writeln!(out, "**In plain terms:** {}\n", law.note).unwrap();
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

    out.push_str(EVIDENCE_NOTE);
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

/// Whether the file at `path` already holds `rendered`, reading CRLF line
/// endings as LF. A file that cannot be read does not.
fn is_current(path: &Path, rendered: &str) -> bool {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .replace("\r\n", "\n")
        == rendered
}

/// `--check`: the specification at `path` must be exactly what `render` makes.
fn check_spec(path: &str) -> Result<String, String> {
    if !is_current(Path::new(path), &render()) {
        return Err(format!(
            "{path} is stale. Regenerate with:\n  cargo run -p checkers-spec-gen -- {path}"
        ));
    }
    let laws = all_in_reading_order().len();
    Ok(format!(
        "{path} is up to date: {} chapters, {laws} laws",
        Chapter::ALL.len()
    ))
}

/// Write the specification to `path`, creating its directory if need be.
fn write_spec(path: &str) -> Result<String, String> {
    if let Some(parent) = Path::new(path).parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
    }
    std::fs::write(path, render()).map_err(|e| format!("failed to write {path}: {e}"))?;

    let laws = all_in_reading_order();
    let proven = laws
        .iter()
        .filter(|l| l.evidence == Evidence::Proof)
        .count();
    Ok(format!(
        "wrote {path}: {} chapters, {} laws ({proven} Kani-proven)",
        Chapter::ALL.len(),
        laws.len()
    ))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // Every mode ends in a summary on stdout or an error on stderr.
    let outcome = match args.as_slice() {
        // Registry modes take no path: the destination is fixed, because the
        // file is `include!`d from a known location in checkers-core.
        [flag] if flag == "--emit-registry" => registry::emit(&workspace_root())
            .map(|n| format!("wrote {}: {n} laws", registry::GENERATED_PATH))
            .map_err(|e| e.to_string()),
        [flag] if flag == "--check-registry" => registry::check(&workspace_root())
            .map(|n| format!("{} is up to date: {n} laws", registry::GENERATED_PATH))
            .map_err(|e| e.to_string()),
        [flag, path] if flag == "--check" => check_spec(path),
        [path] => write_spec(path),
        _ => {
            eprintln!(
                "usage: checkers-spec-gen [--check] <output.md>\n       \
                 checkers-spec-gen --emit-registry | --check-registry"
            );
            return ExitCode::from(2);
        }
    };
    match outcome {
        Ok(summary) => {
            println!("{summary}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
