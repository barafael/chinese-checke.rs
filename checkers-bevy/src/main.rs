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
use bevy::ecs::system::SystemParam;
use bevy::input::touch::{TouchInput, TouchPhase};
use bevy::prelude::*;
// Not in the prelude, unlike the rest of the window API.
use bevy::window::{Monitor, PrimaryMonitor};
use bevy_matchbox::prelude::MatchboxSocket;
use checkers_ai::{Ai, AiConfig};
use checkers_bevy::ai::{Action, AiPace};
use checkers_bevy::board_view::{
    BOARD_FRAME, HOLE_RADIUS, HOLE_SPACING, PIECE_RADIUS, coord_to_world, player_colour,
    world_to_coord,
};
use checkers_bevy::replay::{self, TraceMarker};
use checkers_bevy::setup::Seating;
use checkers_bevy::{
    AppState, Selection, Session, audit, format_round_duration, lobby, move_log, net, record,
    sound, web,
};
use checkers_core::geometry::{Coord, all_holes, camp_of, on_board};
use checkers_core::law::{LAWS, verify_all};
use checkers_core::position::{Player, Position};
use checkers_core::rules::Outcome;
use checkers_net::NetState;
use std::collections::HashMap;

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
        .init_resource::<AiEngine>()
        .init_resource::<AiPace>()
        .init_resource::<replay::Replay>()
        .add_plugins((lobby::plugin, sound::plugin))
        .add_systems(Startup, setup)
        // Not state-scoped: the lobby is the first thing shown, and it is the
        // screen whose buttons the old size hid.
        .add_systems(Update, (size_to_monitor, scale_ui_to_window))
        .add_systems(
            OnExit(AppState::InGame),
            // The menu is a full teardown: the next deal rebuilds the board
            // and the UI from nothing.
            exit_round_teardown,
        )
        .add_systems(
            OnEnter(AppState::InGame),
            (
                // A fresh engine per round: the deal is where a round's tuning is decided,
                // and a new engine carries no repetition memory from the last
                // one.
                |mut engine: ResMut<AiEngine>, mut pace: ResMut<AiPace>| {
                    engine.0 = Ai::new(AiConfig::default());
                    pace.reset();
                },
                lobby::apply_seats,
                spawn_board,
                spawn_ui,
            )
                .chain(),
        )
        .add_systems(
            Update,
            (
                // Bevy caps a chained tuple at twenty systems, and this chain
                // outgrew that. Split at the natural seam — input and move
                // sequencing versus the view-sync systems — and chain the
                // halves: order is preserved exactly.
                (
                    // The record viewer owns both the board and the keyboard
                    // while it is up, so every play system stands down for
                    // exactly that time (`ReplayView` exists only then);
                    // `handle_view_keys` is the lone exception, opening and
                    // stepping the viewer.
                    handle_buttons.run_if(not(resource_exists::<replay::ReplayView>)),
                    handle_clicks.run_if(not(resource_exists::<replay::ReplayView>)),
                    replay::handle_view_keys,
                    handle_keys.run_if(not(resource_exists::<replay::ReplayView>)),
                    // The game-over card's way out, next to its `M` key.
                    exit_to_lobby,
                    ai_one_shot,
                    toggle_status,
                    // A viewer session is rebuilt per step; without this guard
                    // the game-over card would report the viewer's age, not
                    // the round's length.
                    stamp_session_clock.run_if(not(resource_exists::<replay::ReplayView>)),
                    // Drains the outbox and applies only host-sequenced moves,
                    // so it must run after input and before the view syncs.
                    // The computer plays through the same outbox as a human:
                    // one sequencing path, no privileged moves.
                    ai_take_turn.run_if(not(resource_exists::<replay::ReplayView>)),
                    net::pump,
                    sound_watch,
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

/// A root of the in-game UI (status column, turn controls). Marks what
/// leaving to the lobby must tear down; the game-over card has its own
/// marker and goes with it.
#[derive(Component)]
struct HudUi;

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

/// The in-game buttons: the turn controls and the game-over card's `Menu`.
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
    /// Hand the finished round back to the lobby. Lives on the game-over
    /// card, next to the `M` key that does the same.
    Menu,
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
    let (Ok(monitor), Ok(mut window)) = (monitors.single(), windows.single_mut()) else {
        return;
    };
    *done = true;

    let scale = if monitor.scale_factor > 0.0 {
        monitor.scale_factor as f32
    } else {
        1.0
    };
    let logical = UVec2::new(monitor.physical_width, monitor.physical_height).as_vec2() / scale;
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
    // Framed by `fit_projection` rather than the default `WindowSize` scaling:
    // `WindowSize` maps one world unit to one pixel, pinning the board's size
    // in pixels — small on a large monitor, cropped in a small window.
    // `AutoMin` over `BOARD_FRAME` keeps the whole board visible and scales
    // with the window instead.
    //
    // One camera for the whole app: the lobby renders and picks through it
    // before any game exists, and every round draws through it after.
    commands.spawn((Camera2d, fit_projection()));

    // The full law registry is worth its cost once, at startup.
    if let Err(violation) = verify_all() {
        panic!("the specification does not hold: {violation}");
    }
    audit(&Position::initial(), &Seating::Six.players());
}

/// The camera's framing: at least [`BOARD_FRAME`] world units visible,
/// keeping aspect ratio. `AutoMin` guarantees the frame always fits entirely —
/// window too small and the camera zooms out, larger and it zooms in until the
/// frame is full — so the board's on-screen size follows the window instead of
/// being pinned in pixels.
fn fit_projection() -> Projection {
    Projection::Orthographic(OrthographicProjection {
        scaling_mode: ScalingMode::AutoMin {
            min_width: BOARD_FRAME.x,
            min_height: BOARD_FRAME.y,
        },
        ..OrthographicProjection::default_2d()
    })
}

/// Spawn the in-game UI: status panel and turn controls.
fn spawn_ui(mut commands: Commands) {
    // Bottom-left column: the active-base indicator (colour swatch + label)
    // above the status text.
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(10.0),
                left: Val::Px(12.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(3.0),
                ..default()
            },
            HudUi,
        ))
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
                row.spawn((text("", 15.0, TEXT), TurnText));
            });
            col.spawn((text("", 15.0, TEXT), StatusText));
        });

    // Turn controls, centred at the top: Confirm, Cancel, Resign, and the
    // record controls.
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(10.0),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                column_gap: Val::Px(8.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
            HudUi,
        ))
        .with_children(|row| {
            for (which, label) in [
                (ControlButton::Confirm, "Confirm (Enter)"),
                (ControlButton::Cancel, "Cancel (Backspace)"),
                (ControlButton::Resign, "Resign"),
                (ControlButton::Save, "Save"),
                (ControlButton::Open, "Open"),
                (ControlButton::Replay, "Replay"),
            ] {
                control_button(row, which, label);
            }
        });
}

