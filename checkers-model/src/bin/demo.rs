//! Prints the star, then plays a demo game.
//!
//! Also runs the solo reachability search that shows the win condition is
//! actually attainable — a greedy heuristic stalls on this board, so a proper
//! search is needed to demonstrate it.

use std::collections::{BinaryHeap, HashMap, HashSet};

use checkers_model::board::Player;
use checkers_model::{Board, Coord, Game, Outcome, State, legal_moves};

fn main() {
    let board = Board::new();
    board.validate();

    println!("Board (digits = camps, dots = central hexagon):\n");
    println!("{}", board.render());

    for p in 0..6u8 {
        print!(
            "  camp {p}: {} holes, {} hexagon contacts",
            board.camp(p).len(),
            board.camp_hex_contacts(p)
        );
        println!(" | target = camp {}", (p + 3) % 6);
    }

    let initial = State::initial(Board::new());
    println!("\nInitial mobility:");
    for p in 0..6u8 {
        println!(
            "  player {p}: {} legal moves",
            legal_moves(&initial, p).len()
        );
    }

    // --- random game ---
    let mut game = Game::new();
    let mut rng = checkers_model::prng::Prng::new(0xABCD);
    let outcome = game.run(2000, |_, _, moves| rng.below(moves.len() as u32) as usize);
    println!(
        "\nRandom play, 2000 plies: {}",
        match outcome {
            Some(Outcome::Winner(p)) => format!("player {p} won"),
            Some(Outcome::Draw) => "draw".to_string(),
            None => "no result (random play does not converge)".to_string(),
        }
    );

    // --- solo reachability ---
    match solo_fill_target(0) {
        Some(n) => println!(
            "Solo reachability search: player 0 filled the opposite camp in {n} moves \
             (some solution, not a shortest one -- see solo_fill_target)"
        ),
        None => println!("Solo reachability search: budget exhausted"),
    }
}

/// Greedy best-first search: can player 0 alone move from its camp into the
/// opposite camp? Other players are removed, isolating goal reachability.
///
/// This answers **reachability only**. The heuristic sums per-piece hex
/// distance, but a jump covers two hexes in one move, so it overestimates and is
/// therefore inadmissible — the move count returned is *some* solution, not a
/// shortest one. (A Python prototype of the same search found 51; this one finds
/// 67. Both are valid; neither is proven optimal.)
fn solo_fill_target(player: Player) -> Option<usize> {
    let board = Board::new();
    let target: HashSet<Coord> = board.target_camp(player).iter().copied().collect();

    let start: Vec<Coord> = {
        let mut v: Vec<Coord> = board.camp(player).iter().copied().collect();
        v.sort();
        v
    };

    let heuristic = |pieces: &[Coord]| -> i32 {
        pieces
            .iter()
            .filter(|p| !target.contains(p))
            .map(|p| target.iter().map(|t| p.distance(*t)).min().unwrap_or(0))
            .sum()
    };

    // max-heap on Reverse(f) via negated cost
    let mut open = BinaryHeap::new();
    let mut best: HashMap<Vec<Coord>, i32> = HashMap::new();
    open.push((-heuristic(&start), 0i32, start.clone()));
    best.insert(start, 0);

    let mut expanded = 0usize;
    while let Some((_, g, pieces)) = open.pop() {
        if pieces.iter().all(|c| target.contains(c)) {
            return Some(g as usize);
        }
        if best.get(&pieces).is_some_and(|&b| g > b) {
            continue;
        }
        expanded += 1;
        if expanded > 200_000 {
            return None;
        }

        let mut state = State::empty(board.clone());
        for &c in &pieces {
            state.set(c, Some(player));
        }

        for mv in legal_moves(&state, player) {
            let mut next: Vec<Coord> = pieces
                .iter()
                .copied()
                .filter(|&c| c != mv.origin)
                .chain(std::iter::once(mv.destination))
                .collect();
            next.sort();

            let ng = g + 1;
            if best.get(&next).is_none_or(|&b| ng < b) {
                best.insert(next.clone(), ng);
                open.push((-(ng + heuristic(&next)), ng, next));
            }
        }
    }
    None
}
