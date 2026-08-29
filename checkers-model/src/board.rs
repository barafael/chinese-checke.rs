//! Board geometry: the star, the camps, and the invariants that prove it is a
//! star rather than a 121-hole look-alike (Impl. Spec. §§2-6).

use std::collections::{HashSet, VecDeque};

use crate::coord::{Coord, Dir};

pub const NUM_PLAYERS: usize = 6;
pub const HOLES: usize = 121;
pub const CAMP_SIZE: usize = 10;
pub const HEX_HOLES: usize = 61;

/// Player index 0..6.
pub type Player = u8;

/// The board: the set of playable holes plus the six camps.
#[derive(Debug, Clone)]
pub struct Board {
    holes: HashSet<Coord>,
    hex: HashSet<Coord>,
    camps: [HashSet<Coord>; NUM_PLAYERS],
}

impl Board {
    /// Radius-4 central hexagon, 61 holes (§2).
    ///
    /// All three constraints are required: `|q+r| <= 4` alone describes an
    /// unbounded strip.
    fn central_hex() -> HashSet<Coord> {
        (-4..=4)
            .flat_map(|q| (-4..=4).map(move |r| Coord::new(q, r)))
            .filter(|c| c.q.abs() <= 4 && c.r.abs() <= 4 && (c.q + c.r).abs() <= 4)
            .collect()
    }

    /// Camp `C_0`, seated flush against the hexagon edge `q = 4` with its apex
    /// pointing **outward**: columns q=5..8 of sizes 4,3,2,1 (§3).
    ///
    /// The inward-pointing variant `-q+5 <= r <= 0` also yields 10 holes and a
    /// 121-hole board, but meets the hexagon in a single hole, so the camps hang
    /// off the corners. See `tests::inward_camp_is_rejected`.
    fn base_camp() -> HashSet<Coord> {
        (5..=8)
            .flat_map(|q| (-4..=-(q - 4)).map(move |r| Coord::new(q, r)))
            .collect()
    }

    /// Build the star: hexagon plus the six rotations of `C_0` (§5).
    pub fn new() -> Self {
        let hex = Self::central_hex();
        let base = Self::base_camp();

        let camps: [HashSet<Coord>; NUM_PLAYERS] = std::array::from_fn(|i| {
            base.iter()
                .map(|c| c.rotate60_n(i as u32))
                .collect::<HashSet<_>>()
        });

        let mut holes = hex.clone();
        for camp in &camps {
            holes.extend(camp.iter().copied());
        }

        Self { holes, hex, camps }
    }

    pub fn contains(&self, c: Coord) -> bool {
        self.holes.contains(&c)
    }

