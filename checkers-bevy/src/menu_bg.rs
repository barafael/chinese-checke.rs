//! The live bot race behind the main menu.
//!
//! The menu is a still card otherwise; this gives it a living board. Two
//! engines race a real two-player game in the background — the very game the
//! "Watch 2 Bots" button starts — sharing the driver and the pacing rules with
//! the watched demo. It is purely presentational: the world entities are
//! despawned on leaving the menu and never touch the real `Session`.
//!
//! Rendering is deliberately simple — the classic flat board, one material per
//! player — reused from the in-game geometry so the two boards cannot drift
//! apart. The in-game camera (created at startup) already exists across every
//! state, so nothing here owns a camera: the board is drawn into the same
//! world and the menu UI renders on top of it.

use bevy::prelude::*;
use std::time::Duration;

use checkers_ai::{Ai, AiConfig};
use checkers_core::geometry::{all_holes, camp_of};
use checkers_core::position::Player;
use checkers_core::rules::Outcome;

use crate::ai::{Action, AiPace};
use crate::board_view::{HOLE_RADIUS, PIECE_RADIUS, coord_to_world, player_colour};
use crate::setup::Seating;
use crate::{AppState, Session};

/// Marker for the persistent board (holes, camp rings). Despawned with the
/// whole background on leaving the menu.
#[derive(Component)]
struct BgBoard;

/// Marker for a piece. Pieces are rebuilt wholesale whenever the race moves, so
/// they are kept separate from the board for an easy despawn query.
#[derive(Component)]
struct BgPiece;

/// The whole race: its own game plus the engine and the driver's pacing. A
/// separate resource, deliberately not the shared `Session`/`Ai`/`AiPace`,
/// because this game is only ever watched and must not leak into a real one.
///
/// The background runs for exactly as long as the menu is open, then is thrown
/// away — so the race's pointers never advance while a real game is played.
#[derive(Resource)]
struct MenuDemo {
    session: Session,
    ai: Ai,
    pace: AiPace,
    /// Set whenever the piece meshes are out of date (a move was applied, or
    /// the race was (re)dealt). Cleared by [`sync_pieces`]; this is how the
    /// background avoids rebuilding sixty pieces every single frame.
    needs_redraw: bool,
}

impl Default for MenuDemo {
    fn default() -> Self {
        let mut session = Session::new(Seating::Two);
        session.deal_two();
        Self {
            session,
            ai: Ai::new(AiConfig {
                // Cheaper than the watched demo: this runs behind a static menu
                // for however long the menu is open, so it must stay light.
                budget: Duration::from_millis(30),
                max_depth: 8,
            }),
            pace: AiPace::default(),
            needs_redraw: true,
        }
    }
}

pub fn plugin(app: &mut App) {
    app.init_resource::<MenuDemo>()
        .add_systems(OnEnter(AppState::Menu), (reset_demo, spawn_board).chain())
        .add_systems(
            Update,
            (drive, sync_pieces)
                .chain()
                .run_if(in_state(AppState::Menu)),
        )
        .add_systems(OnExit(AppState::Menu), despawn);
}

/// Start a fresh race each time the menu is reached, so the board is lively
/// rather than the frozen middle of the last one.
fn reset_demo(mut demo: ResMut<MenuDemo>) {
    demo.session.deal_two();
    demo.ai.forget();
    demo.pace.reset();
    demo.needs_redraw = true;
}

