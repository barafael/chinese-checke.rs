//! The full body of normative claims, one Rust type per law.
//!
//! Laws are grouped by the part of the specification they come from. Each is
//! registered in [`crate::law::LAWS`] at link time, so `tests/laws.rs` checks
//! them all without naming any, and the spec generator documents them all
//! without a hand-maintained list.

pub mod geometry;
pub mod rules;
