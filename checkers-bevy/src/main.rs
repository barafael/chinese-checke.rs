//! Bevy front-end for the machine-checked Chinese Checkers rules.
//!
//! The rules live entirely in `checkers-core`; this crate renders a [`Game`] and
//! turns clicks into moves. Destinations are taken from the rules rather than
//! constructed, so the UI has no path to a position the rules disallow.
//!
//! The session state, staged-turn interaction, and networking glue live in the
//! library half of this crate ([`checkers_bevy`]) so the integration tests can
//! drive them without a window; this binary owns the schedule, the rendering,
//! and the input systems.
//!
//! # Self-validation
//!
//! Two levels, because they cost very different amounts:
//!
//! - **At startup**, [`verify_all`] runs the entire law registry. Worth its
//!   roughly one-second cost once, and the app refuses to draw a board that
//!   fails its own specification.
//! - **After every committed turn**, [`audit`] applies the position invariants
//!   to the live position. Linear in the number of holes, so it is safe per move.
//!
//! Running the full registry per move would be far too slow: every law
//! regenerates its own sample games, and it never inspects the caller's
//! position. That distinction is what [`checkers_core::audit`] exists for.

use bevy::prelude::*;
// Not in the prelude, unlike the rest of the window API.
use bevy::window::{Monitor, PrimaryMonitor};
use checkers_bevy::board_view::{
    BOARD_HALF_EXTENT, HOLE_RADIUS, HOLE_SPACING, PIECE_RADIUS, coord_to_world, world_to_coord,
};
use checkers_bevy::setup::Seating;
use checkers_bevy::{AppState, Selection, Session, audit, lobby, net};
use checkers_core::geometry::{all_holes, camp_of, on_board};
use checkers_core::law::{LAWS, verify_all};
use checkers_core::position::{Player, Position};
use checkers_core::rules::Outcome;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Chinese Checkers".into(),
                // Two thirds of the monitor's *work area*, centred, so the whole
                // window is on screen and nothing is under the taskbar.
                //
                // This is not cosmetic. The previous fixed 980px height exceeded
                // the 912px work area on this display, so the bottom ~70 logical
                // pixels were behind the taskbar — which is precisely where the
                // lobby's buttons are anchored. They rendered correctly the whole
                // time and were simply off-screen: a layout probe showed them at
                // y=2307 of a 2450px surface, correctly sized. A blank-looking
                // lobby with no visible controls was the symptom.
                //
                // The real size is set by `size_to_monitor` on the first frame,
                // since the monitor is not known until winit has created the
                // window. This is only the pre-resize backbuffer.
                resolution: (900u32, 700u32).into(),
                position: WindowPosition::Centered(MonitorSelection::Primary),
                resize_constraints: WindowResizeConstraints {
                    // Below this the board no longer fits and the camera starts
                    // zooming out; there is no reason to allow less.
                    min_width: 480.0,
                    min_height: 480.0,
                    ..default()
                },
                // Track the containing element rather than rendering at a fixed
                // 900x980 and letting CSS stretch the result, which distorts the
                // board. Safe here because `body` takes its size from the
                // viewport, not from the canvas — the feedback loop this field
                // warns about needs a parent sized by its children.
                fit_canvas_to_parent: true,
                ..default()
            }),
            ..default()
        }))
        .insert_resource(ClearColor(Color::srgb(0.09, 0.09, 0.11)))
        .init_resource::<Session>()
        .init_resource::<StatusVisible>()
        .init_state::<AppState>()
        .add_plugins(lobby::plugin)
        .add_systems(Startup, setup)
        // Not state-scoped: the lobby is the first thing shown, and it is the
        // screen whose buttons the old size hid.
        .add_systems(Update, size_to_monitor)
        .add_systems(
            OnEnter(AppState::InGame),
            (lobby::apply_seats, spawn_board).chain(),
        )
        .add_systems(
            Update,
            (
                handle_buttons,
                handle_clicks,
                handle_keys,
                toggle_status,
                fit_camera_to_window,
                // Drains the outbox and applies only host-sequenced moves, so
                // it must run after input and before the view syncs.
                net::pump,
                sync_pieces,
                sync_highlights,
                sync_status,
                sync_status_visibility,
                sync_buttons,
            )
                .chain()
                .run_if(in_state(AppState::InGame)),
        )
        .run();
}

