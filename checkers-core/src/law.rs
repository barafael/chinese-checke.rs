//! The [`Law`] trait: a normative claim from the specification, expressed as a
//! Rust type that carries its own statement *and* its own check.
//!
//! # Why a trait rather than a doc comment
//!
//! A `// §16` comment is a string. Nothing detects it when a section is
//! renumbered, when a rule changes, or when a claim is silently dropped. A
//! `Law` impl puts four things in one block that cannot drift apart:
//!
//! | Face | Field |
//! |---|---|
//! | Stable identity | [`Law::ID`] |
//! | The mathematics | [`Law::STATEMENT`] |
//! | Where it came from | [`Law::CHAPTER`] |
//! | What it means operationally | [`Law::holds`] |
//!
//! The domain the claim is quantified over is [`Law::Subject`] — that is what
//! turns a $\forall$ into something a property test can generate.
//!
//! # What this does and does not give you
//!
//! Registration is by *link time collection*, so a declared law is always
//! executed and always documented; you cannot forget one. This is real
//! traceability.
//!
//! It is **not** proof. [`Law::holds`] is a runtime predicate: the type system
//! does not check that [`Law::STATEMENT`] means what [`Law::holds`] computes.
//! Laws whose domain is small and arithmetic are additionally *proven* with the
//! Kani harnesses in [`crate::geometry`]; the rest are checked over generated
//! inputs by the property tests. See the crate docs for the split.

use core::fmt::Debug;

// wasm has no distributed slice; the registry is a generated array there.
#[cfg(not(target_family = "wasm"))]
use linkme::distributed_slice;

use crate::spec::Chapter;

/// A link-time record of one law, used by the test harness and the spec
/// generator so neither needs a hand-maintained list.
#[derive(Debug)]
pub struct LawInfo {
    /// Stable identifier, e.g. `"CC-INV-PIECES"`.
    pub id: &'static str,
    /// The formal statement, in LaTeX (rendered by rustdoc + KaTeX).
    pub statement: &'static str,
    /// Which chapter of the specification this claim belongs to.
    ///
    /// A [`Chapter`] rather than a string: a law cannot cite a section that does
    /// not exist, and there are no numbers to renumber.
    pub chapter: Chapter,
    /// One-line prose gloss of the claim.
    pub summary: &'static str,
    /// How the claim is established.
    pub evidence: Evidence,
    /// Type-erased driver that checks the law over its own subjects.
    pub verify: fn() -> Result<(), Violation>,
}

/// How strongly a law is established.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Evidence {
    /// Proven for the entire domain by the Kani harnesses in
    /// [`crate::geometry`]. The strongest level available here.
    Proof,
    /// Checked over inputs produced by a `proptest` strategy.
    Property,
    /// Checked exhaustively over a finite domain in ordinary Rust.
    Exhaustive,
    /// Checked against fixed examples only — the weakest level.
    Example,
}

impl Evidence {
    pub const fn label(self) -> &'static str {
        match self {
            Evidence::Proof => "proof (Kani)",
            Evidence::Property => "property test",
            Evidence::Exhaustive => "exhaustive",
            Evidence::Example => "example",
        }
    }
}

/// A law violation, carrying enough context to identify the claim and the input.
#[derive(Debug, Clone)]
pub struct Violation {
    pub id: &'static str,
    pub chapter: Chapter,
    pub detail: String,
    pub subject: String,
}

impl core::fmt::Display for Violation {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "law {} (ch. {} {}) violated: {}\n  subject: {}",
            self.id,
            self.chapter.number(),
            self.chapter.title(),
            self.detail,
            self.subject
        )
    }
}

impl core::error::Error for Violation {}

/// Every law, however this target manages to collect them.
///
/// Iterate it to check every law ([`verify_all`]) or to generate the
/// specification document.
///
/// # Two mechanisms, one authority
///
/// On native targets this is a `linkme` distributed slice: each
/// [`crate::register_law!`] places its record in a named linker section and the
/// linker concatenates them. Registration is therefore *unforgettable* — the
/// law is collected because it exists, and there is no list to update.
///
/// `linkme` has no `wasm32` implementation (WebAssembly has no
/// linker-defined section-boundary symbols to take the address of), so web
/// builds use the generated `laws_generated.rs` array instead — emitted by
/// `checkers-spec-gen` from the native build, where the linker *did* do the
/// collecting.
///
/// That makes native the authority and the generated file a derivative. The
/// derivative can go stale, so two things guard it:
///
/// - `cargo run -p checkers-spec-gen -- --check-registry` fails if the file
///   disagrees with the linker. CI runs it, and so does the deploy workflow
///   before it builds.
/// - `tests::the_generated_registry_matches_the_linker` compares law
///   *identities*, not just counts, on every native `cargo test`.
///
/// This is weaker than link-time collection — a forgotten regeneration is
/// possible where a forgotten registration was not — but it is *mechanically
/// checked* rather than trusted, which is the most that can be had while
/// wasm lacks the primitive.
#[cfg(not(target_family = "wasm"))]
#[distributed_slice]
pub static LAWS: [LawInfo];

