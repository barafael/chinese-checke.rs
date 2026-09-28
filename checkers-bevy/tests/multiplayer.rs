//! Live multiplayer lobby: two and three real instances on one room.
//!
//! `lobby_flow` drives the lobby decisions in a single app with no socket.
//! These tests go one step further: **each instance is its own headless Bevy
//! app** running the real lobby systems (`elect_host`, `pump_socket`,
//! `select_corner`, `handle_buttons`, `apply_seats`), each with its own real
//! [`MatchboxSocket`]. The apps are introduced by an in-process full-mesh
//! signaling server (the same crate the fork ships for native development)
//! and then talk peer-to-peer over an actual WebRTC data channel.
//!
//! So the whole shared-room contract crosses a real wire here: host election,
//! greetings, corner claims, engine seats, the roster broadcasts, and the
//! host's `Start` — and the guest's lobby reflects what the peer actually
//! configured, because there is no other copy of the truth to read.
//!
//! There is no window and no renderer: `MinimalPlugins`, plus the input and
//! state plugins the lobby reads. An instance is driven exactly like the web
//! build is operated — digit keys select a corner, the corner buttons human /
//! computer / off, F flips the foreign-camp rule, Enter starts — so the
//! code paths under test are the app's own, not a reimplementation.
//!
//! These are the slow siblings of the unit tests — each scenario waits for a
//! real peer handshake to complete — so they are `#[ignore]`d and opt in:
//!
//! ```sh
//! cargo test -p checkers-bevy --test multiplayer -- --ignored --nocapture
//! ```

mod common;

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use bevy::prelude::*;
use bevy_matchbox::prelude::*;
use checkers_bevy::lobby::{self, ChosenVariants, CornerCommand};
use checkers_bevy::{AppState, Session};
use checkers_core::position::Player;
use checkers_net::{NetState, RoomId, Signaling};
use common::{choose, net, press, session, set_state, state, status};

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
                matchbox_signaling::SignalingServer::full_mesh_builder(([127, 0, 0, 1], port))
                    .build();
            runtime
                .block_on(server.serve())
                .expect("the signaling server ran");
        })
        .expect("spawn the signaling server thread");

    port
}

/// How long to wait for the peers to find each other, for the room to settle
/// on what was configured, and for a step that needs no handshake.
const CONNECT: Duration = Duration::from_secs(45);
const SETTLE: Duration = Duration::from_secs(30);
const BRIEF: Duration = Duration::from_secs(10);

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
/// The socket is opened by the app's own [`checkers_net::open_socket`] on
/// entering the lobby, pointed at the in-process server through
/// [`Signaling`]. The state machine is the app's: lobby systems run while in
/// the lobby, entering the game runs [`checkers_bevy::lobby::apply_seats`] on
/// the `OnEnter` transition, and the game's own [`checkers_bevy::net::pump`]
/// runs while in it.
fn instance(name: &str, room: &RoomId, port: u16) -> App {
    let mut app = common::lobby_app();
    app.insert_resource(room.clone())
        .insert_resource(Signaling(format!("ws://127.0.0.1:{port}")))
        .add_systems(OnEnter(AppState::Lobby), checkers_net::open_socket)
        .add_systems(
            Update,
            checkers_bevy::net::pump.run_if(in_state(AppState::InGame)),
        )
        .add_systems(
            Update,
            (
                lobby::elect_host,
                lobby::pump_socket,
                lobby::settle_name_clash,
                lobby::select_corner,
                lobby::handle_buttons,
            )
                .chain()
                .run_if(in_state(AppState::Lobby)),
        )
        .add_systems(OnEnter(AppState::InGame), lobby::apply_seats);
    app.world_mut().resource_mut::<NetState>().name = name.into();
    app
}

/// Start a signaling server and put one instance per name into a fresh room,
/// waiting until each sees all the others and one of them is the host.
fn connect(label: &str, names: &[&str]) -> Vec<App> {
    let port = start_signaling_server();
    let room = fresh_room(label);
    println!(
        "[server] full-mesh signaling on ws://127.0.0.1:{port}/{}",
        room.0
    );
    let mut apps: Vec<App> = names
        .iter()
        .map(|name| instance(name, &room, port))
        .collect();
    let others = apps.len() - 1;
    wait_for(
        &mut apps,
        CONNECT,
        &format!("the {} instances never saw each other", names.len()),
        |apps| {
            apps.iter().all(|a| net(a).peers.len() == others) && apps.iter().any(|a| net(a).is_host)
        },
    );
    apps
}

