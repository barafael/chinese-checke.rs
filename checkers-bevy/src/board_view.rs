//! Mapping between board coordinates and screen space.
//!
//! This is the only place that knows how a [`Coord`] becomes a pixel. Keeping it
//! separate means the geometry in `checkers-core` stays engine-free, and the
//! round-trip can be tested without a window.

use bevy::prelude::*;
use checkers_core::geometry::{Coord, Dir, all_holes, on_board};
use checkers_core::position::Player;

/// Distance between adjacent hole centres, in pixels.
pub const HOLE_SPACING: f32 = 34.0;
/// Radius of a hole marker.
pub const HOLE_RADIUS: f32 = 7.0;
/// Radius of a piece.
pub const PIECE_RADIUS: f32 = 13.0;

/// The colour used to draw a player's pieces. Shared by the in-game board and
/// the menu's background bot race so the two can never disagree on a camp's
/// colour.
pub fn player_colour(player: Player) -> Color {
    match player.index() {
        0 => Color::srgb(0.90, 0.35, 0.30),
        1 => Color::srgb(0.95, 0.72, 0.20),
        2 => Color::srgb(0.45, 0.78, 0.35),
        3 => Color::srgb(0.35, 0.72, 0.90),
        4 => Color::srgb(0.55, 0.50, 0.90),
        _ => Color::srgb(0.92, 0.92, 0.92),
    }
}

/// Half-extent of the star in world units, plus room for the piece radius.
///
/// The star spans x ∈ [-204, 204] and y ∈ [-236, 236] at [`HOLE_SPACING`]; the
/// board is centred on the origin, so one half-extent describes both sides.
/// Measured from [`crate::board_view::coord_to_world`] over every hole rather
/// than derived by hand — see `the_board_fits_within_its_half_extent`.
pub const BOARD_HALF_EXTENT: Vec2 = Vec2::new(204.0 + PIECE_RADIUS, 236.0 + PIECE_RADIUS);

/// The whole board plus breathing room: what a camera should frame.
///
/// Cameras that fit the window (`ScalingMode::AutoMin`) guarantee at least
/// this many world units are visible, so the star fills the window up to a
/// tasteful margin without ever touching its edge, and its on-screen size
/// follows the window size.
pub const BOARD_FRAME: Vec2 = Vec2::new(
    BOARD_HALF_EXTENT.x * 2.0 * 1.12,
    BOARD_HALF_EXTENT.y * 2.0 * 1.12,
);

/// Axial to screen, using the pointy-top convention.
///
/// $x = s\,(q + r/2)$ and $y = -\tfrac{\sqrt{3}}{2} s\, r$ for hole spacing
/// $s$; the $y$ negation puts increasing `r` downward, matching how the board
/// is conventionally drawn while Bevy's world axes point up.
pub fn coord_to_world(c: Coord) -> Vec2 {
    Vec2::new(
        HOLE_SPACING * (c.q as f32 + 0.5 * c.r as f32),
        -HOLE_SPACING * 3.0_f32.sqrt() / 2.0 * c.r as f32,
    )
}

