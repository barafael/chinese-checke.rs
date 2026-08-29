//! A jump turn in progress (chapter 9).
//!
//! [`crate::rules::Game::play`] is atomic: it takes a whole move and commits it.
//! A user interface that lets a player hop one step at a time needs something
//! finer — the ability to advance a single hop, see what is available next, and
//! abandon the whole turn without touching the game.
//!
//! [`JumpTurn`] is that intermediate state. It holds the moving piece's path and
//! nothing else; the [`Position`] it was built from is unchanged until the turn
//! is committed, so cancelling is free.
//!
//! # Why single hops need their own enumerator
//!
//! [`crate::rules::jump_destinations`] returns the *transitive closure* — every
//! hole reachable by any number of hops. Offering that as the next click would
//! let a player skip intermediate holes, so a staged interface needs
//! [`single_hop_destinations`], which returns only the immediate neighbours-over-
//! a-blocker.
//!
//! The two agree in the way chapter 9 requires: the closure is exactly what you
//! reach by iterating single hops. `CC-TURN-HOP-CLOSURE` checks that.

use crate::geometry::{Coord, Dir, on_board};
use crate::position::{Move, Player, Position, is_legal_jump};

/// Destinations reachable by **exactly one** jump from `origin`.
///
/// Unlike [`crate::rules::jump_destinations`], this does not chain. Use it to
/// drive a staged interface where the player commits one hop at a time.
pub fn single_hop_destinations(pos: &Position, origin: Coord) -> Vec<Coord> {
    let Some(player) = pos.occupant(origin) else {
        return Vec::new();
    };
    let mut out: Vec<Coord> = Dir::ALL
        .iter()
        .filter(|d| is_legal_jump(pos, player, origin, **d))
        .map(|d| origin.jump_dest(*d))
        .collect();
    out.sort();
    out
}

/// Why a staged jump turn could not be committed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitError {
    /// No hop has been taken, so there is nothing to commit. A turn must move
    /// the piece somewhere other than where it started (chapter 9).
    NoHopsTaken,
}

impl core::fmt::Display for CommitError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            CommitError::NoHopsTaken => write!(
                f,
                "a jump turn must take at least one hop; ending where it began \
                 is indistinguishable from not moving"
            ),
        }
    }
}

impl core::error::Error for CommitError {}

/// A jump turn being built up one hop at a time.
///
/// The turn owns a scratch copy of the position so that intermediate hops are
/// visible to the caller (for rendering) without committing anything.
#[derive(Debug, Clone)]
pub struct JumpTurn {
    player: Player,
    origin: Coord,
    /// Holes visited, starting with `origin`.
    path: Vec<Coord>,
    /// The position with the piece moved along `path` so far.
    scratch: Position,
}

impl JumpTurn {
    /// Begin a jump turn with the piece at `origin`.
    ///
    /// Returns `None` if `origin` does not hold one of `player`'s pieces.
    pub fn begin(pos: &Position, player: Player, origin: Coord) -> Option<Self> {
        if pos.occupant(origin) != Some(player) {
            return None;
        }
        Some(Self {
            player,
            origin,
            path: vec![origin],
            scratch: pos.clone(),
        })
    }

    pub fn player(&self) -> Player {
        self.player
    }

    pub fn origin(&self) -> Coord {
        self.origin
    }

    /// Where the piece currently sits.
    pub fn current(&self) -> Coord {
        *self.path.last().expect("the path always holds the origin")
    }

    /// Holes visited so far, starting with the origin.
    pub fn path(&self) -> &[Coord] {
        &self.path
    }

    /// Hops taken so far.
    pub fn hops(&self) -> usize {
        self.path.len() - 1
    }

    /// The position including the hops taken so far. Not committed.
    pub fn preview(&self) -> &Position {
        &self.scratch
    }

