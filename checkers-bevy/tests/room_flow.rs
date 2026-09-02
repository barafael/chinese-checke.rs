//! The room field end to end: typing, validation, and actually rejoining.
//!
//! The failure this guards is a control that *looks* like it works. The room is
//! baked into the signaling URL when the socket opens, so editing `RoomId` alone
//! changes the label and nothing else — the peer stays in the old room while the
//! screen claims otherwise. No unit test of the parser would notice.

use bevy::input::ButtonState;
use bevy::input::InputPlugin;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use checkers_bevy::AppState;
use checkers_bevy::lobby::{
    ChosenSeating, EditAction, NameEdit, RoomEdit, choose_seating, edit_action, edit_room,
    not_editing,
};
use checkers_bevy::setup::Seating;
use checkers_net::{NetState, RoomId, Seat};

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, InputPlugin, StatesPlugin))
        .init_state::<AppState>()
        // The app boots into the menu; these tests exercise lobby fields.
        .insert_state(AppState::Lobby)
        .init_resource::<RoomId>()
        .init_resource::<RoomEdit>()
        // `not_editing` reads both editors.
        .init_resource::<NameEdit>()
        .init_resource::<ChosenSeating>()
        .init_resource::<NetState>()
        .add_systems(Update, edit_room.run_if(in_state(AppState::Lobby)));
    app
}

/// Send a keypress as the window does, with layout-aware text.
fn press(app: &mut App, key: KeyCode, text: Option<&str>) {
    app.world_mut().write_message(KeyboardInput {
        key_code: key,
        logical_key: Key::Character("x".into()),
        state: ButtonState::Pressed,
        text: text.map(Into::into),
        repeat: false,
        window: Entity::PLACEHOLDER,
    });
    app.update();
}

fn type_name(app: &mut App, name: &str) {
    for c in name.chars() {
        press(app, KeyCode::KeyA, Some(&c.to_string()));
    }
}

fn open(app: &mut App) {
    let mut edit = app.world_mut().resource_mut::<RoomEdit>();
    edit.active = true;
    edit.buffer.clear();
}

#[test]
fn typing_a_name_and_committing_changes_the_room() {
    let mut app = app();
    open(&mut app);
    type_name(&mut app, "kitchen-table");
    assert_eq!(app.world().resource::<RoomEdit>().buffer, "kitchen-table");

    press(&mut app, KeyCode::Enter, None);

    assert_eq!(app.world().resource::<RoomId>().0, "kitchen-table");
    assert!(
        !app.world().resource::<RoomEdit>().active,
        "committing must close the field"
    );
}

/// The whole point: the lobby must be re-entered so `open_socket` runs again
/// against the new room. Without it the peer keeps talking to the old room while
/// the screen shows the new name.
#[test]
fn committing_re_enters_the_lobby_so_the_socket_reopens() {
    let mut app = app();
    open(&mut app);
    type_name(&mut app, "other-room");
    press(&mut app, KeyCode::Enter, None);

    assert!(
        matches!(
            app.world().resource::<NextState<AppState>>(),
            NextState::Pending(AppState::Lobby)
        ),
        "a room change must re-enter the lobby so the socket reopens"
    );
}

/// Everything the old room told us must be forgotten, or the peer arrives in the
/// new room already believing it is the host of it.
#[test]
fn changing_room_forgets_the_old_rooms_state() {
    let mut app = app();
    {
        let mut net = app.world_mut().resource_mut::<NetState>();
        net.is_host = true;
        net.next_seq = 7;
        net.last_applied_seq = Some(6);
        net.name = "ada".into();
        net.seats = vec![Seat {
            peer: "p".into(),
            name: "p".into(),
            player: Some(0),
            ready: true,
            spectate: false,
        }];
    }

    open(&mut app);
    type_name(&mut app, "elsewhere");
    press(&mut app, KeyCode::Enter, None);

    let net = app.world().resource::<NetState>();
    assert!(!net.is_host, "host status belonged to the old room");
    assert!(net.seats.is_empty(), "seats were assigned by the old host");
    assert_eq!(net.next_seq, 0);
    assert_eq!(net.last_applied_seq, None);
    assert_eq!(net.name, "ada", "the player's own name is not per-room");
}