/// Screen to axial, rounding to the nearest hole.
///
/// Rounds in cube space, where the three coordinates sum to zero: round each,
/// then correct the one that drifted furthest. Rounding `q` and `r`
/// independently would pick the wrong hole near cell boundaries.
pub fn world_to_coord(p: Vec2) -> Coord {
    let rf = -2.0 * p.y / (HOLE_SPACING * 3.0_f32.sqrt());
    let qf = p.x / HOLE_SPACING - rf / 2.0;
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

// --- Shared board geometry --------------------------------------------------
//
// Every Chinese checkers board has the same 121 holes, the same adjacency
// lines, and the same six camp triangles; visualizations differ only in how
// they project and draw them. So this geometry is computed once here, in
// plane coordinates, and every style consumes it. Nothing in this section
// knows anything about any particular look.

/// Every hole centre, in plane coordinates.
pub fn hole_points() -> Vec<Vec2> {
    all_holes().iter().map(|c| coord_to_world(*c)).collect()
}

/// One segment per adjacency, deduplicated: the connection lines.
pub fn hole_edges() -> Vec<(Vec2, Vec2)> {
    let holes = all_holes();
    let mut out = Vec::new();
    for c in &holes {
        for d in Dir::ALL {
            let n = c.neighbour(d);
            if on_board(n) && (c.q, c.r) < (n.q, n.r) {
                out.push((coord_to_world(*c), coord_to_world(n)));
            }
        }
    }
    out
}

/// The six camp triangles, one per player, in plane coordinates.
///
/// A camp is an exact triangle of lattice holes, so its three corners are
/// the triple of its holes spanning the largest parallelogram. That is found
/// with an exact integer cross product in axial coordinates — the plane
/// mapping only scales areas, so the ranking there would be identical, minus
/// the rounding. C(10,3) = 120 combinations per camp, once per style spawn.
pub fn camp_triangles() -> [[Vec2; 3]; 6] {
    let mut out = [[Vec2::ZERO; 3]; 6];
    for (i, player) in Player::ALL.iter().enumerate() {
        let camp = player.start_camp();
        let n = camp.len();
        let mut corners = [camp[0], camp[1], camp[2]];
        let mut best = 0_i32;
        for a in 0..n {
            for b in a + 1..n {
                for c in b + 1..n {
                    let (p, q, r) = (camp[a], camp[b], camp[c]);
                    let area = ((q.q - p.q) * (r.r - p.r) - (q.r - p.r) * (r.q - p.q)).abs();
                    if area > best {
                        best = area;
                        corners = [p, q, r];
                    }
                }
            }
        }
        out[i] = [
            coord_to_world(corners[0]),
            coord_to_world(corners[1]),
            coord_to_world(corners[2]),
        ];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use checkers_core::geometry::all_holes;

    /// Cross product z-component of (b−a) × (c−a). Test-only: winding and
    /// enclosure checks below are the sole users.
    fn cross(a: Vec2, b: Vec2, c: Vec2) -> f32 {
        (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
    }

    /// [`BOARD_HALF_EXTENT`] must actually contain the board, or the camera-fit
    /// system will crop it. A hand-written constant is exactly the kind of thing
    /// that rots when `HOLE_SPACING` changes, so it is checked rather than
    /// trusted.
    #[test]
    fn the_board_fits_within_its_half_extent() {
        let mut worst = Vec2::ZERO;
        for c in all_holes() {
            let p = coord_to_world(c).abs();
            worst = worst.max(p);
        }

        assert!(
            worst.x + PIECE_RADIUS <= BOARD_HALF_EXTENT.x,
            "board reaches x={} (+{PIECE_RADIUS} for the piece) but the half-extent is {}",
            worst.x,
            BOARD_HALF_EXTENT.x
        );
        assert!(
            worst.y + PIECE_RADIUS <= BOARD_HALF_EXTENT.y,
            "board reaches y={} (+{PIECE_RADIUS} for the piece) but the half-extent is {}",
            worst.y,
            BOARD_HALF_EXTENT.y
        );

        // Not wastefully large either: a half-extent much bigger than the board
        // would leave the board small and the window mostly empty.
        assert!(
            worst.x + PIECE_RADIUS >= BOARD_HALF_EXTENT.x - 1.0,
            "the half-extent is loose by more than a pixel in x"
        );
    }

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

    /// The shared geometry every visualization consumes must be complete, and
    /// its adjacency lines must be real hole-to-hole steps.
    #[test]
    fn the_board_geometry_is_complete() {
        assert_eq!(hole_points().len(), 121);
        let edges = hole_edges();
        assert!(!edges.is_empty());
        for (a, b) in edges {
            assert!((a.distance(b) - HOLE_SPACING).abs() < 1e-3);
        }
    }

    /// Each camp triangle must be a real triangle whose three corners are
    /// camp holes, and it must cover every hole the player starts on —
    /// otherwise a style that paints the triangle would leave starting
    /// pieces sitting on bare board.
    #[test]
    fn camp_triangles_enclose_their_camp() {
        const EPS: f32 = 1e-2;
        for (i, player) in Player::ALL.iter().enumerate() {
            let camp = player.start_camp();
            let tri = camp_triangles()[i];

            // Corner order follows the cube directions, so either winding is
            // fine — but the sign test must know which one it got.
            let orient = cross(tri[0], tri[1], tri[2]);
            assert!(orient.abs() > 1.0, "camp {} collapsed", i);

            for corner in tri {
                assert!(
                    camp.iter().any(|c| coord_to_world(*c) == corner),
                    "camp {} corner {corner:?} is not a camp hole",
                    i
                );
            }

            for &hole in camp {
                let p = coord_to_world(hole);
                let mut inside = true;
                for (a, b) in [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])] {
                    let s = cross(a, b, p);
                    // On an edge the cross is zero up to rounding noise,
                    // which this epsilon absorbs; the coordinates run into
                    // the hundreds, so real straddling is far larger.
                    inside &= if orient > 0.0 { s >= -EPS } else { s <= EPS };
                }
                assert!(inside, "camp {} hole {hole:?} outside its triangle", i);
            }
        }
    }
}
