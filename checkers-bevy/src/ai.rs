//! The paced turn driver for computer seats: bevy-independent, driven by an
//! injected clock.
//!
//! Two rules give a human eyes the game: a move happens **at most once per
//! second**, and each hop of a jump is staged through the rules' own
//! [`JumpTurn`] — one hop per second, with the preview showing exactly where
//! the piece is mid-flight. The driver decides; the Bevy system performs.

use crate::{Selection, Session};
use bevy::ecs::resource::Resource;
use checkers_ai::Ai;
use checkers_core::geometry::Coord;
use checkers_core::position::{Move, MoveKind};
use checkers_core::turn::JumpTurn;
use std::time::{Duration, Instant};

/// Minimum wall-clock spacing between two visible actions (a move, a commit,
/// or a single hop). The engine's own thinking time comes on top of this.
pub const MOVE_INTERVAL: Duration = Duration::from_secs(1);

/// What the driver wants done this frame.
#[derive(Debug, PartialEq)]
pub enum Action {
    /// Throttled, or nothing to do.
    Wait,
    /// Animate one hop of a staged jump. The preview position already shows
    /// the piece on the new hole.
    Hop(Coord),
    /// A staged jump finished: commit this move through the outbox.
    Commit(Move),
    /// A plain move (a step, or the game passed): commit through the outbox —
    /// or, for a pass, apply the rules' pass directly.
    Play(Move),
    /// The seat has no legal move: forfeit the turn.
    Pass,
    /// The demo could not reach a result — a standing shuffle the stall
    /// detector or the hard move ceiling called out. Reasons in the message.
    Abandon(String),
}

/// Pacing state for one game. Not `Session` state: the pacing of *what is
/// shown* is a driver concern, and the session is rebuilt on every deal.
#[derive(Resource)]
pub struct AiPace {
    next_allowed: Option<Instant>,
    /// Remaining hops of the staged jump, excluding the hop just taken. Empty
    /// while no jump is staged.
    route: Vec<Coord>,
    /// Set once the end-of-game line has been logged.
    pub result_logged: bool,
    /// The best progress (sum of remaining distance, negated) any single
    /// player has posted so far. A stall detector: when neither side posts a
    /// new record for a full window, the demo has drifted into a shuffle it
    /// will not resolve, and is abandoned honestly.
    best_progress: i32,
    /// Plies since the last progress record.
    plies_stalled: u32,
    /// Total plies driven, the backstop against an interminable shuffle.
    total_plies: u32,
}

/// A window (in plies) with no new progress records that ends the demo.
pub const STALL_WINDOW: u32 = 200;

/// A hard ceiling on plies: a demo that runs this long without a result is a
/// shuffle, not a race, and the log gets an honest abandonment line.
pub const MAX_MOVES: u32 = 240;

impl Default for AiPace {
    fn default() -> Self {
        let mut p = Self::new();
        // `new` initialises an un-stalled, past-the-edge sentinel.
        p.plies_stalled = 0;
        p
    }
}

impl AiPace {
    fn new() -> Self {
        Self {
            next_allowed: None,
            route: Vec::new(),
            result_logged: false,
            best_progress: i32::MIN,
            plies_stalled: 0,
            total_plies: 0,
        }
    }
    pub fn reset(&mut self) {
        self.next_allowed = None;
        self.route.clear();
        self.result_logged = false;
        self.best_progress = i32::MIN;
        self.plies_stalled = 0;
        self.total_plies = 0;
    }

    fn ready(&self, now: Instant) -> bool {
        self.next_allowed.is_none_or(|t| now >= t)
    }

    fn schedule(&mut self, now: Instant) {
        self.next_allowed = Some(now + MOVE_INTERVAL);
    }

    /// The progress metric for the stall detector: how much of the *best-placed*
    /// seat's race has been run, as the sum over its pieces of `(MAXDIST -
    /// distance_to_target_apex)`. Starts near zero and grows toward the
    /// finish. Tracking the leading side catches the shuffle: when the leader
    /// posts no new record for a whole window, the demo has stopped being a
    /// race.
    fn progress(&self, session: &Session) -> i32 {
        const MAXDIST: i32 = 16;
        let pos = session.game.position();
        let mut best = 0;
        for &p in &session.ai_players {
            let t = usize::from(p.index());
            let apex = checkers_core::geometry::rotate_n(
                checkers_core::geometry::Coord::new(8, -4),
                ((t + 3) % 6) as u32,
            );
            let sum: i32 = pos
                .pieces_of(p)
                .iter()
                .map(|c| MAXDIST - c.distance(apex))
                .sum();
            best = best.max(sum);
        }
        best
    }

    /// Count one move just emitted by [`Self::advance`] and consult the stall
    /// detector. Returns `Some(Abandon)` once the leading side has gone a
    /// whole window without a new progress record, or the game has run past a
    /// hard ceiling — a race neither side can finish. The returned `None` lets
    /// the caller emit its normal action.
    fn after_move(&mut self, session: &Session) -> Option<Action> {
        self.plies_stalled += 1;
        self.total_plies += 1;
        let p = self.progress(session);
        if p > self.best_progress {
            self.best_progress = p;
            self.plies_stalled = 0;
        }
        if self.total_plies >= MAX_MOVES {
            return Some(Action::Abandon(format!(
                "stalled: no finish in {MAX_MOVES} moves"
            )));
        }
        (self.plies_stalled >= STALL_WINDOW).then(|| {
            Action::Abandon(format!(
                "stall: the leading seat made no progress in {} plies",
                STALL_WINDOW
            ))
        })
    }

