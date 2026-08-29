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

/// Every law registers itself here at link time. Nothing maintains this by hand.
///
/// Iterate it to check every law ([`crate::law::verify_all`]) or to generate the
/// specification document.
#[distributed_slice]
pub static LAWS: [LawInfo];

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

/// Register a [`Law`] in the [`LAWS`] slice.
///
/// The second argument is the name of the generated static; it only needs to be
/// unique within its module.
#[macro_export]
macro_rules! register_law {
    ($law:ty, $slot:ident) => {
        #[$crate::law::reexport::distributed_slice($crate::law::LAWS)]
        static $slot: $crate::law::LawInfo = $crate::law::LawInfo {
            id: <$law as $crate::law::Law>::ID,
            statement: <$law as $crate::law::Law>::STATEMENT,
            chapter: <$law as $crate::law::Law>::CHAPTER,
            summary: <$law as $crate::law::Law>::SUMMARY,
            evidence: <$law as $crate::law::Law>::EVIDENCE,
            verify: || <$law as $crate::law::Law>::verify(),
        };
    };
}

/// Re-exports for [`crate::register_law!`]; not part of the public API.
#[doc(hidden)]
pub mod reexport {
    pub use linkme::distributed_slice;
}

/// Check every registered law. Used by `tests/laws.rs`.
pub fn verify_all() -> Result<(), Violation> {
    for law in LAWS {
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
