//! Board geometry: coordinates, directions, rotation, and the star.
//!
//! Everything here is written **loop-free** so that the Kani harnesses at the
//! bottom of this file can prove the geometry laws over the whole domain. A
//! symbolic loop bound makes bounded model checking diverge — an early version
//! of [`rotate_n`] written as `(0..n).fold(..)` sent Kani unwinding past 6900
//! iterations before it was killed. The `match` below is the price of
//! provability, and it is a cheap one.
//!
//! Laws proven about this module: see [`crate::laws::geometry`].

/// Axial hex coordinate (Impl §1).
///
/// The third cube coordinate is implicit: $s = -q - r$.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Coord {
    pub q: i32,
    pub r: i32,
}

impl Coord {
    pub const ORIGIN: Coord = Coord::new(0, 0);

    pub const fn new(q: i32, r: i32) -> Self {
        Self { q, r }
    }

    /// The implicit third cube coordinate, $s = -q-r$.
    pub const fn s(self) -> i32 {
        -self.q - self.r
    }

    /// Neighbour in direction `d`: $x + d$.
    pub const fn neighbour(self, d: Dir) -> Self {
        Self::new(self.q + d.dq(), self.r + d.dr())
    }

    /// Jump destination in direction `d`: $x + 2d$ (Impl §10).
    pub const fn jump_dest(self, d: Dir) -> Self {
        Self::new(self.q + 2 * d.dq(), self.r + 2 * d.dr())
    }

    /// Point reflection through the centre: $-x$.
    pub const fn negate(self) -> Self {
        Self::new(-self.q, -self.r)
    }

    /// Hex distance.
    pub const fn distance(self, other: Self) -> i32 {
        let dq = (self.q - other.q).abs();
        let dr = (self.r - other.r).abs();
        let ds = (self.s() - other.s()).abs();
        if dq >= dr && dq >= ds {
            dq
        } else if dr >= ds {
            dr
        } else {
            ds
        }
    }
}

/// One $60^\circ$ rotation: $R(q,r) = (-r,\; q+r)$ (Impl §4).
pub const fn rotate60(c: Coord) -> Coord {
    Coord::new(-c.r, c.q + c.r)
}

/// $R^n$ for any `n`, written loop-free so bounded model checking terminates.
///
/// Only `n % 6` matters, since $R^6 = \mathrm{id}$.
pub const fn rotate_n(c: Coord, n: u32) -> Coord {
    match n % 6 {
        0 => c,
        1 => rotate60(c),
        2 => rotate60(rotate60(c)),
        3 => rotate60(rotate60(rotate60(c))),
        4 => rotate60(rotate60(rotate60(rotate60(c)))),
        _ => rotate60(rotate60(rotate60(rotate60(rotate60(c))))),
    }
}

/// The six adjacency directions, in rotational order (Impl §1).
///
/// Being an enum rather than a `[Coord; 6]` makes "there are exactly six
/// directions" a *structural* fact: no other value is representable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Dir {
    /// $(1, 0)$
    E,
    /// $(1, -1)$
    Ne,
    /// $(0, -1)$
    Nw,
    /// $(-1, 0)$
    W,
    /// $(-1, 1)$
    Sw,
    /// $(0, 1)$
    Se,
}

impl Dir {
    /// All six directions in rotational order.
    pub const ALL: [Dir; 6] = [Dir::E, Dir::Ne, Dir::Nw, Dir::W, Dir::Sw, Dir::Se];

    pub const fn dq(self) -> i32 {
        match self {
            Dir::E | Dir::Ne => 1,
            Dir::Nw | Dir::Se => 0,
            Dir::W | Dir::Sw => -1,
        }
    }

    pub const fn dr(self) -> i32 {
        match self {
            Dir::E | Dir::W => 0,
            Dir::Ne | Dir::Nw => -1,
            Dir::Sw | Dir::Se => 1,
        }
    }

    /// The opposite direction, $-d$.
    pub const fn opposite(self) -> Dir {
        match self {
            Dir::E => Dir::W,
            Dir::Ne => Dir::Sw,
            Dir::Nw => Dir::Se,
            Dir::W => Dir::E,
            Dir::Sw => Dir::Ne,
            Dir::Se => Dir::Nw,
        }
    }

    pub const fn as_coord(self) -> Coord {
        Coord::new(self.dq(), self.dr())
    }
}

/// Radius of the central hexagon.
pub const HEX_RADIUS: i32 = 4;

/// Is `c` in the central hexagon $H_4$? (Impl §2)
///
/// $$H_4 = \{(q,r) \in \mathbb{Z}^2 : |q| \le 4,\ |r| \le 4,\ |q+r| \le 4\}$$
///
/// All three constraints are needed: $|q+r| \le 4$ alone is an unbounded strip.
pub const fn in_hex(c: Coord) -> bool {
    c.q.abs() <= HEX_RADIUS && c.r.abs() <= HEX_RADIUS && (c.q + c.r).abs() <= HEX_RADIUS
}

