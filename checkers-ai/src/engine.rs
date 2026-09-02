//! The bitboard state and its rules-shaped operations.
//!
//! One `u128` per player over 121 holes, moves as `(origin, destination)`
//! pairs packed into a `u16`, and make/unmake as three XORs. Move generation
//! mirrors the rules exactly: steps to empty adjacent holes, plus the full
//! chained-jump closure per origin (chapter 9 — occupancy is fixed within a
//! turn, so a per-origin visited set is exact).

use crate::tables::{PROGRESS_MAX, TABLES};
use checkers_core::rules::Game;

/// A move as `(origin << 7) | destination`, both hole indexes below 121.
pub type RawMove = u16;

pub fn pack(from: u8, to: u8) -> RawMove {
    ((from as u16) << 7) | to as u16
}

pub fn unpack(raw: RawMove) -> (u8, u8) {
    ((raw >> 7) as u8, (raw & 0x7f) as u8)
}

/// The search's own view of a position: who stands where, and whose turn it
/// is. Everything else is derived.
#[derive(Debug, Clone)]
pub struct State {
    pub pieces: [u128; 6],
    pub occupied: u128,
    /// Seat index of the player to move.
    pub turn: u8,
    pub hash: u64,
}

impl State {
    /// Build from the rules' game: who stands where, with the game's active
    /// player to move.
    pub fn of_game(game: &Game) -> State {
        let t = &TABLES;
        let mut pieces = [0u128; 6];
        let mut occupied = 0u128;
        let pos = game.position();
        for hole in pos.holes() {
            if let Some(player) = pos.occupant(*hole)
                && let Some(&i) = t.index.get(hole)
            {
                pieces[player.index() as usize] |= 1u128 << i;
                occupied |= 1u128 << i;
            }
        }
        let mut state = Self {
            pieces,
            occupied,
            turn: game.turn().index(),
            hash: 0,
        };
        state.hash = state.zobrist();
        state
    }

    /// The board without the turn: two states with equal piece placement are
    /// the same *race position* even if the other seat moves next. The engine
    /// refuses to re-create either kind of repeat — same-turn shuffling is a
    /// single wasted move, cross-turn shuffling is two.
    pub fn piece_hash(&self) -> u64 {
        let t = &TABLES;
        let mut h = 0u64;
        for (p, pieces) in self.pieces.iter().enumerate() {
            let mut bits = *pieces;
            while bits != 0 {
                let i = bits.trailing_zeros() as usize;
                h ^= t.zobrist_piece[p][i];
                bits &= bits - 1;
            }
        }
        h
    }

    pub fn zobrist(&self) -> u64 {
        let t = &TABLES;
        let mut h = 0u64;
        for p in 0..6usize {
            let mut bits = self.pieces[p];
            while bits != 0 {
                let i = bits.trailing_zeros() as usize;
                h ^= t.zobrist_piece[p][i];
                bits &= bits - 1;
            }
        }
        h ^= t.zobrist_turn[self.turn as usize];
        h
    }

    pub fn apply(&mut self, mv: RawMove) {
        let (from, to) = unpack(mv);
        let p = self.turn as usize;
        let (fmask, tmask) = (1u128 << from, 1u128 << to);
        self.pieces[p] ^= fmask | tmask;
        self.occupied ^= fmask;
        self.occupied |= tmask;
        let next = (self.turn + 1) % 6;
        let t = &TABLES;
        // The turn key must be swapped, not merely toggled out: the hash of a
        // state always carries the key of the seat to move. Getting this wrong
        // conflates different states in the transposition table and the search
        // plays nonsense with complete confidence.
        self.hash ^= t.zobrist_piece[p][from as usize]
            ^ t.zobrist_piece[p][to as usize]
            ^ t.zobrist_turn[p]
            ^ t.zobrist_turn[next as usize];
        self.turn = next;
    }

    /// Inverse of [`State::apply`], kept for the round-trip tests: search
    /// itself clones states, which is cheaper than faithful unmaking.
    #[allow(dead_code)]
    pub fn undo(&mut self, mv: RawMove) {
        // Unapply: wind the turn back first, then the move is the same XOR —
        // except for `occupied`, whose apply is not invertible (OR never is):
        // the destination bit is cleared and the origin restored.
        self.turn = (self.turn + 5) % 6;
        let (from, to) = unpack(mv);
        let p = self.turn as usize;
        let (fmask, tmask) = (1u128 << from, 1u128 << to);
        self.pieces[p] ^= fmask | tmask;
        self.occupied &= !tmask;
        self.occupied |= fmask;
        let next = (self.turn + 1) % 6;
        let t = &TABLES;
        self.hash ^= t.zobrist_piece[p][from as usize]
            ^ t.zobrist_piece[p][to as usize]
            ^ t.zobrist_turn[p]
            ^ t.zobrist_turn[next as usize];
    }

