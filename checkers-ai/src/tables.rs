//! Precomputed geometry tables.
//!
//! The engine never touches coordinates during search: every hole is an index
//! into arrays built once from [`checkers_core::geometry`]. The tables are the
//! bridge between the rules' coordinate world and the engine's bitboard world,
//! and they are derived from the rules rather than hand-written, so a change
//! to the geometry flows through.

use checkers_core::geometry::{Coord, Dir, all_holes, camp_of, in_camp, rotate_n};
use std::array::from_fn;
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
    /// Which camp each hole belongs to, per the rules' `camp_of`: 0–5 for the
    /// triangle tips, `u8::MAX` for the central hexagon's holes.
    pub camp: [u8; HOLES],
    /// Zobrist keys: one per (player, hole), plus one per player to move.
    pub zobrist_piece: [[u64; HOLES]; 6],
    pub zobrist_turn: [u64; 6],
}

/// The coordinate a hole index denotes.
pub fn coord_of(index: u8) -> Coord {
    TABLES.coord[index as usize]
}

/// The hole index of a board coordinate, for tests building states by hand.
#[cfg(test)]
pub(crate) fn index_of(c: Coord) -> Option<usize> {
    TABLES.index.get(&c).copied()
}

fn build() -> Tables {
    let holes = all_holes();
    assert_eq!(holes.len(), HOLES, "the board is 121 holes");
    let coord = from_fn(|i| holes[i]);
    let index: HashMap<Coord, usize> = holes.iter().enumerate().map(|(i, &c)| (c, i)).collect();
    let hole_at = |c: Coord| index.get(&c).map(|&i| i as u8);

    let mut nbr = [[None; HOLES]; 6];
    let mut jmp = [[None; HOLES]; 6];
    for (i, c) in holes.iter().enumerate() {
        for (d, dir) in Dir::ALL.iter().enumerate() {
            nbr[d][i] = hole_at(c.neighbour(*dir));
            jmp[d][i] = hole_at(c.jump_dest(*dir));
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
            if in_camp(*c, target_camp as u32) {
                target[p] |= 1u128 << i;
            }
        }
    }

    // Each hole's camp under the rules' `camp_of`, stored so the engine's own
    // move filter can fence landings without touching the geometry crate.
    let camp = from_fn(|i| camp_of(holes[i]).map_or(u8::MAX, |k| k as u8));

    // Zobrist keys from the workspace's own xorshift, so the crate stays
    // dependency-free and the hashes are stable across runs.
    let mut rng = checkers_core::Xorshift::new(0x2A11_C0DE);
    let zobrist_piece = from_fn(|_| from_fn(|_| rng.next_u64()));
    let zobrist_turn = from_fn(|_| rng.next_u64());

    Tables {
        coord,
        index,
        nbr,
        jmp,
        dist,
        target,
        camp,
        zobrist_piece,
        zobrist_turn,
    }
}
