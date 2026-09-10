//! The lobby: one setup screen for every way a game can be played.
//!
//! The star's six wedges are the six camps; a game is whatever corners got
//! filled:
//!
//! * **Solo** (no peers in the room): the local [`Table`] says which corners
//!   a human plays and which this device's engine drives. Presets fill the
//!   symmetric seatings.
//! * **Shared room**: a click on a free wedge *is* the claim; the host may
//!   seat engines on free corners, un-seat players, and remove engines. The
//!   host starts once two or more corners are claimed. Peers who claimed
//!   nothing watch.
//!
//! The room lives in the page's URL ([`crate::web`]); the lobby only shows
//! it. The host is the peer with the lexicographically smallest `PeerId`,
//! recomputed every frame so host loss self-heals. The host owns the roster:
//! guests announce themselves with [`NetMsg::Hello`] and claim with
//! [`NetMsg::Claim`]; everything else is the host broadcasting
//! [`NetMsg::Roster`]. One authority, so no guest ever reconciles two
//! sources of truth about which player it commands.

use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::ui::RelativeCursorPosition;
use bevy_matchbox::prelude::*;
use checkers_net::{CH_RELIABLE, NetMsg, NetState, RoomId, Seat, broadcast, decode};

use crate::board_view::player_colour;
use crate::setup::Seating;
use crate::{AppState, Session};
use checkers_core::position::Player;
use checkers_core::rules::Variants;

/// Push the host's roster to every peer. The roster broadcast follows every
/// roster change, so it lives in one place.
fn publish_roster(socket: &mut MatchboxSocket, net: &NetState, peers: &[PeerId]) {
    broadcast(socket, peers, &NetMsg::Roster(net.seats.clone()));
}

/// Marker for everything spawned by the lobby, so leaving despawns it wholesale.
#[derive(Component)]
pub struct LobbyUi;

/// Every clickable control on the screen bar the star's own hit area.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub enum LobbyButton {
    Start,
    /// Fill the table with a symmetric setup. Solo-only: a shared table is
    /// claimed corner by corner, never overwritten by one peer's shortcut.
    Preset(Seating),
    /// Declare or switch off the "no piece may rest in a foreign camp" rule.
    ForeignCamps,
    /// Perform [`CornerCommand`] on the currently selected corner.
    CornerAction(CornerCommand),
}

/// Rows only the sequencing authority (the host, or any solo table) may use:
/// starting the game, the house rules, seating engines. Hidden from guests,
/// who could only press them to be refused.
#[derive(Component)]
pub struct HostOnly;

/// Rows that only make sense before the room is shared: the seating presets.
/// In a shared room every corner is claimed on the star instead.
#[derive(Component)]
pub struct SoloOnly;

/// What a [`LobbyButton::CornerAction`] press asks for, against the selected
/// corner.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub enum CornerCommand {
    Human,
    Cpu,
    Off,
}

/// One star petal: which camp it stands for. Clicking selects the corner.
#[derive(Component)]
pub struct CornerPetal(pub usize);

/// One star petal's text, carrying its corner so the label can follow the
/// state without being rebuilt.
#[derive(Component)]
pub struct CornerText(pub usize);

#[derive(Component)]
struct RosterText;

/// One corner's state, as configured locally. Only the solo setup writes this
/// resource; a networked table is read from the roster's seats instead.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum CornerState {
    /// No one sits here.
    #[default]
    Empty,
    /// A corner played by hand on this device.
    Human,
    /// A corner played by this device's engine.
    Cpu,
}

/// The six corners as configured by this device. Read when the game is dealt
/// on a solo setup; ignored while peers share the room.
#[derive(Resource, Debug, Clone, Default)]
pub struct Table(pub [CornerState; 6]);

/// The corner currently selected for action — which petal is highlighted.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SelectedCorner(pub Option<usize>);

/// The house-rule switches the host picked, set in the lobby and read when the
/// game starts. Same resource-over-field reasoning as [`Table`], and the same
/// rule: only the host decides, since the shared game must play under one rule
/// set for every peer.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ChosenVariants(pub Variants);

/// The one-line status message under the roster: why a press was refused,
/// what a press did. Lobby UI state, deliberately not part of [`NetState`] —
/// the wire state describes the room, not this screen.
#[derive(Resource, Debug, Clone, Default)]
pub struct LobbyStatus(pub String);

pub fn plugin(app: &mut App) {
    // The room comes from the URL — a share link lands you in the sender's
    // lobby, and a bare page is redirected to a fresh generated room so the
    // address bar is always shareable. Native builds read `CCHKRS_ROOM` and
    // simply generate when it is unset. The room is never edited afterwards.
    let room = crate::web::room_from_url().unwrap_or_else(crate::web::random_room);
    crate::web::share_room(&room);
    app.insert_resource(room)
        // The session's pet name is drawn at boot and never changed.
        .insert_resource(NetState {
            name: crate::web::petname(),
            ..NetState::default()
        })
        .init_resource::<Table>()
        .init_resource::<SelectedCorner>()
        .init_resource::<HoveredCorner>()
        .init_resource::<SectorArt>()
        .init_resource::<ChosenVariants>()
        .init_resource::<LobbyStatus>()
        .add_systems(
            OnEnter(AppState::Lobby),
            // The wedge art must exist before the star can reference it.
            (checkers_net::open_socket, ensure_sector_art, spawn).chain(),
        )
        .add_systems(OnExit(AppState::Lobby), despawn)
        .add_systems(
            Update,
            (
                // Lobby machinery stays out of the game: ungated, `pump_socket`
                // raced `net::pump` for the same socket. Everything here is
                // lobby furniture.
                (
                    elect_host,
                    pump_socket,
                    select_corner,
                    handle_buttons,
                    broadcast_cursor,
                )
                    .chain()
                    .run_if(in_state(AppState::Lobby)),
                (
                    sync_button_styles,
                    // Host-only and solo-only rows fold away for the players
                    // who could not use them; the corner rows follow the
                    // selection.
                    sync_host_rows,
                    sync_corner_actions,
                    hover_corner,
                    // The wedge tint follows the state, and the labels follow
                    // the roster; both after the hover is settled this frame.
                    (
                        sync_corner_styles,
                        draw_corner_labels,
                        draw_roster,
                        sync_remote_cursors,
                    )
                        .chain(),
                )
                    .chain()
                    .run_if(in_state(AppState::Lobby)),
            ),
        );
}

/// Paint every non-petal button: selected mode, hover, press.
///
/// Runs every frame and writes only real changes, so hover repaints
/// immediately without dirtying the UI otherwise.
fn sync_button_styles(
    variants: Res<ChosenVariants>,
    net: Res<NetState>,
    table: Res<Table>,
    selected: Res<SelectedCorner>,
    mut buttons: Query<(&Interaction, &LobbyButton, &mut BackgroundColor)>,
) {
    let solo = net.peers.is_empty();
    let active = selected.0.map(|i| {
        let p = Player::new(i as u8).expect("corner indices are below six");
        let cmd = if solo {
            match &table.0[i] {
                CornerState::Empty => CornerCommand::Off,
                CornerState::Human => CornerCommand::Human,
                CornerState::Cpu => CornerCommand::Cpu,
            }
        } else {
            match net.seats.iter().find(|s| s.player == Some(i as u32)) {
                // A free corner reads as its empty state.
                None => CornerCommand::Off,
                // An occupied corner is an engine seat or a human claim.
                Some(seat) if seat.engine => CornerCommand::Cpu,
                Some(_) => CornerCommand::Human,
            }
        };
        (p, cmd)
    });

    for (interaction, button, mut bg) in buttons.iter_mut() {
        let selected = match button {
            LobbyButton::ForeignCamps => variants.0.forbid_foreign_camps,
            LobbyButton::CornerAction(cmd) => active.is_some_and(|(_, a)| a == *cmd),
            LobbyButton::Start | LobbyButton::Preset(_) => false,
        };
        let colour = match interaction {
            Interaction::Pressed if selected => CHOSEN_DOWN,
            Interaction::Pressed => DOWN,
            Interaction::Hovered if selected => CHOSEN_HOVER,
            Interaction::Hovered => HOVER,
            Interaction::None if selected => CHOSEN,
            Interaction::None => IDLE,
        };
        if bg.0 != colour {
            bg.0 = colour;
        }
    }
}