// Text greys, brightest first: the status panel and per-player statistics,
// then the game-over card's totals, then its headings and hints.
const TEXT: Color = Color::srgb(0.85, 0.85, 0.88);
const TEXT_DIM: Color = Color::srgb(0.72, 0.72, 0.78);
const TEXT_FAINT: Color = Color::srgb(0.62, 0.62, 0.68);

/// A control button at rest.
const IDLE: Color = Color::srgb(0.18, 0.18, 0.21);

/// One line of UI text at a pixel size and colour.
fn text(content: impl Into<String>, size: f32, colour: Color) -> impl Bundle {
    (
        Text::new(content),
        TextFont {
            font_size: FontSize::Px(size),
            ..default()
        },
        TextColor(colour),
    )
}

/// One control button. Factored out because the turn controls and the
/// game-over card's `Menu` must not drift apart in padding, radius, or text.
fn control_button(parent: &mut ChildSpawnerCommands, which: ControlButton, label: &str) {
    parent
        .spawn((
            Button,
            Node {
                padding: UiRect::axes(Val::Px(12.0), Val::Px(7.0)),
                // In Bevy 0.19 BorderRadius is a Node field, not a
                // standalone component.
                border_radius: BorderRadius::all(Val::Px(4.0)),
                ..default()
            },
            BackgroundColor(IDLE),
            which,
        ))
        .with_child(text(label, 13.0, Color::srgb(0.9, 0.9, 0.92)));
}

