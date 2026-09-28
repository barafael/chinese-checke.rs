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

pub mod ai;
pub mod board_view;
pub mod lobby;
pub mod move_log;
pub mod net;
pub mod record;
pub mod replay;
pub mod setup;
pub mod sound;
pub mod ui;
pub mod web;

use bevy::prelude::*;
use checkers_core::audit::audit_position;
use checkers_core::geometry::Coord;
use checkers_core::position::{Move as GameMove, MoveKind as GameMoveKind, Player, Position};
use checkers_core::rules::{self, Game, Variants, jump_routes};
use checkers_core::turn::{JumpTurn, single_hop_destinations, step_destinations};
use checkers_net::WireMove;
use std::time::Duration;

use crate::move_log::coords;
use crate::record::{GameRecord, RecordFault};
use crate::setup::Seating;

/// One setup screen, then the board. The lobby is the *only* screen before a
/// game: it configures the corners (human, computer, or empty) and the room
/// (solo when no peers are present, networked otherwise), and the board only
/// exists in [`AppState::InGame`] — so every way a screen can refuse to start
/// must say so, because a silent refusal would look like a blank screen.
#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AppState {
    /// The single setup screen: hex star, corners, room, roster. Networked
    /// when peers are present, local play otherwise.
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

    /// Where one player is pinned to this peer — solo play against the
    /// computer — that player resigns even when it is not their turn:
    /// pressing the button mid-AI-move must not concede the engine's seat.
    #[test]
    fn the_pinned_player_resigns_even_off_turn() {
        let mut session = Session::new(Seating::Two);
        session.local_player = Some(Player::ALL[3]);

        // It is player 0's turn; the pinned seat is player 3.
        assert_eq!(session.game.turn(), Player::ALL[0]);

        session.resign();

        assert_eq!(
            session.game.outcome(),
            Some(rules::Outcome::Resigned(Player::ALL[3])),
            "the pinned seat gives up, not whoever happens to be on turn"
        );
    }

    /// A committed move is recorded with the path it flew: a step is the two
    /// holes it touched.
    #[test]
    fn a_step_is_recorded_with_its_two_hole_path() {
        let mut session = Session::new(Seating::Two);
        let mv = session
            .game
            .legal_moves()
            .into_iter()
            .find(|m| m.kind == GameMoveKind::Step)
            .expect("the opening position has steps");

        session.commit(&mv);

        let last = session.last_move.expect("a commit records the move");
        assert_eq!(last.mover, Player::ALL[0]);
        assert_eq!(last.path.len(), 2);
        assert_eq!(last.path[0], mv.origin);
        assert_eq!(*last.path.last().unwrap(), mv.destination);
    }

    /// A jump committed in route-free wire form still yields a concrete path:
    /// the same deterministic rebuild the stats use.
    #[test]
    fn a_route_free_jump_is_rebuilt_into_a_path() {
        let mut session = Session::new(Seating::Two);
        let mut mv = session
            .game
            .legal_moves()
            .into_iter()
            .find(|m| m.kind == GameMoveKind::Jump)
            .expect("the opening position has jumps");
        mv.route = None;

        session.commit(&mv);

        let last = session.last_move.expect("a commit records the move");
        assert!(last.path.len() >= 2, "a jump path has at least two holes");
        assert_eq!(last.path[0], mv.origin, "the path starts at the origin");
        assert_eq!(
            last.path.last(),
            Some(&mv.destination),
            "the path ends at the destination"
        );
        for pair in last.path.windows(2) {
            let mid = Coord::new((pair[0].q + pair[1].q) / 2, (pair[0].r + pair[1].r) / 2);
            assert!(
                session.game.position().occupant(mid).is_some(),
                "every hop's midpoint still holds the jumped piece (CC-JUMP-NO-CAPTURE)"
            );
        }
    }

    /// Under the "no foreign rest" rule, a staged move whose destination is a
    /// foreign triangle is refused at confirmation with a message, not silently
    /// dropped when the rules re-check it. The staging comes straight from the
    /// free `legal_moves`, so only the fence in `confirm` separates a staged
    /// move from the board.
    #[test]
    fn confirm_refuses_a_move_resting_in_a_foreign_camp() {
        let mut session = staged_foreign_camp_jump(Variants {
            forbid_foreign_camps: true,
        });
        session.confirm();
        assert!(
            session.outbox.is_empty(),
            "the foreign landing must not be sent"
        );
        assert!(
            session.message.contains("foreign"),
            "refusal must say why: {}",
            session.message
        );

        // Without the rule the same staged move flows through `confirm`.
        let mut open = staged_foreign_camp_jump(Variants::default());
        open.confirm();
        assert_eq!(open.outbox.len(), 1, "the open game still commits the jump");
    }

    /// A staged move that is not the fence's doing — the destination is illegal
    /// outright — must be refused with that reason, not the foreign camp.
    #[test]
    fn confirm_says_the_board_moved_on_for_a_stale_staging() {
        let mut session = Session::new(Seating::Two);
        let pieces = session.game.position().pieces_of(session.game.turn());
        let origin = *pieces.first().expect("the initial board offers pieces");
        // A step that stays put is never legal: no fence involved.
        session.selection = Selection::Pend {
            mv: GameMove::step(origin, origin),
            preview: session.game.position().clone(),
        };

        session.confirm();
        assert!(session.outbox.is_empty(), "the stale move must not be sent");
        assert!(
            session.message.contains("no longer legal"),
            "refusal must say the staging went stale, got: {}",
            session.message
        );
        assert!(
            !session.message.contains("foreign"),
            "a stale move is not a fence refusal: {}",
            session.message
        );
    }

    /// The hexagon/camp-1 fixture, under `variants`: player 0's piece at
    /// (0,4) in the hexagon; camp 1's hole at (1,4) holds player 3. The jump
    /// over it lands at (2,4), inside camp 1 — a triangle player 0 may pass
    /// through but never rest in — and is staged, awaiting confirmation.
    fn staged_foreign_camp_jump(variants: Variants) -> Session {
        let origin = Coord::new(0, 4);
        let mut position = Position::empty();
        position.set(origin, Some(Player::ALL[0]));
        position.set(Coord::new(1, 4), Some(Player::ALL[3]));
        let mut session = Session::new(Seating::Two);
        session.game = Game::compose(position, Player::ALL[0], &[Player::ALL[0], Player::ALL[3]])
            .with_variants(variants);
        session.selection = Selection::Pend {
            mv: GameMove::jump(origin, Coord::new(2, 4)),
            preview: session.game.position().clone(),
        };
        session
    }

    /// Only someone else's move is replay-animated. A seated peer skips its
    /// own; an unseated one — hotseat, spectator — replays everything.
    #[test]
    fn only_an_opponents_move_is_replayed() {
        let mut session = Session::new(Seating::Two);
        let mv = session.game.legal_moves()[0].clone();

        // Hotseat: no seat, so every move animates.
        session.commit(&mv);
        assert!(session.should_replay());

        // Seated as the mover: my own move is not news.
        let mut mine = Session::new(Seating::Two);
        mine.local_player = Some(Player::ALL[0]);
        mine.commit(&mv);
        assert!(!mine.should_replay());

        // Seated elsewhere: the move is an opponent's.
        let mut theirs = Session::new(Seating::Two);
        theirs.local_player = Some(Player::ALL[3]);
        theirs.commit(&mv);
        assert!(theirs.should_replay());

        // A new game clears the record.
        let mut fresh = Session::new(Seating::Two);
        fresh.commit(&mv);
        fresh = Session::new(Seating::Two);
        assert!(fresh.last_move.is_none());
        assert!(!fresh.should_replay());
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
    /// Whether the round was dealt from a shared room's roster rather than the
    /// local table. Decided once, at the deal: the peers present *now* say
    /// nothing about the round being played — a room can empty mid-game, and
    /// a solo round can follow a shared one whose seats linger in the roster.
    pub shared: bool,
    /// Moves this peer has committed but that are not yet applied. Submitted
    /// by [`net::pump`] for sequencing; solo play takes the same path.
    pub outbox: Vec<GameMove>,
    /// The seats the computer plays. Empty means every seated camp is human.
    pub ai_players: Vec<Player>,
    /// Who is seated, in turn order. A partial board is audited against its own
    /// players.
    pub players: Vec<Player>,
    /// Running totals, shown on the game-over screen.
    pub stats: GameStats,
    /// Every committed move, in play order, in the route-free wire form the
    /// record format stores. Appended by `commit` — the one point every path
    /// through the game goes through.
    history: Vec<WireMove>,
    /// The move committed most recently and the concrete path it flew, set by
    /// `commit` — the raw material for replaying the opponent's
    /// last turn as an animation, with its trace left on the board.
    pub last_move: Option<LastMove>,
}