/// Fold rows away for the players who could only press them to be refused:
/// host-only rows (Start, the house rules, seating an engine) vanish for
/// guests, and the solo presets vanish once the room is shared.
fn sync_host_rows(
    net: Res<NetState>,
    mut rows: Query<(&mut Visibility, AnyOf<(&HostOnly, &SoloOnly)>)>,
) {
    let host = net.sequences();
    let solo = net.peers.is_empty();
    for (mut visibility, (host_only, solo_only)) in &mut rows {
        let wanted = if host_only.is_some() && host || solo_only.is_some() && solo {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *visibility != wanted {
            *visibility = wanted;
        }
    }
}

/// One button. Factored out because the lobby spawns rows of them and the
/// padding, radius, and text styling must not drift between the rows.
fn button(parent: &mut ChildSpawnerCommands, label: &str, tag: LobbyButton) {
    parent
        .spawn((
            Button,
            Node {
                padding: UiRect::axes(Val::Px(14.0), Val::Px(8.0)),
                border_radius: BorderRadius::all(Val::Px(5.0)),
                ..default()
            },
            BackgroundColor(IDLE),
            tag,
        ))
        .with_child((
            Text::new(label),
            TextFont {
                font_size: FontSize::Px(15.0),
                ..default()
            },
            TextColor(Color::srgb(0.9, 0.9, 0.92)),
        ));
}

/// The shared palette. `CHOSEN` marks the button whose mode is active, with
/// its own hover and press.
pub(crate) const IDLE: Color = Color::srgb(0.22, 0.22, 0.27);
pub(crate) const HOVER: Color = Color::srgb(0.29, 0.29, 0.35);
pub(crate) const DOWN: Color = Color::srgb(0.17, 0.17, 0.21);
pub(crate) const CHOSEN: Color = Color::srgb(0.20, 0.45, 0.28);
pub(crate) const CHOSEN_HOVER: Color = Color::srgb(0.25, 0.53, 0.34);
pub(crate) const CHOSEN_DOWN: Color = Color::srgb(0.16, 0.37, 0.23);

/// The lobby is a two-column flex row filling the window: the star on the
/// left, every control on the right. Side by side the columns stay short
/// enough to fit a 600px-tall window by construction, which a single stacked
/// column never could.
fn spawn(mut commands: Commands, art: Res<SectorArt>, net: Res<NetState>, room: Res<RoomId>) {
    commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                column_gap: Val::Px(32.0),
                padding: UiRect::all(Val::Px(24.0)),
                ..default()
            },
            LobbyUi,
        ))
        .with_children(|root| {
            // Left: the setup itself.
            root.spawn(Node {
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(12.0),
                ..default()
            })
            .with_children(|col| {
                header(col, "Lobby");
                col.spawn((
                    Text::new("The star is the setup: click a corner, then set who sits there."),
                    TextFont {
                        font_size: FontSize::Px(14.0),
                        ..default()
                    },
                    TextColor(Color::srgb(0.62, 0.62, 0.68)),
                ));

                // The hex star. Wedges are out-facing triangle sectors over a
                // single hit area; digits 1..6 select the same corners.
                star(col, &art);
            });

            // Right: what to do with it.
            root.spawn(Node {
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(10.0),
                ..default()
            })
            .with_children(|col| {
                // The room you are in and the name you go by — both fixed for
                // the session, so they are baked in at spawn: the room because
                // the page's link is the invitation, the name because the
                // roster shows it.
                header(col, "Room");
                col.spawn((
                    Text::new(format!("{} - you are {}", room.0, net.name)),
                    TextFont {
                        font_size: FontSize::Px(14.0),
                        ..default()
                    },
                    TextColor(Color::srgb(0.88, 0.88, 0.9)),
                ));
                col.spawn(Node {
                    column_gap: Val::Px(10.0),
                    ..default()
                })
                .with_children(|hint| {
                    hint.spawn((
                        Text::new("Share this page's link to invite players."),
                        TextFont {
                            font_size: FontSize::Px(13.0),
                            ..default()
                        },
                        TextColor(Color::srgb(0.62, 0.62, 0.68)),
                    ));
                });

                header(col, "Table");
                // Presets are shortcuts for the symmetric setups; they fill the
                // whole table, so what is configured and what the shortcut
                // leaves can never silently disagree. Solo-only: a shared
                // table is claimed corner by corner on the star.
                col.spawn((
                    Node {
                        column_gap: Val::Px(10.0),
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    SoloOnly,
                ))
                .with_children(|row| {
                    for seating in Seating::ALL {
                        button(
                            row,
                            &format!("Preset {}", seating.label()),
                            LobbyButton::Preset(seating),
                        );
                    }
                });

                // What to do with the selected corner. [`sync_corner_actions`]
                // shows exactly one of these rows: a free corner offers
                // seating, a claimed corner offers cancelling it.
                col.spawn((Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(10.0),
                    ..default()
                },))
                    .with_children(|corner| {
                        // Seat a free corner. Seating an engine is the host's
                        // call alone — the engine runs in the host's process —
                        // so guests never see the button.
                        corner
                            .spawn((
                                Node {
                                    column_gap: Val::Px(10.0),
                                    align_items: AlignItems::Center,
                                    ..default()
                                },
                                SeatButtons,
                            ))
                            .with_children(|row| {
                                button(
                                    row,
                                    "Human",
                                    LobbyButton::CornerAction(CornerCommand::Human),
                                );
                                row.spawn((
                                    Button,
                                    Node {
                                        padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                                        border_radius: BorderRadius::all(Val::Px(5.0)),
                                        ..default()
                                    },
                                    BackgroundColor(IDLE),
                                    LobbyButton::CornerAction(CornerCommand::Cpu),
                                    HostOnly,
                                ))
                                .with_child((
                                    Text::new("Computer"),
                                    TextFont {
                                        font_size: FontSize::Px(14.0),
                                        ..default()
                                    },
                                    TextColor(Color::srgb(0.9, 0.9, 0.92)),
                                ));
                            });

                        // Cancel a claimed corner — yours (release), another
                        // player's (host un-seats), or an engine's (host removes
                        // it). [`corner_effect`] picks the right move.
                        corner
                            .spawn((
                                Node {
                                    column_gap: Val::Px(10.0),
                                    align_items: AlignItems::Center,
                                    ..default()
                                },
                                CancelSeat,
                            ))
                            .with_children(|row| {
                                button(
                                    row,
                                    "Cancel Seat",
                                    LobbyButton::CornerAction(CornerCommand::Off),
                                );
                            });
                    });

                // House rules, one toggle per switch. Only the host decides
                // them, so only the host even sees them.
                col.spawn((
                    Node {
                        margin: UiRect::top(Val::Px(6.0)),
                        ..default()
                    },
                    HostOnly,
                ))
                .with_children(|row| {
                    row.spawn((
                        Text::new("Rules"),
                        TextFont {
                            font_size: FontSize::Px(16.0),
                            ..default()
                        },
                        TextColor(Color::srgb(0.7, 0.7, 0.76)),
                    ));
                });
                col.spawn((
                    Node {
                        column_gap: Val::Px(10.0),
                        ..default()
                    },
                    HostOnly,
                ))
                .with_children(|row| {
                    button(row, "No foreign rest", LobbyButton::ForeignCamps);
                });

                // The roster: who is here and where they sit.
                col.spawn((
                    Text::new(String::new()),
                    TextFont {
                        font_size: FontSize::Px(14.0),
                        ..default()
                    },
                    TextColor(Color::srgb(0.88, 0.88, 0.9)),
                    RosterText,
                ));

                // The host starts the game — the one button guests must not
                // even see, since a shared start belongs to the host alone.
                col.spawn((
                    Node {
                        column_gap: Val::Px(10.0),
                        margin: UiRect::top(Val::Px(4.0)),
                        ..default()
                    },
                    HostOnly,
                ))
                .with_children(|row| {
                    button(row, "Start (Enter)", LobbyButton::Start);
                });
            });
        });
}

/// Is corner `i` claimed? Solo tables read the local [`Table`]; shared rooms
/// read the roster. The one occupancy rule every view and button row asks.
fn corner_is_filled(net: &NetState, table: &Table, i: usize) -> bool {
    if net.peers.is_empty() {
        table.0[i] != CornerState::Empty
    } else {
        net.seats.iter().any(|s| s.player == Some(i as u32))
    }
}

/// Marker on the Human / Computer row: shown while the selected corner is
/// free, so it offers seating.
#[derive(Component)]
struct SeatButtons;

/// Marker on the Cancel Seat row: shown while the selected corner is claimed.
#[derive(Component)]
struct CancelSeat;

