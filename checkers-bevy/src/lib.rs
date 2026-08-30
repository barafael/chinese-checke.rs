//! Library half of the front-end: everything that can run without a window.
//!
//! The binary in `main.rs` wires these types into the Bevy schedule and owns
//! the rendering systems; the integration tests under `tests/` drive this
//! library headlessly. The split is what lets those tests use the *real*
//! coordinate mapping and session logic instead of a stale copy.
//!
//! # Interaction
//!
//! Steps commit immediately — there is nothing to chain. Jumps are **staged**:
//! selecting a piece shows only the destinations reachable in **one** hop, and
//! clicking one moves the piece there, keeps it selected, and reveals the next
//! single hop. The turn is not committed until the player confirms, so the whole
//! chain can be abandoned.
//!
//! Moves are never applied where they are made. They are queued in the
//! session's outbox and applied only when they come back **host-sequenced**
//! ([`net`]), which gives every peer one identical order. Solo play takes the
//! same path — the lone peer sequences for itself — so the networked code is
//! exercised even with one player.
//!
//! Confirming before any hop is refused: chapter 9 requires a jump turn to move
//! the piece, and a turn ending where it began is indistinguishable from not
//! moving. That case is reachable, since a piece can hop back over its blocker.

pub mod board_view;
pub mod lobby;
pub mod net;
pub mod setup;

use bevy::prelude::*;
use checkers_core::audit::audit_position;
use checkers_core::geometry::Coord;
use checkers_core::position::{Move as GameMove, MoveKind as GameMoveKind, Player, Position};
use checkers_core::rules::Game;
use checkers_core::turn::{JumpTurn, single_hop_destinations, step_destinations};

use crate::setup::Seating;

/// Lobby first, then the board. Networked play needs seats assigned before the
/// game is playable, and the lobby is also where the seating is chosen; a solo
/// player passes straight through with `S`.
///
/// Note that the board only exists in [`AppState::InGame`] — `spawn_board` runs
/// on entering it. A lobby that cannot be left therefore shows an empty screen,
/// which is why [`lobby::start_decision`] never makes starting conditional on
/// the network.
#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AppState {
    #[default]
    Lobby,
    InGame,
}

/// What the player is currently doing.
#[derive(Default)]
pub enum Selection {
    /// Nothing selected.
    #[default]
    None,
    /// A piece is selected but no jump has begun, so both steps and first hops
    /// are available.
    Piece { origin: Coord },
    /// A jump turn is under way and awaiting confirmation.
    Jumping { turn: JumpTurn },
}

/// The game plus the UI's selection state.
#[derive(Resource)]
pub struct Session {
    pub game: Game,
    pub selection: Selection,
    pub message: String,
    /// Which player this peer may move. `None` in solo play, where every player
    /// is controlled locally, or when spectating a full game.
    local_player: Option<Player>,
    /// Moves this peer has committed but that are not yet applied.
    ///
    /// Moves are never applied where they are made. They go here, and
    /// [`net::pump`] submits them for sequencing; the game advances only when
    /// the move comes back sequenced. In solo play the pump sequences locally in
    /// the same frame, so the delay is invisible — but the code path is the same
    /// one, which is why solo play exercises it.
    outbox: Vec<GameMove>,
    /// Who is seated. Chosen in the lobby, and needed afterwards because a
    /// partial board is audited against its own seating rather than against the
    /// six-player invariant.
    pub seating: Seating,
}

impl Default for Session {
    fn default() -> Self {
        Self::new(Seating::default())
    }
}

impl Session {
    /// A session for the given seating.
    pub fn new(seating: Seating) -> Self {
        Self {
            game: seating.game(),
            selection: Selection::None,
            message: "Click one of your pieces".into(),
            local_player: None,
            outbox: Vec::new(),
            seating,
        }
    }
}

impl Session {
    /// The position to render: a staged turn's preview, else the real position.
    pub fn display_position(&self) -> &Position {
        match &self.selection {
            Selection::Jumping { turn } => turn.preview(),
            _ => self.game.position(),
        }
    }

    /// The hole the selected piece currently occupies.
    pub fn selected_hole(&self) -> Option<Coord> {
        match &self.selection {
            Selection::None => None,
            Selection::Piece { origin } => Some(*origin),
            Selection::Jumping { turn } => Some(turn.current()),
        }
    }

    /// Holes to highlight as clickable destinations.
    ///
    /// Only ever **one** hop ahead for jumps: offering the full closure would
    /// let the player skip intermediate holes.
    pub fn highlights(&self) -> Vec<Coord> {
        match &self.selection {
            Selection::None => Vec::new(),
            Selection::Piece { origin } => {
                // One piece's own steps and first hops. Filtering `legal_moves`
                // would compute every other piece's jump closure and throw it
                // away — about 150x the work for the same answer.
                let pos = self.game.position();
                let mut out = step_destinations(pos, *origin);
                out.extend(single_hop_destinations(pos, *origin));
                out.sort();
                out.dedup();
                out
            }
            Selection::Jumping { turn } => turn.next_hops(),
        }
    }

    pub fn is_jumping(&self) -> bool {
        matches!(self.selection, Selection::Jumping { .. })
    }

