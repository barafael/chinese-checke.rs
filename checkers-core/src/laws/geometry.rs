//! Geometry laws: the board's construction.
//!
//! Laws marked [`Evidence::Proof`] have a matching `#[kani::proof]` harness in
//! [`crate::geometry`] that establishes them over the whole domain, not merely
//! for sampled inputs. Their `holds` implementations below re-check the same
//! claims in ordinary Rust, so `cargo test` exercises them on every platform —
//! Kani does not build on Windows.
//!
//! Laws marked [`Evidence::Exhaustive`] enumerate a finite domain directly.

use crate::geometry::{
    COORD_BOUND, Coord, Dir, all_holes, camp_of, in_base_camp, in_camp, in_hex, in_inward_camp,
    on_board, rotate_n, rotate60,
};
use crate::law::{Evidence, Law};
use crate::register_law;
use crate::spec::Chapter;

/// Coordinates in the bounding box: the domain the geometry laws range over.
fn bounded_coords() -> Vec<Coord> {
    let mut v = Vec::new();
    for q in -COORD_BOUND..=COORD_BOUND {
        for r in -COORD_BOUND..=COORD_BOUND {
            v.push(Coord::new(q, r));
        }
    }
    v
}

/// $R^6 = \mathrm{id}$
pub struct RotationOrderSix;

impl Law for RotationOrderSix {
    const ID: &'static str = "CC-GEO-ROT-ORDER";
    const STATEMENT: &'static str = r"\forall x \in \mathbb{Z}^2:\ R^6(x) = x";
    const CHAPTER: Chapter = Chapter::Rotation;
    const SUMMARY: &'static str = "The 60-degree rotation has order six.";
    /// In plain terms: Turn the whole board six times and every hole is back where it started.
    const NOTE: &'static str =
        "Turn the whole board six times and every hole is back where it started.";
    const EVIDENCE: Evidence = Evidence::Proof;
    type Subject = Coord;

    fn holds(c: &Coord) -> Result<(), String> {
        if rotate_n(*c, 6) == *c {
            Ok(())
        } else {
            Err(format!("R^6{c:?} = {:?}", rotate_n(*c, 6)))
        }
    }

    fn subjects() -> Vec<Coord> {
        bounded_coords()
    }
}
register_law!(RotationOrderSix, ROTATION_ORDER_SIX);

/// $R^3 = -\mathrm{id}$
pub struct RotationCubedIsNegation;

impl Law for RotationCubedIsNegation {
    const ID: &'static str = "CC-GEO-ROT-NEG";
    const STATEMENT: &'static str = r"\forall x \in \mathbb{Z}^2:\ R^3(x) = -x";
    const CHAPTER: Chapter = Chapter::Rotation;
    const SUMMARY: &'static str =
        "Three rotations equal point reflection, which is why camp i+3 is opposite camp i.";
    /// In plain terms: Turn three times and every hole lands on the hole directly opposite the centre.
    const NOTE: &'static str =
        "Turn three times and every hole lands on the hole directly opposite the centre.";
    const EVIDENCE: Evidence = Evidence::Proof;
    type Subject = Coord;

    fn holds(c: &Coord) -> Result<(), String> {
        if rotate_n(*c, 3) == c.negate() {
            Ok(())
        } else {
            Err(format!(
                "R^3{c:?} = {:?}, not {:?}",
                rotate_n(*c, 3),
                c.negate()
            ))
        }
    }

    fn subjects() -> Vec<Coord> {
        bounded_coords()
    }
}
register_law!(RotationCubedIsNegation, ROTATION_CUBED_IS_NEGATION);

/// $H_4 \cap C_i = \varnothing$ and $C_i \cap C_j = \varnothing$ for $i \neq j$
pub struct RegionsAreDisjoint;

impl Law for RegionsAreDisjoint {
    const ID: &'static str = "CC-GEO-DISJOINT";
    const STATEMENT: &'static str =
        r"V = H_4 \mathbin{\dot\cup} C_0 \mathbin{\dot\cup} \cdots \mathbin{\dot\cup} C_5";
    const CHAPTER: Chapter = Chapter::Board;
    const SUMMARY: &'static str =
        "The hexagon and the six camps are pairwise disjoint, so every hole is covered once.";
    /// In plain terms: Every hole belongs to exactly one region: the middle, or one camp.
    const NOTE: &'static str = "Every hole belongs to exactly one region: the middle, or one camp.";
    const EVIDENCE: Evidence = Evidence::Proof;
    type Subject = Coord;

