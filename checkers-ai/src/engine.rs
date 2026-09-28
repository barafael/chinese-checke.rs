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
    /// No piece may rest in a triangle that is neither its own camp nor its
    /// target camp. Fences the destinations `moves()` offers.
    pub forbid_foreign_camps: bool,
}

impl State {
    /// Build from the rules' game: who stands where, with the game's active
    /// player to move.
    pub fn of_game(game: &Game) -> State {
        let mut pieces = [0u128; 6];
        let mut occupied = 0u128;
        let pos = game.position();
        for hole in pos.holes() {
            if let Some(player) = pos.occupant(*hole)
                && let Some(&i) = TABLES.index.get(hole)
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
            forbid_foreign_camps: game.variants().forbid_foreign_camps,
        };
        state.hash = state.zobrist();
        state
    }

    /// The board without the turn: two states with equal piece placement are
    /// the same *race position* even if the other seat moves next. The engine
    /// refuses to re-create either kind of repeat — same-turn shuffling is a
    /// single wasted move, cross-turn shuffling is two.
    pub fn piece_hash(&self) -> u64 {
        let mut h = 0u64;
        for (p, pieces) in self.pieces.iter().enumerate() {
            let mut bits = *pieces;
            while bits != 0 {
                let i = bits.trailing_zeros() as usize;
                h ^= TABLES.zobrist_piece[p][i];
                bits &= bits - 1;
            }
        }
        h
    }

    /// The full key: the board, plus the seat to move.
    pub fn zobrist(&self) -> u64 {
        self.piece_hash() ^ TABLES.zobrist_turn[self.turn as usize]
    }

    pub fn apply(&mut self, mv: RawMove) {
        let (from, to) = unpack(mv);
        let p = self.turn as usize;
        let (fmask, tmask) = (1u128 << from, 1u128 << to);
        self.pieces[p] ^= fmask | tmask;
        self.occupied ^= fmask;
        self.occupied |= tmask;
        self.hash ^= TABLES.zobrist_piece[p][from as usize] ^ TABLES.zobrist_piece[p][to as usize];
        self.pass();
    }

    /// Hand the turn to the next seat without moving: a forced pass, and the
    /// second half of every move.
    pub fn pass(&mut self) {
        let next = (self.turn + 1) % 6;
        // The turn key must be swapped, not merely toggled out: the hash of a
        // state always carries the key of the seat to move. Getting this wrong
        // conflates different states in the transposition table and the search
        // plays nonsense with complete confidence.
        self.hash ^= TABLES.zobrist_turn[self.turn as usize] ^ TABLES.zobrist_turn[next as usize];
        self.turn = next;
    }

    /// The state after `mv`, leaving this one as it was.
    pub fn after(&self, mv: RawMove) -> State {
        let mut next = self.clone();
        next.apply(mv);
        next
    }

    /// Whether `player` has filled its target camp — the win.
    pub fn has_won(&self, player: usize) -> bool {
        self.pieces[player] & TABLES.target[player] == TABLES.target[player]
    }

