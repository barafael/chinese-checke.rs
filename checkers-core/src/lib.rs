//! Six-player Chinese Checkers: the rules, checked against their specification.
//!
//! Every normative claim is a type implementing [`law::Law`] — ID, math,
//! chapter, evidence level, and executable check in one impl block, registered
//! at link time in [`law::LAWS`]. Nothing can be documented without being
//! checked, or checked without being documented. Chapters are
//! [`spec::Chapter`] values, so a law cannot cite a section that does not
//! exist.
//!
//! The specification document (embedded below) is generated from the same
//! registry: `cargo run -p checkers-spec-gen -- specs/specification.md`.
//!
//! [`law::Evidence`] says how each claim is established: Kani proof
//! (`scripts/verify-proofs.sh`), exhaustive check, or property test. A proof
//! means check and code agree for all inputs — not that the LaTeX in
//! [`law::Law::STATEMENT`] says what the check computes. That gap needs a human.

#![doc = include_str!("../../specs/specification.md")]

pub mod audit;
pub mod geometry;
pub mod law;
pub mod laws;
pub mod position;
pub mod rng;
pub mod rules;
pub mod spec;
pub mod turn;

pub use audit::{PositionFault, audit_position};
pub use geometry::{Coord, Dir};
pub use law::{Evidence, Law, LawInfo, Violation};
pub use position::{Move, MoveKind, Player, Position};
pub use rng::Xorshift;
pub use rules::{Game, Outcome};
pub use spec::Chapter;
pub use turn::{JumpTurn, single_hop_destinations, step_destinations};