/// Draw the empty star: one disc per hole, a faint ring marking each camp.
fn spawn_board(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    let hole = meshes.add(Circle::new(HOLE_RADIUS));
    let camp = materials.add(Color::srgb(0.30, 0.30, 0.36));
    let plain = materials.add(Color::srgb(0.22, 0.22, 0.26));
    // A translucent tint per camp so the two racing sides read from across the
    // room, dark enough that the menu card on top stays legible.
    let tints = [
        materials.add(Color::srgba(0.90, 0.35, 0.30, 0.20)),
        materials.add(Color::srgba(0.95, 0.72, 0.20, 0.20)),
        materials.add(Color::srgba(0.45, 0.78, 0.35, 0.20)),
        materials.add(Color::srgba(0.35, 0.72, 0.90, 0.20)),
        materials.add(Color::srgba(0.55, 0.50, 0.90, 0.20)),
        materials.add(Color::srgba(0.92, 0.92, 0.92, 0.14)),
    ];
    for c in all_holes() {
        let p = coord_to_world(c);
        let base = if camp_of(c).is_some() {
            camp.clone()
        } else {
            plain.clone()
        };
        commands.spawn((
            Mesh2d(hole.clone()),
            MeshMaterial2d(base),
            Transform::from_xyz(p.x, p.y, 0.0),
            BgBoard,
        ));
        if let Some(player) = camp_of(c) {
            commands.spawn((
                Mesh2d(hole.clone()),
                MeshMaterial2d(tints[player as usize].clone()),
                Transform::from_xyz(p.x, p.y, 0.01),
                BgBoard,
            ));
        }
    }
}

/// Let the two engines play one visible step of their race. Hops and the start
/// of a move are one action per frame; the pacing throttle (`MOVE_INTERVAL`)
/// is the driver's job, so we just tick the clock forward each frame.
fn drive(mut demo: ResMut<MenuDemo>, time: Res<Time>) {
    let now = time.elapsed();
    // Split the single deref into borrows of each field: `advance` needs all
    // three mutably, and the borrow checker cannot split through the `ResMut`.
    let action = {
        let MenuDemo {
            session, ai, pace, ..
        } = &mut *demo;
        pace.advance(session, ai, now)
    };
    match action {
        Action::Wait => {}
        Action::Hop(_) => {}
        Action::Pass => {
            demo.session.game.pass();
            demo.needs_redraw = true;
        }
        Action::Commit(mv) | Action::Play(mv) => {
            demo.session.game.play(&mv);
            demo.needs_redraw = true;
        }
        Action::Abandon(reason) if !demo.pace.result_logged => {
            demo.pace.result_logged = true;
            info!("menu background race abandoned: {reason}");
        }
        Action::Abandon(_) => {}
    }
    if demo.session.game.is_over() && !demo.pace.result_logged {
        demo.pace.result_logged = true;
        let line = match demo.session.game.outcome() {
            Some(Outcome::Winner(p)) => format!("player {}", p.index()),
            Some(Outcome::Resigned(p)) => format!("player {} resigned", p.index()),
            Some(Outcome::Draw) | None => "draw".to_string(),
        };
        info!("menu background race over: {line}");
    }
}

/// Rebuild the pieces from the background session's displayed position. Runs
/// after [`drive`] each frame, so a committed move repaints immediately.
fn sync_pieces(
    mut commands: Commands,
    mut demo: ResMut<MenuDemo>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    pieces: Query<Entity, With<BgPiece>>,
) {
    if !demo.needs_redraw {
        return;
    }
    demo.needs_redraw = false;
    for e in pieces.iter() {
        commands.entity(e).despawn();
    }
    let mesh = meshes.add(Circle::new(PIECE_RADIUS));
    let mats: Vec<_> = Player::ALL
        .iter()
        .map(|&p| materials.add(player_colour(p)))
        .collect();
    for &c in demo.session.display_position().holes() {
        let Some(player) = demo.session.display_position().occupant(c) else {
            continue;
        };
        let p = coord_to_world(c);
        commands.spawn((
            Mesh2d(mesh.clone()),
            MeshMaterial2d(mats[player.index() as usize].clone()),
            Transform::from_xyz(p.x, p.y, 1.0),
            BgPiece,
        ));
    }
}

fn despawn(
    mut commands: Commands,
    board: Query<Entity, With<BgBoard>>,
    pieces: Query<Entity, With<BgPiece>>,
) {
    for e in board.iter().chain(pieces.iter()) {
        commands.entity(e).despawn();
    }
}