/// Leaving the round tears down everything the round owns: the board meshes,
/// pieces, highlights and trace, the in-game UI, the game-over card, and the
/// record viewer — which otherwise would re-open its overlay in the middle of
/// the next round.
///
/// The camera is the one thing that stays: the lobby renders and picks
/// through it, and the next deal draws through it again.
///
/// The entities are one query, not one per marker, so an entity carrying
/// several of these markers is despawned exactly once.
fn exit_round_teardown(mut commands: Commands, owned: Query<Entity, RoundOwned>) {
    for e in owned.iter() {
        commands.entity(e).despawn();
    }
    // Removing a resource that is not there does nothing, so this needs no
    // check that a viewer is open.
    commands.remove_resource::<replay::ReplayView>();
}

/// Every entity a round owns, whatever markers it carries.
type RoundOwned = Or<(
    With<HoleMarker>,
    With<PieceMarker>,
    With<Overlay>,
    With<TraceMarker>,
    With<HudUi>,
    With<GameOverUi>,
)>;

/// The board: one entity per hole. Spawned on entering the round; the pieces
/// and highlights on top of it are rebuilt by the sync systems.
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
            &camp_mat
        } else {
            &hole_mat
        };
        commands.spawn((on_hole(&hole_mesh, material, c, 0.0), HoleMarker));
    }
}

/// A flat board mesh centred on hole `c`, at depth `z`: holes, pieces, and
/// everything drawn over them.
fn on_hole(mesh: &Handle<Mesh>, material: &Handle<ColorMaterial>, c: Coord, z: f32) -> impl Bundle {
    let p = coord_to_world(c);
    (
        Mesh2d(mesh.clone()),
        MeshMaterial2d(material.clone()),
        Transform::from_xyz(p.x, p.y, z),
    )
}

fn handle_buttons(
    interactions: Query<(&Interaction, &ControlButton), Changed<Interaction>>,
    mut session: ResMut<Session>,
    sounds: Res<sound::Sounds>,
    on: Res<sound::SoundOn>,
    mut commands: Commands,
) {
    for (interaction, which) in interactions.iter() {
        // On another player's move the controls are inert — the click would
        // otherwise confirm or cancel a selection this peer cannot touch.
        if *interaction != Interaction::Pressed || !session.may_act() {
            continue;
        }
        match which {
            ControlButton::Confirm => session.confirm(),
            ControlButton::Cancel => {
                session.cancel();
                sounds.play(&mut commands, *on, sound::SoundKind::Cancel);
            }
            ControlButton::Resign => session.resign(),
            ControlButton::Save => {
                session.message = match web::save_record(&session.to_record().to_text()) {
                    Ok(where_) => format!("Saved{where_}"),
                    Err(e) => format!("Save failed: {e}"),
                };
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
                    Ok(rec) => match Session::resumed_prefix(&rec, rec.moves.len()) {
                        Ok(s) => {
                            let view = replay::ReplayView::at_end(rec);
                            *session = s;
                            session.message = view.status();
                            commands.insert_resource(view);
                        }
                        Err(f) => session.message = format!("Could not replay: {f}"),
                    },
                    Err(f) => session.message = format!("Could not open: {f}"),
                },
                Err(e) => session.message = format!("Replay failed: {e}"),
            },
            // Handled by [`exit_to_lobby`], which must not be gated on
            // `may_act` — a finished game leaves the turn wherever it ended.
            ControlButton::Menu => {}
        }
    }
}

/// The way out of a finished round: the game-over card's `Menu` button, or
/// the `M` key once the game is over. Both hand the state back to the lobby;
/// [`exit_round_teardown`] on the exit transition clears the screen.
fn exit_to_lobby(
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Query<(&Interaction, &ControlButton), Changed<Interaction>>,
    session: Res<Session>,
    mut next_state: ResMut<NextState<AppState>>,
) {
    let pressed = buttons.iter().any(|(interaction, which)| {
        *interaction == Interaction::Pressed && *which == ControlButton::Menu
    });
    if pressed || (keys.just_pressed(KeyCode::KeyM) && session.game.is_over()) {
        next_state.set(AppState::Lobby);
    }
}

/// Fingers being tracked for a tap, and the feed they come from: a release
/// near where its finger landed is a tap, a release far away is a drag.
#[derive(SystemParam)]
struct TouchTaps<'w, 's> {
    events: MessageReader<'w, 's, TouchInput>,
    starts: Local<'s, HashMap<u64, Vec2>>,
}