    fn holds(c: &Coord) -> Result<(), String> {
        let in_h = in_hex(*c);
        let camps: Vec<u32> = (0..6).filter(|&i| in_camp(*c, i)).collect();

        if in_h && !camps.is_empty() {
            return Err(format!("{c:?} is in the hexagon and camp(s) {camps:?}"));
        }
        if camps.len() > 1 {
            return Err(format!("{c:?} is in multiple camps: {camps:?}"));
        }
        Ok(())
    }

    fn subjects() -> Vec<Coord> {
        bounded_coords()
    }
}
register_law!(RegionsAreDisjoint, REGIONS_ARE_DISJOINT);

/// $V = -V$
pub struct BoardIsCentrallySymmetric;

impl Law for BoardIsCentrallySymmetric {
    const ID: &'static str = "CC-GEO-SYMMETRY";
    const STATEMENT: &'static str = r"\forall x:\ x \in V \iff -x \in V";
    const CHAPTER: Chapter = Chapter::Board;
    const SUMMARY: &'static str = "The board is symmetric under point reflection through centre.";
    /// In plain terms: For every hole there is a matching hole straight across the centre.
    const NOTE: &'static str =
        "For every hole there is a matching hole straight across the centre.";
    const EVIDENCE: Evidence = Evidence::Proof;
    type Subject = Coord;

    fn holds(c: &Coord) -> Result<(), String> {
        if on_board(*c) == on_board(c.negate()) {
            Ok(())
        } else {
            Err(format!(
                "{c:?} on board = {}, but its negation = {}",
                on_board(*c),
                on_board(c.negate())
            ))
        }
    }

    fn subjects() -> Vec<Coord> {
        bounded_coords()
    }
}
register_law!(BoardIsCentrallySymmetric, BOARD_IS_CENTRALLY_SYMMETRIC);

/// $C_{(i+3) \bmod 6} = -C_i$
pub struct OppositeCampIsReflection;

impl Law for OppositeCampIsReflection {
    const ID: &'static str = "CC-GEO-OPPOSITE";
    const STATEMENT: &'static str = r"C_{(i+3) \bmod 6} = -C_i = \{-x : x \in C_i\}";
    const CHAPTER: Chapter = Chapter::Rotation;
    const SUMMARY: &'static str = "A player's target camp is the point reflection of their start.";
    /// In plain terms: The camp across the centre is the mirror image of your own camp.
    const NOTE: &'static str = "The camp across the centre is the mirror image of your own camp.";
    const EVIDENCE: Evidence = Evidence::Proof;
    type Subject = (Coord, u32);

    fn holds(&(c, i): &(Coord, u32)) -> Result<(), String> {
        if in_camp(c, (i + 3) % 6) == in_camp(c.negate(), i) {
            Ok(())
        } else {
            Err(format!(
                "{c:?} breaks the opposite-camp relation for camp {i}"
            ))
        }
    }

    fn subjects() -> Vec<(Coord, u32)> {
        bounded_coords()
            .into_iter()
            .flat_map(|c| (0..6).map(move |i| (c, i)))
            .collect()
    }
}
register_law!(OppositeCampIsReflection, OPPOSITE_CAMP_IS_REFLECTION);

/// $-d \in D$ for every $d \in D$
pub struct DirectionsCloseUnderNegation;

impl Law for DirectionsCloseUnderNegation {
    const ID: &'static str = "CC-DIR-INVOLUTION";
    const STATEMENT: &'static str = r"\forall d \in D:\ -d \in D \ \land\ -(-d) = d";
    const CHAPTER: Chapter = Chapter::Coordinates;
    const SUMMARY: &'static str = "Directions come in opposite pairs, so adjacency is symmetric.";
    /// In plain terms: Every direction has an opposite: you can always step back the way you came.
    const NOTE: &'static str =
        "Every direction has an opposite: you can always step back the way you came.";
    const EVIDENCE: Evidence = Evidence::Proof;
    type Subject = Dir;

