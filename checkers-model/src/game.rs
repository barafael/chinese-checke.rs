//! Turn sequencing, including the pass/draw rule (Impl. Spec. §§22, 24-25).

use crate::board::{Board, NUM_PLAYERS, Player};
use crate::moves::{Move, legal_moves};
use crate::state::State;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Winner(Player),
    /// All six players passed in succession: the position is frozen (§25).
    Draw,
}

#[derive(Debug, Clone)]
pub struct Game {
    state: State,
    turn: Player,
    outcome: Option<Outcome>,
    consecutive_passes: u32,
}

impl Game {
    pub fn new() -> Self {
        Self {
            state: State::initial(Board::new()),
            turn: 0,
            outcome: None,
            consecutive_passes: 0,
        }
    }

    pub fn from_state(state: State, turn: Player) -> Self {
        Self {
            state,
            turn,
            outcome: None,
            consecutive_passes: 0,
        }
    }

    pub fn state(&self) -> &State {
        &self.state
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
        legal_moves(&self.state, self.turn)
    }

    /// Advance the active player. Note this is tracked as explicit state rather
    /// than derived from a turn counter, because passing breaks `t mod 6`
    /// (Formal Rules §12, §17).
    fn advance(&mut self) {
        self.turn = (self.turn + 1) % NUM_PLAYERS as Player;
    }

    /// Play one move for the active player (§24).
    pub fn play(&mut self, mv: &Move) {
        assert!(!self.is_over(), "game is already over");
        debug_assert!(
            self.legal_moves().contains(mv),
            "illegal move {mv:?} for player {}",
            self.turn
        );

        self.state = self.state.apply(mv);
        self.consecutive_passes = 0;

        if self.state.has_won(self.turn) {
            self.outcome = Some(Outcome::Winner(self.turn));
            return;
        }
        self.advance();
    }

    /// The active player has no legal move and forfeits the turn (§25).
    ///
    /// This case is reachable, not impossible: a player whose ten pieces fill a
    /// camp can be sealed in. An unconditional `assert !moves.is_empty()` would
    /// be unsound.
    pub fn pass(&mut self) {
        assert!(!self.is_over(), "game is already over");
        assert!(
            self.legal_moves().is_empty(),
            "player {} has legal moves and may not pass",
            self.turn
        );

        self.consecutive_passes += 1;
        if self.consecutive_passes as usize == NUM_PLAYERS {
            self.outcome = Some(Outcome::Draw);
            return;
        }
        self.advance();
    }

    /// Drive the game with a move-choosing closure until it ends or `max_plies`
    /// is exhausted. Returns the outcome if one was reached.
    pub fn run<F>(&mut self, max_plies: usize, mut choose: F) -> Option<Outcome>
    where
        F: FnMut(&State, Player, &[Move]) -> usize,
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
            let idx = choose(&self.state, self.turn, &moves);
            let mv = moves[idx].clone();
            self.play(&mv);
            self.state.validate();
        }
        self.outcome
    }
}

impl Default for Game {
    fn default() -> Self {
        Self::new()
    }
}

/// Construct a position where player 0 holds all ten pieces but has no legal
/// move: the camp frontier and every jump landing behind it are blocked.
///
/// Demonstrates that the draft spec's `assert size(moves) > 0` was unsound.
pub fn blocked_position() -> State {
    use crate::coord::Dir;
    use std::collections::HashSet;

    let board = Board::new();
    let mut state = State::empty(board.clone());

    let camp: Vec<_> = board.camp(0).iter().copied().collect();
    for c in &camp {
        state.set(*c, Some(0));
    }

    // Block the camp's frontier holes.
    let frontier: HashSet<_> = camp
        .iter()
        .flat_map(|c| Dir::ALL.map(|d| c.add(d)))
        .filter(|c| board.contains(*c) && !board.camp(0).contains(c))
        .collect();
    for c in &frontier {
        state.set(*c, Some(1));
    }

    // Block every hole a camp piece could land on by jumping.
    let landings: HashSet<_> = camp
        .iter()
        .flat_map(|c| Dir::ALL.map(|d| (c.add(d), c.jump_dest(d))))
        .filter(|(mid, dest)| {
            board.contains(*mid) && board.contains(*dest) && !state.is_empty(*mid)
        })
        .map(|(_, dest)| dest)
        .collect();
    for c in landings {
        if state.is_empty(c) {
            state.set(c, Some(1));
        }
    }

    state
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turn_order_cycles() {
        let mut game = Game::new();
        for expected in [0, 1, 2, 3, 4, 5, 0, 1] {
            assert_eq!(game.turn(), expected);
            let mv = game.legal_moves()[0].clone();
            game.play(&mv);
        }
    }

    /// The draft spec asserted this could not happen.
    #[test]
    fn blocked_player_has_no_moves_and_has_not_won() {
        let state = blocked_position();
        assert!(legal_moves(&state, 0).is_empty(), "player 0 must be stuck");
        assert_eq!(state.pieces_of(0).len(), 10, "while holding all ten pieces");
        assert!(!state.has_won(0), "and this is neither a win ...");
        for p in 1..6 {
            assert!(!state.has_won(p), "... nor a win for anyone else");
        }
    }

    #[test]
    fn stuck_player_passes_and_play_continues() {
        let mut game = Game::from_state(blocked_position(), 0);
        assert!(game.legal_moves().is_empty());
        game.pass();
        assert_eq!(game.turn(), 1, "turn advanced past the stuck player");
        assert!(!game.is_over());
    }

    #[test]
    fn six_consecutive_passes_is_a_draw() {
        // Fully packed board: no empty hole to step into or land on, so every
        // player is frozen. (Not a legal game position — piece counts are wrong
        // — but it is the cleanest way to exercise the draw path.)
        let board = Board::new();
        let mut state = State::empty(board.clone());
        for (i, c) in board.holes().enumerate() {
            state.set(c, Some((i % 6) as u8));
        }
        for p in 0..6 {
            assert!(
                legal_moves(&state, p).is_empty(),
                "player {p} should be frozen"
            );
        }

        let mut game = Game::from_state(state, 0);
        for _ in 0..6 {
            assert!(!game.is_over());
            game.pass();
        }
        assert_eq!(game.outcome(), Some(Outcome::Draw));
    }

    #[test]
    #[should_panic(expected = "may not pass")]
    fn cannot_pass_with_legal_moves_available() {
        Game::new().pass();
    }

    #[test]
    fn random_play_preserves_invariants() {
        let mut game = Game::new();
        let mut rng = crate::prng::Prng::new(2024);
        game.run(500, |_, _, moves| rng.below(moves.len() as u32) as usize);
        game.state().validate();
    }
}