/// Show exactly one of the two corner rows, according to the selected corner:
/// a free corner offers seating, a claimed corner offers cancelling it. With
/// nothing selected both fold away — an empty star has nothing to act on, and
/// a guest that claimed nothing is a spectator to the sidebar.
fn sync_corner_actions(
    net: Res<NetState>,
    table: Res<Table>,
    selected: Res<SelectedCorner>,
    mut seat_rows: Query<&mut Visibility, (With<SeatButtons>, Without<CancelSeat>)>,
    mut cancel_rows: Query<&mut Visibility, (With<CancelSeat>, Without<SeatButtons>)>,
) {
    let claimed = selected
        .0
        .is_some_and(|i| corner_is_filled(&net, &table, i));
    let seat_wanted = selected.0.is_some() && !claimed;
    for mut visibility in &mut seat_rows {
        let wanted = if seat_wanted {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *visibility != wanted {
            *visibility = wanted;
        }
    }
    for mut visibility in &mut cancel_rows {
        let wanted = if claimed {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *visibility != wanted {
            *visibility = wanted;
        }
    }
}

/// Seconds between cursor broadcasts, and how long a silent cursor stays on
/// screen before it is hidden as gone.
const CURSOR_INTERVAL: f32 = 0.1;
const CURSOR_LINGER_SECS: f64 = 3.0;

/// A remote peer's pointer drawn over the lobby: where it was last reported
/// (already scaled into this window's logical pixels), where it is drawn
/// (eased toward the report so it glides rather than jumps), and when it was
/// last heard from.
#[derive(Component)]
pub struct RemoteCursor {
    peer: String,
    target: Vec2,
    display: Vec2,
    last_seen: f64,
}

/// Broadcast this pointer at a lazy 10 Hz while the lobby is up, as
/// **fractions of the window's width and height** (0..1, origin top-left).
/// Windows differ in size, so absolute pixels would drift apart across peers;
/// fractions land every pointer at the same relative spot, and each receiver
/// scales them back into its own logical pixels — the space bevy_ui lays out
/// in. Matchbox connects every peer to every peer, so a plain broadcast
/// reaches the whole room; the sender is the `from` on arrival and no origin
/// field is needed.
fn broadcast_cursor(
    time: Res<Time>,
    windows: Query<&Window>,
    net: Res<NetState>,
    socket: Option<ResMut<MatchboxSocket>>,
    mut next_at: Local<f32>,
) {
    let Some(mut socket) = socket else {
        return;
    };
    if net.peers.is_empty() || time.elapsed_secs() < *next_at {
        return;
    }
    *next_at = time.elapsed_secs() + CURSOR_INTERVAL;
    let Some(window) = windows.single().ok() else {
        return;
    };
    let Some(pos) = window.cursor_position() else {
        return;
    };
    let (width, height) = (window.width(), window.height());
    if width <= 0.0 || height <= 0.0 {
        return;
    }
    broadcast(
        &mut socket,
        &net.peers,
        &NetMsg::Cursor {
            pos: [pos.x / width, pos.y / height],
        },
    );
}

/// A cursor's colour is the corner its peer claimed — the same colour the
/// wedge wears — and its label the roster name.
fn cursor_identity(net: &NetState, peer: &str) -> (Color, String) {
    match net.seats.iter().find(|s| s.peer == peer) {
        Some(seat) => {
            let colour = seat
                .player
                .and_then(|i| Player::new(i as u8))
                .map(player_colour)
                .unwrap_or(IDLE);
            (colour, seat.name.clone())
        }
        None => (IDLE, format!("peer {}", &peer[..peer.len().min(4)])),
    }
}

/// Draw every remote cursor where its peer's pointer is, and keep the label
/// and colour following the roster.
#[allow(clippy::type_complexity)]
fn sync_remote_cursors(
    time: Res<Time>,
    net: Res<NetState>,
    mut cursors: Query<(
        &mut RemoteCursor,
        &mut Node,
        &mut BackgroundColor,
        &mut Visibility,
        &Children,
    )>,
    mut labels: Query<(&mut Text, &mut TextColor), Without<RemoteCursor>>,
) {
    // Exponential smoothing: fast enough to follow, slow enough to hide
    // the 10 Hz steps.
    let alpha = 1.0 - (-6.0 * time.delta_secs()).exp();
    let now = time.elapsed_secs_f64();
    for (mut cursor, mut node, mut bg, mut vis, kids) in cursors.iter_mut() {
        cursor.display = cursor.display.lerp(cursor.target, alpha);
        node.left = Val::Px(cursor.display.x);
        node.top = Val::Px(cursor.display.y);
        let want = if now - cursor.last_seen > CURSOR_LINGER_SECS {
            Visibility::Hidden
        } else {
            Visibility::Visible
        };
        if *vis != want {
            *vis = want;
        }
        let (colour, label) = cursor_identity(&net, &cursor.peer);
        if bg.0 != colour {
            bg.0 = colour;
        }
        for kid in kids.iter() {
            if let Ok((mut text, mut text_colour)) = labels.get_mut(kid) {
                if **text != label {
                    **text = label.clone();
                }
                if text_colour.0 != colour {
                    text_colour.0 = colour;
                }
            }
        }
    }
}

/// A section heading in the lobby.
fn header(parent: &mut ChildSpawnerCommands, label: &str) {
    parent.spawn((
        Text::new(label),
        TextFont {
            font_size: FontSize::Px(20.0),
            ..default()
        },
        TextColor(Color::srgb(0.92, 0.92, 0.95)),
    ));
}

/// Star geometry: the container the wedges and their labels live in.
const STAR_W: f32 = 420.0;
const STAR_H: f32 = 360.0;

/// Wedge geometry: each corner is an **equilateral** triangle pointing
/// outward, like the board's camp triangles. With the apex at
/// [`WEDGE_OUTER`] on the camp's axis and the base corners on the ±30° rays
/// at `WEDGE_OUTER / √3`, the triangle is exactly equilateral and the six
/// wedges meet corner-to-corner in a rosette — the star's silhouette — while
/// the middle stays open. Labels may overhang the wedges.
const WEDGE_OUTER: f32 = 175.0;

/// Corner `i`'s direction, in container coordinates (y down), so the angle
/// arithmetic matches [`Window::cursor_position`] directly.
fn wedge_angle(i: usize) -> f32 {
    (60.0 * i as f32 - 90.0).to_radians()
}

/// The three corners of corner `i`'s wedge, in container-local pixels: the
/// outward tip first, then the two base corners. Apex on the axis at
/// [`WEDGE_OUTER`], base corners on the ±30° rays at `WEDGE_OUTER / √3` —
/// the arrangement that makes the triangle equilateral.
fn wedge_vertices(i: usize) -> [Vec2; 3] {
    let t = wedge_angle(i);
    let beta = 30f32.to_radians();
    let centre = Vec2::new(STAR_W / 2.0, STAR_H / 2.0);
    let apex = centre + WEDGE_OUTER * Vec2::new(t.cos(), t.sin());
    let base_r = WEDGE_OUTER / 3.0f32.sqrt();
    let left = centre + base_r * Vec2::new((t - beta).cos(), (t - beta).sin());
    let right = centre + base_r * Vec2::new((t + beta).cos(), (t + beta).sin());
    [apex, left, right]
}

/// Which side of the line `a -> b` the point `p` is on, as a signed area.
fn cross(a: Vec2, b: Vec2, p: Vec2) -> f32 {
    (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x)
}

/// Is `p` inside the triangle `v`? Sign tests, so an exact wedge hit needs no
/// rounding: the click area is the triangle, not its bounding box.
fn point_in_triangle(p: Vec2, v: [Vec2; 3]) -> bool {
    let (s1, s2, s3) = (
        cross(v[0], v[1], p),
        cross(v[1], v[2], p),
        cross(v[2], v[0], p),
    );
    (s1 >= 0.0 && s2 >= 0.0 && s3 >= 0.0) || (s1 <= 0.0 && s2 <= 0.0 && s3 <= 0.0)
}

/// Is container-local `p` inside corner `i`'s wedge?
fn wedge_contains(i: usize, p: Vec2) -> bool {
    point_in_triangle(p, wedge_vertices(i))
}

/// Which corner the cursor is over. `normalized` is [`RelativeCursorPosition`]'s
/// centre-relative position (`-0.5 .. 0.5`, y down); it is mapped back into
/// container coordinates so the wedges' own geometry can answer. `None` over
/// the empty middle — the middle is the star, not a button.
fn sector_at(normalized: Vec2) -> Option<usize> {
    let local = normalized * vec2(STAR_W, STAR_H) + vec2(STAR_W / 2.0, STAR_H / 2.0);
    (0..6).find(|&i| wedge_contains(i, local))
}

/// Where corner `i`'s two label lines sit: the wedge's centroid.
fn label_pos(i: usize) -> Vec2 {
    let v = wedge_vertices(i);
    (v[0] + v[1] + v[2]) / 3.0
}

/// The six wedge textures, rasterized once on first lobby entry: white where
/// the wedge is, transparent elsewhere, so the [`ImageNode`] tint paints the
/// colour and the hover and selection shading stay a colour write.
#[derive(Resource, Default)]
struct SectorArt(Option<[Handle<Image>; 6]>);

/// Rasterize corner `i`'s wedge into a full-star-size RGBA texture.
fn sector_image(i: usize) -> Image {
    let (w, h) = (STAR_W as u32, STAR_H as u32);
    let mut data = vec![0u8; (w * h * 4) as usize];
    let v = wedge_vertices(i);
    for y in 0..h {
        for x in 0..w {
            if point_in_triangle(Vec2::new(x as f32 + 0.5, y as f32 + 0.5), v) {
                let o = ((y * w + x) * 4) as usize;
                data[o..o + 4].copy_from_slice(&[255, 255, 255, 255]);
            }
        }
    }
    Image::new(
        Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// Build [`SectorArt`] once; a no-op on every lobby re-entry.
fn ensure_sector_art(mut art: ResMut<SectorArt>, mut images: ResMut<Assets<Image>>) {
    if art.0.is_some() {
        return;
    }
    let handles = std::array::from_fn(|i| images.add(sector_image(i)));
    art.0 = Some(handles);
}

/// The eight offsets the label's black border copies take around the white
/// fill (a ~1px ring). Bevy's text pipeline has no stroke, so a legible
/// border over the coloured wedges is a shadow ring of copies.
const OUTLINE_OFFSETS: [Vec2; 8] = [
    Vec2::new(1.0, 0.0),
    Vec2::new(-1.0, 0.0),
    Vec2::new(0.0, 1.0),
    Vec2::new(0.0, -1.0),
    Vec2::new(0.71, 0.71),
    Vec2::new(0.71, -0.71),
    Vec2::new(-0.71, 0.71),
    Vec2::new(-0.71, -0.71),
];

/// One outlined label line: the white fill copy on top, black copies offset
/// around it underneath. All copies carry the same string —
/// [`draw_corner_labels`] writes it into every one via the [`CornerText`]
/// marker on the stack. Each copy stretches the stack's width and centres its
/// glyphs, so the copies cannot drift apart.
fn outlined_line(parent: &mut ChildSpawnerCommands, tag: usize, font_size: f32, fill: Color) {
    parent
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Px(font_size * 1.3),
                ..default()
            },
            CornerText(tag),
        ))
        .with_children(|stack| {
            for offset in OUTLINE_OFFSETS {
                outlined_copy(stack, offset, font_size, Color::BLACK);
            }
            outlined_copy(stack, Vec2::ZERO, font_size, fill);
        });
}

/// One copy of an outlined label: a full-size flex box, shifted by `offset`,
/// with the text dead-centred inside it. Making the box fill the stack on
/// every side (`top: y` with `bottom: -y`) keeps it full-height while the
/// whole line slides by the outline offset, so the glyphs stay centred — the
/// flex centring below is exact and needs no knowledge of the font's line
/// metrics.
fn outlined_copy(stack: &mut ChildSpawnerCommands, offset: Vec2, font_size: f32, colour: Color) {
    stack
        .spawn(Node {
            position_type: PositionType::Absolute,
            left: Val::Px(offset.x),
            right: Val::Px(-offset.x),
            top: Val::Px(offset.y),
            bottom: Val::Px(-offset.y),
            display: Display::Flex,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        })
        .with_child((
            Text::new(String::new()),
            TextFont {
                font_size: FontSize::Px(font_size),
                ..default()
            },
            TextLayout::new(Justify::Center, LineBreak::NoWrap),
            TextColor(colour),
        ));
}

/// The star: a fixed container holding one out-facing wedge per corner and its
/// two label lines. The container itself is the only button — hit-testing is
/// angular, against [`sector_at`] — and the middle stays empty, which is what
/// makes the star read as a star.
fn star(parent: &mut ChildSpawnerCommands, art: &SectorArt) {
    let handles = art
        .0
        .as_ref()
        .expect("sector art is built before the lobby spawns");
    parent
        .spawn(Node {
            width: Val::Px(STAR_W),
            height: Val::Px(STAR_H),
            ..default()
        })
        .with_children(|node| {
            node.spawn((
                Button,
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(0.0),
                    top: Val::Px(0.0),
                    width: Val::Px(STAR_W),
                    height: Val::Px(STAR_H),
                    ..default()
                },
                StarHit,
                RelativeCursorPosition::default(),
            ));
            for (i, handle) in handles.iter().enumerate() {
                node.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(0.0),
                        top: Val::Px(0.0),
                        width: Val::Px(STAR_W),
                        height: Val::Px(STAR_H),
                        ..default()
                    },
                    ImageNode {
                        image: handle.clone(),
                        ..default()
                    },
                    CornerPetal(i),
                ));
            }
            for i in 0..6 {
                let mid = label_pos(i);
                node.spawn(Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(mid.x - 55.0),
                    top: Val::Px(mid.y - 18.0),
                    width: Val::Px(110.0),
                    height: Val::Px(36.0),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    row_gap: Val::Px(1.0),
                    ..default()
                })
                .with_children(|label| {
                    // White fill over a black border, so the labels stay
                    // legible over whichever colour the wedge wears.
                    outlined_line(label, i * 2, 14.0, Color::srgb(0.96, 0.96, 0.97));
                    outlined_line(label, i * 2 + 1, 11.0, Color::srgb(0.85, 0.85, 0.88));
                });
            }
        });
}

