//! Generates `checkers-core/src/laws_generated.rs`, the law registry used on
//! `wasm32`.
//!
//! `checkers_core::law::LAWS` is a `linkme` distributed slice on native
//! targets, but `linkme` has no `wasm32` support, so the web build needs the
//! array spelled out. This generator writes it from a native run, where the
//! linker has done the collecting.
//!
//! The file is a derivative and can go stale, so two checks guard it:
//! `--check-registry` (CI) and a law-identity comparison on every native
//! `cargo test`.
//!
//! Law *types* are discovered by reading `register_law!` invocations from the
//! source — the one place that names both type and ID. Parsing text is
//! ordinarily weak, but it is cross-checked against the linker: any
//! disagreement fails generation rather than emitting a wrong registry.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::Path;

use checkers_core::law::all_sorted;

/// Where the registry is written, relative to the workspace root.
pub const GENERATED_PATH: &str = "checkers-core/src/laws_generated.rs";

/// The modules scanned for `register_law!` invocations.
const LAW_SOURCES: [(&str, &str); 2] = [
    (
        "crate::laws::geometry",
        "checkers-core/src/laws/geometry.rs",
    ),
    ("crate::laws::rules", "checkers-core/src/laws/rules.rs"),
];

/// A law as named at its registration site.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Registration {
    /// Fully qualified path, e.g. `crate::laws::geometry::RotationOrderSix`.
    pub path: String,
    /// The bare type name, for error messages.
    pub type_name: String,
}

#[derive(Debug)]
pub enum RegistryError {
    Read {
        path: String,
        source: std::io::Error,
    },
    Write {
        path: String,
        source: std::io::Error,
    },
    /// The source scan and the linker disagree about which laws exist.
    Disagreement {
        scanned: usize,
        linked: usize,
        detail: String,
    },
    /// The file on disk is not what this generator would write.
    Stale,
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegistryError::Read { path, source } => write!(f, "cannot read {path}: {source}"),
            RegistryError::Write { path, source } => write!(f, "cannot write {path}: {source}"),
            RegistryError::Disagreement {
                scanned,
                linked,
                detail,
            } => write!(
                f,
                "the register_law! scan found {scanned} law(s) but the linker \
                 collected {linked}: {detail}"
            ),
            RegistryError::Stale => write!(
                f,
                "{GENERATED_PATH} is stale. Regenerate with:\n  \
                 cargo run -p checkers-spec-gen -- --emit-registry"
            ),
        }
    }
}

/// Extract the law type named by each `register_law!(Type, SLOT);` invocation.
///
/// Scans the whole text, not lines: rustfmt wraps long invocations, and a
/// line-oriented scan silently missed four of the forty-two laws.
fn scan(source: &str, module: &str) -> Vec<Registration> {
    const NEEDLE: &str = "register_law!(";
    let mut out = Vec::new();
    let mut rest = source;

    while let Some(at) = rest.find(NEEDLE) {
        // Everything before the match, to tell an invocation from a mention of
        // the macro in prose or its own definition.
        let preceding = rest[..at].rsplit('\n').next().unwrap_or_default().trim();
        let after = &rest[at + NEEDLE.len()..];
        rest = after;

        // A doc comment or the `macro_rules!` arm, not a call.
        if preceding.starts_with("//") || preceding.starts_with('#') || preceding.contains('`') {
            continue;
        }

        // `split` on a `&str` always yields at least its first segment, so
        // neither extraction below can be empty-handed.
        let args = after.split(')').next().unwrap_or_default();
        let type_name = args.split(',').next().unwrap_or_default().trim();
        // A `$law:ty` metavariable or anything else that is not a type name.
        if !type_name.starts_with(|c: char| c.is_ascii_uppercase())
            || !type_name.chars().all(|c| c.is_alphanumeric() || c == '_')
        {
            continue;
        }

        out.push(Registration {
            path: format!("{module}::{type_name}"),
            type_name: type_name.to_string(),
        });
    }
    out
}

