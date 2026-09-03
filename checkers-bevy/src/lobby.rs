//! Lobby screen: join a room, choose how many players, see who is here, start.
//!
//! Built with plain Bevy UI rather than egui, to match the in-game buttons and
//! to avoid a dependency for a handful of widgets.
//!
//! Every control has a key *and* a button, and neither is the primary: the keys
//! are what the hints name, and the buttons exist because a lobby whose only
//! affordances are typed is indistinguishable from an empty screen. That is not
//! a hypothetical either: the buttons were once laid out below the bottom of
//! the window, behind the taskbar, and so never drawn at all.
//!
//! # Choosing the room
//!
//! `R` opens the room field; Enter joins, Esc cancels. Changing the room
//! **reopens the socket**, because the room is part of the signaling URL — see
//! [`edit_room`]. Editing is modal, and every other key is suppressed while it
//! holds the keyboard, or typing a name containing `s` would start a game.
//!
//! # Choosing the seating
//!
//! `2`, `3`, `6`, or `Tab` to cycle; see [`crate::setup`] for what a seating is
//! and why only those three counts are offered. Only the host may change it,
//! since it describes the shared game rather than one peer's view of it.
//!
//! # Leaving the lobby
//!
//! **`S` always starts a solo game**, every seated player driven locally,
//! without consulting the network at all. That escape hatch is not a
//! convenience: the board is only spawned on entering [`AppState::InGame`], so
//! *any* way for the lobby to become unleaveable shows up as a blank window with
//! no explanation.
//!
//! It has happened. Peers from an unrelated project shared the old default room
//! on the public signaling server, this build lost the host election, and it
//! waited forever for a `Start` nobody was going to send. Enter still starts a
//! shared game, but every way it can refuse names `S`.
//!
//! # Host election
//!
//! The peer with the lexicographically smallest `PeerId` hosts. That is not
//! elegant, but it is *deterministic without negotiation*: every peer computes
//! the same answer from the same peer list, so there is no election round-trip
//! and no window in which two peers both believe they are host. When the host
//! leaves, the next-smallest id takes over on the next frame.
//!
//! # Seats
//!
//! The host owns the roster. Guests announce themselves with
//! [`NetMsg::Hello`] and toggle [`NetMsg::Ready`]; everything else is the host
//! broadcasting [`NetMsg::Roster`]. Seats are assigned in join order, which
//! keeps the assignment rule stateless — a guest never has to reconcile two
//! sources of truth about which player it commands.

use bevy::input::ButtonState;
use bevy::input::keyboard::KeyboardInput;
use bevy::prelude::*;
use bevy_matchbox::prelude::*;
use checkers_net::{CH_RELIABLE, NetMsg, NetState, RoomId, Seat, broadcast, decode};

use crate::setup::Seating;
use crate::{AppState, Session};

/// Push the host's roster to every peer. The roster broadcast follows every
/// roster change, so it lives in one place.
fn publish_roster(socket: &mut MatchboxSocket, net: &NetState, peers: &[PeerId]) {
    broadcast(socket, peers, &NetMsg::Roster(net.seats.clone()));
}

/// Marker for everything spawned by the lobby, so leaving despawns it wholesale.
#[derive(Component)]
pub struct LobbyUi;

#[derive(Component)]
pub enum LobbyButton {
    Ready,
    Start,
    /// Always available, never network-dependent.
    Solo,
    /// Pick a seating. One button per option rather than a cycling control, so
    /// the current choice and the alternatives are visible at once.
    Seats(Seating),
    /// Declare or renounce spectator status.
    Spectate,
    /// Back to the main menu.
    Back,
}

/// Which editor an on-screen text input drives.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum FieldKind {
    Room,
    Name,
}

/// An always-visible text input box: click to focus (or its key), Enter
/// commits, Esc leaves. The value shown is driven by [`draw_room`] and
/// [`draw_name`].
#[derive(Component)]
struct TextInput(FieldKind);

/// The value text inside an input box.
#[derive(Component)]
struct InputText(FieldKind);

/// The error line under an input box.
#[derive(Component)]
struct InputError(FieldKind);

#[derive(Component)]
struct RosterText;

/// The chosen seating, set in the lobby and read when the game starts.
///
/// A resource rather than a field of [`Session`] because the session is rebuilt
/// from it on entering the game — reading a value out of the thing it
/// initialises would be circular.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChosenSeating(pub Seating);

pub fn plugin(app: &mut App) {
    // A share link carries the room in the URL fragment; honour it over the
    // default so a link lands you in the sender's lobby. Native builds have no
    // URL, and this is a no-op there.
    if let Some(room) = crate::web::room_from_url() {
        app.insert_resource(room);
    }
    app.init_resource::<NetState>()
        .init_resource::<RoomId>()
        .init_resource::<ChosenSeating>()
        .init_resource::<RoomEdit>()
        .init_resource::<NameEdit>()
        .add_systems(OnEnter(AppState::Lobby), (checkers_net::open_socket, spawn))
        .add_systems(OnExit(AppState::Lobby), despawn)
        .add_systems(
            Update,
            (
                elect_host,
                pump_socket,
                // First, and the rest are suppressed while a field holds the
                // keyboard: typing a room named "solo" must not start a game on
                // the `s`.
                (edit_room, edit_name),
                focus_input_fields.run_if(not_editing),
                (choose_seating, handle_buttons).run_if(not_editing),
                sync_button_styles,
                sync_input_styles,
                draw_roster,
                draw_room,
                draw_name,
            )
                .chain()
                .run_if(in_state(AppState::Lobby)),
        );
}

/// Whether a field has the keyboard.
///
/// Every other lobby key is gated on this. Without it each character typed is
/// also a command — `s` starts a solo game, `2` changes the seating, Enter
/// starts a shared one — so a field would be unusable for any name containing
/// them, which is nearly all of them.
pub fn not_editing(room: Res<RoomEdit>, name: Res<NameEdit>) -> bool {
    !room.active && !room.consumed_input && !name.active && !name.consumed_input
}

