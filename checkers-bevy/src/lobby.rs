//! Lobby screen: the hex star and the corners on it.
//!
//! This is the *only* setup screen, for every way a game can be played. The
//! star is drawn with six petals, one per camp. Each petal is configured —
//! human, computer, or empty — and a game is whatever corners got filled:
//!
//! * with no peers in the room, the configuration is local, and a filled
//!   corner is either played by hand or by the engine on this device;
//! * with peers, every guest claims its own corner (and `Name` says who it
//!   is), the host may seat an engine on a free corner, and the table starts
//!   once two or more corners are claimed and everyone is ready.
//!
//! Spectators are simply peers who claimed nothing. "Watch two bots" is now
//! just a corner configuration — two adjacent... / any two engines — set up
//! in this one screen. There is no separate menu, hotseat panel, or player
//! count: 2/3/6 presets exist as shortcuts that fill the symmetric camps.
//!
//! Built with plain Bevy UI rather than egui, to match the in-game buttons and
//! to avoid a dependency for a handful of widgets.
//!
//! Every control has a key *and* a button where a key is natural, and the keys
//! are what the hints name. Buttons exist because a lobby whose only
//! affordances are typed is indistinguishable from an empty screen.
//!
//! # Choosing the room
//!
//! `R` opens the room field; Enter joins, Esc cancels. Changing the room
//! **reopens the socket**, because the room is part of the signaling URL — see
//! [`edit_room`]. Fields are modal, and every other key is suppressed while
//! one holds the keyboard.
//!
//! # Host election
//!
//! The peer with the lexicographically smallest `PeerId` hosts, recomputed
//! every frame so host loss self-heals. That is not elegant, but it is
//! *deterministic without negotiation*: every peer computes the same answer
//! from the same peer list.
//!
//! # Seats
//!
//! The host owns the roster. Guests announce themselves with
//! [`NetMsg::Hello`] and claim a corner with [`NetMsg::Claim`]; everything else
//! is the host broadcasting [`NetMsg::Roster`]. A corner is granted to the
//! first claimant because the host is the only peer that decides, so a guest
//! never has to reconcile two sources of truth about which player it commands.
//! Peers who claimed nothing watch as spectators.

use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::input::ButtonState;
use bevy::input::keyboard::KeyboardInput;
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
    Ready,
    Start,
    /// Change the selected corner: active only on solo setups, where the table
    /// is this device's own configuration.
    Preset(Seating),
    /// Declare or switch off the "no piece may rest in a foreign camp" rule.
    ForeignCamps,
    /// Perform [`CornerCommand`] on the currently selected corner.
    CornerAction(CornerCommand),
}

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

/// Which editor an on-screen text input drives.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    Room,
    Name,
    /// The name of the selected human corner, in solo setups.
    Corner,
}

/// An always-visible text input box: click to focus (or its key), Enter
/// commits, Esc leaves. The value shown is driven by [`draw_room`],
/// [`draw_name`] and [`draw_corner`].
#[derive(Component)]
pub struct TextInput(pub FieldKind);

/// The value text inside an input box.
#[derive(Component)]
struct InputText(FieldKind);

/// The error line under an input box.
#[derive(Component)]
struct InputError(FieldKind);

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
    Human(String),
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

pub fn plugin(app: &mut App) {
    // A share link carries the room in the URL fragment; honour it over the
    // default so a link lands you in the sender's lobby. Native builds have no
    // URL, and this is a no-op there.
    if let Some(room) = crate::web::room_from_url() {
        app.insert_resource(room);
    }
    app.init_resource::<NetState>()
        .init_resource::<RoomId>()
        .init_resource::<Table>()
        .init_resource::<SelectedCorner>()
        .init_resource::<HoveredCorner>()
        .init_resource::<SectorArt>()
        .init_resource::<ChosenVariants>()
        .init_resource::<RoomEdit>()
        .init_resource::<NameEdit>()
        .init_resource::<CornerEdit>()
        .add_systems(
            OnEnter(AppState::Lobby),
            // The wedge art must exist before the star can reference it.
            (checkers_net::open_socket, ensure_sector_art, spawn).chain(),
        )
        .add_systems(OnExit(AppState::Lobby), despawn)
        .add_systems(
            Update,
            (
                elect_host,
                pump_socket,
                // First, and the rest are suppressed while a field holds the
                // keyboard: typing a name must not also start a game on the
                // Enter that commits it.
                (edit_room, edit_name, edit_corner),
                focus_input_fields.run_if(not_editing),
                (select_corner, handle_buttons).run_if(not_editing),
                // The modal's exit: an elsewhere-click closes the focused
                // field, while `not_editing` keeps this frame's click from
                // also acting on whatever was clicked.
                blur_on_elsewhere_click.run_if(in_state(AppState::Lobby)),
                broadcast_cursor.run_if(in_state(AppState::Lobby)),
                (
                    sync_button_styles,
                    sync_input_styles,
                    sync_corner_settings,
                    hover_corner,
                    // The wedge tint follows the state, and the labels follow
                    // the roster; both after the hover is settled this frame.
                    (
                        sync_corner_styles,
                        draw_corner_labels,
                        draw_roster,
                        draw_room,
                        draw_name,
                        draw_corner,
                        sync_remote_cursors,
                    )
                        .chain(),
                )
                    .chain()
                    .run_if(in_state(AppState::Lobby)),
            ),
        );
}

