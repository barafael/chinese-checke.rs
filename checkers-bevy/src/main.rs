//! Bevy front-end for the machine-checked Chinese Checkers rules.
//!
//! The rules live entirely in `checkers-core`; this crate only renders a
//! [`Game`] and turns clicks into moves. Every state change goes through
//! [`Game::play`] or [`Game::pass`], so the UI cannot reach a position the rules
//! would not allow.
//!
//! # Self-validation
//!
//! Two levels, because they cost very different amounts:
//!
//! - **At startup**, [`verify_all`] runs the entire law registry. Worth its
//!   roughly one-second cost once, and it means the app refuses to draw a board
//!   that does not satisfy its own specification.
//! - **After every move**, [`audit`] applies the position invariants to the live
//!   position. This is linear in the number of holes, so it is safe per move.
//!
//! Running the full registry per move would be far too slow: every law
//! regenerates its own sample games, and it never inspects the caller's
//! position. That distinction is what [`checkers_core::audit`] exists for.
//!
//! Either way, a rendering or input bug that corrupted game state fails loudly
//! with the offending law cited, rather than quietly producing an illegal board.

mod board_view;

use bevy::prelude::*;
use board_view::{HOLE_RADIUS, HOLE_SPACING, PIECE_RADIUS, coord_to_world, world_to_coord};
use checkers_core::audit::audit_position;
use checkers_core::geometry::{Coord, all_holes, on_board};
use checkers_core::law::{LAWS, verify_all};
use checkers_core::position::{Player, Position};
use checkers_core::rules::{Game, Outcome, jump_destinations, legal_moves};

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Chinese Checkers".into(),
                resolution: (900u32, 900u32).into(),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(ClearColor(Color::srgb(0.09, 0.09, 0.11)))
        .init_resource::<Session>()
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                handle_clicks,
                handle_keys,
                sync_pieces,
                sync_highlights,
                sync_status,
            )
                .chain(),
        )
        .run();
}

/// The game plus the UI's selection state.
#[derive(Resource)]
struct Session {
    game: Game,
    /// The hole whose piece is selected, if any.
    selected: Option<Coord>,
    /// Destinations the selected piece may reach.
    targets: Vec<Coord>,
    /// Last action, shown in the status line.
    message: String,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            game: Game::new(),
            selected: None,
            targets: Vec::new(),
            message: "Click one of your pieces".into(),
        }
    }
}

impl Session {
    /// Select `hole` if the active player owns a piece there.
    fn select(&mut self, hole: Coord) {
        let player = self.game.turn();
        if self.game.position().occupant(hole) != Some(player) {
            return;
        }
        let moves = legal_moves(self.game.position(), player);
        self.targets = moves
            .iter()
            .filter(|m| m.origin == hole)
            .map(|m| m.destination)
            .collect();
        self.selected = Some(hole);

        let jumps = jump_destinations(self.game.position(), hole).len();
        self.message = format!(
            "Player {} selected ({},{}): {} destination(s), {jumps} by jumping",
            player.index(),
            hole.q,
            hole.r,
            self.targets.len()
        );
    }

    fn clear_selection(&mut self) {
        self.selected = None;
        self.targets.clear();
    }

    /// Attempt to move the selected piece to `hole`.
    fn try_move(&mut self, hole: Coord) {
        let Some(origin) = self.selected else {
            return;
        };
        let player = self.game.turn();

        // Take the move from the rules rather than constructing one, so an
        // illegal destination simply is not found.
        let Some(mv) = legal_moves(self.game.position(), player)
            .into_iter()
            .find(|m| m.origin == origin && m.destination == hole)
        else {
            self.message = format!("({},{}) is not a legal destination", hole.q, hole.r);
            return;
        };

        let kind = mv.kind;
        self.game.play(&mv);
        self.clear_selection();
        self.message = format!(
            "Player {} {} ({},{}) -> ({},{})",
            player.index(),
            if kind == checkers_core::position::MoveKind::Jump {
                "jumped"
            } else {
                "stepped"
            },
            origin.q,
            origin.r,
            hole.q,
            hole.r
        );

        // The position just changed: hold it to the specification.
        audit(self.game.position());

        // A player with no legal move must pass (chapter 12).
        while !self.game.is_over() && self.game.legal_moves().is_empty() {
            let stuck = self.game.turn();
            self.game.pass();
            self.message = format!("{} — player {} passed", self.message, stuck.index());
        }
    }
}

/// Hold the live position to the specification's invariants.
///
/// Uses [`audit_position`], which is linear in the number of holes, rather than
/// the law registry. Running the registry per move would be far too slow: each
/// law regenerates its own sample games, so [`checkers_core::law::verify_all`]
/// costs on the order of a second and never inspects the caller's position. That
/// belongs in tests; this belongs in the loop.
fn audit(position: &Position) {
    if let Err(fault) = audit_position(position) {
        panic!("specification violated while playing: {fault}");
    }
}

// --- marker components -----------------------------------------------------

#[derive(Component)]
struct HoleMarker;

