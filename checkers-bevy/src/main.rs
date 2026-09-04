//! Bevy front-end for the machine-checked Chinese Checkers rules.
//!
//! The rules live in `checkers-core`; this binary renders the [`Game`] and
//! turns clicks into moves, always taking destinations from the rules rather
//! than constructing them. Shared logic lives in [`checkers_bevy`] so the
//! tests can drive it headlessly.
//!
//! Self-validation runs at two costs: the full law registry once at startup
//! ([`verify_all`]), and the linear position audit after every move
//! ([`audit`]).

use bevy::camera::ScalingMode;
use bevy::prelude::*;
// Not in the prelude, unlike the rest of the window API.
use bevy::window::{Monitor, PrimaryMonitor};
use checkers_ai::{Ai, AiConfig};
use checkers_bevy::ai::{Action, AiPace};
use checkers_bevy::board_amlah;
use checkers_bevy::board_style::{
    self, AmlahCamera, BoardStyle, BoardVisual, ClassicCamera, OrbitCamera,
};
use checkers_bevy::board_view::{
    BOARD_FRAME, HOLE_RADIUS, HOLE_SPACING, PIECE_RADIUS, camp_triangles, coord_to_world,
    hole_edges, hole_points, player_colour, world_to_coord,
};
use checkers_bevy::menu::AiStrength;
use checkers_bevy::replay;
use checkers_bevy::setup::Seating;
use checkers_bevy::{
    AppState, Selection, Session, audit, format_round_duration, lobby, menu, menu_bg, net, record,
    web,
};
use checkers_core::geometry::{Coord, all_holes, camp_of, on_board};
use checkers_core::law::{LAWS, verify_all};
use checkers_core::position::{Player, Position};
use checkers_core::rules::Outcome;
use checkers_net::NetState;

fn main() {
    // Before anything else: on the web, keep the browser's right-click menu
    // from interrupting the 3D camera's right-drag orbit. No-op on native.
    web::prevent_context_menu();
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
        .init_resource::<BoardStyle>()
        .init_resource::<OrbitCamera>()
        .init_state::<AppState>()
        .init_resource::<AiEngine>()
        .init_resource::<AiPace>()
        .init_resource::<replay::Replay>()
        .add_plugins(lobby::plugin)
        .add_plugins(menu::plugin)
        .add_plugins(menu_bg::plugin)
        .add_systems(Startup, setup)
        // Not state-scoped: the lobby is the first thing shown, and it is the
        // screen whose buttons the old size hid.
        .add_systems(Update, (size_to_monitor, scale_ui_to_window))
        .add_systems(
            OnEnter(AppState::InGame),
            // Only the UI is spawned here. The board visuals are spawned by
            // `apply_style`, so entering the game and switching styles go
            // through one code path.
            (
                // A fresh engine at the chosen strength: the deal is where a
                // round's tuning is decided, and a new engine carries no
                // repetition memory from the last one.
                |mut engine: ResMut<AiEngine>,
                 mut pace: ResMut<AiPace>,
                 strength: Res<AiStrength>| {
                    engine.0 = Ai::new(AiConfig::strength(strength.0));
                    pace.reset();
                },
                lobby::apply_seats,
                spawn_ui,
            )
                .chain(),
        )
        .add_systems(
            Update,
            (
                // Bevy caps a chained tuple at twenty systems, and this chain
                // outgrew that. Split at the natural seam — input, styling,
                // and move sequencing versus the view-sync systems — and
                // chain the halves: order is preserved exactly.
                (
                    handle_buttons,
                    handle_clicks,
                    // The record viewer's keys. It holds `ResMut<ReplayView>`,
                    // so Bevy runs it only while a record is being walked
                    // through; the play keys stand down for exactly that time.
                    replay::handle_view_keys,
                    handle_keys,
                    ai_one_shot,
                    board_style::handle_style_key,
                    // Spawns the board for the current style on entry, and
                    // tears the old one down and builds the new one on `V`.
                    // Must run before the sync systems, which read the style
                    // to decide what kind of meshes pieces and highlights
                    // are.
                    apply_style,
                    toggle_status,
                    stamp_session_clock,
                    board_style::orbit_camera,
                    // The zoom the player chose stays proportional when the
                    // window changes shape: scale the radius by the ratio of
                    // fit distances, so the board keeps its framing at any
                    // size.
                    board_style::fit_orbit_to_window,
                    // Drains the outbox and applies only host-sequenced moves,
                    // so it must run after input and before the view syncs.
                    // The computer plays through the same outbox as a human:
                    // one sequencing path, no privileged moves.
                    ai_take_turn,
                    net::pump,
                    // Queue the opponent's move for its replay before the
                    // board is redrawn, so the flight takes over the piece on
                    // the very frame it lands.
                    replay::watch,
                )
                    .chain(),
                (
                    sync_pieces,
                    // The flight drives the landed piece's transform; it must
                    // see the rebuilt pieces first.
                    replay::advance,
                    sync_highlights,
                    // Gray trace of the opponent's last path. After the
                    // flight, so a completed animation paints its trace the
                    // same frame.
                    replay::sync_trace,
                    sync_status,
                    sync_status_visibility,
                    sync_buttons,
                    sync_turn_indicator,
                    sync_camp_indicator,
                    sync_game_over,
                )
                    .chain(),
            )
                .chain()
                .run_if(in_state(AppState::InGame)),
        )
        .run();
}