// --- marker components -----------------------------------------------------

#[derive(Component)]
struct HoleMarker;

/// A rendered piece. Carries no data: pieces are rebuilt from the position
/// wholesale, so nothing needs to look up which hole an entity came from.
#[derive(Component)]
struct PieceMarker;

/// Anything drawn on top of the board that is rebuilt whenever the selection
/// changes: destination dots, the selection ring, and the staged jump trail.
///
/// One component rather than three, so the despawn query stays simple.
#[derive(Component)]
struct Overlay;

#[derive(Component)]
struct StatusText;

/// Whether the status panel is shown. Toggled with `T`.
///
/// Kept out of [`Session`] because it is a view preference, not game state:
/// folding it in would make every toggle look like a position change to the
/// `is_changed()` gates the sync systems rely on.
#[derive(Resource)]
struct StatusVisible(bool);

impl Default for StatusVisible {
    fn default() -> Self {
        Self(true)
    }
}

/// The two turn-control buttons.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum ControlButton {
    Confirm,
    Cancel,
}

/// Distinct, roughly colour-blind-safe hues for the six players.
fn player_colour(player: Player) -> Color {
    match player.index() {
        0 => Color::srgb(0.90, 0.35, 0.30),
        1 => Color::srgb(0.95, 0.72, 0.20),
        2 => Color::srgb(0.45, 0.78, 0.35),
        3 => Color::srgb(0.35, 0.72, 0.90),
        4 => Color::srgb(0.55, 0.50, 0.90),
        _ => Color::srgb(0.92, 0.92, 0.92),
    }
}

/// Size the window to two thirds of the monitor and centre it.
///
/// Runs after startup rather than in the `Window` descriptor because the monitor
/// is not known until winit has created the window; `Monitor` entities do not
/// exist before then. Runs once, so the player can freely resize afterwards.
///
/// Two thirds of the *full* monitor rather than of the work area, because
/// [`Monitor`] does not expose the work area — it has no notion of the taskbar.
/// That is still the fix for the occlusion bug: at 2/3 the window is well inside
/// the usable region, whereas the old fixed 980px height exceeded this display's
/// 912px work area and pushed the lobby's bottom-anchored buttons underneath the
/// taskbar.
///
/// The monitor reports physical pixels, so the logical size the window wants is
/// divided by the scale factor — 2.5 on this display. Skipping that would ask
/// for a window 2.5x too large.
fn size_to_monitor(
    mut windows: Query<&mut Window>,
    monitors: Query<&Monitor, With<PrimaryMonitor>>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let Ok(monitor) = monitors.single() else {
        return;
    };
    let Ok(mut window) = windows.single_mut() else {
        return;
    };
    *done = true;

    let scale = if monitor.scale_factor > 0.0 {
        monitor.scale_factor as f32
    } else {
        1.0
    };
    let logical = Vec2::new(
        monitor.physical_width as f32 / scale,
        monitor.physical_height as f32 / scale,
    );
    let wanted = logical * 2.0 / 3.0;

    window.resolution.set(wanted.x, wanted.y);
    window.position = WindowPosition::Centered(MonitorSelection::Primary);
    info!(
        "sized window to {}x{} logical ({}x{} monitor at {scale}x)",
        wanted.x.round(),
        wanted.y.round(),
        monitor.physical_width,
        monitor.physical_height
    );
}

