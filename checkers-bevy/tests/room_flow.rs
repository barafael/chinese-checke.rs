//! The room and name fields end to end: typing, validation, committing, and
//! actually rejoining.
//!
//! The failure this guards is a control that *looks* like it works. The room is
//! baked into the signaling URL when the socket opens, so editing `RoomId` alone
//! changes the label and nothing else — the peer stays in the old room while the
//! screen claims otherwise. No unit test of the parser would notice.

mod common;

use bevy::input::ButtonState;
use bevy::prelude::*;
use checkers_bevy::AppState;
use checkers_bevy::lobby::{
    ApplyName, EditAction, FieldEdit, FieldKind, LobbyStatus, NAME_MAX_LEN, PendingClaim,
    SelectedCorner, apply_name, edit_action, fields_plugin, not_editing, select_corner,
};
use checkers_bevy::sound::{self, SoundOn};
use checkers_net::{NetState, RoomId, Seat};
use common::{key_message, net, set_state};

/// How many times the lobby has been entered — where the app opens the room's
/// socket.
#[derive(Resource, Default)]
struct LobbyEntries(u32);

/// The fields as the app registers them ([`fields_plugin`]), over the
/// resources they commit to.
fn app() -> App {
    let mut app = common::headless_app();
    app.insert_resource(RoomId::parse("room-1").expect("a valid room parses"))
        .init_resource::<NetState>()
        .init_resource::<LobbyStatus>()
        .init_resource::<LobbyEntries>()
        .add_systems(
            OnEnter(AppState::Lobby),
            |mut entries: ResMut<LobbyEntries>| entries.0 += 1,
        )
        .add_plugins(fields_plugin);
    // The first frame enters the lobby the app boots into.
    app.update();
    app
}

fn lobby_entries(app: &App) -> u32 {
    app.world().resource::<LobbyEntries>().0
}

/// The key a character is typed with, for the letters that mean something
/// elsewhere in the app; any other character comes from an innocent key.
fn key_of(c: char) -> KeyCode {
    match c {
        'm' => KeyCode::KeyM,
        's' => KeyCode::KeyS,
        'r' => KeyCode::KeyR,
        'n' => KeyCode::KeyN,
        '3' => KeyCode::Digit3,
        _ => KeyCode::KeyA,
    }
}

/// Send a keypress and run a frame.
///
/// Message only, never `ButtonInput::press`: `InputPlugin` derives the
/// resource from the message in `PreUpdate`, exactly as for the window, so one
/// message feeds both the field (which reads messages) and every system that
/// reads the resource.
fn press(app: &mut App, key: KeyCode, text: Option<&str>) {
    app.world_mut()
        .write_message(key_message(key, ButtonState::Pressed, text));
    app.update();
}

fn type_into(app: &mut App, text: &str) {
    for c in text.chars() {
        press(app, key_of(c), Some(&c.to_string()));
    }
}

/// Several keypresses arriving in one frame, as a fast typist's do.
fn one_frame(app: &mut App, keys: &[(KeyCode, Option<&str>)]) {
    for (key, text) in keys {
        app.world_mut()
            .write_message(key_message(*key, ButtonState::Pressed, *text));
    }
    app.update();
}

fn open(app: &mut App, kind: FieldKind) {
    app.world_mut().resource_mut::<FieldEdit>().open(kind, "");
}

/// Open a field, type `text` into it and press Enter.
fn commit(app: &mut App, kind: FieldKind, text: &str) {
    open(app, kind);
    type_into(app, text);
    press(app, KeyCode::Enter, None);
}

fn focus(app: &App) -> Option<FieldKind> {
    app.world().resource::<FieldEdit>().focus
}

fn buffer(app: &App) -> &str {
    &app.world().resource::<FieldEdit>().buffer
}

fn error(app: &App) -> &str {
    &app.world().resource::<FieldEdit>().error
}

fn room(app: &App) -> &str {
    &app.world().resource::<RoomId>().0
}

/// A refused commit leaves the field open on `kind`, and says why.
fn assert_refused(app: &App, kind: FieldKind) {
    assert_eq!(
        focus(app),
        Some(kind),
        "the field must stay open to be corrected"
    );
    assert!(!error(app).is_empty(), "the refusal must be explained");
}