// --- marker components -----------------------------------------------------

// The draw bundle lives in the library, so the replay module's trace system
// shares it with these systems.
use checkers_bevy::draw::DrawContext;

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

/// The colour swatch naming the active home base.
#[derive(Component)]
struct TurnSwatch;

/// The label beside the swatch: whose base is active, and whether it is ours.
#[derive(Component)]
struct TurnText;

/// Entities of the game-over overlay, so it can be despawned on restart.
#[derive(Component)]
struct GameOverUi;

/// Board rings marking the active player's home camp.
#[derive(Component)]
struct CampMarker;

/// Whether the status panel is shown, toggled with `T`. A view preference,
/// not game state, so it lives outside [`Session`].
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
    /// Concede the round. Button-only by design: a resignation is a deliberate
    /// visit to a labelled control, not a key a stray finger finds.
    Resign,
    /// Save the round as a `.cchkrs` record.
    Save,
    /// Open a `.cchkrs` record and resume it.
    Open,
    /// Open a `.cchkrs` record and walk through it.
    Replay,
}

/// Size the window to two thirds of the monitor and centre it.
///
/// Runs after startup (winit does not know the monitor before then), once, in
/// logical pixels (the monitor reports physical ones). Two thirds of the
/// full monitor — [`Monitor`] has no notion of the work area — which keeps
/// the window clear of the taskbar.
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

/// Verify the specification and spawn the camera. Runs once, before the
/// lobby: the app refuses to show anything if its own laws do not hold.
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
    // `WindowSize` mapped one world unit to one pixel, which is what
    // `HOLE_SPACING = 34` was chosen against. It has one flaw: the board's
    // on-screen size was then fixed in pixels, tiny on a large monitor and
    // cropped in a small window. `fit_projection` replaces that: the camera
    // frames the board plus breathing room, so the board fills the window at
    // any size and any shape.
    //
    // Marked as the classic style's camera: `apply_style` despawns and
    // respawns it if the player ever switches visualizations. It must carry
    // `BoardVisual` for that, and the lobby needs a camera before the game
    // state exists, which is why `setup` spawns it rather than leaving it to
    // the style system.
    commands.spawn((Camera2d, fit_projection(), ClassicCamera, BoardVisual));

    // The full law registry is worth its cost once, at startup.
    if let Err(violation) = verify_all() {
        panic!("the specification does not hold: {violation}");
    }
    audit(&Position::initial(), Seating::Six);
}

/// The classic camera's framing: at least [`BOARD_FRAME`] world units visible,
/// keeping aspect ratio.
///
/// `AutoMin` guarantees the board always fits entirely — window too small and
/// the camera zooms out, window large and it zooms in until the frame is full —
/// so the board's on-screen size follows the window instead of being pinned in
/// pixels. The earlier `AutoMin` attempt that the `WindowSize` comment warns
/// about was replaced by this one constant framing; the magnified-text symptom
/// it describes came from pinning a *fixed* size, not from fitting a minimum.
fn fit_projection() -> Projection {
    Projection::Orthographic(OrthographicProjection {
        scaling_mode: ScalingMode::AutoMin {
            min_width: BOARD_FRAME.x,
            min_height: BOARD_FRAME.y,
        },
        ..OrthographicProjection::default_2d()
    })
}

/// Spawn the in-game UI: status panel and turn controls. Not marked
/// [`BoardVisual`]: the UI is the same in every visualization.
fn spawn_ui(mut commands: Commands) {
    // Bottom-left column: the active-base indicator (colour swatch + label)
    // above the status text.
    commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(10.0),
            left: Val::Px(12.0),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(3.0),
            ..default()
        })
        .with_children(|col| {
            col.spawn(Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(6.0),
                ..default()
            })
            .with_children(|row| {
                row.spawn((
                    Node {
                        width: Val::Px(11.0),
                        height: Val::Px(11.0),
                        border_radius: BorderRadius::all(Val::Px(2.0)),
                        ..default()
                    },
                    BackgroundColor(Color::NONE),
                    TurnSwatch,
                ));
                row.spawn((
                    Text::new(""),
                    TextFont {
                        font_size: FontSize::Px(15.0),
                        ..default()
                    },
                    TextColor(Color::srgb(0.85, 0.85, 0.88)),
                    TurnText,
                ));
            });
            col.spawn((
                Text::new(""),
                TextFont {
                    font_size: FontSize::Px(15.0),
                    ..default()
                },
                TextColor(Color::srgb(0.85, 0.85, 0.88)),
                StatusText,
            ));
        });

    // Turn controls, top right: Confirm, Cancel, Resign.
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
                (ControlButton::Resign, "Resign"),
                (ControlButton::Save, "Save"),
                (ControlButton::Open, "Open"),
                (ControlButton::Replay, "Replay"),
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