/// Whether a field has the keyboard.
///
/// Every other lobby key is gated on this. Without it each character typed is
/// also a command — Space readies, Enter starts, a digit selects a corner — so
/// a field would be unusable for any name containing them, which is nearly all
/// of them.
pub fn not_editing(room: Res<RoomEdit>, name: Res<NameEdit>, corner: Res<CornerEdit>) -> bool {
    !room.active
        && !room.consumed_input
        && !name.active
        && !name.consumed_input
        && !corner.active
        && !corner.consumed_input
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
                CornerState::Human(_) => CornerCommand::Human,
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
            LobbyButton::Ready => net.my_seat().is_some_and(|s| s.ready),
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

/// Paint the text inputs: a focused field gets a green border and a darker
/// well so it is obvious the keyboard is captured; an unfocused one brightens
/// its border on hover, so the box reads as clickable.
fn sync_input_styles(
    room: Res<RoomEdit>,
    name: Res<NameEdit>,
    corner: Res<CornerEdit>,
    mut inputs: Query<(
        &Interaction,
        &TextInput,
        &mut BackgroundColor,
        &mut BorderColor,
    )>,
) {
    for (interaction, input, mut bg, mut border) in inputs.iter_mut() {
        let focused = match input.0 {
            FieldKind::Room => room.active,
            FieldKind::Name => name.active,
            FieldKind::Corner => corner.active,
        };
        let border_colour = if focused {
            CHOSEN
        } else {
            match interaction {
                Interaction::Hovered => HOVER,
                _ => Color::srgb(0.35, 0.35, 0.40),
            }
        };
        let well = if focused {
            Color::srgb(0.15, 0.15, 0.19)
        } else {
            IDLE
        };
        if bg.0 != well {
            bg.0 = well;
        }
        if border.top != border_colour {
            *border = BorderColor::all(border_colour);
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

/// The room-name editor.
///
/// Editing is *modal*: while any editor holds the keyboard every other key is
/// suppressed, because the alternative is that typing a room named "solo"
/// starts a game on the `s`. A mode is the smaller evil here, and `Esc` always
/// leaves it.
#[derive(Resource, Default)]
pub struct RoomEdit {
    pub active: bool,
    /// What has been typed so far. Only committed to [`RoomId`] on Enter, so an
    /// abandoned edit cannot leave the socket pointing somewhere unintended.
    pub buffer: String,
    /// Why the last commit was refused, shown beneath the field.
    pub error: String,
    /// Set for the rest of the frame in which the field handled a keypress.
    ///
    /// Closing the field is not enough on its own. The lobby systems are
    /// `.chain()`ed, so `handle_buttons` runs *after* `edit_room` in the same
    /// frame: committing with Enter cleared `active`, and the very same Enter
    /// then fell through and started the game.
    ///
    /// A run condition cannot see "this frame's input was already used", so the
    /// editor records it. Cleared at the top of each `edit_*` run.
    pub consumed_input: bool,
}

/// The player-name editor. Same modal pattern as [`RoomEdit`] — including the
/// input-consumption flag — but committing writes the display name and
/// re-greets peers rather than reopening a socket.
#[derive(Resource, Default)]
pub struct NameEdit {
    pub active: bool,
    pub buffer: String,
    pub error: String,
    pub consumed_input: bool,
}

/// The selected corner's name editor. Same modal pattern, but it writes
/// [`Table`], and only on solo setups where the selected corner is human.
#[derive(Resource, Default)]
pub struct CornerEdit {
    pub active: bool,
    pub buffer: String,
    pub error: String,
    pub consumed_input: bool,
}

/// What a keypress does to an editor.
///
/// Returned rather than applied so the decision is testable without a window;
/// the `edit_*` systems are the thin performers.
#[derive(Debug, PartialEq, Eq)]
pub enum EditAction {
    /// Add to the buffer.
    Insert(char),
    Backspace,
    /// Commit the buffer.
    Commit,
    /// Abandon the edit.
    Cancel,
    /// Not for the editor.
    Ignore,
}

/// Classify one keypress during editing.
///
/// `text` is [`bevy::input::keyboard::KeyboardInput::text`], which respects the
/// keyboard layout — reading `key_code` instead would give a US-layout guess.
pub fn edit_action(key: KeyCode, text: Option<&str>) -> EditAction {
    match key {
        KeyCode::Enter | KeyCode::NumpadEnter => return EditAction::Commit,
        KeyCode::Escape => return EditAction::Cancel,
        KeyCode::Backspace => return EditAction::Backspace,
        _ => {}
    }
    // A single character only: `text` can hold two when a dead key did not
    // combine.
    match text.and_then(|t| {
        let mut chars = t.chars();
        chars.next().filter(|_| chars.next().is_none())
    }) {
        // Control characters arrive here as text on some platforms.
        Some(c) if !c.is_control() => EditAction::Insert(c),
        _ => EditAction::Ignore,
    }
}

/// The lobby is a **two-column flex row filling the window**: the star and
/// its caption on the left, every control on the right.
///
/// Two earlier layouts failed the same way. Anchoring the buttons at
/// `bottom: 40px` put them off-screen entirely on a display whose work area is
/// shorter than the window. A single centred column cannot place anything
/// outside the window, but its content stacks to roughly a thousand pixels —
/// star, three button rows, three labelled fields and their error lines, the
/// rules, the roster, and the start row — so on any ordinary window it
/// overflowed both ends and the edges clipped. Side by side, the column
/// heights are the star's ~430px and the controls' ~500px, and the layout fits
/// a 600px-tall window by construction.
fn spawn(mut commands: Commands, art: Res<SectorArt>) {
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
                // General settings: who I am and where I am. These travel with
                // the Hello and the share link, so they come first — everything
                // below them is about the table, not the player.
                header(col, "General");

                // The room field: a real text input, click or `R` to focus.
                field_row(col, "Room", FieldKind::Room, "R");
                col.spawn((
                    Text::new(String::new()),
                    TextFont {
                        font_size: FontSize::Px(14.0),
                        ..default()
                    },
                    TextColor(Color::srgb(0.85, 0.35, 0.35)),
                    InputError(FieldKind::Room),
                ));

                // The player-name field, same pattern.
                field_row(col, "Name", FieldKind::Name, "N");
                col.spawn((
                    Text::new(String::new()),
                    TextFont {
                        font_size: FontSize::Px(14.0),
                        ..default()
                    },
                    TextColor(Color::srgb(0.85, 0.35, 0.35)),
                    InputError(FieldKind::Name),
                ));

                header(col, "Table");
                // Presets are shortcuts for the symmetric setups; they fill the
                // whole table, so what is configured and what the shortcut
                // leaves can never silently disagree.
                col.spawn(Node {
                    column_gap: Val::Px(10.0),
                    align_items: AlignItems::Center,
                    ..default()
                })
                .with_children(|row| {
                    for seating in Seating::ALL {
                        button(
                            row,
                            &format!("Preset {}", seating.label()),
                            LobbyButton::Preset(seating),
                        );
                    }
                });

                // The claimed corner's settings. In a shared room these exist
                // only once a corner is claimed — clicking an empty wedge is
                // the claim — so [`sync_corner_settings`] folds them away
                // until then.
                col.spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        row_gap: Val::Px(10.0),
                        ..default()
                    },
                    CornerSettings,
                ))
                .with_children(|corner| {
                    // What to do with the selected corner.
                    corner
                        .spawn(Node {
                            column_gap: Val::Px(10.0),
                            align_items: AlignItems::Center,
                            ..default()
                        })
                        .with_children(|row| {
                            for (label, cmd) in [
                                ("Human", CornerCommand::Human),
                                ("Computer", CornerCommand::Cpu),
                                ("Empty", CornerCommand::Off),
                            ] {
                                button(row, label, LobbyButton::CornerAction(cmd));
                            }
                        });

                    // The selected corner's name, on solo setups. Spawned
                    // always; the focus and draw systems stand it down in
                    // shared rooms.
                    corner_name_row(corner);
                    corner.spawn((
                        Text::new(String::new()),
                        TextFont {
                            font_size: FontSize::Px(14.0),
                            ..default()
                        },
                        TextColor(Color::srgb(0.85, 0.35, 0.35)),
                        InputError(FieldKind::Corner),
                    ));
                });

                // House rules, one toggle per switch.
                header(col, "Rules");
                col.spawn(Node {
                    column_gap: Val::Px(10.0),
                    ..default()
                })
                .with_children(|row| {
                    button(row, "No foreign rest", LobbyButton::ForeignCamps);
                });

                // The roster: who is here, ready, or waiting for a corner.
                col.spawn((
                    Text::new(String::new()),
                    TextFont {
                        font_size: FontSize::Px(14.0),
                        ..default()
                    },
                    TextColor(Color::srgb(0.88, 0.88, 0.9)),
                    RosterText,
                ));

                col.spawn(Node {
                    column_gap: Val::Px(10.0),
                    margin: UiRect::top(Val::Px(4.0)),
                    ..default()
                })
                .with_children(|row| {
                    button(row, "Ready (Space)", LobbyButton::Ready);
                    button(row, "Start (Enter)", LobbyButton::Start);
                });
            });
        });
}

