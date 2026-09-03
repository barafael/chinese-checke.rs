//! The search: iterative deepening under a time budget.
//!
//! Two-player games are searched with negamax and alpha-beta pruning over a
//! zobrist transposition table — the classical stack, and for a race game with
//! no captures it goes deep fast because move ordering by forward progress is
//! naturally good.
//!
//! With three or more players the tree is wide and alpha-beta's soundness
//! assumptions break down (the next player is not minimising), so the engine
//! plays maxⁿ: each seat maximises its own component of the score vector, at
//! a depth the time budget allows. The transposition table is deliberately not
//! used there — multi-component values do not admit single-value flags.
//!
//! Every score a node returns is from the perspective of the player to move
//! at that node; parents negate. That invariant is what lets the WIN checks,
//! the evaluation, and the pass handling compose without sign errors.
//!
//! Both searches treat an empty move list as a forced pass, and both stop at
//! the time budget, discarding a partially searched depth (standard practice:
//! a partial iteration's scores are unsound; its best move is not used).

use crate::engine::{RawMove, State};
use crate::tables::TABLES;
use crate::{AiConfig, AiStats};
use std::collections::HashMap;
use std::time::Duration;

// `Instant` is deliberately taken from Bevy's platform crate rather than std:
// on `wasm32-unknown-unknown` std's wall clock panics, while Bevy's selects a
// `performance.now()`-backed clock on the web and mirrors `std::time::Instant`
// everywhere else. Neither the engine nor its config mention Bevy otherwise.
use bevy_platform::time::Instant;

/// A win is worth more than any evaluation; winning sooner ranks higher.
const WIN: i32 = 100_000;
/// A search that reaches all-passes is the draw the rules describe.
const DRAW: i32 = 0;

/// Transposition entry, two-player only. Flags: exact, lower bound, upper
/// bound — the alpha-beta classic.
#[derive(Debug, Clone, Copy)]
struct Entry {
    depth: u8,
    flag: u8,
    score: i32,
    mv: Option<RawMove>,
}

const EXACT: u8 = 0;
const LOWER: u8 = 1;
const UPPER: u8 = 2;

/// A pass-through hasher for u64 keys: zobrist keys are already uniform, and
/// SipHash would spend more time hashing than searching.
#[derive(Default)]
struct IdentityHasher(u64);

impl std::hash::Hasher for IdentityHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        // Only ever called with a u64 key by HashMap<u64, _>.
        self.0 = u64::from_le_bytes(bytes.try_into().expect("8 bytes"));
    }
}

type Table = HashMap<u64, Entry, std::hash::BuildHasherDefault<IdentityHasher>>;

fn table() -> Table {
    HashMap::with_hasher(std::hash::BuildHasherDefault::default())
}

/// Deadline and node counter shared by the recursion of one search.
struct Budget {
    deadline: Instant,
    nodes: u64,
    expired: bool,
}

impl Budget {
    fn new(budget: Duration) -> Self {
        Self {
            deadline: Instant::now() + budget,
            nodes: 0,
            expired: false,
        }
    }
    fn tick(&mut self) -> bool {
        self.nodes += 1;
        if self.nodes.is_multiple_of(4096) && Instant::now() >= self.deadline {
            self.expired = true;
        }
        self.expired
    }
}