/// Tear down whatever visualization is on screen and build the current one.
///
/// This is the whole switch mechanism: board state is never touched, and the
/// sync systems rebuild pieces and highlights from the unchanged session in
/// the new style the same frame. A `Local` tracker rather than
/// `is_changed()` makes first entry deterministic.
fn apply_style(
    style: Res<BoardStyle>,
    mut applied: Local<Option<BoardStyle>>,
    visuals: Query<Entity, With<BoardVisual>>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut std_materials: ResMut<Assets<StandardMaterial>>,
) {
    if *applied == Some(*style) {
        return;
    }
    *applied = Some(*style);

    for e in &visuals {
        commands.entity(e).despawn();
    }
    // Dropped on every switch so the cone mesh dies with the style; respawned
    // below when the amlah board comes back.
    commands.remove_resource::<board_amlah::AmlahAssets>();

    match *style {
        BoardStyle::Classic => {
            commands.spawn((Camera2d, fit_projection(), ClassicCamera, BoardVisual));
            spawn_classic_board(&mut commands, &mut meshes, &mut materials);
        }
        BoardStyle::Amlah => {
            // Spawned at the fixed camera; `orbit_camera` repositions it to the
            // player's remembered orbit every frame, so switching back to the
            // 3D style restores the last view rather than jerking to a default.
            commands.spawn((
                Camera3d::default(),
                Transform::from_translation(board_amlah::CAMERA_POS)
                    .looking_at(Vec3::ZERO, Vec3::Y),
                AmlahCamera,
                BoardVisual,
            ));
            spawn_amalah_board(&mut commands, &mut meshes, &mut std_materials);
        }
    }
}

/// The classic board: one entity per hole, exactly as it has always been.
fn spawn_classic_board(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
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
            BoardVisual,
        ));
    }
}

/// The amlah board: three baked meshes — plate with accent triangles, holes,
/// connection lines — and the shared cone mesh for pieces.
fn spawn_amalah_board(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    // The surface mesh carries its colours per-vertex (cream plate, six
    // accent camps), so the material stays white and multiplies through.
    // Culling is off: the camp corner order is whichever the cube-direction
    // picks produce, and one flat unlit material is cheaper than fussing
    // over winding.
    let surface_mat = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        unlit: true,
        cull_mode: None,
        ..default()
    });
    let ink_mat = materials.add(StandardMaterial {
        base_color: board_amlah::INK,
        unlit: true,
        ..default()
    });

    let triangles = camp_triangles();
    commands.spawn((
        Mesh3d(meshes.add(board_amlah::build_surface_mesh(&triangles))),
        MeshMaterial3d(surface_mat),
        BoardVisual,
    ));

    let holes = hole_points();
    commands.spawn((
        Mesh3d(meshes.add(board_amlah::build_holes_mesh(&holes))),
        MeshMaterial3d(ink_mat.clone()),
        BoardVisual,
    ));

    let edges = hole_edges();
    commands.spawn((
        Mesh3d(meshes.add(board_amlah::build_lines_mesh(&edges))),
        MeshMaterial3d(ink_mat),
        BoardVisual,
    ));

    let cone = meshes.add(Cone {
        radius: board_amlah::PEG_RADIUS,
        height: board_amlah::PEG_HEIGHT,
    });
    commands.insert_resource(board_amlah::AmlahAssets { cone });
}

fn handle_buttons(
    interactions: Query<(&Interaction, &ControlButton), Changed<Interaction>>,
    mut session: ResMut<Session>,
    viewer: Option<Res<replay::ReplayView>>,
    mut commands: Commands,
) {
    // The viewer hides the controls, so this cannot fire — but if it ever
    // did, a click must not rewrite the derived session under the cursor.
    if viewer.is_some() {
        return;
    }
    for (interaction, which) in interactions.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        // On another player's move the controls are inert — the click would
        // otherwise confirm or cancel a selection this peer cannot touch.
        if !session.may_act() {
            continue;
        }
        match which {
            ControlButton::Confirm => session.confirm(),
            ControlButton::Cancel => session.cancel(),
            ControlButton::Resign => session.resign(),
            ControlButton::Save => {
                let text = session.to_record().to_text();
                match web::save_record(&text) {
                    Ok(where_) => session.message = format!("Saved{where_}"),
                    Err(e) => session.message = format!("Save failed: {e}"),
                }
            }
            ControlButton::Open => match web::load_record() {
                Ok(text) => match record::GameRecord::from_text(&text)
                    .and_then(|rec| Session::resumed(&rec))
                {
                    Ok(resumed) => {
                        let note = if session.game.is_over() || !session.history().is_empty() {
                            " (the previous round is gone)"
                        } else {
                            ""
                        };
                        *session = resumed;
                        session.message = format!("Resumed{note}");
                    }
                    Err(f) => session.message = format!("Could not resume: {f}"),
                },
                Err(e) => session.message = format!("Open failed: {e}"),
            },
            ControlButton::Replay => match web::load_record() {
                Ok(text) => match record::GameRecord::from_text(&text) {
                    Ok(rec) => {
                        let n = rec.moves.len();
                        match Session::resumed_prefix(&rec, n) {
                            Ok(s) => {
                                *session = s;
                                commands.insert_resource(replay::ReplayView::at_end(rec));
                                session.message = format!(
                                    "Replay: move {n} of {n} - arrows step, Space autoplay, Esc back"
                                );
                            }
                            Err(f) => session.message = format!("Could not replay: {f}"),
                        }
                    }
                    Err(f) => session.message = format!("Could not open: {f}"),
                },
                Err(e) => session.message = format!("Replay failed: {e}"),
            },
        }
    }
}