    /// Destinations available for the next single hop.
    ///
    /// Note this may include holes already visited: a piece can legitimately hop
    /// back over a blocker (chapter 9). Revisiting is allowed here because the
    /// player is choosing each hop explicitly, so there is no search to
    /// terminate.
    pub fn next_hops(&self) -> Vec<Coord> {
        single_hop_destinations(&self.scratch, self.current())
    }

    /// Can the turn end here?
    pub fn can_commit(&self) -> bool {
        self.hops() > 0 && self.current() != self.origin
    }

    /// Take one hop to `dest`.
    ///
    /// Returns `false` and changes nothing if `dest` is not a legal single hop
    /// from the current hole.
    pub fn hop(&mut self, dest: Coord) -> bool {
        if !on_board(dest) || !self.next_hops().contains(&dest) {
            return false;
        }
        let from = self.current();
        self.scratch.set(from, None);
        self.scratch.set(dest, Some(self.player));
        self.path.push(dest);
        true
    }

    /// Undo the most recent hop. Returns `false` if no hop has been taken.
    pub fn undo(&mut self) -> bool {
        if self.hops() == 0 {
            return false;
        }
        let from = self.path.pop().expect("hops() > 0");
        let back = self.current();
        self.scratch.set(from, None);
        self.scratch.set(back, Some(self.player));
        true
    }

