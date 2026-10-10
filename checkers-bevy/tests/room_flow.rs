//! The room and name fields end to end: typing, validation, committing, and
//! actually rejoining.
//!
//! The failure this guards is a control that *looks* like it works. The room is
//! baked into the signaling URL when the socket opens, so editing `RoomId` alone
//! changes the label and nothing else — the peer stays in the old room while the
//! screen claims otherwise. No unit test of the parser would notice.
//!
//! The fields are Bevy's own text input, spawned from the lobby's own scene,
//! and the keys reach them the way the window's do: as key messages, dispatched
//! to whichever field holds the keyboard.

mod common;

use bevy::input::ButtonState;
use bevy::input_focus::InputFocus;
use bevy::prelude::*;
use bevy::text::EditableText;
use checkers_bevy::AppState;
use checkers_bevy::lobby::{
    ApplyName, FieldKind, FieldRefusal, InputField, LobbyStatus, NAME_MAX_LEN, PendingClaim,
    SelectedCorner, clicks_plugin, fields_plugin, input_field, select_corner,
};
use checkers_bevy::sound::{self, SoundOn};
use checkers_net::{NetState, RoomId, Seat};
use common::{key_message, net, set_state};

/// How many times the lobby has been entered — where the app opens the room's
/// socket.
#[derive(Resource, Default)]
struct LobbyEntries(u32);

/// Both fields as the lobby spawns them, under the plugin the app registers
/// them with ([`fields_plugin`]), over the resources they commit to.
fn app() -> App {
    let mut app = common::headless_app();
    common::text_input(&mut app);
    app.insert_resource(RoomId::parse("room-1").expect("a valid room parses"))
        .init_resource::<NetState>()
        .init_resource::<LobbyStatus>()
        .init_resource::<LobbyEntries>()
        .add_systems(
            OnEnter(AppState::Lobby),
            |mut entries: ResMut<LobbyEntries>| entries.0 += 1,
        )
        .add_plugins(fields_plugin);
    app.world_mut().resource_mut::<NetState>().name = "ada".into();
    for kind in [FieldKind::Room, FieldKind::Name] {
        app.world_mut()
            .spawn_scene(input_field(kind))
            .expect("the field spawns");
    }
    // The first frame enters the lobby the app boots into, and fills the
    // fields with the committed values.
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

/// Press and release a key within a frame, and run the frame.
///
/// Messages only, never `ButtonInput::press`: `InputPlugin` derives the
/// resource from the messages in `PreUpdate`, exactly as for the window, so one
/// message feeds both the field (which is handed the messages) and every
/// system that reads the resource.
fn press(app: &mut App, key: KeyCode, text: Option<&str>) {
    one_frame(app, &[(key, text)]);
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
        app.world_mut()
            .write_message(key_message(*key, ButtonState::Released, None));
    }
    app.update();
}

fn field(app: &mut App, kind: FieldKind) -> Entity {
    let mut fields = app.world_mut().query::<(Entity, &InputField)>();
    fields
        .iter(app.world())
        .find(|(_, f)| f.0 == kind)
        .map(|(e, _)| e)
        .expect("both fields are spawned")
}

/// Focus a field with its key, as the player does: `R` the room, `N` the name.
fn open(app: &mut App, kind: FieldKind) {
    let key = match kind {
        FieldKind::Room => KeyCode::KeyR,
        FieldKind::Name => KeyCode::KeyN,
    };
    press(app, key, Some(&format!("{key:?}")[3..].to_lowercase()));
    assert_eq!(focus(app), Some(kind), "the key focuses its field");
}

/// Empty the focused field, as select-all and delete would.
fn clear(app: &mut App, kind: FieldKind) {
    let field = field(app, kind);
    app.world_mut()
        .get_mut::<EditableText>(field)
        .expect("a field is editable text")
        .clear();
    app.update();
}