/// Which instance won the election.
fn host_index(apps: &[App]) -> usize {
    apps.iter().position(|a| net(a).is_host).expect("a host")
}

/// The corners a roster actually seats, sorted — the amount every peer must
/// agree on. `players` in the `Start` message is built from exactly this.
fn roster_players(net: &NetState) -> Vec<u32> {
    let mut corners: Vec<u32> = net.seats.iter().filter_map(|s| s.player).collect();
    corners.sort_unstable();
    corners.dedup();
    corners
}

/// Wait until every instance sees exactly the same roster.
fn wait_greeted(apps: &mut [App]) {
    wait_for(apps, SETTLE, "the greetings never settled", |apps| {
        let first = &net(&apps[0]).seats;
        apps.iter().all(|a| &net(a).seats == first)
    });
}

/// Wait until every instance's roster carries exactly these players. Unlike
/// plain agreement — which is true the moment two empty rosters match — this
/// waits for the *configured* outcome, so a claim broadcast this frame has
/// time to round-trip through the host and back.
fn wait_players(apps: &mut [App], expected: &[u32], what: &str) {
    wait_for(
        apps,
        SETTLE,
        &format!("{what} on corners {expected:?}"),
        |apps| apps.iter().all(|a| roster_players(net(a)) == expected),
    );
}

/// The host presses Enter, and every instance must enter a fresh round: in
/// the game, and not still on the game-over card of the last one.
fn start(apps: &mut [App], host_i: usize, what: &str) {
    press(&mut apps[host_i], KeyCode::Enter);
    wait_for(apps, SETTLE, what, |apps| {
        apps.iter()
            .all(|a| in_game(a) && !session(a).game.is_over())
    });
}

/// A compact, readable rendering of a roster for the logs.
fn fmt_roster(net: &NetState) -> String {
    let seats: Vec<String> = net
        .seats
        .iter()
        .map(|s| {
            let corner = s.player.map_or("-".into(), |p| p.to_string());
            let engine = if s.engine { "(engine)" } else { "" };
            format!("{}@{corner} {engine}", s.name)
        })
        .collect();
    format!("[{}]", seats.join(" "))
}

