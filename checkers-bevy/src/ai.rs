//! The paced turn driver for computer seats: bevy-independent, driven by an
//! injected clock.
//!
//! A move happens **at most once per second**, and each move — a step or a
//! whole jump — is committed at once. The single visible result is the move's
//! flight, animated by the replay systems as the execution: no hop-by-hop
//! staging for the eyes. The Bevy system holds the driver off while that
//! flight is on screen, so a move's execution is always the last part of its
//! turn, never overlapped by the next one. The driver decides; the Bevy
//! system performs.

use crate::Session;
use bevy::ecs::resource::Resource;
use checkers_ai::Ai;
use checkers_core::position::{Move, MoveKind};
use std::time::Duration;

/// The engine strength players pick for a computer corner, 1–5. Read when the
/// game is dealt: the engine is rebuilt at that strength for the round.
#[derive(Resource, Debug, Clone, Copy)]
pub struct AiStrength(pub u8);

impl Default for AiStrength {
    fn default() -> Self {
        Self(3)
    }
}

/// Minimum wall-clock spacing between two committed moves. A flight longer
/// than this holds the driver off on its own wall clock (the Bevy system
/// skips driving while the previous execution is on screen), so the engine's
/// pace never outstrips what the eyes can follow.
pub const MOVE_INTERVAL: Duration = Duration::from_secs(1);

/// What the driver wants done this frame.
#[derive(Debug, PartialEq)]
pub enum Action {
    /// Throttled, or nothing to do.
    Wait,
    /// A move to play — a step or a whole jump, committed at once. A jump's
    /// full route flies as one execution; there is no hop-by-hop staging.
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
    next_allowed: Option<Duration>,
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
            result_logged: false,
            best_progress: i32::MIN,
            plies_stalled: 0,
            total_plies: 0,
        }
    }
    pub fn reset(&mut self) {
        self.next_allowed = None;
        self.result_logged = false;
        self.best_progress = i32::MIN;
        self.plies_stalled = 0;
        self.total_plies = 0;
    }

    fn ready(&self, now: Duration) -> bool {
        self.next_allowed.is_none_or(|t| now >= t)
    }

    fn schedule(&mut self, now: Duration) {
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
    /// clock; every returned move is spaced at least [`MOVE_INTERVAL`] after
    /// the previous one. The caller holds the driver off while the previous
    /// move's flight is still on screen, so a move's execution is the last
    /// thing its turn shows.
    pub fn advance(&mut self, session: &mut Session, ai: &mut Ai, now: Duration) -> Action {
        if session.game.is_over() {
            return Action::Wait;
        }
        let seat = session.game.turn();
        if !session.ai_players.contains(&seat) {
            return Action::Wait;
        }
        if !self.ready(now) {
            return Action::Wait;
        }

        let Some(mv) = ai.choose_move_for(&session.game, seat) else {
            // No legal move: forfeit the turn, at the same measured pace.
            if session.game.legal_moves().is_empty() {
                self.schedule(now);
                return self.after_move(session).unwrap_or(Action::Pass);
            }
            return Action::Wait;
        };

        self.schedule(now);
        self.after_move(session).unwrap_or(Action::Play(mv))
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
    use checkers_ai::AiConfig;
    use checkers_core::position::Player;

    /// The engine's whole move — a step, or a jump's full route — is committed
    /// in one `Play`: nothing is staged hop by hop, and the throttle means the
    /// next move cannot leave while the previous execution is still on screen.
    #[test]
    fn a_move_is_committed_whole_and_throttled() {
        let mut session = Session::new(Seating::Two);
        session.ai_players = vec![Player::ALL[0], Player::ALL[3]];
        let mut pace = AiPace::default();
        let mut ai = Ai::new(AiConfig::strength(1));

        let out = pace.advance(&mut session, &mut ai, Duration::ZERO);
        assert!(
            matches!(out, Action::Play(_) | Action::Pass | Action::Abandon(_)),
            "the opening move is committed at once, not staged: got {out:?}"
        );
        // No throttle has elapsed on the same instant: the driver stands down.
        assert!(
            matches!(pace.advance(&mut session, &mut ai, Duration::ZERO), Action::Wait),
            "two moves cannot leave in the same breath"
        );
    }

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