    /// Every move the player to move may make. Empty means the seat must pass.
    pub fn moves(&self) -> Vec<RawMove> {
        let t = &TABLES;
        let p = self.turn as usize;
        let own = self.pieces[p];
        let mut out = Vec::with_capacity(24);
        let mut origins = own;
        while origins != 0 {
            let from = origins.trailing_zeros() as u8;
            origins &= origins - 1;

            // Steps.
            for d in 0..6 {
                if let Some(n) = t.nbr[d][from as usize]
                    && self.occupied & (1u128 << n) == 0
                {
                    out.push(pack(from, n));
                }
            }

            // Chained jumps: breadth-first over landing holes. The piece's own
            // origin counts as visited, so a full circle back home is never
            // offered (chapter 9).
            let mut visited = own;
            let mut stack = vec![from];
            while let Some(cur) = stack.pop() {
                for d in 0..6 {
                    if let (Some(mid), Some(dest)) =
                        (t.nbr[d][cur as usize], t.jmp[d][cur as usize])
                        && self.occupied & (1u128 << mid) != 0
                        && self.occupied & (1u128 << dest) == 0
                        && visited & (1u128 << dest) == 0
                    {
                        visited |= 1u128 << dest;
                        out.push(pack(from, dest));
                        stack.push(dest);
                    }
                }
            }
        }
        out
    }

