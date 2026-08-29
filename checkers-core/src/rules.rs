//! Move generation and turn sequencing (chapters 9–15).

use std::collections::HashSet;

use crate::geometry::{Coord, Dir, on_board};
use crate::position::{Move, PLAYERS, Player, Position, is_legal_step};

/// Destinations reachable from `origin` by one or more jumps (chapter 9).
///
/// Exact and terminating. Within a turn only the moving piece moves and nothing
/// is captured, so the occupancy of every other hole — the set $\Omega$ — is
/// fixed. A jump out of a hole therefore depends only on that hole, and the
/// reachable set is the forward closure of a graph fixed once per turn. A single
/// visited set over **positions** suffices.
///
/// The origin is excluded from the result: a turn ending where it began is
/// indistinguishable from not moving.
pub fn jump_destinations(pos: &Position, origin: Coord) -> HashSet<Coord> {
    let omega: HashSet<Coord> = pos.occupied_except(origin).into_iter().collect();

    let mut visited = HashSet::from([origin]);
    let mut frontier = vec![origin];
    let mut reachable = HashSet::new();

    while !frontier.is_empty() {
        let mut next = Vec::new();
        for cur in frontier {
            for d in Dir::ALL {
                let mid = cur.neighbour(d);
                let dest = cur.jump_dest(d);
                if on_board(mid)
                    && on_board(dest)
                    && omega.contains(&mid)
                    && !omega.contains(&dest)
                    && visited.insert(dest)
                {
                    reachable.insert(dest);
                    next.push(dest);
                }
            }
        }
        frontier = next;
    }

    // The origin is seeded into `visited`, so the search can never offer it as a
    // destination: a turn ending where it began is indistinguishable from not
    // moving. No explicit removal is needed, and `CC-JUMP-REVISIT` checks it.
    debug_assert!(!reachable.contains(&origin));
    reachable
}

/// Enumerate jump *routes*, with a simple-path guard (chapter 9).
///
/// The guard is necessary, not cosmetic: a piece can hop out over a blocker and
/// straight back, so the space of routes is infinite. Forbidding repeats within
/// the current path bounds every route's length by the number of holes. This is
/// a presentational restriction and does not change the destination set.
pub fn jump_routes(pos: &Position, origin: Coord, max_hops: usize) -> Vec<Vec<Coord>> {
    let omega: HashSet<Coord> = pos.occupied_except(origin).into_iter().collect();
    let mut out = Vec::new();
    let mut path = vec![origin];
    walk_routes(&omega, origin, &mut path, max_hops, &mut out);
    out
}

fn walk_routes(
    omega: &HashSet<Coord>,
    cur: Coord,
    path: &mut Vec<Coord>,
    max_hops: usize,
    out: &mut Vec<Vec<Coord>>,
) {
    if path.len() > max_hops {
        return;
    }
    for d in Dir::ALL {
        let mid = cur.neighbour(d);
        let dest = cur.jump_dest(d);
        if on_board(mid)
            && on_board(dest)
            && omega.contains(&mid)
            && !omega.contains(&dest)
            && !path.contains(&dest)
        {
            path.push(dest);
            out.push(path.clone());
            walk_routes(omega, dest, path, max_hops, out);
            path.pop();
        }
    }
}

/// All legal moves for `player` (chapter 10).
///
/// One jump move per reachable destination, since distinct routes to the same
/// hole are the same move.
pub fn legal_moves(pos: &Position, player: Player) -> Vec<Move> {
    let mut moves = Vec::new();

    for origin in pos.pieces_of(player) {
        for d in Dir::ALL {
            let to = origin.neighbour(d);
            if on_board(to) && is_legal_step(pos, player, origin, to) {
                moves.push(Move::step(origin, to));
            }
        }
        for dest in jump_destinations(pos, origin) {
            moves.push(Move::jump(origin, dest));
        }
    }

    moves
}

/// Apply a move: vacate the origin, occupy the destination (chapter 11).
///
/// No intermediate hole is touched and nothing is captured.
pub fn apply(pos: &Position, mv: &Move) -> Position {
    let player = pos
        .occupant(mv.origin)
        .expect("move origin must hold a piece");
    let mut next = pos.clone();
    next.set(mv.origin, None);
    next.set(mv.destination, Some(player));
    next
}

/// Replay a move's route hole-by-hole. Used to check that route application and
/// the net effect agree (chapter 11).
pub fn apply_route(pos: &Position, mv: &Move) -> Position {
    let Some(route) = &mv.route else {
        return apply(pos, mv);
    };
    let player = pos
        .occupant(mv.origin)
        .expect("move origin must hold a piece");

    let mut next = pos.clone();
    let mut cur = mv.origin;
    for &hop in route.iter().skip(1) {
        next.set(cur, None);
        next.set(hop, Some(player));
        cur = hop;
    }
    next
}

/// How a game ended (chapter 12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Winner(Player),
    /// All six players passed in succession, so the position is frozen.
    Draw,
}