    pub fn holes(&self) -> impl Iterator<Item = Coord> + '_ {
        self.holes.iter().copied()
    }

    pub fn hex(&self) -> &HashSet<Coord> {
        &self.hex
    }

    pub fn camp(&self, player: Player) -> &HashSet<Coord> {
        &self.camps[player as usize % NUM_PLAYERS]
    }

    /// Target camp `O_i = C_{(i+3) mod 6}` (§7).
    pub fn target_camp(&self, player: Player) -> &HashSet<Coord> {
        self.camp((player + 3) % NUM_PLAYERS as u8)
    }

    /// Camp-to-hexagon adjacent pairs. Exactly 8 for a correct camp (§6):
    /// four base holes, each with two hexagon neighbours.
    pub fn camp_hex_contacts(&self, player: Player) -> usize {
        self.camp(player)
            .iter()
            .flat_map(|c| Dir::ALL.map(|d| c.add(d)))
            .filter(|n| self.hex.contains(n))
            .count()
    }

    pub fn is_connected(&self) -> bool {
        let Some(&start) = self.holes.iter().next() else {
            return true;
        };
        let mut seen = HashSet::from([start]);
        let mut queue = VecDeque::from([start]);
        while let Some(x) = queue.pop_front() {
            for d in Dir::ALL {
                let y = x.add(d);
                if self.holes.contains(&y) && seen.insert(y) {
                    queue.push_back(y);
                }
            }
        }
        seen.len() == self.holes.len()
    }

    /// The strong invariants of §6. Cardinality alone is insufficient.
    pub fn validate(&self) {
        assert_eq!(self.holes.len(), HOLES, "board must have 121 holes");
        assert_eq!(self.hex.len(), HEX_HOLES, "hexagon must have 61 holes");

        // Disjoint union: every hole covered exactly once.
        let mut seen = HashSet::new();
        for region in std::iter::once(&self.hex).chain(&self.camps) {
            for &x in region {
                assert!(seen.insert(x), "regions overlap at {x:?}");
            }
        }
        assert_eq!(seen, self.holes, "regions must exactly cover the board");

        for i in 0..NUM_PLAYERS as Player {
            assert_eq!(self.camp(i).len(), CAMP_SIZE, "camp {i} size");
            assert_eq!(
                self.camp_hex_contacts(i),
                8,
                "camp {i} must sit flush against a hexagon edge (8 contacts)"
            );
            let opposite: HashSet<Coord> = self.camp(i).iter().map(|c| c.negate()).collect();
            assert_eq!(
                *self.target_camp(i),
                opposite,
                "camp {i}'s target must be its point reflection"
            );
        }

        assert!(self.is_connected(), "board must be connected");
        let mirrored: HashSet<Coord> = self.holes.iter().map(|c| c.negate()).collect();
        assert_eq!(mirrored, self.holes, "board must be centrally symmetric");
    }

    /// ASCII star, row `r` drawn at horizontal offset `2q + r` (§5.1).
    pub fn render(&self) -> String {
        let label = |c: Coord| -> char {
            (0..NUM_PLAYERS as Player)
                .find(|&i| self.camp(i).contains(&c))
                .map_or('.', |i| char::from_digit(i as u32, 10).unwrap())
        };

        let min_off = self.holes().map(|c| 2 * c.q + c.r).min().unwrap_or(0);
        let (min_r, max_r) = (
            self.holes().map(|c| c.r).min().unwrap_or(0),
            self.holes().map(|c| c.r).max().unwrap_or(0),
        );

        let mut out = String::new();
        for r in min_r..=max_r {
            let row: Vec<Coord> = self.holes().filter(|c| c.r == r).collect();
            if row.is_empty() {
                continue;
            }
            let width = row.iter().map(|c| 2 * c.q + c.r - min_off).max().unwrap();
            let mut line = vec![' '; width as usize + 1];
            for c in row {
                line[(2 * c.q + c.r - min_off) as usize] = label(c);
            }
            out.push_str(line.iter().collect::<String>().trim_end());
            out.push('\n');
        }
        out
    }
}

impl Default for Board {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn board_satisfies_all_invariants() {
        Board::new().validate();
    }

    #[test]
    fn sizes_are_as_specified() {
        let b = Board::new();
        assert_eq!(b.holes().count(), 121);
        assert_eq!(b.hex().len(), 61);
        for i in 0..6 {
            assert_eq!(b.camp(i).len(), 10);
        }
    }

    #[test]
    fn opposite_camps_are_point_reflections() {
        let b = Board::new();
        for i in 0..6u8 {
            let expect: HashSet<Coord> = b.camp(i).iter().map(|c| c.negate()).collect();
            assert_eq!(*b.target_camp(i), expect);
            // and the relation is an involution
            assert_eq!(b.target_camp((i + 3) % 6), b.camp(i));
        }
    }

    /// The bug the original draft spec shipped: the inward-pointing triangle
    /// passes every cardinality check but is not a star.
    #[test]
    fn inward_camp_is_rejected() {
        let hex = Board::central_hex();
        let inward: HashSet<Coord> = (5..=8)
            .flat_map(|q| (-q + 5..=0).map(move |r| Coord::new(q, r)))
            .collect();

        // Cardinality checks all pass ...
        assert_eq!(inward.len(), CAMP_SIZE);
        let camps: Vec<HashSet<Coord>> = (0..6)
            .map(|i| inward.iter().map(|c| c.rotate60_n(i)).collect())
            .collect();
        let mut all = hex.clone();
        for c in &camps {
            all.extend(c.iter().copied());
        }
        assert_eq!(all.len(), HOLES, "the look-alike also has 121 holes");

        // ... but the contact invariant catches it: 1 contact, not 8.
        for camp in &camps {
            let contacts = camp
                .iter()
                .flat_map(|c| Dir::ALL.map(|d| c.add(d)))
                .filter(|n| hex.contains(n))
                .count();
            assert_eq!(contacts, 1, "inward camps touch the hexagon once");
        }
    }

    #[test]
    fn render_looks_like_a_star() {
        let rendered = Board::new().render();
        let lines: Vec<&str> = rendered.lines().collect();
        assert_eq!(lines.len(), 17, "9 hexagon rows + 4 rows per polar camp");

        // Top and bottom rows are single-hole camp apexes.
        assert_eq!(lines[0].trim(), "5");
        assert_eq!(lines[16].trim(), "2");
        // The widest rows are the hexagon edges flanked by two camps.
        assert!(lines[4].starts_with("4 4 4 4"));
        assert!(lines[4].trim_end().ends_with("0 0 0 0"));
    }
}