/// Is `c` in camp $C_0$? (Impl §3)
///
/// $$C_0 = \{(q,r) : 5 \le q \le 8,\ -4 \le r \le -(q-4)\}$$
///
/// The triangle points **outward**: its four-hole base lies flush against the
/// hexagon edge $q = 4$ and its apex is the single hole $(8,-4)$.
pub const fn in_base_camp(c: Coord) -> bool {
    c.q >= 5 && c.q <= 8 && c.r >= -HEX_RADIUS && c.r <= -(c.q - HEX_RADIUS)
}

/// The camp definition the draft specification shipped, kept **only** so the
/// laws can demonstrate why it is wrong.
///
/// $$C_0^{\text{bad}} = \{(q,r) : 5 \le q \le 8,\ -q+5 \le r \le 0\}$$
///
/// It has ten holes and yields a 121-hole board, so every cardinality check
/// passes — but the triangle points *inward*, meeting the hexagon at the single
/// hole $(5,0)$ instead of along a four-hole edge. See
/// [`crate::laws::geometry::InwardCampIsDegenerate`].
pub const fn in_inward_camp(c: Coord) -> bool {
    c.q >= 5 && c.q <= 8 && c.r >= -c.q + 5 && c.r <= 0
}

/// Is `c` in camp $C_i = R^i(C_0)$? (Impl §4)
///
/// Implemented by rotating `c` *back* into $C_0$'s frame.
pub const fn in_camp(c: Coord, camp: u32) -> bool {
    in_base_camp(rotate_n(c, (6 - camp % 6) % 6))
}

/// Is `c` a playable hole? $V = H_4 \sqcup C_0 \sqcup \cdots \sqcup C_5$ (Impl §5)
pub const fn on_board(c: Coord) -> bool {
    in_hex(c)
        || in_camp(c, 0)
        || in_camp(c, 1)
        || in_camp(c, 2)
        || in_camp(c, 3)
        || in_camp(c, 4)
        || in_camp(c, 5)
}

/// Which camp contains `c`, if any.
pub const fn camp_of(c: Coord) -> Option<u32> {
    if in_camp(c, 0) {
        Some(0)
    } else if in_camp(c, 1) {
        Some(1)
    } else if in_camp(c, 2) {
        Some(2)
    } else if in_camp(c, 3) {
        Some(3)
    } else if in_camp(c, 4) {
        Some(4)
    } else if in_camp(c, 5) {
        Some(5)
    } else {
        None
    }
}

/// Coordinates outside this bound cannot be on the board, so proofs and
/// enumeration can be restricted to it.
pub const COORD_BOUND: i32 = 8;

/// Every playable hole, in sorted order.
pub fn all_holes() -> Vec<Coord> {
    let mut holes = Vec::with_capacity(121);
    for q in -COORD_BOUND..=COORD_BOUND {
        for r in -COORD_BOUND..=COORD_BOUND {
            let c = Coord::new(q, r);
            if on_board(c) {
                holes.push(c);
            }
        }
    }
    holes
}

// ---------------------------------------------------------------------------
// Kani proof harnesses.
//
// These run only under `cargo kani` (Linux/WSL; Kani does not build on Windows).
// They prove the geometry laws over every coordinate in the bounding box, which
// for these claims is the entire meaningful domain.
// ---------------------------------------------------------------------------

#[cfg(kani)]
mod proofs {
    use super::*;

    /// A symbolic coordinate, bounded so rotation arithmetic cannot overflow.
    fn any_coord() -> Coord {
        let q: i32 = kani::any();
        let r: i32 = kani::any();
        kani::assume(q >= -16 && q <= 16);
        kani::assume(r >= -16 && r <= 16);
        Coord::new(q, r)
    }

    fn any_camp() -> u32 {
        let i: u32 = kani::any();
        kani::assume(i < 6);
        i
    }

    fn any_dir() -> Dir {
        let i: u8 = kani::any();
        kani::assume(i < 6);
        match i {
            0 => Dir::E,
            1 => Dir::Ne,
            2 => Dir::Nw,
            3 => Dir::W,
            4 => Dir::Sw,
            _ => Dir::Se,
        }
    }

