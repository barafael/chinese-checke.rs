//! Test fixtures shared by the engine's unit-test modules.

#![cfg(test)]

use checkers_core::geometry::Coord;

use crate::engine::State;
use crate::tables::index_of;

/// A state with a few pieces placed by coordinate.
pub fn state_with(p0: &[Coord], p3: &[Coord], turn: u8) -> State {
    let mut s = State {
        pieces: [0; 6],
        occupied: 0,
        turn,
        hash: 0,
        forbid_foreign_camps: false,
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