#[cfg(target_family = "wasm")]
pub use generated::LAWS;

#[cfg(target_family = "wasm")]
mod generated {
    include!("laws_generated.rs");
}

/// A normative claim from the specification.
///
/// Implementors are zero-sized marker types; see [`crate::laws`] for the full
/// set. Register each one with [`crate::register_law!`].
pub trait Law {
    /// Stable identifier. Never reuse or renumber these — they are the anchor
    /// that survives section renumbering.
    const ID: &'static str;
    /// The formal statement in LaTeX, without delimiters.
    const STATEMENT: &'static str;
    /// Which chapter this claim belongs to.
    const CHAPTER: Chapter;
    /// One-line prose gloss.
    const SUMMARY: &'static str;
    /// How this law is established.
    const EVIDENCE: Evidence;

    /// The domain this law is quantified over.
    type Subject: Debug;

    /// Does the law hold for `subject`? `Err` describes the violation.
    fn holds(subject: &Self::Subject) -> Result<(), String>;

    /// The inputs to check. Property-tested laws return a representative
    /// sample here; the exhaustive ones return their whole domain.
    fn subjects() -> Vec<Self::Subject>;

    /// Check the law over every subject, reporting the first violation.
    fn verify() -> Result<(), Violation> {
        for subject in Self::subjects() {
            if let Err(detail) = Self::holds(&subject) {
                return Err(Violation {
                    id: Self::ID,
                    chapter: Self::CHAPTER,
                    detail,
                    subject: format!("{subject:?}"),
                });
            }
        }
        Ok(())
    }
}

/// Build a [`LawInfo`] from a [`Law`] type.
///
/// Used by [`crate::register_law!`] and by the generated wasm registry, so the
/// two cannot describe a law differently.
#[macro_export]
macro_rules! law_info {
    ($law:ty) => {
        $crate::law::LawInfo {
            id: <$law as $crate::law::Law>::ID,
            statement: <$law as $crate::law::Law>::STATEMENT,
            chapter: <$law as $crate::law::Law>::CHAPTER,
            summary: <$law as $crate::law::Law>::SUMMARY,
            evidence: <$law as $crate::law::Law>::EVIDENCE,
            verify: || <$law as $crate::law::Law>::verify(),
        }
    };
}

/// Register a [`Law`] in the [`LAWS`] slice.
///
/// The second argument names the generated static; it only needs to be unique
/// within its module.
///
/// On `wasm32` this expands to nothing: `linkme` cannot collect anything there,
/// so the entries come from the generated array instead (see [`LAWS`]). The
/// invocation is still what the generator reads to *find* the law, so it stays
/// in the source on every target.
#[cfg(not(target_family = "wasm"))]
#[macro_export]
macro_rules! register_law {
    ($law:ty, $slot:ident) => {
        #[$crate::law::reexport::distributed_slice($crate::law::LAWS)]
        static $slot: $crate::law::LawInfo = $crate::law_info!($law);
    };
}

#[cfg(target_family = "wasm")]
#[macro_export]
macro_rules! register_law {
    ($law:ty, $slot:ident) => {};
}

/// Re-exports for [`crate::register_law!`]; not part of the public API.
#[doc(hidden)]
pub mod reexport {
    #[cfg(not(target_family = "wasm"))]
    pub use linkme::distributed_slice;
}

/// Check every registered law. Used by `tests/laws.rs`.
pub fn verify_all() -> Result<(), Violation> {
    // `.iter()` rather than `for law in LAWS`: on wasm this is an array, which
    // would be moved out of a static.
    for law in LAWS.iter() {
        (law.verify)()?;
    }
    Ok(())
}

/// All registered laws, sorted by ID for stable output.
pub fn all_sorted() -> Vec<&'static LawInfo> {
    let mut laws: Vec<&'static LawInfo> = LAWS.iter().collect();
    laws.sort_by_key(|l| l.id);
    laws
}

