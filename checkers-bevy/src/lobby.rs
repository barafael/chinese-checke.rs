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

/// What pressing Enter should do.
///
/// Split out from [`handle_buttons`] because it is the part that was wrong: the
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

pub fn start_decision(net: &NetState) -> StartDecision {
    if net.peers.is_empty() {
        return StartDecision::Solo;
    }
    if !net.sequences() {
        return StartDecision::Refuse("Only the host can start the game.".into());
    }
    let waiting: Vec<&str> = net
        .seats
        .iter()
        .filter(|s| !s.ready)
        .map(|s| s.name.as_str())
        .collect();
    if waiting.is_empty() {
        StartDecision::Multiplayer
    } else {
        StartDecision::Refuse(format!("Waiting for: {}", waiting.join(", ")))
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

    // Starting must not depend on the socket. If the signaling server is
    // unreachable — blocked, down, or just slow — a solo player would otherwise
    // be stuck on a blank screen with no way forward and nothing explaining why.
    if start && matches!(start_decision(&net), StartDecision::Solo) {
        info!("starting a solo game");
        next_state.set(AppState::InGame);
        return;
    }

    let Some(mut socket) = socket else {
        if start {
            net.status = "No connection yet — press Enter again to play solo.".into();
        }
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

    // Every refusal states its reason: a button that silently does nothing is
    // indistinguishable from a broken build, which is how the blank-screen bug
    // presented.
    if start {
        match start_decision(&net) {
            StartDecision::Solo => next_state.set(AppState::InGame),
            StartDecision::Multiplayer => {
                let msg = NetMsg::Start {
                    seats: net.seats.clone(),
                };
                broadcast(&mut socket, &peers, &msg);
                next_state.set(AppState::InGame);
            }
            StartDecision::Refuse(why) => net.status = why,
        }
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
        out.push_str("No other players yet.\n");
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

    // Enter always does something, so say so plainly. The earlier wording
    // implied a solo player had to wait for peers that were never coming.
    out.push_str(
        "\nPress Enter to play — solo if nobody else has joined, \
         all six players on one board.\n\
         Space toggles ready when others are here; the host starts the game.",
    );
    if !net.status.is_empty() {
        out.push_str(&format!("\n\n{}", net.status));
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use checkers_net::Seat;

    fn seat(name: &str, ready: bool) -> Seat {
        Seat {
            peer: name.into(),
            name: name.into(),
            player: Some(0),
            ready,
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
        assert_eq!(start_decision(&net), StartDecision::Solo);
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
        assert_eq!(start_decision(&net), StartDecision::Solo);
    }

    #[test]
    fn a_guest_is_told_only_the_host_can_start() {
        let mut net = NetState::default();
        // A peer is present and we are not the host.
        net.peers.push(fake_peer());
        net.is_host = false;
        match start_decision(&net) {
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

        match start_decision(&net) {
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

    #[test]
    fn the_host_starts_once_everyone_is_ready() {
        let mut net = NetState::default();
        net.peers.push(fake_peer());
        net.is_host = true;
        net.seats = vec![seat("ada", true), seat("grace", true)];
        assert_eq!(start_decision(&net), StartDecision::Multiplayer);
    }

    /// A stand-in peer. `PeerId` is a `Uuid` newtype with no `FromStr`, so this
    /// goes through the tuple field.
    fn fake_peer() -> PeerId {
        PeerId(uuid::Uuid::from_u128(1))
    }
}