/// Paint every button: selected mode, hover, press.
///
/// Selected mode per button: the chosen seating for the seat buttons, the
/// open editor for Room/Name, this peer's seat flags for Ready and Spectate.
/// Momentary actions (Start, Solo, Back) have no mode to show. Runs every
/// frame — a dozen buttons — and writes only real changes, so hover repaints
/// immediately without dirtying the UI otherwise.
fn sync_button_styles(
    chosen: Res<ChosenSeating>,
    net: Res<NetState>,
    mut buttons: Query<(&Interaction, &LobbyButton, &mut BackgroundColor)>,
) {
    for (interaction, button, mut bg) in buttons.iter_mut() {
        let selected = match button {
            LobbyButton::Seats(s) => *s == chosen.0,
            LobbyButton::Ready => net.my_seat().is_some_and(|s| s.ready && !s.spectate),
            LobbyButton::Spectate => net.my_seat().is_some_and(|s| s.spectate),
            LobbyButton::Start | LobbyButton::Solo | LobbyButton::Back => false,
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

/// One button. Factored out because the lobby now spawns two rows of them and
/// the padding, radius, and text styling must not drift between the rows.
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

/// An unselected button; hover and press brighten then darken it. `CHOSEN`
/// marks the button whose mode is active, with its own hover and press.
/// Shared with the menu screens so every screen speaks one vocabulary.
pub(crate) const IDLE: Color = Color::srgb(0.22, 0.22, 0.27);
pub(crate) const HOVER: Color = Color::srgb(0.29, 0.29, 0.35);
pub(crate) const DOWN: Color = Color::srgb(0.17, 0.17, 0.21);
pub(crate) const CHOSEN: Color = Color::srgb(0.20, 0.45, 0.28);
pub(crate) const CHOSEN_HOVER: Color = Color::srgb(0.25, 0.53, 0.34);
pub(crate) const CHOSEN_DOWN: Color = Color::srgb(0.16, 0.37, 0.23);

/// The room-name editor.
///
/// Editing is *modal*: while [`RoomEdit::active`] every other lobby key is
/// suppressed, because the alternative is that typing a room called "solo"
/// starts a solo game on the `s`. A mode is the smaller evil here, and `Esc`
/// always leaves it.
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
    /// then fell through and started the game. Typing a room name dropped
    /// straight onto the board.
    ///
    /// A run condition cannot see "this frame's input was already used", so the
    /// editor records it. Cleared at the top of each `edit_room` run.
    pub consumed_input: bool,
}

/// The player-name editor. Same modal pattern as [`RoomEdit`] — including the
/// input-consumption flag, for the same reason — but committing writes the
/// display name and re-greets peers rather than reopening a socket.
#[derive(Resource, Default)]
pub struct NameEdit {
    pub active: bool,
    pub buffer: String,
    pub error: String,
    pub consumed_input: bool,
}

/// What a keypress does to the room-name editor.
///
/// Returned rather than applied so the decision is testable without a window;
/// [`edit_room`] is the thin system that performs it.
#[derive(Debug, PartialEq, Eq)]
pub enum EditAction {
    /// Add to the buffer.
    Insert(char),
    Backspace,
    /// Commit the buffer as the new room.
    Commit,
    /// Abandon the edit, keeping the current room.
    Cancel,
    /// Not for the editor.
    Ignore,
}

/// Classify one keypress during editing.
///
/// `text` is [`bevy::input::keyboard::KeyboardInput::text`], which respects the
/// keyboard layout — reading `key_code` instead would give a US-layout guess and
/// type the wrong characters on this machine's keyboard.
pub fn edit_action(key: KeyCode, text: Option<&str>) -> EditAction {
    match key {
        KeyCode::Enter | KeyCode::NumpadEnter => return EditAction::Commit,
        KeyCode::Escape => return EditAction::Cancel,
        KeyCode::Backspace => return EditAction::Backspace,
        _ => {}
    }
    // A single character only: `text` can hold two when a dead key did not
    // combine, and a room name is not the place to resolve that.
    match text.and_then(|t| {
        let mut chars = t.chars();
        chars.next().filter(|_| chars.next().is_none())
    }) {
        // Control characters arrive here as text on some platforms; they are not
        // valid in a room name and must not enter the buffer.
        Some(c) if !c.is_control() => EditAction::Insert(c),
        _ => EditAction::Ignore,
    }
}

/// The lobby is one **flex column filling the window**, not a set of
/// absolutely-positioned corners.
///
/// The previous layout anchored the buttons at `bottom: 40px`, which put them
/// off-screen entirely on a display whose work area is shorter than the window:
/// they were laid out at y=2307 of a 2450px surface, behind the taskbar. A
/// window-filling column with the content at the top cannot place anything
/// outside the window, whatever the window's size — so the failure mode is gone
/// by construction rather than by choosing a better offset.
fn spawn(mut commands: Commands, chosen: Res<ChosenSeating>) {
    commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                row_gap: Val::Px(14.0),
                padding: UiRect::all(Val::Px(40.0)),
                ..default()
            },
            LobbyUi,
        ))
        .with_children(|col| {
            header(col, "Lobby");

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

            // Seating, with the keys that also set it.
            header(col, "Players at this table");
            col.spawn(Node {
                column_gap: Val::Px(10.0),
                ..default()
            })
            .with_children(|row| {
                for seating in Seating::ALL {
                    button(row, seating.label(), LobbyButton::Seats(seating));
                }
            });

            // The roster: who is here, ready, or spectating.
            col.spawn((
                Text::new(String::new()),
                TextFont {
                    font_size: FontSize::Px(15.0),
                    ..default()
                },
                TextColor(Color::srgb(0.88, 0.88, 0.9)),
                RosterText,
            ));

            col.spawn(Node {
                column_gap: Val::Px(10.0),
                margin: UiRect::top(Val::Px(6.0)),
                ..default()
            })
            .with_children(|row| {
                button(row, "Ready (Space)", LobbyButton::Ready);
                button(row, "Start (Enter)", LobbyButton::Start);
                button(row, "Spectate (P)", LobbyButton::Spectate);
                button(row, "Solo (S)", LobbyButton::Solo);
                button(row, "Back (Esc)", LobbyButton::Back);
            });
        });

    // Paint the initial choice, so the highlight is right on the first frame
    // rather than only after the first interaction.
    commands.insert_resource(*chosen);
}