/// Open a field, replace its text with `text` and press Enter.
fn commit(app: &mut App, kind: FieldKind, text: &str) {
    open(app, kind);
    clear(app, kind);
    type_into(app, text);
    press(app, KeyCode::Enter, None);
}

/// Which field holds the keyboard.
fn focus(app: &mut App) -> Option<FieldKind> {
    let focused = app.world().resource::<InputFocus>().get()?;
    app.world().get::<InputField>(focused).map(|f| f.0)
}

/// What a field shows.
fn text(app: &mut App, kind: FieldKind) -> String {
    let field = field(app, kind);
    app.world()
        .get::<EditableText>(field)
        .expect("a field is editable text")
        .value()
        .to_string()
}

fn error(app: &App) -> String {
    app.world()
        .resource::<FieldRefusal>()
        .0
        .as_ref()
        .map(|(_, why)| why.clone())
        .unwrap_or_default()
}

fn room(app: &App) -> &str {
    &app.world().resource::<RoomId>().0
}

/// A refused commit leaves the field focused on `kind`, and says why.
fn assert_refused(app: &mut App, kind: FieldKind) {
    assert_eq!(
        focus(app),
        Some(kind),
        "the field must keep the keyboard to be corrected"
    );
    assert!(!error(app).is_empty(), "the refusal must be explained");
}

#[test]
fn a_field_shows_the_committed_value() {
    let mut app = app();
    assert_eq!(text(&mut app, FieldKind::Room), "room-1");
    assert_eq!(text(&mut app, FieldKind::Name), "ada");
}

#[test]
fn typing_a_room_and_committing_changes_the_room() {
    let mut app = app();
    open(&mut app, FieldKind::Room);
    clear(&mut app, FieldKind::Room);
    type_into(&mut app, "kitchen-table");
    assert_eq!(text(&mut app, FieldKind::Room), "kitchen-table");

    press(&mut app, KeyCode::Enter, None);

    assert_eq!(room(&app), "kitchen-table");
    assert_eq!(focus(&mut app), None, "committing lets go of the keyboard");
}

