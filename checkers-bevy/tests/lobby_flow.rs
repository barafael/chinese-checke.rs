//! Drives the real lobby schedule with real input events.
//!
//! The unit tests in `lobby` cover `corner_effect`, `start_decision` and the
//! deal as pure functions, and the ones in `setup` cover what a seating means.
//! Neither covers the *wiring*: that the key reaches the system, that the
//! system writes the resource, that entering the game rebuilds the session
//! from it, and that the board which results matches the choice.
//!
//! That gap is not hypothetical. Pressing a corner digit against the running
//! app produced nothing until the buttons rewrote the table, and the unit tests
//! were green throughout — because they never exercised the path from keypress
//! to dealt board. Synthetic keystrokes through the window manager turned out
//! to be an unreliable way to check it, so the schedule is driven directly here.
//!
//! No `DefaultPlugins`: no window, no renderer, no signaling socket. Only the
//! state machine and the lobby's own systems, which is what is under test.

mod common;

use bevy::prelude::*;
use checkers_bevy::lobby::{CornerCommand, CornerState, LobbyButton, SelectedCorner, Table};
use checkers_bevy::setup::Seating;
use checkers_bevy::{AppState, Session, lobby};
use checkers_core::position::Player;
use common::{choose, click, digit_key, press, state, status};

/// A minimal app running the lobby's decision systems.
///
/// `lobby::plugin` is not used wholesale: it registers `open_socket`, which
/// would reach for the network. The systems that interpret input are added
/// directly instead. `handle_buttons` takes the socket as an `Option` — no
/// `MatchboxSocket` resource, so every press here is a solo press.
fn app() -> App {
    let mut app = common::lobby_app();
    app.add_systems(
        Update,
        (lobby::select_corner, lobby::handle_buttons).run_if(in_state(AppState::Lobby)),
    )
    .add_systems(OnEnter(AppState::InGame), lobby::apply_seats);
    app
}

/// Enter the game from the lobby and settle the state transition.
fn enter_the_game(app: &mut App) {
    press(app, KeyCode::Enter);
    app.update();
}

#[test]
fn each_digit_selects_its_corner() {
    for corner in 0..6 {
        let mut app = app();
        press(&mut app, digit_key(corner));
        assert_eq!(
            app.world().resource::<SelectedCorner>().0,
            Some(corner),
            "digit {} must select corner {corner}",
            corner + 1
        );
    }
}

/// A preset is a shortcut that fills the whole table with a symmetric game, so
/// what is configured and what the shortcut claims can never silently differ.
#[test]
fn a_preset_fills_the_table() {
    let mut app = app();
    click(&mut app, LobbyButton::Preset(Seating::Two));

    let table = &app.world().resource::<Table>().0;
    assert_eq!(table[0], CornerState::Human);
    assert_eq!(table[3], CornerState::Human);
    assert_eq!(
        table.iter().filter(|c| **c != CornerState::Empty).count(),
        2,
        "a preset fills exactly its camps"
    );
}

/// The corner buttons rewrite the selected corner.
#[test]
fn corner_commands_configure_the_table() {
    let mut app = app();
    choose(&mut app, CornerCommand::Human, 0);
    assert_eq!(app.world().resource::<Table>().0[0], CornerState::Human);

    click(&mut app, LobbyButton::CornerAction(CornerCommand::Cpu));
    assert_eq!(app.world().resource::<Table>().0[0], CornerState::Cpu);

    click(&mut app, LobbyButton::CornerAction(CornerCommand::Off));
    assert_eq!(app.world().resource::<Table>().0[0], CornerState::Empty);
}

/// The whole point: Enter deals exactly the configured corners. Configure 0 by
/// hand and 3 as an engine, start, and the session must be built for precisely
/// those two camps — the composition of choice, transition, and rebuild.
#[test]
fn the_game_deals_exactly_the_configured_corners() {
    let mut app = app();
    choose(&mut app, CornerCommand::Human, 0);
    choose(&mut app, CornerCommand::Cpu, 3);

    enter_the_game(&mut app);

    let session = app.world().resource::<Session>();
    assert_eq!(
        session.players,
        vec![Player::ALL[0], Player::ALL[3]],
        "the deal reads the configured corners, sorted"
    );
    assert_eq!(session.ai_players, vec![Player::ALL[3]]);
    assert_eq!(session.game.position().pieces_of(Player::ALL[0]).len(), 10);
    assert_eq!(session.game.position().pieces_of(Player::ALL[5]).len(), 0);
}

/// One corner cannot be a game: the turn would visit a player's camp with
/// nobody else to play. Enter must refuse, explain, and stay in the lobby.
#[test]
fn a_single_corner_refuses_to_start() {
    let mut app = app();
    choose(&mut app, CornerCommand::Cpu, 0);

    enter_the_game(&mut app);

    assert_eq!(
        state(&app),
        AppState::Lobby,
        "a one-corner start must be refused"
    );
    let refusal = status(&app);
    assert!(
        refusal.contains("two corners"),
        "must explain the refusal: {refusal}"
    );
}

/// Every corner an engine, and nobody is a player: the board still starts, as
/// a watched race.
#[test]
fn an_all_cpu_table_starts_as_a_spectator() {
    let mut app = app();
    choose(&mut app, CornerCommand::Cpu, 0);
    choose(&mut app, CornerCommand::Cpu, 3);

    enter_the_game(&mut app);

    let session = app.world().resource::<Session>();
    assert!(
        session.spectating,
        "two engines with no human is a watched race"
    );
    assert_eq!(session.ai_players.len(), 2);
}