#[test]
fn typing_a_room_and_committing_changes_the_room() {
    let mut app = app();
    open(&mut app, FieldKind::Room);
    type_into(&mut app, "kitchen-table");
    assert_eq!(buffer(&app), "kitchen-table");

    press(&mut app, KeyCode::Enter, None);

    assert_eq!(room(&app), "kitchen-table");
    assert_eq!(focus(&app), None, "committing must close the field");
    assert!(buffer(&app).is_empty(), "the buffer is spent once applied");
}

/// The whole point: the lobby must be re-entered so `open_socket` runs again
/// against the new room. Without it the peer keeps talking to the old room while
/// the screen shows the new name.
#[test]
fn committing_re_enters_the_lobby_so_the_socket_reopens() {
    let mut app = app();
    let before = lobby_entries(&app);
    commit(&mut app, FieldKind::Room, "other-room");

    assert_eq!(
        lobby_entries(&app),
        before + 1,
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
        net.seats = vec![Seat::human("p", "p", Some(0))];
    }

    commit(&mut app, FieldKind::Room, "elsewhere");

    let net = net(&app);
    assert!(!net.is_host, "host status belonged to the old room");
    assert!(net.seats.is_empty(), "seats were assigned by the old host");
    assert_eq!(net.next_seq, 0);
    assert_eq!(net.last_applied_seq, None);
    assert_eq!(net.name, "ada", "the player's own name is not per-room");
}

#[test]
fn escape_abandons_the_edit_and_keeps_the_room() {
    let mut app = app();
    let before = room(&app).to_string();

    open(&mut app, FieldKind::Room);
    type_into(&mut app, "typo");
    press(&mut app, KeyCode::Escape, None);

    assert_eq!(room(&app), before, "cancelling must not change the room");
    assert_eq!(focus(&app), None);
}

/// An invalid room must be refused *and explained*, leaving the field open so
/// the player can correct it rather than losing what they typed.
#[test]
fn an_invalid_room_is_refused_with_a_reason() {
    let mut app = app();
    let before = room(&app).to_string();

    commit(&mut app, FieldKind::Room, "bad/room");

    assert_refused(&app, FieldKind::Room);
    let error = error(&app);
    assert!(error.contains('/'), "must name the character: {error}");
    assert_eq!(buffer(&app), "bad/room", "what was typed must survive");
    assert_eq!(
        room(&app),
        before,
        "a refused room must not change the room"
    );
}

#[test]
fn backspace_deletes_the_last_character() {
    let mut app = app();
    open(&mut app, FieldKind::Room);
    type_into(&mut app, "abc");
    press(&mut app, KeyCode::Backspace, None);
    assert_eq!(buffer(&app), "ab");
}

/// The buffer must not grow past what `parse` accepts, so the player is stopped
/// at the limit rather than told afterwards that it is all too long.
#[test]
fn the_buffer_stops_at_the_length_limit() {
    let mut app = app();
    open(&mut app, FieldKind::Room);
    type_into(&mut app, &"a".repeat(RoomId::MAX_LEN + 10));
    assert_eq!(buffer(&app).chars().count(), RoomId::MAX_LEN);
}

/// Keys must not reach the field when it is closed, or a keypress meant for the
/// lobby turns up in a room typed later.
#[test]
fn keys_are_ignored_while_the_room_field_is_closed() {
    let mut app = app();
    assert_eq!(focus(&app), None);
    type_into(&mut app, "ghost");
    assert!(
        buffer(&app).is_empty(),
        "a closed field must not accumulate text"
    );
}

