//! Naive reference implementation of star-shaped (six-player) Chinese Checkers.
//!
//! **Prototype.** This is the workspace's *differential model*: a deliberately
//! simple, obviously-correct implementation that an optimised engine can be
//! tested against. It is not tuned, not `no_std`, and uses `HashMap`/`HashSet`
//! throughout where a real engine would use a packed 121-cell array and
//! bitboards.
//!
//! Normative claims live in `checkers-core` as registered `Law` impls; cite law
//! IDs rather than section numbers.
//!
//! Notable claims pinned down by tests:
//!
//! - Camps point **outward** and meet the hexagon in 8 adjacent pairs. The
//!   inward-pointing variant also yields 121 holes, so cardinality checks alone
//!   do not catch it — see `board::tests::inward_camp_is_rejected`.
//! - Jump search is a BFS over **positions**, not over `(state, position)`
//!   pairs; within one turn occupancy is a function of position. See
//!   [`moves::jump_destinations`] and `bfs_agrees_with_exhaustive_path_search`.
//! - A player can have **zero** legal moves while holding all ten pieces, so
//!   turns must handle passing. See [`game::blocked_position`].

pub mod board;
pub mod coord;
pub mod game;
pub mod moves;
pub mod prng;
pub mod state;

pub use board::{Board, Player};
pub use coord::{Coord, Dir};
pub use game::{Game, Outcome};
pub use moves::{Move, MoveKind, legal_moves};
pub use state::State;
