//! Move representation and generation (Impl. Spec. §§11-12, 16, 19-20).

use std::collections::HashSet;

use crate::board::Player;
use crate::coord::{Coord, Dir};
use crate::state::State;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MoveKind {
    Step,
    Jump,
}

/// A move is identified by `(kind, origin, destination)` — see §19.
///
/// `route` is presentational only (animation, notation) and is deliberately
/// excluded from `PartialEq`/`Hash`: by §21 the resulting position depends only
/// on origin and destination, so two routes to the same hole are one move.
#[derive(Debug, Clone)]
pub struct Move {
    pub kind: MoveKind,
    pub origin: Coord,
    pub destination: Coord,
    pub route: Option<Vec<Coord>>,
}

impl Move {
    pub fn step(origin: Coord, destination: Coord) -> Self {
        Self {
            kind: MoveKind::Step,
            origin,
            destination,
            route: None,
        }
    }

    pub fn jump(origin: Coord, destination: Coord) -> Self {
        Self {
            kind: MoveKind::Jump,
            origin,
            destination,
            route: None,
        }
    }

    pub fn with_route(mut self, route: Vec<Coord>) -> Self {
        self.route = Some(route);
        self
    }

    fn key(&self) -> (MoveKind, Coord, Coord) {
        (self.kind, self.origin, self.destination)
    }
}

impl PartialEq for Move {
    fn eq(&self, other: &Self) -> bool {
        self.key() == other.key()
    }
}

impl Eq for Move {}

impl std::hash::Hash for Move {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.key().hash(state);
    }
}

/// Is `origin -> origin + d` a legal adjacent step for `player`? (§11)
pub fn is_legal_step(state: &State, player: Player, origin: Coord, to: Coord) -> bool {
    state.owner(origin) == Some(player)
        && state.board().contains(to)
        && Dir::ALL.iter().any(|d| origin.add(*d) == to)
        && state.is_empty(to)
}

/// Destinations reachable by one or more jumps (§16).
///
/// Exact and terminating: a turn moves one piece and never captures, so the
/// occupancy of every *other* hole is fixed for the whole turn. Jump legality
/// out of a hole therefore depends only on that hole, and the reachable set is
/// the forward closure of a graph fixed once per turn. A single visited set over
/// **positions** suffices — keying on `(state, position)` is unnecessary and
/// does not terminate (§17, §18).
pub fn jump_destinations(state: &State, origin: Coord) -> HashSet<Coord> {
    let board = state.board();

    // Ω: occupied holes excluding the moving piece.
    let others: HashSet<Coord> = state.occupied().filter(|&c| c != origin).collect();

    let mut visited = HashSet::from([origin]);
    let mut frontier = vec![origin];
    let mut reachable = HashSet::new();

    while !frontier.is_empty() {
        let mut next = Vec::new();
        for cur in frontier {
            for d in Dir::ALL {
                let mid = cur.add(d);
                let dest = cur.jump_dest(d);
                if board.contains(mid)
                    && board.contains(dest)
                    && others.contains(&mid)
                    && !others.contains(&dest)
                    && visited.insert(dest)
                {
                    reachable.insert(dest);
                    next.push(dest);
                }
            }
        }
        frontier = next;
    }

    reachable
}

/// Enumerate jump *routes* (§18). Needs an explicit guard: the space of legal
/// paths is infinite because a piece may jump out and back. The simple-path
/// guard below is a presentational restriction and does not change the
/// destination set computed by [`jump_destinations`].
pub fn jump_routes(state: &State, origin: Coord, max_len: usize) -> Vec<Vec<Coord>> {
    let board = state.board();
    let others: HashSet<Coord> = state.occupied().filter(|&c| c != origin).collect();
    let mut out = Vec::new();

    fn walk(
        board: &crate::board::Board,
        others: &HashSet<Coord>,
        cur: Coord,
        path: &mut Vec<Coord>,
        max_len: usize,
        out: &mut Vec<Vec<Coord>>,
    ) {
        // path includes the origin, so its jump count is len() - 1
        if path.len() > max_len {
            return;
        }
        for d in Dir::ALL {
            let mid = cur.add(d);
            let dest = cur.jump_dest(d);
            if board.contains(mid)
                && board.contains(dest)
                && others.contains(&mid)
                && !others.contains(&dest)
                && !path.contains(&dest)
            {
                path.push(dest);
                out.push(path.clone());
                walk(board, others, dest, path, max_len, out);
                path.pop();
            }
        }
    }

    let mut path = vec![origin];
    walk(board, &others, origin, &mut path, max_len, &mut out);
    out
}

