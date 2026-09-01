//! The amlah visualization: the board as three baked meshes under a fixed,
//! tilted 3D camera.
//!
//! Style borrowed from `../amlah`: a cream hex plate carrying six accent
//! camp triangles, with holes and connection lines in near-black on top, and
//! cone-shaped pieces. The board geometry — holes, adjacency, camp triangles
//! — is the shared one from [`crate::board_view`], because every Chinese
//! checkers board has it; this module contributes only the style: a palette,
//! a projection of the shared plane coordinates onto the XZ plane
//! ([`SCALE`]), layer offsets along Y, and three baked meshes.
//!
//! The whole static board is three meshes (plate+triangles, holes, lines),
//! mirroring amlah's combined-mesh design: three draw calls, no per-frame
//! work, versus the classic style's one entity per hole.

use std::f32::consts::TAU;

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

use crate::board_view::hole_points;

/// Classic-plane units (pixels) → this style's 3D units.
///
/// Chosen so a hole spacing lands on amlah's 0.28√3 ≈ 0.485, which lets every
/// other constant below be amlah's verbatim and the proportions match the
/// look the style was borrowed from.
pub const SCALE: f32 = 0.014_265;

/// Radius of a hole fill.
pub const HOLE_RADIUS: f32 = 0.07;
/// Radius of the dark ring around a hole.
const OUTLINE_RADIUS: f32 = 0.08;
const TRIANGLE_Y: f32 = -0.002;
pub const HOLE_FILL_Y: f32 = 0.004;
const HOLE_OUTLINE_Y: f32 = 0.002;
const EDGE_Y: f32 = 0.005;
const LINE_HALF_WIDTH: f32 = 0.005;
const HEX_BASE_Y: f32 = -0.04;
const HEX_BASE_THICKNESS: f32 = 0.08;

/// Cone pieces, sized like amlah's pegs: clearly smaller than the hole, so a
/// full camp still reads as individual pieces.
pub const PEG_RADIUS: f32 = 0.045;
pub const PEG_HEIGHT: f32 = 0.18;

/// Where the style's camera sits: high enough to frame the whole star, low
/// enough that the tilt reads as a table rather than a diagram.
pub const CAMERA_POS: Vec3 = Vec3::new(0.0, 11.5, 8.0);

/// Amlah's palette. Camp triangle *i* and player *i*'s pieces share a colour,
/// so a piece always sits on its own camp's accent.
pub const PLATE: Color = Color::srgb(0.98, 0.94, 0.82);
pub const ACCENTS: [Color; 6] = [
    Color::srgb(0.95, 0.15, 0.15),
    Color::srgb(0.1, 0.75, 0.1),
    Color::srgb(0.1, 0.45, 1.0),
    Color::srgb(1.0, 0.85, 0.1),
    Color::srgb(1.0, 0.35, 0.65),
    Color::srgb(0.55, 0.2, 1.0),
];
pub const INK: Color = Color::srgb(0.1, 0.1, 0.1);
/// Destination dots: green, per amlah's move markers — white would vanish on
/// the cream plate.
pub const MOVE_DOT: Color = Color::srgb(0.25, 0.9, 0.45);

/// A classic-plane point, projected onto the board plane (Y = 0).
pub fn plane_to_world3(p: Vec2) -> Vec3 {
    Vec3::new(p.x * SCALE, 0.0, -p.y * SCALE)
}

/// A point on the board plane, back to classic-plane units. Inverse of
/// [`plane_to_world3`] — this is what makes mouse picking one conversion
/// away from the same [`crate::board_view::world_to_coord`] the classic
/// style uses.
pub fn world3_to_plane(w: Vec3) -> Vec2 {
    Vec2::new(w.x, -w.z) / SCALE
}

/// The plate must comfortably cover the star, or tips poke off the edge.
///
/// A hexagon's boundary dips to its inradius — [`hex_radius`] × √3⁄2 —
/// halfway between corners, so that is the reach a tip can actually rely on.
fn hex_radius() -> f32 {
    let half = hole_points()
        .iter()
        .map(|p| p.length())
        .fold(0.0_f32, f32::max);
    half * SCALE * 1.5
}