/// Search the state for its player to move, over the seated `players`.
///
/// `history` holds zobrist keys this game has already seen; a root move into
/// one of them is refused while any alternative exists, which is what stops a
/// race engine from shuffling in a won position.
pub fn search(state: &State, config: &AiConfig, history: &[u64]) -> (Option<RawMove>, AiStats) {
    // Every seat that owns a piece. The rules guarantee ten per seated player
    // and none for vacant camps, so piece presence identifies the seated set.
    let mut seated: Vec<usize> = (0..6).filter(|&p| state.pieces[p] != 0).collect();

    let mut budget = Budget::new(config.budget);
    let mut tt = table();
    let mut best: Option<RawMove> = None;
    let mut stats = AiStats::default();

    if seated.len() == 2 {
        // Two players: negamax from the mover's seat.
        let me = state.turn as usize;
        if let Some(pos) = seated.iter().position(|&p| p == me) {
            seated.rotate_left(pos);
        }
        let rival = seated[1];
        // The race-game tempo trap: at even depths the leaf sits after the
        // opponent's move, and the evaluation's home-fill and straggler terms
        // swing hundreds of points with it. Taking the deepest iteration's
        // move therefore alternates between racing and despairing — the
        // observed behaviour is an engine hopping one piece sideways forever.
        // Instead, keep the best move over completed iterations: in a race,
        // the most optimistic *confirmed* line is the one that goes forward.
        let mut best_score = i32::MIN;
        for depth in 1..=config.max_depth {
            let mut ctx = Ctx {
                me,
                rival,
                budget: &mut budget,
                tt: &mut tt,
                no_tt: false,
            };
            let (score, mv) = negamax(state, &mut ctx, depth, i32::MIN + 1, i32::MAX, 0);
            if budget.expired {
                break;
            }
            if let Some(mv) = mv
                && score > best_score
            {
                best_score = score;
                best = Some(mv);
                stats.depth = depth;
                stats.nodes = budget.nodes;
            }
            if score.abs() > WIN / 2 {
                break; // forced result found: deeper search cannot improve it
            }
        }
    } else {
        for depth in 1..=config.max_depth.min(8) {
            let (_vector, mv) = maxn(state, &seated, depth, &mut budget);
            if budget.expired {
                break;
            }
            if let Some(mv) = mv {
                best = Some(mv);
                stats.depth = depth;
                stats.nodes = budget.nodes;
            }
        }
    }

    // A safety net: an expired budget must never read as "pass". If no
    // iteration completed, play the statically best move.
    let best = best.or_else(|| state.moves().first().copied());

    // Anti-shuffling at the root: refuse to revisit a known position while
    // any alternative exists.
    let best = refuse_history(state, best, history);
    (best, stats)
}

/// Static root ordering: forward progress first, then everything else.
fn ordered_moves(state: &State) -> Vec<RawMove> {
    let mut moves = state.moves();
    let me = state.turn as usize;
    moves.sort_by_key(|&mv| {
        let (from, to) = crate::engine::unpack(mv);
        -(TABLES.dist[me][from as usize] - TABLES.dist[me][to as usize])
    });
    moves
}

/// The search's best move, unless it revisits a known position and some other
/// move does not — in which case the escape move plays. A race engine that
/// shuffles is a race engine that loses.
fn refuse_history(state: &State, best: Option<RawMove>, history: &[u64]) -> Option<RawMove> {
    // Both kinds of repeat count: the same board with the same seat to move
    // (one wasted move) and the same board with the other seat to move (two).
    let leads_back = |mv: RawMove| {
        let mut probe = state.clone();
        probe.apply(mv);
        history.contains(&probe.hash) || history.contains(&probe.piece_hash())
    };
    match best {
        Some(mv) if leads_back(mv) => {
            let escape = ordered_moves(state).into_iter().find(|&mv| !leads_back(mv));
            escape.or(Some(mv))
        }
        other => other,
    }
}

/// Everything a two-player node needs beyond the state itself.
struct Ctx<'a> {
    me: usize,
    rival: usize,
    budget: &'a mut Budget,
    tt: &'a mut Table,
    /// Debug switch: a table that answers nothing cannot corrupt anything.
    no_tt: bool,
}