    fn holds(d: &Dir) -> Result<(), String> {
        let o = d.opposite();
        if o.dq() != -d.dq() || o.dr() != -d.dr() {
            return Err(format!("{d:?}.opposite() = {o:?} does not negate it"));
        }
        if o.opposite() != *d {
            return Err(format!("{d:?} opposite is not an involution"));
        }
        Ok(())
    }

    fn subjects() -> Vec<Dir> {
        Dir::ALL.to_vec()
    }
}
register_law!(
    DirectionsCloseUnderNegation,
    DIRECTIONS_CLOSE_UNDER_NEGATION
);

/// A jump displaces by $2d$, so it never lands on its own origin.
pub struct JumpDisplacement;

impl Law for JumpDisplacement {
    const ID: &'static str = "CC-JUMP-DISPLACEMENT";
    const STATEMENT: &'static str =
        r"\forall x, d \in D:\ x + 2d \neq x \ \land\ 2(x+d) = x + (x+2d)";
    const CHAPTER: Chapter = Chapter::Jumps;
    const SUMMARY: &'static str =
        "A hop moves by twice a direction; the jumped hole is exactly the midpoint.";
    /// In plain terms: A jump lands exactly two holes away, with the jumped hole in the middle.
    const NOTE: &'static str =
        "A jump lands exactly two holes away, with the jumped hole in the middle.";
    const EVIDENCE: Evidence = Evidence::Proof;
    type Subject = (Coord, Dir);

    fn holds(&(c, d): &(Coord, Dir)) -> Result<(), String> {
        let dest = c.jump_dest(d);
        if dest == c {
            return Err(format!("jump from {c:?} via {d:?} returned to origin"));
        }
        let mid = c.neighbour(d);
        if mid.q * 2 != c.q + dest.q || mid.r * 2 != c.r + dest.r {
            return Err(format!("{mid:?} is not the midpoint of {c:?}->{dest:?}"));
        }
        Ok(())
    }

    fn subjects() -> Vec<(Coord, Dir)> {
        bounded_coords()
            .into_iter()
            .flat_map(|c| Dir::ALL.map(move |d| (c, d)))
            .collect()
    }
}
register_law!(JumpDisplacement, JUMP_DISPLACEMENT);

/// The board has exactly 121 holes: 61 + 6×10.
pub struct BoardCardinality;

impl Law for BoardCardinality {
    const ID: &'static str = "CC-GEO-CARDINALITY";
    const STATEMENT: &'static str =
        r"|V| = 61 + 6 \cdot 10 = 121,\quad |H_4| = 61,\quad |C_i| = 10";
    const CHAPTER: Chapter = Chapter::Board;
    const SUMMARY: &'static str = "Hexagon of 61 holes plus six camps of 10 gives 121 holes.";
    /// In plain terms: The board has 121 holes: 61 in the middle and 10 in each of the six camps.
    const NOTE: &'static str =
        "The board has 121 holes: 61 in the middle and 10 in each of the six camps.";
    const EVIDENCE: Evidence = Evidence::Exhaustive;
    type Subject = ();

    fn holds((): &()) -> Result<(), String> {
        let holes = all_holes();
        if holes.len() != 121 {
            return Err(format!("board has {} holes, expected 121", holes.len()));
        }
        let hex = holes.iter().filter(|c| in_hex(**c)).count();
        if hex != 61 {
            return Err(format!("hexagon has {hex} holes, expected 61"));
        }
        for i in 0..6 {
            let n = holes.iter().filter(|c| in_camp(**c, i)).count();
            if n != 10 {
                return Err(format!("camp {i} has {n} holes, expected 10"));
            }
        }
        Ok(())
    }

    fn subjects() -> Vec<()> {
        vec![()]
    }
}
register_law!(BoardCardinality, BOARD_CARDINALITY);

/// Each camp meets the hexagon in exactly eight adjacent pairs.
///
/// This is the law that distinguishes a genuine star from the draft's
/// look-alike, which cardinality alone cannot detect.
pub struct CampContactCount;

impl Law for CampContactCount {
    const ID: &'static str = "CC-GEO-CONTACT";
    const STATEMENT: &'static str =
        r"\forall i:\ \left|\{(x,y) \in C_i \times H_4 : y - x \in D\}\right| = 8";
    const CHAPTER: Chapter = Chapter::Camps;
    const SUMMARY: &'static str =
        "Each camp's four-hole base sits flush against a hexagon edge, giving eight contact pairs.";
    /// In plain terms: Each camp hugs the middle along exactly eight neighbouring pairs of holes.
    const NOTE: &'static str =
        "Each camp hugs the middle along exactly eight neighbouring pairs of holes.";
    const EVIDENCE: Evidence = Evidence::Exhaustive;
    type Subject = u32;