/// Marker on the star's single hit area: the whole container is one button,
/// and clicks are resolved to a corner by angle, not by rectangles.
#[derive(Component)]
pub struct StarHit;

/// Which corner the cursor currently rests on, for wedge shading.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
struct HoveredCorner(Option<usize>);

fn despawn(mut commands: Commands, ui: Query<Entity, With<LobbyUi>>) {
    for e in ui.iter() {
        commands.entity(e).despawn();
    }
}

/// What pressing Enter should do.
///
/// Split out from `handle_buttons` because it is the part that decides whether
/// a board can exist at all: solo or shared, the board is only spawned on
/// entering [`AppState::InGame`], so a refusal that cannot be acted on looks
/// like a blank screen.
#[derive(Debug, PartialEq, Eq)]
pub enum StartDecision {
    /// Nobody else is here: deal the configured corners on this device.
    Solo,
    /// The room is full enough and ready: tell the peers and begin.
    Multiplayer,
    /// Refuse, with a reason to show the player.
    Refuse(String),
}

pub fn start_decision(net: &NetState, table: &Table) -> StartDecision {
    let configured = table.0.iter().filter(|c| **c != CornerState::Empty).count();

    if net.peers.is_empty() {
        if configured < 2 {
            return StartDecision::Refuse(
                "At least two corners must be filled before the game can start.".into(),
            );
        }
        return StartDecision::Solo;
    }

    // A guest cannot start a *shared* game, and must be told so rather than
    // having Enter do nothing.
    if !net.sequences() {
        return StartDecision::Refuse(
            "Only the host can start a shared game. Claim a corner and wait.".into(),
        );
    }

    let seated: Vec<&Seat> = net.seats.iter().filter(|s| s.player.is_some()).collect();
    if seated.len() < 2 {
        return StartDecision::Refuse(
            "At least two corners must be claimed before the game can start.".into(),
        );
    }
    StartDecision::Multiplayer
}

/// Smallest `PeerId` hosts. Recomputed every frame so host loss self-heals.
///
/// `pub` so the multiplayer integration test can run the real election in a
/// headless instance, exactly as the app schedules it.
pub fn elect_host(socket: Option<ResMut<MatchboxSocket>>, mut net: ResMut<NetState>) {
    let Some(mut socket) = socket else {
        return;
    };

    crate::net::sync_peers(&mut socket, &mut net);

    if net.my_id.is_none() {
        net.my_id = socket.id();
    }
    let Some(me) = net.my_id else {
        return;
    };

    let was_host = net.is_host;
    net.is_host = net.peers.iter().all(|p| me.to_string() < p.to_string());

    if net.is_host && !was_host {
        net.next_seq = net.last_applied_seq.map_or(0, |s| s + 1);
        info!("became host");
    }
}

/// The whole lobby conversation, one message at a time: greetings, corner
/// claims, the host's roster broadcasts, and the `Start` that moves everyone
/// into the game. Runs on the socket every frame.
///
/// `pub` so the multiplayer integration test can run the real pump in a
/// headless instance, exactly as the app schedules it.
#[allow(clippy::too_many_arguments)]
pub fn pump_socket(
    socket: Option<ResMut<MatchboxSocket>>,
    mut net: ResMut<NetState>,
    mut status: ResMut<LobbyStatus>,
    mut variants: ResMut<ChosenVariants>,
    mut next_state: ResMut<NextState<AppState>>,
    state: Res<State<AppState>>,
    mut commands: Commands,
    time: Res<Time>,
    windows: Query<&Window>,
    mut cursors: Query<(Entity, &mut RemoteCursor)>,
) {
    let Some(mut socket) = socket else {
        return;
    };

    // Announce ourselves to peers we have not greeted yet: once when we first
    // have a name, and afterwards only to peers that join later.
    let peers = net.peers.clone();
    let me = net.my_id.map(|id| id.to_string()).unwrap_or_default();

    // The host prunes seats whose peer has left the mesh: a refresh (or a
    // crashed tab) mints a fresh peer id, so the old seat would otherwise sit
    // in the roster forever, and the list only ever grew. Publish only on an
    // actual change, so an idle room costs nothing.
    if net.sequences() {
        let before = net.seats.clone();
        prune_departed(&mut net);
        if net.seats != before {
            publish_roster(&mut socket, &net, &peers);
        }
    }

    if net.name.is_empty() && !me.is_empty() {
        net.name = format!("player-{}", &me[..me.len().min(4)]);
    }
    let unacquainted: Vec<PeerId> = peers
        .iter()
        .filter(|p| !net.greeted.contains(p))
        .copied()
        .collect();
    if !net.name.is_empty() && !unacquainted.is_empty() {
        let hello = NetMsg::Hello {
            name: net.name.clone(),
        };
        broadcast(&mut socket, &unacquainted, &hello);
        // The host's own seat is not created by a Hello it never receives.
        if net.sequences() {
            let name = net.name.clone();
            seat_for(&mut net, &me, &name);
        }
        net.greeted.extend(unacquainted);
    }

    let inbox: Vec<(PeerId, Box<[u8]>)> = socket.channel_mut(CH_RELIABLE).receive();
    for (from, raw) in inbox {
        let Some(msg) = decode(&raw) else {
            continue;
        };
        match msg {
            NetMsg::Hello { name } => {
                if net.sequences() {
                    // A repeat Hello from a known peer is a rename, not a
                    // duplicate join: the name field re-greets precisely so
                    // this happens.
                    if let Some(seat) = net.seats.iter_mut().find(|s| s.peer == from.to_string()) {
                        if seat.name != name {
                            seat.name = name;
                            publish_roster(&mut socket, &net, &peers);
                        }
                    } else {
                        seat_for(&mut net, &from.to_string(), &name);
                        publish_roster(&mut socket, &net, &peers);
                    }
                }
            }
            NetMsg::Claim(claim) => {
                if net.sequences() {
                    let key = from.to_string();
                    // Read the roster before mutating it: the grant must see
                    // the corners as the peers do, and the sender's own hold.
                    let claims = match claim {
                        Some(c) if c < 6 => {
                            let free = !net.seats.iter().any(|s| s.player == Some(c));
                            let mine = net
                                .seats
                                .iter()
                                .any(|s| s.peer == key && s.player == Some(c));
                            Some((c, free, mine))
                        }
                        _ => None,
                    };
                    match claims {
                        Some((corner, free, mine)) => {
                            if let Some(seat) = net.seats.iter_mut().find(|s| s.peer == key) {
                                // Grant a free corner, or the sender's own; a
                                // move to a new corner frees the old one on the
                                // same grant.
                                if free || mine {
                                    if !mine {
                                        seat.player = None;
                                    }
                                    seat.player = Some(corner);
                                    status.0 = format!("{} claimed corner {corner}.", seat.name);
                                }
                            }
                        }
                        // Releasing, or an impossible corner number: drop the claim.
                        _ => {
                            if let Some(seat) = net.seats.iter_mut().find(|s| s.peer == key) {
                                seat.player = None;
                            }
                        }
                    }
                    publish_roster(&mut socket, &net, &peers);
                }
            }
            // Guests take the host's roster verbatim; it is the only authority.
            NetMsg::Roster(seats) => net.seats = seats,
            // The host's rule switch, live. The host is the only authority, so
            // a sequencing peer ignores its own echo; guests take it verbatim.
            NetMsg::Variants {
                forbid_foreign_camps,
            } => {
                if !net.sequences() {
                    variants.0.forbid_foreign_camps = forbid_foreign_camps;
                }
            }
            // A peer's pointer, drawn over the lobby only — the game has its
            // own presentation, and leftover dots must not haunt it. The wire
            // carries fractions of the sender's window; scale them into this
            // window's pixels so the dot sits at the same relative spot.
            NetMsg::Cursor { pos } => {
                if *state.get() != AppState::Lobby {
                    continue;
                }
                let Some(window) = windows.single().ok() else {
                    continue;
                };
                let pos = Vec2::new(pos[0] * window.width(), pos[1] * window.height());
                let key = from.to_string();
                let now = time.elapsed_secs_f64();
                if let Some((_, mut cursor)) = cursors.iter_mut().find(|(_, c)| c.peer == key) {
                    cursor.target = pos;
                    cursor.last_seen = now;
                } else {
                    let (colour, label) = cursor_identity(&net, &key);
                    commands
                        .spawn((
                            Node {
                                position_type: PositionType::Absolute,
                                left: Val::Px(pos.x),
                                top: Val::Px(pos.y),
                                width: Val::Px(10.0),
                                height: Val::Px(10.0),
                                border_radius: BorderRadius::all(Val::Px(5.0)),
                                ..default()
                            },
                            BackgroundColor(colour),
                            LobbyUi,
                            RemoteCursor {
                                peer: key,
                                target: pos,
                                display: pos,
                                last_seen: now,
                            },
                        ))
                        .with_children(|dot| {
                            dot.spawn((
                                Node {
                                    position_type: PositionType::Absolute,
                                    left: Val::Px(12.0),
                                    top: Val::Px(-4.0),
                                    ..default()
                                },
                                Text::new(label),
                                TextFont {
                                    font_size: FontSize::Px(12.0),
                                    ..default()
                                },
                                TextColor(colour),
                            ));
                        });
                }
            }
            NetMsg::Start {
                seats,
                forbid_foreign_camps,
                ..
            } => {
                net.seats = seats;
                variants.0.forbid_foreign_camps = forbid_foreign_camps;
                next_state.set(AppState::InGame);
            }
            // Moves cannot arrive before the game starts, but a late duplicate
            // from a previous game in the same room could. Ignore rather than
            // mis-apply.
            NetMsg::Move(_) | NetMsg::Sequenced { .. } => {}
        }
    }
}

