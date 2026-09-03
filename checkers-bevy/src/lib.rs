//! Library half of the front-end: everything that can run without a window,
//! so the integration tests drive the real session logic headlessly. The
//! binary in `main.rs` owns the Bevy schedule and rendering.
//!
//! # Interaction
//!
//! Steps commit at once; jumps are **staged** one hop at a time and only
//! commit on confirm. Moves are queued in the session's outbox and applied
//! only when they come back **host-sequenced** ([`net`]) — solo play takes
//! the same path, so the networked code is always exercised. Confirming a
//! turn that never moved is refused (chapter 9).
//!
//! # Visualizations
//!
//! The session is board state; the visuals are a pure function of session +
//! [`BoardStyle`](board_style::BoardStyle), rebuilt wholesale on change, so
//! styles switch mid-game (`V`) without touching play. Two styles: `Classic`
//! (flat 2D) and `Amlah` ([`board_amlah`], 3D).

pub mod ai;
pub mod board_amlah;
pub mod board_style;
pub mod board_view;
pub mod lobby;
pub mod menu;
pub mod menu_bg;
pub mod move_log;
pub mod net;
pub mod setup;
pub mod web;

use bevy::prelude::*;
use checkers_core::audit::audit_position;
use checkers_core::geometry::Coord;
use checkers_core::position::{Move as GameMove, MoveKind as GameMoveKind, Player, Position};
use checkers_core::rules::{Game, jump_routes};
use checkers_core::turn::{JumpTurn, single_hop_destinations, step_destinations};
use std::time::Duration;

use crate::setup::Seating;

/// The menu first, then either the lobby (networked) or the hotseat panel
/// (one device), then the board. The board only exists in
/// [`AppState::InGame`], so every way a screen can refuse to start must say
/// so — a silent refusal would look like a blank screen.
#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AppState {
    /// Title screen: choose multiplayer or hotseat.
    #[default]
    Menu,
    /// Networked play: room, roster, readiness.
    Lobby,
    /// Offline play with every camp on this device.
    Hotseat,
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
    /// A single move (a step) has been scheduled and awaits confirmation. Holds
    /// the move plus a scratch preview so the board shows the piece at its
    /// destination before the player commits.
    Pend { mv: GameMove, preview: Position },
    /// A jump turn is under way and awaiting confirmation.
    Jumping { turn: JumpTurn },
}

/// Running totals for the game-over screen.
///
/// Counted where moves are applied ([`net::pump`]), so every peer — replaying
/// the same sequenced moves — agrees on the numbers.
#[derive(Default, Debug, Clone, Copy)]
pub struct GameStats {
    /// Committed moves per player index.
    pub moves: [u32; 6],
    /// Committed jump moves per player index.
    pub jumps: [u32; 6],
    /// Turns forfeited because the player had no legal move.
    pub passes: u32,
    /// Single hops flown per player; one jump turn can chain many.
    pub hops: [u32; 6],
    /// Hops per player that crossed a piece belonging to *another* player.
    pub hops_over_others: [u32; 6],
    /// The longest single jump turn, in hops.
    pub longest_jump: u32,
    /// Who flew [`GameStats::longest_jump`].
    pub longest_jump_by: u8,
    /// The clock reading this round started from. `None` until a frame has
    /// run, so fresh deals and tests start clean.
    pub started_at: Option<Duration>,
}

impl GameStats {
    /// Every player's committed moves.
    pub fn total_moves(&self) -> u32 {
        self.moves.iter().sum()
    }

    /// Record the clock reading this round started from — the first call
    /// wins, so a replaced session re-stamps on the next frame.
    pub fn note_started(&mut self, now: Duration) {
        self.started_at.get_or_insert(now);
    }

    /// How long the round has run, given the current clock reading.
    pub fn round_duration(&self, now: Duration) -> Option<Duration> {
        self.started_at.map(|start| now - start)
    }
}

