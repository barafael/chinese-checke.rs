//! Helpers shared by the integration tests: the app run headless, keys and
//! buttons pressed the way the window presses them, and small fixtures.

// Each file under `tests/` is its own crate and uses only part of this module.
#![allow(dead_code)]

use bevy::input::ButtonState;
use bevy::input::InputPlugin;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input_focus::{InputDispatchPlugin, InputFocusPlugin};
use bevy::picking::events::{PointerRelease, PointerState};
use bevy::prelude::*;
use bevy::scene::ScenePlugin;
use bevy::state::app::StatesPlugin;
use bevy::text::TextPlugin;
use bevy::ui_widgets::{Activate, TextInputPlugin};
use bevy::window::{ExitCondition, PrimaryWindow};
use checkers_bevy::lobby::{
    self, ChosenVariants, CornerCommand, LobbyButton, LobbyStatus, PendingClaim, SelectedCorner,
    Table,
};
use checkers_bevy::{AppState, Session};
use checkers_core::geometry::Coord;
use checkers_core::position::{Player, Position};
use checkers_net::NetState;

/// An app with no window, no renderer and no socket: only the input and state
/// plugins the lobby reads.
pub fn headless_app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, InputPlugin, StatesPlugin))
        .init_state::<AppState>()
        // The app boots into the menu; these tests start in the lobby.
        .insert_state(AppState::Lobby);
    app
}

/// Bevy's text input, run headless: input focus and the dispatch of keys to
/// the focused field, the text pipeline that applies the edits, and the few
/// resources its systems read that a real window and pointer would bring.
/// Plus the primary window, which keys go to while no field is focused, and
/// scenes, which the fields are spawned from.
pub fn text_input(app: &mut App) {
    app.add_plugins((
        AssetPlugin::default(),
        ScenePlugin,
        WindowPlugin {
            primary_window: None,
            exit_condition: ExitCondition::DontExit,
            ..default()
        },
        InputFocusPlugin,
        InputDispatchPlugin,
        TextPlugin,
        TextInputPlugin,
    ))
    .add_message::<PointerRelease>()
    .init_resource::<PointerState>()
    .init_resource::<UiScale>();
    app.world_mut().spawn((Window::default(), PrimaryWindow));
}

/// [`headless_app`] with the resources the lobby's own systems work on, and
/// its clicks reported the way the app reports them.
pub fn lobby_app() -> App {
    let mut app = headless_app();
    app.add_plugins(lobby::clicks_plugin)
        .init_resource::<Session>()
        .init_resource::<Table>()
        .init_resource::<SelectedCorner>()
        .init_resource::<ChosenVariants>()
        .init_resource::<NetState>()
        .init_resource::<LobbyStatus>()
        .init_resource::<PendingClaim>();
    app
}

/// A key message as the window sends it, with layout-aware `text`, and the
/// logical key a text field reads its editing keys from.
pub fn key_message(key: KeyCode, state: ButtonState, text: Option<&str>) -> KeyboardInput {
    let logical_key = match key {
        KeyCode::Enter | KeyCode::NumpadEnter => Key::Enter,
        KeyCode::Escape => Key::Escape,
        _ => Key::Character(text.unwrap_or("x").into()),
    };
    KeyboardInput {
        key_code: key,
        logical_key,
        state,
        text: text.map(Into::into),
        repeat: false,
        window: Entity::PLACEHOLDER,
    }
}

/// Press a key the way the window does: by sending a [`KeyboardInput`] message
/// (down, a frame, up).
///
/// Not by calling `ButtonInput::press` directly. `InputPlugin` runs
/// `keyboard_input_system` in `PreUpdate`, and that begins by clearing
/// `just_pressed` — so a flag set before `update()` is wiped before any `Update`
/// system sees it, and every assertion fails while the app is perfectly correct.
/// I hit exactly that and briefly took it for the bug I was chasing.
pub fn press(app: &mut App, key: KeyCode) {
    app.world_mut()
        .write_message(key_message(key, ButtonState::Pressed, None));
    app.update();
    app.world_mut()
        .write_message(key_message(key, ButtonState::Released, None));
}

/// The digit key that selects `corner` (0-based).
pub fn digit_key(corner: usize) -> KeyCode {
    match corner {
        0 => KeyCode::Digit1,
        1 => KeyCode::Digit2,
        2 => KeyCode::Digit3,
        3 => KeyCode::Digit4,
        4 => KeyCode::Digit5,
        _ => KeyCode::Digit6,
    }
}

/// Click a lobby button: the [`Activate`] a completed click fires on the
/// button, which the app reports to `handle_buttons` as the button's
/// [`LobbyButton`] message.
pub fn click(app: &mut App, tag: LobbyButton) {
    activate(app, tag);
}

/// Fire [`Activate`] on a button carrying `tag`, as a completed click does,
/// and run the frame that reads it.
pub fn activate(app: &mut App, tag: impl Component) {
    let button = app.world_mut().spawn(tag).id();
    app.world_mut().trigger(Activate { entity: button });
    app.update();
    app.world_mut().despawn(button);
}

/// Issue a corner command: select the corner with its digit key, then press
/// the Human / Computer / Off button.
pub fn choose(app: &mut App, command: CornerCommand, corner: usize) {
    press(app, digit_key(corner));
    click(app, LobbyButton::CornerAction(command));
}

pub fn state(app: &App) -> AppState {
    *app.world().resource::<State<AppState>>().get()
}

pub fn status(app: &App) -> &str {
    &app.world().resource::<LobbyStatus>().0
}

pub fn set_state(app: &mut App, state: AppState) {
    app.world_mut()
        .resource_mut::<NextState<AppState>>()
        .set(state);
}

pub fn net(app: &App) -> &NetState {
    app.world().resource::<NetState>()
}

pub fn session(app: &App) -> &Session {
    app.world().resource::<Session>()
}

/// The first of `player`'s pieces that `destinations` offers anywhere to go,
/// and the first hole it offers.
pub fn first_offered(
    pos: &Position,
    player: Player,
    destinations: fn(&Position, Coord) -> Vec<Coord>,
) -> Option<(Coord, Coord)> {
    pos.pieces_of(player)
        .into_iter()
        .find_map(|origin| Some((origin, *destinations(pos, origin).first()?)))
}