/// Drop seats whose peer has left the room, and greeting memory of peers that
/// are gone. A refresh mints a fresh `PeerId`, so without this the old seat
/// would sit in the roster forever. Engine seats have no peer behind them —
/// they belong to the host — so they survive, and so does the host's own
/// seat: matchbox's peer list is *other* peers and never names yourself.
/// Pure, so the rule is testable without a socket; the host runs it every
/// lobby frame.
fn prune_departed(net: &mut NetState) {
    let me = net.my_id.as_ref().map(|id| id.to_string());
    net.seats.retain(|s| {
        s.engine
            || Some(&s.peer) == me.as_ref()
            || net.peers.iter().any(|p| p.to_string() == s.peer)
    });
    net.greeted.retain(|p| net.peers.contains(p));
}

/// Add a seat if this peer has none. A new seat claims nothing; the corner is
/// the seat's owner's to take, with [`NetMsg::Claim`].
fn seat_for(net: &mut NetState, peer: &str, name: &str) {
    if net.seats.iter().any(|s| s.peer == peer) {
        return;
    }
    net.seats.push(Seat {
        peer: peer.to_string(),
        name: name.to_string(),
        player: None,
        engine: false,
    });
}

/// Seat an engine at a specific corner: a roster entry no peer commands,
/// ready from the moment it sits, which `Start` reads like any other claim.
/// Host-only — the engine is driven by the host's own process.
fn seat_engine_at(net: &mut NetState, corner: usize) {
    let n = net.seats.iter().filter(|s| s.engine).count();
    net.seats.push(Seat {
        peer: format!("engine-{n}"),
        name: "Engine".into(),
        player: Some(corner as u32),
        engine: true,
    });
}

/// Remove the engine from a corner, if one sits there.
fn remove_engine_at(net: &mut NetState, corner: usize) {
    net.seats
        .retain(|s| !(s.engine && s.player == Some(corner as u32)));
}

/// The camps this peer's own engine should drive, from the roster's engine
/// seats. Only the sequencing authority - the host - runs engines: its moves
/// reach every peer as ordinary sequenced moves, and a guest that also ran
/// them would drive the same seat twice.
pub fn engine_camps(net: &NetState) -> Vec<Player> {
    if !net.sequences() {
        return Vec::new();
    }
    net.seats
        .iter()
        .filter(|s| s.engine)
        .filter_map(|s| s.player)
        .filter_map(|i| Player::new(i as u8))
        .collect()
}

/// The `Start` the host broadcasts: the final roster and the corners that are
/// seated. `players` is derived from the seats rather than assumed, so the
/// wire always carries the table that was actually claimed.
pub fn start_message(net: &NetState, variants: Variants) -> NetMsg {
    let mut players: Vec<u32> = net.seats.iter().filter_map(|s| s.player).collect();
    players.sort_unstable();
    players.dedup();
    NetMsg::Start {
        seats: net.seats.clone(),
        players,
        forbid_foreign_camps: variants.forbid_foreign_camps,
    }
}

/// Which corner a keypress selects. `1`..`6` name the camps directly.
fn corner_from_keys(keys: &ButtonInput<KeyCode>, current: Option<usize>) -> Option<usize> {
    if let Some(digit) = (1..=6).find(|d| keys.just_pressed(key_for(*d))) {
        return Some(digit as usize - 1);
    }
    current
}

/// The key that selects corner `digit` (1-based).
fn key_for(digit: u32) -> KeyCode {
    match digit {
        1 => KeyCode::Digit1,
        2 => KeyCode::Digit2,
        3 => KeyCode::Digit3,
        4 => KeyCode::Digit4,
        5 => KeyCode::Digit5,
        _ => KeyCode::Digit6,
    }
}

/// What clicking corner `sector` does, given the room.
///
/// Solo setups select the corner for the sidebar buttons. Shared rooms claim
/// an empty corner on the spot — clicking the triangle *is* the claim — and a
/// claimed corner is selected for the sidebar, whoever holds it: your own to
/// cancel, another player's or an engine's for the host to cancel for them.
#[derive(Debug, PartialEq, Eq)]
pub enum SectorClick {
    Select(usize),
    Claim(u32),
}

pub fn sector_click(net: &NetState, sector: usize) -> SectorClick {
    if net.peers.is_empty() {
        return SectorClick::Select(sector);
    }
    match net.seats.iter().find(|s| s.player == Some(sector as u32)) {
        None => SectorClick::Claim(sector as u32),
        Some(_) => SectorClick::Select(sector),
    }
}

/// Apply a claim: the host edits its own roster and republishes; a guest asks
/// the host and waits for the roster. Returns the status line.
fn send_claim(
    socket: &mut MatchboxSocket,
    net: &mut NetState,
    claim: Option<u32>,
    me: &str,
) -> String {
    if net.sequences() {
        if let Some(seat) = net.seats.iter_mut().find(|s| s.peer == me) {
            seat.player = claim;
        }
        publish_roster(socket, net, &net.peers);
        return match claim {
            Some(c) => format!("You hold corner {c}."),
            None => "Corner released.".into(),
        };
    }
    broadcast(socket, &net.peers, &NetMsg::Claim(claim));
    match claim {
        Some(c) => format!("Claiming corner {c}..."),
        None => "Releasing my corner...".into(),
    }
}

/// Select the corner a wedge click or digit key names.
///
/// A system of its own rather than a branch of [`handle_buttons`] so a headless
/// test can drive corner selection without a socket. The star has one button;
/// the wedge under the cursor is resolved by angle in `sector_at`.
#[allow(clippy::type_complexity)]
pub fn select_corner(
    mut hit: Query<(&Interaction, &RelativeCursorPosition), (With<StarHit>, Changed<Interaction>)>,
    keys: Res<ButtonInput<KeyCode>>,
    mut socket: Option<ResMut<MatchboxSocket>>,
    mut net: ResMut<NetState>,
    mut selected: ResMut<SelectedCorner>,
    mut status: ResMut<LobbyStatus>,
) {
    for (interaction, rel) in hit.iter_mut() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let Some(sector) = rel.normalized.and_then(sector_at) else {
            continue;
        };
        let me = net.my_seat().map(|s| s.peer.clone()).unwrap_or_default();
        match sector_click(&net, sector) {
            SectorClick::Select(i) => selected.0 = Some(i),
            SectorClick::Claim(c) => {
                if let Some(s) = socket.as_mut() {
                    status.0 = send_claim(s, &mut net, Some(c), &me);
                }
                // The claimed corner is the selected one: the very next
                // Computer / Cancel Seat press acts on it without a second
                // wedge click. Without this the first press after a claim was
                // silently swallowed — nothing was selected to act on.
                selected.0 = Some(c as usize);
            }
        }
    }
    if let Some(next) = corner_from_keys(&keys, selected.0) {
        selected.0 = Some(next);
    }
}

/// Track which wedge the cursor rests on, for the hover shading.
fn hover_corner(
    hit: Query<&RelativeCursorPosition, With<StarHit>>,
    mut hovered: ResMut<HoveredCorner>,
) {
    let over = hit
        .iter()
        .next()
        .and_then(|rel| rel.normalized.and_then(sector_at));
    if hovered.0 != over {
        hovered.0 = over;
    }
}

/// The concrete effect a [`CornerCommand`] presses on the selected corner.
/// Deciding it is a pure function so the rules are testable without a socket;
/// [`handle_buttons`] performs the result against the network.
#[derive(Debug, PartialEq, Eq)]
pub enum CornerEffect {
    /// A solo setup: the corner's new local state.
    Local(CornerState),
    /// Claim or release a corner in a shared room (guest -> host, or host
    /// directly).
    Claim(Option<u32>),
    /// The host seats an engine at this corner.
    AddEngine(u32),
    /// The host removes the engine from this corner.
    RemoveEngine(u32),
    /// The host un-seats the player holding this corner.
    Unseat(u32),
}

pub fn corner_effect(
    net: &NetState,
    me: &str,
    corner: u32,
    cmd: CornerCommand,
) -> Result<CornerEffect, String> {
    if !net.peers.is_empty() {
        // Shared room: the roster owns the corners.
        let owner = net.seats.iter().find(|s| s.player == Some(corner));
        if let Some(owner) = owner {
            if owner.engine {
                if net.sequences() && cmd == CornerCommand::Off {
                    return Ok(CornerEffect::RemoveEngine(corner));
                }
                return Err(format!("An engine plays corner {corner}."));
            }
            if owner.peer == me {
                return match cmd {
                    CornerCommand::Off => Ok(CornerEffect::Claim(None)),
                    CornerCommand::Human => Err("You already hold this corner.".into()),
                    CornerCommand::Cpu => {
                        Err("Seat an engine on a free corner: cancel this seat first.".into())
                    }
                };
            }
            // The host may un-seat anyone: a stray claim, a peer that wandered
            // off, a table being rearranged before the start.
            if net.sequences() && cmd == CornerCommand::Off {
                return Ok(CornerEffect::Unseat(corner));
            }
            return Err(format!(
                "Corner {corner} is held by {}. Only the host can un-seat a player.",
                owner.name
            ));
        }
        return match cmd {
            // The human claiming this corner is me — any peer may.
            CornerCommand::Human => Ok(CornerEffect::Claim(Some(corner))),
            CornerCommand::Cpu if net.sequences() => Ok(CornerEffect::AddEngine(corner)),
            CornerCommand::Cpu => Err("Only the host can seat an engine.".into()),
            CornerCommand::Off => Err(format!("Corner {corner} is already free.")),
        };
    }

    // Solo setup: rewrite the local table.
    let state = match cmd {
        CornerCommand::Human => CornerState::Human,
        CornerCommand::Cpu => CornerState::Cpu,
        CornerCommand::Off => CornerState::Empty,
    };
    Ok(CornerEffect::Local(state))
}

/// Fill the whole table with a symmetric preset, replacing whatever was there,
/// so the shortcut and the deal can never silently disagree.
pub fn apply_preset(table: &mut Table, seating: Seating) {
    for (i, corner) in table.0.iter_mut().enumerate() {
        let seated = seating
            .players()
            .contains(&Player::new(i as u8).expect("corner indices are below six"));
        *corner = if seated {
            CornerState::Human
        } else {
            CornerState::Empty
        };
    }
}