/// A section heading in lobby/menu screens.
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
            row.spawn((
                Button,
                Node {
                    width: Val::Px(260.0),
                    padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::all(Val::Px(5.0)),
                    ..default()
                },
                BackgroundColor(IDLE),
                BorderColor {
                    top: Color::srgb(0.35, 0.35, 0.40),
                    right: Color::srgb(0.35, 0.35, 0.40),
                    bottom: Color::srgb(0.35, 0.35, 0.40),
                    left: Color::srgb(0.35, 0.35, 0.40),
                },
                TextInput(kind),
            ))
            .with_children(|box_node| {
                box_node.spawn((
                    Text::new(String::new()),
                    TextFont {
                        font_size: FontSize::Px(15.0),
                        ..default()
                    },
                    TextColor(Color::srgb(0.9, 0.9, 0.92)),
                    InputText(kind),
                ));
            });
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

fn despawn(mut commands: Commands, ui: Query<Entity, With<LobbyUi>>) {
    for e in ui.iter() {
        commands.entity(e).despawn();
    }
}

/// What pressing Enter should do.
///
/// Split out from `handle_buttons` because it is the part that was wrong: the
/// original required a non-empty, all-ready roster, and a solo player's seat is
/// only created once the socket connects and starts out `ready: false`. So a
/// lone player faced a blank screen — the board only exists in
/// [`AppState::InGame`] — with Enter doing nothing and no explanation.
#[derive(Debug, PartialEq, Eq)]
pub enum StartDecision {
    /// Nobody else is here: play all six players locally.
    Solo,
    /// Tell the peers and begin.
    Multiplayer,
    /// Refuse, with a reason to show the player.
    Refuse(String),
}

pub fn start_decision(net: &NetState, chosen: Seating) -> StartDecision {
    if net.peers.is_empty() {
        return StartDecision::Solo;
    }

    // A guest cannot start a *shared* game, and must be told so rather than
    // having Enter do nothing. This is not hypothetical: the old default room on
    // the shared signaling server collided with another project's sessions, so
    // this build found two peers, lost the host election, and sat in a lobby it
    // could never leave — which is what a blank screen looked like from outside.
    //
    // Every refusal names `S` because that key never consults the network.
    if !net.sequences() {
        return StartDecision::Refuse(
            "Only the host can start a shared game. Press S to play solo.".into(),
        );
    }

    let waiting: Vec<&str> = net
        .seats
        .iter()
        // Spectators have no say in when a game starts, so they cannot make
        // anyone wait.
        .filter(|s| !s.spectate && !s.ready)
        .map(|s| s.name.as_str())
        .collect();
    if waiting.is_empty() {
        // A start with fewer players than the seating's camps would strand the
        // uncontrolled ones: the turn would reach a player nobody commands and
        // wait forever. Choosing Six with two joined players must be a
        // refusal, not a crippled game. Spectators command nothing, so only
        // seated players count.
        let players = net.seats.iter().filter(|s| !s.spectate).count();
        if players == chosen.count() {
            StartDecision::Multiplayer
        } else {
            StartDecision::Refuse(format!(
                "{} joined but this seating needs {}. Press Tab or 2/3/6 to change it, \
                 or S to play solo.",
                players,
                chosen.count()
            ))
        }
    } else {
        StartDecision::Refuse(format!(
            "Waiting for: {}. Press S to play solo instead.",
            waiting.join(", ")
        ))
    }
}

/// Smallest `PeerId` hosts. Recomputed every frame so host loss self-heals.
fn elect_host(socket: Option<ResMut<MatchboxSocket>>, mut net: ResMut<NetState>) {
    let Some(mut socket) = socket else {
        return;
    };

    for (peer, state) in socket.update_peers() {
        match state {
            PeerState::Connected => {
                if !net.peers.contains(&peer) {
                    net.peers.push(peer);
                }
            }
            PeerState::Disconnected => net.peers.retain(|p| *p != peer),
        }
    }

    if net.my_id.is_none() {
        net.my_id = socket.id();
    }
    let Some(me) = net.my_id else {
        return;
    };

    let was_host = net.is_host;
    net.is_host = net.peers.iter().all(|p| me.to_string() < p.to_string());

    // A guest that just became host inherits the roster it already has, and
    // takes over sequencing from the next move. Nothing to migrate: `next_seq`
    // is derived from what has been applied, not from the departed host.
    if net.is_host && !was_host {
        net.next_seq = net.last_applied_seq.map_or(0, |s| s + 1);
        info!("became host");
    }
}

fn pump_socket(
    socket: Option<ResMut<MatchboxSocket>>,
    mut net: ResMut<NetState>,
    mut chosen: ResMut<ChosenSeating>,
    mut next_state: ResMut<NextState<AppState>>,
) {
    let Some(mut socket) = socket else {
        return;
    };

    // Announce ourselves to peers we have not greeted yet: once when we first
    // have a name, and afterwards only to peers that join later. Greeting
    // everyone every frame would flood the channel — the host ignores repeat
    // Hellos, but the traffic is not free.
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
            NetMsg::Spectate(spectate) => {
                if net.sequences() {
                    let key = from.to_string();
                    if let Some(seat) = net.seats.iter_mut().find(|s| s.peer == key) {
                        // Leaving spectator mode must not strand the seat: it
                        // gets no camp until the host next starts a game.
                        seat.spectate = spectate;
                        if spectate {
                            seat.ready = true;
                        }
                    }
                    publish_roster(&mut socket, &net, &peers);
                }
            }
            // Guests take the host's roster verbatim; it is the only authority.
            NetMsg::Roster(seats) => net.seats = seats,
            NetMsg::Seating(players) => {
                // The host's live choice: adopt it, unless it is unrecognised
                // (another build's seating), in which case keep ours and say
                // so — the same rule as on `Start`.
                if net.sequences() {
                    continue;
                }
                match adopt_seating(&players, chosen.0) {
                    Ok(seating) => {
                        if chosen.0 != seating {
                            chosen.0 = seating;
                            net.status = format!("Host set the table: {}.", seating.label());
                        }
                    }
                    Err(complaint) => net.status = complaint,
                }
            }
            NetMsg::Start { seats, players } => {
                net.seats = seats;
                match adopt_seating(&players, chosen.0) {
                    Ok(seating) => chosen.0 = seating,
                    Err(complaint) => {
                        warn!(?players, "unrecognised seating from host");
                        net.status = complaint;
                    }
                }
                next_state.set(AppState::InGame);
            }
            // Moves cannot arrive before the game starts, but a late duplicate
            // from a previous game in the same room could. Ignore rather than
            // mis-apply.
            NetMsg::Move(_) | NetMsg::Sequenced { .. } => {}
        }
    }
}

