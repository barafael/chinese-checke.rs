//! Mapping between board coordinates and screen space.
//!
//! This is the only place that knows how a [`Coord`] becomes a pixel. Keeping it
//! separate means the geometry in `checkers-core` stays engine-free, and the
//! round-trip can be tested without a window.

use bevy::prelude::*;
use checkers_core::geometry::Coord;

/// Distance between adjacent hole centres, in pixels.
pub const HOLE_SPACING: f32 = 34.0;
/// Radius of a hole marker.
pub const HOLE_RADIUS: f32 = 7.0;
/// Radius of a piece.
pub const PIECE_RADIUS: f32 = 13.0;

/// Axial to screen, using the pointy-top convention.
///
/// $x = \sqrt{3}\,(q + r/2)$ and $y = -\tfrac{3}{2} r$; the $y$ negation puts
/// increasing `r` downward, matching how the board is conventionally drawn while
/// Bevy's world axes point up.
pub fn coord_to_world(c: Coord) -> Vec2 {
    let scale = HOLE_SPACING / 3.0_f32.sqrt();
    Vec2::new(
        scale * 3.0_f32.sqrt() * (c.q as f32 + c.r as f32 / 2.0),
        -scale * 1.5 * c.r as f32,
    )
}

/// Screen to axial, rounding to the nearest hole.
///
/// Rounds in cube space, where the three coordinates sum to zero: round each,
/// then correct the one that drifted furthest. Rounding `q` and `r`
/// independently would pick the wrong hole near cell boundaries.
pub fn world_to_coord(p: Vec2) -> Coord {
    let scale = HOLE_SPACING / 3.0_f32.sqrt();
    let qf = (p.x / (scale * 3.0_f32.sqrt())) - (-p.y / (scale * 1.5)) / 2.0;
    let rf = -p.y / (scale * 1.5);
    let sf = -qf - rf;

    let (mut q, mut r, s) = (qf.round(), rf.round(), sf.round());
    let (dq, dr, ds) = ((q - qf).abs(), (r - rf).abs(), (s - sf).abs());

    if dq > dr && dq > ds {
        q = -r - s;
    } else if dr > ds {
        r = -q - s;
    }

    Coord::new(q as i32, r as i32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use checkers_core::geometry::all_holes;

    /// Every hole must round-trip: a hole whose centre maps back to a different
    /// hole would make clicks land on the wrong cell.
    #[test]
    fn every_hole_round_trips() {
        for c in all_holes() {
            let back = world_to_coord(coord_to_world(c));
            assert_eq!(back, c, "{c:?} mapped to {back:?}");
        }
    }

    /// Small offsets around a centre still resolve to the same hole, so clicks
    /// near an edge are not ambiguous.
    #[test]
    fn nearby_offsets_resolve_to_the_same_hole() {
        let jitter = HOLE_SPACING * 0.3;
        for c in all_holes() {
            let centre = coord_to_world(c);
            for (dx, dy) in [(jitter, 0.0), (-jitter, 0.0), (0.0, jitter), (0.0, -jitter)] {
                let back = world_to_coord(centre + Vec2::new(dx, dy));
                assert_eq!(back, c, "{c:?} jittered by ({dx},{dy}) gave {back:?}");
            }
        }
    }

    /// Distinct holes never share a screen position.
    #[test]
    fn holes_do_not_collide_on_screen() {
        let holes = all_holes();
        for (i, a) in holes.iter().enumerate() {
            for b in &holes[i + 1..] {
                let d = coord_to_world(*a).distance(coord_to_world(*b));
                assert!(
                    d > HOLE_SPACING * 0.5,
                    "{a:?} and {b:?} are only {d} apart on screen"
                );
            }
        }
    }

    /// Adjacent holes are exactly one spacing apart, so the rendered board is a
    /// regular hex grid rather than a sheared one.
    #[test]
    fn adjacent_holes_are_one_spacing_apart() {
        use checkers_core::geometry::{Dir, on_board};

        for c in all_holes() {
            for d in Dir::ALL {
                let n = c.neighbour(d);
                if !on_board(n) {
                    continue;
                }
                let dist = coord_to_world(c).distance(coord_to_world(n));
                assert!(
                    (dist - HOLE_SPACING).abs() < 0.01,
                    "{c:?}->{n:?} is {dist} apart, expected {HOLE_SPACING}"
                );
            }
        }
    }

    #[test]
    fn the_centre_hole_is_at_the_origin() {
        assert_eq!(coord_to_world(Coord::ORIGIN), Vec2::ZERO);
    }
}