/// A game in progress.
///
/// The active player is explicit state rather than a turn counter modulo six,
/// because passing advances the player without consuming a turn in that sense
/// (chapter 12).
#[derive(Debug, Clone)]
pub struct Game {
    position: Position,
    turn: Player,
    outcome: Option<Outcome>,
    consecutive_passes: u32,
}

impl Game {
    pub fn new() -> Self {
        Self {
            position: Position::initial(),
            turn: Player::ALL[0],
            outcome: None,
            consecutive_passes: 0,
        }
    }

    pub fn from_position(position: Position, turn: Player) -> Self {
        Self {
            position,
            turn,
            outcome: None,
            consecutive_passes: 0,
        }
    }

    pub fn position(&self) -> &Position {
        &self.position
    }

    pub fn turn(&self) -> Player {
        self.turn
    }

    pub fn outcome(&self) -> Option<Outcome> {
        self.outcome
    }

    pub fn is_over(&self) -> bool {
        self.outcome.is_some()
    }

    pub fn legal_moves(&self) -> Vec<Move> {
        legal_moves(&self.position, self.turn)
    }

    /// Play one move for the active player (chapter 12).
    pub fn play(&mut self, mv: &Move) {
        assert!(!self.is_over(), "the game is already over");
        debug_assert!(
            self.legal_moves().contains(mv),
            "illegal move {mv:?} for player {:?}",
            self.turn
        );

        self.position = apply(&self.position, mv);
        self.consecutive_passes = 0;

        if self.position.has_won(self.turn) {
            self.outcome = Some(Outcome::Winner(self.turn));
            return;
        }
        self.turn = self.turn.next();
    }

    /// The active player has no legal move and forfeits the turn (chapter 12).
    ///
    /// This case is reachable, not impossible: a player whose ten pieces fill a
    /// camp can be sealed in.
    pub fn pass(&mut self) {
        assert!(!self.is_over(), "the game is already over");
        assert!(
            self.legal_moves().is_empty(),
            "player {:?} has legal moves and may not pass",
            self.turn
        );

        self.consecutive_passes += 1;
        if self.consecutive_passes as usize == PLAYERS {
            self.outcome = Some(Outcome::Draw);
            return;
        }
        self.turn = self.turn.next();
    }

    /// Advance until the game ends or `max_plies` is exhausted.
    pub fn run<F>(&mut self, max_plies: usize, mut choose: F) -> Option<Outcome>
    where
        F: FnMut(&Position, Player, &[Move]) -> usize,
    {
        for _ in 0..max_plies {
            if self.is_over() {
                break;
            }
            let moves = self.legal_moves();
            if moves.is_empty() {
                self.pass();
                continue;
            }
            let idx = choose(&self.position, self.turn, &moves);
            let mv = moves[idx].clone();
            self.play(&mv);
        }
        self.outcome
    }
}

impl Default for Game {
    fn default() -> Self {
        Self::new()
    }
}

/// A position where player 0 holds all ten pieces yet has no legal move.
///
/// Demonstrates that "the active player always has a move" is false, so turn
/// handling must cope with passing (chapter 12).
pub fn blocked_position() -> Position {
    let mut pos = Position::empty();
    let player = Player::ALL[0];
    let blocker = Player::ALL[1];

    let camp = player.start_camp();
    for c in &camp {
        pos.set(*c, Some(player));
    }

    // Block the camp's frontier.
    let frontier: HashSet<Coord> = camp
        .iter()
        .flat_map(|c| Dir::ALL.map(|d| c.neighbour(d)))
        .filter(|c| on_board(*c) && !camp.contains(c))
        .collect();
    for c in &frontier {
        pos.set(*c, Some(blocker));
    }

    // Block every hole a camp piece could land on.
    let landings: HashSet<Coord> = camp
        .iter()
        .flat_map(|c| Dir::ALL.map(|d| (c.neighbour(d), c.jump_dest(d))))
        .filter(|(mid, dest)| on_board(*mid) && on_board(*dest) && !pos.is_empty_hole(*mid))
        .map(|(_, dest)| dest)
        .collect();
    for c in landings {
        if pos.is_empty_hole(c) {
            pos.set(c, Some(blocker));
        }
    }

    pos
}

/// A fully packed board: nobody can move at all.
pub fn frozen_position() -> Position {
    let mut pos = Position::empty();
    for (i, c) in pos.holes().to_vec().into_iter().enumerate() {
        pos.set(c, Some(Player::wrapping((i % PLAYERS) as u8)));
    }
    pos
}

/// A piece with two blockers in line, so single hops chain twice.
///
/// Exported next to [`blocked_position`] and [`frozen_position`] so tests in
/// other crates can share one definition of this fixture rather than each
/// rebuilding it.
pub fn two_hop_position() -> (Position, Coord) {
    let mut pos = Position::empty();
    let origin = Coord::new(0, 0);
    pos.set(origin, Some(Player::ALL[0]));
    pos.set(Coord::new(1, 0), Some(Player::ALL[1]));
    pos.set(Coord::new(3, 0), Some(Player::ALL[1]));
    (pos, origin)
}