/// Add a seat if this peer has none, assigning the lowest free player index.
fn seat_for(net: &mut NetState, peer: &str, name: &str) {
    if net.seats.iter().any(|s| s.peer == peer) {
        return;
    }
    let taken: Vec<u32> = net.seats.iter().filter_map(|s| s.player).collect();
    let player = (0..6).find(|i| !taken.contains(i));
    net.seats.push(Seat {
        peer: peer.to_string(),
        name: name.to_string(),
        player,
        ready: false,
        spectate: false,
    });
}

/// Which seating to deal on receiving the host's `Start`.
///
/// The host's choice wins outright, because every peer must deal the same board.
/// An unrecognised set means the host runs a build offering a seating this one
/// does not; the local choice is kept and the player is told, rather than a
/// different game being dealt in silence.
///
/// Returns the complaint as an `Err` payload so the caller decides where it is
/// shown, and so a test can assert the player is actually told.
fn adopt_seating(players: &[u32], current: Seating) -> Result<Seating, String> {
    Seating::from_indices(players).ok_or_else(|| {
        format!(
            "The host chose a seating this build does not know ({players:?}); \
             starting with {} instead.",
            current.label()
        )
    })
}

/// The `Start` the host broadcasts: the final roster and the seating to deal.
///
/// A named function rather than an inline struct literal so a test can assert
/// what actually goes on the wire. That is not ceremony: I injected a fault here
/// — sending [`Seating::default`] instead of the host's choice, reproducing the
/// exact bug this field was added to fix — and the whole suite still passed,
/// because every test built its own `NetMsg::Start` and none covered the code
/// that builds the real one.
pub fn start_message(net: &NetState, seating: Seating) -> NetMsg {
    NetMsg::Start {
        seats: net.seats.clone(),
        players: seating.indices(),
    }
}

/// Bind the roster's join-order slots to the chosen seating's camp indices.
///
/// [`seat_for`] hands out roster slots in join order (0, 1, 2, …), but a
/// seating plays with *specific* camps — Two is {0, 3}, Three is {0, 2, 4} —
/// so the k-th peer to join commands `players()[k]`, and any further peers sit
/// out as spectators. Without this rebinding the two lists disagree: in
/// two-player mode the seat list said player 1 while the board's second camp
/// belonged to player 3, so when the turn reached camp 3 nobody controlled it
/// and every screen sat on "Waiting for player 3" forever.
///
/// Runs on the host at start, before [`start_message`] clones the seats, so
/// the host's own [`NetState`] and every guest's arrive at the same binding.
fn assign_seating(net: &mut NetState, seating: Seating) {
    // Camps go to seated players in join order; spectators command nothing,
    // and — as ever — peers beyond the seating's camps sit out.
    let mut next = seating.indices().into_iter();
    for seat in net.seats.iter_mut().filter(|s| !s.spectate) {
        seat.player = next.next();
    }
    for seat in net.seats.iter_mut().filter(|s| s.spectate) {
        seat.player = None;
    }
}

/// Which seating a keypress selects. `Tab` cycles, so the whole control is
/// reachable without knowing the individual digits.
fn seating_from_keys(keys: &ButtonInput<KeyCode>, current: Seating) -> Option<Seating> {
    if keys.just_pressed(KeyCode::Tab) {
        return Some(current.next());
    }
    for (key, seating) in [
        (KeyCode::Digit2, Seating::Two),
        (KeyCode::Digit3, Seating::Three),
        (KeyCode::Digit6, Seating::Six),
    ] {
        if keys.just_pressed(key) {
            return Some(seating);
        }
    }
    None
}

/// Choose the seating, from the digit keys, `Tab`, or the seat buttons.
///
/// Drive the room-name editor, and rejoin on commit.
///
/// Changing the room means **reopening the socket**: the room is baked into the
/// signaling URL, so editing [`RoomId`] alone would change the label and nothing
/// else — a control that appears to work and does not. Removing
/// `MatchboxSocket` makes `open_socket` run again on the next entry to the
/// lobby, and [`NetState::leave_room`] discards everything the old room's
/// socket told us.
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
                // Bounded here as well as in `parse`, so the field cannot grow
                // without limit and then be refused wholesale.
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

/// Apply the name editor's keypresses. Same classification as the room field —
/// [`edit_action`] — but committing writes the display name and re-greets: the
/// roster is only ever exchanged on Hello, so a rename without a re-greet would
/// leave every peer looking at the old name.
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