    /// The move this turn represents, ready for [`crate::rules::Game::play`].
    ///
    /// Carries the full route for presentation, but its identity is
    /// `(kind, origin, destination)` as chapter 10 requires.
    pub fn to_move(&self) -> Result<Move, CommitError> {
        if !self.can_commit() {
            return Err(CommitError::NoHopsTaken);
        }
        Ok(Move::jump(self.origin, self.current()).with_route(self.path.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::{Game, jump_destinations};
    use std::collections::HashSet;

    fn two_hop_setup() -> (Position, Coord) {
        // Piece at the origin with blockers two apart, so two hops chain.
        let mut pos = Position::empty();
        let origin = Coord::new(0, 0);
        pos.set(origin, Some(Player::ALL[0]));
        pos.set(Coord::new(1, 0), Some(Player::ALL[1]));
        pos.set(Coord::new(3, 0), Some(Player::ALL[1]));
        (pos, origin)
    }

    #[test]
    fn single_hops_do_not_chain() {
        let (pos, origin) = two_hop_setup();
        let hops = single_hop_destinations(&pos, origin);
        assert_eq!(
            hops,
            vec![Coord::new(2, 0)],
            "only the first hop is offered"
        );

        // Whereas the closure reaches further.
        let closure = jump_destinations(&pos, origin);
        assert!(closure.contains(&Coord::new(4, 0)), "closure should chain");
        assert!(closure.len() > hops.len());
    }

    #[test]
    fn hopping_advances_and_reveals_the_next_hop() {
        let (pos, origin) = two_hop_setup();
        let mut turn = JumpTurn::begin(&pos, Player::ALL[0], origin).unwrap();

        assert_eq!(turn.hops(), 0);
        assert!(!turn.can_commit(), "no hops taken yet");

        assert!(turn.hop(Coord::new(2, 0)));
        assert_eq!(turn.current(), Coord::new(2, 0));
        assert_eq!(turn.hops(), 1);
        assert!(turn.can_commit());

        // From here the second blocker is jumpable.
        assert!(turn.next_hops().contains(&Coord::new(4, 0)));
        assert!(turn.hop(Coord::new(4, 0)));
        assert_eq!(turn.path(), &[origin, Coord::new(2, 0), Coord::new(4, 0)]);
    }

    #[test]
    fn illegal_hops_are_refused_and_change_nothing() {
        let (pos, origin) = two_hop_setup();
        let mut turn = JumpTurn::begin(&pos, Player::ALL[0], origin).unwrap();

        // A hole reachable only by chaining is not a legal single hop.
        assert!(!turn.hop(Coord::new(4, 0)));
        assert_eq!(turn.hops(), 0);
        assert_eq!(turn.current(), origin);

        // Nor is an arbitrary empty hole.
        assert!(!turn.hop(Coord::new(0, 3)));
        assert_eq!(turn.hops(), 0);
    }

    #[test]
    fn undo_restores_the_previous_hole() {
        let (pos, origin) = two_hop_setup();
        let mut turn = JumpTurn::begin(&pos, Player::ALL[0], origin).unwrap();

        assert!(!turn.undo(), "nothing to undo yet");
        turn.hop(Coord::new(2, 0));
        turn.hop(Coord::new(4, 0));
        assert_eq!(turn.hops(), 2);

        assert!(turn.undo());
        assert_eq!(turn.current(), Coord::new(2, 0));
        assert!(turn.undo());
        assert_eq!(turn.current(), origin);
        assert_eq!(turn.hops(), 0);
        assert!(!turn.undo());
    }

    #[test]
    fn committing_with_no_hops_is_refused() {
        let (pos, origin) = two_hop_setup();
        let turn = JumpTurn::begin(&pos, Player::ALL[0], origin).unwrap();
        assert_eq!(turn.to_move(), Err(CommitError::NoHopsTaken));
    }

    #[test]
    fn a_committed_turn_is_a_legal_move() {
        let mut game = Game::new();
        let player = game.turn();
        // Find a piece with a jump available in the initial position.
        let origin = game
            .position()
            .pieces_of(player)
            .into_iter()
            .find(|c| !single_hop_destinations(game.position(), *c).is_empty())
            .expect("the initial position has jumps");

        let mut turn = JumpTurn::begin(game.position(), player, origin).unwrap();
        let dest = turn.next_hops()[0];
        assert!(turn.hop(dest));

        let mv = turn.to_move().expect("one hop is committable");
        assert!(
            game.legal_moves().contains(&mv),
            "a staged turn must produce a move the rules accept"
        );
        game.play(&mv);
        assert_eq!(game.position().occupant(dest), Some(player));
    }

    /// The staged interface must not expand what is reachable: iterating single
    /// hops reaches exactly the closure of chapter 9.
    #[test]
    fn iterated_single_hops_reach_exactly_the_closure() {
        let (pos, origin) = two_hop_setup();

        // Breadth-first over single hops, mirroring what a player could do.
        let mut seen = HashSet::from([origin]);
        let mut frontier = vec![origin];
        let mut reached = HashSet::new();
        while let Some(cur) = frontier.pop() {
            // Re-walking to `cur` is unnecessary because blockers never move
            // during a turn: place the piece at `cur` and enumerate from there.
            let mut scratch = pos.clone();
            if cur != origin {
                scratch.set(origin, None);
                scratch.set(cur, Some(Player::ALL[0]));
            }
            let turn = JumpTurn::begin(&scratch, Player::ALL[0], cur).unwrap();
            for d in turn.next_hops() {
                if seen.insert(d) {
                    reached.insert(d);
                    frontier.push(d);
                }
            }
        }

        assert_eq!(
            reached,
            jump_destinations(&pos, origin),
            "staged hops must reach exactly the closure"
        );
    }

    #[test]
    fn beginning_on_an_empty_or_foreign_hole_fails() {
        let (pos, _) = two_hop_setup();
        assert!(JumpTurn::begin(&pos, Player::ALL[0], Coord::new(0, 3)).is_none());
        assert!(JumpTurn::begin(&pos, Player::ALL[0], Coord::new(1, 0)).is_none());
    }

    #[test]
    fn the_preview_shows_hops_without_committing() {
        let (pos, origin) = two_hop_setup();
        let mut turn = JumpTurn::begin(&pos, Player::ALL[0], origin).unwrap();
        turn.hop(Coord::new(2, 0));

        assert_eq!(
            turn.preview().occupant(Coord::new(2, 0)),
            Some(Player::ALL[0])
        );
        assert!(turn.preview().is_empty_hole(origin));
        // The original position is untouched.
        assert_eq!(pos.occupant(origin), Some(Player::ALL[0]));
        assert!(pos.is_empty_hole(Coord::new(2, 0)));
    }
}
