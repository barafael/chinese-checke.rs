//! Live multiplayer lobby: two and three real instances on one room.
//!
//! `lobby_flow` and `room_flow` drive the lobby decisions in a single app with
//! no socket. These tests go one step further: **each instance is its own
//! headless Bevy app** running the real lobby systems (`elect_host`,
//! `pump_socket`, `select_corner`, `handle_buttons`, `apply_seats`), each with
//! its own real [`MatchboxSocket`]. The apps are introduced by an in-process
//! full-mesh signaling server (the same crate the fork ships for native
//! development) and then talk peer-to-peer over an actual WebRTC data channel.
//!
//! So the whole shared-room contract crosses a real wire here: host election,
//! greetings, corner claims, engine seats, the roster broadcasts, readiness,
//! and the host's `Start` — and the guest's lobby reflects what the peer
//! actually configured, because there is no other copy of the truth to read.
//!
//! There is no window and no renderer: `MinimalPlugins`, plus the input and
//! state plugins the lobby reads. An instance is driven exactly like the web
//! build is operated — digit keys select a corner, the corner buttons human /
//! computer / off, Space readies, F flips the foreign-camp rule, Enter starts —
//! so the code paths under test are the app's own, not a reimplementation.
//!
//! These are the slow siblings of the unit tests — each scenario waits for a
//! real peer handshake to complete — so they are `#[ignore]`d and opt in:
//!
//! ```sh
//! cargo test -p checkers-bevy --test multiplayer -- --ignored --nocapture
//! ```

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use bevy::input::ButtonState;
use bevy::input::InputPlugin;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use bevy_matchbox::prelude::*;
use checkers_bevy::lobby::{ChosenVariants, CornerCommand, LobbyButton, SelectedCorner, Table};
use checkers_bevy::{AppState, Session};
use checkers_core::position::Player;
use checkers_net::{NetState, RoomId};

/// A local full-mesh signaling server, on a free loopback port. Runs on its
/// own tokio runtime in a background thread, exactly like `matchbox_server`;
/// the port is discovered by probing before the thread takes it over.
fn start_signaling_server() -> u16 {
    let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("a free loopback port");
    let port = probe.local_addr().expect("probe address").port();
    drop(probe);

    std::thread::Builder::new()
        .name("signaling-server".into())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("a tokio runtime for the signaling server");
            let server =
                matchbox_signaling::SignalingServer::full_mesh_builder(std::net::SocketAddr::new(
                    std::net::IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1)),
                    port,
                ))
                .build();
            runtime
                .block_on(server.serve())
                .expect("the signaling server ran");
        })
        .expect("spawn the signaling server thread");

    port
}

static ROOM_SEQ: AtomicU64 = AtomicU64::new(0);

/// A room unique to this run: one per scenario, so a leftover peer from a
/// previous scenario can never leak into the next.
fn fresh_room(label: &str) -> RoomId {
    let n = ROOM_SEQ.fetch_add(1, Ordering::Relaxed);
    let name = format!("mp-{label}-{n}-{:x}", std::process::id());
    RoomId::parse(&name).expect("a generated room name is valid")
}

/// One headless instance: a real app, a real socket, the lobby's own systems.
///
/// The socket is opened with exactly the builder [`checkers_net::open_socket`]
/// uses, only pointed at the in-process server. The state machine is the app's:
/// lobby systems run while in the lobby, and entering the game runs
/// [`checkers_bevy::lobby::apply_seats`] on the `OnEnter` transition.
fn instance(name: &str, room: &RoomId, port: u16) -> App {
    let mut app = App::new();
    let url = format!("ws://127.0.0.1:{port}/{}", room.0);
    app.add_plugins((MinimalPlugins, InputPlugin, StatesPlugin))
        .init_state::<AppState>()
        .insert_state(AppState::Lobby)
        .init_resource::<Session>()
        .init_resource::<Table>()
        .init_resource::<SelectedCorner>()
        .init_resource::<ChosenVariants>()
        .init_resource::<NetState>()
        .add_systems(Startup, move |mut commands: Commands| {
            let socket: MatchboxSocket = WebRtcSocketBuilder::new(url.clone())
                .reconnect_attempts(None)
                .add_reliable_channel()
                .into();
            commands.insert_resource(socket);
        })
        .add_systems(
            Update,
            (
                checkers_bevy::lobby::elect_host,
                checkers_bevy::lobby::pump_socket,
                checkers_bevy::lobby::select_corner,
                checkers_bevy::lobby::handle_buttons,
            )
                .chain()
                .run_if(in_state(AppState::Lobby)),
        )
        .add_systems(OnEnter(AppState::InGame), checkers_bevy::lobby::apply_seats);
    app.world_mut().resource_mut::<NetState>().name = name.into();
    app
}

