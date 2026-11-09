//! Coordinate labels: `O` writes every hole's `q,r` on the board, over
//! everything else drawn there, so one player can direct another — "move to
//! 2,-3" — by what both of them can read.
//!
//! The switch is the table's, not the peer's: in a shared round a press is
//! broadcast as [`NetMsg::Coordinates`], and every peer shows or hides the
//! labels with it. The message carries the new state rather than "toggle",
//! so peers that somehow disagreed agree again on the next press.

use bevy::prelude::*;
use bevy_matchbox::prelude::MatchboxSocket;
use checkers_core::geometry::{Coord, all_holes};
use checkers_net::{NetMsg, NetState, broadcast};

use crate::board_view::coord_to_world;
use crate::lobby::OUTLINE_OFFSETS;
use crate::{AppState, Session};

/// Whether the coordinate labels are shown. Starts hidden every round, so
/// every peer at a table starts the round agreeing on it.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoordLabels {
    pub shown: bool,
}

/// One hole's label.
#[derive(Component)]
#[require(DespawnOnExit<AppState> = DespawnOnExit(AppState::InGame))]
pub struct CoordLabel;

/// Depth of the labels: above the holes (0), pieces (1), camp rings and
/// highlights (up to 2), and a piece at the top of its flight (2.2).
pub const LABEL_Z: f32 = 10.0;

/// Glyph size in world units. A label must fit inside a piece: the widest,
/// `-4,-4`, is five monospace glyphs of ~0.6 em, which at 9 units is 27 —
/// the piece is 26 across, and the slack is the glyphs' side bearings.
const LABEL_SIZE: f32 = 9.0;

/// How much finer the glyphs are rasterised than they are drawn. `Text2d` is
/// rasterised at its font size in world units, and the camera magnifies the
/// board two- to fourfold; drawn at size, the labels would be a blur.
const OVERSAMPLE: f32 = 4.0;

/// Over a piece: every player's colour is bright, so dark text reads on all.
const ON_PIECE: Color = Color::srgb(0.06, 0.06, 0.08);
/// Over an empty hole: the holes and the backdrop are dark.
const ON_HOLE: Color = Color::srgb(0.86, 0.86, 0.90);
/// Around the light labels: an empty hole may carry a white destination dot
/// or the gold jump trail, which light text alone washes out on, so the
/// labels there get a black ring of copies, as the lobby's corner labels do.
/// In the glyphs' oversampled units, so about a screen pixel.
const OUTLINE_WIDTH: f32 = 3.0;

pub fn plugin(app: &mut App) {
    app.init_resource::<CoordLabels>()
        .add_systems(OnEnter(AppState::InGame), hide);
}

/// Every round starts with the labels hidden. `set_if_neq`, so a round that
/// was already hidden does not count as a change.
fn hide(mut labels: ResMut<CoordLabels>) {
    labels.set_if_neq(CoordLabels { shown: false });
}

/// A label's text: `q,r`, the status line's `(q,r)` without the brackets,
/// which would not fit inside a piece.
pub fn label_text(c: Coord) -> String {
    format!("{},{}", c.q, c.r)
}

/// `O` flips the labels — for every peer, in a shared round.
pub fn toggle(
    keys: Res<ButtonInput<KeyCode>>,
    mut labels: ResMut<CoordLabels>,
    session: Res<Session>,
    net: Res<NetState>,
    socket: Option<ResMut<MatchboxSocket>>,
) {
    if !keys.just_pressed(KeyCode::KeyO) {
        return;
    }
    labels.shown = !labels.shown;
    if session.shared
        && let Some(mut socket) = socket
    {
        let msg = NetMsg::Coordinates {
            shown: labels.shown,
        };
        broadcast(&mut socket, &net.peers, &msg);
    }
}

