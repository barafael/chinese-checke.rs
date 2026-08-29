//! Axial hex coordinates and the six directions (Impl. Spec. §1, §10).

/// Axial hex coordinate. See Impl. Spec. §1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Coord {
    pub q: i32,
    pub r: i32,
}

impl Coord {
    pub const fn new(q: i32, r: i32) -> Self {
        Self { q, r }
    }

    /// `x + d`
    pub const fn add(self, d: Dir) -> Self {
        Self::new(self.q + d.dq(), self.r + d.dr())
    }

    /// `x + 2d`, the jump destination (§10).
    pub const fn jump_dest(self, d: Dir) -> Self {
        Self::new(self.q + 2 * d.dq(), self.r + 2 * d.dr())
    }

    /// Point reflection through the centre. `C_{i+3} = -C_i` (§4).
    pub const fn negate(self) -> Self {
        Self::new(-self.q, -self.r)
    }

    /// 60° rotation `R(q,r) = (-r, q+r)` (§4).
    pub const fn rotate60(self) -> Self {
        Self::new(-self.r, self.q + self.r)
    }

    pub fn rotate60_n(self, n: u32) -> Self {
        (0..n).fold(self, |c, _| c.rotate60())
    }

    /// Hex distance, used only by the demo bin's heuristic.
    pub fn distance(self, other: Self) -> i32 {
        let (dq, dr) = (self.q - other.q, self.r - other.r);
        dq.abs().max(dr.abs()).max((dq + dr).abs())
    }
}

/// The six adjacency directions, in rotational order (§1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dir {
    /// (1, 0)
    D0,
    /// (1, -1)
    D1,
    /// (0, -1)
    D2,
    /// (-1, 0)
    D3,
    /// (-1, 1)
    D4,
    /// (0, 1)
    D5,
}

impl Dir {
    pub const ALL: [Dir; 6] = [Dir::D0, Dir::D1, Dir::D2, Dir::D3, Dir::D4, Dir::D5];

    pub const fn dq(self) -> i32 {
        match self {
            Dir::D0 | Dir::D1 => 1,
            Dir::D2 | Dir::D5 => 0,
            Dir::D3 | Dir::D4 => -1,
        }
    }

    pub const fn dr(self) -> i32 {
        match self {
            Dir::D0 | Dir::D3 => 0,
            Dir::D1 | Dir::D2 => -1,
            Dir::D4 | Dir::D5 => 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directions_are_closed_under_negation() {
        // -d ∈ D for every d ∈ D (§1).
        for d in Dir::ALL {
            assert!(
                Dir::ALL
                    .iter()
                    .any(|e| e.dq() == -d.dq() && e.dr() == -d.dr()),
                "{d:?} has no opposite"
            );
        }
    }

    #[test]
    fn rotation_has_order_six() {
        // R^6 = id (§4).
        let x = Coord::new(3, -1);
        assert_eq!(x.rotate60_n(6), x);
        assert_ne!(x.rotate60_n(3), x);
    }

    #[test]
    fn three_rotations_equal_negation() {
        // R^3 = -id, which is why C_{i+3} is the opposite camp (§4).
        for q in -4..=4 {
            for r in -4..=4 {
                let x = Coord::new(q, r);
                assert_eq!(x.rotate60_n(3), x.negate());
            }
        }
    }

    #[test]
    fn rotation_steps_backwards_through_dir_indices() {
        // R maps d0 -> d5, documented in §4's orientation note.
        let d0 = Coord::new(Dir::D0.dq(), Dir::D0.dr());
        let d5 = Coord::new(Dir::D5.dq(), Dir::D5.dr());
        assert_eq!(d0.rotate60(), d5);
    }
}