fn handle_clicks(
    buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    cameras: Query<(&Camera, &GlobalTransform, Option<&AmlahCamera>)>,
    controls: Query<&Interaction, With<ControlButton>>,
    mut session: ResMut<Session>,
    viewer: Option<Res<replay::ReplayView>>,
) {
    // The record viewer's board is read-only: clicks move nothing.
    if viewer.is_some() || !buttons.just_pressed(MouseButton::Left) || session.game.is_over() {
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
    let Ok((camera, cam_tf, amlah_cam)) = cameras.single() else {
        return;
    };

    // Each style projects the cursor onto the classic *plane* — the shared
    // coordinate space of `board_view` — so the hole resolution and the
    // distance check below are identical for both. The classic camera gets
    // there in one step; the amlah camera intersects the cursor ray with the
    // board plane (Y = 0) and converts back.
    let plane = if amlah_cam.is_some() {
        let Ok(ray) = camera.viewport_to_world(cam_tf, cursor) else {
            return;
        };
        if ray.direction.y.abs() < 1e-6 {
            return;
        }
        let t = -ray.origin.y / ray.direction.y;
        if t < 0.0 {
            return;
        }
        board_amlah::world3_to_plane(ray.origin + *ray.direction * t)
    } else {
        let Ok(world) = camera.viewport_to_world_2d(cam_tf, cursor) else {
            return;
        };
        world
    };

    let hole = world_to_coord(plane);
    if !on_board(hole) || coord_to_world(hole).distance(plane) > HOLE_SPACING * 0.5 {
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

fn handle_keys(
    keys: Res<ButtonInput<KeyCode>>,
    mut session: ResMut<Session>,
    viewer: Option<Res<replay::ReplayView>>,
) {
    // The record viewer owns the keyboard while it is up.
    if viewer.is_some() {
        return;
    }
    if (keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::NumpadEnter))
        && session.may_act()
    {
        // Someone else's turn: confirmation is inert so this peer cannot submit a
        // move on another player's behalf.
        session.confirm();
    }
    if keys.just_pressed(KeyCode::Backspace) && session.may_act() {
        session.cancel();
    }
    if keys.just_pressed(KeyCode::KeyU) && session.may_act() {
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
fn toggle_status(keys: Res<ButtonInput<KeyCode>>, mut visible: ResMut<StatusVisible>) {
    if keys.just_pressed(KeyCode::KeyT) {
        visible.0 = !visible.0;
    }
}

/// Stamp the session's clock once per round. A replaced session arrives with
/// `started_at: None`, so the next frame re-stamps it; nothing else writes
/// the field. Bevy's clock, not the wall clock, so this works on wasm.
fn stamp_session_clock(
    mut session: ResMut<Session>,
    time: Res<Time>,
    viewer: Option<Res<replay::ReplayView>>,
) {
    // A viewer session is rebuilt per step; stamping it would make the
    // game-over card report the viewer's age, not the round's length.
    if viewer.is_some() {
        return;
    }
    session.stats.note_started(time.elapsed());
}

/// Scale the interface with the window, so menus, fields, and the status panel
/// use the available space rather than being pinned to the 900x700 window the
/// widgets were laid out against. Tiny windows shrink the text but keep it
/// legible; large monitors grow everything so the UI does not huddle in a
/// corner of an otherwise empty screen.
///
/// One resource moves the whole interface: `UiScale` multiplies every fixed
/// `px` value, and Bevy rasterises text at the scaled size, so it stays crisp.
/// Bounds are a taste judgement — below the floor the layout would collapse,
/// above the ceiling a full-screen lobby reads like a billboard.
fn scale_ui_to_window(windows: Query<&Window, Changed<Window>>, mut scale: ResMut<UiScale>) {
    let Ok(window) = windows.single() else {
        return;
    };
    let factor = (window.width() / 900.0).min(window.height() / 700.0);
    scale.0 = factor.clamp(0.65, 1.5);
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

/// Redraw pieces from the position being displayed, despawn-and-respawn so
/// the view cannot drift from the model. Runs on a style change as well as a
/// session change, which is what makes `V` rebuild pieces without a move.
fn sync_pieces(
    draw: DrawContext,
    amlah: Option<Res<board_amlah::AmlahAssets>>,
    existing: Query<Entity, With<PieceMarker>>,
    session: Res<Session>,
    style: Res<BoardStyle>,
) {
    let DrawContext {
        mut commands,
        mut meshes,
        mut materials,
        mut std_materials,
    } = draw;
    if !session.is_changed() && !style.is_changed() {
        return;
    }
    let position = session.display_position();

    // Unconditional despawn-and-respawn. The previous early-out compared only
    // which holes were occupied, not by whom, and only ran when the session had
    // already changed — so it never actually skipped anything.
    for e in existing.iter() {
        commands.entity(e).despawn();
    }

    match *style {
        BoardStyle::Classic => {
            let mesh = meshes.add(Circle::new(PIECE_RADIUS));
            // One material per player rather than per piece: the rebuild runs
            // on every committed turn, and 60 fresh handles are waste for six
            // colours.
            let mats: Vec<_> = Player::ALL
                .iter()
                .map(|&p| materials.add(player_colour(p)))
                .collect();
            for &c in position.holes() {
                let Some(player) = position.occupant(c) else {
                    continue;
                };
                let p = coord_to_world(c);
                commands.spawn((
                    Mesh2d(mesh.clone()),
                    MeshMaterial2d(mats[player.index() as usize].clone()),
                    Transform::from_xyz(p.x, p.y, 1.0),
                    PieceMarker,
                    replay::PieceCoord(c),
                ));
            }
        }
        BoardStyle::Amlah => {
            // `apply_style` runs earlier in the chain and inserted this when
            // it built the amlah board; if it is somehow missing, skip rather
            // than panic — the next change rebuilds again.
            let Some(assets) = amlah else {
                return;
            };
            let mats: Vec<_> = Player::ALL
                .iter()
                .map(|&p| {
                    std_materials.add(StandardMaterial {
                        base_color: board_amlah::ACCENTS[p.index() as usize],
                        unlit: true,
                        ..default()
                    })
                })
                .collect();
            for &c in position.holes() {
                let Some(player) = position.occupant(c) else {
                    continue;
                };
                let w = board_amlah::plane_to_world3(coord_to_world(c));
                commands.spawn((
                    Mesh3d(assets.cone.clone()),
                    MeshMaterial3d(mats[player.index() as usize].clone()),
                    // Cone geometry is centred; the base sits on the holes.
                    Transform::from_xyz(
                        w.x,
                        board_amlah::HOLE_FILL_Y + board_amlah::PEG_HEIGHT * 0.5,
                        w.z,
                    ),
                    PieceMarker,
                    replay::PieceCoord(c),
                ));
            }
        }
    }
}

fn sync_highlights(
    draw: DrawContext,
    stale: Query<Entity, With<Overlay>>,
    session: Res<Session>,
    style: Res<BoardStyle>,
) {
    let DrawContext {
        mut commands,
        mut meshes,
        mut materials,
        mut std_materials,
    } = draw;
    if !session.is_changed() && !style.is_changed() {
        return;
    }
    for e in stale.iter() {
        commands.entity(e).despawn();
    }

    match *style {
        BoardStyle::Classic => {
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
        BoardStyle::Amlah => {
            // Everything is a flat shape just above the connection lines,
            // which sit at EDGE_Y = 0.005.
            let flat = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
            let at = |c: Coord, y: f32| {
                let w = board_amlah::plane_to_world3(coord_to_world(c));
                Transform::from_rotation(flat).with_translation(Vec3::new(w.x, y, w.z))
            };

            // Trail of the staged jump so far.
            if let Selection::Jumping { turn } = &session.selection {
                let dot = meshes.add(Circle::new(0.04));
                let mat = std_materials.add(StandardMaterial {
                    base_color: Color::srgb(1.0, 0.85, 0.4),
                    unlit: true,
                    ..default()
                });
                for hole in turn.path() {
                    commands.spawn((
                        Mesh3d(dot.clone()),
                        MeshMaterial3d(mat.clone()),
                        at(*hole, 0.008),
                        Overlay,
                    ));
                }
            }

            // Ring around the selected piece. White vanishes on the cream
            // plate, so the idle ring is ink; gold while a jump is staged.
            if let Some(sel) = session.selected_hole() {
                let ring = meshes.add(Annulus::new(0.09, 0.115));
                let colour = if session.is_jumping() {
                    Color::srgb(1.0, 0.82, 0.30)
                } else {
                    board_amlah::INK
                };
                commands.spawn((
                    Mesh3d(ring),
                    MeshMaterial3d(std_materials.add(StandardMaterial {
                        base_color: colour,
                        unlit: true,
                        ..default()
                    })),
                    at(sel, 0.007),
                    Overlay,
                ));
            }

            // One hop ahead only.
            let dot = meshes.add(Circle::new(0.05));
            let mat = std_materials.add(StandardMaterial {
                base_color: board_amlah::MOVE_DOT,
                unlit: true,
                ..default()
            });
            for t in session.highlights() {
                commands.spawn((
                    Mesh3d(dot.clone()),
                    MeshMaterial3d(mat.clone()),
                    at(t, 0.006),
                    Overlay,
                ));
            }
        }
    }
}

/// Dim the controls when they would do nothing, so the staged state is
/// legible; brighten on hover, darken on press. Runs every frame so hover
/// repaints immediately; writes only real colour changes. The resign button
/// is additionally hidden entirely in networked games — conceding there must
/// reach every peer over the wire, which it does not yet.
fn sync_buttons(
    session: Res<Session>,
    net: Res<NetState>,
    viewer: Option<Res<replay::ReplayView>>,
    mut buttons: Query<(
        &Interaction,
        &ControlButton,
        &mut BackgroundColor,
        &mut Visibility,
    )>,
) {
    for (interaction, which, mut bg, mut vis) in buttons.iter_mut() {
        // While the record viewer is up the whole row steps aside: its keys
        // own the input, and the controls have nothing to act on.
        let wanted = if viewer.is_some() {
            Visibility::Hidden
        } else if matches!(
            which,
            ControlButton::Resign
                | ControlButton::Save
                | ControlButton::Open
                | ControlButton::Replay
        ) {
            // Record controls are local-mode, like resign: a networked round
            // is shared state, and one peer's save would say nothing about
            // the rest of the table.
            if net.seats.is_empty() {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            }
        } else {
            Visibility::Inherited
        };
        if *vis != wanted {
            *vis = wanted;
        }
        // Buttons do nothing on someone else's turn — dim them entirely so the
        // controls cannot look actionable. Confirm also needs a staged move to
        // submit, Cancel needs a selection to abandon. Save and Open are
        // always available in a local game: a finished round is worth saving.
        let active = session.may_act();
        let base = match which {
            ControlButton::Confirm if active && session.can_confirm() => {
                Color::srgb(0.20, 0.45, 0.28)
            }
            ControlButton::Cancel if active && session.selected_hole().is_some() => {
                Color::srgb(0.45, 0.24, 0.24)
            }
            ControlButton::Resign if active && !session.game.is_over() => {
                Color::srgb(0.36, 0.27, 0.22)
            }
            ControlButton::Save | ControlButton::Open => Color::srgb(0.22, 0.28, 0.38),
            _ => Color::srgb(0.18, 0.18, 0.21),
        };
        let colour = match interaction {
            Interaction::Pressed => base.darker(0.15),
            Interaction::Hovered if base != Color::srgb(0.18, 0.18, 0.21) => base.lighter(0.15),
            _ => base,
        };
        if bg.0 != colour {
            bg.0 = colour;
        }
    }
}

/// The colour swatch + label naming the active home base: whose camp is to
/// move, and whether it is ours.
fn sync_turn_indicator(
    session: Res<Session>,
    net: Res<NetState>,
    mut swatch: Query<&mut BackgroundColor, With<TurnSwatch>>,
    mut text: Query<&mut Text, With<TurnText>>,
) {
    if !session.is_changed() {
        return;
    }
    let (Ok(mut swatch), Ok(mut text)) = (swatch.single_mut(), text.single_mut()) else {
        return;
    };

    let (colour, label) = match session.game.outcome() {
        Some(Outcome::Winner(p)) => (player_colour(p), "Game over".into()),
        Some(Outcome::Resigned(p)) => (player_colour(p), "Game over - resignation".into()),
        Some(Outcome::Draw) => (Color::srgb(0.6, 0.6, 0.66), "Game over - draw".into()),
        None => {
            let active = session.game.turn();
            let colour = player_colour(active);
            let label = if session.local_player() == Some(active) {
                "Your home base - you to move".to_string()
            } else {
                let who = player_label(&net, active);
                let waiting = if session.local_player().is_some() {
                    " (waiting)"
                } else {
                    ""
                };
                format!("Home base to move: {who}{waiting}")
            };
            (colour, label)
        }
    };

    swatch.0 = colour;
    **text = label;
}

/// The player's lobby name if known, else "Player N".
fn player_label(net: &NetState, p: Player) -> String {
    net.seats
        .iter()
        .find(|s| s.player == Some(u32::from(p.index())))
        .map(|s| s.name.clone())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| format!("Player {}", p.index()))
}

/// Rings around the active player's home camp, so the base whose turn it is
/// is visible on the board itself, not only in the status line. Rebuilt when
/// the turn changes.
fn sync_camp_indicator(
    draw: DrawContext,
    stale: Query<Entity, With<CampMarker>>,
    session: Res<Session>,
    style: Res<BoardStyle>,
) {
    let DrawContext {
        mut commands,
        mut meshes,
        mut materials,
        mut std_materials,
    } = draw;
    if !session.is_changed() && !style.is_changed() {
        return;
    }
    for e in stale.iter() {
        commands.entity(e).despawn();
    }
    if session.game.is_over() {
        return;
    }

    let camp = session.game.turn().start_camp();
    match *style {
        BoardStyle::Classic => {
            // The player's own hue, over the neutral grey camp.
            let colour = player_colour(session.game.turn());
            let ring = meshes.add(Annulus::new(PIECE_RADIUS + 1.0, PIECE_RADIUS + 3.0));
            let mat = materials.add(colour.with_alpha(0.55));
            for &c in camp {
                let p = coord_to_world(c);
                commands.spawn((
                    Mesh2d(ring.clone()),
                    MeshMaterial2d(mat.clone()),
                    Transform::from_xyz(p.x, p.y, 1.2),
                    CampMarker,
                ));
            }
        }
        BoardStyle::Amlah => {
            // White, reading against the same-hue accent triangle that marks
            // every camp; only the active one is ringed.
            let ring = meshes.add(Annulus::new(0.095, 0.11));
            let mat = std_materials.add(StandardMaterial {
                base_color: Color::WHITE,
                unlit: true,
                ..default()
            });
            for &c in camp {
                let w = board_amlah::plane_to_world3(coord_to_world(c));
                commands.spawn((
                    Mesh3d(ring.clone()),
                    MeshMaterial3d(mat.clone()),
                    Transform::from_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2))
                        .with_translation(Vec3::new(w.x, 0.0065, w.z)),
                    CampMarker,
                ));
            }
        }
    }
}

/// The game-over overlay: winner and statistics. Spawned when the game ends,
/// despawned when a new game begins (`R`).
fn sync_game_over(
    session: Res<Session>,
    net: Res<NetState>,
    time: Res<Time>,
    existing: Query<Entity, With<GameOverUi>>,
    mut commands: Commands,
) {
    let over = session.game.is_over();
    let shown = !existing.is_empty();
    if over == shown {
        return;
    }
    if !over {
        for e in existing.iter() {
            commands.entity(e).despawn();
        }
        return;
    }

    let (title, title_colour) = match session.game.outcome() {
        Some(Outcome::Winner(p)) => {
            let title = if session.local_player() == Some(p) {
                "You win!".to_string()
            } else {
                format!("{} wins!", player_label(&net, p))
            };
            (title, player_colour(p))
        }
        Some(Outcome::Resigned(p)) => {
            let title = if session.local_player() == Some(p) {
                "You resign.".to_string()
            } else {
                format!("{} resigns.", player_label(&net, p))
            };
            (title, player_colour(p))
        }
        Some(Outcome::Draw) | None => ("Draw: every player is blocked.".to_string(), Color::WHITE),
    };

    commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            GameOverUi,
        ))
        .with_children(|root| {
            root.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(8.0),
                    padding: UiRect::axes(Val::Px(34.0), Val::Px(24.0)),
                    border_radius: BorderRadius::all(Val::Px(8.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.10, 0.10, 0.13, 0.97)),
            ))
            .with_children(|panel| {
                panel.spawn((
                    Text::new(title),
                    TextFont {
                        font_size: FontSize::Px(30.0),
                        ..default()
                    },
                    TextColor(title_colour),
                ));
                panel.spawn((
                    Text::new("Statistics"),
                    TextFont {
                        font_size: FontSize::Px(15.0),
                        ..default()
                    },
                    TextColor(Color::srgb(0.62, 0.62, 0.68)),
                ));
                for p in session.seating.players() {
                    let i = p.index() as usize;
                    let mut line = format!(
                        "{}:  {} moves  ({} by jump)",
                        player_label(&net, p),
                        session.stats.moves[i],
                        session.stats.jumps[i]
                    );
                    if let Some(pct) =
                        (100 * session.stats.hops_over_others[i]).checked_div(session.stats.hops[i])
                    {
                        line.push_str(&format!(
                            ",  {} hops ({pct}% over others)",
                            session.stats.hops[i]
                        ));
                    }
                    panel.spawn((
                        Text::new(line),
                        TextFont {
                            font_size: FontSize::Px(15.0),
                            ..default()
                        },
                        TextColor(Color::srgb(0.85, 0.85, 0.88)),
                    ));
                }
                panel.spawn((
                    Text::new(format!(
                        "{} moves total, {} passed turns",
                        session.stats.total_moves(),
                        session.stats.passes
                    )),
                    TextFont {
                        font_size: FontSize::Px(14.0),
                        ..default()
                    },
                    TextColor(Color::srgb(0.72, 0.72, 0.78)),
                ));
                if let Some(d) = session.stats.round_duration(time.elapsed()) {
                    panel.spawn((
                        Text::new(format!("Round lasted {}", format_round_duration(d))),
                        TextFont {
                            font_size: FontSize::Px(14.0),
                            ..default()
                        },
                        TextColor(Color::srgb(0.72, 0.72, 0.78)),
                    ));
                }
                if session.stats.longest_jump > 0 {
                    let by = Player::new(session.stats.longest_jump_by)
                        .expect("longest-jump player is below six");
                    panel.spawn((
                        Text::new(format!(
                            "Longest jump: {} hops ({})",
                            session.stats.longest_jump,
                            player_label(&net, by)
                        )),
                        TextFont {
                            font_size: FontSize::Px(14.0),
                            ..default()
                        },
                        TextColor(Color::srgb(0.72, 0.72, 0.78)),
                    ));
                }
                panel.spawn((
                    Text::new("Press R for a new game"),
                    TextFont {
                        font_size: FontSize::Px(13.0),
                        ..default()
                    },
                    TextColor(Color::srgb(0.62, 0.62, 0.68)),
                ));
            });
        });
}