fn negamax(
    state: &State,
    ctx: &mut Ctx,
    depth: u8,
    mut alpha: i32,
    mut beta: i32,
    passes: u32,
) -> (i32, Option<RawMove>) {
    if ctx.budget.tick() {
        return (0, None);
    }

    // Terminal, from the mover's perspective. A finished camp means the owner
    // won: +WIN if that is the mover, -WIN if it is the other seat.
    let mover = state.turn as usize;
    let other = if mover == ctx.me { ctx.rival } else { ctx.me };
    if state.pieces[mover] & TABLES.target[mover] == TABLES.target[mover] {
        return (WIN - (24 - depth) as i32, None);
    }
    if state.pieces[other] & TABLES.target[other] == TABLES.target[other] {
        return (-(WIN - (24 - depth) as i32), None);
    }
    // Twelve seats in a row without a move: two full rounds of passes, the
    // draw the rules describe.
    if passes >= 12 {
        return (DRAW, None);
    }

    if depth == 0 {
        // Mover-relative: own race score against the only rival.
        let score = (state.eval_for(mover) - state.eval_for(other)) * 4;
        return (score, None);
    }

    let alpha_orig = alpha;
    let mut best_mv: Option<RawMove> = ctx.tt.get(&state.hash).and_then(|e| e.mv);

    let entry = if ctx.no_tt {
        None
    } else {
        ctx.tt.get(&state.hash)
    };
    if let Some(entry) = entry
        && entry.depth >= depth
    {
        match entry.flag {
            EXACT => return (entry.score, entry.mv),
            LOWER => alpha = alpha.max(entry.score),
            UPPER => beta = beta.min(entry.score),
            _ => {}
        }
        if alpha >= beta {
            return (entry.score, entry.mv);
        }
    }

    let mut moves = ordered_moves(state);
    if moves.is_empty() {
        // Forced pass: the turn advances, the pass counter guards termination.
        let old_turn = state.turn as usize;
        let mut next = state.clone();
        next.turn = (next.turn + 1) % 6;
        next.hash ^= TABLES.zobrist_turn[old_turn] ^ TABLES.zobrist_turn[next.turn as usize];
        let (score, _) = negamax(
            &next,
            ctx,
            depth.saturating_sub(1),
            -beta,
            -alpha,
            passes + 1,
        );
        return (-score, None);
    }

    if let Some(tt_mv) = best_mv
        && let Some(pos) = moves.iter().position(|&m| m == tt_mv)
    {
        moves.swap(0, pos);
    }

    let mut flag = UPPER;
    let mut best_score = i32::MIN;
    for mv in moves {
        let mut next = state.clone();
        next.apply(mv);
        let (score, _) = negamax(&next, ctx, depth.saturating_sub(1), -beta, -alpha, 0);
        let score = -score;
        if ctx.budget.expired {
            return (0, None);
        }
        if score > best_score {
            best_score = score;
            best_mv = Some(mv);
        }
        if best_score > alpha {
            alpha = best_score;
            flag = EXACT;
        }
        if alpha >= beta {
            flag = LOWER;
            break;
        }
    }

    if !ctx.no_tt {
        ctx.tt.insert(
            state.hash,
            Entry {
                depth,
                flag,
                score: best_score,
                mv: best_mv,
            },
        );
    }
    let _ = alpha_orig;
    (best_score, best_mv)
}