/// All legal moves for `player` (§20). One jump move per reachable destination.
pub fn legal_moves(state: &State, player: Player) -> Vec<Move> {
    let mut moves = Vec::new();

    for origin in state.pieces_of(player) {
        for d in Dir::ALL {
            let to = origin.add(d);
            if is_legal_step(state, player, origin, to) {
                moves.push(Move::step(origin, to));
            }
        }
        for dest in jump_destinations(state, origin) {
            moves.push(Move::jump(origin, dest));
        }
    }

    moves
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Board;
    use std::collections::HashSet;

    #[test]
    fn initial_mobility_is_uniform() {
        // Follows from six-fold symmetry; a malformed camp breaks it (§6).
        let state = State::initial(Board::new());
        for p in 0..6 {
            assert_eq!(legal_moves(&state, p).len(), 14, "player {p}");
        }
    }

    #[test]
    fn move_generation_has_no_duplicates() {
        let state = State::initial(Board::new());
        for p in 0..6 {
            let moves = legal_moves(&state, p);
            let unique: HashSet<&Move> = moves.iter().collect();
            assert_eq!(unique.len(), moves.len(), "duplicate moves for player {p}");
        }
    }

    #[test]
    fn route_is_excluded_from_move_identity() {
        let (a, b) = (Coord::new(0, 0), Coord::new(4, 0));
        let m1 = Move::jump(a, b).with_route(vec![a, Coord::new(2, 0), b]);
        let m2 = Move::jump(a, b).with_route(vec![a, Coord::new(2, -2), b]);
        assert_eq!(m1, m2, "same origin+destination is the same move");
    }

    #[test]
    fn initial_mobility_splits_into_eight_steps_and_six_jumps() {
        // The camp's back row can already jump over its own front row into the
        // hexagon, so jumps exist from move one.
        let state = State::initial(Board::new());
        for p in 0..6 {
            let moves = legal_moves(&state, p);
            let steps = moves.iter().filter(|m| m.kind == MoveKind::Step).count();
            let jumps = moves.iter().filter(|m| m.kind == MoveKind::Jump).count();
            assert_eq!((steps, jumps), (8, 6), "player {p}");
        }
    }

    #[test]
    fn initial_jumps_all_land_in_the_hexagon() {
        let state = State::initial(Board::new());
        for p in 0..6 {
            for origin in state.pieces_of(p) {
                for dest in jump_destinations(&state, origin) {
                    assert!(
                        state.board().hex().contains(&dest),
                        "{dest:?} should be a hexagon hole"
                    );
                }
            }
        }
    }

    #[test]
    fn jump_routes_may_revisit_the_origin() {
        // §18: piece jumps out over a blocker and straight back.
        let board = Board::new();
        let mut state = State::empty(board);
        let origin = Coord::new(0, 0);
        state.set(origin, Some(0));
        state.set(Coord::new(1, 0), Some(1));

        let routes = jump_routes(&state, origin, 4);
        assert!(
            routes
                .iter()
                .any(|r| r.len() >= 2 && r[1] == Coord::new(2, 0)),
            "should be able to jump the blocker"
        );
        // The destination set never contains the origin itself.
        assert!(!jump_destinations(&state, origin).contains(&origin));
    }

    /// The key claim of the corrected §16: BFS over positions agrees with an
    /// exhaustive path search. This is the check that showed the draft spec's
    /// `(state, position)` formulation to be unnecessary.
    #[test]
    fn bfs_agrees_with_exhaustive_path_search() {
        let board = Board::new();
        let all: Vec<Coord> = {
            let mut v: Vec<Coord> = board.holes().collect();
            v.sort();
            v
        };
        let mut rng = crate::prng::Prng::new(0xC0FFEE);

        for _ in 0..200 {
            let mut state = State::empty(board.clone());
            let n = 8 + rng.below(40) as usize;
            let mut occupied = Vec::new();
            for _ in 0..n {
                let c = all[rng.below(all.len() as u32) as usize];
                if state.is_empty(c) {
                    state.set(c, Some((occupied.len() % 6) as u8));
                    occupied.push(c);
                }
            }
            if occupied.is_empty() {
                continue;
            }
            let origin = occupied[rng.below(occupied.len() as u32) as usize];

            let bfs = jump_destinations(&state, origin);
            let dfs: HashSet<Coord> = jump_routes(&state, origin, all.len())
                .iter()
                .map(|r| *r.last().unwrap())
                .filter(|&c| c != origin)
                .collect();

            assert_eq!(bfs, dfs, "BFS and path search must agree from {origin:?}");
        }
    }
}