    /// The per-player race score: progress toward the target apex, a bonus for
    /// pieces already home, and a penalty on the furthest-behind piece. The
    /// race is lost by stragglers, so the worst piece is weighted, not summed.
    /// (The search compares seats with this; nothing else is needed.)
    pub fn eval_for(&self, player: usize) -> i32 {
        let t = &TABLES;
        let p = player;
        let mut own = self.pieces[p];
        let mut score = 0;
        let mut worst = 0i32;
        while own != 0 {
            let i = own.trailing_zeros() as usize;
            own &= own - 1;
            let d = t.dist[p][i];
            score += (PROGRESS_MAX - d) * 10;
            if t.target[p] >> i & 1 == 1 {
                score += 25;
            }
            worst = worst.max(d);
        }
        score - worst * 12
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::index_of;
    use checkers_core::Xorshift;
    use checkers_core::geometry::Coord;
    use checkers_core::position::Player;
    use checkers_core::rules::legal_moves;

    /// A state with a few pieces placed by coordinate.
    fn state_with(p0: &[Coord], p3: &[Coord], turn: u8) -> State {
        let mut s = State {
            pieces: [0; 6],
            occupied: 0,
            turn,
            hash: 0,
        };
        for (player, coords) in [(0usize, p0), (3usize, p3)] {
            for c in coords {
                let i = index_of(*c).expect("test coordinate is a board hole");
                s.pieces[player] |= 1u128 << i;
                s.occupied |= 1u128 << i;
            }
        }
        s.hash = s.zobrist();
        s
    }

    fn random_games() -> Vec<Game> {
        let mut rng = Xorshift::new(0xAB1E);
        let mut games = vec![Game::new()];
        // Composed games: two and three players.
        games.push(Game::for_players(&[Player::ALL[0], Player::ALL[3]]));
        games.push(Game::for_players(&[
            Player::ALL[0],
            Player::ALL[2],
            Player::ALL[4],
        ]));

        // And positions reached by random play, which exercise moves the
        // opening never shows.
        let mut played = Vec::new();
        for game in &mut games {
            let mut g = game.clone();
            for _ in 0..12 {
                if g.is_over() {
                    break;
                }
                let moves = g.legal_moves();
                if moves.is_empty() {
                    g.pass();
                    continue;
                }
                g.play(&moves[rng.below(moves.len())].clone());
                played.push(g.clone());
            }
        }
        played
    }

    /// The engine's move list must be exactly the rules' move list, on every
    /// position of every game shape. This is the parity that makes everything
    /// else trustworthy: search over a wrong move list is confidently wrong.
    #[test]
    fn movegen_matches_the_rules() {
        for game in random_games() {
            let player = game.turn();
            let state = State::of_game(&game);

            let mut rules: Vec<(u8, u8)> = legal_moves(game.position(), player)
                .into_iter()
                .map(|mv| {
                    (
                        index_of(mv.origin).expect("legal origin") as u8,
                        index_of(mv.destination).expect("legal destination") as u8,
                    )
                })
                .collect();
            rules.sort_unstable();
            rules.dedup();

            let mut engine: Vec<(u8, u8)> = state.moves().into_iter().map(unpack).collect();
            engine.sort_unstable();
            engine.dedup();

            assert_eq!(
                rules,
                engine,
                "movegen diverged for player {}",
                player.index()
            );
        }
    }

    /// Apply then undo must restore the state exactly — bits, turn, and hash.
    /// The search clones rather than unmakes, so this is the guarantee that
    /// cloning carries no hidden state.
    #[test]
    fn apply_and_undo_round_trip() {
        let mut rng = Xorshift::new(0x41FA);
        let mut game = Game::new();
        for _ in 0..6 {
            if game.is_over() {
                break;
            }
            let moves = game.legal_moves();
            if moves.is_empty() {
                game.pass();
                continue;
            }
            game.play(&moves[rng.below(moves.len())].clone());

            let mut state = State::of_game(&game);
            let snapshot = state.clone();
            let walk = state.moves()[rng.below(state.moves().len())];
            // A short random walk, then back out of it.
            let mut taken = Vec::new();
            state.apply(walk);
            for _ in 0..4 {
                let moves = state.moves();
                if moves.is_empty() {
                    break;
                }
                let mv = moves[rng.below(moves.len())];
                state.apply(mv);
                taken.push(mv);
            }
            for mv in taken.into_iter().rev() {
                state.undo(mv);
            }
            state.undo(walk);
            assert_eq!(state.pieces, snapshot.pieces);
            assert_eq!(state.occupied, snapshot.occupied);
            assert_eq!(state.turn, snapshot.turn);
            assert_eq!(state.hash, snapshot.hash);
        }
    }

    /// The hash must see the difference between "same pieces, other seat to
    /// move" — a search that cannot tell those apart would reuse scores for
    /// the wrong player.
    #[test]
    fn hash_distinguishes_the_turn() {
        let a = state_with(&[Coord::new(0, 0)], &[Coord::new(-2, 0)], 0);
        let mut b = a.clone();
        b.turn = 3;
        b.hash = b.zobrist();
        assert_ne!(a.hash, b.hash);
    }

    /// Progress is the heart of the evaluation: a configuration with a piece
    /// well along the race must outrank the same configuration with that piece
    /// still at home.
    #[test]
    fn advanced_pieces_outrank_home_pieces() {
        let advanced = state_with(
            &[Coord::new(-6, 2), Coord::new(5, -4), Coord::new(6, -4)],
            &[],
            0,
        );
        let backward = state_with(
            &[Coord::new(0, 0), Coord::new(5, -4), Coord::new(6, -4)],
            &[],
            0,
        );
        assert!(advanced.eval_for(0) > backward.eval_for(0));
    }

    /// The straggler rule: nine pieces nearly home plus one abandoned at the
    /// start must lose to nine nearly home plus one moderately advanced. This
    /// is the discipline the strategy literature calls decisive.
    #[test]
    fn a_straggler_hurts_more_than_a_slow_field() {
        let nearly_home: Vec<Coord> = [
            (-8, 4),
            (-7, 4),
            (-7, 3),
            (-6, 4),
            (-6, 3),
            (-6, 2),
            (-5, 4),
            (-5, 3),
            (-4, 4),
        ]
        .iter()
        .map(|&(q, r)| Coord::new(q, r))
        .collect();

        let with_straggler = state_with(
            &[nearly_home.as_slice(), &[Coord::new(8, -4)]].concat(),
            &[],
            0,
        );
        let advanced_together = state_with(
            &[nearly_home.as_slice(), &[Coord::new(-4, 0)]].concat(),
            &[],
            0,
        );
        assert!(
            advanced_together.eval_for(0) > with_straggler.eval_for(0),
            "abandoning a straggler must not pay"
        );
    }

    /// Pieces already inside the target camp score a home bonus on top of
    /// their progress.
    #[test]
    fn home_pieces_score_a_bonus() {
        let apex = state_with(&[Coord::new(-8, 4)], &[], 0);
        assert!(apex.eval_for(0) > 0);
        // The apex is distance zero and inside the camp: pure bonus plus the
        // full progress term.
        let apex_score = apex.eval_for(0);
        assert!(apex_score >= PROGRESS_MAX * 10 + 25);
    }
}

#[cfg(test)]
mod hash_tests {
    use super::*;
    use checkers_core::Xorshift;

    /// The incremental hash must always equal the canonical hash of the state
    /// it describes. When this drifted, the transposition table answered
    /// queries for states it had never seen — and the search played nonsense
    /// with complete confidence.
    #[test]
    fn incremental_hash_matches_the_canonical_hash() {
        let mut rng = Xorshift::new(0xBEEF);
        let mut game = Game::new();
        for _ in 0..8 {
            if game.is_over() {
                break;
            }
            let moves = game.legal_moves();
            if moves.is_empty() {
                game.pass();
                continue;
            }
            game.play(&moves[rng.below(moves.len())].clone());
            let state = State::of_game(&game);
            assert_eq!(
                state.hash,
                state.zobrist(),
                "a fresh state must hash canonically"
            );

            // And the incrementally updated hash must agree after a move.
            let moves = state.moves();
            if moves.is_empty() {
                continue;
            }
            let mut moved = state.clone();
            moved.apply(moves[rng.below(moves.len())]);
            assert_eq!(
                moved.hash,
                moved.zobrist(),
                "apply's incremental hash diverged from the canonical hash"
            );
        }
    }
}
