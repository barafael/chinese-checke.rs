//! Lobby screen: join a room, see who is here, get a seat, start.
//!
//! Built with plain Bevy UI rather than egui, to match the in-game buttons and
//! to avoid a dependency for four widgets.
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

use bevy::prelude::*;
use bevy_matchbox::prelude::*;
use checkers_net::{CH_RELIABLE, NetMsg, NetState, RoomId, Seat, broadcast, decode, send_to};

use crate::{AppState, Session};

/// Marker for everything spawned by the lobby, so leaving despawns it wholesale.
#[derive(Component)]
pub struct LobbyUi;

#[derive(Component)]
enum LobbyButton {
    Ready,
    Start,
}

#[derive(Component)]
struct RosterText;

pub fn plugin(app: &mut App) {
    app.init_resource::<NetState>()
        .init_resource::<RoomId>()
        .add_systems(OnEnter(AppState::Lobby), (checkers_net::open_socket, spawn))
        .add_systems(OnExit(AppState::Lobby), despawn)
        .add_systems(
            Update,
            (elect_host, pump_socket, handle_buttons, draw_roster)
                .chain()
                .run_if(in_state(AppState::Lobby)),
        );
}

fn spawn(mut commands: Commands, room: Res<RoomId>) {
    commands.spawn((
        Text::new(format!("Room \"{}\"\nConnecting…", room.0)),
        TextFont {
            font_size: FontSize::Px(17.0),
            ..default()
        },
        TextColor(Color::srgb(0.88, 0.88, 0.9)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(40.0),
            left: Val::Px(40.0),
            ..default()
        },
        RosterText,
        LobbyUi,
    ));

    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(40.0),
                left: Val::Px(40.0),
                column_gap: Val::Px(10.0),
                ..default()
            },
            LobbyUi,
        ))
        .with_children(|row| {
            for (label, tag) in [
                ("Ready (Space)", LobbyButton::Ready),
                ("Start (Enter)", LobbyButton::Start),
            ] {
                row.spawn((
                    Button,
                    Node {
                        padding: UiRect::axes(Val::Px(14.0), Val::Px(8.0)),
                        border_radius: BorderRadius::all(Val::Px(5.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.22, 0.22, 0.27)),
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
        });
}

fn despawn(mut commands: Commands, ui: Query<Entity, With<LobbyUi>>) {
    for e in ui.iter() {
        commands.entity(e).despawn();
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
    mut next_state: ResMut<NextState<AppState>>,
) {
    let Some(mut socket) = socket else {
        return;
    };

    // Announce ourselves to peers we have not greeted. Cheap enough to redo on
    // roster change; the host ignores a repeat Hello from a known peer.
    let peers = net.peers.clone();
    let me = net.my_id.map(|id| id.to_string()).unwrap_or_default();
    if net.name.is_empty() && !me.is_empty() {
        net.name = format!("player-{}", &me[..me.len().min(4)]);
        let hello = NetMsg::Hello {
            name: net.name.clone(),
        };
        broadcast(&mut socket, &peers, &hello);
        // The host's own seat is not created by a Hello it never receives.
        if net.sequences() {
            seat_for(&mut net, &me, &me.clone());
        }
    }

    let inbox: Vec<(PeerId, Box<[u8]>)> = socket.channel_mut(CH_RELIABLE).receive();
    for (from, raw) in inbox {
        let Some(msg) = decode(&raw) else {
            continue;
        };
        match msg {
            NetMsg::Hello { name } => {
                if net.sequences() {
                    seat_for(&mut net, &from.to_string(), &name);
                    let roster = NetMsg::Roster(net.seats.clone());
                    broadcast(&mut socket, &peers, &roster);
                }
            }
            NetMsg::Ready(ready) => {
                if net.sequences() {
                    let key = from.to_string();
                    if let Some(seat) = net.seats.iter_mut().find(|s| s.peer == key) {
                        seat.ready = ready;
                    }
                    let roster = NetMsg::Roster(net.seats.clone());
                    broadcast(&mut socket, &peers, &roster);
                }
            }
            // Guests take the host's roster verbatim; it is the only authority.
            NetMsg::Roster(seats) => net.seats = seats,
            NetMsg::Start { seats } => {
                net.seats = seats;
                next_state.set(AppState::InGame);
            }
            // Moves cannot arrive before the game starts, but a late duplicate
            // from a previous game in the same room could. Ignore rather than
            // mis-apply.
            NetMsg::Move(_) | NetMsg::Sequenced { .. } => {}
        }
    }

    // Greet any peer that joined after us.
    if !net.name.is_empty() {
        for peer in &peers {
            let hello = NetMsg::Hello {
                name: net.name.clone(),
            };
            send_to(&mut socket, *peer, &hello);
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
    });
}

fn handle_buttons(
    buttons: Query<(&Interaction, &LobbyButton), Changed<Interaction>>,
    keys: Res<ButtonInput<KeyCode>>,
    socket: Option<ResMut<MatchboxSocket>>,
    mut net: ResMut<NetState>,
    mut next_state: ResMut<NextState<AppState>>,
) {
    let mut ready = keys.just_pressed(KeyCode::Space);
    let mut start = keys.just_pressed(KeyCode::Enter);
    for (interaction, button) in buttons.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match button {
            LobbyButton::Ready => ready = true,
            LobbyButton::Start => start = true,
        }
    }

    let Some(mut socket) = socket else {
        return;
    };
    let peers = net.peers.clone();

    if ready {
        let me = net.my_id.map(|id| id.to_string()).unwrap_or_default();
        let now = match net.seats.iter().find(|s| s.peer == me) {
            Some(seat) => !seat.ready,
            None => true,
        };
        if net.sequences() {
            if let Some(seat) = net.seats.iter_mut().find(|s| s.peer == me) {
                seat.ready = now;
            }
            let roster = NetMsg::Roster(net.seats.clone());
            broadcast(&mut socket, &peers, &roster);
        } else {
            broadcast(&mut socket, &peers, &NetMsg::Ready(now));
        }
    }

    // Only the host may start, and only once every seat is ready. Guests
    // pressing Enter do nothing rather than starting a game the host has not
    // agreed to.
    if start && net.sequences() && !net.seats.is_empty() && net.seats.iter().all(|s| s.ready) {
        let msg = NetMsg::Start {
            seats: net.seats.clone(),
        };
        broadcast(&mut socket, &peers, &msg);
        next_state.set(AppState::InGame);
    }
}

fn draw_roster(
    net: Res<NetState>,
    room: Res<RoomId>,
    mut text: Query<&mut Text, With<RosterText>>,
) {
    if !net.is_changed() {
        return;
    }
    let Ok(mut text) = text.single_mut() else {
        return;
    };

    let mut out = format!(
        "Room \"{}\"  |  {}  |  {} peer(s) connected\n\n",
        room.0,
        if net.sequences() { "host" } else { "guest" },
        net.peers.len()
    );
    if net.seats.is_empty() {
        out.push_str("Waiting for peers…\n");
    }
    let me = net.my_id.map(|id| id.to_string()).unwrap_or_default();
    for seat in &net.seats {
        out.push_str(&format!(
            "  {} {}  player {}  {}\n",
            if seat.peer == me { ">" } else { " " },
            seat.name,
            seat.player.map_or("-".into(), |p| p.to_string()),
            if seat.ready { "ready" } else { "…" },
        ));
    }
    out.push_str("\nSpace toggles ready. The host starts once everyone is ready.");
    **text = out;
}

/// Seat the local player once the game begins, so the in-game systems know
/// which of the six players this peer is allowed to move.
pub fn apply_seats(net: Res<NetState>, mut session: ResMut<Session>) {
    session.local_player = net.my_player();
    session.message = match session.local_player {
        Some(p) => format!("You are player {}", p.index()),
        None => "Spectating".into(),
    };
}