/// A Bevy system: the parameter count is the world access it needs, so the
/// lint threshold is waived for it.
#[allow(clippy::too_many_arguments)]
pub fn handle_buttons(
    buttons: Query<(&Interaction, &LobbyButton), Changed<Interaction>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut socket: Option<ResMut<MatchboxSocket>>,
    mut net: ResMut<NetState>,
    mut table: ResMut<Table>,
    mut selected: ResMut<SelectedCorner>,
    mut variants: ResMut<ChosenVariants>,
    mut status: ResMut<LobbyStatus>,
    mut next_state: ResMut<NextState<AppState>>,
) {
    let mut start = keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::NumpadEnter);
    let mut foreign = keys.just_pressed(KeyCode::KeyF);
    let deselect = keys.just_pressed(KeyCode::Escape);
    let mut preset: Option<Seating> = None;
    let mut command: Option<CornerCommand> = None;

    for (interaction, button) in buttons.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match button {
            LobbyButton::Start => start = true,
            LobbyButton::ForeignCamps => foreign = true,
            LobbyButton::Preset(s) => preset = Some(*s),
            LobbyButton::CornerAction(cmd) => command = Some(*cmd),
        }
    }

    if deselect {
        selected.0 = None;
    }

    let solo = net.peers.is_empty();
    let me = net.my_seat().map(|s| s.peer.clone()).unwrap_or_default();

    // Presets are solo-only furniture ([`SoloOnly`] hides the row once the
    // room is shared, and no key names them).
    if let Some(seating) = preset
        && solo
    {
        apply_preset(&mut table, seating);
        status.0 = format!("Table filled: {}.", seating.label());
    }

    if let (Some(corner), Some(cmd)) = (selected.0, command) {
        let corner = corner as u32;
        match corner_effect(&net, &me, corner, cmd) {
            Ok(CornerEffect::Local(state)) => {
                table.0[corner as usize] = state;
                status.0 = match cmd {
                    CornerCommand::Human => format!("Corner {corner}: human."),
                    CornerCommand::Cpu => format!("Corner {corner}: computer."),
                    CornerCommand::Off => format!("Corner {corner}: empty."),
                };
            }
            Ok(CornerEffect::Claim(claim)) => {
                if let Some(s) = socket.as_mut() {
                    status.0 = send_claim(s, &mut net, claim, &me);
                }
            }
            Ok(CornerEffect::AddEngine(c)) => {
                seat_engine_at(&mut net, c as usize);
                if let Some(s) = socket.as_mut() {
                    publish_roster(s, &net, &net.peers);
                }
                status.0 = format!("Engine seated at corner {c}.");
            }
            Ok(CornerEffect::RemoveEngine(c)) => {
                remove_engine_at(&mut net, c as usize);
                if let Some(s) = socket.as_mut() {
                    publish_roster(s, &net, &net.peers);
                }
                status.0 = format!("Corner {c}: engine removed.");
            }
            Ok(CornerEffect::Unseat(c)) => {
                if let Some(seat) = net
                    .seats
                    .iter_mut()
                    .find(|s| s.player == Some(c) && !s.engine)
                {
                    let name = seat.name.clone();
                    seat.player = None;
                    if let Some(s) = socket.as_mut() {
                        publish_roster(s, &net, &net.peers);
                    }
                    status.0 = format!("{} was un-seated from corner {c}.", name);
                }
            }
            Err(why) => status.0 = why,
        }
    }

    if foreign {
        // Only the host decides the rules, for the same reason only the host
        // decides the table: the shared game must play under one rule set.
        if net.sequences() {
            variants.0.forbid_foreign_camps = !variants.0.forbid_foreign_camps;
            // Broadcast live, so every lobby shows one game before the Start
            // repeats it; with no peers this is a no-op.
            if let Some(s) = socket.as_mut() {
                broadcast(
                    s,
                    &net.peers,
                    &NetMsg::Variants {
                        forbid_foreign_camps: variants.0.forbid_foreign_camps,
                    },
                );
            }
            status.0 = if variants.0.forbid_foreign_camps {
                "Rule on: no piece rests in a foreign triangle.".into()
            } else {
                "Rule off: the specification's game.".into()
            };
        } else {
            status.0 = "Only the host chooses the rules.".into();
        }
    }

    if start {
        match start_decision(&net, &table) {
            StartDecision::Solo => next_state.set(AppState::InGame),
            StartDecision::Multiplayer => {
                if let Some(s) = socket.as_mut() {
                    broadcast(s, &net.peers, &start_message(&net, variants.0));
                }
                next_state.set(AppState::InGame);
            }
            StartDecision::Refuse(why) => status.0 = why,
        }
    }
}

/// Draw the roster and the running status line.
fn draw_roster(
    net: Res<NetState>,
    table: Res<Table>,
    status: Res<LobbyStatus>,
    mut text: Query<&mut Text, With<RosterText>>,
) {
    if !net.is_changed() && !table.is_changed() && !status.is_changed() {
        return;
    }
    let Ok(mut text) = text.single_mut() else {
        return;
    };

    let me = net.my_seat().map(|s| s.peer.clone()).unwrap_or_default();
    let mut out;
    if net.peers.is_empty() {
        let filled = table.0.iter().filter(|c| **c != CornerState::Empty).count();
        let cpu = table.0.iter().filter(|c| **c == CornerState::Cpu).count();
        out = format!("local setup  |  {filled} corner(s) filled, {cpu} by computer\n\n");
        if filled == 0 {
            out.push_str(
                "The star is empty. Click a petal (or 1-6), then choose Human / Computer.\n",
            );
        } else if cpu == filled {
            out.push_str("Every corner is an engine - Enter starts as a spectator.\n");
        }
    } else {
        out = format!(
            "{}  |  {} peer(s) here\n\n",
            if net.sequences() { "host" } else { "guest" },
            net.peers.len()
        );
        if net.seats.is_empty() {
            out.push_str("No one here yet - share the room name.\n");
        }
        for seat in &net.seats {
            let corner = seat
                .player
                .map_or_else(|| "no corner".into(), |p| format!("corner {p}"));
            let engine = if seat.engine { " (computer)" } else { "" };
            out.push_str(&format!(
                "  {} {}  {}{}\n",
                if seat.peer == me { ">" } else { " " },
                seat.name,
                corner,
                engine,
            ));
        }
    }

    out.push_str(if net.peers.is_empty() {
        "\nEnter starts this table locally.\nDigits 1-6 select a corner."
    } else {
        "\nClaim a corner on the star; the host starts the game with Enter."
    });
    if !status.0.is_empty() {
        out.push_str(&format!("\n\n{}", status.0));
    }
    **text = out;
}

/// Keep the star's petals in step with the table/roster: colour, selection
/// ring, state line.
fn sync_corner_styles(
    net: Res<NetState>,
    table: Res<Table>,
    selected: Res<SelectedCorner>,
    hovered: Res<HoveredCorner>,
    mut petals: Query<(&CornerPetal, &mut ImageNode)>,
) {
    for (petal, mut img) in petals.iter_mut() {
        let p = Player::new(petal.0 as u8).expect("corner indices are below six");
        let base = if corner_is_filled(&net, &table, petal.0) {
            player_colour(p)
        } else {
            IDLE
        };
        // Selection lights the wedge strongly, the cursor resting on it mildly.
        let factor = if selected.0 == Some(petal.0) {
            1.25
        } else if hovered.0 == Some(petal.0) {
            1.15
        } else {
            1.0
        };
        let colour = shade(base, factor);
        if img.color != colour {
            img.color = colour;
        }
    }
}

/// Draw the two lines of every star petal.
///
/// Each line is a stack of copies (white fill over black border copies, see
/// [`outlined_line`]); the marker sits on the stack and the string goes into
/// every copy, so fill and border can never disagree.
fn draw_corner_labels(
    net: Res<NetState>,
    table: Res<Table>,
    lines: Query<(&CornerText, &Children)>,
    boxes: Query<&Children>,
    mut texts: Query<&mut Text>,
) {
    if !net.is_changed() && !table.is_changed() {
        return;
    }
    let solo = net.peers.is_empty();
    for (label, copies) in &lines {
        let i = label.0 / 2;
        let line = label.0 % 2;
        let p = Player::new(i as u8).expect("corner indices are below six");
        let (title, sub) = if solo {
            match &table.0[i] {
                CornerState::Empty => ("Empty".into(), "click to select".into()),
                CornerState::Human => (format!("P{}", p.index()), "human".into()),
                CornerState::Cpu => ("Computer".into(), "CPU".into()),
            }
        } else if let Some(seat) = net.seats.iter().find(|s| s.player == Some(i as u32)) {
            if seat.engine {
                ("Engine".into(), "CPU".into())
            } else {
                (seat.name.clone(), "human".into())
            }
        } else {
            ("Empty".into(), "click to claim".into())
        };
        let wanted = if line == 0 { title } else { sub };
        for copy in copies.iter() {
            // Each copy is a flex box; its one child is the text leaf.
            if let Ok(leaf) = boxes.get(copy)
                && let Some(text_entity) = leaf.iter().next()
                && let Ok(mut text) = texts.get_mut(text_entity)
                && **text != wanted
            {
                **text = wanted.clone();
            }
        }
    }
}

/// Brighten or darken a colour for hover and press. Done on the sRGB byte
/// scale, which keeps the luminance of a piece colour looking like itself.
fn shade(c: Color, factor: f32) -> Color {
    let rgba = c.to_srgba();
    let (r, g, b, a) = (rgba.red, rgba.green, rgba.blue, rgba.alpha);
    Color::srgba(
        (r * factor).min(1.0),
        (g * factor).min(1.0),
        (b * factor).min(1.0),
        a,
    )
}

/// The camps that sit down in the current room: which corners are filled,
/// which of them this device's engine drives, which corner this peer
/// commands, and whether this peer only watches. Pure, so the deal is
/// testable without a world.
///
/// Solo setups read the local [`Table`] — every configured corner, whether
/// played by hand (`CornerState::Human`) or by the engine. Shared rooms read
/// the roster's claims instead, so every peer deals the same board no matter
/// what corners each device configured.
pub fn deal_for(net: &NetState, table: &Table) -> (Vec<Player>, Vec<Player>, Option<Player>, bool) {
    if net.peers.is_empty() {
        let players: Vec<Player> = (0..6)
            .filter(|&i| table.0[i] != CornerState::Empty)
            .filter_map(|i| Player::new(i as u8))
            .collect();
        let ai: Vec<Player> = (0..6)
            .filter(|&i| table.0[i] == CornerState::Cpu)
            .filter_map(|i| Player::new(i as u8))
            .collect();
        let spectating = !ai.is_empty() && ai.len() >= players.len();
        (players, ai, None, spectating)
    } else {
        let players: Vec<Player> = net
            .seats
            .iter()
            .filter_map(|s| s.player)
            .filter_map(|i| Player::new(i as u8))
            .collect();
        let ai = engine_camps(net);
        let local = net.my_player();
        let spectating = net.my_seat().is_none_or(|s| s.player.is_none());
        (players, ai, local, spectating)
    }
}

