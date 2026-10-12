//! The agent interface, driven the way an agent drives it: over the socket.
//!
//! No window and no renderer — the app here is the common headless harness
//! plus the real agent bridge, with a real `TcpStream` on the other end of
//! the wire. What is under test is exactly what a script outside the process
//! experiences: a request goes out, frames run, and the answer comes back
//! through the socket.
//!
//! The one thing this cannot cover is a screenshot's pixels: there is no
//! renderer, so the capture path must refuse instead of hang — asserted in
//! [`a_screenshot_without_a_renderer_refuses_instead_of_hanging`]. The live
//! capture itself is exercised by `scripts/agent_client.py` against a real
//! window.

mod common;

use bevy::input_focus::{FocusCause, InputDispatchPlugin, InputFocus, InputFocusPlugin};
use bevy::picking::input::PointerInputPlugin;
use bevy::prelude::*;
use bevy::text::TextPlugin;
use bevy::ui_widgets::{TextInput, TextInputPlugin};
use bevy::window::{ExitCondition, PrimaryWindow};
use checkers_bevy::agent;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

/// An app the agent can drive: headless, but with a real primary window
/// component, the input plugins the injected events flow through, and the
/// text stack a `text` command types into.
fn agent_app() -> (App, u16) {
    let mut app = common::headless_app();
    app.add_plugins((
        AssetPlugin::default(),
        bevy::scene::ScenePlugin,
        WindowPlugin {
            primary_window: None,
            exit_condition: ExitCondition::DontExit,
            ..default()
        },
        InputFocusPlugin,
        InputDispatchPlugin,
        TextPlugin,
        TextInputPlugin,
        // The picking core and its mouse-input feed: injected pointer events
        // flow through the same set the real app's do.
        bevy::picking::PickingPlugin,
        PointerInputPlugin,
    ))
    .add_message::<bevy::picking::events::PointerRelease>()
    .init_resource::<bevy::picking::events::PointerState>()
    .init_resource::<UiScale>()
    .init_resource::<checkers_bevy::Session>();
    app.world_mut().spawn((Window::default(), PrimaryWindow));
    let port = agent::bridge_for_tests(&mut app, 0).expect("the agent bridge binds on port 0");
    (app, port)
}

/// A socket on the other end, speaking the protocol one line at a time.
///
/// One reader for the connection's whole life: a fresh `BufReader` per read
/// would buffer a coalesced reply pair and drop everything but the first
/// line, desynchroning every pipelined exchange.
struct Client {
    stream: TcpStream,
    reader: BufReader<TcpStream>,
}

impl Client {
    fn connect(port: u16) -> Self {
        let stream = TcpStream::connect(("127.0.0.1", port)).expect("the agent socket accepts");
        // Replies are written by another thread, always within a frame or
        // two; the timeout turns "no reply yet" into an error the polling
        // callers retry, instead of a hang.
        stream
            .set_read_timeout(Some(Duration::from_millis(100)))
            .expect("a read timeout");
        let reader = BufReader::new(stream.try_clone().expect("a stream clone"));
        Self { stream, reader }
    }

    fn send(&mut self, line: &str) {
        writeln!(self.stream, "{line}").expect("the agent socket accepts a line");
        self.stream.flush().expect("flush");
    }

    fn read_line(&mut self) -> std::io::Result<String> {
        let mut line = String::new();
        self.reader.read_line(&mut line)?;
        Ok(line)
    }