    pub fn can_confirm(&self) -> bool {
        match &self.selection {
            Selection::Jumping { turn } => turn.can_commit(),
            _ => false,
        }
    }

    /// May this peer act right now?
    ///
    /// `None` means solo play, where every player is driven locally. Otherwise
    /// the peer may only move on its own turn — enforced here so an out-of-turn
    /// click never reaches the outbox, and again by the rules on the receiving
    /// side, which reject any move that is not currently legal.
    fn may_act(&self) -> bool {
        match self.local_player {
            None => true,
            Some(p) => p == self.game.turn(),
        }
    }

    pub fn select(&mut self, hole: Coord) {
        if !self.may_act() {
            self.message = format!("Waiting for player {}", self.game.turn().index());
            return;
        }
        let player = self.game.turn();
        if self.game.position().occupant(hole) != Some(player) {
            return;
        }
        self.selection = Selection::Piece { origin: hole };

        let total = self.highlights().len();
        let hops = single_hop_destinations(self.game.position(), hole).len();
        self.message = format!(
            "Player {} selected ({},{}): {total} destination(s), {hops} by jumping",
            player.index(),
            hole.q,
            hole.r
        );
    }

    pub fn clear_selection(&mut self) {
        self.selection = Selection::None;
    }

    /// Click on `hole` while something is selected.
    pub fn activate(&mut self, hole: Coord) {
        if !self.highlights().contains(&hole) {
            self.message = format!("({},{}) is not a legal destination", hole.q, hole.r);
            return;
        }
        let player = self.game.turn();

        match &mut self.selection {
            Selection::None => {}

            Selection::Piece { origin } => {
                let origin = *origin;

                // A step commits at once; a first hop begins a staged turn.
                let step = checkers_core::rules::legal_moves(self.game.position(), player)
                    .into_iter()
                    .find(|m| {
                        m.origin == origin && m.destination == hole && m.kind == GameMoveKind::Step
                    });

                if let Some(mv) = step {
                    self.outbox.push(mv);
                    self.clear_selection();
                    self.message = format!(
                        "Player {} stepped ({},{}) -> ({},{})",
                        player.index(),
                        origin.q,
                        origin.r,
                        hole.q,
                        hole.r
                    );
                    return;
                }

                let Some(mut turn) = JumpTurn::begin(self.game.position(), player, origin) else {
                    return;
                };
                if turn.hop(hole) {
                    let remaining = turn.next_hops().len();
                    self.message = format!("Hop 1 to ({},{}). {}", hole.q, hole.r, hint(remaining));
                    self.selection = Selection::Jumping { turn };
                }
            }

            Selection::Jumping { turn } => {
                if turn.hop(hole) {
                    let hops = turn.hops();
                    let remaining = turn.next_hops().len();
                    self.message =
                        format!("Hop {hops} to ({},{}). {}", hole.q, hole.r, hint(remaining));
                }
            }
        }
    }

    /// Commit the staged jump turn.
    pub fn confirm(&mut self) {
        let Selection::Jumping { turn } = &self.selection else {
            return;
        };
        let mv = match turn.to_move() {
            Ok(mv) => mv,
            Err(e) => {
                // Reachable: the piece hopped back to where it began.
                self.message = format!("Cannot confirm - {e}");
                return;
            }
        };

        let player = self.game.turn();
        let hops = turn.hops();
        let dest = mv.destination;

        self.outbox.push(mv);
        self.clear_selection();
        self.message = format!(
            "Player {} jumped {hops} hop(s) to ({},{})",
            player.index(),
            dest.q,
            dest.r
        );
    }

    /// Abandon the staged turn without touching the game.
    pub fn cancel(&mut self) {
        self.message = if self.is_jumping() {
            "Jump cancelled".into()
        } else {
            "Selection cleared".into()
        };
        self.clear_selection();
    }

    /// Undo the most recent hop, keeping the turn open.
    pub fn undo_hop(&mut self) {
        let Selection::Jumping { turn } = &mut self.selection else {
            return;
        };
        if !turn.undo() {
            return;
        }
        let hops = turn.hops();
        self.message = format!("Undid a hop ({hops} remaining)");
        if hops == 0 {
            // Back at the start: fall back to plain selection so steps are
            // offered again.
            let origin = turn.origin();
            self.selection = Selection::Piece { origin };
        }
    }
}

fn hint(remaining: usize) -> String {
    if remaining == 0 {
        "No further hops - press Enter to confirm.".into()
    } else {
        format!("{remaining} further hop(s), or press Enter to confirm.")
    }
}

/// Hold the live position to the specification's invariants.
///
/// At six players this is the specification's own audit. With fewer, the
/// six-player piece count cannot hold by construction, so the seating's
/// restricted conservation check stands in — see [`setup`] for why the law is
/// not weakened instead.
pub fn audit(position: &Position, seating: Seating) {
    if seating == Seating::Six {
        if let Err(fault) = audit_position(position) {
            panic!("specification violated while playing: {fault}");
        }
        return;
    }
    if let Err(fault) = seating.audit(position) {
        panic!("specification violated while playing: {fault}");
    }
}