/// Committing the room already joined must not tear down a working session.
#[test]
fn committing_the_same_room_does_not_rejoin() {
    let mut app = app();
    let current = room(&app).to_string();
    let before = lobby_entries(&app);
    app.world_mut().resource_mut::<NetState>().is_host = true;

    commit(&mut app, FieldKind::Room, &current);

    assert_eq!(room(&app), current);
    assert!(
        net(&app).is_host,
        "re-committing the same room must not reset the session"
    );
    assert_eq!(
        lobby_entries(&app),
        before,
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
    open(&mut app, FieldKind::Room);
    press(&mut app, KeyCode::KeyQ, Some("q"));
    assert_eq!(buffer(&app), "q");
}

/// A field that closes stops reading. Esc, then `a`, then Enter in one frame
/// used to cancel, type into the closed field, and commit it: the player
/// joined room "a" right after cancelling.
#[test]
fn keys_after_escape_in_the_same_frame_are_dropped() {
    let mut app = app();
    let before = room(&app).to_string();
    open(&mut app, FieldKind::Room);
    one_frame(
        &mut app,
        &[
            (KeyCode::Escape, None),
            (KeyCode::KeyA, Some("a")),
            (KeyCode::Enter, None),
        ],
    );
    assert_eq!(focus(&app), None);
    assert_eq!(room(&app), before, "the cancelled field joined a room");
    assert!(buffer(&app).is_empty(), "a closed field took more keys");
}

/// Enter, `x`, Enter in one frame changes the room once, to what was typed
/// before the first Enter — not twice, leaving the room it just joined.
#[test]
fn keys_after_a_commit_in_the_same_frame_are_dropped() {
    let mut app = app();
    open(&mut app, FieldKind::Room);
    type_into(&mut app, "kitchen");
    one_frame(
        &mut app,
        &[
            (KeyCode::Enter, None),
            (KeyCode::KeyX, Some("x")),
            (KeyCode::Enter, None),
        ],
    );
    assert_eq!(room(&app), "kitchen");
    assert_eq!(focus(&app), None);
    assert!(buffer(&app).is_empty(), "a closed field took more keys");
}

/// A field left open when the game starts is closed, not waiting focused in
/// the lobby the player comes back to, where it would swallow the lobby's keys.
#[test]
fn leaving_the_lobby_closes_the_field() {
    let mut app = app();
    open(&mut app, FieldKind::Room);
    type_into(&mut app, "half");

    set_state(&mut app, AppState::InGame);
    app.update();
    set_state(&mut app, AppState::Lobby);
    app.update();

    assert_eq!(focus(&app), None, "the field must not survive the round");
    assert!(buffer(&app).is_empty());
}

/// The lobby's own systems as the plugin chains them, so a keypress the field
/// handled cannot also be seen by the systems that run after it.
fn chained_app() -> App {
    let mut app = app();
    app.init_resource::<SelectedCorner>()
        .init_resource::<PendingClaim>()
        .add_systems(
            Update,
            // The *real* run condition, not a copy of it. An inline duplicate here
            // made an earlier version of this test pass with the guard removed
            // from the app -- it was checking its own logic.
            select_corner
                .run_if(not_editing)
                .run_if(in_state(AppState::Lobby)),
        );
    app
}

/// A keypress the field consumed must not reach any system that reads the
/// keyboard.
///
/// Found by running the app, not by testing it. Committing with Enter closed
/// the field, and the *same* Enter fell through to `handle_buttons` and
/// started the game — typing a room name dropped straight onto the board.
///
/// The system that stands in for the rest of the app runs unconditionally: the
/// field clears the presses it takes, so no reader needs a guard of its own.
/// Two things matter for the test to see the bug: send only the message (see
/// [`press`]), and test the commit frame — while a character is being typed
/// the field is still open, and that frame is easy.
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

    let mut app = app();
    app.init_resource::<SawEnter>()
        .add_systems(Update, downstream);
    app.world_mut()
        .resource_mut::<FieldEdit>()
        .open(FieldKind::Room, "kitchen");

    press(&mut app, KeyCode::Enter, None);

    assert_eq!(room(&app), "kitchen", "the commit itself must still work");
    assert!(
        !app.world().resource::<SawEnter>().0,
        "the Enter that committed the room must not also reach the lobby"
    );
}

/// The keyboard belongs to the field for exactly as long as it is open, or the
/// lobby stays deaf after any edit — trading a leak for a lockout.
#[test]
fn a_closed_field_gives_the_keys_back() {
    let mut app = chained_app();
    open(&mut app, FieldKind::Room);
    press(&mut app, KeyCode::Escape, None);
    press(&mut app, KeyCode::Digit3, Some("3"));
    assert_eq!(
        app.world().resource::<SelectedCorner>().0,
        Some(2),
        "the next digit selects its corner again"
    );
}