    /// The next reply, waiting up to half a minute for it — running the app
    /// while it waits, because a pipelined request is only served by a frame.
    fn read(&mut self, app: &mut App) -> serde_json::Value {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            match self.read_line() {
                Ok(line) if !line.is_empty() => {
                    return serde_json::from_str(&line).expect("a reply is JSON");
                }
                Ok(_) => panic!("the game closed the connection"),
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) && Instant::now() < deadline =>
                {
                    app.update();
                }
                Err(e) => panic!("no reply from the game: {e}"),
            }
        }
    }

    fn try_read(&mut self) -> Result<serde_json::Value, std::io::Error> {
        let line = self.read_line()?;
        serde_json::from_str(&line).map_err(|e| std::io::Error::other(e.to_string()))
    }

    /// Send, run frames until the game has served the request, and read the
    /// reply. Every command is answered on the game thread, so the frames
    /// `read` runs while waiting cover them all — except screenshots, whose
    /// caller loops frames and polls `try_read`.
    fn ask(&mut self, app: &mut App, line: &str) -> serde_json::Value {
        self.send(line);
        app.update();
        self.read(app)
    }
}

/// A focused, editable text field for the `text` command to type into.
/// `TextInput` brings `EditableText` and its edit mode via required
/// components.
fn focused_field(app: &mut App) -> Entity {
    let field = app.world_mut().spawn(TextInput).id();
    app.world_mut()
        .resource_mut::<InputFocus>()
        .set(field, FocusCause::Pressed);
    field
}

fn field_text(app: &App, field: Entity) -> String {
    app.world()
        .entity(field)
        .get::<bevy::text::EditableText>()
        .expect("the field is still there")
        .editor
        .text()
        .to_string()
}

/// The one window, read only.
fn window(app: &mut App) -> Window {
    let mut query = app.world_mut().query::<&Window>();
    (*query.single(app.world()).expect("a window")).clone()
}