/// Build the game for the configured table and seat the local player.
///
/// The session is *rebuilt* here rather than mutated, because the players
/// determine the starting position and there is no meaningful way to reseat a
/// board that has already been dealt. Runs on entering the game, before the
/// board is spawned.
pub fn apply_seats(
    net: Res<NetState>,
    table: Res<Table>,
    variants: Res<ChosenVariants>,
    mut session: ResMut<Session>,
) {
    let (players, ai, local_player, spectating) = deal_for(&net, &table);
    let wording = if spectating {
        if net.peers.is_empty() {
            "Spectating - the engines play each other.".into()
        } else {
            "Spectating - watch, but do not touch.".into()
        }
    } else if let Some(p) = local_player {
        format!("You are player {} of {}", p.index(), players.len())
    } else {
        format!("Playing all {} corners on this device", players.len())
    };

    *session = Session::for_players(&players, variants.0);
    session.ai_players = ai;
    session.local_player = local_player;
    session.spectating = spectating;
    session.message = wording;
}

#[cfg(test)]
mod tests {
    use super::*;
    use checkers_core::position::Player;
    use checkers_net::Seat;

    fn seat(name: &str, player: Option<u32>) -> Seat {
        Seat {
            peer: name.into(),
            name: name.into(),
            player,
            engine: false,
        }
    }

    /// A lone device with no peers deals whatever corners it configured.
    #[test]
    fn a_solo_table_starts_with_the_configured_corners() {
        let net = NetState::default();
        let mut table = Table::default();
        table.0[0] = CornerState::Human;
        table.0[3] = CornerState::Cpu;
        assert_eq!(start_decision(&net, &table), StartDecision::Solo);
    }