#[test]
fn escape_abandons_the_edit_and_keeps_the_room() {
    let mut app = app();
    let before = app.world().resource::<RoomId>().0.clone();

    open(&mut app);
    type_name(&mut app, "typo");
    press(&mut app, KeyCode::Escape, None);

    assert_eq!(
        app.world().resource::<RoomId>().0,
        before,
        "cancelling must not change the room"
    );
    assert!(!app.world().resource::<RoomEdit>().active);
}

/// An invalid name must be refused *and explained*, leaving the field open so
/// the player can correct it rather than losing what they typed.
#[test]
fn an_invalid_name_is_refused_with_a_reason() {
    let mut app = app();
    let before = app.world().resource::<RoomId>().0.clone();

    open(&mut app);
    type_name(&mut app, "bad/name");
    press(&mut app, KeyCode::Enter, None);

    let edit = app.world().resource::<RoomEdit>();
    assert!(edit.active, "the field must stay open to be corrected");
    assert!(!edit.error.is_empty(), "the refusal must be explained");
    assert!(
        edit.error.contains('/'),
        "must name the character: {}",
        edit.error
    );
    assert_eq!(edit.buffer, "bad/name", "what was typed must survive");
    assert_eq!(
        app.world().resource::<RoomId>().0,
        before,
        "a refused name must not change the room"
    );
}

#[test]
fn backspace_deletes_the_last_character() {
    let mut app = app();
    open(&mut app);
    type_name(&mut app, "abc");
    press(&mut app, KeyCode::Backspace, None);
    assert_eq!(app.world().resource::<RoomEdit>().buffer, "ab");
}

/// The buffer must not grow past what `parse` accepts, so the player is stopped
/// at the limit rather than told afterwards that it is all too long.
#[test]
fn the_buffer_stops_at_the_length_limit() {
    let mut app = app();
    open(&mut app);
    type_name(&mut app, &"a".repeat(RoomId::MAX_LEN + 10));
    assert_eq!(
        app.world().resource::<RoomEdit>().buffer.chars().count(),
        RoomId::MAX_LEN
    );
}

/// Keys must not reach the field when it is closed, or a keypress meant for the
/// lobby turns up in a name typed later.
#[test]
fn keys_are_ignored_while_the_field_is_closed() {
    let mut app = app();
    assert!(!app.world().resource::<RoomEdit>().active);
    type_name(&mut app, "ghost");
    assert!(
        app.world().resource::<RoomEdit>().buffer.is_empty(),
        "a closed field must not accumulate text"
    );
}

/// Committing the room already joined must not tear down a working session.
#[test]
fn committing_the_same_room_does_not_rejoin() {
    let mut app = app();
    let current = app.world().resource::<RoomId>().0.clone();
    app.world_mut().resource_mut::<NetState>().is_host = true;

    open(&mut app);
    type_name(&mut app, &current);
    press(&mut app, KeyCode::Enter, None);

    assert_eq!(app.world().resource::<RoomId>().0, current);
    assert!(
        app.world().resource::<NetState>().is_host,
        "re-committing the same room must not reset the session"
    );
    assert!(
        matches!(
            app.world().resource::<NextState<AppState>>(),
            NextState::Unchanged
        ),
        "no state transition for a no-op change"
    );
}

/// `edit_action` and the system must agree; a divergence would make the unit
/// tests describe behaviour the app does not have.
#[test]
fn the_classifier_matches_what_the_system_does() {
    assert_eq!(
        edit_action(KeyCode::KeyQ, Some("q")),
        EditAction::Insert('q')
    );
    let mut app = app();
    open(&mut app);
    press(&mut app, KeyCode::KeyQ, Some("q"));
    assert_eq!(app.world().resource::<RoomEdit>().buffer, "q");
}

/// The lobby's own systems, in the order the plugin chains them, so a keypress
/// the field handled cannot also be seen by the systems that run after it.
fn chained_app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, InputPlugin, StatesPlugin))
        .init_state::<AppState>()
        // The app boots into the menu; these tests exercise lobby fields.
        .insert_state(AppState::Lobby)
        .init_resource::<RoomId>()
        .init_resource::<RoomEdit>()
        // `not_editing` reads both editors.
        .init_resource::<NameEdit>()
        .init_resource::<ChosenSeating>()
        .init_resource::<NetState>()
        .add_systems(
            Update,
            (
                edit_room,
                // The *real* run condition, not a copy of it. An inline
                // duplicate here made this test pass with the guard removed
                // from the app -- it was checking its own logic.
                choose_seating.run_if(not_editing),
            )
                .chain()
                .run_if(in_state(AppState::Lobby)),
        );
    app
}