/// Pump every instance until `ok`, failing with `what` and every instance's
/// state if the deadline comes first.
///
/// The only source of timing in these tests: WebRTC handshakes and message
/// delivery are real and asynchronous, so a scenario is a sequence of
/// *wait for condition*, each holding until it is actually true.
fn wait_for(apps: &mut [App], timeout: Duration, what: &str, mut ok: impl FnMut(&[App]) -> bool) {
    let deadline = Instant::now() + timeout;
    while !ok(apps) {
        assert!(Instant::now() < deadline, "{what}: {}", describe(apps));
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
            format!(
                "{}: peers={} host={} seats={} players={:?} state={:?} status={}",
                n.name,
                n.peers.len(),
                n.is_host,
                fmt_roster(n),
                roster_players(n),
                state(a),
                status(a),
            )
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

fn in_game(app: &App) -> bool {
    state(app) == AppState::InGame
}

/// Every instance dealt exactly these corners.
fn assert_dealt(apps: &[App], corners: &[usize]) {
    let players: Vec<Player> = corners.iter().map(|&c| Player::ALL[c]).collect();
    for (i, app) in apps.iter().enumerate() {
        assert_eq!(
            session(app).players,
            players,
            "instance {i} dealt the wrong corners"
        );
    }
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
    let mut apps = connect("pair", &["A", "B"]);

    // Greetings first: every instance must have its seat before it can claim a
    // corner for it, so nothing is configured until the empty baseline roster
    // is agreed everywhere.
    wait_greeted(&mut apps);

    let host_i = host_index(&apps);
    let guest_i = 1 - host_i;
    let host_name = net(&apps[host_i]).name.clone();
    let guest_name = net(&apps[guest_i]).name.clone();
    println!(
        "[{host_name}] won the election; {guest_name} is the guest. The host configures the room, the guest logs its own setup and what it sees."
    );

    // The host-configured scenario: the foreign-camp house rule on, an engine
    // at corner 0, a human claim at corner 1.
    println!("[{host_name}] configures: house rule on (F), corner 1 human, corner 0 engine");
    press(&mut apps[host_i], KeyCode::KeyF);
    choose(&mut apps[host_i], CornerCommand::Human, 1);
    choose(&mut apps[host_i], CornerCommand::Cpu, 0);

    // The guest's own setup: it claims corner 4, then logs what the host's
    // configuration looks like over the wire.
    println!("[{guest_name}] sets up itself: corner 4 human");
    choose(&mut apps[guest_i], CornerCommand::Human, 4);

    wait_players(&mut apps, &[0, 1, 4], "the configured table never settled");
    println!(
        "[{guest_name}] after the host's setup I see the roster {}",
        fmt_roster(net(&apps[guest_i]))
    );

    // The host just starts; readiness is not part of the setup any more.
    start(&mut apps, host_i, "the game never started everywhere");
    println!("[{host_name}] started the game; everyone is in.");

    assert_dealt(&apps, &[0, 1, 4]);
    let host = session(&apps[host_i]);
    let guest = session(&apps[guest_i]);
    assert_eq!(host.ai_players, vec![Player::ALL[0]]);
    assert_eq!(guest.ai_players, Vec::<Player>::new());
    assert_eq!(host.local_player(), Some(Player::ALL[1]));
    assert_eq!(guest.local_player(), Some(Player::ALL[4]));

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
    println!(
        "[{host_name}] dealt {}, I play player 1",
        session_text(host)
    );
    println!(
        "[{guest_name}] dealt {}, I play player 4",
        session_text(guest)
    );
    println!(
        "PASS: two instances share one engine table; the guest logged exactly what the host set up."
    );
}

/// Three instances criss-crossing: each one configures itself. The host claims
/// a corner and seats an engine, one guest claims a corner, the other guest
/// claims a different corner — every instance ends up inside the same game at
/// the same moment, each driving its own camp.
#[test]
#[ignore = "runs live WebRTC peers on an in-process signaling server"]
fn three_instances_criss_cross() {
    let mut apps = connect("triple", &["A", "B", "C"]);
    wait_greeted(&mut apps);
    let host_i = host_index(&apps);
    let host_name = net(&apps[host_i]).name.clone();
    println!("[{host_name}] won the election among three.");

    // The host sets its own camp and seats the engine.
    choose(&mut apps[host_i], CornerCommand::Human, 5);
    choose(&mut apps[host_i], CornerCommand::Cpu, 0);
    println!("[{host_name}] configured: corner 5 human, corner 0 engine");

    // The two guests each claim a different corner; the smaller peer id goes
    // first so the test knows who should end up where.
    let mut guests: Vec<usize> = (0..apps.len()).filter(|&i| i != host_i).collect();
    guests.sort_by_key(|&i| net(&apps[i]).my_id.map(|id| id.to_string()));
    let corners = [4, 1];
    for (&guest_i, corner) in guests.iter().zip(corners) {
        choose(&mut apps[guest_i], CornerCommand::Human, corner);
        let name = &net(&apps[guest_i]).name;
        println!("[{name}] configured itself: corner {corner} human");
    }

    wait_players(
        &mut apps,
        &[0, 1, 4, 5],
        "the configured table never settled",
    );
    let engine_seats: usize = net(&apps[0]).seats.iter().filter(|s| s.engine).count();
    assert_eq!(engine_seats, 1, "exactly one engine, seated by the host");

    println!("[{host_name}] starting.");
    start(&mut apps, host_i, "the game never started everywhere");
    assert_dealt(&apps, &[0, 1, 4, 5]);

    // The criss-cross landed where the claims went: every human camp has one
    // driver, the engine camp is driven by the host alone.
    let host = session(&apps[host_i]);
    assert_eq!(host.local_player(), Some(Player::ALL[5]));
    assert_eq!(host.ai_players, vec![Player::ALL[0]]);
    println!(
        "[{host_name}] dealt {}, I play player 5, engine drives player 0",
        session_text(host)
    );
    // Each guest ended driving the corner it claimed.
    for (&guest_i, corner) in guests.iter().zip(corners) {
        let name = &net(&apps[guest_i]).name;
        let guest = session(&apps[guest_i]);
        assert!(guest.ai_players.is_empty(), "only the host runs the engine");
        assert_eq!(
            guest.local_player(),
            Some(Player::ALL[corner]),
            "guest {name} must drive its own claim at corner {corner}"
        );
        println!(
            "[{name}] dealt {}, I play player {corner}",
            session_text(guest)
        );
    }
    println!("PASS: three instances criss-cross; each drives its own camp of one shared game.");
}

/// Three instances again, but one takes no camp: the observer watches. The
/// two active peers decide the game between them, and the observer still sees
/// the same roster and the same deal, with nothing to command.
#[test]
#[ignore = "runs live WebRTC peers on an in-process signaling server"]
fn a_spectator_watches_the_pair() {
    let mut apps = connect("watch", &["A", "B", "C"]);
    wait_greeted(&mut apps);
    let host_i = host_index(&apps);
    // The spectator is the peer with the largest id, which can never also be
    // the host (the host is the smallest id), so the election cannot fold a
    // watcher into the active pair. Whichever physical instance that is.
    let spectator_i = (0..apps.len())
        .max_by_key(|&i| net(&apps[i]).my_id.map(|id| id.to_string()))
        .expect("three instances");
    debug_assert_ne!(spectator_i, host_i, "the host is one of the players");
    let player_i = (0..apps.len())
        .find(|&i| i != spectator_i && i != host_i)
        .expect("two active peers");
    let host_name = net(&apps[host_i]).name.clone();
    let player_name = net(&apps[player_i]).name.clone();
    let spectator_name = net(&apps[spectator_i]).name.clone();

    // The host seats the engine on its own corner and claims it; the other
    // active peer claims its corner. The spectator configures and readies
    // nothing.
    choose(&mut apps[host_i], CornerCommand::Human, 2);
    choose(&mut apps[host_i], CornerCommand::Cpu, 0);
    choose(&mut apps[player_i], CornerCommand::Human, 4);
    println!(
        "[{host_name}] configures: corner 2 human, corner 0 engine; [{player_name}] claims corner 4; [{spectator_name}] watches."
    );

    wait_players(&mut apps, &[0, 2, 4], "the configured table never settled");
    start(&mut apps, host_i, "the game never started everywhere");
    assert_dealt(&apps, &[0, 2, 4]);

    let watcher = session(&apps[spectator_i]);
    assert_eq!(
        watcher.local_player(),
        None,
        "the spectator commands no camp"
    );
    assert!(
        watcher.spectating,
        "the observer must know it is only watching"
    );
    println!(
        "[{spectator_name}] dealt {} and spectates; nobody commands me, I watch the pair.",
        session_text(watcher)
    );
    println!("PASS: a spectator observes the pair and receives the same game.");
}

/// This instance's own id, as its socket reports it.
fn socket_id(app: &mut App) -> Option<PeerId> {
    app.world_mut().resource_mut::<MatchboxSocket>().id()
}

/// End the round in one instance, as a win, a draw or an abandonment would.
fn finish_round(app: &mut App) {
    app.world_mut().resource_mut::<Session>().game.abandon();
}

/// A finished round rematches over the socket it was played on.
///
/// Returning to the lobby used to open a second socket: a fresh peer id nobody
/// else knew, while every peer kept addressing the dead one. Here both peers
/// finish a round, come back, and the host deals again — the ids must be the
/// ones the round was played with, and the corners carry over. Then the guest
/// lingers on the game-over card while the host deals a third round: the
/// host's `Start` must reach it in the game and pull it in.
#[test]
#[ignore = "runs live WebRTC peers on an in-process signaling server"]
fn a_finished_round_rematches_over_the_same_socket() {
    let mut apps = connect("rematch", &["A", "B"]);
    wait_greeted(&mut apps);
    let host_i = host_index(&apps);
    let guest_i = 1 - host_i;

    choose(&mut apps[host_i], CornerCommand::Human, 0);
    choose(&mut apps[guest_i], CornerCommand::Human, 3);
    wait_players(&mut apps, &[0, 3], "the claims never settled");
    start(&mut apps, host_i, "the first round never started");

    let ids: Vec<Option<PeerId>> = apps.iter_mut().map(socket_id).collect();
    let peers: Vec<Vec<PeerId>> = apps.iter().map(|a| net(a).peers.clone()).collect();
    println!("[round 1] ids {ids:?}; both in the game");

    // Both finish and come back.
    for app in apps.iter_mut() {
        finish_round(app);
        set_state(app, AppState::Lobby);
    }
    wait_for(&mut apps, BRIEF, "never returned to the lobby", |apps| {
        apps.iter().all(|a| !in_game(a))
    });
    for (i, app) in apps.iter_mut().enumerate() {
        assert_eq!(socket_id(app), ids[i], "instance {i} opened a new socket");
        assert_eq!(net(app).my_id, ids[i], "instance {i} forgot its id");
        assert_eq!(net(app).peers, peers[i], "instance {i} lost its peer");
    }
    assert_eq!(
        roster_players(net(&apps[host_i])),
        vec![0, 3],
        "the corners carry over into the rematch"
    );

    start(&mut apps, host_i, "the rematch never started everywhere");
    println!("[round 2] the rematch dealt on both peers");

    // The guest lingers on the game-over card; only the host goes back.
    for app in apps.iter_mut() {
        finish_round(app);
    }
    set_state(&mut apps[host_i], AppState::Lobby);
    wait_for(&mut apps, BRIEF, "the host never returned", |apps| {
        !in_game(&apps[host_i])
    });
    start(
        &mut apps,
        host_i,
        "the host's Start never pulled the lingering guest in",
    );
    assert_dealt(&apps, &[0, 3]);
    println!("PASS: two rematches over one socket, one of them from the game-over card.");
}

/// A stalled engine-only race ends on every peer. Only the host drives the
/// engines, so only the host can call the stall; its guests end the round
/// when the host's `Abandon` arrives, instead of waiting for a move that is
/// never coming.
#[test]
#[ignore = "runs live WebRTC peers on an in-process signaling server"]
fn an_abandoned_engine_race_ends_everywhere() {
    use bevy::ecs::system::RunSystemOnce;
    use checkers_core::rules::Outcome;

    let mut apps = connect("abandon", &["A", "B"]);
    wait_greeted(&mut apps);
    let host_i = host_index(&apps);
    let guest_i = 1 - host_i;

    choose(&mut apps[host_i], CornerCommand::Cpu, 0);
    choose(&mut apps[host_i], CornerCommand::Cpu, 3);
    wait_players(&mut apps, &[0, 3], "the engines were never seated");
    start(&mut apps, host_i, "the race never started");

    // What the host's engine driver does when the stall detector trips.
    apps[host_i]
        .world_mut()
        .run_system_once(
            |mut session: ResMut<Session>,
             mut socket: Option<ResMut<MatchboxSocket>>,
             net: Res<NetState>| {
                checkers_bevy::net::abandon_round(&mut session, socket.as_deref_mut(), &net);
            },
        )
        .expect("the abandonment runs");

    wait_for(
        &mut apps,
        BRIEF,
        "the guest never learned the race was abandoned",
        |apps| session(&apps[guest_i]).game.outcome() == Some(Outcome::Abandoned),
    );
    println!("PASS: the host's abandonment ended the round on the guest too.");
}

/// Two instances that drew the same name end up with two names, the host
/// keeping its own; and a guest's claim is answered on its status line once
/// the host's roster grants it, instead of reading "Claiming..." forever.
#[test]
#[ignore = "runs live WebRTC peers on an in-process signaling server"]
fn a_shared_name_is_settled_and_a_claim_answered() {
    let mut apps = connect("clash", &["gecko", "gecko"]);
    let host_i = host_index(&apps);
    let guest_i = 1 - host_i;

    wait_for(&mut apps, SETTLE, "the clash was never settled", |apps| {
        let names: Vec<Vec<&str>> = apps
            .iter()
            .map(|a| {
                let mut n: Vec<&str> = net(a).seats.iter().map(|s| s.name.as_str()).collect();
                n.sort_unstable();
                n
            })
            .collect();
        names[0].len() == 2 && names[0][0] != names[0][1] && names[0] == names[1]
    });
    assert_eq!(net(&apps[host_i]).name, "gecko", "the host keeps its name");
    assert_ne!(net(&apps[guest_i]).name, "gecko", "the guest gives way");
    println!("[clash] the guest is now {}", net(&apps[guest_i]).name);

    choose(&mut apps[guest_i], CornerCommand::Human, 3);
    wait_for(&mut apps, SETTLE, "the claim was never answered", |apps| {
        status(&apps[guest_i]) == "You hold corner 3."
    });
    println!("PASS: the name clash was settled and the guest's claim answered.");
}

fn session_text(session: &Session) -> String {
    let players: Vec<u8> = session.players.iter().map(|p| p.index()).collect();
    format!("players {players:?}")
}