    /// One corner cannot be a game: the turn would visit a player's camp with
    /// nobody else to play.
    #[test]
    fn one_corner_cannot_start() {
        let net = NetState::default();
        let mut table = Table::default();
        table.0[0] = CornerState::Human;
        match start_decision(&net, &table) {
            StartDecision::Refuse(why) => assert!(why.contains("two corners"), "{why}"),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    /// A guest cannot start a shared game, and must be told so rather than
    /// having Enter do nothing.
    #[test]
    fn a_guest_is_told_only_the_host_can_start() {
        let mut net = NetState::default();
        net.peers.push(fake_peer());
        net.is_host = false;
        match start_decision(&net, &Table::default()) {
            StartDecision::Refuse(why) => assert!(why.contains("host"), "{why}"),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    /// Fewer than two claimed corners cannot start.
    #[test]
    fn a_shared_start_needs_two_claimed_corners() {
        let mut net = NetState::default();
        net.peers.push(fake_peer());
        net.is_host = true;
        net.seats = vec![seat("ada", Some(0))];
        assert!(
            matches!(
                start_decision(&net, &Table::default()),
                StartDecision::Refuse(_)
            ),
            "one claim cannot be a game"
        );
    }

    #[test]
    fn the_host_starts_two_claimed_corners() {
        let mut net = NetState::default();
        net.peers.push(fake_peer());
        net.is_host = true;
        net.seats = vec![seat("ada", Some(0)), seat("grace", Some(3))];
        assert_eq!(
            start_decision(&net, &Table::default()),
            StartDecision::Multiplayer
        );
    }

    /// A claimed corner plus an engine seat reaches the two-corner minimum.
    #[test]
    fn engines_count_toward_the_two_corners() {
        let mut net = NetState::default();
        net.peers.push(fake_peer());
        net.is_host = true;
        net.seats = vec![seat("ada", Some(0))];
        let mut engine = seat("engine-0", Some(3));
        engine.engine = true;
        net.seats.push(engine);
        assert_eq!(
            start_decision(&net, &Table::default()),
            StartDecision::Multiplayer
        );
    }

    /// The `Start` broadcast derives the players from the seats, so the wire
    /// always carries the table the host actually claims.
    #[test]
    fn the_start_message_carries_the_claimed_corners() {
        let net = NetState {
            seats: vec![seat("ada", Some(3)), seat("grace", Some(0))],
            ..Default::default()
        };
        let NetMsg::Start { players, .. } = start_message(&net, Variants::default()) else {
            panic!("start_message must build a Start");
        };
        assert_eq!(players, vec![0, 3], "corners are sorted, not join order");
    }

    /// The host seats an engine on a free corner; it claims that corner and
    /// reads as ready so it never blocks a start.
    #[test]
    fn an_engine_seat_claims_its_corner() {
        let mut net = NetState {
            is_host: true,
            // A friend is here: engine seating is a shared-room action, never
            // a solo rewrite of the local table.
            peers: vec![fake_peer()],
            ..Default::default()
        };
        let effect = corner_effect(&net, "", 2, CornerCommand::Cpu);
        assert_eq!(effect, Ok(CornerEffect::AddEngine(2)));
        seat_engine_at(&mut net, 2);
        assert!(net.seats[0].engine);
        assert_eq!(net.seats[0].player, Some(2));

        // The host removes it again.
        assert_eq!(
            corner_effect(&net, "", 2, CornerCommand::Off),
            Ok(CornerEffect::RemoveEngine(2))
        );
        remove_engine_at(&mut net, 2);
        assert!(net.seats.is_empty());
    }

    /// Only the sequencing authority runs engines.
    #[test]
    fn engines_drive_on_the_host_alone() {
        let mut host = NetState {
            is_host: true,
            ..Default::default()
        };
        seat_engine_at(&mut host, 0);
        let guest = NetState {
            peers: vec![fake_peer()],
            seats: host.seats.clone(),
            ..Default::default()
        };
        assert_eq!(engine_camps(&host), vec![Player::ALL[0]]);
        assert!(engine_camps(&guest).is_empty());
    }

    /// A guest claims a free corner; the host applies it. The pure decision is
    /// a claim message, and the host's own application moves the seat.
    #[test]
    fn a_claim_moves_the_seat() {
        let mut net = NetState::default();
        net.peers.push(fake_peer());
        seat_for(&mut net, "host", "ada");
        assert_eq!(net.seats[0].player, None, "no corner until claimed");

        let effect = corner_effect(&net, "host", 1, CornerCommand::Human)
            .expect("a free corner is claimable");
        assert_eq!(effect, CornerEffect::Claim(Some(1)));
        net.seats[0].player = Some(1);
        assert_eq!(net.seats[0].player, Some(1));
    }

    /// A corner already held by someone else is not claimable, and the reason
    /// names the holder.
    #[test]
    fn an_occupied_corner_is_not_claimable() {
        let mut net = NetState::default();
        net.peers.push(fake_peer());
        net.seats = vec![seat("ada", Some(2))];
        let err = corner_effect(&net, "grace", 2, CornerCommand::Human).expect_err("taken");
        assert!(err.contains("ada"), "must name the holder: {err}");
    }

    /// Pressing "Off" on my own corner releases it; anything else is refused
    /// with the reason attached.
    #[test]
    fn releasing_my_own_corner() {
        let mut net = NetState::default();
        net.peers.push(fake_peer());
        net.seats = vec![seat("ada", Some(2))];
        assert_eq!(
            corner_effect(&net, "ada", 2, CornerCommand::Off),
            Ok(CornerEffect::Claim(None))
        );
        assert!(corner_effect(&net, "ada", 2, CornerCommand::Human).is_err());
    }

    /// A guest cannot place an engine; that is the host's call.
    #[test]
    fn only_the_host_seats_engines() {
        let net = NetState {
            peers: vec![fake_peer()],
            ..Default::default()
        };
        assert!(corner_effect(&net, "grace", 1, CornerCommand::Cpu).is_err());
        let host = NetState {
            is_host: true,
            peers: vec![fake_peer()],
            ..Default::default()
        };
        assert_eq!(
            corner_effect(&host, "host", 1, CornerCommand::Cpu),
            Ok(CornerEffect::AddEngine(1))
        );
    }

    /// Solo commands map straight onto corner states.
    #[test]
    fn solo_commands_rewrite_the_table() {
        let net = NetState::default();
        assert_eq!(
            corner_effect(&net, "", 0, CornerCommand::Cpu),
            Ok(CornerEffect::Local(CornerState::Cpu))
        );
        assert_eq!(
            corner_effect(&net, "", 0, CornerCommand::Human),
            Ok(CornerEffect::Local(CornerState::Human))
        );
        assert_eq!(
            corner_effect(&net, "", 0, CornerCommand::Off),
            Ok(CornerEffect::Local(CornerState::Empty))
        );
    }

    /// The preset fills exactly the seating's corners, presenting the
    /// symmetric game the shortcut claims.
    #[test]
    fn presets_fill_the_seatings_corners() {
        let mut table = Table::default();
        apply_preset(&mut table, Seating::Two);
        assert_eq!(table.0[0], CornerState::Human);
        assert_eq!(table.0[3], CornerState::Human);
        assert_eq!(table.0[1], CornerState::Empty);
        assert_eq!(table.0[2], CornerState::Empty);
    }

    /// Digit keys select their corner; unrelated keys change nothing.
    #[test]
    fn the_digits_select_their_corner() {
        for digit in 1..=6 {
            let mut keys = ButtonInput::default();
            keys.press(key_for(digit));
            assert_eq!(
                corner_from_keys(&keys, None),
                Some(digit as usize - 1),
                "{digit}"
            );
        }
        let mut keys = ButtonInput::default();
        keys.press(KeyCode::KeyX);
        assert_eq!(corner_from_keys(&keys, None), None);
    }

    /// The deal must run on the configured corners: `apply_seats` rebuilds the
    /// session from them, and a session built for two corners must not be
    /// holding a board nobody set up. Solo setups read the local table; the
    /// engine camps match; all-engine tables are spectated.
    #[test]
    fn the_configured_corners_build_the_session() {
        let net = NetState::default();
        let mut table = Table::default();
        table.0[0] = CornerState::Human;
        table.0[2] = CornerState::Cpu;
        table.0[4] = CornerState::Human;
        let (players, ai, local, spectating) = deal_for(&net, &table);
        assert_eq!(
            players,
            vec![Player::ALL[0], Player::ALL[2], Player::ALL[4]]
        );
        assert_eq!(ai, vec![Player::ALL[2]]);
        assert_eq!(local, None);
        assert!(!spectating);

        // All engines: the table starts as a watched race.
        let mut table = Table::default();
        table.0[0] = CornerState::Cpu;
        table.0[3] = CornerState::Cpu;
        let (players, ai, _local, spectating) = deal_for(&net, &table);
        assert_eq!(players, vec![Player::ALL[0], Player::ALL[3]]);
        assert_eq!(ai, vec![Player::ALL[0], Player::ALL[3]]);
        assert!(spectating, "all-engine tables are spectated");
    }

    /// A shared start deals the roster's claims, not the local table.
    #[test]
    fn a_shared_start_deals_the_claims() {
        let mut net = NetState::default();
        let me = PeerId(uuid::Uuid::from_u128(1));
        net.my_id = Some(me);
        net.peers = vec![me, fake_peer()];
        net.is_host = true;
        let host = me.to_string();
        net.seats = vec![seat(&host, Some(0)), seat("grace", Some(3))];
        let mut table = Table::default();
        table.0[5] = CornerState::Cpu;

        let (players, ai, local, spectating) = deal_for(&net, &table);
        assert_eq!(players, vec![Player::ALL[0], Player::ALL[3]]);
        assert!(ai.is_empty(), "the local table is ignored in a shared room");
        assert_eq!(local, Some(Player::ALL[0]));
        assert!(!spectating);
    }

    fn fake_peer() -> PeerId {
        PeerId(uuid::Uuid::from_u128(2))
    }

    /// Claiming a corner on the star must also select it: otherwise the first
    /// sidebar press after a claim was silently swallowed, acting on nothing.
    #[test]
    fn claiming_a_corner_selects_it() {
        use bevy::ecs::system::RunSystemOnce;

        let mut world = World::new();
        let mut net = NetState::default();
        net.peers.push(fake_peer());
        net.is_host = true;
        let me = PeerId(uuid::Uuid::from_u128(1));
        net.my_id = Some(me);
        net.seats = vec![seat(&me.to_string(), None)];
        world.insert_resource(net);
        world.insert_resource(SelectedCorner(None));
        world.init_resource::<LobbyStatus>();
        world.init_resource::<ButtonInput<KeyCode>>();

        // A press on corner 1's centroid — free, so a claim.
        let centre = Vec2::new(STAR_W / 2.0, STAR_H / 2.0);
        let size = vec2(STAR_W, STAR_H);
        let mid = {
            let v = wedge_vertices(1);
            (v[0] + v[1] + v[2]) / 3.0
        };
        world.spawn((
            Button,
            Interaction::Pressed,
            StarHit,
            RelativeCursorPosition {
                cursor_over: true,
                normalized: Some((mid - centre) / size),
            },
        ));

        world.run_system_once(select_corner).unwrap();

        assert_eq!(
            world.resource::<SelectedCorner>().0,
            Some(1),
            "the claimed corner must be the selected one"
        );
    }

    /// The wedges leave the middle of the star clear — that is the point of
    /// them — and each points at its own camp's direction.
    #[test]
    fn the_centre_is_not_a_corner_and_each_wedge_is_its_own() {
        let centre = Vec2::new(STAR_W / 2.0, STAR_H / 2.0);
        let size = vec2(STAR_W, STAR_H);
        let normalized_of = |p: Vec2| (p - centre) / size;
        assert_eq!(
            sector_at(Vec2::ZERO),
            None,
            "the middle of the star selects nothing"
        );
        for i in 0..6 {
            let v = wedge_vertices(i);
            let mid = (v[0] + v[1] + v[2]) / 3.0;
            // The centroid, as a normalized container position, hits corner i.
            assert_eq!(
                sector_at(normalized_of(mid)),
                Some(i),
                "wedge {i}'s own centroid must resolve to corner {i}"
            );
        }
        // A point between two wedges — straight out from the centre between
        // corners 0 and 1 — belongs to neither.
        let angle = (-60.0f32).to_radians();
        let between = centre + 120.0 * Vec2::new(angle.cos(), angle.sin());
        assert_eq!(sector_at(normalized_of(between)), None);
    }

    /// The wedges are equilateral, like the board's camp triangles: all three
    /// sides equal, and the tip the outermost point of its wedge.
    #[test]
    fn the_wedges_are_equilateral() {
        let centre = vec2(STAR_W / 2.0, STAR_H / 2.0);
        for i in 0..6 {
            let [apex, left, right] = wedge_vertices(i);
            let side = left.distance(right);
            assert!(
                (apex.distance(left) - side).abs() < 1e-3
                    && (apex.distance(right) - side).abs() < 1e-3,
                "wedge {i} is not equilateral"
            );
            assert!(
                apex.distance(centre) > left.distance(centre),
                "wedge {i}'s tip must face outward"
            );
        }
    }

    /// The rasterized wedge is opaque inside the triangle and transparent at
    /// the centre, so the tint paints a sector and the star stays visible.
    #[test]
    fn the_wedge_texture_paints_the_triangle_only() {
        let image = sector_image(0);
        let centre = Vec2::new(STAR_W / 2.0, STAR_H / 2.0);
        let v = wedge_vertices(0);
        let mid = (v[0] + v[1] + v[2]) / 3.0;
        let Some(data) = &image.data else {
            panic!("the wedge texture keeps its pixel data");
        };
        let alpha_at = |p: Vec2| -> u8 {
            let (x, y) = (
                (p.x as u32).min(STAR_W as u32 - 1),
                (p.y as u32).min(STAR_H as u32 - 1),
            );
            data[((y * STAR_W as u32 + x) * 4 + 3) as usize]
        };
        assert_eq!(alpha_at(mid), 255);
        assert_eq!(alpha_at(centre), 0);
    }

    /// Solo: a wedge click only selects; the sidebar buttons configure.
    #[test]
    fn a_solo_wedge_click_selects() {
        let net = NetState::default();
        assert_eq!(sector_click(&net, 3), SectorClick::Select(3));
    }

    /// Shared: an empty wedge is claimed on the spot — the click is the claim.
    #[test]
    fn a_shared_click_on_an_empty_corner_claims_it() {
        let mut net = NetState::default();
        let me = PeerId(uuid::Uuid::from_u128(1));
        net.my_id = Some(me);
        net.peers = vec![me, fake_peer()];
        net.seats = vec![seat(&me.to_string(), None)];
        assert_eq!(sector_click(&net, 5), SectorClick::Claim(5));
    }

    /// A peer that leaves takes its seat with it: a refresh mints a new peer
    /// id, and without pruning the roster only ever grew. Note `net.peers`
    /// never names the host itself — matchbox lists only *other* peers — so
    /// the host's own seat is exempted explicitly.
    #[test]
    fn departed_peers_lose_their_seats() {
        let mut net = NetState::default();
        let me = PeerId(uuid::Uuid::from_u128(1));
        let gone = PeerId(uuid::Uuid::from_u128(2));
        let here = PeerId(uuid::Uuid::from_u128(3));
        net.my_id = Some(me);
        net.is_host = true;
        // Others only: the mesh never reports the host's own id as a peer.
        net.peers = vec![here];
        net.greeted = vec![gone, here];
        net.seats = vec![
            seat(&me.to_string(), Some(0)),
            seat(&gone.to_string(), Some(3)),
            Seat {
                engine: true,
                ..seat("engine-0", Some(4))
            },
            seat(&here.to_string(), None),
        ];

        prune_departed(&mut net);

        assert!(
            net.seats.iter().any(|s| s.peer == me.to_string()),
            "the host's own seat must survive the prune"
        );
        assert!(
            !net.seats.iter().any(|s| s.peer == gone.to_string()),
            "the departed peer's seat must go"
        );
        assert!(
            net.seats.iter().any(|s| s.peer == here.to_string()),
            "connected peers keep their seats"
        );
        assert!(
            net.seats.iter().any(|s| s.engine),
            "engines have no peer behind them and survive"
        );
        assert!(
            !net.greeted.contains(&gone),
            "greeting memory of the departed is dropped"
        );
        assert!(net.greeted.contains(&here));
    }

    /// Shared: clicking a claimed corner selects it — yours, another
    /// player's, or an engine's — so the sidebar's Cancel Seat can act on it;
    /// a free corner is claimed on the spot.
    #[test]
    fn a_shared_click_selects_any_claimed_corner() {
        let mut net = NetState::default();
        let me = PeerId(uuid::Uuid::from_u128(1));
        let other = fake_peer();
        net.my_id = Some(me);
        net.peers = vec![me, other];
        net.seats = vec![
            seat(&me.to_string(), Some(2)),
            seat("grace", Some(1)),
            Seat {
                engine: true,
                ..seat("bot", Some(4))
            },
        ];
        assert_eq!(sector_click(&net, 2), SectorClick::Select(2));
        assert_eq!(sector_click(&net, 1), SectorClick::Select(1));
        assert_eq!(sector_click(&net, 4), SectorClick::Select(4));
        assert_eq!(sector_click(&net, 3), SectorClick::Claim(3));
    }

    /// The host can un-seat another player's corner; a guest cannot touch a
    /// corner held by someone else.
    #[test]
    fn the_host_unseats_a_player() {
        let mut net = NetState::default();
        net.peers.push(fake_peer());
        net.is_host = true;
        let host = "host";
        net.seats = vec![seat(host, Some(0)), seat("grace", Some(3))];

        assert_eq!(
            corner_effect(&net, host, 3, CornerCommand::Off),
            Ok(CornerEffect::Unseat(3)),
            "the host un-seats anyone"
        );
        assert_eq!(
            corner_effect(&net, host, 0, CornerCommand::Off),
            Ok(CornerEffect::Claim(None)),
            "releasing one's own corner is a release, not an un-seat"
        );

        net.is_host = false;
        let effect = corner_effect(&net, "grace", 0, CornerCommand::Off);
        assert!(effect.is_err(), "a guest cannot un-seat another player");
    }
}
