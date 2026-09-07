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

use bevy::input::InputPlugin;
use bevy::input::keyboard::KeyboardInput;
use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use checkers_bevy::lobby::{
    ChosenVariants, CornerCommand, CornerState, LobbyButton, SelectedCorner, Table,
};
use checkers_bevy::setup::Seating;
use checkers_bevy::{AppState, Session};
use checkers_core::position::Player;

/// A minimal app running the lobby's decision systems.
///
/// `lobby::plugin` is not used wholesale: it registers `open_socket`, which
/// would reach for the network. The systems that interpret input are added
/// directly instead. `handle_buttons` takes the socket as an `Option` — no
/// `MatchboxSocket` resource, so every press here is a solo press.
fn app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, InputPlugin, StatesPlugin))
        .init_state::<AppState>()
        // The app boots into the lobby directly for these tests.
        .insert_state(AppState::Lobby)
        .init_resource::<Session>()
        .init_resource::<Table>()
        .init_resource::<SelectedCorner>()
        .init_resource::<ChosenVariants>()
        .init_resource::<checkers_net::NetState>()
        .add_systems(
            Update,
            (checkers_bevy::lobby::select_corner, checkers_bevy::lobby::handle_buttons)
                .run_if(in_state(AppState::Lobby)),
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

/// A clickable button in the world. `handle_buttons` reads
/// `(&Interaction, &LobbyButton)` with a `Changed` filter, so a press is
/// inserting `Interaction::Pressed` for one frame and standing it down again,
/// which both marks the change and lets the same button be pressed again.
fn spawn_button(app: &mut App, tag: LobbyButton) -> Entity {
    app.world_mut().spawn((Button, Interaction::None, tag)).id()
}

fn press_button(app: &mut App, button: Entity) {
    app.world_mut().entity_mut(button).insert(Interaction::Pressed);
    app.update();
    app.world_mut().entity_mut(button).insert(Interaction::None);
}

/// Enter the game from the lobby and settle the state transition.
fn enter_the_game(app: &mut App) {
    press(app, KeyCode::Enter);
    app.update();
}

#[test]
fn each_digit_selects_its_corner() {
    for digit in 1..=6 {
        let mut app = app();
        press(&mut app, key_for(digit));
        assert_eq!(
            app.world().resource::<SelectedCorner>().0,
            Some(digit as usize - 1),
            "{digit} must select corner {}",
            digit - 1
        );
    }
}

/// A preset is a shortcut that fills the whole table with a symmetric game, so
/// what is configured and what the shortcut claims can never silently differ.
#[test]
fn a_preset_fills_the_table() {
    let mut app = app();
    let two = spawn_button(&mut app, LobbyButton::Preset(Seating::Two));
    press_button(&mut app, two);

    let table = app.world().resource::<Table>().0.clone();
    assert_eq!(table[0], CornerState::Human("P0".into()));
    assert_eq!(table[3], CornerState::Human("P3".into()));
    assert_eq!(
        table.iter().filter(|c| **c != CornerState::Empty).count(),
        2,
        "a preset fills exactly its camps"
    );
}

/// The corner buttons rewrite the selected corner, keeping the name when a
/// human corner is toggled off and on.
#[test]
fn corner_commands_configure_the_table() {
    let mut app = app();
    press(&mut app, KeyCode::Digit1);
    let human = spawn_button(&mut app, LobbyButton::CornerAction(CornerCommand::Human));
    press_button(&mut app, human);
    assert_eq!(
        app.world().resource::<Table>().0[0],
        CornerState::Human("P0".into())
    );

    let cpu = spawn_button(&mut app, LobbyButton::CornerAction(CornerCommand::Cpu));
    press_button(&mut app, cpu);
    assert_eq!(app.world().resource::<Table>().0[0], CornerState::Cpu);

    let off = spawn_button(&mut app, LobbyButton::CornerAction(CornerCommand::Off));
    press_button(&mut app, off);
    assert_eq!(app.world().resource::<Table>().0[0], CornerState::Empty);
}

/// The whole point: Enter deals exactly the configured corners. Configure 0 by
/// hand and 3 as an engine, start, and the session must be built for precisely
/// those two camps — the composition of choice, transition, and rebuild.
#[test]
fn the_game_deals_exactly_the_configured_corners() {
    let mut app = app();

    press(&mut app, KeyCode::Digit1);
    let human = spawn_button(&mut app, LobbyButton::CornerAction(CornerCommand::Human));
    press_button(&mut app, human);
    press(&mut app, KeyCode::Digit4);
    let cpu = spawn_button(&mut app, LobbyButton::CornerAction(CornerCommand::Cpu));
    press_button(&mut app, cpu);

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
    press(&mut app, KeyCode::Digit1);
    let cpu = spawn_button(&mut app, LobbyButton::CornerAction(CornerCommand::Cpu));
    press_button(&mut app, cpu);

    enter_the_game(&mut app);

    assert_eq!(
        app.world().resource::<State<AppState>>().get(),
        &AppState::Lobby,
        "a one-corner start must be refused"
    );
    let status = app.world().resource::<checkers_net::NetState>().status.clone();
    assert!(status.contains("two corners"), "must explain the refusal: {status}");
}

/// Every corner an engine, and nobody is a player: the board still starts, as
/// a watched race.
#[test]
fn an_all_cpu_table_starts_as_a_spectator() {
    let mut app = app();
    press(&mut app, KeyCode::Digit1);
    let cpu = spawn_button(&mut app, LobbyButton::CornerAction(CornerCommand::Cpu));
    press_button(&mut app, cpu);
    press(&mut app, KeyCode::Digit4);
    let cpu2 = spawn_button(&mut app, LobbyButton::CornerAction(CornerCommand::Cpu));
    press_button(&mut app, cpu2);

    enter_the_game(&mut app);

    let session = app.world().resource::<Session>();
    assert!(session.spectating, "two engines with no human is a watched race");
    assert_eq!(session.ai_players.len(), 2);
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