/// A rendered piece, tagged with the hole it represents.
#[derive(Component)]
struct PieceMarker {
    hole: Coord,
}

#[derive(Component)]
struct TargetMarker;

#[derive(Component)]
struct SelectionMarker;

#[derive(Component)]
struct StatusText;

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

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    // Check the board and initial position before drawing anything. The full
    // law registry is worth its cost once, at startup.
    if let Err(violation) = verify_all() {
        panic!("the specification does not hold: {violation}");
    }
    audit(&Position::initial());

    commands.spawn(Camera2d);

    let hole_mesh = meshes.add(Circle::new(HOLE_RADIUS));
    let hole_mat = materials.add(Color::srgb(0.22, 0.22, 0.26));
    let camp_mat = materials.add(Color::srgb(0.30, 0.30, 0.36));

    for c in all_holes() {
        // Tint camp holes slightly so the star's points are legible when empty.
        let material = if checkers_core::geometry::camp_of(c).is_some() {
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
            font_size: FontSize::Px(16.0),
            ..default()
        },
        TextColor(Color::srgb(0.85, 0.85, 0.88)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(10.0),
            left: Val::Px(12.0),
            ..default()
        },
        StatusText,
    ));
}

fn handle_clicks(
    buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    mut session: ResMut<Session>,
) {
    if !buttons.just_pressed(MouseButton::Left) || session.game.is_over() {
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
    if !on_board(hole) {
        return;
    }
    // Ignore clicks that land between holes.
    if coord_to_world(hole).distance(world) > HOLE_SPACING * 0.5 {
        return;
    }

    let player = session.game.turn();
    if session.game.position().occupant(hole) == Some(player) {
        session.select(hole);
    } else if session.selected.is_some() {
        session.try_move(hole);
    }
}

fn handle_keys(keys: Res<ButtonInput<KeyCode>>, mut session: ResMut<Session>) {
    if keys.just_pressed(KeyCode::Escape) {
        session.clear_selection();
        session.message = "Selection cleared".into();
    }
    if keys.just_pressed(KeyCode::KeyR) {
        *session = Session::default();
        session.message = "New game".into();
    }
}

/// Redraw pieces from the authoritative position.
///
/// Despawn-and-respawn rather than diffing: at 60 pieces it is not worth the
/// complexity, and it guarantees the view cannot drift from the model.
fn sync_pieces(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    existing: Query<(Entity, &PieceMarker)>,
    session: Res<Session>,
) {
    if !session.is_changed() {
        return;
    }

    // Nothing to do if every rendered piece still matches the position. The
    // hole tag is what makes that comparison possible.
    let rendered: Vec<Coord> = existing.iter().map(|(_, m)| m.hole).collect();
    let occupied: Vec<Coord> = all_holes()
        .into_iter()
        .filter(|c| session.game.position().occupant(*c).is_some())
        .collect();
    if rendered.len() == occupied.len() && rendered.iter().all(|c| occupied.contains(c)) {
        return;
    }

    for (e, _) in existing.iter() {
        commands.entity(e).despawn();
    }

    let mesh = meshes.add(Circle::new(PIECE_RADIUS));
    for c in all_holes() {
        let Some(player) = session.game.position().occupant(c) else {
            continue;
        };
        let p = coord_to_world(c);
        commands.spawn((
            Mesh2d(mesh.clone()),
            MeshMaterial2d(materials.add(player_colour(player))),
            Transform::from_xyz(p.x, p.y, 1.0),
            PieceMarker { hole: c },
        ));
    }
}

fn sync_highlights(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    targets: Query<Entity, With<TargetMarker>>,
    selection: Query<Entity, With<SelectionMarker>>,
    session: Res<Session>,
) {
    if !session.is_changed() {
        return;
    }
    for e in targets.iter().chain(selection.iter()) {
        commands.entity(e).despawn();
    }

    if let Some(sel) = session.selected {
        let ring = meshes.add(Annulus::new(PIECE_RADIUS + 2.0, PIECE_RADIUS + 5.0));
        let p = coord_to_world(sel);
        commands.spawn((
            Mesh2d(ring),
            MeshMaterial2d(materials.add(Color::WHITE)),
            Transform::from_xyz(p.x, p.y, 2.0),
            SelectionMarker,
        ));
    }

    let dot = meshes.add(Circle::new(HOLE_RADIUS * 0.8));
    let mat = materials.add(Color::srgba(1.0, 1.0, 1.0, 0.75));
    for t in &session.targets {
        let p = coord_to_world(*t);
        commands.spawn((
            Mesh2d(dot.clone()),
            MeshMaterial2d(mat.clone()),
            Transform::from_xyz(p.x, p.y, 2.0),
            TargetMarker,
        ));
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

    let moves = if session.game.is_over() {
        0
    } else {
        session.game.legal_moves().len()
    };

    **text = format!(
        "{header}\n{}\n{moves} legal move(s)  |  {} laws checked each turn\n\
         Click a piece, then a highlighted hole.  Esc clears, R restarts.",
        session.message,
        LAWS.len()
    );
}