    /// Inverse of [`State::apply`], used only by the round-trip tests: search
    /// itself clones states, which is cheaper than faithful unmaking.
    #[cfg(test)]
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
    ///
    /// Under [`self.forbid_foreign_camps`] a landing inside a foreign triangle
    /// is not offered (the rules' own filter: a move may rest only in the
    /// mover's camp or its target's). Reachability is untouched, so a chain
    /// may still pass *through* another camp to rest beyond it.
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
                    && self.may_rest(n)
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
                        if self.may_rest(dest) {
                            out.push(pack(from, dest));
                        }
                        stack.push(dest);
                    }
                }
            }
        }
        out
    }

    /// Whether a piece of the player to move may **rest** on hole `to` under
    /// this state's variant. The central hexagon is always allowed; the only
    /// holes ever flatly excluded are foreign camps with the toggle on.
    fn may_rest(&self, to: u8) -> bool {
        if !self.forbid_foreign_camps {
            return true;
        }
        let camp = TABLES.camp[to as usize];
        camp == u8::MAX || camp == self.turn || camp == self.turn.wrapping_add(3) % 6
    }

    /// The per-player race score: progress toward the target apex, a bonus for
    /// pieces already home, and a penalty on the furthest-behind piece. The
    /// race is lost by stragglers, so the worst piece is weighted, not summed.
    /// (The search compares seats with this; nothing else is needed.)
    pub fn eval_for(&self, player: usize) -> i32 {
        let t = &TABLES;
        let mut own = self.pieces[player];
        let mut score = 0;
        let mut worst = 0i32;
        while own != 0 {
            let i = own.trailing_zeros() as usize;
            own &= own - 1;
            let d = t.dist[player][i];
            score += (PROGRESS_MAX - d) * 10;
            if t.target[player] >> i & 1 == 1 {
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
    use crate::testutil::state_with;
    use checkers_core::Xorshift;
    use checkers_core::geometry::Coord;
    use checkers_core::position::{Move, Player};
    use checkers_core::rules::legal_moves;
    use std::collections::BTreeSet;

    fn random_games() -> Vec<Game> {
        let mut rng = Xorshift::new(0xAB1E);
        let games = [
            Game::new(),
            // Composed games: two and three players.
            Game::for_players(&[Player::ALL[0], Player::ALL[3]]),
            Game::for_players(&[Player::ALL[0], Player::ALL[2], Player::ALL[4]]),
        ];

        // And positions reached by random play, which exercise moves the
        // opening never shows.
        let mut played = Vec::new();
        for mut g in games {
            for _ in 0..12 {
                if g.is_over() {
                    break;
                }
                let moves = g.legal_moves();
                if moves.is_empty() {
                    g.pass();
                    continue;
                }
                g.play(&moves[rng.below(moves.len())]);
                played.push(g.clone());
            }
        }
        played
    }

    /// The engine's move list must be exactly the rules' `legal` list, as
    /// `(origin, destination)` hole pairs; `what` names the list in a failure.
    fn assert_movegen_matches(legal: &[Move], state: &State, what: &str) {
        let hole = |c| index_of(c).expect("legal hole") as u8;
        let rules: BTreeSet<(u8, u8)> = legal
            .iter()
            .map(|mv| (hole(mv.origin), hole(mv.destination)))
            .collect();
        let engine: BTreeSet<(u8, u8)> = state.moves().into_iter().map(unpack).collect();
        assert_eq!(rules, engine, "{what} diverged for player {}", state.turn);
    }

    /// The engine's move list must be exactly the rules' move list, on every
    /// position of every game shape. This is the parity that makes everything
    /// else trustworthy: search over a wrong move list is confidently wrong.
    #[test]
    fn movegen_matches_the_rules() {
        for game in random_games() {
            let legal = legal_moves(game.position(), game.turn());
            assert_movegen_matches(&legal, &State::of_game(&game), "movegen");
        }
    }

    /// With the foreign-camp rule on, the engine's move list must still be
    /// exactly the rules' (game-level, filtered) move list — the same parity as
    /// `movegen_matches_the_rules`, under a variant that fences landings.
    #[test]
    fn movegen_matches_the_rules_under_forbid_foreign_camps() {
        for game in random_games() {
            let game = game.with_variants(checkers_core::rules::Variants {
                forbid_foreign_camps: true,
            });
            if game.is_over() {
                continue;
            }
            let state = State::of_game(&game);
            assert_movegen_matches(&game.legal_moves(), &state, "filtered movegen");
        }
    }

    /// A jump that would land in a foreign camp drops out of the engine's move
    /// list exactly when the rule is on; the same chain with the rule off stays
    /// open. The setup: seat 0's piece at (0,4) can hop over the wall at (1,4)
    /// into camp 1 — a triangle seat 0 may pass through but never rest in.
    #[test]
    fn a_jump_landing_in_a_foreign_camp_is_not_offered() {
        let origin = Coord::new(0, 4);
        let landing = Coord::new(2, 4);
        let (from, to) = (
            index_of(origin).unwrap() as u8,
            index_of(landing).unwrap() as u8,
        );
        assert_eq!(
            TABLES.camp[to as usize], 1,
            "the landing must be inside camp 1 for this test"
        );

        let open = state_with(&[origin], &[Coord::new(1, 4)], 0);
        assert!(
            open.moves().contains(&pack(from, to)),
            "the jump into camp 1 is open with the rule off"
        );

        let mut fenced = open.clone();
        fenced.forbid_foreign_camps = true;
        assert!(
            fenced.moves().iter().all(|&m| unpack(m).1 != to),
            "the rule closes the foreign landing"
        );
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
            game.play(&moves[rng.below(moves.len())]);

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
        assert!(apex.eval_for(0) >= PROGRESS_MAX * 10 + 25);
    }

    /// The incremental hash must equal the canonical hash of the state it
    /// describes, because the transposition table keys on it.
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
            game.play(&moves[rng.below(moves.len())]);
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
            let moved = state.after(moves[rng.below(moves.len())]);
            assert_eq!(
                moved.hash,
                moved.zobrist(),
                "apply's incremental hash diverged from the canonical hash"
            );
        }
    }
}
