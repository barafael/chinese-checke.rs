//! Six-player (star-shaped) Chinese Checkers: the rules, and the machinery that
//! keeps them tied to their specification.
//!
//! # How the specification and the code stay in sync
//!
//! Every normative claim is a Rust type implementing [`law::Law`]. One impl block
//! carries the claim's identity, its mathematics, the chapter it belongs to, how
//! strongly it is established, the domain it quantifies over, and the executable
//! check — so those cannot drift apart. Laws register themselves in
//! [`law::LAWS`] at link time, which means:
//!
//! - the test suite runs every law without naming any, so a declared law cannot
//!   go unchecked;
//! - the specification document is generated from the same registry, so a law
//!   cannot be documented without being checked, or checked without being
//!   documented.
//!
//! Chapters are a [`spec::Chapter`] enum rather than section numbers, so a law
//! cannot cite a section that does not exist, and there is no numbering to fall
//! out of date.
//!
//! # Two views of one source
//!
//! The doc comments and law impls in this crate are the single source of truth.
//! They are rendered two ways, because neither view alone is sufficient:
//!
//! - **rustdoc** (this page, with KaTeX for the math) — the developer view: read
//!   a claim next to the code implementing it.
//! - **`checkers-spec-gen`** — the specification view: linear, ordered,
//!   single-file, diffable. rustdoc sorts items alphabetically with no stable way
//!   to override it, so it cannot present chapters in reading order.
//!
//! # Levels of evidence
//!
//! [`law::Evidence`] records how each claim is established. Geometry laws are
//! *proven* over their whole domain by the Kani harnesses in [`geometry`]; run
//! them with `scripts/verify-proofs.sh` under Linux or WSL, since Kani does not
//! build on Windows. Others are checked exhaustively over a finite domain, or
//! over generated inputs.
//!
//! Note that proof here means the check and the code agree for all inputs — not
//! that the LaTeX in [`law::Law::STATEMENT`] means what the check computes. That
//! last gap needs a human reader, which is why statement and check sit together.

pub mod audit;
pub mod geometry;
pub mod law;
pub mod laws;
pub mod position;
pub mod rules;
pub mod spec;

pub use audit::{PositionFault, audit_position};
pub use geometry::{Coord, Dir};
pub use law::{Evidence, Law, LawInfo, Violation};
pub use position::{Move, MoveKind, Player, Position};
pub use rules::{Game, Outcome};
pub use spec::Chapter;
