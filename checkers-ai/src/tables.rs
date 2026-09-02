//! Precomputed geometry tables.
//!
//! The engine never touches coordinates during search: every hole is an index
//! into arrays built once from [`checkers_core::geometry`]. The tables are the
//! bridge between the rules' coordinate world and the engine's bitboard world,
//! and they are derived from the rules rather than hand-written, so a change
//! to the geometry flows through.

use checkers_core::geometry::{Coord, Dir, all_holes, in_camp, rotate_n};
use std::collections::HashMap;
use std::sync::LazyLock;

pub const HOLES: usize = 121;
/// Strictly greater than any hex distance on the board, so
/// `PROGRESS_MAX - distance` is a non-negative per-piece progress score.
pub const PROGRESS_MAX: i32 = 16;

pub static TABLES: LazyLock<Tables> = LazyLock::new(build);

pub struct Tables {
    /// Coordinate of each hole index. Indexes are positions in the sorted
    /// `all_holes()` order, which is also the rules' canonical hole order.
    pub coord: [Coord; HOLES],
    /// Hole index of each coordinate, for building engine states from the
    /// rules' positions.
    pub index: HashMap<Coord, usize>,
    /// Step neighbour and jump landing per direction; `None` off the board.
    pub nbr: [[Option<u8>; HOLES]; 6],
    pub jmp: [[Option<u8>; HOLES]; 6],
    /// Hex distance from each hole to the apex of player p's target camp.
    /// Lower is further along the race for that player.
    pub dist: [[i32; HOLES]; 6],
    /// Bitmask of the target camp's holes per player.
    pub target: [u128; 6],
    /// Zobrist keys: one per (player, hole), plus one per player to move.
    pub zobrist_piece: [[u64; HOLES]; 6],
    pub zobrist_turn: [u64; 6],
}

/// The coordinate a hole index denotes.
pub fn coord_of(index: u8) -> Coord {
    TABLES.coord[index as usize]
}

/// The hole index of a board coordinate. Kept for tests and callers building
/// states by hand.
#[allow(dead_code)]
pub fn index_of(c: Coord) -> Option<usize> {
    TABLES.index.get(&c).copied()
}

fn build() -> Tables {
    let holes = all_holes();
    assert_eq!(holes.len(), HOLES, "the board is 121 holes");
    let mut coord = [Coord::ORIGIN; HOLES];
    let mut index = HashMap::with_capacity(HOLES);
    for (i, c) in holes.iter().enumerate() {
        coord[i] = *c;
        index.insert(*c, i);
    }

    let mut nbr = [[None; HOLES]; 6];
    let mut jmp = [[None; HOLES]; 6];
    for (i, c) in holes.iter().enumerate() {
        for (d, dir) in Dir::ALL.iter().enumerate() {
            let n = c.neighbour(*dir);
            if let Some(&ni) = index.get(&n) {
                nbr[d][i] = Some(ni as u8);
            }
            let j = c.jump_dest(*dir);
            if let Some(&ji) = index.get(&j) {
                jmp[d][i] = Some(ji as u8);
            }
        }
    }

    // Player p races toward camp (p+3) % 6, whose apex is the base apex
    // rotated p+3 times.
    let mut dist = [[0; HOLES]; 6];
    let mut target = [0u128; 6];
    for p in 0..6usize {
        let target_camp = (p + 3) % 6;
        let apex = rotate_n(Coord::new(8, -4), target_camp as u32);
        for (i, c) in holes.iter().enumerate() {
            dist[p][i] = c.distance(apex);
        }
        let mut mask = 0u128;
        for c in holes.iter().copied() {
            if in_camp(c, target_camp as u32)
                && let Some(&i) = index.get(&c)
            {
                mask |= 1u128 << i;
            }
        }
        target[p] = mask;
    }

    // Zobrist keys from the workspace's own xorshift, so the crate stays
    // dependency-free and the hashes are stable across runs.
    let mut rng = checkers_core::Xorshift::new(0x2A11_C0DE);
    let mut zobrist_piece = [[0u64; HOLES]; 6];
    for row in zobrist_piece.iter_mut() {
        for key in row.iter_mut() {
            *key = rng.next_u64();
        }
    }
    let mut zobrist_turn = [0u64; 6];
    for key in zobrist_turn.iter_mut() {
        *key = rng.next_u64();
    }

    Tables {
        coord,
        index,
        nbr,
        jmp,
        dist,
        target,
        zobrist_piece,
        zobrist_turn,
    }
}