/// A committed move and the path it took, origin first, destination last.
#[derive(Debug, Clone, PartialEq)]
pub struct LastMove {
    pub mover: Player,
    pub path: Vec<Coord>,
}

impl Default for Session {
    fn default() -> Self {
        Self::new(Seating::default())
    }
}

impl Session {
    /// A session for the given seating: every camp driven locally.
    pub fn new(seating: Seating) -> Self {
        Self::with_variants(seating, Variants::default())
    }

    /// A session for the given seating and house-rule switches.
    pub fn with_variants(seating: Seating, variants: Variants) -> Self {
        Self::for_players(&seating.players(), variants)
    }

    /// A session over an arbitrary set of players — the corners a table
    /// configured, human or engine — in index order, so turn order follows the
    /// board regardless of who set up which corner. Every provided corner is
    /// filled and driven; `ai_players` still decides which are engines.
    pub fn for_players(players: &[Player], variants: Variants) -> Self {
        let game = Game::for_players(players).with_variants(variants);
        Self {
            players: game.players().to_vec(),
            game,
            selection: Selection::None,
            message: "Click one of your pieces".into(),
            local_player: None,
            spectating: false,
            shared: false,
            outbox: Vec::new(),
            ai_players: Vec::new(),
            stats: GameStats::default(),
            history: Vec::new(),
            last_move: None,
        }
    }