/// Redraw the labels from the position being displayed: dark over a piece,
/// light and outlined over an empty hole. Rebuilt wholesale when the switch
/// or the position changes, as the pieces are.
pub fn sync(
    mut commands: Commands,
    labels: Res<CoordLabels>,
    session: Res<Session>,
    stale: Query<Entity, With<CoordLabel>>,
) {
    if !labels.is_changed() && !session.is_changed() {
        return;
    }
    for e in stale.iter() {
        commands.entity(e).despawn();
    }
    if !labels.shown {
        return;
    }

    let position = session.display_position();
    for c in all_holes() {
        let occupied = position.occupant(c).is_some();
        let colour = if occupied { ON_PIECE } else { ON_HOLE };
        let p = coord_to_world(c);
        let mut label = commands.spawn((
            CoordLabel,
            glyphs(c, colour),
            Transform::from_xyz(p.x, p.y, LABEL_Z).with_scale(Vec3::splat(1.0 / OVERSAMPLE)),
        ));
        if !occupied {
            label.with_children(|ring| {
                for offset in OUTLINE_OFFSETS {
                    let at = (offset * OUTLINE_WIDTH).extend(-0.01);
                    ring.spawn((glyphs(c, Color::BLACK), Transform::from_translation(at)));
                }
            });
        }
    }
}

/// One copy of a hole's label text, in `colour`.
fn glyphs(c: Coord, colour: Color) -> impl Bundle {
    (
        Text2d::new(label_text(c)),
        TextFont {
            font_size: FontSize::Px(LABEL_SIZE * OVERSAMPLE),
            ..default()
        },
        TextColor(colour),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board_view::PIECE_RADIUS;

    /// Every label fits the five glyphs [`LABEL_SIZE`] was chosen for, so
    /// none spills past its piece.
    #[test]
    fn every_label_fits_inside_a_piece() {
        let widest = all_holes()
            .into_iter()
            .map(|c| label_text(c).chars().count())
            .max()
            .expect("the board has holes");
        assert_eq!(widest, 5, "LABEL_SIZE assumes five glyphs at most");
        assert!(widest as f32 * 0.6 * LABEL_SIZE <= 2.0 * PIECE_RADIUS + 1.0);
    }

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(bevy::state::app::StatesPlugin)
            .init_state::<AppState>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<Session>()
            .init_resource::<NetState>()
            .add_plugins(plugin)
            .add_systems(Update, (toggle, sync).chain());
        app
    }

    fn press_o(app: &mut App) {
        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        keys.release(KeyCode::KeyO);
        keys.clear();
        keys.press(KeyCode::KeyO);
        app.update();
    }

    fn labels(app: &mut App) -> Vec<(Vec3, Color)> {
        let world = app.world_mut();
        world
            .query_filtered::<(&Transform, &TextColor), With<CoordLabel>>()
            .iter(world)
            .map(|(t, c)| (t.translation, c.0))
            .collect()
    }

    /// `O` labels every hole, over everything else on the board, dark over
    /// the pieces and light over the empty holes; a second `O` takes the
    /// labels away.
    #[test]
    fn o_labels_every_hole_and_o_again_clears_them() {
        let mut app = app();
        app.update();
        assert!(labels(&mut app).is_empty(), "hidden until asked for");

        press_o(&mut app);
        let shown = labels(&mut app);
        assert_eq!(shown.len(), all_holes().len());
        assert!(shown.iter().all(|(at, _)| at.z == LABEL_Z));

        let position = app.world().resource::<Session>().display_position().clone();
        for c in all_holes() {
            let at = coord_to_world(c);
            let (_, colour) = shown
                .iter()
                .find(|(t, _)| t.truncate() == at)
                .expect("a label on every hole");
            let wanted = if position.occupant(c).is_some() {
                ON_PIECE
            } else {
                ON_HOLE
            };
            assert_eq!(*colour, wanted, "{c:?} is labelled in the wrong colour");
        }

        press_o(&mut app);
        assert!(labels(&mut app).is_empty(), "a second O hides them");
    }
}