/// Maxⁿ: every seated player maximises its own score component. Returns the
/// full vector plus the best move for the seat to move.
fn maxn(
    state: &State,
    seated: &[usize],
    depth: u8,
    budget: &mut Budget,
) -> ([i32; 6], Option<RawMove>) {
    if budget.tick() {
        return ([0; 6], None);
    }

    let mover = state.turn as usize;
    // A finished camp scores a win for its owner.
    for &p in seated {
        if state.pieces[p] & TABLES.target[p] == TABLES.target[p] {
            let mut v = [0; 6];
            v[p] = WIN;
            return (v, None);
        }
    }

    if depth == 0 {
        let mut v = [0; 6];
        for &p in seated {
            v[p] = state.eval_for(p) * 4;
        }
        return (v, None);
    }

    let moves = ordered_moves(state);
    if moves.is_empty() {
        let old_turn = state.turn as usize;
        let mut next = state.clone();
        next.turn = (next.turn + 1) % 6;
        next.hash ^= TABLES.zobrist_turn[old_turn] ^ TABLES.zobrist_turn[next.turn as usize];
        return maxn(&next, seated, depth.saturating_sub(1), budget);
    }

    let mut best_vector = [i32::MIN; 6];
    let mut best_mv: Option<RawMove> = None;
    for mv in moves {
        let mut next = state.clone();
        next.apply(mv);
        let (vector, _) = maxn(&next, seated, depth.saturating_sub(1), budget);
        if budget.expired {
            return (best_vector, best_mv);
        }
        if vector[mover] > best_vector[mover] {
            best_vector = vector;
            best_mv = Some(mv);
        }
    }
    (best_vector, best_mv)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{pack, unpack};
    use crate::tables::index_of;
    use crate::{Ai, AiConfig};
    use checkers_core::geometry::Coord;
    use checkers_core::position::Player;
    use checkers_core::rules::Game;
    use std::time::Duration;

    fn config() -> AiConfig {
        AiConfig {
            budget: Duration::from_millis(40),
            max_depth: 6,
        }
    }

    /// A sparse state: one racing piece for seat 0, a wall for seat 3.
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

    /// The search must refuse a root move that lands on a position this game
    /// has already seen, while any alternative exists. The setup: seat 0's
    /// piece at (2, 0) can jump the wall at (1, 0) back to (0, 0) — and (0, 0)
    /// is exactly where the last position had it.
    #[test]
    fn the_engine_refuses_to_shuffle_back() {
        let state = state_with(
            &[Coord::new(2, 0), Coord::new(3, -1)],
            &[Coord::new(1, 0), Coord::new(-1, 0)],
            0,
        );

        // The shuffling move: jump back over the wall.
        let shuffle = pack(
            index_of(Coord::new(2, 0)).expect("origin") as u8,
            index_of(Coord::new(0, 0)).expect("destination") as u8,
        );

        // Without history, the search may pick it; the jump is real progress
        // through the middle.
        let (free, _) = search(&state, &config(), &[]);
        assert!(free.is_some(), "a moveable position must yield a move");

        // With (0,0)-having-been-occupied in the history, that exact landing
        // is out: the engine must pick something else rather than shuffle.
        let landing = {
            let mut probe = state.clone();
            probe.apply(shuffle);
            probe.hash
        };
        let (refused, _) = search(&state, &config(), &[landing]);
        let refused = refused.expect("a moveable position must still yield a move");
        assert_ne!(refused, shuffle, "the engine returned to a known position");
    }

    /// A seat with no legal move passes; the search answers `None`, which the
    /// caller turns into the rules' `pass`. The fixture seals seat 0's corner
    /// piece behind occupied neighbours with no landing beyond them.
    #[test]
    fn a_stuck_seat_is_a_pass() {
        let state = state_with(
            &[Coord::new(8, -4)],
            &[
                Coord::new(7, -4),
                Coord::new(7, -3),
                Coord::new(6, -4),
                Coord::new(6, -2),
            ],
            0,
        );
        let offered: Vec<String> = state
            .moves()
            .into_iter()
            .map(|mv| {
                let (f, t) = unpack(mv);
                let (f, t) = (TABLES.coord[f as usize], TABLES.coord[t as usize]);
                format!("({},{}) -> ({},{})", f.q, f.r, t.q, t.r)
            })
            .collect();
        assert!(
            offered.is_empty(),
            "the fixture must be stuck, but offers: {offered:?}"
        );
        let (mv, _) = search(&state, &config(), &[]);
        assert!(mv.is_none(), "no move exists, so no move may be returned");
    }

    /// Black-box, through the public API: the chosen move is always one of the
    /// rules' legal moves, for any game shape, and a seat that is not to move
    /// is refused.
    #[test]
    fn chosen_moves_are_always_legal() {
        let mut ai = Ai::new(config());
        let mut rng = checkers_core::Xorshift::new(7);

        for players in [
            vec![Player::ALL[0], Player::ALL[3]],
            vec![Player::ALL[0], Player::ALL[2], Player::ALL[4]],
        ] {
            let mut game = Game::for_players(&players);
            for _ in 0..6 {
                if game.is_over() {
                    break;
                }
                let legal = game.legal_moves();
                if legal.is_empty() {
                    game.pass();
                    continue;
                }
                let Some(mv) = ai.choose_move(&game) else {
                    panic!("a moveable position must yield a move");
                };
                assert!(
                    legal.iter().any(|m| m.kind == mv.kind
                        && m.origin == mv.origin
                        && m.destination == mv.destination),
                    "the engine played {mv:?}, which the rules do not offer"
                );
                game.play(&mv);

                // The wrong seat is refused outright.
                let wrong = Player::wrapping(game.turn().index() + 1);
                assert!(ai.choose_move_for(&game, wrong).is_none());
                let _ = rng.below(1); // keep the generator referenced
            }
        }
    }
}