/// Verify the specification and spawn the camera. Runs once, before the lobby:
/// the app refuses to show anything at all if its own laws do not hold.
fn setup(mut commands: Commands) {
    // Camera first. Verification takes roughly a second, and on wasm that runs
    // on the browser's only thread — spawning the camera afterwards meant the
    // tab painted nothing at all until the registry finished, which is
    // indistinguishable from a hung build.
    //
    // The default `WindowSize` scaling, deliberately.
    //
    // I first set `ScalingMode::AutoMin` over the board's extent, reasoning it
    // would fit the board to any canvas. It does the opposite: `AutoMin` and
    // `AutoMax` both *pin* the viewport to the given size in world units, so a
    // 434-unit viewport in a 900px window magnifies everything by the ratio —
    // and again by the display scale factor. The screenshot showed about a tenth
    // of the board, with the UI text blown up to match.
    //
    // `WindowSize` maps one world unit to one pixel, which is what
    // `HOLE_SPACING = 34` was chosen against. `fit_camera_to_window` handles the
    // only case this leaves open: a window too small for the board.
    commands.spawn(Camera2d);

    // The full law registry is worth its cost once, at startup.
    if let Err(violation) = verify_all() {
        panic!("the specification does not hold: {violation}");
    }
    audit(&Position::initial(), Seating::Six);
}

/// Spawn the board, status panel, and turn controls on entering the game.
fn spawn_board(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    let hole_mesh = meshes.add(Circle::new(HOLE_RADIUS));
    let hole_mat = materials.add(Color::srgb(0.22, 0.22, 0.26));
    let camp_mat = materials.add(Color::srgb(0.30, 0.30, 0.36));

    for c in all_holes() {
        let material = if camp_of(c).is_some() {
            camp_mat.clone()
        } else {
            hole_mat.clone()
        };
        let p = coord_to_world(c);
        commands.spawn((
            Mesh2d(hole_mesh.clone()),
            MeshMaterial2d(material),
            Transform::from_xyz(p.x, p.y, 0.0),
            HoleMarker,
        ));
    }

    commands.spawn((
        Text::new(""),
        TextFont {
            font_size: FontSize::Px(15.0),
            ..default()
        },
        TextColor(Color::srgb(0.85, 0.85, 0.88)),
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(10.0),
            left: Val::Px(12.0),
            ..default()
        },
        StatusText,
    ));

    // Turn controls, top right: Confirm then Cancel.
    commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            top: Val::Px(10.0),
            right: Val::Px(12.0),
            column_gap: Val::Px(8.0),
            ..default()
        })
        .with_children(|row| {
            for (which, label) in [
                (ControlButton::Confirm, "Confirm (Enter)"),
                (ControlButton::Cancel, "Cancel (Backspace)"),
            ] {
                row.spawn((
                    Button,
                    Node {
                        padding: UiRect::axes(Val::Px(12.0), Val::Px(7.0)),
                        // In Bevy 0.19 BorderRadius is a Node field, not a
                        // standalone component.
                        border_radius: BorderRadius::all(Val::Px(4.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.18, 0.18, 0.21)),
                    which,
                ))
                .with_child((
                    Text::new(label),
                    TextFont {
                        font_size: FontSize::Px(13.0),
                        ..default()
                    },
                    TextColor(Color::srgb(0.9, 0.9, 0.92)),
                ));
            }
        });
}

fn handle_buttons(
    interactions: Query<(&Interaction, &ControlButton), Changed<Interaction>>,
    mut session: ResMut<Session>,
) {
    for (interaction, which) in interactions.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match which {
            ControlButton::Confirm => session.confirm(),
            ControlButton::Cancel => session.cancel(),
        }
    }
}