fn sync_status(
    session: Res<Session>,
    style: Res<BoardStyle>,
    mut text: Query<&mut Text, With<StatusText>>,
) {
    if !session.is_changed() && !style.is_changed() {
        return;
    }
    let Ok(mut text) = text.single_mut() else {
        return;
    };

    let header = match session.game.outcome() {
        Some(Outcome::Winner(p)) => format!("Player {} wins!", p.index()),
        Some(Outcome::Resigned(p)) => format!("Player {} resigns.", p.index()),
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
        Selection::Pend { mv, .. } => format!(
            "  |  staging a step to ({},{})",
            mv.destination.q, mv.destination.r
        ),
        _ => String::new(),
    };

    **text = format!(
        "{header}{staged}\n{}\n{} laws checked at startup  |  invariants checked each turn  |  style: {}\n\
         Click a piece, then a highlighted hole. Jumps chain one hop at a time.\n\
         Enter confirms, Backspace cancels, U undoes a hop, R restarts, V switches the board style, T hides this.",
        session.message,
        LAWS.len(),
        style.label(),
    );
}

// --- the computer opponent --------------------------------------------------

/// The persistent engine. It remembers the game's recent positions for the
/// anti-shuffle rule, and forgets them when a new game is dealt.
#[derive(Resource)]
struct AiEngine(Ai);