    fn holds(camp: &u32) -> Result<(), String> {
        let contacts = all_holes()
            .into_iter()
            .filter(|c| in_camp(*c, *camp))
            .flat_map(|c| Dir::ALL.map(move |d| c.neighbour(d)))
            .filter(|n| in_hex(*n))
            .count();
        if contacts == 8 {
            Ok(())
        } else {
            Err(format!(
                "camp {camp} has {contacts} hexagon contacts, expected 8"
            ))
        }
    }

    fn subjects() -> Vec<u32> {
        (0..6).collect()
    }
}
register_law!(CampContactCount, CAMP_CONTACT_COUNT);

/// The draft specification's inward-pointing camp is degenerate.
///
/// It satisfies every cardinality constraint yet meets the hexagon at the single
/// hole $(5,0)$, so the six camps hang off the corners instead of forming a star.
pub struct InwardCampIsDegenerate;

impl Law for InwardCampIsDegenerate {
    const ID: &'static str = "CC-GEO-INWARD-BAD";
    const STATEMENT: &'static str =
        r"\left|\{x \in C_0^{\text{bad}} : \exists d \in D,\ x + d \in H_4\}\right| = 1";
    const CHAPTER: Chapter = Chapter::Camps;
    const SUMMARY: &'static str =
        "The inward-pointing camp variant touches the hexagon at one hole, so it is not a star.";
    /// In plain terms: The wrong camp shape touches the middle at just one hole, and that is how you catch it.
    const NOTE: &'static str =
        "The wrong camp shape touches the middle at just one hole, and that is how you catch it.";
    const EVIDENCE: Evidence = Evidence::Proof;
    type Subject = Coord;

    fn holds(c: &Coord) -> Result<(), String> {
        if !in_inward_camp(*c) {
            return Ok(());
        }
        let touches = Dir::ALL.iter().any(|d| in_hex(c.neighbour(*d)));
        if touches && *c != Coord::new(5, 0) {
            return Err(format!("unexpected inward-camp contact hole {c:?}"));
        }
        Ok(())
    }

    fn subjects() -> Vec<Coord> {
        bounded_coords()
    }
}
register_law!(InwardCampIsDegenerate, INWARD_CAMP_IS_DEGENERATE);

/// Every hole belongs to the hexagon or exactly one camp, and `camp_of` agrees.
pub struct CampOfIsConsistent;

impl Law for CampOfIsConsistent {
    const ID: &'static str = "CC-GEO-CAMP-OF";
    const STATEMENT: &'static str = r"\forall x \in V:\ \mathrm{camp}(x) = i \iff x \in C_i,\quad \mathrm{camp}(x) = \bot \iff x \in H_4";
    const CHAPTER: Chapter = Chapter::Board;
    const SUMMARY: &'static str = "Camp lookup agrees with camp membership on every hole.";
    /// In plain terms: Asking which camp a hole is in always agrees with the camp definitions.
    const NOTE: &'static str =
        "Asking which camp a hole is in always agrees with the camp definitions.";
    const EVIDENCE: Evidence = Evidence::Exhaustive;
    type Subject = Coord;

    fn holds(c: &Coord) -> Result<(), String> {
        match camp_of(*c) {
            Some(i) => {
                if !in_camp(*c, i) {
                    return Err(format!("camp_of({c:?}) = {i} but not in that camp"));
                }
                if in_hex(*c) {
                    return Err(format!("{c:?} is in a camp and the hexagon"));
                }
            }
            None => {
                if (0..6).any(|i| in_camp(*c, i)) {
                    return Err(format!("camp_of({c:?}) = None but it is in a camp"));
                }
            }
        }
        Ok(())
    }

    fn subjects() -> Vec<Coord> {
        bounded_coords()
    }
}
register_law!(CampOfIsConsistent, CAMP_OF_IS_CONSISTENT);

/// The base camp is where the specification says it is.
pub struct BaseCampHoles;

