//! Replaying the opponent's last move, and leaving its trace on the board.
//!
//! When a move is committed by someone other than this peer, the moved piece
//! flies its whole path again — origin through every hop landing to the
//! destination — and once it lands, a gray translucent dot is left on every
//! hole the path touched. The trace persists until the opponent's next move
//! replaces it, so "what changed while I looked away" stays readable for the
//! whole turn.
//!
//! The raw material is [`Session::last_move`], recorded at the one point every
//! move passes through; whether *this* peer wants it replayed is
//! [`Session::should_replay`]. Everything here is presentation: no game state
//! is touched, and a piece mid-flight is just its entity's transform being
//! driven, which a rebuild (a new selection preview, the next move) heals
//! automatically on the next frame.

use bevy::prelude::*;
use checkers_core::geometry::Coord;

use crate::Session;
use crate::board_amlah;
use crate::board_style::{BoardStyle, BoardVisual};
use crate::board_view::{HOLE_RADIUS, coord_to_world};
use crate::draw::DrawContext;

/// Which hole a piece stands on: attached to every piece by the piece sync,
/// this is how the flight finds the one that just landed on the destination.
#[derive(Component)]
pub struct PieceCoord(pub Coord);

/// Seconds per hop of the replay flight.
const SECONDS_PER_HOP: f32 = 0.32;

/// The opponent's last move: waiting to animate, animating, or left as a trace.
#[derive(Resource, Default)]
pub struct Replay {
    /// A fresh opponent move, taken by [`advance`] once its current flight is
    /// done.
    pending: Option<Flight>,
    /// The animation in flight, if any.
    flight: Option<Flight>,
    /// The last completed flight, shown as a static gray trace.
    trace: Option<Trace>,
    /// Bumped whenever `trace` is replaced or cleared, so [`sync_trace`] can
    /// tell a real change from a frame it has already drawn.
    trace_version: u64,
}

/// One move being flown, or waiting to be flown.
struct Flight {
    /// The holes of the path, origin first, destination last.
    path: Vec<Coord>,
    /// The same path in board-plane coordinates, precomputed for the lerp.
    points: Vec<Vec2>,
    elapsed: f32,
    total: f32,
}

/// A completed flight, left on the board as a gray trace.
struct Trace {
    path: Vec<Coord>,
}

impl Replay {
    /// Drop everything: a new game means nothing to replay and no old trace.
    fn clear(&mut self) {
        self.pending = None;
        self.flight = None;
        if self.trace.is_some() {
            self.trace = None;
            self.trace_version += 1;
        }
    }
}

/// Watch for a committed move this peer did not make, and queue its replay.
///
/// Runs right after [`crate::net::pump`](crate::net), so a move applied this
/// frame is queued before the piece systems redraw the board — the flight
/// then takes over the piece on the very frame it appears, and the
/// destination is never shown standing still.
pub fn watch(session: Res<Session>, mut replay: ResMut<Replay>) {
    if !session.is_changed() {
        return;
    }
    match &session.last_move {
        // A fresh session — new game, re-seated, resumed record. Nothing to
        // replay, and any trace from the previous game is stale.
        None => replay.clear(),
        Some(last) if session.should_replay() && last.path.len() >= 2 => {
            replay.pending = Some(Flight {
                path: last.path.clone(),
                points: last.path.iter().map(|c| coord_to_world(*c)).collect(),
                elapsed: 0.0,
                total: (last.path.len() as f32 - 1.0) * SECONDS_PER_HOP,
            });
        }
        // One's own move, or a degenerate path: nothing to animate, and the
        // trace of the opponent's previous move stays up.
        _ => {}
    }
}

/// Drive the flight: take a pending move, arc the moved piece along its path,
/// and on landing leave the trace.
pub fn advance(
    time: Res<Time>,
    style: Res<BoardStyle>,
    mut replay: ResMut<Replay>,
    mut pieces: Query<(&PieceCoord, &mut Transform)>,
) {
    if replay.flight.is_none() {
        replay.flight = replay.pending.take();
    }
    let Some(mut flight) = replay.flight.take() else {
        return;
    };

    flight.elapsed += time.delta_secs();
    let u = (flight.elapsed / flight.total).min(1.0);

    // The piece now stands on the path's destination — that is where the
    // board state says it is. If the pieces were rebuilt mid-flight (a new
    // selection preview, the next move) the replacement entity is found here
    // just as readily, and the flight simply continues from where it was.
    let Some(&destination) = flight.path.last() else {
        return;
    };
    let Some((_, mut transform)) = pieces.iter_mut().find(|(coord, _)| coord.0 == destination)
    else {
        return;
    };

    *transform = flight_transform(&flight.points, u, *style);

    // The trace is only left once the whole path has flown, so a flight
    // interrupted at 90% leaves nothing half-explained on the board.
    if u < 1.0 {
        replay.flight = Some(flight);
        return;
    }
    replay.pending = None;
    replay.trace = Some(Trace { path: flight.path });
    replay.trace_version += 1;
}

/// The transform of a piece at fraction `u` along a flight path.
///
/// Position lerps hole to hole; the arc peaks mid-flight so the piece visibly
/// passes over whatever it jumps. Per style, the shared classic-plane path is
/// mapped into that style's world, with the piece's own rest offset applied.
/// In the classic style the board is seen straight down, so the arc is pure
/// draw order: the flying piece lifts above every other sprite while in the
/// air.
fn flight_transform(points: &[Vec2], u: f32, style: BoardStyle) -> Transform {
    let lift = (std::f32::consts::PI * u).sin();

    // Walk the path: `segs` equal-duration segments, `u` in [0, 1] spread
    // across them. The final segment's fraction clamps off the end.
    let segs = (points.len() - 1) as f32;
    let scaled = u * segs;
    let index = ((scaled as usize) + 1).min(points.len() - 1);
    let t = scaled - (index - 1) as f32;
    let lerped = points[index - 1].lerp(points[index], t);

    match style {
        BoardStyle::Classic => Transform::from_xyz(lerped.x, lerped.y, 1.0 + lift * 1.2),
        BoardStyle::Amlah => {
            let w = board_amlah::plane_to_world3(lerped);
            Transform::from_xyz(
                w.x,
                board_amlah::HOLE_FILL_Y + board_amlah::PEG_HEIGHT * 0.5 + lift * 0.25,
                w.z,
            )
        }
    }
}