fn handle_clicks(
    buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    controls: Query<&Interaction, With<ControlButton>>,
    mut session: ResMut<Session>,
) {
    if !buttons.just_pressed(MouseButton::Left) || session.game.is_over() {
        return;
    }
    // Do not treat a click on a control button as a board click.
    if controls.iter().any(|i| *i != Interaction::None) {
        return;
    }

    let Ok(window) = windows.single() else {
        return;
    };
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    let Ok((camera, cam_tf)) = cameras.single() else {
        return;
    };
    let Ok(world) = camera.viewport_to_world_2d(cam_tf, cursor) else {
        return;
    };

    let hole = world_to_coord(world);
    if !on_board(hole) || coord_to_world(hole).distance(world) > HOLE_SPACING * 0.5 {
        return;
    }

    // While staging a jump, a click is either the next hop or nothing: switching
    // pieces mid-turn would silently discard the staged hops.
    if session.is_jumping() {
        session.activate(hole);
        return;
    }

    let player = session.game.turn();
    if session.game.position().occupant(hole) == Some(player) {
        session.select(hole);
    } else if session.selected_hole().is_some() {
        session.activate(hole);
    }
}

fn handle_keys(keys: Res<ButtonInput<KeyCode>>, mut session: ResMut<Session>) {
    if keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::NumpadEnter) {
        session.confirm();
    }
    if keys.just_pressed(KeyCode::Backspace) {
        session.cancel();
    }
    if keys.just_pressed(KeyCode::KeyU) {
        session.undo_hop();
    }
    if keys.just_pressed(KeyCode::Escape) {
        session.clear_selection();
        session.message = "Selection cleared".into();
    }
    if keys.just_pressed(KeyCode::KeyR) {
        *session = Session::default();
        session.message = "New game".into();
    }
}

/// `T` toggles the status panel.
///
/// Separate from [`sync_status`], which early-outs unless the session changed:
/// folding the toggle in there would leave a keypress with no visible effect
/// until the player's next move.
fn toggle_status(keys: Res<ButtonInput<KeyCode>>, mut visible: ResMut<StatusVisible>) {
    if keys.just_pressed(KeyCode::KeyT) {
        visible.0 = !visible.0;
    }
}

/// Zoom out when the window is too small to show the whole board.
///
/// Only ever zooms *out*: at the default projection one world unit is one pixel,
/// which is the scale `HOLE_SPACING` was designed for, so enlarging the board in
/// a big window is not wanted. Scaling the projection rather than the entities
/// keeps `coord_to_world` in one place, and clicks stay correct because
/// `viewport_to_world_2d` applies the same projection.
fn fit_camera_to_window(
    windows: Query<&Window, Changed<Window>>,
    mut projections: Query<&mut Projection, With<Camera2d>>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let Ok(mut projection) = projections.single_mut() else {
        return;
    };
    let Projection::Orthographic(ortho) = &mut *projection else {
        return;
    };

    let needed = BOARD_HALF_EXTENT * 2.0;
    let available = Vec2::new(window.width(), window.height());
    // `scale` divides: >1 shows more world per pixel, i.e. zooms out.
    let shortfall = (needed / available).max_element();
    ortho.scale = shortfall.max(1.0);
}