    /// The round as a [`GameRecord`]: players, engine seats, the house-rule
    /// switches, and every move committed so far.
    pub fn to_record(&self) -> GameRecord {
        GameRecord {
            players: self.players.clone(),
            ai_players: self.ai_players.clone(),
            variants: self.game.variants(),
            moves: self.history.clone(),
        }
    }

    /// Every committed move so far, in play order.
    pub fn history(&self) -> &[WireMove] {
        &self.history
    }

    /// Rebuild a session from a record by replaying the moves through the
    /// rules. Each recorded move is resolved against the legal moves of the
    /// position it occurs in — the rules, not the record, decide — and the
    /// law audit runs after every one, so a forged or corrupted record is
    /// refused rather than resumed. Auto-passes are re-derived as they were
    /// the first time round.
    pub fn resumed(record: &GameRecord) -> Result<Self, RecordFault> {
        Self::resumed_prefix(record, record.moves.len())
    }

    /// Rebuild the session `up_to` plies into a record — the replay viewer's
    /// cursor. A position part-way through a round is exactly the same
    /// derivation as a full resume, just stopped early; `up_to` past the end
    /// clamps to it.
    pub fn resumed_prefix(record: &GameRecord, up_to: usize) -> Result<Self, RecordFault> {
        let mut session = Self::for_players(&record.players, record.variants);
        session.ai_players = record.ai_players.clone();
        for (ply, wire) in record.moves.iter().take(up_to).enumerate() {
            if session.game.is_over() {
                return Err(RecordFault::Replay {
                    ply,
                    why: "the game was already over".into(),
                });
            }
            let Some(mv) = session.resolve(wire) else {
                let who = session.game.turn().index();
                let kind = if wire.jump { "jump" } else { "step" };
                let ((q0, r0), (q1, r1)) = (wire.origin, wire.destination);
                return Err(RecordFault::Replay {
                    ply,
                    why: format!(
                        "player {who} cannot {kind} ({q0},{r0}) -> ({q1},{r1}) where it occurs"
                    ),
                });
            };
            session.commit(&mv);
            // Settled silently: the passes and the ending were logged when they
            // happened, and a viewer step or a resume must not tell them again.
            crate::net::settle(&mut session);
        }
        // The record was replayed for state, not for show: nothing about a
        // resumption should fire the opponent-move animation on load.
        session.last_move = None;
        Ok(session)
    }

    /// Resolve a move that arrived as a wire move — from the host or from a
    /// record — against the rules. `None` when the rules reject it, and always
    /// once the game is over: [`Game::legal_moves`] still lists the winner's
    /// moves after the winning one, and playing one would panic.
    pub fn resolve(&self, wire: &WireMove) -> Option<GameMove> {
        if self.game.is_over() {
            return None;
        }
        wire.resolve(&self.game.legal_moves())
    }