/// Marker on the corner-settings group that only makes sense once this peer
/// holds a corner of its own.
#[derive(Component)]
struct CornerSettings;

/// Fold the corner settings away until they can do something: in a shared
/// room, a peer that claims nothing only watches, so its corner controls would
/// be a row of refusals. `Display::None` rather than hidden visibility, so no
/// dead gap is left behind.
fn sync_corner_settings(net: Res<NetState>, mut groups: Query<&mut Node, With<CornerSettings>>) {
    let wanted = if net.peers.is_empty() || net.my_seat().is_some_and(|s| s.player.is_some()) {
        Display::Flex
    } else {
        Display::None
    };
    for mut node in groups.iter_mut() {
        if node.display != wanted {
            node.display = wanted;
        }
    }
}

/// Seconds between cursor broadcasts, and how long a silent cursor stays on
/// screen before it is hidden as gone.
const CURSOR_INTERVAL: f32 = 0.1;
const CURSOR_LINGER_SECS: f64 = 3.0;

/// A remote peer's pointer drawn over the lobby: where it was last reported,
/// where it is drawn (eased toward the report so it glides rather than jumps),
/// and when it was last heard from.
#[derive(Component)]
pub struct RemoteCursor {
    peer: String,
    target: Vec2,
    display: Vec2,
    last_seen: f64,
}