fn net(app: &App) -> &NetState {
    app.world().resource::<NetState>()
}

/// The corners a roster actually seats, sorted — the amount every peer must
/// agree on. `players` in the `Start` message is built from exactly this.
fn roster_players(net: &NetState) -> Vec<u32> {
    let mut corners: Vec<u32> = net.seats.iter().filter_map(|s| s.player).collect();
    corners.sort_unstable();
    corners.dedup();
    corners
}

/// Wait until every instance's roster carries exactly these players. Unlike
/// plain agreement — which is true the moment two empty rosters match — this
/// waits for the *configured* outcome, so a claim broadcast this frame has
/// time to round-trip through the host and back before the assertion runs.
fn wait_players(apps: &mut [App], expected: &[u32], timeout: Duration) -> bool {
    wait_for(apps, timeout, |apps| {
        apps.iter().all(|a| roster_players(net(a)) == expected)
    })
}

/// A compact, readable rendering of a roster for the logs.
fn fmt_roster(net: &NetState) -> String {
    let seats = net
        .seats
        .iter()
        .map(|s| {
            format!(
                "{}@{} {}",
                s.name,
                s.player.map_or("-".into(), |p| p.to_string()),
                if s.engine { "(engine)" } else { "" }
            )
        })
        .collect::<Vec<_>>()
        .join(" ");
    format!("[{}]", seats)
}