/// All registered laws in **specification reading order**: by chapter, then by
/// ID within a chapter.
///
/// This is the ordering the generated document uses. rustdoc cannot reproduce
/// it — it sorts items alphabetically and offers no stable way to override that
/// — which is why the specification view is generated rather than read from
/// rustdoc output.
pub fn all_in_reading_order() -> Vec<&'static LawInfo> {
    let mut laws: Vec<&'static LawInfo> = LAWS.iter().collect();
    laws.sort_by(|a, b| {
        a.chapter
            .number()
            .cmp(&b.chapter.number())
            .then_with(|| a.id.cmp(b.id))
    });
    laws
}

/// The laws belonging to one chapter, in ID order.
pub fn for_chapter(chapter: Chapter) -> Vec<&'static LawInfo> {
    let mut laws: Vec<&'static LawInfo> = LAWS.iter().filter(|l| l.chapter == chapter).collect();
    laws.sort_by_key(|l| l.id);
    laws
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The registry is not empty, and no law's ID is blank or duplicated.
    ///
    /// This exists because [`LAWS`] is a `linkme` distributed slice — `#[used]`
    /// linker statics, which current rustc **drops under LTO**. The failure is
    /// silent in the worst way: `LAWS` comes up empty, `verify_all()` iterates
    /// nothing and returns `Ok`, and the app starts announcing that it validated
    /// against a specification it never actually read.
    ///
    /// `lto = false` in the workspace manifest is the fix; this test is what
    /// notices if that line is ever removed, or if a future toolchain strips the
    /// statics anyway.
    #[test]
    fn laws_are_registered() {
        assert!(
            !LAWS.is_empty(),
            "the law registry is empty: linkme registration was stripped, \
             so verify_all() would silently check nothing"
        );

        // A lower bound, not an exact count, so adding laws does not fail the
        // build. Partial stripping would still be caught.
        assert!(
            LAWS.len() >= 40,
            "only {} law(s) registered; expected at least 40, so registration \
             looks partially stripped",
            LAWS.len()
        );

        let mut ids: Vec<&str> = LAWS.iter().map(|l| l.id).collect();
        ids.sort_unstable();
        assert!(
            ids.iter().all(|id| !id.is_empty()),
            "every law needs a non-empty ID"
        );

        let before = ids.len();
        ids.dedup();
        assert_eq!(
            ids.len(),
            before,
            "duplicate law IDs: two laws would collide in the generated document"
        );
    }

    /// The generated wasm registry lists exactly the laws the linker collected.
    ///
    /// On native targets [`LAWS`] is authoritative; the generated array is a
    /// derivative that the web build depends on. A derivative can go stale,
    /// which is precisely what link-time collection used to make impossible, so
    /// the staleness has to be caught somewhere.
    ///
    /// This compares the type names referenced by the generated file with the
    /// law types the linker knows about. Adding a law and forgetting
    /// `--emit-registry` fails here, on the *native* test run, rather than
    /// silently shipping a short registry to the web.
    ///
    /// Only runs on native: on wasm the generated file *is* `LAWS`, so comparing
    /// them would be a tautology.
    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn the_generated_registry_matches_the_linker() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/laws_generated.rs");
        let source = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("cannot read the generated registry at {path}: {e}"));

        let entries = source
            .lines()
            .filter(|l| l.trim().starts_with("crate::law_info!("))
            .count();

        // The generator records the IDs the linker gave it, so identities can be
        // compared and not just counts: a law swapped for another leaves the
        // count untouched.
        let recorded: Vec<&str> = source
            .lines()
            .find_map(|l| l.trim().strip_prefix("// law-ids: "))
            .unwrap_or_default()
            .split(',')
            .filter(|s| !s.is_empty())
            .collect();

        let mut live: Vec<&str> = LAWS.iter().map(|l| l.id).collect();
        live.sort_unstable();

        assert!(
            entries > 0,
            "the generated registry has no entries; run \
             `cargo run -p checkers-spec-gen -- --emit-registry`"
        );
        assert_eq!(
            entries,
            LAWS.len(),
            "the generated registry lists {entries} law(s) but the linker \
             collected {}. Regenerate with \
             `cargo run -p checkers-spec-gen -- --emit-registry`.",
            LAWS.len()
        );
        assert_eq!(
            recorded, live,
            "the generated registry is for a different set of laws. Regenerate \
             with `cargo run -p checkers-spec-gen -- --emit-registry`."
        );
    }

    /// Every registered law actually passes. `tests/laws.rs` covers this too,
    /// but having it here means `cargo test -p checkers-core` alone is enough to
    /// catch a broken law.
    #[test]
    fn every_law_holds() {
        if let Err(violation) = verify_all() {
            panic!("a registered law does not hold: {violation}");
        }
    }
}