/// Broadcast this pointer at a lazy 10 Hz while the lobby is up. Positions are
/// window-logical, origin top-left — the same space bevy_ui lays out in — so
/// the receiver draws it where the sender saw it. Matchbox connects every peer
/// to every peer, so a plain broadcast reaches the whole room; the sender is
/// the `from` on arrival and no origin field is needed.
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
    let Some(pos) = windows.single().ok().and_then(|w| w.cursor_position()) else {
        return;
    };
    broadcast(
        &mut socket,
        &net.peers,
        &NetMsg::Cursor {
            pos: [pos.x, pos.y],
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
    // The easing constant from omdurman's overlay: fast enough to follow,
    // slow enough to hide the 10 Hz steps.
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

/// One labelled input row: the label, the text input box, and the key that
/// focuses it. Enter commits, Esc leaves; visuals follow in
/// [`sync_input_styles`].
fn field_row(parent: &mut ChildSpawnerCommands, label: &str, kind: FieldKind, key: &str) {
    parent
        .spawn(Node {
            column_gap: Val::Px(10.0),
            align_items: AlignItems::Center,
            ..default()
        })
        .with_children(|row| {
            row.spawn((
                Text::new(label),
                TextFont {
                    font_size: FontSize::Px(14.0),
                    ..default()
                },
                TextColor(Color::srgb(0.62, 0.62, 0.68)),
            ));
            input_box(row, kind);
            row.spawn((
                Text::new(key),
                TextFont {
                    font_size: FontSize::Px(14.0),
                    ..default()
                },
                TextColor(Color::srgb(0.62, 0.62, 0.68)),
            ));
        });
}

/// The corner-name row, a field row with a label that follows the selection.
fn corner_name_row(parent: &mut ChildSpawnerCommands) {
    parent
        .spawn(Node {
            column_gap: Val::Px(10.0),
            align_items: AlignItems::Center,
            ..default()
        })
        .with_children(|row| {
            row.spawn((
                Text::new("Corner name"),
                TextFont {
                    font_size: FontSize::Px(14.0),
                    ..default()
                },
                TextColor(Color::srgb(0.62, 0.62, 0.68)),
            ));
            input_box(row, FieldKind::Corner);
        });
}

/// A text input box, shared by every field row.
fn input_box(parent: &mut ChildSpawnerCommands, kind: FieldKind) {
    parent
        .spawn((
            Button,
            Node {
                width: Val::Px(240.0),
                padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(5.0)),
                ..default()
            },
            BackgroundColor(IDLE),
            BorderColor::all(Color::srgb(0.35, 0.35, 0.40)),
            TextInput(kind),
        ))
        .with_child((
            Text::new(String::new()),
            TextFont {
                font_size: FontSize::Px(14.0),
                ..default()
            },
            TextColor(Color::srgb(0.9, 0.9, 0.92)),
            InputText(kind),
        ));
}

/// Star geometry: the container the wedges and their labels live in.
const STAR_W: f32 = 420.0;
const STAR_H: f32 = 360.0;

/// Wedge geometry: each corner is an out-facing triangle sector, base at
/// [`WEDGE_INNER`] from the centre, apex at [`WEDGE_OUTER`] — the tip points
/// away from the star, like a ray, and the [`WEDGE_HALF_ANGLE_DEG`] spread
/// keeps a gap between neighbouring wedges. The inner radius is what leaves
/// the middle of the star empty and readable instead of covered by six
/// overlapping rectangles.
const WEDGE_INNER: f32 = 60.0;
const WEDGE_OUTER: f32 = 175.0;
const WEDGE_HALF_ANGLE_DEG: f32 = 26.0;

/// Corner `i`'s direction, in container coordinates (y down), so the angle
/// arithmetic matches [`Window::cursor_position`] directly.
fn wedge_angle(i: usize) -> f32 {
    (60.0 * i as f32 - 90.0).to_radians()
}

/// The three corners of corner `i`'s wedge, in container-local pixels: the
/// outward tip first, then the two base corners near the star.
fn wedge_vertices(i: usize) -> [Vec2; 3] {
    let t = wedge_angle(i);
    let beta = WEDGE_HALF_ANGLE_DEG.to_radians();
    let centre = Vec2::new(STAR_W / 2.0, STAR_H / 2.0);
    let dir = Vec2::new(t.cos(), t.sin());
    let left = Vec2::new((t + beta).cos(), (t + beta).sin());
    let right = Vec2::new((t - beta).cos(), (t - beta).sin());
    [
        centre + WEDGE_OUTER * dir,
        centre + WEDGE_INNER * left,
        centre + WEDGE_INNER * right,
    ]
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
                    top: Val::Px(mid.y - 17.0),
                    width: Val::Px(110.0),
                    height: Val::Px(36.0),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    row_gap: Val::Px(1.0),
                    ..default()
                })
                .with_children(|label| {
                    label.spawn((
                        Text::new("Empty"),
                        TextFont {
                            font_size: FontSize::Px(14.0),
                            ..default()
                        },
                        TextColor(Color::srgb(0.9, 0.9, 0.92)),
                        CornerText(i * 2),
                    ));
                    label.spawn((
                        Text::new("click to select"),
                        TextFont {
                            font_size: FontSize::Px(11.0),
                            ..default()
                        },
                        TextColor(Color::srgb(0.62, 0.62, 0.68)),
                        CornerText(i * 2 + 1),
                    ));
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
            "Only the host can start a shared game. Claim a corner and get ready.".into(),
        );
    }

    // Engines read as ready the moment they sit, so every not-ready seat is a
    // human who claimed a corner.
    let seated: Vec<&Seat> = net.seats.iter().filter(|s| s.player.is_some()).collect();
    if seated.len() < 2 {
        return StartDecision::Refuse(
            "At least two corners must be claimed before the game can start.".into(),
        );
    }
    let waiting: Vec<&str> = seated
        .iter()
        .filter(|s| !s.ready)
        .map(|s| s.name.as_str())
        .collect();
    if !waiting.is_empty() {
        return StartDecision::Refuse(format!("Waiting for: {}.", waiting.join(", ")));
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
/// claims, readiness, the host's roster broadcasts, and the `Start` that moves
/// everyone into the game. Runs on the socket every frame.
///
/// `pub` so the multiplayer integration test can run the real pump in a
/// headless instance, exactly as the app schedules it.
#[allow(clippy::too_many_arguments)]
pub fn pump_socket(
    socket: Option<ResMut<MatchboxSocket>>,
    mut net: ResMut<NetState>,
    mut variants: ResMut<ChosenVariants>,
    mut next_state: ResMut<NextState<AppState>>,
    state: Res<State<AppState>>,
    mut commands: Commands,
    time: Res<Time>,
    mut cursors: Query<(Entity, &mut RemoteCursor)>,
) {
    let Some(mut socket) = socket else {
        return;
    };

    // Announce ourselves to peers we have not greeted yet: once when we first
    // have a name, and afterwards only to peers that join later.
    let peers = net.peers.clone();
    let me = net.my_id.map(|id| id.to_string()).unwrap_or_default();
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
            NetMsg::Ready(ready) => {
                if net.sequences() {
                    let key = from.to_string();
                    if let Some(seat) = net.seats.iter_mut().find(|s| s.peer == key) {
                        seat.ready = ready;
                    }
                    publish_roster(&mut socket, &net, &peers);
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
                                    net.status = format!("{} claimed corner {corner}.", seat.name);
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
            // own presentation, and leftover dots must not haunt it.
            NetMsg::Cursor { pos } => {
                if *state.get() != AppState::Lobby {
                    continue;
                }
                let pos = Vec2::new(pos[0], pos[1]);
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
            NetMsg::Move(_) | NetMsg::Sequenced { .. } | NetMsg::Spectate(_) => {}
        }
    }
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
        ready: false,
        spectate: false,
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
        ready: true,
        spectate: false,
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
/// an empty corner on the spot — clicking the triangle *is* the claim — and
/// selecting your own corner hands it to the sidebar to configure or release.
#[derive(Debug, PartialEq, Eq)]
pub enum SectorClick {
    Select(usize),
    Claim(u32),
    Status(String),
}

pub fn sector_click(net: &NetState, me: &str, sector: usize) -> SectorClick {
    if net.peers.is_empty() {
        return SectorClick::Select(sector);
    }
    match net.seats.iter().find(|s| s.player == Some(sector as u32)) {
        None => SectorClick::Claim(sector as u32),
        Some(seat) if seat.peer == me => SectorClick::Select(sector),
        Some(seat) if seat.engine => {
            SectorClick::Status(format!("An engine plays corner {sector}."))
        }
        Some(seat) => SectorClick::Status(format!("Corner {sector} is held by {}.", seat.name)),
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
/// the wedge under the cursor is resolved by angle in [`sector_at`].
#[allow(clippy::type_complexity)]
pub fn select_corner(
    mut hit: Query<(&Interaction, &RelativeCursorPosition), (With<StarHit>, Changed<Interaction>)>,
    keys: Res<ButtonInput<KeyCode>>,
    mut socket: Option<ResMut<MatchboxSocket>>,
    mut net: ResMut<NetState>,
    mut selected: ResMut<SelectedCorner>,
) {
    for (interaction, rel) in hit.iter_mut() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let Some(sector) = rel.normalized.and_then(sector_at) else {
            continue;
        };
        let me = net.my_seat().map(|s| s.peer.clone()).unwrap_or_default();
        match sector_click(&net, &me, sector) {
            SectorClick::Select(i) => selected.0 = Some(i),
            SectorClick::Claim(c) => {
                if let Some(s) = socket.as_mut() {
                    net.status = send_claim(s, &mut net, Some(c), &me);
                }
            }
            SectorClick::Status(why) => net.status = why,
        }
    }
    if let Some(next) = corner_from_keys(&keys, selected.0) {
        selected.0 = Some(next);
    }
}

/// Clicking anywhere that is not a text field closes the focused one.
///
/// A click *on* a field — including the focused one — is not "elsewhere", so
/// reaching for another field never closes the one in hand. The system runs
/// after the lobby's action systems, but ordering is not what makes the modal
/// hold: while a field is active `not_editing` keeps every action system out
/// for the whole frame, so the dismissing click is spent on the dismissal and
/// the next click is the one that acts.
pub fn blur_on_elsewhere_click(
    mouse: Res<ButtonInput<MouseButton>>,
    clicked: Query<(&Interaction, Has<TextInput>), Changed<Interaction>>,
    mut room: ResMut<RoomEdit>,
    mut name: ResMut<NameEdit>,
    mut corner: ResMut<CornerEdit>,
) {
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }
    if clicked
        .iter()
        .any(|(interaction, is_field)| *interaction == Interaction::Pressed && is_field)
    {
        return;
    }
    if room.active {
        room.active = false;
        room.buffer.clear();
        room.error.clear();
    }
    if name.active {
        name.active = false;
        name.buffer.clear();
        name.error.clear();
    }
    if corner.active {
        corner.active = false;
        corner.buffer.clear();
        corner.error.clear();
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
}

pub fn corner_effect(
    net: &NetState,
    table: &Table,
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
                        Err("Only the host can seat an engine, and only on an empty corner.".into())
                    }
                };
            }
            return Err(format!("Corner {corner} is held by {}.", owner.name));
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
        CornerCommand::Human => {
            let name = match &table.0[corner as usize] {
                CornerState::Human(name) => name.clone(),
                _ => format!("P{corner}"),
            };
            CornerState::Human(name)
        }
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
            CornerState::Human(format!("P{i}"))
        } else {
            CornerState::Empty
        };
    }
}