fn handle_clicks(
    buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    controls: Query<&Interaction, With<ControlButton>>,
    mut session: ResMut<Session>,
    mut taps: TouchTaps,
) {
    // The record viewer's board is read-only: clicks move nothing.
    if session.game.is_over() {
        return;
    }

    // A touch counts as a click when the finger lifts within a flick of where
    // it landed. On the web canvas, winit prevents the browser's emulated
    // mouse events, so without this a touchscreen could select nothing. The
    // mouse path below is untouched, and a drag travels too far to qualify.
    let mut tap: Option<Vec2> = None;
    for event in taps.events.read() {
        match event.phase {
            TouchPhase::Started | TouchPhase::Moved => {
                taps.starts.insert(event.id, event.position);
            }
            TouchPhase::Ended | TouchPhase::Canceled => {
                if let Some(start) = taps.starts.remove(&event.id)
                    && start.distance(event.position) < 12.0
                {
                    tap = Some(event.position);
                }
            }
        }
    }
    let mouse_click = buttons.just_pressed(MouseButton::Left);
    if !mouse_click && tap.is_none() {
        return;
    }
    // Do not treat a click on a control button as a board click.
    if controls.iter().any(|i| *i != Interaction::None) {
        return;
    }

    let (Ok(window), Ok((camera, cam_tf))) = (windows.single(), cameras.single()) else {
        return;
    };
    let Some(cursor) = tap.or_else(|| window.cursor_position()) else {
        return;
    };
    let Ok(plane) = camera.viewport_to_world_2d(cam_tf, cursor) else {
        return;
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

/// Everything an input system drives in a live round: the session being
/// played, the table and house rules it was dealt from, and the sound
/// handler. One parameter in place of seven, which keeps `handle_keys` under
/// the clippy argument limit.
#[derive(SystemParam)]
struct PlayContext<'w, 's> {
    session: ResMut<'w, Session>,
    net: Res<'w, NetState>,
    table: Res<'w, lobby::Table>,
    variants: Res<'w, lobby::ChosenVariants>,
    sounds: Res<'w, sound::Sounds>,
    on: Res<'w, sound::SoundOn>,
    commands: Commands<'w, 's>,
}

fn handle_keys(keys: Res<ButtonInput<KeyCode>>, play: PlayContext) {
    let PlayContext {
        mut session,
        net,
        table,
        variants,
        sounds,
        on,
        mut commands,
    } = play;
    if keys.any_just_pressed([KeyCode::Enter, KeyCode::NumpadEnter]) && session.may_act() {
        // Someone else's turn: confirmation is inert so this peer cannot submit a
        // move on another player's behalf.
        session.confirm();
    }
    if keys.just_pressed(KeyCode::Backspace) && session.may_act() {
        session.cancel();
        sounds.play(&mut commands, *on, sound::SoundKind::Cancel);
    }
    if keys.just_pressed(KeyCode::KeyU) && session.may_act() {
        session.undo_hop();
    }
    if keys.just_pressed(KeyCode::Escape) {
        // The message names what was actually cleared, jump or selection.
        session.cancel();
    }
    // A shared round is shared state; only the host restarts it, from the
    // lobby. Solo, `R` just deals the configured table afresh.
    if keys.just_pressed(KeyCode::KeyR) && !session.shared {
        *session = lobby::deal_session(&net, &table, variants.0);
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
fn stamp_session_clock(mut session: ResMut<Session>, time: Res<Time>) {
    session.stats.note_started(time.elapsed());
}

/// Scale the interface with the window, so menus, fields, and the status panel
/// use the available space rather than being pinned to a fixed widget size. Tiny windows shrink the text but keep it
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
/// the view cannot drift from the model.
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

    // Unconditional despawn-and-respawn: the previous `is_changed` early-out
    // compared only occupied holes — not by whom — so it never skipped anything.
    for e in existing.iter() {
        commands.entity(e).despawn();
    }

    let mesh = meshes.add(Circle::new(PIECE_RADIUS));
    // One material per player rather than per piece: the rebuild runs on
    // every committed turn, and 60 fresh handles are waste for six colours.
    let mats = Player::ALL.map(|p| materials.add(player_colour(p)));
    for &c in position.holes() {
        let Some(player) = position.occupant(c) else {
            continue;
        };
        commands.spawn((
            on_hole(&mesh, &mats[player.index() as usize], c, 1.0),
            PieceMarker,
            replay::PieceCoord(c),
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
        for &hole in turn.path() {
            commands.spawn((on_hole(&dot, &mat, hole, 1.5), Overlay));
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
        let mat = materials.add(colour);
        commands.spawn((on_hole(&ring, &mat, sel, 2.0), Overlay));
    }

    // One hop ahead only.
    let dot = meshes.add(Circle::new(HOLE_RADIUS * 0.85));
    let mat = materials.add(Color::srgba(1.0, 1.0, 1.0, 0.8));
    for t in session.highlights() {
        commands.spawn((on_hole(&dot, &mat, t, 2.0), Overlay));
    }
}

/// Dim the controls when they would do nothing, so the staged state is
/// legible; brighten on hover, darken on press. Runs every frame so hover
/// repaints immediately; writes only real colour changes. The resign button
/// is additionally hidden entirely in networked games — conceding there must
/// reach every peer over the wire, which it does not yet.
fn sync_buttons(
    session: Res<Session>,
    viewer: Option<Res<replay::ReplayView>>,
    mut buttons: Query<(
        &Interaction,
        &ControlButton,
        &mut BackgroundColor,
        &mut Visibility,
    )>,
) {
    for (interaction, which, mut bg, mut vis) in buttons.iter_mut() {
        // Record controls are local-mode, like resign: a shared round is
        // shared state, and one peer's save would say nothing about the
        // rest of the table.
        let local_only = matches!(
            which,
            ControlButton::Resign
                | ControlButton::Save
                | ControlButton::Open
                | ControlButton::Replay
        );
        // While the record viewer is up the whole row steps aside: its keys
        // own the input, and the controls have nothing to act on.
        let wanted = if viewer.is_some() || (local_only && session.shared) {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
        vis.set_if_neq(wanted);
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
            _ => IDLE,
        };
        let colour = match interaction {
            Interaction::Pressed => base.darker(0.15),
            Interaction::Hovered if base != IDLE => base.lighter(0.15),
            _ => base,
        };
        bg.set_if_neq(BackgroundColor(colour));
    }
}

/// Answer hops and commits with sound. Watches the session rather than the
/// input paths, so a staged hop ticks whether it came from a click, a key,
/// or the engine, and a commit settles whether it was applied locally or
/// arrived already sequenced.
fn sound_watch(
    session: Res<Session>,
    sounds: Res<sound::Sounds>,
    on: Res<sound::SoundOn>,
    mut staged: Local<usize>,
    mut committed: Local<u32>,
    mut commands: Commands,
) {
    let hops = staged_hop_count(&session);
    if hops > *staged {
        sounds.play(&mut commands, *on, sound::SoundKind::Hop);
    }
    *staged = hops;

    let moves = session.stats.total_moves();
    if moves > *committed {
        sounds.play(&mut commands, *on, sound::SoundKind::Commit);
    }
    *committed = moves;
}

/// How many hops the staged turn has flown: none before a selection, one for
/// a pending step, the chain length mid-jump.
fn staged_hop_count(session: &Session) -> usize {
    match &session.selection {
        Selection::None | Selection::Piece { .. } => 0,
        Selection::Pend { .. } => 1,
        Selection::Jumping { turn } => turn.hops(),
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
        Some(Outcome::Abandoned) => (Color::srgb(0.7, 0.62, 0.42), "Game over - abandoned".into()),
        None => {
            let active = session.game.turn();
            let label = if session.local_player() == Some(active) {
                "Your home base - you to move".to_string()
            } else {
                let who = player_label(&net, &session, active);
                let waiting = if session.local_player().is_some() {
                    " (waiting)"
                } else {
                    ""
                };
                format!("Home base to move: {who}{waiting}")
            };
            (player_colour(active), label)
        }
    };

    swatch.0 = colour;
    **text = label;
}

/// The player's lobby name in a shared round, else "Player N".
fn player_label(net: &NetState, session: &Session, p: Player) -> String {
    session
        .roster_name(net, p)
        .map_or_else(|| format!("Player {}", p.index()), str::to_string)
}

/// Rings around the active player's home camp, so the base whose turn it is
/// is visible on the board itself, not only in the status line. Rebuilt when
/// the turn changes.
fn sync_camp_indicator(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    stale: Query<Entity, With<CampMarker>>,
    session: Res<Session>,
) {
    if !session.is_changed() {
        return;
    }
    for e in stale.iter() {
        commands.entity(e).despawn();
    }
    if session.game.is_over() {
        return;
    }

    // The player's own hue, over the neutral grey camp.
    let turn = session.game.turn();
    let ring = meshes.add(Annulus::new(PIECE_RADIUS + 1.0, PIECE_RADIUS + 3.0));
    let mat = materials.add(player_colour(turn).with_alpha(0.55));
    for &c in turn.start_camp() {
        commands.spawn((on_hole(&ring, &mat, c, 1.2), CampMarker));
    }
}

/// The game-over overlay: winner and statistics. Spawned when the game ends,
/// despawned when a new game begins (`R`).
fn sync_game_over(
    session: Res<Session>,
    net: Res<NetState>,
    time: Res<Time>,
    sounds: Res<sound::Sounds>,
    on: Res<sound::SoundOn>,
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

    // This peer's own seat reads as "you", anyone else's by name.
    let said_by = |p: Player, you: &str, they: &str| {
        if session.local_player() == Some(p) {
            you.to_string()
        } else {
            format!("{} {they}", player_label(&net, &session, p))
        }
    };
    let (title, title_colour) = match session.game.outcome() {
        Some(Outcome::Winner(p)) => (said_by(p, "You win!", "wins!"), player_colour(p)),
        Some(Outcome::Resigned(p)) => (said_by(p, "You resign.", "resigns."), player_colour(p)),
        Some(Outcome::Draw) | None => ("Draw: every player is blocked.".to_string(), Color::WHITE),
        Some(Outcome::Abandoned) => (
            "Game over: the race stalled without result.".to_string(),
            Color::srgb(0.7, 0.62, 0.42),
        ),
    };

    // A resignation falls; any other ending rings.
    let ending = match session.game.outcome() {
        Some(Outcome::Resigned(_)) => sound::SoundKind::Resign,
        _ => sound::SoundKind::Win,
    };
    sounds.play(&mut commands, *on, ending);

    let stats = &session.stats;
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
                panel.spawn(text(title, 30.0, title_colour));
                panel.spawn(text("Statistics", 15.0, TEXT_FAINT));
                for p in session.players.iter().copied() {
                    let i = p.index() as usize;
                    let mut line = format!(
                        "{}:  {} moves  ({} by jump)",
                        player_label(&net, &session, p),
                        stats.moves[i],
                        stats.jumps[i]
                    );
                    let hops = stats.hops[i];
                    if let Some(pct) = (100 * stats.hops_over_others[i]).checked_div(hops) {
                        line.push_str(&format!(",  {hops} hops ({pct}% over others)"));
                    }
                    panel.spawn(text(line, 15.0, TEXT));
                }
                let moves = stats.total_moves();
                let totals = format!("{moves} moves total, {} passed turns", stats.passes);
                panel.spawn(text(totals, 14.0, TEXT_DIM));
                if let Some(d) = stats.round_duration(time.elapsed()) {
                    let lasted = format!("Round lasted {}", format_round_duration(d));
                    panel.spawn(text(lasted, 14.0, TEXT_DIM));
                }
                if stats.longest_jump > 0 {
                    let by = Player::new(stats.longest_jump_by)
                        .expect("longest-jump player is below six");
                    let who = player_label(&net, &session, by);
                    let longest = format!("Longest jump: {} hops ({who})", stats.longest_jump);
                    panel.spawn(text(longest, 14.0, TEXT_DIM));
                }
                panel
                    .spawn(Node {
                        column_gap: Val::Px(10.0),
                        align_items: AlignItems::Center,
                        margin: UiRect::top(Val::Px(6.0)),
                        ..default()
                    })
                    .with_children(|row| {
                        control_button(row, ControlButton::Menu, "Menu (M)");
                        // `R` re-deals a solo table only; a shared round is
                        // restarted by the host, from the lobby.
                        if !session.shared {
                            row.spawn(text("R deals a new game", 13.0, TEXT_FAINT));
                        }
                    });
            });
        });
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
        Some(Outcome::Resigned(p)) => format!("Player {} resigns.", p.index()),
        Some(Outcome::Draw) => "Draw: every player is blocked.".to_string(),
        Some(Outcome::Abandoned) => "Game abandoned: the race stalled.".to_string(),
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

    // `R` re-deals a solo table only; a shared round is restarted by the host.
    let restart = if session.shared { "" } else { "R restarts, " };
    **text = format!(
        "{header}{staged}\n{}\n{} laws checked at startup  |  invariants checked each turn\n\
         Click a piece, then a highlighted hole. Jumps chain one hop at a time.\n\
         Enter confirms, Backspace cancels, U undoes a hop, {restart}T hides this.",
        session.message,
        LAWS.len(),
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
/// it moves in takes as long as the engine thinks. While the previous move's
/// flight is still on screen, the driver is held off — the execution is the
/// last part of its turn, and the next turn waits for it.
fn ai_take_turn(
    mut session: ResMut<Session>,
    mut engine: ResMut<AiEngine>,
    mut pace: ResMut<AiPace>,
    time: Res<Time>,
    replay_state: Res<replay::Replay>,
    mut socket: Option<ResMut<MatchboxSocket>>,
    net: Res<NetState>,
) {
    // The driver only has opinions about a move once the previous execution
    // has finished flying — see `Replay::busy`.
    let action = if replay_state.busy() {
        Action::Wait
    } else {
        pace.advance(&mut session, &mut engine.0, time.elapsed())
    };
    let moves = session.stats.total_moves();
    let seat = session.game.turn().index();
    match action {
        Action::Wait => {}
        Action::Play(mv) => {
            let described = move_log::describe(&mv);
            move_log::log(&format!("{}. p{seat} {described}", moves + 1));
            session.message = format!("Player {seat} (computer): {described}");
            session.outbox.push(mv);
        }
        Action::Pass => {
            move_log::log(&format!("{}. p{seat} passes", moves + 1));
            session.selection = Selection::None;
            checkers_bevy::net::after_turn(&mut session);
        }
        // An engine-only race neither side can resolve: log it honestly and
        // end the game — for every peer watching, too.
        Action::Abandon(reason) => {
            move_log::log(&format!("# game abandoned: {reason} after {moves} moves"));
            pace.result_logged = true;
            checkers_bevy::net::abandon_round(&mut session, socket.as_deref_mut(), &net);
        }
    }

    // The end-of-game line, exactly once.
    if session.game.is_over() && !pace.result_logged {
        pace.result_logged = true;
        let result = match session.game.outcome() {
            Some(Outcome::Winner(p)) => format!("player {} wins", p.index()),
            Some(Outcome::Resigned(p)) => format!("player {} resigned", p.index()),
            _ => "draw".to_string(),
        };
        let total = session.stats.total_moves();
        move_log::log(&format!("# game over: {result} after {total} moves"));
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

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;

    /// Leaving for the lobby must leave a camera alive: the lobby renders and
    /// picks through it. A teardown that took the camera with it left the
    /// screen on the round's last frame — indistinguishable from a hang.
    #[test]
    fn leaving_the_round_leaves_a_camera() {
        let mut world = World::new();

        let camera = world.spawn(Camera2d).id();
        let hole = world.spawn(HoleMarker).id();
        let piece = world.spawn(PieceMarker).id();
        let dot = world.spawn(Overlay).id();
        let trace = world.spawn(TraceMarker).id();
        let hud = world.spawn(HudUi).id();
        let card = world.spawn(GameOverUi).id();

        world.run_system_once(exit_round_teardown).unwrap();
        world.flush();

        assert!(
            world.get_entity(camera).is_ok(),
            "the lobby's camera must survive the teardown"
        );
        for (e, what) in [
            (hole, "board meshes"),
            (piece, "pieces"),
            (dot, "highlights"),
            (trace, "the trace"),
            (hud, "the HUD"),
            (card, "the game-over card"),
        ] {
            assert!(
                world.get_entity(e).is_err(),
                "{what} must go with the round"
            );
        }
    }
}