impl Law for BaseCampHoles {
    const ID: &'static str = "CC-GEO-BASE-CAMP";
    const STATEMENT: &'static str =
        r"C_0 = \{(q,r) : 5 \le q \le 8,\ -4 \le r \le -(q-4)\},\quad \text{columns } 4,3,2,1";
    const CHAPTER: Chapter = Chapter::Camps;
    const SUMMARY: &'static str =
        "Camp 0 occupies columns q=5..8 with 4,3,2,1 holes, apex outward at (8,-4).";
    /// In plain terms: The home camp fills four columns of 4, 3, 2 and 1 holes, pointing outward.
    const NOTE: &'static str =
        "The home camp fills four columns of 4, 3, 2 and 1 holes, pointing outward.";
    const EVIDENCE: Evidence = Evidence::Exhaustive;
    type Subject = ();

    fn holds((): &()) -> Result<(), String> {
        for (q, expected) in [(5, 4), (6, 3), (7, 2), (8, 1)] {
            let n = (-8..=8).filter(|r| in_base_camp(Coord::new(q, *r))).count();
            if n != expected {
                return Err(format!("column q={q} has {n} holes, expected {expected}"));
            }
        }
        if !in_base_camp(Coord::new(8, -4)) {
            return Err("apex (8,-4) is not in the base camp".into());
        }
        Ok(())
    }

    fn subjects() -> Vec<()> {
        vec![()]
    }
}
register_law!(BaseCampHoles, BASE_CAMP_HOLES);

/// The degenerate camp's contact count, stated as a law so the prose in
/// [`crate::spec::Chapter::Camps`] cannot claim the wrong number.
///
/// A correct camp contributes eight camp-to-hexagon adjacent pairs across four
/// contact holes; the inward variant contributes one pair across one hole.
pub struct InwardCampContactCount;

impl Law for InwardCampContactCount {
    const ID: &'static str = "CC-GEO-INWARD-CONTACT";
    const STATEMENT: &'static str =
        r"\left|\{(x,y) \in C_0^{\text{bad}} \times H_4 : y - x \in D\}\right| = 1";
    const CHAPTER: Chapter = Chapter::Camps;
    const SUMMARY: &'static str =
        "The inward camp has one hexagon contact pair, against eight for a correct camp.";
    /// In plain terms: A proper camp meets the middle at four holes; the wrong shape meets it at one.
    const NOTE: &'static str =
        "A proper camp meets the middle at four holes; the wrong shape meets it at one.";
    const EVIDENCE: Evidence = Evidence::Exhaustive;
    type Subject = ();

    fn holds((): &()) -> Result<(), String> {
        let inward_pairs = bounded_coords()
            .into_iter()
            .filter(|c| in_inward_camp(*c))
            .flat_map(|c| Dir::ALL.map(move |d| c.neighbour(d)))
            .filter(|n| in_hex(*n))
            .count();
        if inward_pairs != 1 {
            return Err(format!(
                "inward camp has {inward_pairs} hexagon contact pairs, expected 1"
            ));
        }

        let inward_holes = bounded_coords()
            .into_iter()
            .filter(|c| in_inward_camp(*c))
            .filter(|c| Dir::ALL.iter().any(|d| in_hex(c.neighbour(*d))))
            .count();
        if inward_holes != 1 {
            return Err(format!(
                "inward camp has {inward_holes} contact holes, expected 1"
            ));
        }

        let outward_holes = bounded_coords()
            .into_iter()
            .filter(|c| in_base_camp(*c))
            .filter(|c| Dir::ALL.iter().any(|d| in_hex(c.neighbour(*d))))
            .count();
        if outward_holes != 4 {
            return Err(format!(
                "correct camp has {outward_holes} contact holes, expected 4"
            ));
        }
        Ok(())
    }

    fn subjects() -> Vec<()> {
        vec![()]
    }
}
register_law!(InwardCampContactCount, INWARD_CAMP_CONTACT_COUNT);

/// The rotation has order **exactly** six, not merely at most six.
///
/// Found by auditing the proof harnesses: `R^6 = id` alone is satisfied by the
/// identity map, by point reflection, and by the `(q-r)` sign flip. The order
/// must be bounded from below too.
pub struct RotationOrderIsExact;