/// Scan every law module, then verify the result against the linker.
pub fn collect(root: &Path) -> Result<Vec<Registration>, RegistryError> {
    let mut found = Vec::new();
    for (module, relative) in LAW_SOURCES {
        let path = root.join(relative);
        let source = std::fs::read_to_string(&path).map_err(|source| RegistryError::Read {
            path: relative.to_string(),
            source,
        })?;
        found.extend(scan(&source, module));
    }
    found.sort();

    // The scan is only trustworthy because this comparison exists.
    let linked = all_sorted();
    if found.len() != linked.len() {
        let scanned_names: BTreeSet<&str> = found.iter().map(|r| r.type_name.as_str()).collect();
        return Err(RegistryError::Disagreement {
            scanned: found.len(),
            linked: linked.len(),
            detail: format!(
                "scanned types: {scanned_names:?}. A law registered by a macro-generated \
                 or non-literal register_law! call cannot be discovered by the scan."
            ),
        });
    }

    Ok(found)
}

/// Render the registry file.
pub fn render(laws: &[Registration]) -> String {
    let mut out = String::new();

    out.push_str(
        "// @generated by checkers-spec-gen. Do not edit.\n\
         //\n\
         // The law registry for wasm32, where `linkme` cannot collect anything (no\n\
         // linker-defined section-boundary symbols exist to take the address of).\n\
         // Native builds ignore this file entirely and use the distributed slice,\n\
         // which is the authority this file is derived from.\n\
         //\n\
         // Regenerate:      cargo run -p checkers-spec-gen -- --emit-registry\n\
         // Check freshness: cargo run -p checkers-spec-gen -- --check-registry\n\n",
    );

    // The IDs the linker reported, so the native cross-check can compare law
    // *identities* and not merely how many there are. A law swapped for another
    // keeps the count the same. `all_sorted` lists them by ID.
    let ids: Vec<&str> = all_sorted().iter().map(|l| l.id).collect();
    let _ = writeln!(out, "// law-ids: {}\n", ids.join(","));

    let _ = writeln!(
        out,
        "pub static LAWS: [crate::law::LawInfo; {}] = [",
        laws.len()
    );
    for law in laws {
        let _ = writeln!(out, "    crate::law_info!({}),", law.path);
    }
    out.push_str("];\n");

    out
}

pub fn emit(root: &Path) -> Result<usize, RegistryError> {
    let laws = collect(root)?;
    std::fs::write(root.join(GENERATED_PATH), render(&laws)).map_err(|source| {
        RegistryError::Write {
            path: GENERATED_PATH.to_string(),
            source,
        }
    })?;
    Ok(laws.len())
}

pub fn check(root: &Path) -> Result<usize, RegistryError> {
    let laws = collect(root)?;
    if crate::is_current(&root.join(GENERATED_PATH), &render(&laws)) {
        Ok(laws.len())
    } else {
        Err(RegistryError::Stale)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace_root;

    #[test]
    fn the_scan_reads_a_registration() {
        let found = scan(
            "pub struct Foo;\nregister_law!(Foo, FOO);\n",
            "crate::laws::geometry",
        );
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, "crate::laws::geometry::Foo");
    }

    /// rustfmt wraps invocations with long arguments, and a line-oriented scan
    /// missed exactly these — four of the forty-two laws.
    #[test]
    fn the_scan_reads_a_wrapped_registration() {
        let found = scan(
            "register_law!(\n    DirectionsCloseUnderNegation,\n    DIRECTIONS_CLOSE\n);\n",
            "m",
        );
        assert_eq!(found.len(), 1, "a wrapped invocation must still be found");
        assert_eq!(found[0].type_name, "DirectionsCloseUnderNegation");
    }

    #[test]
    fn the_scan_ignores_prose_and_the_macro_definition() {
        // A doc comment mentioning the macro, and the macro's own arm.
        let source = "\
/// Register each one with [`crate::register_law!`].
macro_rules! register_law {
    ($law:ty, $slot:ident) => {};
}
register_law!(Real, REAL);
";
        let found = scan(source, "m");
        assert_eq!(
            found
                .iter()
                .map(|r| r.type_name.as_str())
                .collect::<Vec<_>>(),
            vec!["Real"],
            "only the real invocation should be picked up"
        );
    }

    /// The generated array must reference every law the linker knows about.
    /// This is the check that makes text scanning acceptable.
    #[test]
    fn the_scan_agrees_with_the_linker() {
        let root = workspace_root();
        let found = collect(&root).expect("scan must agree with the linker");
        assert_eq!(found.len(), all_sorted().len());
    }

    #[test]
    fn the_generated_file_is_up_to_date() {
        let root = workspace_root();
        if let Err(e) = check(&root) {
            panic!("{e}");
        }
    }
}