/// A Bevy system: the parameter count is the world access it needs, so the
/// lint threshold is waived for it and [`focus_input_fields`].
#[allow(clippy::too_many_arguments)]
pub fn handle_buttons(
    buttons: Query<(&Interaction, &LobbyButton), Changed<Interaction>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut socket: Option<ResMut<MatchboxSocket>>,
    mut net: ResMut<NetState>,
    mut table: ResMut<Table>,
    mut selected: ResMut<SelectedCorner>,
    mut variants: ResMut<ChosenVariants>,
    mut next_state: ResMut<NextState<AppState>>,
) {
    let mut ready = keys.just_pressed(KeyCode::Space);
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
            LobbyButton::Ready => ready = true,
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

    if let Some(seating) = preset {
        if solo {
            apply_preset(&mut table, seating);
            net.status = format!("Table filled: {}.", seating.label());
        } else {
            net.status = "In a shared room, claim corners on the star instead.".into();
        }
    }

    if let (Some(corner), Some(cmd)) = (selected.0, command) {
        let corner = corner as u32;
        match corner_effect(&net, &table, &me, corner, cmd) {
            Ok(CornerEffect::Local(state)) => {
                table.0[corner as usize] = state;
                net.status = match cmd {
                    CornerCommand::Human => format!("Corner {corner}: human."),
                    CornerCommand::Cpu => format!("Corner {corner}: computer."),
                    CornerCommand::Off => format!("Corner {corner}: empty."),
                };
            }
            Ok(CornerEffect::Claim(claim)) => {
                if let Some(s) = socket.as_mut() {
                    net.status = send_claim(s, &mut net, claim, &me);
                }
            }
            Ok(CornerEffect::AddEngine(c)) => {
                seat_engine_at(&mut net, c as usize);
                if let Some(s) = socket.as_mut() {
                    publish_roster(s, &net, &net.peers);
                }
                net.status = format!("Engine seated at corner {c}.");
            }
            Ok(CornerEffect::RemoveEngine(c)) => {
                remove_engine_at(&mut net, c as usize);
                if let Some(s) = socket.as_mut() {
                    publish_roster(s, &net, &net.peers);
                }
                net.status = format!("Corner {c}: engine removed.");
            }
            Err(why) => net.status = why,
        }
    }

    if foreign {
        // Only the host decides the rules, for the same reason only the host
        // decides the table: the shared game must play under one rule set.
        if solo || net.sequences() {
            variants.0.forbid_foreign_camps = !variants.0.forbid_foreign_camps;
            // The switch is a setting like any other: broadcast it live, so
            // every lobby shows one game before the Start repeats it.
            if !solo && let Some(s) = socket.as_mut() {
                broadcast(
                    s,
                    &net.peers,
                    &NetMsg::Variants {
                        forbid_foreign_camps: variants.0.forbid_foreign_camps,
                    },
                );
            }
            net.status = if variants.0.forbid_foreign_camps {
                "Rule on: no piece rests in a foreign triangle.".into()
            } else {
                "Rule off: the specification's game.".into()
            };
        } else {
            net.status = "Only the host chooses the rules.".into();
        }
    }

    if ready {
        if solo {
            net.status = "Playing on this device - no ready sign needed.".into();
        } else if net.my_seat().is_none_or(|s| s.player.is_none()) {
            net.status = "Claim a corner first, then ready up.".into();
        } else {
            let now = !net.my_seat().is_some_and(|s| s.ready);
            if net.sequences() {
                if let Some(seat) = net.seats.iter_mut().find(|s| s.peer == me) {
                    seat.ready = now;
                }
                if let Some(s) = socket.as_mut() {
                    publish_roster(s, &net, &net.peers);
                }
            } else if let Some(s) = socket.as_mut() {
                broadcast(s, &net.peers, &NetMsg::Ready(now));
            }
            net.status = if now {
                "You are ready - waiting for the rest of the table.".into()
            } else {
                "Not ready.".into()
            };
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
            StartDecision::Refuse(why) => net.status = why,
        }
    }
}

/// Draw the roster and the running status line.
fn draw_roster(
    net: Res<NetState>,
    table: Res<Table>,
    mut text: Query<&mut Text, With<RosterText>>,
) {
    if !net.is_changed() && !table.is_changed() {
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
            let status = if seat.ready { "ready" } else { "..." };
            let engine = if seat.engine { " (computer)" } else { "" };
            out.push_str(&format!(
                "  {} {}  {}  {}{}\n",
                if seat.peer == me { ">" } else { " " },
                seat.name,
                corner,
                status,
                engine,
            ));
        }
    }

    out.push_str(if net.peers.is_empty() {
        "\nEnter starts this table locally.\nDigits 1-6 select a corner."
    } else {
        "\nClaim a corner on the star; Space readies you; Enter starts for everyone."
    });
    if !net.status.is_empty() {
        out.push_str(&format!("\n\n{}", net.status));
    }
    **text = out;
}

/// Draw the name input: the buffer with a caret while focused, the current
/// name otherwise.
fn draw_name(
    net: Res<NetState>,
    edit: Res<NameEdit>,
    mut values: Query<(&mut Text, &InputText)>,
    mut errors: Query<(&mut Text, &InputError), Without<InputText>>,
) {
    if !net.is_changed() && !edit.is_changed() {
        return;
    }
    for (mut text, kind) in &mut values {
        if kind.0 != FieldKind::Name {
            continue;
        }
        **text = if edit.active {
            format!("{}_", edit.buffer)
        } else if net.name.is_empty() {
            "(unnamed)".into()
        } else {
            net.name.clone()
        };
    }
    for (mut text, kind) in &mut errors {
        if kind.0 != FieldKind::Name {
            continue;
        }
        **text = if edit.active && !edit.error.is_empty() {
            edit.error.clone()
        } else {
            String::new()
        };
    }
}

/// Draw the room input: the buffer with a caret while focused, the current
/// room otherwise; any commit error shows on the line beneath.
fn draw_room(
    room: Res<RoomId>,
    edit: Res<RoomEdit>,
    mut values: Query<(&mut Text, &InputText)>,
    mut errors: Query<(&mut Text, &InputError), Without<InputText>>,
) {
    if !room.is_changed() && !edit.is_changed() {
        return;
    }
    for (mut text, kind) in &mut values {
        if kind.0 != FieldKind::Room {
            continue;
        }
        **text = if edit.active {
            format!("{}_", edit.buffer)
        } else {
            room.0.clone()
        };
    }
    for (mut text, kind) in &mut errors {
        if kind.0 != FieldKind::Room {
            continue;
        }
        **text = if edit.active && !edit.error.is_empty() {
            edit.error.clone()
        } else {
            String::new()
        };
    }
}