fn sync_status_visibility(
    visible: Res<StatusVisible>,
    mut text: Query<&mut Visibility, With<StatusText>>,
) {
    if !visible.is_changed() {
        return;
    }
    let Ok(mut v) = text.single_mut() else {
        return;
    };
    *v = if visible.0 {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
}

/// Redraw pieces from the position being displayed.
///
/// Despawn-and-respawn rather than diffing: at 60 pieces it is not worth the
/// complexity, and it guarantees the view cannot drift from the model.
fn sync_pieces(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    existing: Query<Entity, With<PieceMarker>>,
    session: Res<Session>,
) {
    if !session.is_changed() {
        return;
    }
    let position = session.display_position();

    // Unconditional despawn-and-respawn. The previous early-out compared only
    // which holes were occupied, not by whom, and only ran when the session had
    // already changed — so it never actually skipped anything.
    for e in existing.iter() {
        commands.entity(e).despawn();
    }

    let mesh = meshes.add(Circle::new(PIECE_RADIUS));
    for &c in position.holes() {
        let Some(player) = position.occupant(c) else {
            continue;
        };
        let p = coord_to_world(c);
        commands.spawn((
            Mesh2d(mesh.clone()),
            MeshMaterial2d(materials.add(player_colour(player))),
            Transform::from_xyz(p.x, p.y, 1.0),
            PieceMarker,
        ));
    }
}

fn sync_highlights(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    stale: Query<Entity, With<Overlay>>,
    session: Res<Session>,
) {
    if !session.is_changed() {
        return;
    }
    for e in stale.iter() {
        commands.entity(e).despawn();
    }

    // Trail of the staged jump so far.
    if let Selection::Jumping { turn } = &session.selection {
        let dot = meshes.add(Circle::new(HOLE_RADIUS * 0.55));
        let mat = materials.add(Color::srgba(1.0, 0.85, 0.4, 0.55));
        for hole in turn.path() {
            let p = coord_to_world(*hole);
            commands.spawn((
                Mesh2d(dot.clone()),
                MeshMaterial2d(mat.clone()),
                Transform::from_xyz(p.x, p.y, 1.5),
                Overlay,
            ));
        }
    }

    // Ring around the selected piece; gold while a jump is staged.
    if let Some(sel) = session.selected_hole() {
        let ring = meshes.add(Annulus::new(PIECE_RADIUS + 2.0, PIECE_RADIUS + 5.0));
        let colour = if session.is_jumping() {
            Color::srgb(1.0, 0.82, 0.30)
        } else {
            Color::WHITE
        };
        let p = coord_to_world(sel);
        commands.spawn((
            Mesh2d(ring),
            MeshMaterial2d(materials.add(colour)),
            Transform::from_xyz(p.x, p.y, 2.0),
            Overlay,
        ));
    }

    // One hop ahead only.
    let dot = meshes.add(Circle::new(HOLE_RADIUS * 0.85));
    let mat = materials.add(Color::srgba(1.0, 1.0, 1.0, 0.8));
    for t in session.highlights() {
        let p = coord_to_world(t);
        commands.spawn((
            Mesh2d(dot.clone()),
            MeshMaterial2d(mat.clone()),
            Transform::from_xyz(p.x, p.y, 2.0),
            Overlay,
        ));
    }
}

/// Dim the controls when they would do nothing, so the staged state is legible.
fn sync_buttons(session: Res<Session>, mut buttons: Query<(&ControlButton, &mut BackgroundColor)>) {
    if !session.is_changed() {
        return;
    }
    for (which, mut bg) in buttons.iter_mut() {
        let enabled = match which {
            ControlButton::Confirm => session.can_confirm(),
            ControlButton::Cancel => session.selected_hole().is_some(),
        };
        bg.0 = if enabled {
            match which {
                ControlButton::Confirm => Color::srgb(0.20, 0.45, 0.28),
                ControlButton::Cancel => Color::srgb(0.45, 0.24, 0.24),
            }
        } else {
            Color::srgb(0.18, 0.18, 0.21)
        };
    }
}

fn sync_status(session: Res<Session>, mut text: Query<&mut Text, With<StatusText>>) {
    if !session.is_changed() {
        return;
    }
    let Ok(mut text) = text.single_mut() else {
        return;
    };

    let header = match session.game.outcome() {
        Some(Outcome::Winner(p)) => format!("Player {} wins!", p.index()),
        Some(Outcome::Draw) => "Draw: every player is blocked.".to_string(),
        None => format!("Player {}'s turn", session.game.turn().index()),
    };

    let staged = match &session.selection {
        Selection::Jumping { turn } => {
            // Take the reason from the error itself rather than restating it, so
            // the two cannot drift apart.
            let why = match turn.to_move() {
                Ok(_) => String::new(),
                Err(e) => format!(" - {e}"),
            };
            format!("  |  staging {} hop(s){why}", turn.hops())
        }
        _ => String::new(),
    };

    **text = format!(
        "{header}{staged}\n{}\n{} laws checked at startup  |  invariants checked each turn\n\
         Click a piece, then a highlighted hole. Jumps chain one hop at a time.\n\
         Enter confirms, Backspace cancels, U undoes a hop, R restarts, T hides this.",
        session.message,
        LAWS.len()
    );
}