impl Default for AiEngine {
    fn default() -> Self {
        Self(Ai::new(AiConfig::default()))
    }
}

/// Let the computer play the current seat, if it owns one.
///
/// Runs after input and before the network pump: the engine's move enters the
/// outbox like any human's, so multiplayer sequencing applies to it verbatim.
/// The call is synchronous and thinks for the configured budget, so the frame
/// it moves in takes as long as the engine thinks.
fn ai_take_turn(
    mut session: ResMut<Session>,
    mut engine: ResMut<AiEngine>,
    mut pace: ResMut<AiPace>,
    time: Res<Time>,
    viewer: Option<Res<replay::ReplayView>>,
) {
    // The viewer's session is a derived copy; the engine must not advance it.
    if viewer.is_some() {
        return;
    }
    // The driver owns the staged-jump selection while its hops fly, so the
    // human-selection gate lives inside it. The pacing clock is Bevy's, which
    // is available on wasm where std's wall clock is not.
    let now = time.elapsed();
    let move_no = session.stats.total_moves() + 1;
    match pace.advance(&mut session, &mut engine.0, now) {
        Action::Wait => {}
        Action::Hop(hole) => {
            session.message = format!(
                "Player {} hops to ({},{})",
                session.game.turn().index(),
                hole.q,
                hole.r
            );
        }
        Action::Commit(mv) | Action::Play(mv) => {
            let seat = session.game.turn().index();
            let line = format!(
                "{}. p{} {}",
                move_no,
                seat,
                checkers_bevy::ai::describe(&mv)
            );
            checkers_bevy::move_log::log(&line);
            session.message = format!(
                "Player {} (computer): {}",
                seat,
                checkers_bevy::ai::describe(&mv)
            );
            session.outbox.push(mv);
        }
        Action::Pass => {
            let seat = session.game.turn().index();
            checkers_bevy::move_log::log(&format!("{}. p{} passes", move_no, seat));
            session.game.pass();
        }
        // A headless stall neither side can resolve: log it honestly and stop
        // advancing — the log is the whole point of watching the race.
        Action::Abandon(reason) => {
            checkers_bevy::move_log::log(&format!(
                "# game abandoned: {reason} after {} moves",
                session.stats.total_moves()
            ));
            session.message = "Game abandoned: mutual deadlock".to_string();
            pace.result_logged = true;
        }
    }

    // The end-of-game line, exactly once.
    if session.game.is_over() && !pace.result_logged {
        pace.result_logged = true;
        let line = match session.game.outcome() {
            Some(checkers_core::rules::Outcome::Winner(p)) => format!(
                "# game over: player {} wins after {} moves",
                p.index(),
                session.stats.total_moves()
            ),
            Some(checkers_core::rules::Outcome::Resigned(p)) => format!(
                "# game over: player {} resigned after {} moves",
                p.index(),
                session.stats.total_moves()
            ),
            _ => format!(
                "# game over: draw after {} moves",
                session.stats.total_moves()
            ),
        };
        checkers_bevy::move_log::log(&line);
    }
}

/// `A` hands the current seat to the computer for one move — a hint, a
/// resignation to a difficult position, or a way to watch the engine race.
fn ai_one_shot(
    keys: Res<ButtonInput<KeyCode>>,
    mut session: ResMut<Session>,
    mut engine: ResMut<AiEngine>,
) {
    // Spectators watch: the A key is not theirs to press.
    if !keys.just_pressed(KeyCode::KeyA) || session.game.is_over() || session.spectating {
        return;
    }
    if let Some(mv) = engine.0.choose_move(&session.game) {
        session.message = format!(
            "The computer suggests ({},{}) -> ({},{})",
            mv.origin.q, mv.origin.r, mv.destination.q, mv.destination.r
        );
        session.outbox.push(mv);
    }
}