/// Draw the corner-name input: its buffer while focused, else the selected
/// corner's name on a solo setup, else nothing.
fn draw_corner(
    net: Res<NetState>,
    table: Res<Table>,
    selected: Res<SelectedCorner>,
    edit: Res<CornerEdit>,
    mut values: Query<(&mut Text, &InputText)>,
    mut errors: Query<(&mut Text, &InputError), Without<InputText>>,
) {
    if !net.is_changed() && !table.is_changed() && !selected.is_changed() && !edit.is_changed() {
        return;
    }
    let solo_name = if net.peers.is_empty() {
        selected.0.and_then(|i| match &table.0[i] {
            CornerState::Human(name) => Some(name.clone()),
            _ => None,
        })
    } else {
        None
    };
    for (mut text, kind) in &mut values {
        if kind.0 != FieldKind::Corner {
            continue;
        }
        **text = if edit.active {
            format!("{}_", edit.buffer)
        } else {
            solo_name.clone().unwrap_or_default()
        };
    }
    for (mut text, kind) in &mut errors {
        if kind.0 != FieldKind::Corner {
            continue;
        }
        **text = if edit.active && !edit.error.is_empty() {
            edit.error.clone()
        } else {
            String::new()
        };
    }
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
    let solo = net.peers.is_empty();
    for (petal, mut img) in petals.iter_mut() {
        let p = Player::new(petal.0 as u8).expect("corner indices are below six");
        let filled = if solo {
            table.0[petal.0] != CornerState::Empty
        } else {
            net.seats.iter().any(|s| s.player == Some(petal.0 as u32))
        };
        let base = if filled { player_colour(p) } else { IDLE };
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
fn draw_corner_labels(
    net: Res<NetState>,
    table: Res<Table>,
    mut texts: Query<(&CornerText, &mut Text)>,
) {
    if !net.is_changed() && !table.is_changed() {
        return;
    }
    let solo = net.peers.is_empty();
    for (label, mut text) in &mut texts {
        let i = label.0 / 2;
        let line = label.0 % 2;
        let p = Player::new(i as u8).expect("corner indices are below six");
        let (title, sub) = if solo {
            match &table.0[i] {
                CornerState::Empty => ("Empty".into(), "click to select".into()),
                CornerState::Human(name) => {
                    let name = if name.is_empty() {
                        format!("P{}", p.index())
                    } else {
                        name.clone()
                    };
                    (name, "human".into())
                }
                CornerState::Cpu => ("Computer".into(), "CPU".into()),
            }
        } else if let Some(seat) = net.seats.iter().find(|s| s.player == Some(i as u32)) {
            if seat.engine {
                ("Engine".into(), "CPU".into())
            } else {
                let sub = if seat.ready { "ready" } else { "..." };
                (seat.name.clone(), sub.into())
            }
        } else {
            ("Empty".into(), "click to claim".into())
        };
        let wanted = if line == 0 { title } else { sub };
        if **text != wanted {
            **text = wanted;
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

/// Apply the name editor's keypresses. Same classification as the other
/// fields, but committing writes the selected corner's name into [`Table`],
/// and only where the selected corner is human on a solo setup.
fn edit_corner(
    mut keys: MessageReader<KeyboardInput>,
    mut edit: ResMut<CornerEdit>,
    mut table: ResMut<Table>,
    selected: Res<SelectedCorner>,
    mut net: ResMut<NetState>,
) {
    edit.consumed_input = false;

    if !edit.active {
        keys.clear();
        return;
    }

    for event in keys.read() {
        if event.state != ButtonState::Pressed {
            continue;
        }
        edit.consumed_input = true;
        match edit_action(event.key_code, event.text.as_deref()) {
            EditAction::Insert(c) => {
                if edit.buffer.chars().count() < RoomId::MAX_LEN {
                    edit.buffer.push(c);
                    edit.error.clear();
                }
            }
            EditAction::Backspace => {
                edit.buffer.pop();
                edit.error.clear();
            }
            EditAction::Cancel => {
                edit.active = false;
                edit.buffer.clear();
                edit.error.clear();
            }
            EditAction::Commit => {
                let trimmed = edit.buffer.trim().to_string();
                if trimmed.is_empty() {
                    edit.error = "A name cannot be empty.".into();
                } else if let Some(i) = selected.0
                    && net.peers.is_empty()
                {
                    table.0[i] = CornerState::Human(trimmed);
                    edit.active = false;
                    edit.buffer.clear();
                    edit.error.clear();
                    net.status = format!("Corner {i} named.");
                }
            }
            EditAction::Ignore => {}
        }
    }
}

/// Focus an input box: by clicking it, or with its key (`R` room, `N` name).
/// Focusing one field unfocuses the others; each is seeded with its current
/// value, so a small change does not mean retyping the whole thing.
///
/// Runs only while no field holds the keyboard — the modal rule. Leaving a
/// field (Enter/Esc) is what frees the keys again.
///
/// A Bevy system: the parameter count is the world access it needs, so the
/// lint threshold is waived as with [`handle_buttons`].
#[allow(clippy::too_many_arguments)]
fn focus_input_fields(
    buttons: Query<(&Interaction, &TextInput), Changed<Interaction>>,
    keys: Res<ButtonInput<KeyCode>>,
    room: Res<RoomId>,
    mut net: ResMut<NetState>,
    mut room_edit: ResMut<RoomEdit>,
    mut name_edit: ResMut<NameEdit>,
    mut corner_edit: ResMut<CornerEdit>,
    table: Res<Table>,
    selected: Res<SelectedCorner>,
) {
    let mut clicked: Option<FieldKind> = None;
    for (interaction, input) in buttons.iter() {
        if *interaction == Interaction::Pressed {
            clicked = Some(input.0);
        }
    }

    let corner_ok = net.peers.is_empty()
        && selected
            .0
            .is_some_and(|i| matches!(table.0[i], CornerState::Human(_)));
    if clicked == Some(FieldKind::Corner) && !corner_ok {
        if net.peers.is_empty() {
            net.status = "Set this corner to Human before naming it.".into();
        }
        return;
    }

    let focus_room = clicked == Some(FieldKind::Room) || keys.just_pressed(KeyCode::KeyR);
    let focus_name = clicked == Some(FieldKind::Name) || keys.just_pressed(KeyCode::KeyN);
    let focus_corner = clicked == Some(FieldKind::Corner) && corner_ok;
    if !focus_room && !focus_name && !focus_corner {
        return;
    }

    room_edit.active = focus_room;
    name_edit.active = focus_name && !focus_room && !focus_corner;
    corner_edit.active = focus_corner;
    if focus_room {
        room_edit.buffer = room.0.clone();
        room_edit.error.clear();
    }
    if focus_name {
        name_edit.buffer = if net.name.is_empty() {
            String::new()
        } else {
            net.name.clone()
        };
        name_edit.error.clear();
    }
    if focus_corner {
        let name = selected.0.and_then(|i| match &table.0[i] {
            CornerState::Human(name) => Some(name.clone()),
            _ => None,
        });
        corner_edit.buffer = name.unwrap_or_default();
        corner_edit.error.clear();
    }
    // The key (or click) that opened a field belongs to it, not to the
    // systems chained after this one.
    room_edit.consumed_input = true;
    name_edit.consumed_input = true;
    corner_edit.consumed_input = true;
    net.status = "Editing. Enter accepts, Esc cancels.".into();
}

/// Apply the room-name editor's keypresses, and rejoin on commit.
///
/// Changing the room means **reopening the socket**: the room is baked into the
/// signaling URL, so editing [`RoomId`] alone would change the label and
/// nothing else. Removing `MatchboxSocket` makes `open_socket` run again on the
/// next entry to the lobby, and [`NetState::leave_room`] discards everything
/// the old room's socket told us.
pub fn edit_room(
    mut commands: Commands,
    mut keys: MessageReader<KeyboardInput>,
    mut edit: ResMut<RoomEdit>,
    mut room: ResMut<RoomId>,
    mut net: ResMut<NetState>,
    mut next_state: ResMut<NextState<AppState>>,
) {
    // Only ever true for the remainder of the frame that set it.
    edit.consumed_input = false;

    if !edit.active {
        // Drain regardless: a buffered keypress from before the field opened
        // must not appear in it later.
        keys.clear();
        return;
    }

    for event in keys.read() {
        if event.state != ButtonState::Pressed {
            continue;
        }
        // Whatever this key turns out to mean, it belonged to the field. Set
        // before the match so that closing the field below cannot let the same
        // press through to `handle_buttons` later in the chain.
        edit.consumed_input = true;
        match edit_action(event.key_code, event.text.as_deref()) {
            EditAction::Insert(c) => {
                if edit.buffer.chars().count() < RoomId::MAX_LEN {
                    edit.buffer.push(c);
                    edit.error.clear();
                }
            }
            EditAction::Backspace => {
                edit.buffer.pop();
                edit.error.clear();
            }
            EditAction::Cancel => {
                edit.active = false;
                edit.buffer.clear();
                edit.error.clear();
            }
            EditAction::Commit => match RoomId::parse(&edit.buffer) {
                Ok(parsed) => {
                    edit.active = false;
                    edit.error.clear();
                    if parsed == *room {
                        // Same room: rejoining would drop the peers already here
                        // for no reason.
                        net.status = format!("Already in room \"{}\".", room.0);
                        continue;
                    }
                    info!(from = %room.0, to = %parsed.0, "changing room");
                    *room = parsed;
                    // Publish the new room in the URL, so the address always
                    // points where this peer is.
                    crate::web::share_room(&room);
                    net.leave_room();
                    net.status = format!("Joining room \"{}\"...", room.0);
                    // Drop the old socket and re-enter the lobby, which reopens
                    // it against the new room.
                    commands.remove_resource::<MatchboxSocket>();
                    next_state.set(AppState::Lobby);
                }
                Err(why) => edit.error = why.to_string(),
            },
            EditAction::Ignore => {}
        }
    }
}

/// Apply the name editor's keypresses. Same classification as the room field,
/// but committing writes the display name and re-greets: the roster is only
/// ever exchanged on Hello, so a rename without a re-greet would leave every
/// peer looking at the old name.
pub fn edit_name(
    mut keys: MessageReader<KeyboardInput>,
    mut edit: ResMut<NameEdit>,
    mut net: ResMut<NetState>,
) {
    edit.consumed_input = false;

    if !edit.active {
        keys.clear();
        return;
    }

    for event in keys.read() {
        if event.state != ButtonState::Pressed {
            continue;
        }
        edit.consumed_input = true;
        match edit_action(event.key_code, event.text.as_deref()) {
            EditAction::Insert(c) => {
                if edit.buffer.chars().count() < RoomId::MAX_LEN {
                    edit.buffer.push(c);
                    edit.error.clear();
                }
            }
            EditAction::Backspace => {
                edit.buffer.pop();
                edit.error.clear();
            }
            EditAction::Cancel => {
                edit.active = false;
                edit.buffer.clear();
                edit.error.clear();
            }
            EditAction::Commit => {
                let trimmed = edit.buffer.trim().to_string();
                if trimmed.is_empty() {
                    edit.error = "A name cannot be empty.".into();
                } else {
                    edit.active = false;
                    if trimmed != net.name {
                        net.name = trimmed;
                        // Re-greet everyone, so the roster carries the new name.
                        net.greeted.clear();
                        net.status = "Name updated.".into();
                    }
                    edit.buffer.clear();
                }
            }
            EditAction::Ignore => {}
        }
    }
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

    fn seat(name: &str, player: Option<u32>, ready: bool) -> Seat {
        Seat {
            peer: name.into(),
            name: name.into(),
            player,
            ready,
            spectate: false,
            engine: false,
        }
    }

    /// A lone device with no peers deals whatever corners it configured.
    #[test]
    fn a_solo_table_starts_with_the_configured_corners() {
        let net = NetState::default();
        let mut table = Table::default();
        table.0[0] = CornerState::Human("Caro".into());
        table.0[3] = CornerState::Cpu;
        assert_eq!(start_decision(&net, &table), StartDecision::Solo);
    }

    /// One corner cannot be a game: the turn would visit a player's camp with
    /// nobody else to play.
    #[test]
    fn one_corner_cannot_start() {
        let net = NetState::default();
        let mut table = Table::default();
        table.0[0] = CornerState::Human("Caro".into());
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

    /// The host is told who it is waiting for.
    #[test]
    fn the_host_is_told_who_it_is_waiting_for() {
        let mut net = NetState::default();
        net.peers.push(fake_peer());
        net.is_host = true;
        net.seats = vec![seat("ada", Some(0), true), seat("grace", Some(3), false)];

        match start_decision(&net, &Table::default()) {
            StartDecision::Refuse(why) => {
                assert!(why.contains("grace"), "should name who is not ready: {why}");
                assert!(
                    !why.contains("ada"),
                    "should not name a ready player: {why}"
                );
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    /// Fewer than two claimed corners cannot start, whatever the readiness.
    #[test]
    fn a_shared_start_needs_two_claimed_corners() {
        let mut net = NetState::default();
        net.peers.push(fake_peer());
        net.is_host = true;
        net.seats = vec![seat("ada", Some(0), true)];
        assert!(
            matches!(
                start_decision(&net, &Table::default()),
                StartDecision::Refuse(_)
            ),
            "one claim cannot be a game"
        );
    }

    #[test]
    fn the_host_starts_once_everyone_is_ready() {
        let mut net = NetState::default();
        net.peers.push(fake_peer());
        net.is_host = true;
        net.seats = vec![seat("ada", Some(0), true), seat("grace", Some(3), true)];
        assert_eq!(
            start_decision(&net, &Table::default()),
            StartDecision::Multiplayer
        );
    }

    /// A claimed corner, a seat that needs no readiness: engines read as
    /// ready from the moment they sit, and count toward the two claims.
    #[test]
    fn engines_count_toward_the_two_corners() {
        let mut net = NetState::default();
        net.peers.push(fake_peer());
        net.is_host = true;
        net.seats = vec![seat("ada", Some(0), true)];
        let mut engine = seat("engine-0", Some(3), false);
        engine.engine = true;
        engine.ready = true;
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
            seats: vec![seat("ada", Some(3), true), seat("grace", Some(0), true)],
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
        let effect = corner_effect(&net, &Table::default(), "", 2, CornerCommand::Cpu);
        assert_eq!(effect, Ok(CornerEffect::AddEngine(2)));
        seat_engine_at(&mut net, 2);
        assert!(net.seats[0].engine);
        assert!(net.seats[0].ready, "an engine must never block a start");
        assert_eq!(net.seats[0].player, Some(2));

        // The host removes it again.
        assert_eq!(
            corner_effect(&net, &Table::default(), "", 2, CornerCommand::Off),
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

        let effect = corner_effect(&net, &Table::default(), "host", 1, CornerCommand::Human)
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
        net.seats = vec![seat("ada", Some(2), true)];
        let err = corner_effect(&net, &Table::default(), "grace", 2, CornerCommand::Human)
            .expect_err("taken");
        assert!(err.contains("ada"), "must name the holder: {err}");
    }

    /// Pressing "Off" on my own corner releases it; anything else is refused
    /// with the reason attached.
    #[test]
    fn releasing_my_own_corner() {
        let mut net = NetState::default();
        net.peers.push(fake_peer());
        net.seats = vec![seat("ada", Some(2), true)];
        assert_eq!(
            corner_effect(&net, &Table::default(), "ada", 2, CornerCommand::Off),
            Ok(CornerEffect::Claim(None))
        );
        assert!(corner_effect(&net, &Table::default(), "ada", 2, CornerCommand::Human).is_err());
    }

    /// A guest cannot place an engine; that is the host's call.
    #[test]
    fn only_the_host_seats_engines() {
        let net = NetState {
            peers: vec![fake_peer()],
            ..Default::default()
        };
        assert!(corner_effect(&net, &Table::default(), "grace", 1, CornerCommand::Cpu).is_err());
        let host = NetState {
            is_host: true,
            peers: vec![fake_peer()],
            ..Default::default()
        };
        assert_eq!(
            corner_effect(&host, &Table::default(), "host", 1, CornerCommand::Cpu),
            Ok(CornerEffect::AddEngine(1))
        );
    }

    /// Solo commands rewrite the local table, keeping an existing name when a
    /// human corner is toggled off and on.
    #[test]
    fn solo_commands_rewrite_the_table() {
        let net = NetState::default();
        let mut table = Table::default();
        table.0[0] = CornerState::Human("Caro".into());
        assert_eq!(
            corner_effect(&net, &table, "", 0, CornerCommand::Cpu),
            Ok(CornerEffect::Local(CornerState::Cpu))
        );
        assert_eq!(
            corner_effect(&net, &table, "", 0, CornerCommand::Human),
            Ok(CornerEffect::Local(CornerState::Human("Caro".into())))
        );
        assert_eq!(
            corner_effect(&net, &table, "", 0, CornerCommand::Off),
            Ok(CornerEffect::Local(CornerState::Empty))
        );
    }

    /// The preset fills exactly the seating's corners, presenting the
    /// symmetric game the shortcut claims.
    #[test]
    fn presets_fill_the_seatings_corners() {
        let mut table = Table::default();
        apply_preset(&mut table, Seating::Two);
        assert_eq!(table.0[0], CornerState::Human("P0".into()));
        assert_eq!(table.0[3], CornerState::Human("P3".into()));
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

    #[test]
    fn ordinary_characters_are_inserted() {
        for (key, text, want) in [
            (KeyCode::KeyA, Some("a"), 'a'),
            (KeyCode::KeyZ, Some("Z"), 'Z'),
            (KeyCode::Digit4, Some("4"), '4'),
            (KeyCode::Minus, Some("-"), '-'),
            (KeyCode::KeyY, Some("z"), 'z'),
        ] {
            assert_eq!(edit_action(key, text), EditAction::Insert(want));
        }
    }

    #[test]
    fn the_editing_keys_are_recognised() {
        assert_eq!(edit_action(KeyCode::Enter, None), EditAction::Commit);
        assert_eq!(edit_action(KeyCode::NumpadEnter, None), EditAction::Commit);
        assert_eq!(edit_action(KeyCode::Escape, None), EditAction::Cancel);
        assert_eq!(edit_action(KeyCode::Backspace, None), EditAction::Backspace);
    }

    /// The deal must run on the configured corners: `apply_seats` rebuilds the
    /// session from them, and a session built for two corners must not be
    /// holding a board nobody set up. Solo setups read the local table; the
    /// engine camps match; all-engine tables are spectated.
    #[test]
    fn the_configured_corners_build_the_session() {
        let net = NetState::default();
        let mut table = Table::default();
        table.0[0] = CornerState::Human("Caro".into());
        table.0[2] = CornerState::Cpu;
        table.0[4] = CornerState::Human("Lee".into());
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
        net.seats = vec![seat(&host, Some(0), true), seat("grace", Some(3), true)];
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
        assert_eq!(sector_click(&net, "", 3), SectorClick::Select(3));
    }

    /// Shared: an empty wedge is claimed on the spot — the click is the claim.
    #[test]
    fn a_shared_click_on_an_empty_corner_claims_it() {
        let mut net = NetState::default();
        let me = PeerId(uuid::Uuid::from_u128(1));
        net.my_id = Some(me);
        net.peers = vec![me, fake_peer()];
        net.seats = vec![seat(&me.to_string(), None, false)];
        assert_eq!(
            sector_click(&net, &me.to_string(), 5),
            SectorClick::Claim(5)
        );
    }

    /// Shared: clicking your own corner selects it for the sidebar; someone
    /// else's corner is refused by name, and an engine corner is not touchable.
    #[test]
    fn a_shared_click_on_an_owned_corner_selects_or_refuses() {
        let mut net = NetState::default();
        let me = PeerId(uuid::Uuid::from_u128(1));
        let other = fake_peer();
        net.my_id = Some(me);
        net.peers = vec![me, other];
        net.seats = vec![
            seat(&me.to_string(), Some(2), false),
            Seat {
                engine: true,
                ..seat("bot", Some(4), true)
            },
        ];
        let me = me.to_string();
        assert_eq!(sector_click(&net, &me, 2), SectorClick::Select(2));
        match sector_click(&net, &me, 4) {
            SectorClick::Status(why) => assert!(why.contains("engine"), "{why}"),
            other => panic!("an engine corner is not clickable, got {other:?}"),
        }
        match sector_click(&net, &me, 1) {
            SectorClick::Claim(1) => {}
            other => panic!("corner 1 is free, got {other:?}"),
        }
    }

    /// An elsewhere-click — on a bare button or on no UI at all — closes the
    /// focused field; clicking a field itself never does.
    #[test]
    fn an_elsewhere_click_closes_the_focused_field() {
        use bevy::ecs::system::RunSystemOnce;

        let mut world = World::new();
        world.init_resource::<ButtonInput<MouseButton>>();
        world.init_resource::<RoomEdit>();
        world.init_resource::<NameEdit>();
        world.init_resource::<CornerEdit>();
        world.resource_mut::<RoomEdit>().active = true;

        // A press on a bare button (a stand-in: the star hit area, the ready
        // button) is an elsewhere-click.
        world
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        world.spawn((Button, Interaction::Pressed));
        world.run_system_once(blur_on_elsewhere_click).unwrap();
        assert!(
            !world.resource::<RoomEdit>().active,
            "clicking outside the fields must close the focused one"
        );

        // A press on a text field is not "elsewhere": focus survives it.
        world.resource_mut::<RoomEdit>().active = true;
        world
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        world.spawn((Button, Interaction::Pressed, TextInput(FieldKind::Name)));
        world.run_system_once(blur_on_elsewhere_click).unwrap();
        assert!(
            world.resource::<RoomEdit>().active,
            "a click on a field is not an elsewhere-click"
        );

        // No left press this frame, no blur — a mere hover is not a dismissal.
        world.resource_mut::<ButtonInput<MouseButton>>().clear();
        world.spawn((Button, Interaction::Hovered));
        world.run_system_once(blur_on_elsewhere_click).unwrap();
        assert!(world.resource::<RoomEdit>().active);
    }
}