/// The handshake: the socket answers, and says which port it bound.
#[test]
fn ping_answers_with_the_bound_port() {
    let (mut app, port) = agent_app();
    let mut client = Client::connect(port);
    let reply = client.ask(&mut app, r#"{"cmd":"ping"}"#);
    assert_eq!(reply["ok"], true);
    assert_eq!(reply["port"], port);
}

/// `state` reports the dealt board: the default six-player table, ten pieces
/// per camp, nobody having moved.
#[test]
fn state_reports_the_starting_board() {
    let (mut app, port) = agent_app();
    let mut client = Client::connect(port);
    let reply = client.ask(&mut app, r#"{"cmd":"state"}"#);
    assert_eq!(reply["ok"], true);
    let state = &reply["state"];
    assert_eq!(state["phase"], "Lobby");
    assert_eq!(state["turn"], 0);
    assert_eq!(state["board"].as_array().expect("a board").len(), 60);
    assert_eq!(state["selection"]["kind"], "none");
    assert_eq!(state["moves_committed"], 0);
    assert_eq!(state["outcome"], serde_json::Value::Null);
    let players: Vec<u64> = state["players"]
        .as_array()
        .expect("players")
        .iter()
        .map(|p| p.as_u64().expect("a player index"))
        .collect();
    assert_eq!(players, vec![0, 1, 2, 3, 4, 5], "six camps are seated");
}

/// A click arrives as a real left-button press: `ButtonInput` reports it,
/// the window believes its cursor moved there, and the release lands the
/// frame after — so the game's own `just_pressed` window sees exactly one.
#[test]
fn a_click_presses_and_the_release_lands_the_next_frame() {
    let (mut app, port) = agent_app();
    let mut client = Client::connect(port);

    let reply = client.ask(&mut app, r#"{"cmd":"click","x":400,"y":300}"#);
    assert_eq!(reply["ok"], true);

    let world = app.world();
    let buttons = world.resource::<ButtonInput<MouseButton>>();
    assert!(buttons.just_pressed(MouseButton::Left), "the press is seen");
    assert!(buttons.pressed(MouseButton::Left));
    assert_eq!(
        window(&mut app).cursor_position(),
        Some(Vec2::new(400.0, 300.0)),
        "the cursor really moved"
    );

    app.update();
    let world = app.world();
    let buttons = world.resource::<ButtonInput<MouseButton>>();
    assert!(
        !buttons.pressed(MouseButton::Left),
        "released one frame later"
    );
    assert!(buttons.just_released(MouseButton::Left));
}

/// Every frame a left press was seen, with the cursor position of that
/// frame. Pipelined clicks are only ever observed *after* several frames
/// have run — the reply loop runs the app — so the per-frame property is
/// recorded here and asserted at the end.
#[derive(Resource, Default)]
struct PressLog(Vec<(u32, Vec2)>);

fn log_presses(
    mut frames: Local<u32>,
    buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    mut log: ResMut<PressLog>,
) {
    *frames += 1;
    if buttons.just_pressed(MouseButton::Left) {
        let at = windows
            .single()
            .ok()
            .and_then(|w| w.cursor_position())
            .unwrap_or_default();
        log.0.push((*frames, at));
    }
}

/// Two clicks sent back to back must not merge, and the second click's
/// cursor position must not overtake the first click's press: presses go out
/// one frame apart, in queue order, each against the cursor position of its
/// own frame.
#[test]
fn two_hasty_clicks_stay_two_clicks() {
    let (mut app, port) = agent_app();
    app.add_systems(Update, log_presses);
    app.init_resource::<PressLog>();
    let mut client = Client::connect(port);

    client.send(r#"{"cmd":"click","x":10,"y":10}"#);
    client.send(r#"{"cmd":"click","x":20,"y":20}"#);
    for _ in 0..4 {
        app.update();
    }
    assert_eq!(client.read(&mut app)["ok"], true);
    assert_eq!(client.read(&mut app)["ok"], true);

    let log = &app.world().resource::<PressLog>().0;
    assert_eq!(log.len(), 2, "two clicks, not one and not three: {log:?}");
    assert_eq!(
        log[0].1,
        Vec2::new(10.0, 10.0),
        "the first click clicks where it aimed"
    );
    assert_eq!(
        log[1].1,
        Vec2::new(20.0, 20.0),
        "the second click clicks where it aimed"
    );
    assert!(
        log[1].0 > log[0].0,
        "the second press is a distinct, later frame: {log:?}"
    );
}

/// A key arrives on both channels the game reads: the `ButtonInput<KeyCode>`
/// the round's shortcuts watch.
#[test]
fn a_key_press_reaches_button_input() {
    let (mut app, port) = agent_app();
    let mut client = Client::connect(port);

    let reply = client.ask(&mut app, r#"{"cmd":"key","key":"enter"}"#);
    assert_eq!(reply["ok"], true);

    let keys = app.world().resource::<ButtonInput<KeyCode>>();
    assert!(keys.just_pressed(KeyCode::Enter));
    assert!(keys.pressed(KeyCode::Enter));
}

/// Typed text flows through the real keyboard dispatch — the focused
/// [`TextInput`] widget receives the characters and inserts them, exactly as
/// if a keyboard had typed them. This is the path the lobby's fields run on.
#[test]
fn text_types_into_the_focused_field() {
    let (mut app, port) = agent_app();
    let mut client = Client::connect(port);
    let field = focused_field(&mut app);

    let reply = client.ask(&mut app, r#"{"cmd":"text","text":"hi"}"#);
    assert_eq!(reply["ok"], true);
    assert_eq!(reply["typed"], 2);

    // Two characters, one press per frame, each release the frame after.
    for _ in 0..3 {
        app.update();
    }

    assert_eq!(field_text(&app, field), "hi");
}

/// An unknown key name is refused with a reason, not silently dropped.
#[test]
fn an_unknown_key_name_is_refused() {
    let (mut app, port) = agent_app();
    let mut client = Client::connect(port);
    let reply = client.ask(&mut app, r#"{"cmd":"key","key":"meta"}"#);
    assert_eq!(reply["ok"], false);
    assert!(
        reply["error"]
            .as_str()
            .expect("an error message")
            .contains("meta"),
        "the refusal names the key: {reply}"
    );
}

/// A malformed line is answered with a parse error and the connection lives
/// on for the next request.
#[test]
fn a_bad_request_is_refused_and_the_connection_survives() {
    let (mut app, port) = agent_app();
    let mut client = Client::connect(port);

    client.send(r#"{"cmd":"teleport"}"#);
    // The parse error is answered by the connection thread without a frame.
    let refused = client.read(&mut app);
    assert_eq!(refused["ok"], false);

    let reply = client.ask(&mut app, r#"{"cmd":"ping"}"#);
    assert_eq!(reply["ok"], true, "the connection still works");
}

/// `window` and `resize` see and set the real window.
#[test]
fn window_reports_and_resize_sets() {
    let (mut app, port) = agent_app();
    let mut client = Client::connect(port);

    let reply = client.ask(&mut app, r#"{"cmd":"window"}"#);
    assert_eq!(reply["ok"], true);
    assert!(reply["width"].as_f64().expect("a width") > 0.0);
    assert!(reply["height"].as_f64().expect("a height") > 0.0);

    let reply = client.ask(&mut app, r#"{"cmd":"resize","width":640,"height":480}"#);
    assert_eq!(reply["ok"], true);

    let reply = client.ask(&mut app, r#"{"cmd":"window"}"#);
    assert_eq!(reply["width"], 640.0);
    assert_eq!(reply["height"], 480.0);
    assert_eq!(window(&mut app).width(), 640.0);
}

/// `state` also reports the lobby: its status line (where a refused press
/// writes its reason) and every labelled button, at the coordinates a
/// `click` takes.
#[test]
fn state_reports_the_lobby_status_and_its_buttons() {
    let (mut app, port) = agent_app();
    use checkers_bevy::lobby::{LobbyStatus, SelectedCorner, Table};
    app.insert_resource(LobbyStatus(
        "At least two corners must be filled before the game can start.".into(),
    ))
    .insert_resource(Table::default())
    .insert_resource(SelectedCorner::default());
    let button = app.world_mut().spawn(bevy::ui_widgets::Button).id();
    app.world_mut().entity_mut(button).insert((
        Visibility::Inherited,
        ComputedNode {
            size: Vec2::new(80.0, 30.0),
            ..default()
        },
        UiGlobalTransform::from_translation(Vec2::new(400.0, 650.0)),
        ComputedUiRenderTargetInfo::default(),
    ));
    let label = app.world_mut().spawn(Text("Human".into())).id();
    app.world_mut().entity_mut(button).add_child(label);

    let mut client = Client::connect(port);
    let reply = client.ask(&mut app, r#"{"cmd":"state"}"#);
    let state = &reply["state"];

    assert_eq!(
        state["lobby"]["status"], "At least two corners must be filled before the game can start.",
        "the refusal an agent needs to read"
    );
    assert_eq!(state["lobby"]["table"][0], "Empty");
    assert_eq!(state["lobby"]["selected_corner"], serde_json::Value::Null);

    let buttons = state["buttons"].as_array().expect("buttons");
    assert_eq!(buttons.len(), 1, "{buttons:?}");
    assert_eq!(buttons[0]["label"], "Human");
    assert_eq!(buttons[0]["x"], 400.0);
    assert_eq!(buttons[0]["y"], 650.0);
}

/// Without a renderer there is nothing to capture; the request must come
/// back with an error within a bounded number of frames rather than hang
/// the agent.
#[test]
fn a_screenshot_without_a_renderer_refuses_instead_of_hanging() {
    let (mut app, port) = agent_app();
    let mut client = Client::connect(port);

    client.send(r#"{"cmd":"screenshot"}"#);
    let mut reply = None;
    for _ in 0..(agent::screenshot_timeout_frames() + 10) {
        app.update();
        if let Ok(value) = client.try_read() {
            reply = Some(value);
            break;
        }
    }
    let reply = reply.expect("the screenshot answers eventually");
    assert_eq!(reply["ok"], false);
    assert!(
        reply["error"]
            .as_str()
            .expect("an error message")
            .contains("headless"),
        "the refusal says why: {reply}"
    );
}
