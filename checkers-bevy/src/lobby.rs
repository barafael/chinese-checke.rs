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
}

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
    app.init_resource::<NetState>()
        .init_resource::<RoomId>()
        .init_resource::<ChosenSeating>()
        .add_systems(OnEnter(AppState::Lobby), (checkers_net::open_socket, spawn))
        .add_systems(OnExit(AppState::Lobby), despawn)
        .add_systems(
            Update,
            (
                elect_host,
                pump_socket,
                choose_seating,
                handle_buttons,
                sync_seat_buttons,
                draw_roster,
            )
                .chain()
                .run_if(in_state(AppState::Lobby)),
        );
}

/// Highlight the chosen seating.
///
/// Runs every frame but early-outs unless the choice changed, so clicking a
/// seat button repaints once rather than continuously.
fn sync_seat_buttons(
    chosen: Res<ChosenSeating>,
    mut buttons: Query<(&LobbyButton, &mut BackgroundColor)>,
) {
    if !chosen.is_changed() {
        return;
    }
    for (button, mut bg) in buttons.iter_mut() {
        if let LobbyButton::Seats(seating) = button {
            bg.0 = if *seating == chosen.0 { CHOSEN } else { IDLE };
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

/// An unselected button, and the selected seating. Named because
/// [`sync_seat_buttons`] has to reset to exactly the spawn colour.
const IDLE: Color = Color::srgb(0.22, 0.22, 0.27);
const CHOSEN: Color = Color::srgb(0.20, 0.45, 0.28);

/// The lobby is one **flex column filling the window**, not a set of
/// absolutely-positioned corners.
///
/// The previous layout anchored the buttons at `bottom: 40px`, which put them
/// off-screen entirely on a display whose work area is shorter than the window:
/// they were laid out at y=2307 of a 2450px surface, behind the taskbar. A
/// window-filling column with the content at the top cannot place anything
/// outside the window, whatever the window's size — so the failure mode is gone
/// by construction rather than by choosing a better offset.
fn spawn(mut commands: Commands, room: Res<RoomId>, chosen: Res<ChosenSeating>) {
    commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::FlexStart,
                padding: UiRect::all(Val::Px(40.0)),
                row_gap: Val::Px(18.0),
                ..default()
            },
            LobbyUi,
        ))
        .with_children(|col| {
            col.spawn((
                Text::new(format!("Room \"{}\"\nConnecting...", room.0)),
                TextFont {
                    font_size: FontSize::Px(17.0),
                    ..default()
                },
                TextColor(Color::srgb(0.88, 0.88, 0.9)),
                RosterText,
            ));

            // Seating, with the keys that also set it.
            col.spawn((
                Text::new("Players  (2 / 3 / 6, or Tab to cycle)"),
                TextFont {
                    font_size: FontSize::Px(14.0),
                    ..default()
                },
                TextColor(Color::srgb(0.62, 0.62, 0.68)),
            ));
            col.spawn(Node {
                column_gap: Val::Px(10.0),
                ..default()
            })
            .with_children(|row| {
                for seating in Seating::ALL {
                    button(row, seating.label(), LobbyButton::Seats(seating));
                }
            });

            col.spawn(Node {
                column_gap: Val::Px(10.0),
                margin: UiRect::top(Val::Px(6.0)),
                ..default()
            })
            .with_children(|row| {
                button(row, "Play solo (S)", LobbyButton::Solo);
                button(row, "Ready (Space)", LobbyButton::Ready);
                button(row, "Start shared (Enter)", LobbyButton::Start);
            });
        });

    // Paint the initial choice, so the highlight is right on the first frame
    // rather than only after the first interaction.
    commands.insert_resource(*chosen);
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

pub fn start_decision(net: &NetState) -> StartDecision {
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
        .filter(|s| !s.ready)
        .map(|s| s.name.as_str())
        .collect();
    if waiting.is_empty() {
        StartDecision::Multiplayer
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
                    seat_for(&mut net, &from.to_string(), &name);
                    publish_roster(&mut socket, &net, &peers);
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
            // Guests take the host's roster verbatim; it is the only authority.
            NetMsg::Roster(seats) => net.seats = seats,
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
/// A system of its own rather than part of the button handling, because it needs
/// nothing from the socket. That makes it directly drivable in a headless test —
/// which matters, since the wiring from keypress to dealt board is exactly what
/// the pure-function tests could not cover.
pub fn choose_seating(
    buttons: Query<(&Interaction, &LobbyButton), Changed<Interaction>>,
    keys: Res<ButtonInput<KeyCode>>,
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
            // Handled by `choose_seating`.
            LobbyButton::Seats(_) => {}
        }
    }

    // Checked before the socket is even looked at, so nothing about the network
    // can prevent it. `start` also plays solo when nobody else is present.
    if solo || (start && matches!(start_decision(&net), StartDecision::Solo)) {
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
            publish_roster(&mut socket, &net, &peers);
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
                broadcast(&mut socket, &peers, &start_message(&net, chosen.0));
                next_state.set(AppState::InGame);
            }
            StartDecision::Refuse(why) => net.status = why,
        }
    }
}

fn draw_roster(
    net: Res<NetState>,
    room: Res<RoomId>,
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
        "Room \"{}\"  |  {}  |  {} peer(s) connected  |  {}\n\n",
        room.0,
        if net.sequences() { "host" } else { "guest" },
        net.peers.len(),
        chosen.0.label(),
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
            if seat.ready { "ready" } else { "..." },
        ));
    }

    // Enter always does something, so say so plainly. The earlier wording
    // implied a solo player had to wait for peers that were never coming.
    //
    // The count comes from the chosen seating rather than being written out:
    // hard-coding "all six players" left the hint contradicting the buttons
    // directly above it whenever a shorter game was picked.
    out.push_str(&format!(
        "\nPress S to play solo now - all {} players on one board.\n\
         Enter starts a shared game; Space toggles ready when others are here.",
        chosen.0.count()
    ));
    if !net.status.is_empty() {
        out.push_str(&format!("\n\n{}", net.status));
    }
    **text = out;
}

/// Build the game for the chosen seating and seat the local player.
///
/// The session is *rebuilt* here rather than mutated, because the seating
/// determines the starting position and there is no meaningful way to reseat a
/// board that has already been dealt. Runs on entering the game, before
/// `spawn_board`.
pub fn apply_seats(net: Res<NetState>, chosen: Res<ChosenSeating>, mut session: ResMut<Session>) {
    *session = Session::new(chosen.0);
    session.local_player = net.my_player();
    session.message = match session.local_player {
        Some(p) => format!("You are player {} of {}", p.index(), chosen.0.count()),
        None => format!("Playing all {} players locally", chosen.0.count()),
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
            match start_decision(net) {
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
        assert_eq!(start_decision(&net), StartDecision::Multiplayer);
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