/// A keypress the room field consumed must not reach the systems that run after
/// it in the same frame.
///
/// Found by running the app, not by testing it. Committing with Enter cleared
/// `active`, and because the lobby systems are `.chain()`ed, the *same* Enter
/// fell through to `handle_buttons` and started the game — typing a room name
/// dropped straight onto the board.
///
/// Two things this test had to get right before it could see the bug, both of
/// which I got wrong first:
///
/// - **Send only the message.** `edit_room` reads `KeyboardInput` messages;
///   downstream systems read the `ButtonInput` resource. `InputPlugin` derives
///   the resource from the message in `PreUpdate`, so one message gives both. My
///   first version *also* called `press()`, and `keyboard_input_system` begins by
///   clearing `just_pressed` — so the downstream system saw nothing and the test
///   passed with the guard removed.
/// - **Test the commit frame.** While a character is being typed the field stays
///   open, so `!active` alone already suppresses everything downstream. The guard
///   only matters on the one frame the field closes.
#[test]
fn the_committing_keypress_does_not_leak_downstream() {
    /// Stands in for `handle_buttons`, which needs a socket. Records whether the
    /// Enter that committed the room was also visible after the field closed.
    #[derive(Resource, Default)]
    struct SawEnter(bool);

    fn downstream(keys: Res<ButtonInput<KeyCode>>, mut saw: ResMut<SawEnter>) {
        if keys.just_pressed(KeyCode::Enter) {
            saw.0 = true;
        }
    }

    let mut app = App::new();
    app.add_plugins((MinimalPlugins, InputPlugin, StatesPlugin))
        .init_state::<AppState>()
        // The app boots into the menu; these tests exercise lobby fields.
        .insert_state(AppState::Lobby)
        .init_resource::<RoomId>()
        .init_resource::<RoomEdit>()
        // `not_editing` reads both editors.
        .init_resource::<NameEdit>()
        .init_resource::<ChosenSeating>()
        .init_resource::<NetState>()
        .init_resource::<SawEnter>()
        // `not_editing` itself, not a copy: an inline duplicate of the condition
        // would keep passing after the real one was weakened.
        .add_systems(
            Update,
            (edit_room, downstream.run_if(not_editing))
                .chain()
                .run_if(in_state(AppState::Lobby)),
        );

    {
        let mut edit = app.world_mut().resource_mut::<RoomEdit>();
        edit.active = true;
        edit.buffer = "kitchen".into();
    }

    app.world_mut().write_message(KeyboardInput {
        key_code: KeyCode::Enter,
        logical_key: Key::Enter,
        state: ButtonState::Pressed,
        text: None,
        repeat: false,
        window: Entity::PLACEHOLDER,
    });
    app.update();

    assert_eq!(
        app.world().resource::<RoomId>().0,
        "kitchen",
        "the commit itself must still work"
    );
    assert!(
        !app.world().resource::<SawEnter>().0,
        "the Enter that committed the room must not also reach the lobby"
    );
}

/// The guard must not outlive its frame, or the lobby stays deaf after any edit
/// — trading a leak for a lockout.
#[test]
fn the_guard_clears_on_the_next_frame() {
    let mut app = chained_app();
    open(&mut app);
    press(&mut app, KeyCode::Enter, None);
    assert!(app.world().resource::<RoomEdit>().consumed_input);

    // A frame with no input at all.
    app.update();
    assert!(
        !app.world().resource::<RoomEdit>().consumed_input,
        "the guard must not persist past its frame"
    );
}

/// A digit typed into the field is a character, not a seating change.
#[test]
fn a_digit_typed_into_the_field_does_not_change_the_seating() {
    let mut app = chained_app();
    open(&mut app);
    // Message only, so `PreUpdate` fills the resource the way the window would.
    app.world_mut().write_message(KeyboardInput {
        key_code: KeyCode::Digit3,
        logical_key: Key::Character("3".into()),
        state: ButtonState::Pressed,
        text: Some("3".into()),
        repeat: false,
        window: Entity::PLACEHOLDER,
    });
    app.update();

    assert_eq!(app.world().resource::<RoomEdit>().buffer, "3");
    assert_eq!(
        app.world().resource::<ChosenSeating>().0,
        Seating::default(),
        "the digit belonged to the field"
    );
}