    /// `CC-GEO-ROT-ORDER`: $R$ has order **exactly** six.
    ///
    /// Asserting only $R^6 = \mathrm{id}$ is far too weak — the identity map,
    /// point reflection, and the $(q-r)$ sign-flip all satisfy it. The order
    /// must be pinned from below as well: no smaller power may be the identity.
    /// A witness coordinate is enough for the lower bound, since a map that
    /// fixes every point is the identity.
    #[kani::proof]
    fn rotation_has_order_exactly_six() {
        let c = any_coord();
        assert_eq!(rotate_n(c, 6), c, "R^6 must be the identity");

        // Lower bound: R^k is not the identity for 0 < k < 6.
        let w = Coord::new(1, 0);
        assert_ne!(rotate_n(w, 1), w, "R must not fix (1,0)");
        assert_ne!(rotate_n(w, 2), w, "R^2 must not fix (1,0)");
        assert_ne!(rotate_n(w, 3), w, "R^3 must not fix (1,0)");
        assert_ne!(rotate_n(w, 4), w, "R^4 must not fix (1,0)");
        assert_ne!(rotate_n(w, 5), w, "R^5 must not fix (1,0)");
    }

    /// `CC-GEO-ROT-STEP`: one rotation maps each direction to the next.
    ///
    /// This is what actually characterises $R$ as a $60^\circ$ rotation. Without
    /// it, $-\mathrm{id}$ passes every other rotation harness: it satisfies
    /// $R^3 = -\mathrm{id}$ (since $(-\mathrm{id})^3 = -\mathrm{id}$) and
    /// $R^6 = \mathrm{id}$, yet has order two and is not a rotation at all.
    #[kani::proof]
    fn rotation_permutes_directions_cyclically() {
        // R sends d_k to d_{k-1} in the ALL ordering (see the orientation note).
        let mut k = 0;
        while k < 6 {
            let d = Dir::ALL[k];
            let expect = Dir::ALL[(k + 5) % 6];
            let rotated = rotate60(d.as_coord());
            assert_eq!(rotated, expect.as_coord());
            k += 1;
        }
    }

    /// `CC-GEO-ROT-NEG`: $R^3 = -\mathrm{id}$ — why $C_{i+3}$ is opposite.
    #[kani::proof]
    fn three_rotations_is_negation() {
        let c = any_coord();
        assert_eq!(rotate_n(c, 3), c.negate());
    }

    /// `CC-GEO-HEX-CAMP-DISJOINT`: $H_4 \cap C_i = \varnothing$ for **every** $i$.
    ///
    /// Disjointness alone is vacuous — a camp predicate that is always false
    /// satisfies it. [`camps_and_hexagon_are_populated`] supplies the
    /// non-emptiness that makes this meaningful.
    ///
    /// Quantifying over a symbolic camp rather than only $C_0$ is load-bearing.
    /// Fault injection found that dropping the $|q+r| \le 4$ constraint — which
    /// turns the hexagon into an 81-hole rhombus — swallows $C_1$ and $C_4$
    /// whole while leaving $C_0$ untouched, and still yields $|V| = 121$ because
    /// the 20 holes the rhombus gains are exactly the 20 it absorbs. The
    /// $C_0$-only form of this harness, central symmetry, and the board-size
    /// count all passed on that board.
    #[kani::proof]
    fn hex_and_camps_are_disjoint() {
        let c = any_coord();
        let i = any_camp();
        assert!(!(in_hex(c) && in_camp(c, i)));
    }

    /// `CC-GEO-NONVACUOUS`: the regions are populated, with the right sizes.
    ///
    /// Every disjointness, symmetry, and opposite-camp harness in this module is
    /// satisfied by predicates that are identically `false`. Without a
    /// cardinality bound they prove nothing about a *board*. Counting closes
    /// that hole.
    #[kani::proof]
    fn camps_and_hexagon_are_populated() {
        let mut hex = 0;
        let mut camp = 0;
        let mut board = 0;

        let mut q = -COORD_BOUND;
        while q <= COORD_BOUND {
            let mut r = -COORD_BOUND;
            while r <= COORD_BOUND {
                let c = Coord::new(q, r);
                if in_hex(c) {
                    hex += 1;
                }
                if in_base_camp(c) {
                    camp += 1;
                }
                if on_board(c) {
                    board += 1;
                }
                r += 1;
            }
            q += 1;
        }

        assert_eq!(hex, 61, "the hexagon must have 61 holes");
        assert_eq!(camp, 10, "the base camp must have 10 holes");
        assert_eq!(board, 121, "the board must have 121 holes");
    }

    /// `CC-GEO-CAMP-SHAPE`: the base camp's columns hold 4, 3, 2, 1 holes.
    ///
    /// Pins the triangle's shape and orientation, not merely its size: a
    /// ten-hole region of any other shape fails here.
    #[kani::proof]
    fn base_camp_columns_decrease_outward() {
        let mut q = 5;
        while q <= 8 {
            let mut n = 0;
            let mut r = -COORD_BOUND;
            while r <= COORD_BOUND {
                if in_base_camp(Coord::new(q, r)) {
                    n += 1;
                }
                r += 1;
            }
            assert_eq!(n, 9 - q, "column q must hold 9-q holes");
            q += 1;
        }
        // The apex is the single outermost hole.
        assert!(in_base_camp(Coord::new(8, -4)));
    }