#[cfg(test)]
mod brute_force_check {
    use super::*;
    use crate::engine::State;
    use checkers_core::geometry::Coord;
    use checkers_core::position::Player;
    use checkers_core::rules::Game;

    /// Full-window minimax with no pruning and no table — the ground truth
    /// negamax must reproduce.
    fn minimax(state: &State, me: usize, rival: usize, depth: u8, passes: u32) -> i32 {
        let mover = state.turn as usize;
        let other = if mover == me { rival } else { me };
        if state.pieces[mover] & TABLES.target[mover] == TABLES.target[mover] {
            return WIN - (24 - depth) as i32;
        }
        if state.pieces[other] & TABLES.target[other] == TABLES.target[other] {
            return -(WIN - (24 - depth) as i32);
        }
        if passes >= 12 {
            return DRAW;
        }
        if depth == 0 {
            return (state.eval_for(mover) - state.eval_for(other)) * 4;
        }
        let moves = state.moves();
        if moves.is_empty() {
            let old_turn = state.turn as usize;
            let mut next = state.clone();
            next.turn = (next.turn + 1) % 6;
            next.hash ^= TABLES.zobrist_turn[old_turn] ^ TABLES.zobrist_turn[next.turn as usize];
            return -minimax(&next, me, rival, depth.saturating_sub(1), passes + 1);
        }
        moves
            .into_iter()
            .map(|mv| {
                let mut next = state.clone();
                next.apply(mv);
                -minimax(&next, me, rival, depth.saturating_sub(1), 0)
            })
            .max()
            .expect("moves non-empty")
    }

    fn ply2_state() -> State {
        let mut game = Game::for_players(&[Player::ALL[0], Player::ALL[3]]);
        use checkers_core::position::MoveKind;
        for (oq, or, dq, dr) in [(6, -4, 4, -2), (-6, 2, -4, 0)] {
            let (origin, destination) = (Coord::new(oq, or), Coord::new(dq, dr));
            let kind = if origin.distance(destination) == 1 {
                MoveKind::Step
            } else {
                MoveKind::Jump
            };
            game.play(&checkers_core::position::Move {
                kind,
                origin,
                destination,
                route: None,
            });
        }
        State::of_game(&game)
    }

    #[test]
    fn negamax_agrees_with_brute_force() {
        let state = ply2_state();
        let me = state.turn as usize;
        let rival = 3;

        for depth in [1u8, 2, 3, 4] {
            let truth = minimax(&state, me, rival, depth, 0);

            let mut budget = Budget::new(Duration::from_secs(60));
            let mut tt = table();
            let mut ctx = Ctx {
                me,
                rival,
                budget: &mut budget,
                tt: &mut tt,
                no_tt: false,
            };
            let (neg, _) = negamax(&state, &mut ctx, depth, i32::MIN + 1, i32::MAX, 0);

            assert_eq!(
                neg, truth,
                "depth {depth}: negamax {neg} != brute-force {truth}"
            );
        }
    }

    /// And per root move: the move negamax ranks best must be the move the
    /// brute force ranks best.
    #[test]
    fn root_move_choice_agrees_with_brute_force() {
        let state = ply2_state();
        let me = state.turn as usize;
        let rival = 3;

        for depth in [1u8, 2, 3] {
            let mut by_brute: Vec<(i32, u16)> = state
                .moves()
                .into_iter()
                .map(|mv| {
                    let mut next = state.clone();
                    next.apply(mv);
                    (-minimax(&next, me, rival, depth - 1, 0), mv)
                })
                .collect();
            by_brute.sort();

            let mut budget = Budget::new(Duration::from_secs(60));
            let mut tt = table();
            let mut ctx = Ctx {
                me,
                rival,
                budget: &mut budget,
                tt: &mut tt,
                no_tt: false,
            };
            let (_, neg_best) = negamax(&state, &mut ctx, depth, i32::MIN + 1, i32::MAX, 0);

            let best_brute = by_brute.last().copied().map(|(_, mv)| mv);
            assert_eq!(
                neg_best, best_brute,
                "depth {depth}: negamax chose a different move than brute force"
            );
        }
    }
}