/// A digit typed into the field is a character, not a corner selection.
#[test]
fn a_digit_typed_into_the_room_field_does_not_select_a_corner() {
    let mut app = chained_app();
    open(&mut app, FieldKind::Room);
    press(&mut app, KeyCode::Digit3, Some("3"));

    assert_eq!(buffer(&app), "3");
    assert_eq!(
        app.world().resource::<SelectedCorner>().0,
        None,
        "the digit belonged to the field"
    );
}

/// The app-wide shortcuts are deaf while a field holds the keyboard. The sound
/// toggle is not a lobby system at all, and it used to flip once per `m` in a
/// typed name — the default pet names are full of them.
#[test]
fn typing_does_not_toggle_the_sound() {
    let mut app = app();
    app.init_resource::<SoundOn>()
        .add_systems(Update, sound::toggle);
    open(&mut app, FieldKind::Name);
    type_into(&mut app, "mimosa");

    assert_eq!(buffer(&app), "mimosa");
    assert!(
        !app.world().resource::<SoundOn>().0,
        "the letters were the field's, not the sound toggle's"
    );
}

// ---------------------------------------------------------------------------
// The name field and its Apply button.
// ---------------------------------------------------------------------------

#[test]
fn typing_a_name_and_committing_changes_the_name_and_re_greets() {
    let mut app = app();
    commit(&mut app, FieldKind::Name, "ida");

    assert_eq!(net(&app).name, "ida");
    assert_eq!(focus(&app), None, "committing must close the field");
    assert!(buffer(&app).is_empty(), "the buffer is spent once applied");
}

/// A name has its own limit, not the room's.
#[test]
fn the_name_stops_at_its_own_limit() {
    let mut app = app();
    open(&mut app, FieldKind::Name);
    type_into(&mut app, &"a".repeat(NAME_MAX_LEN + 10));
    assert_eq!(buffer(&app).chars().count(), NAME_MAX_LEN);
}

fn apply_app() -> App {
    let mut app = app();
    app.add_systems(Update, apply_name.run_if(in_state(AppState::Lobby)));
    app
}

fn click_apply(app: &mut App) {
    app.world_mut().spawn((ApplyName, Interaction::Pressed));
    app.update();
}

/// The button row: Apply sits next to the field. A click on a field that is not
/// editing focuses it (seeding the buffer), so a click to fix a typo does not
/// wipe the current name first.
#[test]
fn applying_with_the_field_closed_focuses_it() {
    let mut app = apply_app();
    app.world_mut().resource_mut::<NetState>().name = "ada".into();
    click_apply(&mut app);

    assert_eq!(
        focus(&app),
        Some(FieldKind::Name),
        "the click opens the field"
    );
    assert_eq!(buffer(&app), "ada", "the current name seeds the buffer");
    assert_eq!(net(&app).name, "ada");
}

/// The real service: a second click, with the field open, commits.
#[test]
fn applying_with_the_field_open_commits() {
    let mut app = apply_app();
    open(&mut app, FieldKind::Name);
    click_apply(&mut app);

    // It did commit: the empty name was refused.
    assert_refused(&app, FieldKind::Name);
}

/// A name someone else in the room already has is refused: the roster, the
/// corner labels and the cursors could not tell the two apart.
#[test]
fn a_name_already_here_is_refused() {
    let mut app = app();
    app.world_mut().resource_mut::<NetState>().seats =
        vec![Seat::human("someone-else", "ada", None)];
    commit(&mut app, FieldKind::Name, "ada");

    assert_refused(&app, FieldKind::Name);
    assert!(error(&app).contains("already"), "error: {}", error(&app));
    assert_ne!(net(&app).name, "ada");
}

/// An empty name must be refused, keeping the field and what was typed, rather
/// than erasing the player's name in the roster.
#[test]
fn an_empty_name_is_refused() {
    let mut app = app();
    commit(&mut app, FieldKind::Name, "   ");

    assert_refused(&app, FieldKind::Name);
}