    /// Advance the demo by one frame. `now` is injected so tests control the
    /// clock; every returned action is spaced at least [`MOVE_INTERVAL`]
    /// after the previous one.
    pub fn advance(&mut self, session: &mut Session, ai: &mut Ai, now: Instant) -> Action {
        if session.game.is_over() {
            return Action::Wait;
        }
        let seat = session.game.turn();
        if !session.ai_players.contains(&seat) {
            return Action::Wait;
        }

        // 1. A staged jump in flight: commit it when the hops are done.
        if let Selection::Jumping { turn } = &mut session.selection
            && self.route.is_empty()
        {
            if !self.ready(now) {
                return Action::Wait;
            }
            let mv = turn.to_move().expect("a fully played route is committable");
            session.selection = Selection::None;
            self.schedule(now);
            return self.after_move(session).unwrap_or(Action::Commit(mv));
        }

        // 2. Mid-flight: take the next hop.
        if let Selection::Jumping { turn } = &mut session.selection {
            if !self.ready(now) {
                return Action::Wait;
            }
            let hop = self.route.remove(0);
            let taken = turn.hop(hop);
            debug_assert!(taken, "a precomputed route hop must be legal");
            self.schedule(now);
            return Action::Hop(hop);
        }

        // 3. Fresh move, throttled.
        if !self.ready(now) {
            return Action::Wait;
        }

        let Some((mv, route)) = ai.choose_move_route_for(&session.game, seat) else {
            // No legal move: forfeit the turn, at the same measured pace.
            if session.game.legal_moves().is_empty() {
                self.schedule(now);
                return self.after_move(session).unwrap_or(Action::Pass);
            }
            return Action::Wait;
        };

        match mv.kind {
            MoveKind::Step => {
                self.schedule(now);
                self.after_move(session).unwrap_or(Action::Play(mv))
            }
            MoveKind::Jump => {
                // Stage the jump and fly the first hop now; the rest follow at
                // one hop per second, then the commit lands.
                let Some(mut turn) = JumpTurn::begin(session.game.position(), seat, mv.origin)
                else {
                    return Action::Wait;
                };
                let mut route = route;
                let first = route.remove(0);
                let taken = turn.hop(first);
                debug_assert!(taken, "the route's first hop must be legal");
                self.route = route;
                session.selection = Selection::Jumping { turn };
                self.schedule(now);
                Action::Hop(first)
            }
        }
    }
}

/// Describe a move the way the log reads it.
pub fn describe(mv: &Move) -> String {
    let kind = match mv.kind {
        MoveKind::Step => "step",
        MoveKind::Jump => "jump",
    };
    let mut out = format!(
        "{kind} ({},{}) -> ({},{})",
        mv.origin.q, mv.origin.r, mv.destination.q, mv.destination.r
    );
    if let Some(route) = &mv.route
        && route.len() > 2
    {
        let via: Vec<String> = route[1..route.len() - 1]
            .iter()
            .map(|c| format!("({},{})", c.q, c.r))
            .collect();
        out.push_str(&format!(" via {}", via.join(", ")));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Session;
    use crate::setup::Seating;
    use checkers_core::position::Player;

    /// Progress is positive from the start of a two-player race and grows as
    /// pieces advance, so the stall detector has a sound baseline.
    #[test]
    fn progress_is_positive_and_grows() {
        let mut session = Session::new(Seating::Two);
        session.ai_players = vec![Player::ALL[0], Player::ALL[3]];
        let pace = AiPace::default();
        let initial = pace.progress(&session);

        // A single real move posted through the rules advances the metric.
        let moves = session.game.legal_moves();
        session.game.play(&moves[0]);
        let after = pace.progress(&session);
        assert!(initial >= 0, "progress starts non-negative");
        assert!(after >= initial, "one move cannot set progress back");
    }

    /// The hard ceiling backstops an interminable game: enough stale moves and
    /// the driver reports an honest abandonment instead of running forever.
    #[test]
    fn hard_move_ceiling_abandons_honestly() {
        let mut session = Session::new(Seating::Two);
        session.ai_players = vec![Player::ALL[0], Player::ALL[3]];
        let mut pace = AiPace {
            total_plies: MAX_MOVES,
            ..AiPace::default()
        };
        let out = pace.after_move(&session).expect("the ceiling trips");
        assert!(matches!(out, Action::Abandon(_)), "expected abandonment");
    }

    /// A player who keeps playing without making progress trips the window
    /// detector well short of the hard ceiling.
    #[test]
    fn stall_window_abandons_without_progress() {
        let mut session = Session::new(Seating::Two);
        session.ai_players = vec![Player::ALL[0], Player::ALL[3]];
        let mut pace = AiPace::default();
        // Fix progress at the initial value forever: only the window trips.
        pace.best_progress = pace.progress(&session);
        for _ in 0..STALL_WINDOW - 1 {
            assert!(
                pace.after_move(&session).is_none(),
                "before the window there is no abandonment"
            );
        }
        assert!(
            matches!(pace.after_move(&session), Some(Action::Abandon(_))),
            "the window trips on the boundary move"
        );
    }
}