/// Focus an input box: by clicking it, or with its key (`R` room, `N` name).
/// Focusing one field unfocuses the other; each is seeded with its current
/// value, so a small change does not mean retyping the whole thing.
///
/// Runs only while no field holds the keyboard — the modal rule. Leaving a
/// field (Enter/Esc) is what frees the keys again.
fn focus_input_fields(
    buttons: Query<(&Interaction, &TextInput), Changed<Interaction>>,
    keys: Res<ButtonInput<KeyCode>>,
    room: Res<RoomId>,
    mut net: ResMut<NetState>,
    mut room_edit: ResMut<RoomEdit>,
    mut name_edit: ResMut<NameEdit>,
) {
    let mut clicked: Option<FieldKind> = None;
    for (interaction, input) in buttons.iter() {
        if *interaction == Interaction::Pressed {
            clicked = Some(input.0);
        }
    }

    let focus_room = clicked == Some(FieldKind::Room) || keys.just_pressed(KeyCode::KeyR);
    let focus_name = clicked == Some(FieldKind::Name) || keys.just_pressed(KeyCode::KeyN);
    if !focus_room && !focus_name {
        return;
    }

    room_edit.active = focus_room;
    name_edit.active = focus_name && !focus_room;
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
    // The key (or click) that opened a field belongs to it, not to the
    // systems chained after this one.
    room_edit.consumed_input = true;
    name_edit.consumed_input = true;
    net.status = "Editing. Enter accepts, Esc cancels.".into();
}

/// A system of its own rather than part of the button handling, because it needs
/// nothing from the socket. That makes it directly drivable in a headless test —
/// which matters, since the wiring from keypress to dealt board is exactly what
/// the pure-function tests could not cover.
pub fn choose_seating(
    buttons: Query<(&Interaction, &LobbyButton), Changed<Interaction>>,
    keys: Res<ButtonInput<KeyCode>>,
    socket: Option<ResMut<MatchboxSocket>>,
    mut net: ResMut<NetState>,
    mut chosen: ResMut<ChosenSeating>,
) {
    let mut seats = seating_from_keys(&keys, chosen.0);
    for (interaction, button) in buttons.iter() {
        if *interaction == Interaction::Pressed
            && let LobbyButton::Seats(s) = button
        {
            seats = Some(*s);
        }
    }

    let Some(seating) = seats else {
        return;
    };

    // Only the host decides the seating, since it is a property of the shared
    // game rather than of one peer's view. A guest changing it locally would
    // start a board that disagrees with everyone else's.
    if net.peers.is_empty() || net.sequences() {
        chosen.0 = seating;
        net.status = format!("Seating: {}", seating.label());
        // Broadcast live, so every peer watches the table change as it is
        // changed instead of discovering it at `Start`.
        if let Some(mut socket) = socket {
            broadcast(&mut socket, &net.peers, &NetMsg::Seating(seating.indices()));
        }
    } else {
        net.status = "Only the host chooses the number of players.".into();
    }
}

fn handle_buttons(
    buttons: Query<(&Interaction, &LobbyButton), Changed<Interaction>>,
    keys: Res<ButtonInput<KeyCode>>,
    socket: Option<ResMut<MatchboxSocket>>,
    mut net: ResMut<NetState>,
    // Read-only: `choose_seating` owns the writing. Needed here so the host's
    // `Start` can tell every peer which board to deal.
    chosen: Res<ChosenSeating>,
    mut next_state: ResMut<NextState<AppState>>,
) {
    let mut ready = keys.just_pressed(KeyCode::Space);
    let mut start = keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::NumpadEnter);
    let mut spectate = keys.just_pressed(KeyCode::KeyP);
    let mut back = keys.just_pressed(KeyCode::Escape);

    // `S` starts a solo game unconditionally, without consulting the network at
    // all. Enter's behaviour depends on peers and readiness, and every way that
    // can refuse looks identical to a broken build, because the board only
    // exists once the lobby is left. One key that always works is the difference
    // between a bad lobby and an unusable game.
    let mut solo = keys.just_pressed(KeyCode::KeyS);
    for (interaction, button) in buttons.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match button {
            LobbyButton::Ready => ready = true,
            LobbyButton::Start => start = true,
            LobbyButton::Solo => solo = true,
            LobbyButton::Spectate => spectate = true,
            LobbyButton::Back => back = true,
            // Handled by their own systems.
            LobbyButton::Seats(_) => {}
        }
    }

    if back {
        // The socket stays: the menu's Lobby button comes straight back, and
        // re-entering the lobby reopens it only if it was dropped.
        next_state.set(AppState::Menu);
        return;
    }

    // Checked before the socket is even looked at, so nothing about the network
    // can prevent it. `start` also plays solo when nobody else is present.
    if solo || (start && matches!(start_decision(&net, chosen.0), StartDecision::Solo)) {
        // Solo means this peer drives every seated camp, so no seat may keep a
        // player binding — a self-seat left over from lobby greetings would
        // otherwise pin the solo player to camp 0 and strand the rest.
        net.unbind_players();
        info!("starting a solo game");
        next_state.set(AppState::InGame);
        return;
    }

    let Some(mut socket) = socket else {
        if start {
            // No socket and no peers means `start_decision` already returned
            // Solo above, so reaching here needs an explanation rather than
            // silence.
            net.status = "Still connecting... press S to play solo now.".into();
        }
        return;
    };
    let peers = net.peers.clone();
    let me = net.my_seat().map(|s| s.peer.clone()).unwrap_or_default();

    if spectate {
        // Spectator status is my own declaration; the host relays it through
        // the roster. A declared spectator stops counting for readiness.
        let now = !net.my_seat().is_some_and(|s| s.spectate);
        if let Some(seat) = net.seats.iter_mut().find(|s| s.peer == me) {
            seat.spectate = now;
            if now {
                seat.ready = true;
            }
        }
        if net.sequences() {
            publish_roster(&mut socket, &net, &peers);
        } else {
            broadcast(&mut socket, &peers, &NetMsg::Spectate(now));
        }
        net.status = if now {
            "You are a spectator.".into()
        } else {
            "You are a player again; waiting for the next game.".into()
        };
    }

    if ready {
        // Spectators have nothing to ready up: nobody waits for them, so the
        // toggle would only feign progress.
        if net.my_seat().is_some_and(|s| s.spectate) {
            net.status = "Spectators do not ready up.".into();
        } else {
            let now = match net.seats.iter().find(|s| s.peer == me) {
                Some(seat) => !seat.ready,
                None => true,
            };
            if net.sequences() {
                if let Some(seat) = net.seats.iter_mut().find(|s| s.peer == me) {
                    seat.ready = now;
                }
                publish_roster(&mut socket, &net, &peers);
            } else if net.seats.iter().any(|s| s.peer == me) {
                broadcast(&mut socket, &peers, &NetMsg::Ready(now));
            }
        }
    }

    // Every refusal states its reason: a button that silently does nothing is
    // indistinguishable from a broken build, which is how the blank-screen bug
    // presented.
    if start {
        match start_decision(&net, chosen.0) {
            StartDecision::Solo => next_state.set(AppState::InGame),
            StartDecision::Multiplayer => {
                assign_seating(&mut net, chosen.0);
                broadcast(&mut socket, &peers, &start_message(&net, chosen.0));
                next_state.set(AppState::InGame);
            }
            StartDecision::Refuse(why) => net.status = why,
        }
    }
}