fn to_linear(c: Color) -> [f32; 4] {
    c.to_linear().to_f32_array()
}

/// The plate and the six camp triangles as one vertex-coloured mesh:
/// hex top, hex bottom, hex sides, then the triangles floating just above.
pub fn build_surface_mesh(triangles: &[[Vec2; 3]; 6]) -> Mesh {
    let radius = hex_radius();
    let top_y = HEX_BASE_Y;
    let bottom_y = HEX_BASE_Y - HEX_BASE_THICKNESS;
    let step = std::f32::consts::FRAC_PI_3;
    let bg = to_linear(PLATE);

    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut colors: Vec<[f32; 4]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    let mut v = |p: [f32; 3], n: [f32; 3], u: [f32; 2], c: [f32; 4]| {
        let idx = positions.len() as u32;
        positions.push(p);
        normals.push(n);
        uvs.push(u);
        colors.push(c);
        idx
    };
    let mut tri = |a: u32, b: u32, c: u32| indices.extend([a, b, c]);

    // Top face (fan).
    let ctr = v([0.0, top_y, 0.0], [0.0, 1.0, 0.0], [0.5, 0.5], bg);
    let mut ring = Vec::new();
    for i in 0..6 {
        let a = step * i as f32;
        ring.push(v(
            [radius * a.cos(), top_y, radius * a.sin()],
            [0.0, 1.0, 0.0],
            [0.5 + 0.5 * a.cos(), 0.5 + 0.5 * a.sin()],
            bg,
        ));
    }
    for i in 0..6 {
        tri(ctr, ring[i], ring[(i + 1) % 6]);
    }

    // Bottom face (reversed winding).
    let ctr = v([0.0, bottom_y, 0.0], [0.0, -1.0, 0.0], [0.5, 0.5], bg);
    let mut ring = Vec::new();
    for i in 0..6 {
        let a = step * i as f32;
        ring.push(v(
            [radius * a.cos(), bottom_y, radius * a.sin()],
            [0.0, -1.0, 0.0],
            [0.5 + 0.5 * a.cos(), 0.5 + 0.5 * a.sin()],
            bg,
        ));
    }
    for i in 0..6 {
        tri(ctr, ring[(i + 1) % 6], ring[i]);
    }

    // Side quads.
    for i in 0..6 {
        let a0 = step * i as f32;
        let a1 = step * ((i + 1) % 6) as f32;
        let nx = (a0.cos() + a1.cos()) * 0.5;
        let nz = (a0.sin() + a1.sin()) * 0.5;
        let len = (nx * nx + nz * nz).sqrt();
        let (nx, nz) = (nx / len, nz / len);

        let a = v(
            [radius * a0.cos(), top_y, radius * a0.sin()],
            [nx, 0.0, nz],
            [i as f32 / 6.0, 1.0],
            bg,
        );
        let b = v(
            [radius * a1.cos(), top_y, radius * a1.sin()],
            [nx, 0.0, nz],
            [(i + 1) as f32 / 6.0, 1.0],
            bg,
        );
        let c = v(
            [radius * a1.cos(), bottom_y, radius * a1.sin()],
            [nx, 0.0, nz],
            [(i + 1) as f32 / 6.0, 0.0],
            bg,
        );
        let d = v(
            [radius * a0.cos(), bottom_y, radius * a0.sin()],
            [nx, 0.0, nz],
            [i as f32 / 6.0, 0.0],
            bg,
        );
        tri(a, b, c);
        tri(c, d, a);
    }

    // Camp triangles, one flat accent each.
    for (ti, tri_verts) in triangles.iter().enumerate() {
        let accent = to_linear(ACCENTS[ti]);
        let mut corners = [0_u32; 3];
        for (ci, p) in tri_verts.iter().enumerate() {
            let w = plane_to_world3(*p);
            corners[ci] = v([w.x, TRIANGLE_Y, w.z], [0.0, 1.0, 0.0], [0.5; 2], accent);
        }
        tri(corners[0], corners[1], corners[2]);
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// Every hole — dark outline ring plus fill — as one mesh.
pub fn build_holes_mesh(points: &[Vec2]) -> Mesh {
    let segments = 16_u32;
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    for (radius, y) in [(OUTLINE_RADIUS, HOLE_OUTLINE_Y), (HOLE_RADIUS, HOLE_FILL_Y)] {
        for &c in points {
            let w = plane_to_world3(c);
            let (cx, cz) = (w.x, w.z);
            let base = positions.len() as u32;
            positions.push([cx, y, cz]);
            normals.push([0.0, 1.0, 0.0]);
            uvs.push([0.5; 2]);
            for i in 0..segments {
                let angle = -TAU * i as f32 / segments as f32;
                positions.push([cx + radius * angle.cos(), y, cz + radius * angle.sin()]);
                normals.push([0.0, 1.0, 0.0]);
                uvs.push([0.5; 2]);
            }
            for i in 0..segments {
                let next = if i == segments - 1 {
                    base + 1
                } else {
                    base + 2 + i
                };
                indices.extend_from_slice(&[base, base + 1 + i, next]);
            }
        }
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// All connection lines as one mesh of thin quads (a `LineList` has no
/// thickness; amlah's quads are what actually read as drawn lines).
pub fn build_lines_mesh(edges: &[(Vec2, Vec2)]) -> Mesh {
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    for (i, &(from, to)) in edges.iter().enumerate() {
        let dir = (to - from).normalize();
        let perp = Vec2::new(-dir.y, dir.x) * LINE_HALF_WIDTH;
        let base = (i * 4) as u32;

        let a = plane_to_world3(from);
        let b = plane_to_world3(to);
        positions.extend([
            [a.x + perp.x, EDGE_Y, a.z + perp.y],
            [a.x - perp.x, EDGE_Y, a.z - perp.y],
            [b.x - perp.x, EDGE_Y, b.z - perp.y],
            [b.x + perp.x, EDGE_Y, b.z + perp.y],
        ]);
        normals.extend([[0.0, 1.0, 0.0]; 4]);
        uvs.extend([[0.0, 0.0]; 4]);
        indices.extend([base + 1, base, base + 2, base + 2, base, base + 3]);
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// Static per-style assets that should die with the style: the shared cone
/// mesh for pieces. Removed on every style switch, so the GPU asset is freed
/// once the pieces referencing it are rebuilt.
#[derive(Resource)]
pub struct AmlahAssets {
    pub cone: Handle<Mesh>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board_view::coord_to_world;
    use checkers_core::geometry::{Coord, all_holes};

    /// Distinct camps get distinct accents, or pieces could not be told apart.
    #[test]
    fn accents_are_distinct() {
        for (i, a) in ACCENTS.iter().enumerate() {
            for b in &ACCENTS[i + 1..] {
                assert_ne!(a.to_linear(), b.to_linear());
            }
        }
    }

    /// The picking conversion must invert the rendering projection exactly,
    /// or clicks would land on the wrong hole only in this style.
    #[test]
    fn plane_and_world3_round_trip() {
        for c in all_holes() {
            let p = coord_to_world(c);
            let back = world3_to_plane(plane_to_world3(p));
            assert!((back - p).length() < 1e-4, "{c:?} did not round-trip");
        }
    }

    /// The plate must extend past the star even at the hexagon's shallowest
    /// reach — the inradius, halfway between corners — or a tip could poke
    /// off the edge.
    #[test]
    fn the_plate_covers_the_star() {
        let tips = hole_points()
            .iter()
            .map(|p| p.length())
            .fold(0.0_f32, f32::max)
            * SCALE;
        assert!(
            hex_radius() * (3.0_f32.sqrt() / 2.0) > tips,
            "the plate's inradius must clear the star tips"
        );
    }

    /// The centre hole maps to the origin on the board plane, in 3D too.
    #[test]
    fn the_centre_hole_is_at_the_origin() {
        let w = plane_to_world3(coord_to_world(Coord::ORIGIN));
        assert!(w.xz().length() < 1e-5);
    }
}