/// Pump every instance while `ok` is false, until `ok` or the deadline.
///
/// The only source of timing in these tests: WebRTC handshakes and message
/// delivery are real and asynchronous, so a scenario is a sequence of
/// *wait for condition*, each holding until it is actually true.
fn wait_for(apps: &mut [App], timeout: Duration, mut ok: impl FnMut(&[App]) -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if ok(apps) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        for app in apps.iter_mut() {
            app.update();
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// A diagnostic line naming every instance's state, for assertions that fail
/// on a network the test cannot see into.
fn describe(apps: &[App]) -> String {
    apps.iter()
        .map(|a| {
            let n = net(a);
            let state = app_state(a);
            format!(
                "{}: peers={} host={} seats={} players={:?} state={} status={}",
                n.name,
                n.peers.len(),
                n.is_host,
                fmt_roster(n),
                roster_players(n),
                state,
                n.status
            )
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

fn app_state(app: &App) -> &'static str {
    match *app
        .world()
        .resource::<bevy::state::state::State<AppState>>()
        .get()
    {
        AppState::Lobby => "lobby",
        AppState::InGame => "ingame",
    }
}

fn log(line: &str) {
    println!("{line}");
}

fn digit_key(corner: usize) -> KeyCode {
    match corner {
        0 => KeyCode::Digit1,
        1 => KeyCode::Digit2,
        2 => KeyCode::Digit3,
        3 => KeyCode::Digit4,
        4 => KeyCode::Digit5,
        _ => KeyCode::Digit6,
    }
}

/// Press a key the way the window does: by sending a [`KeyboardInput`] message
/// (down, a frame, up). Exactly the helper `lobby_flow` uses — not
/// `ButtonInput::press`, which `InputPlugin` wipes before the update systems
/// see it.
fn press(app: &mut App, key: KeyCode) {
    app.world_mut().write_message(KeyboardInput {
        key_code: key,
        logical_key: Key::Character("x".into()),
        state: ButtonState::Pressed,
        text: None,
        repeat: false,
        window: Entity::PLACEHOLDER,
    });
    app.update();
    app.world_mut().write_message(KeyboardInput {
        key_code: key,
        logical_key: Key::Character("x".into()),
        state: ButtonState::Released,
        text: None,
        repeat: false,
        window: Entity::PLACEHOLDER,
    });
}

fn spawn_button(app: &mut App, tag: LobbyButton) -> Entity {
    app.world_mut().spawn((Button, Interaction::None, tag)).id()
}

fn press_button(app: &mut App, button: Entity) {
    app.world_mut()
        .entity_mut(button)
        .insert(Interaction::Pressed);
    app.update();
    app.world_mut().entity_mut(button).insert(Interaction::None);
}

/// Issue a corner command to one instance: select the corner with its digit
/// key, then press the Human / Computer / Off button.
fn choose(app: &mut App, command: CornerCommand, corner: usize) {
    press(app, digit_key(corner));
    let button = spawn_button(app, LobbyButton::CornerAction(command));
    press_button(app, button);
}

/// Wait until every instance sees exactly the same roster.
fn wait_roster_agreement(apps: &mut [App], timeout: Duration) -> bool {
    wait_for(apps, timeout, |apps| {
        let first = &net(&apps[0]).seats;
        apps.iter().all(|a| &net(a).seats == first)
    })
}

/// Wait until every seated corner on every instance is ready.
fn wait_all_seated_ready(apps: &mut [App], timeout: Duration) -> bool {
    wait_for(apps, timeout, |apps| {
        apps.iter()
            .all(|a| net(a).seats.iter().all(|s| s.player.is_none() || s.ready))
    })
}

/// Wait until every instance has entered the game.
fn wait_in_game(apps: &mut [App], timeout: Duration) -> bool {
    wait_for(apps, timeout, |apps| apps.iter().all(in_game))
}

fn in_game(app: &App) -> bool {
    app.world()
        .resource::<bevy::state::state::State<AppState>>()
        .get()
        == &AppState::InGame
}

/// Two instances: the host is the configurator (sets an engine, claims its own
/// corner, flips the house rule), the guest logs its own corner claim and the
/// host's configuration exactly as it arrives.
///
/// Even who plays which role is chosen by the running system: the election
/// decides, and the test drives whichever instance that is.
#[test]
#[ignore = "runs live WebRTC peers on an in-process signaling server"]
fn two_instances_share_one_engine_table() {
    let port = start_signaling_server();
    let room = fresh_room("pair");
    let mut apps = vec![instance("A", &room, port), instance("B", &room, port)];
    log(&format!(
        "[server] full-mesh signaling on ws://127.0.0.1:{port}/{}",
        room.0
    ));

    let connected = wait_for(&mut apps, Duration::from_secs(45), |apps| {
        apps.iter().all(|a| net(a).peers.len() == 1) && apps.iter().any(|a| net(a).is_host)
    });
    assert!(
        connected,
        "the two instances never saw each other: {}",
        describe(&apps)
    );

    // Greetings first: every instance must have its seat before it can claim a
    // corner for it, so nothing is configured until the empty baseline roster
    // is agreed everywhere.
    let greeted = wait_roster_agreement(&mut apps, Duration::from_secs(30));
    assert!(
        greeted,
        "the greetings never settled before configuring: {}",
        describe(&apps)
    );

    let host_i = apps.iter().position(|a| net(a).is_host).expect("a host");
    let guest_i = 1 - host_i;
    let (host_name, guest_name) = (
        net(&apps[host_i]).name.clone(),
        net(&apps[guest_i]).name.clone(),
    );
    log(&format!(
        "[{}] won the election; {} is the guest. The host configures the room, the guest logs its own setup and what it sees.",
        host_name, guest_name
    ));

    // The host-configured scenario: the foreign-camp house rule on, an engine
    // at corner 0, a human claim at corner 1.
    let host_app = &mut apps[host_i];
    log(&format!(
        "[{host_name}] configures: house rule on (F), corner 1 human, corner 0 engine"
    ));
    press(host_app, KeyCode::KeyF);
    choose(host_app, CornerCommand::Human, 1);
    choose(host_app, CornerCommand::Cpu, 0);

    // The guest's own setup: it claims corner 4, then logs what the host's
    // configuration looks like over the wire.
    let guest_app = &mut apps[guest_i];
    log(&format!("[{guest_name}] sets up itself: corner 4 human"));
    choose(guest_app, CornerCommand::Human, 4);

    let expected = vec![0, 1, 4];
    let configured = wait_players(&mut apps, &expected, Duration::from_secs(30));
    assert!(
        configured,
        "the configured table never settled on corners {expected:?}: {}",
        describe(&apps)
    );
    assert!(
        apps.iter().all(|a| roster_players(net(a)) == expected),
        "everyone must agree on corners {expected:?}: {}",
        describe(&apps)
    );
    log(&format!(
        "[{}] after the host's setup I see the roster {}",
        guest_name,
        fmt_roster(net(&apps[guest_i]))
    ));

    // Both sides ready up; the host starts.
    for app in apps.iter_mut() {
        press(app, KeyCode::Space);
    }
    let ready = wait_all_seated_ready(&mut apps, Duration::from_secs(30));
    assert!(ready, "readiness never converged: {}", describe(&apps));
    log(&format!(
        "[{host_name}] everyone is ready; starting the game"
    ));
    press(&mut apps[host_i], KeyCode::Enter);

    let started = wait_in_game(&mut apps, Duration::from_secs(30));
    assert!(
        started,
        "the game never started everywhere: {}",
        describe(&apps)
    );
    log(&format!("[{host_name}] started the game; everyone is in."));

    let host_session = apps[host_i].world().resource::<Session>();
    let guest_session = apps[guest_i].world().resource::<Session>();
    let all_players = [Player::ALL[0], Player::ALL[1], Player::ALL[4]];
    for (i, app) in apps.iter().enumerate() {
        let session = app.world().resource::<Session>();
        assert_eq!(session.players, all_players, "instance {i} dealt wrong");
    }
    assert_eq!(host_session.ai_players, vec![Player::ALL[0]]);
    assert_eq!(guest_session.ai_players, Vec::<Player>::new());
    assert_eq!(host_session.local_player(), Some(Player::ALL[1]));
    assert_eq!(guest_session.local_player(), Some(Player::ALL[4]));

    // The foreign-camp rule toggled by the host must have crossed the wire in
    // the Start: every peer now plays under the host's switch.
    for app in &apps {
        assert!(
            app.world()
                .resource::<ChosenVariants>()
                .0
                .forbid_foreign_camps,
            "the host's rule must reach every peer"
        );
    }
    log(&format!(
        "[{host_name}] dealt {}, I play player 1",
        session_text(host_session)
    ));
    log(&format!(
        "[{guest_name}] dealt {}, I play player 4",
        session_text(guest_session)
    ));
    log(
        "PASS: two instances share one engine table; the guest logged exactly what the host set up.",
    );
}

/// Three instances criss-crossing: each one configures itself. The host claims
/// a corner and seats an engine, one guest claims a corner, the other guest
/// claims a different corner — every instance ends up inside the same game at
/// the same moment, each driving its own camp.
#[test]
#[ignore = "runs live WebRTC peers on an in-process signaling server"]
fn three_instances_criss_cross() {
    let port = start_signaling_server();
    let room = fresh_room("triple");
    let mut apps = vec![
        instance("A", &room, port),
        instance("B", &room, port),
        instance("C", &room, port),
    ];

    let connected = wait_for(&mut apps, Duration::from_secs(45), |apps| {
        apps.iter().all(|a| net(a).peers.len() == 2) && apps.iter().any(|a| net(a).is_host)
    });
    assert!(
        connected,
        "the three instances never saw each other: {}",
        describe(&apps)
    );
    let greeted = wait_roster_agreement(&mut apps, Duration::from_secs(30));
    assert!(
        greeted,
        "the greetings never settled before configuring: {}",
        describe(&apps)
    );
    let host_i = apps.iter().position(|a| net(a).is_host).expect("a host");
    log(&format!(
        "[{}] won the election among three.",
        net(&apps[host_i]).name
    ));

    // The host sets its own camp and seats the engine.
    let host_name = net(&apps[host_i]).name.clone();
    choose(&mut apps[host_i], CornerCommand::Human, 5);
    choose(&mut apps[host_i], CornerCommand::Cpu, 0);
    log(&format!(
        "[{host_name}] configured: corner 5 human, corner 0 engine"
    ));

    // The two guests each claim a different corner; the smaller peer id goes
    // first so the test knows who should end up where.
    let mut guests: Vec<usize> = (0..apps.len()).filter(|&i| i != host_i).collect();
    guests.sort_by_key(|&i| net(&apps[i]).my_id.map(|id| id.to_string()));
    let corners: [u32; 2] = [4, 1];
    let claimed: Vec<u32> = guests
        .iter()
        .zip(corners)
        .map(|(guest_i, corner)| {
            let name = net(&apps[*guest_i]).name.clone();
            choose(&mut apps[*guest_i], CornerCommand::Human, corner as usize);
            log(&format!(
                "[{name}] configured itself: corner {corner} human"
            ));
            corner
        })
        .collect();

    let expected = vec![0, 1, 4, 5];
    let configured = wait_players(&mut apps, &expected, Duration::from_secs(30));
    assert!(
        configured,
        "the configured table never settled on corners {expected:?}: {}",
        describe(&apps)
    );
    assert!(
        apps.iter().all(|a| roster_players(net(a)) == expected),
        "everyone must agree on corners {expected:?}: {}",
        describe(&apps)
    );
    let engine_seats: usize = net(&apps[0]).seats.iter().filter(|s| s.engine).count();
    assert_eq!(engine_seats, 1, "exactly one engine, seated by the host");

    for app in apps.iter_mut() {
        press(app, KeyCode::Space);
    }
    let ready = wait_all_seated_ready(&mut apps, Duration::from_secs(30));
    assert!(ready, "readiness never converged: {}", describe(&apps));
    let host_name = net(&apps[host_i]).name.clone();
    log(&format!("[{host_name}] everyone is ready; starting."));
    press(&mut apps[host_i], KeyCode::Enter);

    let started = wait_in_game(&mut apps, Duration::from_secs(30));
    assert!(
        started,
        "the game never started everywhere: {}",
        describe(&apps)
    );

    let all_players = [
        Player::ALL[0],
        Player::ALL[1],
        Player::ALL[4],
        Player::ALL[5],
    ];
    for (i, app) in apps.iter().enumerate() {
        let session = app.world().resource::<Session>();
        assert_eq!(session.players, all_players, "instance {i} dealt wrong");
    }
    // The criss-cross landed where the claims went: every human camp has one
    // driver, the engine camp is driven by the host alone.
    let host_session = apps[host_i].world().resource::<Session>();
    let host_locals = (host_session.local_player(), host_session.ai_players.clone());
    let mut guest_locals: Vec<(String, Option<Player>)> = guests
        .iter()
        .map(|&i| {
            let app = &apps[i];
            (
                net(app).name.clone(),
                app.world().resource::<Session>().local_player(),
            )
        })
        .collect();
    guest_locals.sort_by(|a, b| a.0.cmp(&b.0));

    assert_eq!(host_locals.0, Some(Player::ALL[5]));
    assert_eq!(host_locals.1, vec![Player::ALL[0]]);
    for (i, app) in apps.iter().enumerate() {
        if i != host_i {
            assert!(
                app.world().resource::<Session>().ai_players.is_empty(),
                "only the host runs the engine"
            );
        }
    }
    // Each guest ended driving the corner it claimed.
    for (&guest_i, corner) in guests.iter().zip(&claimed) {
        let local = apps[guest_i].world().resource::<Session>().local_player();
        assert_eq!(
            local,
            Some(Player::ALL[*corner as usize]),
            "guest {} must drive its own claim at corner {corner}",
            net(&apps[guest_i]).name
        );
    }
    log(&format!(
        "[{}] dealt {}, I play player 5, engine drives player 0",
        host_name,
        session_text(host_session)
    ));
    for (name, local) in &guest_locals {
        let player = local.expect("a guest must drive its claim");
        log(&format!(
            "[{name}] dealt {}, I play player {}",
            session_text(
                apps.iter()
                    .find(|a| net(a).name == *name)
                    .expect("back to the app")
                    .world()
                    .resource::<Session>()
            ),
            u32::from(player.index())
        ));
    }
    log("PASS: three instances criss-cross; each drives its own camp of one shared game.");
}

/// Three instances again, but one takes no camp: the observer watches. The
/// two active peers decide the game between them, and the observer still sees
/// the same roster and the same deal, with nothing to command.
#[test]
#[ignore = "runs live WebRTC peers on an in-process signaling server"]
fn a_spectator_watches_the_pair() {
    let port = start_signaling_server();
    let room = fresh_room("watch");
    let mut apps = vec![
        instance("A", &room, port),
        instance("B", &room, port),
        instance("C", &room, port),
    ];

    let connected = wait_for(&mut apps, Duration::from_secs(45), |apps| {
        apps.iter().all(|a| net(a).peers.len() == 2) && apps.iter().any(|a| net(a).is_host)
    });
    assert!(
        connected,
        "the three instances never saw each other: {}",
        describe(&apps)
    );
    let greeted = wait_roster_agreement(&mut apps, Duration::from_secs(30));
    assert!(
        greeted,
        "the greetings never settled before configuring: {}",
        describe(&apps)
    );
    let host_i = apps.iter().position(|a| net(a).is_host).expect("a host");
    // The spectator is the peer with the largest id, which can never also be
    // the host (the host is the smallest id), so the election cannot fold a
    // watcher into the active pair. Whichever physical instance that is.
    let spectator_i = apps
        .iter()
        .enumerate()
        .max_by_key(|(_, a)| net(a).my_id.map(|id| id.to_string()))
        .expect("three instances")
        .0;
    let spectator_name = net(&apps[spectator_i]).name.clone();
    let active: Vec<usize> = (0..apps.len()).filter(|&i| i != spectator_i).collect();
    debug_assert!(active.contains(&host_i), "the host is one of the players");
    let player_i = *active
        .iter()
        .find(|&&i| i != host_i)
        .expect("two active peers");
    let host_name = net(&apps[host_i]).name.clone();
    let player_name = net(&apps[player_i]).name.clone();

    // The host seats the engine on its own corner and claims it; the other
    // active peer claims its corner. The spectator configures and readies
    // nothing.
    choose(&mut apps[host_i], CornerCommand::Human, 2);
    choose(&mut apps[host_i], CornerCommand::Cpu, 0);
    choose(&mut apps[player_i], CornerCommand::Human, 4);
    log(&format!(
        "[{host_name}] configures: corner 2 human, corner 0 engine; [{player_name}] claims corner 4; [{spectator_name}] watches."
    ));

    let expected = vec![0, 2, 4];
    let configured = wait_players(&mut apps, &expected, Duration::from_secs(30));
    assert!(
        configured,
        "the configured table never settled on corners {expected:?}: {}",
        describe(&apps)
    );
    assert!(
        apps.iter().all(|a| roster_players(net(a)) == expected),
        "everyone must agree on corners {expected:?}: {}",
        describe(&apps)
    );

    // Only the seated peers ready; the spectator stays a spectator.
    for app in apps.iter_mut() {
        press(app, KeyCode::Space);
    }
    let ready = wait_all_seated_ready(&mut apps, Duration::from_secs(30));
    assert!(ready, "readiness never converged: {}", describe(&apps));
    press(&mut apps[host_i], KeyCode::Enter);

    let started = wait_in_game(&mut apps, Duration::from_secs(30));
    assert!(
        started,
        "the game never started everywhere: {}",
        describe(&apps)
    );

    let all_players = [Player::ALL[0], Player::ALL[2], Player::ALL[4]];
    for (i, app) in apps.iter().enumerate() {
        let session = app.world().resource::<Session>();
        assert_eq!(session.players, all_players, "instance {i} dealt wrong");
    }
    let watcher = apps[spectator_i].world().resource::<Session>();
    assert_eq!(
        watcher.local_player(),
        None,
        "the spectator commands no camp"
    );
    assert!(
        watcher.spectating,
        "the observer must know it is only watching"
    );
    log(&format!(
        "[{spectator_name}] dealt {} and spectates; nobody commands me, I watch the pair.",
        session_text(watcher)
    ));
    log("PASS: a spectator observes the pair and receives the same game.");
}

fn session_text(session: &Session) -> String {
    let players: Vec<u32> = session
        .players
        .iter()
        .map(|p| u32::from(p.index()))
        .collect();
    format!("players {players:?}")
}