fn draw_roster(
    net: Res<NetState>,
    chosen: Res<ChosenSeating>,
    mut text: Query<&mut Text, With<RosterText>>,
) {
    if !net.is_changed() && !chosen.is_changed() {
        return;
    }
    let Ok(mut text) = text.single_mut() else {
        return;
    };

    let mut out = format!(
        "{}  |  {} peer(s) here  |  seating {}\n\n",
        if net.sequences() { "host" } else { "guest" },
        net.peers.len(),
        chosen.0.label(),
    );
    if net.seats.is_empty() {
        out.push_str("No one here yet - share the room name.\n");
    }
    let me = net.my_seat().map(|s| s.peer.clone()).unwrap_or_default();
    for seat in &net.seats {
        let role = if seat.spectate {
            "spectator".into()
        } else {
            format!(
                "player {}",
                seat.player.map_or("-".into(), |p| p.to_string())
            )
        };
        out.push_str(&format!(
            "  {} {}  {}  {}\n",
            if seat.peer == me { ">" } else { " " },
            seat.name,
            role,
            // A declared spectator is never waited for, so the list says so
            // instead of an endless "...".
            if seat.spectate {
                ""
            } else if seat.ready {
                "ready"
            } else {
                "..."
            },
        ));
    }

    // Enter always does something, so say so plainly. The earlier wording
    // implied a solo player had to wait for peers that were never coming.
    //
    // The count comes from the chosen seating rather than being written out:
    // hard-coding "all six players" left the hint contradicting the buttons
    // directly above it whenever a shorter game was picked.
    out.push_str(&format!(
        "\nEnter starts a shared game; Space toggles ready; P spectates;\n\
         S plays all {} camps on this device.",
        chosen.0.count()
    ));
    if !net.status.is_empty() {
        out.push_str(&format!("\n\n{}", net.status));
    }
    **text = out;
}

/// Draw the name input: the buffer with a caret while focused, the current
/// name otherwise. The caret matters — without it a focused field looks
/// identical to a dead one, and the player cannot tell why `S` stopped
/// working.
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

