//! Bevy front-end for the machine-checked Chinese Checkers rules.
//!
//! The rules live entirely in `checkers-core`; this crate renders a [`Game`] and
//! turns clicks into moves. Destinations are taken from the rules rather than
//! constructed, so the UI has no path to a position the rules disallow.
//!
//! # Interaction
//!
//! Steps commit immediately — there is nothing to chain. Jumps are **staged**:
//! selecting a piece shows only the destinations reachable in **one** hop, and
//! clicking one moves the piece there, keeps it selected, and reveals the next
//! single hop. The turn is not committed until the player confirms, so the whole
//! chain can be abandoned.
//!
//! | Input | Effect |
//! |---|---|
//! | Click own piece | Select it |
//! | Click a highlighted hole | Take one hop (jump) or move (step) |
//! | Enter, or the Confirm button | End the jump turn |
//! | Backspace, or the Cancel button | Abandon the whole turn |
//! | U | Undo the last hop |
//! | Escape | Clear the selection |
//! | T | Show/hide the status panel |
//!
//! # Lobby and networked play
//!
//! The app opens in a lobby ([`lobby`]): peers sharing a room id find each other
//! over WebRTC, the host assigns seats, and Enter starts the game once everyone
//! is ready. A solo player presses Enter on an empty roster.
//!
//! Moves are never applied where they are made. They are queued in
//! [`Session::outbox`] and applied only when they come back **host-sequenced**
//! ([`net`]), which gives every peer one identical order. Solo play takes the
//! same path — the lone peer sequences for itself — so the networked code is
//! exercised even with one player.
//!
//! Confirming before any hop is refused: chapter 9 requires a jump turn to move
//! the piece, and a turn ending where it began is indistinguishable from not
//! moving. That case is reachable, since a piece can hop back over its blocker.
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

mod board_view;
mod lobby;
mod net;

use bevy::prelude::*;
use board_view::{HOLE_RADIUS, HOLE_SPACING, PIECE_RADIUS, coord_to_world, world_to_coord};
use checkers_core::audit::audit_position;
use checkers_core::geometry::{Coord, all_holes, camp_of, on_board};
use checkers_core::law::{LAWS, verify_all};
// Aliased because `bevy::prelude` also exports a `Move` (a picking event), and
// an unqualified `Move` here silently resolves to Bevy's.
use checkers_core::position::Move as GameMove;
use checkers_core::position::{MoveKind, Player, Position};
use checkers_core::rules::{Game, Outcome, legal_moves};
use checkers_core::turn::{JumpTurn, single_hop_destinations, step_destinations};

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Chinese Checkers".into(),
                resolution: (900u32, 980u32).into(),
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

/// Lobby first, then the board. Networked play needs seats assigned before the
/// game is playable, and the same gate keeps a solo player's flow unchanged —
/// they simply press Enter on an empty roster.
#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
enum AppState {
    #[default]
    Lobby,
    InGame,
}

/// What the player is currently doing.
#[derive(Default)]
enum Selection {
    /// Nothing selected.
    #[default]
    None,
    /// A piece is selected but no jump has begun, so both steps and first hops
    /// are available.
    Piece { origin: Coord },
    /// A jump turn is under way and awaiting confirmation.
    Jumping { turn: JumpTurn },
}

/// The game plus the UI's selection state.
#[derive(Resource)]
struct Session {
    game: Game,
    selection: Selection,
    message: String,
    /// Which player this peer may move. `None` in solo play, where every player
    /// is controlled locally, or when spectating a full game.
    local_player: Option<Player>,
    /// Moves this peer has committed but that are not yet applied.
    ///
    /// Moves are never applied where they are made. They go here, and
    /// [`net::pump`] submits them for sequencing; the game advances only when
    /// the move comes back sequenced. In solo play the pump sequences locally in
    /// the same frame, so the delay is invisible — but the code path is the same
    /// one, which is why solo play exercises it.
    outbox: Vec<GameMove>,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            game: Game::new(),
            selection: Selection::None,
            message: "Click one of your pieces".into(),
            local_player: None,
            outbox: Vec::new(),
        }
    }
}

impl Session {
    /// The position to render: a staged turn's preview, else the real position.
    fn display_position(&self) -> &Position {
        match &self.selection {
            Selection::Jumping { turn } => turn.preview(),
            _ => self.game.position(),
        }
    }

    /// The hole the selected piece currently occupies.
    fn selected_hole(&self) -> Option<Coord> {
        match &self.selection {
            Selection::None => None,
            Selection::Piece { origin } => Some(*origin),
            Selection::Jumping { turn } => Some(turn.current()),
        }
    }

    /// Holes to highlight as clickable destinations.
    ///
    /// Only ever **one** hop ahead for jumps: offering the full closure would
    /// let the player skip intermediate holes.
    fn highlights(&self) -> Vec<Coord> {
        match &self.selection {
            Selection::None => Vec::new(),
            Selection::Piece { origin } => {
                // One piece's own steps and first hops. Filtering `legal_moves`
                // would compute every other piece's jump closure and throw it
                // away — about 150x the work for the same answer.
                let pos = self.game.position();
                let mut out = step_destinations(pos, *origin);
                out.extend(single_hop_destinations(pos, *origin));
                out.sort();
                out.dedup();
                out
            }
            Selection::Jumping { turn } => turn.next_hops(),
        }
    }

