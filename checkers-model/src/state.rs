//! Game state and the position invariants (Impl. Spec. §§8-9, 13, 21, 28).

use std::collections::HashMap;

use crate::board::{Board, CAMP_SIZE, HOLES, NUM_PLAYERS, Player};
use crate::coord::Coord;
use crate::moves::{Move, MoveKind};

/// Occupancy function `s : V -> P ∪ {∅}` (§8).
#[derive(Debug, Clone)]
pub struct State {
    board: Board,
    occupancy: HashMap<Coord, Option<Player>>,
}

impl State {
    /// All holes empty. Useful for constructing test positions.
    pub fn empty(board: Board) -> Self {
        let occupancy = board.holes().map(|c| (c, None)).collect();
        Self { board, occupancy }
    }

    /// Initial position: player `i` fills camp `C_i` (§9).
    pub fn initial(board: Board) -> Self {
        let mut state = Self::empty(board);
        for player in 0..NUM_PLAYERS as Player {
            let camp: Vec<Coord> = state.board.camp(player).iter().copied().collect();
            for c in camp {
                state.set(c, Some(player));
            }
        }
        state
    }

    pub fn board(&self) -> &Board {
        &self.board
    }

    pub fn owner(&self, c: Coord) -> Option<Player> {
        self.occupancy.get(&c).copied().flatten()
    }

    pub fn is_empty(&self, c: Coord) -> bool {
        matches!(self.occupancy.get(&c), Some(None))
    }

    pub fn set(&mut self, c: Coord, player: Option<Player>) {
        debug_assert!(self.board.contains(c), "{c:?} is not a board hole");
        self.occupancy.insert(c, player);
    }

    pub fn occupied(&self) -> impl Iterator<Item = Coord> + '_ {
        self.occupancy.iter().filter_map(|(c, o)| o.map(|_| *c))
    }

    pub fn pieces_of(&self, player: Player) -> Vec<Coord> {
        let mut v: Vec<Coord> = self
            .occupancy
            .iter()
            .filter_map(|(c, o)| (*o == Some(player)).then_some(*c))
            .collect();
        v.sort(); // deterministic iteration order
        v
    }

    /// Apply a move. The net effect is to vacate the origin and occupy the
    /// destination; intermediate holes are never modified and nothing is
    /// captured, which is why moves dedupe by destination (§21).
    pub fn apply(&self, mv: &Move) -> Self {
        let player = self
            .owner(mv.origin)
            .expect("move origin must hold a piece");

        let mut next = self.clone();
        next.set(mv.origin, None);
        next.set(mv.destination, Some(player));
        next
    }

    /// `Won(s, i) <=> ∀x ∈ C_{(i+3) mod 6}, s(x) = i` (§23).
    pub fn has_won(&self, player: Player) -> bool {
        self.board
            .target_camp(player)
            .iter()
            .all(|&c| self.owner(c) == Some(player))
    }

    /// Position invariants of §28.
    pub fn validate(&self) {
        let mut occupied = 0;
        for player in 0..NUM_PLAYERS as Player {
            let n = self.pieces_of(player).len();
            assert_eq!(n, CAMP_SIZE, "player {player} must own 10 pieces, has {n}");
            occupied += n;
        }
        assert_eq!(occupied, 60, "exactly 60 holes occupied");
        let empty = self.board.holes().filter(|&c| self.is_empty(c)).count();
        assert_eq!(empty, 61, "exactly 61 holes empty");
        assert_eq!(self.occupancy.len(), HOLES);
    }

    /// Replay a move's recorded route hole-by-hole. Only used to check that
    /// route application agrees with the net effect (§21).
    pub fn apply_route(&self, mv: &Move) -> Self {
        let Some(route) = &mv.route else {
            return self.apply(mv);
        };
        assert_eq!(mv.kind, MoveKind::Jump, "only jumps carry routes");

        let player = self.owner(mv.origin).expect("origin must hold a piece");
        let mut next = self.clone();
        let mut current = mv.origin;
        for &hop in route.iter().skip(1) {
            next.set(current, None);
            next.set(hop, Some(player));
            current = hop;
        }
        assert_eq!(current, mv.destination, "route must end at the destination");
        next
    }

    /// Compare occupancy with another state, ignoring board identity.
    pub fn same_occupancy(&self, other: &Self) -> bool {
        self.occupancy == other.occupancy
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::moves::legal_moves;

    #[test]
    fn initial_state_is_valid() {
        State::initial(Board::new()).validate();
    }

    #[test]
    fn initial_state_fills_every_camp() {
        let state = State::initial(Board::new());
        for p in 0..6u8 {
            for &c in state.board().camp(p) {
                assert_eq!(state.owner(c), Some(p));
            }
        }
        // The hexagon starts empty.
        for &c in state.board().hex() {
            assert!(state.is_empty(c));
        }
    }

    #[test]
    fn nobody_has_won_initially() {
        let state = State::initial(Board::new());
        for p in 0..6 {
            assert!(!state.has_won(p));
        }
    }

    #[test]
    fn filling_the_target_camp_wins() {
        let board = Board::new();
        let mut state = State::empty(board);
        let target: Vec<Coord> = state.board().target_camp(0).iter().copied().collect();
        for c in target {
            state.set(c, Some(0));
        }
        assert!(state.has_won(0));
        // ... and does not win for anyone else.
        for p in 1..6 {
            assert!(!state.has_won(p));
        }
    }

    #[test]
    fn invariants_hold_across_many_moves() {
        let mut state = State::initial(Board::new());
        let mut rng = crate::prng::Prng::new(7);
        for ply in 0..300 {
            let player = (ply % 6) as u8;
            let moves = legal_moves(&state, player);
            if moves.is_empty() {
                continue;
            }
            let mv = &moves[rng.below(moves.len() as u32) as usize];
            state = state.apply(mv);
            state.validate();
        }
    }

    /// §21: applying a route hole-by-hole equals applying the net effect.
    /// This is what licenses deduplicating jump moves by destination.
    #[test]
    fn route_application_equals_net_effect() {
        use crate::moves::jump_routes;

        let mut state = State::initial(Board::new());
        let mut rng = crate::prng::Prng::new(4);
        let mut checked = 0;

        for ply in 0..80 {
            let player = (ply % 6) as u8;
            let moves = legal_moves(&state, player);
            if moves.is_empty() {
                continue;
            }
            let mv = moves[rng.below(moves.len() as u32) as usize].clone();

            if mv.kind == MoveKind::Jump {
                let routes: Vec<Vec<Coord>> = jump_routes(&state, mv.origin, 6)
                    .into_iter()
                    .filter(|r| r.last() == Some(&mv.destination))
                    .take(5)
                    .collect();
                for route in routes {
                    let via_route = state.apply_route(&mv.clone().with_route(route));
                    assert!(
                        via_route.same_occupancy(&state.apply(&mv)),
                        "route application diverged from net effect"
                    );
                    checked += 1;
                }
            }
            state = state.apply(&mv);
        }
        assert!(checked > 0, "expected to exercise at least one jump route");
    }
}