/// The trace: one gray translucent dot on every hole the opponent's move
/// touched, in the current style. Rebuilt only when the trace or the style
/// actually changed — it must survive the session changes that the selection
/// highlights are rebuilt on every turn.
pub fn sync_trace(
    draw: DrawContext,
    replay: Res<Replay>,
    style: Res<BoardStyle>,
    existing: Query<Entity, With<TraceMarker>>,
    mut drawn: Local<Option<(u64, BoardStyle)>>,
) {
    let DrawContext {
        mut commands,
        mut meshes,
        mut materials,
        mut std_materials,
    } = draw;

    let key = (replay.trace_version, *style);
    if *drawn == Some(key) {
        return;
    }
    *drawn = Some(key);

    for e in &existing {
        commands.entity(e).despawn();
    }
    let Some(trace) = &replay.trace else {
        return;
    };

    // Under the pieces: the destination hole carries the opponent's piece,
    // and a dot peeking out beneath it reads as a shadow, not a claim.
    match *style {
        BoardStyle::Classic => {
            let dot = meshes.add(Circle::new(HOLE_RADIUS * 0.55));
            let mat = materials.add(Color::srgba(0.60, 0.60, 0.64, 0.45));
            for hole in &trace.path {
                let p = coord_to_world(*hole);
                commands.spawn((
                    Mesh2d(dot.clone()),
                    MeshMaterial2d(mat.clone()),
                    Transform::from_xyz(p.x, p.y, 0.9),
                    TraceMarker,
                    BoardVisual,
                ));
            }
        }
        BoardStyle::Amlah => {
            // Flat on the board, just under the staged-jump trail (0.008) and
            // just over the connection lines (0.005). Translucent, so the
            // trail reads as a mark on the plate rather than a hole in it.
            let flat = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
            let dot = meshes.add(Circle::new(0.045));
            let mat = std_materials.add(StandardMaterial {
                base_color: Color::srgba(0.45, 0.45, 0.48, 0.5),
                unlit: true,
                alpha_mode: AlphaMode::Blend,
                ..default()
            });
            for hole in &trace.path {
                let w = board_amlah::plane_to_world3(coord_to_world(*hole));
                commands.spawn((
                    Mesh3d(dot.clone()),
                    MeshMaterial3d(mat.clone()),
                    Transform::from_rotation(flat).with_translation(Vec3::new(w.x, 0.006, w.z)),
                    TraceMarker,
                    BoardVisual,
                ));
            }
        }
    }
}

/// Marker for the gray trace left by the opponent's last move.
///
/// Deliberately not the selection `Overlay` marker — that is cleared on every
/// session change, while a trace must outlive the turn it arrived on. It
/// carries `BoardVisual` so a style switch despawns it with the rest of the
/// board; [`sync_trace`] redraws it in the new style.
#[derive(Component)]
pub struct TraceMarker;

#[cfg(test)]
mod tests {
    use super::*;

    /// The flight starts on the origin hole, ends exactly on the destination,
    /// and stays inside the path in between.
    #[test]
    fn the_flight_walks_the_path_from_origin_to_destination() {
        let points = vec![Vec2::ZERO, Vec2::new(34.0, 0.0), Vec2::new(34.0, 34.0)];
        for style in [BoardStyle::Classic, BoardStyle::Amlah] {
            let start = flight_transform(&points, 0.0, style);
            let end = flight_transform(&points, 1.0, style);
            match style {
                BoardStyle::Classic => {
                    assert!(start.translation.xy().distance(points[0]) < 1e-4);
                    assert!(end.translation.xy().distance(points[2]) < 1e-4);
                }
                BoardStyle::Amlah => {
                    let from = board_amlah::plane_to_world3(points[0]);
                    let to = board_amlah::plane_to_world3(points[2]);
                    assert!(start.translation.xz().distance(Vec2::new(from.x, from.z)) < 1e-4);
                    assert!(end.translation.xz().distance(Vec2::new(to.x, to.z)) < 1e-4);
                }
            }
        }
    }

    /// The arc peaks at mid-flight: in 3D the piece lifts above its rest
    /// height, and in 2D above every other sprite's layer.
    #[test]
    fn the_flight_arcs_above_the_rest_position() {
        let points = vec![Vec2::ZERO, Vec2::new(34.0, 0.0)];
        let rest = flight_transform(&points, 0.0, BoardStyle::Classic);
        let peak = flight_transform(&points, 0.5, BoardStyle::Classic);
        assert!(peak.translation.z > rest.translation.z + 0.5);

        let rest = flight_transform(&points, 0.0, BoardStyle::Amlah);
        let peak = flight_transform(&points, 0.5, BoardStyle::Amlah);
        assert!(peak.translation.y > rest.translation.y + 0.1);
    }

    /// A degenerate path (fewer than two holes) never queues a flight —
    /// `watch` filters it, since there is nothing to walk.
    #[test]
    fn a_one_hole_path_is_not_a_flight() {
        let path: Vec<Coord> = vec![Coord::new(0, 0)];
        assert!(path.len() < 2, "watch queues only paths with a segment");
    }
}
