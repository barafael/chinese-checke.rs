//! A search engine for the machine-checked Chinese Checkers rules.
//!
//! Bevy-independent by design: the engine speaks [`checkers_core::rules::Game`]
//! and [`checkers_core::position::Move`] and nothing else, so it can drive a
//! Bevy system, a CLI, or a test the same way. The one nod to the web is
//! `bevy_platform::time::Instant` for the search's wall-clock budget (see
//! `search`), which works on `wasm32-unknown-unknown` where `std`'s panics.
//!
//! # What the engine does
//!
//! Chinese checkers is a *race*: nothing is captured and material is
//! constant, so the whole game is how far each piece has travelled toward the
//! opposite camp. That makes progress the evaluation's core; jump chains and
//! shared ladders multiply progress per move, and emptying the home camp
//! matters because one straggler piece loses races.
//!
//! # How this engine plays
//!
//! - **Bitboards**: one `u128` per player over the 121 board holes; moves are
//!   `(origin, destination)` pairs, since a jump route never changes the
//!   resulting position (chapter 10).
//! - **Evaluation**: per-player progress toward their target apex, a home-fill
//!   bonus, and a straggler penalty on the furthest-behind piece.
//! - **Search**: iterative deepening under a time budget. Two-player games
//!   get negamax with alpha-beta pruning and a zobrist transposition table;
//!   with three or more players the tree branches too hard for sound pruning,
//!   so it plays maxⁿ over the seated players at a shallower depth.
//! - **Anti-shuffling**: the engine remembers the game's recent positions and
//!   refuses to revisit one while an alternative exists — a race engine that
//!   shuffles is a race engine that loses.

mod engine;
mod search;
mod tables;
#[cfg(test)]
mod testutil;

use checkers_core::geometry::Dir;
use checkers_core::position::{Move, MoveKind, Player};
use checkers_core::rules::Game;

use engine::State;
use std::time::Duration;

/// How the engine may spend its time. Defaults are tuned for a human-facing
/// app: strong enough to punish shuffling, quick enough not to feel laggy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiConfig {
    /// Wall-clock budget per move. The search finishes its current depth
    /// before stopping, so this is a floor on thinking time, not a ceiling.
    pub budget: Duration,
    /// Hard ply cap. A safety net for endgames where depth is cheap.
    pub max_depth: u8,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            budget: Duration::from_millis(600),
            max_depth: 24,
        }
    }
}

impl AiConfig {
    /// The five human-facing strengths. Level 3 is the default tuning; going
    /// down cuts the budget steeply so the difference is felt within a move,
    /// going up pays a few seconds for visibly deeper play. Every level must
    /// still finish real games — the budget is a floor on the current depth,
    /// and the depth caps keep cheap endgames from being cut short.
    pub fn strength(level: u8) -> Self {
        match level.clamp(1, 5) {
            1 => Self {
                budget: Duration::from_millis(60),
                max_depth: 4,
            },
            2 => Self {
                budget: Duration::from_millis(200),
                max_depth: 8,
            },
            3 => Self::default(),
            4 => Self {
                budget: Duration::from_millis(1500),
                max_depth: 24,
            },
            _ => Self {
                budget: Duration::from_millis(4000),
                max_depth: 32,
            },
        }
    }
}

#[cfg(test)]
mod strength_tests {
    use super::*;

    /// The five levels are five distinct tunings, and level 3 is exactly the
    /// default — the menu's middle choice must not silently re-tune the
    /// engine the rest of the app was balanced around.
    #[test]
    fn the_five_strengths_are_distinct_and_three_is_the_default() {
        let all: Vec<AiConfig> = (1..=5).map(AiConfig::strength).collect();
        for i in 0..all.len() {
            for j in i + 1..all.len() {
                assert_ne!(
                    all[i].budget,
                    all[j].budget,
                    "levels {} and {} share a budget",
                    i + 1,
                    j + 1
                );
            }
        }
        assert_eq!(AiConfig::strength(3), AiConfig::default());
    }

    /// Out-of-range choices clamp into the row rather than panicking: a UI
    /// that can only offer 1–5 never needs a failure path for 0 or 9.
    #[test]
    fn out_of_range_levels_clamp_into_the_row() {
        assert_eq!(AiConfig::strength(0), AiConfig::strength(1));
        assert_eq!(AiConfig::strength(9), AiConfig::strength(5));
    }
}

/// A per-round engine instance.
///
/// Per-round holds the recent-position memory: the engine penalises moves
/// that revisit a position this game has already seen, which is what stops it
/// from shuffling a piece back and forth in a won position. A restart deals a
/// fresh `Ai` rather than reusing this one, so the memory never leaks across
/// rounds.
#[derive(Debug)]
pub struct Ai {
    config: AiConfig,
    recent: Vec<u64>,
}

impl Default for Ai {
    fn default() -> Self {
        Self::new(AiConfig::default())
    }
}

impl Ai {
    pub fn new(config: AiConfig) -> Self {
        Self {
            config,
            recent: Vec::new(),
        }
    }

    fn remember(&mut self, hash: u64) {
        // Bounded: a game is a few hundred plies, and 512 is plenty of
        // shuffle memory.
        if self.recent.len() >= 512 {
            self.recent.remove(0);
        }
        self.recent.push(hash);
    }

    /// The move the engine plays for [`Game`]'s active player, or `None` when
    /// the game is over or the player must pass.
    pub fn choose_move(&mut self, game: &Game) -> Option<Move> {
        self.choose_move_for(game, game.turn())
    }

    /// Like [`Ai::choose_move`], but for a specific seat of the game — the
    /// primitive a "let the computer take this seat" UI needs. The seat must
    /// be the game's active player for the move to be playable.
    pub fn choose_move_for(&mut self, game: &Game, player: Player) -> Option<Move> {
        if game.is_over() || player != game.turn() {
            return None;
        }
        let state = State::of_game(game);
        // Remember the board both ways: with this seat to move (the direct
        // shuffle) and with the other seat to move (the two-move shuffle).
        self.remember(state.hash);
        self.remember(state.piece_hash());
        let raw = search::search(&state, &self.config, &self.recent);
        raw.map(decode)
    }
}

/// Decode an engine move back into the rules' representation. The kind is
/// derived, not carried: one hex step is a step, anything else is a jump.
fn decode(raw: u16) -> Move {
    let (from, to) = engine::unpack(raw);
    let from = tables::coord_of(from);
    let to = tables::coord_of(to);
    let kind = if Dir::ALL.iter().any(|d| from.neighbour(*d) == to) {
        MoveKind::Step
    } else {
        MoveKind::Jump
    };
    Move {
        kind,
        origin: from,
        destination: to,
        route: None,
    }
}