    /// `CC-GEO-CAMPS-DISJOINT`: $C_i \cap C_j = \varnothing$ for $i \neq j$, and
    /// every camp is inhabited.
    ///
    /// The inhabitation clause matters: disjointness on its own is satisfied by
    /// six empty camps.
    #[kani::proof]
    fn distinct_camps_are_disjoint() {
        let c = any_coord();
        let i = any_camp();
        let j = any_camp();
        kani::assume(i != j);
        assert!(!(in_camp(c, i) && in_camp(c, j)));

        // Each camp contains the image of the base camp's apex, so none is empty.
        let apex = Coord::new(8, -4);
        assert!(in_camp(rotate_n(apex, i), i), "camp i must be inhabited");
        assert!(in_camp(rotate_n(apex, j), j), "camp j must be inhabited");
    }

    /// `CC-GEO-SYMMETRY`: $V = -V$.
    #[kani::proof]
    fn board_is_centrally_symmetric() {
        let c = any_coord();
        assert_eq!(on_board(c), on_board(c.negate()));
    }

    /// `CC-GEO-OPPOSITE-CAMP`: $C_{(i+3) \bmod 6} = -C_i$.
    #[kani::proof]
    fn opposite_camp_is_point_reflection() {
        let c = any_coord();
        let i = any_camp();
        assert_eq!(in_camp(c, (i + 3) % 6), in_camp(c.negate(), i));
    }

    /// `CC-DIR-INVOLUTION`: $-d \in D$, and $d$'s opposite negates it.
    #[kani::proof]
    fn direction_opposite_negates() {
        let d = any_dir();
        let o = d.opposite();
        assert_eq!(o.dq(), -d.dq());
        assert_eq!(o.dr(), -d.dr());
        assert_eq!(o.opposite(), d);
    }

    /// `CC-JUMP-DISPLACEMENT`: a jump displaces by $2d \neq 0$, so a single hop
    /// never lands on its own origin.
    #[kani::proof]
    fn jump_never_lands_on_origin() {
        let c = any_coord();
        let d = any_dir();
        assert_ne!(c.jump_dest(d), c);
    }

    /// `CC-JUMP-MIDPOINT`: the jumped-over hole is the midpoint of the hop.
    #[kani::proof]
    fn jumped_hole_is_the_midpoint() {
        let c = any_coord();
        let d = any_dir();
        let mid = c.neighbour(d);
        let dest = c.jump_dest(d);
        assert_eq!(mid.q * 2, c.q + dest.q);
        assert_eq!(mid.r * 2, c.r + dest.r);
    }

    /// `CC-GEO-INWARD-DEGENERATE`: the draft's inward-pointing camp meets the
    /// hexagon at exactly one hole, $(5,0)$ — proof that cardinality checks
    /// cannot distinguish a star from this degenerate look-alike.
    #[kani::proof]
    fn inward_camp_touches_hexagon_only_at_one_hole() {
        let c = any_coord();
        kani::assume(in_inward_camp(c));

        let touches = in_hex(c.neighbour(Dir::E))
            || in_hex(c.neighbour(Dir::Ne))
            || in_hex(c.neighbour(Dir::Nw))
            || in_hex(c.neighbour(Dir::W))
            || in_hex(c.neighbour(Dir::Sw))
            || in_hex(c.neighbour(Dir::Se));

        if touches {
            assert_eq!(c, Coord::new(5, 0));
        }
    }

    /// `CC-GEO-OUTWARD-CONTACT`: by contrast, the correct camp's contact holes
    /// are exactly the four of its base column $q = 5$.
    #[kani::proof]
    fn outward_camp_contact_is_the_base_column() {
        // Count the camp's contact holes exactly. Asserting a *correlation*
        // such as `touches == (c.q == 5)` is not enough: the degenerate
        // inward camp satisfies that too, because its single contact hole is
        // also its only q==5 hole. The distinguishing property is the count.
        let mut contact_holes = 0;
        let mut contact_pairs = 0;
        let mut q = 5;
        while q <= 8 {
            let mut r = -8;
            while r <= 8 {
                let c = Coord::new(q, r);
                if in_base_camp(c) {
                    let mut n = 0;
                    let mut k = 0;
                    while k < 6 {
                        if in_hex(c.neighbour(Dir::ALL[k])) {
                            n += 1;
                        }
                        k += 1;
                    }
                    if n > 0 {
                        contact_holes += 1;
                    }
                    contact_pairs += n;
                }
                r += 1;
            }
            q += 1;
        }
        assert_eq!(contact_holes, 4);
        assert_eq!(contact_pairs, 8);
    }
}
