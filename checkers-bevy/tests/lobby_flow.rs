//! Drives the real lobby schedule with real input events.
//!
//! The unit tests in `lobby` cover `seating_from_keys` as a function, and the
//! ones in `setup` cover what a seating means. Neither covers the *wiring*: that
//! the key actually reaches the system, that the system writes the resource,
//! that entering the game rebuilds the session from it, and that the board which
//! results matches the choice.
//!
//! That gap is not hypothetical. Pressing `3` against the running app produced a
//! two-player board, and the unit tests were green throughout — because they
//! never exercised the path from keypress to dealt board. Synthetic keystrokes
//! through the window manager turned out to be an unreliable way to check it, so
//! the schedule is driven directly here.
//!
//! No `DefaultPlugins`: no window, no renderer, no signaling socket. Only the
//! state machine and the lobby's own systems, which is what is under test.

use bevy::input::InputPlugin;
use bevy::input::keyboard::KeyboardInput;
use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use checkers_bevy::lobby::ChosenSeating;
use checkers_bevy::setup::Seating;
use checkers_bevy::{AppState, Session};
use checkers_core::position::Player;

/// A minimal app running the lobby's decision systems.
///
/// `lobby::plugin` is not used wholesale: it registers `open_socket`, which
/// would reach for the network. The systems that interpret input are added
/// directly instead.
fn app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, InputPlugin, StatesPlugin))
        .init_state::<AppState>()
        // The app boots into the main menu; these tests exercise the lobby, so
        // they start there directly.
        .insert_state(AppState::Lobby)
        .init_resource::<Session>()
        .init_resource::<ChosenSeating>()
        .init_resource::<checkers_net::NetState>()
        .add_systems(
            Update,
            checkers_bevy::lobby::choose_seating.run_if(in_state(AppState::Lobby)),
        )
        .add_systems(OnEnter(AppState::InGame), checkers_bevy::lobby::apply_seats);
    app
}

/// Press a key the way the window does: by sending a [`KeyboardInput`] message.
/// (Bevy 0.19 renamed buffered events to messages, hence `write_message`.)
///
/// Not by calling `ButtonInput::press` directly. `InputPlugin` runs
/// `keyboard_input_system` in `PreUpdate`, and that begins by clearing
/// `just_pressed` — so a flag set before `update()` is wiped before any `Update`
/// system sees it, and every assertion fails while the app is perfectly correct.
/// I hit exactly that and briefly took it for the bug I was chasing.
fn press(app: &mut App, key: KeyCode) {
    app.world_mut().write_message(KeyboardInput {
        key_code: key,
        logical_key: bevy::input::keyboard::Key::Character("x".into()),
        state: bevy::input::ButtonState::Pressed,
        text: None,
        repeat: false,
        window: Entity::PLACEHOLDER,
    });
    app.update();
    app.world_mut().write_message(KeyboardInput {
        key_code: key,
        logical_key: bevy::input::keyboard::Key::Character("x".into()),
        state: bevy::input::ButtonState::Released,
        text: None,
        repeat: false,
        window: Entity::PLACEHOLDER,
    });
}

#[test]
fn each_digit_deals_the_board_it_names() {
    for (key, seating) in [
        (KeyCode::Digit2, Seating::Two),
        (KeyCode::Digit3, Seating::Three),
        (KeyCode::Digit6, Seating::Six),
    ] {
        let mut app = app();
        press(&mut app, key);

        assert_eq!(
            app.world().resource::<ChosenSeating>().0,
            seating,
            "{key:?} must choose {seating:?}"
        );

        // Enter the game and let `apply_seats` rebuild the session.
        app.world_mut()
            .resource_mut::<NextState<AppState>>()
            .set(AppState::InGame);
        app.update();
        app.update();

        let session = app.world().resource::<Session>();
        assert_eq!(session.seating, seating, "{key:?}: session seating");

        // The dealt board must match, which is the claim the unit tests could
        // not make: it is the composition of choice, transition, and rebuild.
        let position = session.game.position();
        for player in Player::ALL {
            let found = position.pieces_of(player).len();
            let expected = if seating.players().contains(&player) {
                10
            } else {
                0
            };
            assert_eq!(
                found,
                expected,
                "{key:?}: player {} has {found} pieces, expected {expected}",
                player.index()
            );
        }
        assert!(
            seating.players().contains(&session.game.turn()),
            "{key:?}: the game must open on a seated player"
        );
    }
}

/// Tab must cycle through every seating and come back, so the whole control is
/// reachable from one key.
#[test]
fn tab_cycles_the_seating_in_the_running_app() {
    let mut app = app();
    let mut seen = vec![app.world().resource::<ChosenSeating>().0];

    for _ in 0..Seating::ALL.len() {
        press(&mut app, KeyCode::Tab);
        let now = app.world().resource::<ChosenSeating>().0;
        if !seen.contains(&now) {
            seen.push(now);
        }
    }

    assert_eq!(
        seen.len(),
        Seating::ALL.len(),
        "Tab must reach every seating, saw {seen:?}"
    );
    assert_eq!(
        app.world().resource::<ChosenSeating>().0,
        Seating::default(),
        "cycling all the way round must return to the start"
    );
}

/// A key that names no seating must leave the choice alone, rather than
/// resetting it to the default.
#[test]
fn an_unrelated_key_leaves_the_choice_alone() {
    let mut app = app();
    press(&mut app, KeyCode::Digit3);
    assert_eq!(app.world().resource::<ChosenSeating>().0, Seating::Three);

    for key in [KeyCode::KeyX, KeyCode::Digit4, KeyCode::Digit5] {
        press(&mut app, key);
        assert_eq!(
            app.world().resource::<ChosenSeating>().0,
            Seating::Three,
            "{key:?} must not change the seating"
        );
    }
}

/// The default must be the full game, so someone who touches nothing gets the
/// six-player board the specification describes.
#[test]
fn the_default_is_the_specified_six_player_game() {
    let mut app = app();
    assert_eq!(app.world().resource::<ChosenSeating>().0, Seating::Six);

    app.world_mut()
        .resource_mut::<NextState<AppState>>()
        .set(AppState::InGame);
    app.update();
    app.update();

    let session = app.world().resource::<Session>();
    assert_eq!(
        session.game.position(),
        &checkers_core::position::Position::initial(),
        "the untouched default must be the standard opening position"
    );
    // And it must satisfy the specification's own audit, unrestricted.
    checkers_core::audit::audit_position(session.game.position(), session.game.players())
        .expect("the six-player default must pass the specification's audit");
}