    fn is_jumping(&self) -> bool {
        matches!(self.selection, Selection::Jumping { .. })
    }

    fn can_confirm(&self) -> bool {
        match &self.selection {
            Selection::Jumping { turn } => turn.can_commit(),
            _ => false,
        }
    }

    /// May this peer act right now?
    ///
    /// `None` means solo play, where every player is driven locally. Otherwise
    /// the peer may only move on its own turn — enforced here so an out-of-turn
    /// click never reaches the outbox, and again by the rules on the receiving
    /// side, which reject any move that is not currently legal.
    fn may_act(&self) -> bool {
        match self.local_player {
            None => true,
            Some(p) => p == self.game.turn(),
        }
    }

    fn select(&mut self, hole: Coord) {
        if !self.may_act() {
            self.message = format!("Waiting for player {}", self.game.turn().index());
            return;
        }
        let player = self.game.turn();
        if self.game.position().occupant(hole) != Some(player) {
            return;
        }
        self.selection = Selection::Piece { origin: hole };

        let total = self.highlights().len();
        let hops = single_hop_destinations(self.game.position(), hole).len();
        self.message = format!(
            "Player {} selected ({},{}): {total} destination(s), {hops} by jumping",
            player.index(),
            hole.q,
            hole.r
        );
    }

    fn clear_selection(&mut self) {
        self.selection = Selection::None;
    }

    /// Click on `hole` while something is selected.
    fn activate(&mut self, hole: Coord) {
        if !self.highlights().contains(&hole) {
            self.message = format!("({},{}) is not a legal destination", hole.q, hole.r);
            return;
        }
        let player = self.game.turn();

        match &mut self.selection {
            Selection::None => {}

            Selection::Piece { origin } => {
                let origin = *origin;

                // A step commits at once; a first hop begins a staged turn.
                let step = legal_moves(self.game.position(), player)
                    .into_iter()
                    .find(|m| {
                        m.origin == origin && m.destination == hole && m.kind == MoveKind::Step
                    });

                if let Some(mv) = step {
                    self.outbox.push(mv);
                    self.clear_selection();
                    self.message = format!(
                        "Player {} stepped ({},{}) -> ({},{})",
                        player.index(),
                        origin.q,
                        origin.r,
                        hole.q,
                        hole.r
                    );
                    return;
                }

                let Some(mut turn) = JumpTurn::begin(self.game.position(), player, origin) else {
                    return;
                };
                if turn.hop(hole) {
                    let remaining = turn.next_hops().len();
                    self.message = format!("Hop 1 to ({},{}). {}", hole.q, hole.r, hint(remaining));
                    self.selection = Selection::Jumping { turn };
                }
            }

            Selection::Jumping { turn } => {
                if turn.hop(hole) {
                    let hops = turn.hops();
                    let remaining = turn.next_hops().len();
                    self.message =
                        format!("Hop {hops} to ({},{}). {}", hole.q, hole.r, hint(remaining));
                }
            }
        }
    }

    /// Commit the staged jump turn.
    fn confirm(&mut self) {
        let Selection::Jumping { turn } = &self.selection else {
            return;
        };
        let mv = match turn.to_move() {
            Ok(mv) => mv,
            Err(e) => {
                // Reachable: the piece hopped back to where it began.
                self.message = format!("Cannot confirm — {e}");
                return;
            }
        };

        let player = self.game.turn();
        let hops = turn.hops();
        let dest = mv.destination;

        self.outbox.push(mv);
        self.clear_selection();
        self.message = format!(
            "Player {} jumped {hops} hop(s) to ({},{})",
            player.index(),
            dest.q,
            dest.r
        );
    }

    /// Abandon the staged turn without touching the game.
    fn cancel(&mut self) {
        self.message = if self.is_jumping() {
            "Jump cancelled".into()
        } else {
            "Selection cleared".into()
        };
        self.clear_selection();
    }

    /// Undo the most recent hop, keeping the turn open.
    fn undo_hop(&mut self) {
        let Selection::Jumping { turn } = &mut self.selection else {
            return;
        };
        if !turn.undo() {
            return;
        }
        let hops = turn.hops();
        self.message = format!("Undid a hop ({hops} remaining)");
        if hops == 0 {
            // Back at the start: fall back to plain selection so steps are
            // offered again.
            let origin = turn.origin();
            self.selection = Selection::Piece { origin };
        }
    }
}

fn hint(remaining: usize) -> String {
    if remaining == 0 {
        "No further hops — press Enter to confirm.".into()
    } else {
        format!("{remaining} further hop(s), or press Enter to confirm.")
    }
}

/// Hold the live position to the specification's invariants.
fn audit(position: &Position) {
    if let Err(fault) = audit_position(position) {
        panic!("specification violated while playing: {fault}");
    }
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

/// Verify the specification and spawn the camera. Runs once, before the lobby:
/// the app refuses to show anything at all if its own laws do not hold.
fn setup(mut commands: Commands) {
    // The full law registry is worth its cost once, at startup.
    if let Err(violation) = verify_all() {
        panic!("the specification does not hold: {violation}");
    }
    audit(&Position::initial());

    commands.spawn(Camera2d);
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
                Err(e) => format!(" — {e}"),
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