impl Law for RotationOrderIsExact {
    const ID: &'static str = "CC-GEO-ROT-EXACT";
    const STATEMENT: &'static str = r"\text{ord}(R) = 6:\quad R^6 = \mathrm{id} \land \forall k \in \{1..5\}:\ R^k \neq \mathrm{id}";
    const CHAPTER: Chapter = Chapter::Rotation;
    const SUMMARY: &'static str =
        "The rotation has order exactly six: no smaller power is the identity.";
    /// In plain terms: Six turns bring every hole home, and no smaller number ever does.
    const NOTE: &'static str = "Six turns bring every hole home, and no smaller number ever does.";
    const EVIDENCE: Evidence = Evidence::Proof;
    type Subject = u32;

    fn holds(k: &u32) -> Result<(), String> {
        let w = Coord::new(1, 0);
        if *k == 6 {
            if rotate_n(w, 6) != w {
                return Err("R^6 is not the identity".into());
            }
        } else if rotate_n(w, *k) == w {
            return Err(format!("R^{k} fixes (1,0), so the order is below six"));
        }
        Ok(())
    }

    fn subjects() -> Vec<u32> {
        (1..=6).collect()
    }
}
register_law!(RotationOrderIsExact, ROTATION_ORDER_IS_EXACT);

/// One rotation advances each direction by one step, which is what makes $R$ a
/// $60^\circ$ rotation rather than some other order-six map.
pub struct RotationPermutesDirections;

impl Law for RotationPermutesDirections {
    const ID: &'static str = "CC-GEO-ROT-STEP";
    const STATEMENT: &'static str = r"\forall k:\ R(d_k) = d_{k-1 \bmod 6}";
    const CHAPTER: Chapter = Chapter::Rotation;
    const SUMMARY: &'static str = "Rotation maps each direction to its neighbour in the cycle.";
    /// In plain terms: One turn rotates each direction onto its neighbour around the cycle.
    const NOTE: &'static str =
        "One turn rotates each direction onto its neighbour around the cycle.";
    const EVIDENCE: Evidence = Evidence::Proof;
    type Subject = usize;

    fn holds(k: &usize) -> Result<(), String> {
        let d = Dir::ALL[*k];
        let expect = Dir::ALL[(*k + 5) % 6];
        let got = rotate60(d.as_coord());
        if got == expect.as_coord() {
            Ok(())
        } else {
            Err(format!("R({d:?}) = {got:?}, expected {expect:?}"))
        }
    }

    fn subjects() -> Vec<usize> {
        (0..6).collect()
    }
}
register_law!(RotationPermutesDirections, ROTATION_PERMUTES_DIRECTIONS);

/// The regions are inhabited and correctly sized.
///
/// Found by auditing: every disjointness, symmetry, and opposite-camp claim is
/// vacuously true of predicates that are identically false. Cardinality is what
/// makes the rest of the geometry laws say something about an actual board.
pub struct RegionsAreNonVacuous;

impl Law for RegionsAreNonVacuous {
    const ID: &'static str = "CC-GEO-NONVACUOUS";
    const STATEMENT: &'static str = r"|H_4| = 61 \land \forall i:\ |C_i| = 10 \land |V| = 121";
    const CHAPTER: Chapter = Chapter::Hexagon;
    const SUMMARY: &'static str =
        "The hexagon, each camp, and the board are inhabited with their stated sizes.";
    /// In plain terms: The regions are real: 61 middle holes, 10 per camp, 121 altogether.
    const NOTE: &'static str =
        "The regions are real: 61 middle holes, 10 per camp, 121 altogether.";
    const EVIDENCE: Evidence = Evidence::Proof;
    type Subject = ();

    fn holds((): &()) -> Result<(), String> {
        let coords = bounded_coords();

        let hex = coords.iter().filter(|c| in_hex(**c)).count();
        if hex != 61 {
            return Err(format!("hexagon has {hex} holes, expected 61"));
        }

        for i in 0..6 {
            let n = coords.iter().filter(|c| in_camp(**c, i)).count();
            if n != 10 {
                return Err(format!("camp {i} has {n} holes, expected 10"));
            }
        }

        let board = coords.iter().filter(|c| on_board(**c)).count();
        if board != 121 {
            return Err(format!("board has {board} holes, expected 121"));
        }
        Ok(())
    }

    fn subjects() -> Vec<()> {
        vec![()]
    }
}
register_law!(RegionsAreNonVacuous, REGIONS_ARE_NON_VACUOUS);