/// Typing goes where the caret is: a field opened with its key keeps its
/// value, so a small change does not mean retyping the whole thing.
#[test]
fn typing_extends_the_current_value() {
    let mut app = app();
    open(&mut app, FieldKind::Room);
    type_into(&mut app, "-b");
    press(&mut app, KeyCode::Enter, None);
    assert_eq!(room(&app), "room-1-b");
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

/// Esc leaves the field and drops the edit: the room stays, and the field
/// shows it again.
#[test]
fn escape_abandons_the_edit_and_keeps_the_room() {
    let mut app = app();

    open(&mut app, FieldKind::Room);
    type_into(&mut app, "typo");
    press(&mut app, KeyCode::Escape, None);
    app.update();

    assert_eq!(room(&app), "room-1", "cancelling must not change the room");
    assert_eq!(focus(&mut app), None);
    assert_eq!(text(&mut app, FieldKind::Room), "room-1");
}

/// An invalid room must be refused *and explained*, leaving the field focused
/// so the player can correct it rather than losing what they typed.
#[test]
fn an_invalid_room_is_refused_with_a_reason() {
    let mut app = app();

    commit(&mut app, FieldKind::Room, "bad/room");

    assert_refused(&mut app, FieldKind::Room);
    let error = error(&app);
    assert!(error.contains('/'), "must name the character: {error}");
    assert_eq!(
        text(&mut app, FieldKind::Room),
        "bad/room",
        "what was typed must survive"
    );
    assert_eq!(
        room(&app),
        "room-1",
        "a refused room must not change the room"
    );
}

/// The refusal belongs to the attempt: once the field lets go, it is gone.
#[test]
fn leaving_a_refused_field_clears_the_refusal() {
    let mut app = app();
    commit(&mut app, FieldKind::Room, "bad/room");
    press(&mut app, KeyCode::Escape, None);
    assert!(error(&app).is_empty());
}

/// The field must not grow past what `parse` accepts, so the player is stopped
/// at the limit rather than told afterwards that it is all too long.
#[test]
fn the_room_stops_at_the_length_limit() {
    let mut app = app();
    open(&mut app, FieldKind::Room);
    clear(&mut app, FieldKind::Room);
    type_into(&mut app, &"a".repeat(RoomId::MAX_LEN + 10));
    assert_eq!(
        text(&mut app, FieldKind::Room).chars().count(),
        RoomId::MAX_LEN
    );
}

/// Keys must not reach a field nobody focused, or a keypress meant for the
/// lobby turns up in the room.
#[test]
fn keys_are_ignored_while_no_field_is_focused() {
    let mut app = app();
    assert_eq!(focus(&mut app), None);
    type_into(&mut app, "ghost");
    assert_eq!(text(&mut app, FieldKind::Room), "room-1");
    assert_eq!(text(&mut app, FieldKind::Name), "ada");
}

/// Committing the room already joined must not tear down a working session.
#[test]
fn committing_the_same_room_does_not_rejoin() {
    let mut app = app();
    let before = lobby_entries(&app);
    app.world_mut().resource_mut::<NetState>().is_host = true;

    commit(&mut app, FieldKind::Room, "room-1");

    assert_eq!(room(&app), "room-1");
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

/// A field that lets go stops committing. Esc, then `a`, then Enter in one
/// frame must not cancel and then join room "...a" anyway: the frame's keys
/// are all handed to the field that held the keyboard when it began.
#[test]
fn keys_after_escape_in_the_same_frame_are_dropped() {
    let mut app = app();
    open(&mut app, FieldKind::Room);
    one_frame(
        &mut app,
        &[
            (KeyCode::Escape, None),
            (KeyCode::KeyA, Some("a")),
            (KeyCode::Enter, None),
        ],
    );
    app.update();
    assert_eq!(focus(&mut app), None);
    assert_eq!(room(&app), "room-1", "the cancelled field joined a room");
    assert_eq!(text(&mut app, FieldKind::Room), "room-1");
}

/// Enter, `x`, Enter in one frame changes the room once, to what was typed
/// before the first Enter — not twice, leaving the room it just joined.
#[test]
fn keys_after_a_commit_in_the_same_frame_are_dropped() {
    let mut app = app();
    open(&mut app, FieldKind::Room);
    clear(&mut app, FieldKind::Room);
    type_into(&mut app, "kitchen");
    one_frame(
        &mut app,
        &[
            (KeyCode::Enter, None),
            (KeyCode::KeyX, Some("x")),
            (KeyCode::Enter, None),
        ],
    );
    app.update();
    assert_eq!(room(&app), "kitchen");
    assert_eq!(focus(&mut app), None);
    assert_eq!(text(&mut app, FieldKind::Room), "kitchen");
}

/// A field focused when the game starts lets go of the keyboard, rather than
/// holding the keys of the lobby the player comes back to.
#[test]
fn leaving_the_lobby_releases_the_field() {
    let mut app = app();
    open(&mut app, FieldKind::Room);
    type_into(&mut app, "half");

    set_state(&mut app, AppState::InGame);
    app.update();
    set_state(&mut app, AppState::Lobby);
    app.update();

    assert_eq!(
        focus(&mut app),
        None,
        "the field must not survive the round"
    );
    assert_eq!(room(&app), "room-1");
}

/// The field's plugin with the corner selection beside it, so a keypress the
/// field took can be seen not to reach a system that reads the keyboard.
fn chained_app() -> App {
    let mut app = app();
    app.init_resource::<SelectedCorner>()
        .init_resource::<PendingClaim>()
        .add_plugins(clicks_plugin)
        // Unguarded, as the app runs it: the field clears the presses it
        // takes, so no reader needs a guard of its own.
        .add_systems(Update, select_corner.run_if(in_state(AppState::Lobby)));
    app
}

/// A keypress the field took must not reach any system that reads the
/// keyboard.
///
/// Found by running the app, not by testing it. Committing with Enter closed
/// the field, and the *same* Enter fell through to `handle_buttons` and
/// started the game — typing a room name dropped straight onto the board.
/// Two things matter for the test to see the bug: send only the message (see
/// [`press`]), and test the commit frame — while a character is being typed
/// the field is still focused, and that frame is easy.
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
    commit(&mut app, FieldKind::Room, "kitchen");

    assert_eq!(room(&app), "kitchen", "the commit itself must still work");
    assert!(
        !app.world().resource::<SawEnter>().0,
        "the Enter that committed the room must not also reach the lobby"
    );
}

/// The keyboard belongs to the field for exactly as long as it is focused, or
/// the lobby stays deaf after any edit — trading a leak for a lockout.
#[test]
fn a_released_field_gives_the_keys_back() {
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

    assert_eq!(text(&mut app, FieldKind::Room), "room-13");
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
    clear(&mut app, FieldKind::Name);
    type_into(&mut app, "mimosa");

    assert_eq!(text(&mut app, FieldKind::Name), "mimosa");
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
    assert_eq!(focus(&mut app), None, "committing lets go of the keyboard");
}

/// A name has its own limit, not the room's.
#[test]
fn the_name_stops_at_its_own_limit() {
    let mut app = app();
    open(&mut app, FieldKind::Name);
    clear(&mut app, FieldKind::Name);
    type_into(&mut app, &"a".repeat(NAME_MAX_LEN + 10));
    assert_eq!(
        text(&mut app, FieldKind::Name).chars().count(),
        NAME_MAX_LEN
    );
}

fn click_apply(app: &mut App) {
    common::activate(app, ApplyName);
}

/// The button row: Apply sits next to the field. A click with the field not
/// focused focuses it, current name and all, so a click to fix a typo does
/// not wipe the name first.
#[test]
fn applying_with_the_field_unfocused_focuses_it() {
    let mut app = app();
    click_apply(&mut app);

    assert_eq!(
        focus(&mut app),
        Some(FieldKind::Name),
        "the click focuses the field"
    );
    assert_eq!(text(&mut app, FieldKind::Name), "ada");
    assert_eq!(net(&app).name, "ada");
}

/// The real service: a click with the field focused commits what it holds.
#[test]
fn applying_with_the_field_focused_commits() {
    let mut app = app();
    open(&mut app, FieldKind::Name);
    clear(&mut app, FieldKind::Name);
    type_into(&mut app, "ida");
    click_apply(&mut app);

    assert_eq!(net(&app).name, "ida");
    assert_eq!(focus(&mut app), None);
}

/// One field at a time: while the room holds the keyboard, Apply stands down
/// rather than committing a name nobody was editing.
#[test]
fn applying_while_the_room_is_focused_does_nothing() {
    let mut app = app();
    open(&mut app, FieldKind::Room);
    click_apply(&mut app);

    assert_eq!(focus(&mut app), Some(FieldKind::Room));
    assert_eq!(net(&app).name, "ada");
}

/// A name someone else in the room already has is refused: the roster, the
/// corner labels and the cursors could not tell the two apart.
#[test]
fn a_name_already_here_is_refused() {
    let mut app = app();
    app.world_mut().resource_mut::<NetState>().seats =
        vec![Seat::human("someone-else", "ida", None)];
    commit(&mut app, FieldKind::Name, "ida");

    assert_refused(&mut app, FieldKind::Name);
    assert!(error(&app).contains("already"), "error: {}", error(&app));
    assert_eq!(net(&app).name, "ada");
}

/// An empty name must be refused, keeping the field and what was typed, rather
/// than erasing the player's name in the roster.
#[test]
fn an_empty_name_is_refused() {
    let mut app = app();
    commit(&mut app, FieldKind::Name, "   ");

    assert_refused(&mut app, FieldKind::Name);
    assert_eq!(net(&app).name, "ada");
}