    /// The lobby name of whoever plays `p` — in a shared round only. A solo
    /// round has no roster: seats still in [`checkers_net::NetState`] are left
    /// over from an earlier shared round, and naming a local corner after them
    /// would be wrong.
    pub fn roster_name<'n>(&self, net: &'n checkers_net::NetState, p: Player) -> Option<&'n str> {
        if !self.shared {
            return None;
        }
        net.seat_at(u32::from(p.index()))
            .map(|s| s.name.as_str())
            .filter(|n| !n.is_empty())
    }

    /// The player this peer controls, if any.
    pub fn local_player(&self) -> Option<Player> {
        self.local_player
    }

    /// Record a committed move for the active player, then play it. The only
    /// path through which the game advances.
    pub(crate) fn commit(&mut self, mv: &GameMove) {
        let mover = self.game.turn();
        let i = usize::from(mover.index());
        self.stats.moves[i] += 1;
        self.history.push(WireMove::from_move(mv));

        // Resolved before the move is played: a rebuilt route enumerates from
        // the *pre-move* position.
        let path = self.fly_route(mv);
        if mv.kind == GameMoveKind::Jump {
            self.stats.jumps[i] += 1;
            let (hops, over_others) = self.count_hops(&path, mover);
            self.stats.hops[i] += hops;
            self.stats.hops_over_others[i] += over_others;
            if hops > self.stats.longest_jump {
                self.stats.longest_jump = hops;
                self.stats.longest_jump_by = mover.index();
            }
        }
        self.last_move = Some(LastMove { mover, path });

        self.game.play(mv);
    }

    /// Whether this peer wants the last committed move replayed: someone
    /// else's. A peer with no seat of its own replays every move, since whoever
    /// moves next is always watching someone else's turn begin. One's own move
    /// arriving back sequenced is not news worth animating.
    pub fn should_replay(&self) -> bool {
        let Some(last) = &self.last_move else {
            return false;
        };
        self.local_player.is_none_or(|me| last.mover != me)
    }

    /// The concrete hole-by-hole trajectory a move flies: origin, any hop
    /// landings, destination. A step touches exactly its two holes. The wire
    /// form carries no jump route — see [`Self::count_hops`] — so a jump's is
    /// rebuilt deterministically, and the stats count this same one:
    /// `jump_routes` enumerates from the pre-move position with the origin
    /// first.
    fn fly_route(&self, mv: &GameMove) -> Vec<Coord> {
        match mv.kind {
            GameMoveKind::Step => vec![mv.origin, mv.destination],
            GameMoveKind::Jump => match &mv.route {
                Some(r) => r.clone(),
                None => jump_routes(self.game.position(), mv.origin, 64)
                    .into_iter()
                    .find(|r| r.last() == Some(&mv.destination))
                    .unwrap_or_default(),
            },
        }
    }

    /// Hops along a jump's route, and how many crossed another player's piece.
    ///
    /// The route is presentational on the wire, so a receiving peer rebuilds
    /// one deterministically ([`Self::fly_route`]) — `jump_routes` enumerates
    /// in fixed direction order — and every peer counts the same numbers. The
    /// flown route and the rebuilt one can differ; by chapter 10 the route is
    /// not part of the move.
    fn count_hops(&self, route: &[Coord], mover: Player) -> (u32, u32) {
        let pos = self.game.position();
        let (mut hops, mut over_others) = (0, 0);
        for pair in route.windows(2) {
            // A jump is symmetric: the crossed hole is the exact midpoint.
            let mid = Coord::new((pair[0].q + pair[1].q) / 2, (pair[0].r + pair[1].r) / 2);
            hops += 1;
            if pos.occupant(mid).is_some_and(|owner| owner != mover) {
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
                // A step rests where it ends, so under the foreign-camp rule a
                // step into a forbidden triangle is not offered at all. A jump
                // only *passes* through its first hop, so those stay unfiltered
                // — the fence applies when the hop becomes the final destination.
                out.retain(|&hole| self.game.variants().may_rest(self.game.turn(), hole));
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
            self.message = if self.spectating {
                "You are spectating.".into()
            } else {
                format!("Waiting for player {}", self.game.turn().index())
            };
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
            "Player {} selected {}: {total} destination(s), {hops} by jumping",
            player.index(),
            coords(hole)
        );
    }

    pub fn clear_selection(&mut self) {
        self.selection = Selection::None;
    }

    /// Click on `hole` while something is selected.
    pub fn activate(&mut self, hole: Coord) {
        if !self.highlights().contains(&hole) {
            self.message = format!("{} is not a legal destination", coords(hole));
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
                let mv = GameMove::step(origin, hole);
                if rules::legal_moves(self.game.position(), player).contains(&mv) {
                    // Show the piece at its destination before the player
                    // commits — same "it has moved" preview a staged jump shows.
                    let preview = rules::apply(self.game.position(), &mv);
                    self.selection = Selection::Pend { mv, preview };
                    self.message = format!(
                        "Player {} steps {} -> {} - press Enter to confirm",
                        player.index(),
                        coords(origin),
                        coords(hole)
                    );
                    return;
                }

                let Some(mut turn) = JumpTurn::begin(self.game.position(), player, origin) else {
                    return;
                };
                if turn.hop(hole) {
                    self.message = hop_message(&turn, hole);
                    self.selection = Selection::Jumping { turn };
                }
            }

            Selection::Jumping { turn } => {
                if turn.hop(hole) {
                    self.message = hop_message(turn, hole);
                }
            }
        }
    }

    /// The human concedes the round. The seat that gives up is this peer's
    /// own where one is pinned (`local_player`); in a hotseat deal, where
    /// every seat is local, it is the seat to move. Idempotent: a finished
    /// game cannot be resigned again.
    pub fn resign(&mut self) {
        if self.game.is_over() {
            return;
        }
        let who = self.local_player.unwrap_or_else(|| self.game.turn());
        self.game.resign(who);
        self.selection = Selection::None;
        self.message = format!("Player {} resigned", who.index());
        crate::move_log::log(&format!("# p{} resigns", who.index()));
        crate::net::log_outcome(self.game.outcome().expect("resigning sets an outcome"));
    }

    /// Commit the staged move — a single step, or a chain of hops. The move is
    /// checked against the variant-filtered `legal_moves` before it is sent,
    /// so a staging that would rest in a foreign triangle (under the toggle)
    /// is refused here with a message instead of silently dropped when the
    /// rules re-check it.
    pub fn confirm(&mut self) {
        let player = self.game.turn();
        // The staged move, and what the status line says once it is sent.
        let (mv, sent) = match &self.selection {
            Selection::Pend { mv, .. } => {
                let (from, to) = (coords(mv.origin), coords(mv.destination));
                let sent = format!("Player {} stepped {from} -> {to}", player.index());
                (mv.clone(), sent)
            }
            Selection::Jumping { turn } => match turn.to_move() {
                Ok(mv) => {
                    let (hops, to) = (turn.hops(), coords(mv.destination));
                    let sent = format!("Player {} jumped {hops} hop(s) to {to}", player.index());
                    (mv, sent)
                }
                // Reachable: the piece hopped back to where it began.
                Err(e) => {
                    self.message = format!("Cannot confirm - {e}");
                    return;
                }
            },
            _ => return,
        };
        if !self.game.legal_moves().contains(&mv) {
            // The foreign-camp fence is the only filter between the raw move
            // list and the playable one, so a move the base rules still enjoy
            // was refused by the fence. Anything else — the turn or the board
            // moved on while the staging sat — is not a fence, and the message
            // must say that instead of inventing one.
            self.message = if rules::legal_moves(self.game.position(), player).contains(&mv) {
                "That would rest in a foreign triangle.".into()
            } else {
                "The staged move is no longer legal - the board moved on.".into()
            };
            return;
        }
        self.outbox.push(mv);
        self.clear_selection();
        self.message = sent;
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

/// The status line after a hop to `hole`: which hop it was, and what is left.
fn hop_message(turn: &JumpTurn, hole: Coord) -> String {
    let hint = match turn.next_hops().len() {
        0 => "No further hops - press Enter to confirm.".to_string(),
        remaining => format!("{remaining} further hop(s), or press Enter to confirm."),
    };
    format!("Hop {} to {}. {hint}", turn.hops(), coords(hole))
}

/// Panic if the live position violates its invariants. Six players: the
/// specification's own audit; fewer: conservation restricted to the seated
/// players (see [`setup`] for why the law is not weakened instead).
pub fn audit(position: &Position, players: &[Player]) {
    if players == Player::ALL.as_slice() {
        if let Err(fault) = audit_position(position, &Player::ALL) {
            panic!("specification violated while playing: {fault}");
        }
        return;
    }
    if let Err(fault) = setup::audit_players(players, position) {
        panic!("specification violated while playing: {fault}");
    }
}