/// A round's length in words: seconds under a minute, minutes and seconds
/// above. Bevy's clock, not the wall clock, so it works on wasm.
pub fn format_round_duration(d: Duration) -> String {
    let total = d.as_secs();
    if total < 60 {
        format!("{total}s")
    } else {
        format!("{}m {}s", total / 60, total % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_round_under_a_minute_reports_seconds_only() {
        assert_eq!(format_round_duration(Duration::from_secs(42)), "42s");
    }

    #[test]
    fn a_round_over_a_minute_reports_minutes_and_seconds() {
        assert_eq!(format_round_duration(Duration::from_secs(204)), "3m 24s");
    }

    #[test]
    fn the_round_duration_runs_from_the_first_stamp() {
        let mut stats = GameStats::default();
        assert_eq!(stats.round_duration(Duration::from_secs(5)), None);
        stats.note_started(Duration::from_secs(5));
        stats.note_started(Duration::from_secs(9));
        assert_eq!(
            stats.round_duration(Duration::from_secs(65)),
            Some(Duration::from_secs(60))
        );
    }
}

/// The game plus the UI's selection state.
#[derive(Resource)]
pub struct Session {
    pub game: Game,
    pub selection: Selection,
    pub message: String,
    /// Which player this peer may move. `None` in hotseat play, where every
    /// player is controlled locally, or when spectating a networked game.
    local_player: Option<Player>,
    /// True when this peer explicitly joined as a spectator. Distinguishes
    /// "watches by choice" from hotseat's "moves everyone", which are both
    /// `local_player: None`.
    pub spectating: bool,
    /// Moves this peer has committed but that are not yet applied. Submitted
    /// by [`net::pump`] for sequencing; solo play takes the same path.
    pub outbox: Vec<GameMove>,
    /// The seats the computer plays. Empty means every seated camp is human.
    pub ai_players: Vec<Player>,
    /// Who is seated. A partial board is audited against its own seating.
    pub seating: Seating,
    /// Running totals, shown on the game-over screen.
    pub stats: GameStats,
}

impl Default for Session {
    fn default() -> Self {
        Self::new(Seating::default())
    }
}

impl Session {
    /// A session for the given seating: every camp driven locally.
    pub fn new(seating: Seating) -> Self {
        Self {
            game: seating.game(),
            selection: Selection::None,
            message: "Click one of your pieces".into(),
            local_player: None,
            spectating: false,
            outbox: Vec::new(),
            ai_players: Vec::new(),
            seating,
            stats: GameStats::default(),
        }
    }

    /// Re-seat this session as the "watch two bots" two-player deal: opposite
    /// camps, both engines. Shared by the watched demo and the menu background
    /// so the two can never disagree on what the race is.
    pub fn deal_two(&mut self) {
        self.seating = Seating::Two;
        self.game = self.seating.game();
        self.ai_players = vec![Player::ALL[0], Player::ALL[3]];
        self.selection = Selection::None;
        self.outbox.clear();
        self.stats = GameStats::default();
    }

    /// The player this peer controls, if any.
    pub fn local_player(&self) -> Option<Player> {
        self.local_player
    }

    /// Record a committed move for the active player, then play it. The only
    /// path through which the game advances.
    pub(crate) fn commit(&mut self, mv: &GameMove) {
        let mover = self.game.turn();
        self.stats.moves[mover.index() as usize] += 1;

        if mv.kind == GameMoveKind::Jump {
            self.stats.jumps[mover.index() as usize] += 1;
            let (hops, over_others) = self.count_hops(mv, mover);
            self.stats.hops[mover.index() as usize] += hops;
            self.stats.hops_over_others[mover.index() as usize] += over_others;
            if hops > self.stats.longest_jump {
                self.stats.longest_jump = hops;
                self.stats.longest_jump_by = mover.index();
            }
        }

        self.game.play(mv);
    }

    /// Hops in a jump move, and how many crossed another player's piece.
    ///
    /// The route is presentational on the wire, so a receiving peer rebuilds
    /// one deterministically — `jump_routes` enumerates in fixed direction
    /// order — and every peer counts the same numbers. The flown route and
    /// the rebuilt one can differ; by chapter 10 the route is not part of
    /// the move.
    fn count_hops(&self, mv: &GameMove, mover: Player) -> (u32, u32) {
        let pos = self.game.position();
        let route = match &mv.route {
            Some(r) => r.clone(),
            None => jump_routes(pos, mv.origin, 64)
                .into_iter()
                .find(|r| r.last() == Some(&mv.destination))
                .unwrap_or_default(),
        };
        if route.len() < 2 {
            return (0, 0);
        }

        let mut hops = 0;
        let mut over_others = 0;
        for pair in route.windows(2) {
            // A jump is symmetric: the crossed hole is the exact midpoint.
            let mid = Coord::new((pair[0].q + pair[1].q) / 2, (pair[0].r + pair[1].r) / 2);
            hops += 1;
            if let Some(owner) = pos.occupant(mid)
                && owner != mover
            {
                over_others += 1;
            }
        }
        (hops, over_others)
    }
}

impl Session {
    /// The position to render: a staged turn's preview, else the real position.
    pub fn display_position(&self) -> &Position {
        match &self.selection {
            Selection::Jumping { turn } => turn.preview(),
            Selection::Pend { preview, .. } => preview,
            _ => self.game.position(),
        }
    }

    /// The hole the selected piece currently occupies.
    pub fn selected_hole(&self) -> Option<Coord> {
        match &self.selection {
            Selection::None => None,
            Selection::Piece { origin } => Some(*origin),
            Selection::Pend { mv, .. } => Some(mv.origin),
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
            // A staged step has no further options until it is confirmed or
            // cancelled; picking another destination first requires cancelling.
            Selection::Pend { .. } => Vec::new(),
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
            Selection::Pend { .. } => true,
            Selection::Jumping { turn } => turn.can_commit(),
            _ => false,
        }
    }

    /// May this peer act right now? Solo play (`None`) always; otherwise only
    /// on its own turn. The rules re-check on the receiving side.
    pub fn may_act(&self) -> bool {
        match self.local_player {
            None => !self.spectating,
            Some(p) => p == self.game.turn(),
        }
    }

    pub fn select(&mut self, hole: Coord) {
        if !self.may_act() {
            if self.spectating {
                self.message = "You are spectating.".into();
            } else {
                self.message = format!("Waiting for player {}", self.game.turn().index());
            }
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

            // A staged step offers no further destinations; guard above rejects
            // the click already, this arm is just for exhaustiveness.
            Selection::Pend { .. } => {}

            Selection::Piece { origin } => {
                let origin = *origin;

                // A step is staged, not played, so the player must confirm it —
                // the same guardrail as a jump. A first hop begins a staged turn.
                let step = checkers_core::rules::legal_moves(self.game.position(), player)
                    .into_iter()
                    .find(|m| {
                        m.origin == origin && m.destination == hole && m.kind == GameMoveKind::Step
                    });

                if let Some(mv) = step {
                    let dest = mv.destination;

                    // Show the piece at its destination before the player
                    // commits — same "it has moved" preview a staged jump shows.
                    let mut preview = self.game.position().clone();
                    preview.set(origin, None);
                    preview.set(dest, Some(player));

                    self.selection = Selection::Pend { mv, preview };
                    self.message = format!(
                        "Player {} steps ({},{}) -> ({},{}) - press Enter to confirm",
                        player.index(),
                        origin.q,
                        origin.r,
                        dest.q,
                        dest.r
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

    /// Commit the staged move — a single step, or a chain of hops.
    pub fn confirm(&mut self) {
        let player = self.game.turn();
        match &self.selection {
            Selection::Pend { mv, .. } => {
                let mv = mv.clone();
                let (from, to) = (mv.origin, mv.destination);
                self.outbox.push(mv);
                self.clear_selection();
                self.message = format!(
                    "Player {} stepped ({},{}) -> ({},{})",
                    player.index(),
                    from.q,
                    from.r,
                    to.q,
                    to.r
                );
            }
            Selection::Jumping { turn } => {
                let mv = match turn.to_move() {
                    Ok(mv) => mv,
                    Err(e) => {
                        // Reachable: the piece hopped back to where it began.
                        self.message = format!("Cannot confirm - {e}");
                        return;
                    }
                };

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
            _ => {}
        }
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

/// Panic if the live position violates its invariants. Six players: the
/// specification's own audit; fewer: the seating's restricted conservation
/// check (see [`setup`] for why the law is not weakened instead).
pub fn audit(position: &Position, seating: Seating) {
    if seating == Seating::Six {
        if let Err(fault) = audit_position(position, &Player::ALL) {
            panic!("specification violated while playing: {fault}");
        }
        return;
    }
    if let Err(fault) = seating.audit(position) {
        panic!("specification violated while playing: {fault}");
    }
}