/// Build the game for the chosen seating and seat the local player.
///
/// The session is *rebuilt* here rather than mutated, because the seating
/// determines the starting position and there is no meaningful way to reseat a
/// board that has already been dealt. Runs on entering the game, before
/// `spawn_board`.
pub fn apply_seats(
    net: Res<NetState>,
    chosen: Res<ChosenSeating>,
    ai_seats: Res<crate::menu::AiSeats>,
    mut session: ResMut<Session>,
) {
    *session = Session::new(chosen.0);
    session.ai_players = ai_seats.0.clone();
    session.local_player = net.my_player();
    // A declared spectator watches: local_player is already None for one, and
    // this flag stops hotseat-style "move everyone" from applying to them. The
    // same applies when the computer owns every seat — the humans are an
    // audience, so board input is blocked while the engines race.
    session.spectating = net.my_seat().is_some_and(|s| s.spectate)
        || (!ai_seats.0.is_empty() && ai_seats.0.len() >= chosen.0.count());
    session.message = if session.spectating {
        "Spectating - watch, but do not touch.".into()
    } else {
        match session.local_player {
            Some(p) => format!("You are player {} of {}", p.index(), chosen.0.count()),
            None => format!("Playing all {} players locally", chosen.0.count()),
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use checkers_core::position::Player;
    use checkers_net::Seat;

    fn seat(name: &str, ready: bool) -> Seat {
        Seat {
            peer: name.into(),
            name: name.into(),
            player: Some(0),
            ready,
            spectate: false,
        }
    }

    fn spectator(name: &str) -> Seat {
        Seat {
            spectate: true,
            ready: true,
            ..seat(name, true)
        }
    }

    /// The bug this function was extracted for: with nobody else present, Enter
    /// must start the game. It used to require a non-empty, all-ready roster,
    /// and a solo player's seat is only created once the socket connects — and
    /// starts out not ready. The board only exists in `InGame`, so the player
    /// was left staring at a blank screen with Enter doing nothing.
    #[test]
    fn a_lone_player_can_always_start() {
        let net = NetState::default();
        assert!(net.peers.is_empty(), "the fixture has no peers");
        assert!(net.seats.is_empty(), "and no seat has been created yet");
        assert_eq!(start_decision(&net, Seating::Two), StartDecision::Solo);
    }

    /// Specifically the socket-never-connected case: no id, no seat, no peers.
    /// An unreachable signaling server must not be able to strand the player.
    #[test]
    fn an_unreachable_signaling_server_does_not_block_solo_play() {
        // Default *is* the never-connected state, which is the point: no id, no
        // seat, no peers is what a failed connection leaves behind.
        let net = NetState::default();
        assert_eq!(net.my_id, None, "no id without a connection");
        assert!(net.seats.is_empty(), "and no seat");
        assert_eq!(start_decision(&net, Seating::Two), StartDecision::Solo);
    }

    #[test]
    fn a_guest_is_told_only_the_host_can_start() {
        let mut net = NetState::default();
        // A peer is present and we are not the host.
        net.peers.push(fake_peer());
        net.is_host = false;
        match start_decision(&net, Seating::Two) {
            StartDecision::Refuse(why) => assert!(why.contains("host"), "{why}"),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn the_host_is_told_who_it_is_waiting_for() {
        let mut net = NetState::default();
        net.peers.push(fake_peer());
        net.is_host = true;
        net.seats = vec![seat("ada", true), seat("grace", false)];

        match start_decision(&net, Seating::Two) {
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

    /// No refusal may be a dead end. The board only exists after the lobby, so a
    /// refusal that does not tell the player how to proceed is indistinguishable
    /// from the game being broken — which is exactly how the shared-room
    /// deadlock presented.
    #[test]
    fn every_refusal_names_the_escape_hatch() {
        let mut guest = NetState::default();
        guest.peers.push(fake_peer());

        let mut waiting = NetState::default();
        waiting.peers.push(fake_peer());
        waiting.is_host = true;
        waiting.seats = vec![seat("ada", false)];

        for net in [&guest, &waiting] {
            match start_decision(net, Seating::Two) {
                StartDecision::Refuse(why) => assert!(
                    why.contains("S to play solo"),
                    "a refusal must offer the solo key: {why:?}"
                ),
                other => panic!("expected a refusal, got {other:?}"),
            }
        }
    }

    #[test]
    fn the_host_starts_once_everyone_is_ready() {
        let mut net = NetState::default();
        net.peers.push(fake_peer());
        net.is_host = true;
        net.seats = vec![seat("ada", true), seat("grace", true)];
        assert_eq!(
            start_decision(&net, Seating::Two),
            StartDecision::Multiplayer
        );
    }

    /// Two-player mode hung forever on "Waiting for player 3": the roster
    /// hands out join-order slots (0, 1), but the Two seating plays camps
    /// {0, 3}, so when the turn reached camp 3 nobody controlled it. The
    /// binding from join order to camps happens at start.
    #[test]
    fn join_order_binds_to_the_seatings_camps() {
        let mut net = NetState {
            seats: vec![seat("ada", true), seat("grace", true), seat("lee", true)],
            ..Default::default()
        };

        assign_seating(&mut net, Seating::Two);

        assert_eq!(net.seats[0].player, Some(0), "the host keeps camp 0");
        assert_eq!(
            net.seats[1].player,
            Some(3),
            "the second joiner commands camp 3, the facing camp"
        );
        assert_eq!(
            net.seats[2].player, None,
            "a peer beyond the seating's camps sits out as a spectator"
        );
    }

    /// Starting a seating with fewer peers than camps would strand the
    /// uncontrolled ones — the turn would wait on a player with no owner.
    #[test]
    fn a_shared_start_needs_a_seat_per_camp() {
        let mut net = NetState::default();
        net.peers.push(fake_peer());
        net.is_host = true;
        net.seats = vec![seat("ada", true), seat("grace", true)];

        assert!(
            matches!(start_decision(&net, Seating::Six), StartDecision::Refuse(_)),
            "two seats cannot play a six-camp game"
        );
        assert_eq!(
            start_decision(&net, Seating::Two),
            StartDecision::Multiplayer,
            "and the same two seats are exactly right for two camps"
        );
    }

    /// A spectator has no say in when the game starts, so an unready spectator
    /// must not appear in the waiting list — and must not block the start.
    #[test]
    fn spectators_are_never_waited_for() {
        let mut net = NetState::default();
        net.peers.push(fake_peer());
        net.is_host = true;
        net.seats = vec![seat("ada", true), seat("grace", true), spectator("lee")];
        assert!(matches!(spectator("lee"), Seat { spectate: true, .. }));

        assert_eq!(
            start_decision(&net, Seating::Two),
            StartDecision::Multiplayer,
            "a spectator neither readies nor blocks"
        );
    }

    /// Camps bind to seated players in join order; a spectator takes none and
    /// keeps none.
    #[test]
    fn spectators_take_no_camp() {
        let mut net = NetState {
            seats: vec![seat("ada", true), spectator("lee"), seat("grace", true)],
            ..Default::default()
        };

        assign_seating(&mut net, Seating::Two);

        assert_eq!(net.seats[0].player, Some(0), "first seated player: camp 0");
        assert_eq!(net.seats[1].player, None, "the spectator commands nothing");
        assert_eq!(
            net.seats[2].player,
            Some(3),
            "the second seated player still gets the facing camp"
        );
    }

    /// Spectators are extra bodies, not extra players: they count for neither
    /// side of the camps-needed check.
    #[test]
    fn spectators_do_not_count_toward_the_camps() {
        let mut net = NetState {
            seats: vec![seat("ada", true), spectator("lee")],
            ..Default::default()
        };
        net.peers.push(fake_peer());
        net.is_host = true;

        assert_eq!(
            start_decision(&net, Seating::Two),
            StartDecision::Refuse(
                "1 joined but this seating needs 2. Press Tab or 2/3/6 to change it, \
                 or S to play solo."
                    .into()
            ),
            "one spectator plus nobody is still not a two-camp game"
        );

        net.seats.push(seat("grace", true));
        assert_eq!(
            start_decision(&net, Seating::Two),
            StartDecision::Multiplayer,
            "two seated players plus a spectator is fine"
        );
    }

    /// A stand-in peer. `PeerId` is a `Uuid` newtype with no `FromStr`, so this
    /// goes through the tuple field.
    fn fake_peer() -> PeerId {
        PeerId(uuid::Uuid::from_u128(1))
    }

    fn keys(pressed: &[KeyCode]) -> ButtonInput<KeyCode> {
        let mut input = ButtonInput::default();
        for key in pressed {
            input.press(*key);
        }
        input
    }

    /// The host must broadcast the seating it *chose*, not a default.
    ///
    /// This is the test that was missing. Replacing `seating.indices()` with
    /// `Seating::default().indices()` in `start_message` — the original bug,
    /// where a guest dealt six players against a three-player host — passed the
    /// entire suite before this existed, because every other test constructed
    /// its own `NetMsg::Start`.
    #[test]
    fn the_start_message_carries_the_chosen_seating() {
        let net = NetState::default();
        for seating in Seating::ALL {
            let NetMsg::Start { players, .. } = start_message(&net, seating) else {
                panic!("start_message must build a Start");
            };
            assert_eq!(
                players,
                seating.indices(),
                "{seating:?} must be the seating broadcast"
            );
            assert_eq!(
                Seating::from_indices(&players),
                Some(seating),
                "{seating:?} must be recoverable by the guest"
            );
        }
    }

    /// The guest must adopt whatever the host sent, for every offered seating.
    #[test]
    fn a_guest_adopts_the_hosts_seating() {
        for host in Seating::ALL {
            // Start from a deliberately different local choice, so adopting is
            // observable rather than coincidental.
            let local = host.next();
            assert_ne!(local, host, "the fixture needs two distinct seatings");
            assert_eq!(adopt_seating(&host.indices(), local), Ok(host));
        }
    }

    /// An unknown seating must keep the local choice *and* say so. Starting a
    /// different game without a word is the failure this guards.
    #[test]
    fn an_unknown_seating_is_reported_not_silently_applied() {
        let local = Seating::Three;
        let complaint = adopt_seating(&[0, 1, 2, 3], local)
            .expect_err("four players is not an offered seating");
        assert!(
            complaint.contains("does not know"),
            "must say the seating is unknown: {complaint}"
        );
        assert!(
            complaint.contains(local.label()),
            "must name what it will use instead: {complaint}"
        );
    }

    /// A seating other than the default must actually differ from it, or the
    /// test above would pass against a hard-coded default by coincidence.
    #[test]
    fn the_broadcast_seating_is_not_always_the_default() {
        let net = NetState::default();
        let non_default = Seating::ALL
            .into_iter()
            .find(|s| *s != Seating::default())
            .expect("more than one seating is offered");

        let NetMsg::Start { players, .. } = start_message(&net, non_default) else {
            panic!("start_message must build a Start");
        };
        assert_ne!(
            players,
            Seating::default().indices(),
            "a non-default choice must not be sent as the default"
        );
    }

    /// Typing must reach the buffer, using the layout-aware `text` field.
    #[test]
    fn ordinary_characters_are_inserted() {
        for (key, text, want) in [
            (KeyCode::KeyA, Some("a"), 'a'),
            (KeyCode::KeyZ, Some("Z"), 'Z'),
            (KeyCode::Digit4, Some("4"), '4'),
            (KeyCode::Minus, Some("-"), '-'),
            // Whatever the layout produced, not what the key code suggests: on a
            // German keyboard the key labelled Y reports "z".
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

    /// Enter, Escape and Backspace must act even when the platform also reports
    /// text for them, or committing would instead type a control character into
    /// the name.
    #[test]
    fn the_editing_keys_win_over_any_text_they_report() {
        assert_eq!(edit_action(KeyCode::Enter, Some("\r")), EditAction::Commit);
        assert_eq!(
            edit_action(KeyCode::Backspace, Some("\u{8}")),
            EditAction::Backspace
        );
        assert_eq!(
            edit_action(KeyCode::Escape, Some("\u{1b}")),
            EditAction::Cancel
        );
    }

    /// A keypress with no usable text must do nothing rather than insert junk.
    #[test]
    fn keys_without_usable_text_are_ignored() {
        // Modifiers and function keys report no text.
        assert_eq!(edit_action(KeyCode::ShiftLeft, None), EditAction::Ignore);
        assert_eq!(edit_action(KeyCode::F5, None), EditAction::Ignore);
        // A control character reported as text is not a room-name character.
        assert_eq!(edit_action(KeyCode::Tab, Some("\t")), EditAction::Ignore);
        // An uncombined dead key can report two characters; refuse rather than
        // guess which was meant.
        assert_eq!(edit_action(KeyCode::KeyA, Some("^a")), EditAction::Ignore);
        assert_eq!(edit_action(KeyCode::KeyA, Some("")), EditAction::Ignore);
    }

    #[test]
    fn the_digits_select_their_seating() {
        for (key, want) in [
            (KeyCode::Digit2, Seating::Two),
            (KeyCode::Digit3, Seating::Three),
            (KeyCode::Digit6, Seating::Six),
        ] {
            assert_eq!(
                seating_from_keys(&keys(&[key]), Seating::Six),
                Some(want),
                "{key:?} must select {want:?}"
            );
        }
    }

    /// Tab must reach every option, so the control is usable without knowing
    /// which digits are valid — 4 and 5 are not.
    #[test]
    fn tab_cycles_through_every_seating() {
        let mut current = Seating::default();
        let mut seen = vec![current];
        for _ in 0..Seating::ALL.len() {
            current = seating_from_keys(&keys(&[KeyCode::Tab]), current)
                .expect("Tab always selects something");
            if !seen.contains(&current) {
                seen.push(current);
            }
        }
        assert_eq!(
            seen.len(),
            Seating::ALL.len(),
            "Tab must reach every seating, saw {seen:?}"
        );
    }

    #[test]
    fn an_unrelated_key_selects_nothing() {
        assert_eq!(
            seating_from_keys(&keys(&[KeyCode::KeyX]), Seating::Six),
            None
        );
        // 4 and 5 are not offered, and must not be silently accepted.
        assert_eq!(
            seating_from_keys(&keys(&[KeyCode::Digit4]), Seating::Six),
            None
        );
        assert_eq!(
            seating_from_keys(&keys(&[KeyCode::Digit5]), Seating::Six),
            None
        );
    }

    /// The seating must survive into the game. `apply_seats` rebuilds the
    /// session from it, and a session built for two players must not be holding
    /// a six-player board.
    #[test]
    fn the_chosen_seating_builds_the_session() {
        for seating in Seating::ALL {
            let session = Session::new(seating);
            assert_eq!(session.seating, seating);
            assert_eq!(
                session.game.position().holes().len(),
                121,
                "the board is always the full star"
            );
            for player in Player::ALL {
                let n = session.game.position().pieces_of(player).len();
                let expected = if seating.players().contains(&player) {
                    10
                } else {
                    0
                };
                assert_eq!(n, expected, "{seating:?}: player {}", player.index());
            }
        }
    }
}
