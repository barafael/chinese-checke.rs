//! The agent interface: a localhost socket through which a script — a test
//! harness, or an AI agent — can drive the running window, with no human
//! input device involved.
//!
//! An agent's loop is exactly a person's: *look* (screenshot), *act* (click,
//! key, text), and *check* (state). The commands are injected at the same
//! boundary real input enters the app — Bevy's window-event messages, the
//! translation layer between the operating system and the ECS — so an agent
//! exercises the genuine input paths: picking, focus, text fields, the
//! keyboard shortcuts. Nothing about the game is bypassed; there is no
//! privileged "make move" command, only what a hand on a mouse could do.
//!
//! # Protocol
//!
//! Newline-delimited JSON over TCP, one request per line, one response line
//! per request, in order. Coordinates are logical pixels, origin at the
//! window's top-left — the same numbers `window.cursor_position()` reports.
//!
//! ```text
//! {"cmd":"screenshot"}                  -> {"ok":true,"width":1280,"height":935,"png":"<base64>"}
//! {"cmd":"click","x":640,"y":468}       -> {"ok":true}
//! {"cmd":"move","x":640,"y":468}        -> {"ok":true}
//! {"cmd":"key","key":"enter"}           -> {"ok":true}
//! {"cmd":"text","text":"hello"}         -> {"ok":true,"typed":5}
//! {"cmd":"state"}                       -> {"ok":true,"state":{...}}
//! {"cmd":"window"}                      -> {"ok":true,"width":...,"scale_factor":...}
//! {"cmd":"resize","width":900,"height":700} -> {"ok":true}
//! {"cmd":"ping"}                        -> {"ok":true,"port":4310}
//! {"cmd":"quit"}                        -> {"ok":true}   (then the app exits)
//! ```
//!
//! Errors answer `{"ok":false,"error":"..."}`.
//!
//! # Enabling
//!
//! The socket binds to `127.0.0.1` only. `CHECKERS_AGENT_PORT` picks the port:
//! unset means 4310, an explicit number is honoured (scanning upward if it is
//! taken, for the two-window multiplayer workflow), `off` disables the whole
//! thing. The chosen port is printed to stdout as `agent port <n>` so a
//! launcher can grep for it.
//!
//! Screenshots ride Bevy's own capture path ([`Screenshot`]), which needs the
//! renderer; headless builds answer with an error after a short timeout
//! instead of hanging the agent.

use base64::Engine as _;
use bevy::app::First;
use bevy::ecs::system::SystemParam;
use bevy::image::Image;
use bevy::input::keyboard::{Key, KeyCode, KeyboardInput, NativeKeyCode};
use bevy::input::mouse::MouseButtonInput;
use bevy::picking::PickingSystems;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::ui::{ComputedUiRenderTargetInfo, UiGlobalTransform};
use bevy::window::{CursorMoved, PrimaryWindow, WindowEvent};
use checkers_core::geometry::Coord;
use serde::Deserialize;
use serde_json::json;
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use crate::lobby;
use crate::{AppState, Session};

/// Frames a screenshot may take before the answer is "no renderer here".
/// Real captures complete in one or two.
const SCREENSHOT_TIMEOUT_FRAMES: u32 = 120;

/// The screenshot timeout, for tests that must wait exactly that long.
#[doc(hidden)]
pub fn screenshot_timeout_frames() -> u32 {
    SCREENSHOT_TIMEOUT_FRAMES
}

/// The port tried when `CHECKERS_AGENT_PORT` says nothing.
const DEFAULT_PORT: u16 = 4310;

/// Registers the agent socket and the systems that serve it. A no-op on wasm,
/// where there is no socket to bind.
pub fn plugin(app: &mut App) {
    if cfg!(target_family = "wasm") {
        return;
    }
    let port = match std::env::var("CHECKERS_AGENT_PORT") {
        Ok(v) if matches!(v.as_str(), "off" | "none") => return,
        Ok(v) => v.parse::<u16>().unwrap_or(DEFAULT_PORT),
        Err(_) => DEFAULT_PORT,
    };
    insert_bridge(app, port);
}

/// Bind `port` (or the next free one above it), spawn the listener thread and
/// register the serving systems.
///
/// Separate from [`plugin`] so tests can pick port 0 — the OS hands out a
/// free one — without touching the environment, which parallel tests share.
fn insert_bridge(app: &mut App, port: u16) {
    if bridge(app, port).is_none() {
        warn!("agent interface could not start near port {port}; it is off");
    }
}

/// Test hook: bind an explicit port — `0` lets the OS pick a free one — and
/// register everything the plugin registers.
#[doc(hidden)]
pub fn bridge_for_tests(app: &mut App, port: u16) -> Option<u16> {
    bridge(app, port)
}

fn bridge(app: &mut App, port: u16) -> Option<u16> {
    let listener = bind(port)?;
    let bound = listener.local_addr().map(|a| a.port()).unwrap_or(port);

    // A grep-friendly line on stdout: launchers parse it rather than assume
    // the default port, which two windows running side by side cannot share.
    println!("agent port {bound}");
    info!("agent interface listening on 127.0.0.1:{bound}");

    let (tx, rx) = mpsc::channel::<Request>();
    if std::thread::Builder::new()
        .name("agent-listener".into())
        .spawn(move || serve(listener, tx))
        .is_err()
    {
        warn!("agent interface could not start its listener thread; it is off");
        return None;
    };

    // The receiver lives behind a mutex only because resources must be Sync;
    // the game thread is its only reader.
    app.insert_resource(AgentServer {
        requests: Mutex::new(rx),
    })
    .insert_resource(AgentPort(bound))
    .insert_resource(AgentQueue::default())
    .insert_resource(PendingScreenshots::default());
    app.add_systems(First, pump.before(PickingSystems::Input));
    Some(bound)
}

/// Bind the first free port in `port..port+16`. The upward scan exists for
/// the two-instance workflow: a second window just takes the next port.
fn bind(port: u16) -> Option<TcpListener> {
    (port..port.saturating_add(16)).find_map(|p| TcpListener::bind(("127.0.0.1", p)).ok())
}

// --- wire types --------------------------------------------------------------

/// Where a response goes. Most requests are answered on the game thread
/// within the same frame; a screenshot answers from an observer after the
/// render, so the sender is shared rather than consumed.
#[derive(Clone)]
struct Reply(Arc<Mutex<mpsc::Sender<String>>>);

impl Reply {
    fn send(&self, value: serde_json::Value) {
        let Ok(text) = serde_json::to_string(&value) else {
            return;
        };
        if let Ok(sender) = self.0.lock()
            && sender.send(text).is_err()
        {
            debug!("agent client hung up before a reply");
        }
    }

    /// A success answer, `ok: true` plus whatever the command has to say.
    fn ok(&self, value: serde_json::Value) {
        let mut value = value;
        if let serde_json::Value::Object(ref mut map) = value {
            map.insert("ok".into(), serde_json::Value::Bool(true));
        }
        self.send(value);
    }

    fn err(&self, error: impl Into<String>) {
        self.send(json!({ "ok": false, "error": error.into() }));
    }
}

/// One parsed request plus the way back to its client.
struct Request {
    command: Command,
    reply: Reply,
}

/// Everything an agent can ask for. Parsed from the `"cmd"` tag; unknown
/// commands are refused by serde itself, with the variant list in the error.
#[derive(Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
enum Command {
    Ping,
    /// The window as PNG, base64-encoded.
    Screenshot,
    /// Logical pixels, origin top-left. Moves the cursor, presses and
    /// releases. Presses go out one per frame, so two hasty clicks stay two
    /// distinct `just_pressed`s for the game's input systems.
    Click {
        x: f32,
        y: f32,
    },
    /// Move the cursor without pressing.
    Move {
        x: f32,
        y: f32,
    },
    /// Press and release a named key. Names are lowercase: `enter`, `space`,
    /// `escape`, `backspace`, `tab`, `delete`, `home`, `end`, `pageup`,
    /// `pagedown`, `left`/`right`/`up`/`down`, `a`–`z`, `0`–`9`, and the
    /// punctuation names `comma`, `period`, `minus`, `equal`. `shift`
    /// uppercases letters.
    Key {
        key: String,
        #[serde(default)]
        shift: bool,
    },
    /// Type a string character by character, through the same path a real
    /// keyboard takes — text fields see it as typing.
    Text {
        text: String,
    },
    /// The game as JSON: board, turn, selection, legal destinations.
    State,
    /// Window size, scale factor, cursor position.
    Window,
    /// Set the window size in logical pixels.
    Resize {
        width: f32,
        height: f32,
    },
    /// Ask the app to exit cleanly.
    Quit,
}

/// Accept loop: one thread per connection, each a plain request-response
/// pump. Parsing happens here so the game thread only ever sees typed
/// commands.
fn serve(listener: TcpListener, enqueue: mpsc::Sender<Request>) {
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let enqueue = enqueue.clone();
        if std::thread::Builder::new()
            .name("agent-connection".into())
            .spawn(move || handle_connection(stream, enqueue))
            .is_err()
        {
            continue;
        }
    }
}

fn handle_connection(stream: TcpStream, enqueue: mpsc::Sender<Request>) {
    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(stream);
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => return, // hung up
            Ok(_) => {}
        }

        let response = match serde_json::from_str::<Command>(&line) {
            Ok(command) => {
                let (tx, rx) = mpsc::channel();
                let request = Request {
                    command,
                    reply: Reply(Arc::new(Mutex::new(tx))),
                };
                if enqueue.send(request).is_err() {
                    return; // the app is gone
                }
                rx.recv_timeout(Duration::from_secs(30))
                    .unwrap_or_else(|_| {
                        r#"{"ok":false,"error":"timed out waiting for the game"}"#.into()
                    })
            }
            Err(e) => serde_json::json!({
                "ok": false,
                "error": format!("bad request: {e}"),
            })
            .to_string(),
        };
        if writeln!(writer, "{response}")
            .and_then(|()| writer.flush())
            .is_err()
        {
            return;
        }
    }
}

// --- the serving side --------------------------------------------------------

/// The receiving half of the agent socket, drained once per frame. The mutex
/// is for `Resource`'s `Sync` bound; the game thread is the only reader.
#[derive(Resource)]
struct AgentServer {
    requests: Mutex<mpsc::Receiver<Request>>,
}

/// The port actually bound, for `ping` answers.
#[derive(Resource)]
struct AgentPort(u16);

/// Input not yet delivered to the app. Everything here becomes real
/// window-event messages, a click or a keypress per frame.
#[derive(Default, Resource)]
struct AgentQueue {
    pending: VecDeque<Injection>,
    /// Presses' other halves, held until the next frame so a press and its
    /// release never land in the same `just_pressed` window.
    next_frame: Vec<Injection>,
    /// Last injected cursor position, physical, for the `delta` the real
    /// winit events carry.
    last_physical: Option<bevy::math::DVec2>,
}

/// One synthetic input event, mirroring what winit would have sent.
#[derive(Clone)]
enum Injection {
    Cursor { physical: bevy::math::DVec2 },
    Press(MouseButton),
    Release(MouseButton),
    KeyPress(KeyboardInput),
    KeyRelease(KeyboardInput),
}

/// Screenshots asked for but not yet captured. The renderer answers by
/// triggering [`ScreenshotCaptured`], which the capture observer matches
/// back to its request here.
#[derive(Default, Resource)]
struct PendingScreenshots {
    list: Vec<PendingScreenshot>,
}

struct PendingScreenshot {
    entity: Entity,
    reply: Reply,
    frames: u32,
}

impl PendingScreenshots {
    fn finish(&mut self, entity: Entity) -> Option<PendingScreenshot> {
        let at = self.list.iter().position(|p| p.entity == entity)?;
        Some(self.list.remove(at))
    }
}

/// The writers an injection needs, bundled to keep `pump` readable.
#[derive(SystemParam)]
struct Injections<'w> {
    cursor: MessageWriter<'w, CursorMoved>,
    mouse: MessageWriter<'w, MouseButtonInput>,
    keyboard: MessageWriter<'w, KeyboardInput>,
    window_events: MessageWriter<'w, WindowEvent>,
    exits: MessageWriter<'w, AppExit>,
}

/// Everything `pump` touches, in one parameter.
#[derive(SystemParam)]
struct AgentCtx<'w, 's> {
    server: ResMut<'w, AgentServer>,
    port: Res<'w, AgentPort>,
    queue: ResMut<'w, AgentQueue>,
    shots: ResMut<'w, PendingScreenshots>,
    windows: Query<'w, 's, (Entity, &'static mut Window), With<PrimaryWindow>>,
    session: Res<'w, Session>,
    phase: Option<Res<'w, State<AppState>>>,
    /// The lobby's status line, where refusals are written. An agent that
    /// presses Enter and nothing happens must be able to read why.
    status: Option<Res<'w, lobby::LobbyStatus>>,
    table: Option<Res<'w, lobby::Table>>,
    selected: Option<Res<'w, lobby::SelectedCorner>>,
    /// The labelled, clickable buttons on screen — the lobby's Start and
    /// corner actions included — with their centres in the same logical
    /// pixels a `click` takes.
    buttons: Query<
        'w,
        's,
        (
            &'static ComputedNode,
            &'static UiGlobalTransform,
            &'static ComputedUiRenderTargetInfo,
            &'static Visibility,
            &'static Children,
        ),
        With<bevy::ui_widgets::Button>,
    >,
    labels: Query<'w, 's, &'static Text>,
    branches: Query<'w, 's, &'static Children, Without<Text>>,
    ui_scale: Res<'w, UiScale>,
    inject: Injections<'w>,
    commands: Commands<'w, 's>,
}

/// Drain agent requests and inject queued input, first thing every frame —
/// before the picking input set, so a cursor move and the press behind it are
/// processed by picking in the frame they are injected, exactly as if the OS
/// had delivered them.
fn pump(mut agent: AgentCtx) {
    // Releases deferred from last frame lead the queue: a click is move,
    // press, then release one frame later, in that order.
    let deferred = std::mem::take(&mut agent.queue.next_frame);
    let mut items: Vec<_> = deferred.into_iter().collect();
    items.extend(agent.queue.pending.drain(..));
    agent.queue.pending = items.into_iter().collect();

    loop {
        let request = {
            let Ok(rx) = agent.server.requests.lock() else {
                break;
            };
            match rx.try_recv() {
                Ok(request) => request,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => break,
            }
        };
        handle(request, &mut agent);
    }

    inject_queued(&mut agent);

    // Screenshots that never captured: headless builds have no renderer.
    let mut timed_out = Vec::new();
    for pending in &mut agent.shots.list {
        pending.frames += 1;
        if pending.frames > SCREENSHOT_TIMEOUT_FRAMES {
            timed_out.push(pending.reply.clone());
        }
    }
    agent
        .shots
        .list
        .retain(|p| p.frames <= SCREENSHOT_TIMEOUT_FRAMES);
    for reply in timed_out {
        reply.err("screenshot timed out - is this build headless?");
    }
}

fn handle(request: Request, agent: &mut AgentCtx) {
    match request.command {
        Command::Ping => request.reply.ok(json!({ "port": agent.port.0 })),
        Command::Move { x, y } => {
            enqueue_cursor(agent, x, y);
            request.reply.ok(json!({}));
        }
        Command::Click { x, y } => {
            enqueue_cursor(agent, x, y);
            agent
                .queue
                .pending
                .push_back(Injection::Press(MouseButton::Left));
            request.reply.ok(json!({}));
        }
        Command::Key { key, shift } => match parse_key(&key, shift) {
            Some((key_code, logical, text)) => {
                agent
                    .queue
                    .pending
                    .push_back(key_injection(key_code, logical, text));
                request.reply.ok(json!({}));
            }
            None => request.reply.err(format!("unknown key name {key:?}")),
        },
        Command::Text { text } => {
            let mut typed = 0;
            for c in text.chars() {
                match char_key(c) {
                    Some((key_code, logical, typed_text)) => {
                        agent
                            .queue
                            .pending
                            .push_back(key_injection(key_code, logical, typed_text));
                        typed += 1;
                    }
                    None => request.reply.err(format!("cannot type {c:?}")),
                }
            }
            request.reply.ok(json!({ "typed": typed }));
        }
        Command::State => request.reply.ok(json!({ "state": state_json(agent) })),
        Command::Window => match agent.windows.single() {
            Ok((_, window)) => request.reply.ok(json!({
                "width": window.width(),
                "height": window.height(),
                "physical_width": window.resolution.physical_width(),
                "physical_height": window.resolution.physical_height(),
                "scale_factor": window.scale_factor(),
                "cursor": window.cursor_position().map(|c| [c.x, c.y]),
                "title": window.title,
                "focused": window.focused,
            })),
            Err(_) => request.reply.err("no window"),
        },
        Command::Resize { width, height } => match agent.windows.single_mut() {
            Ok((_, mut window)) => {
                window.resolution.set(width, height);
                request.reply.ok(json!({}));
            }
            Err(_) => request.reply.err("no window"),
        },
        Command::Screenshot => {
            let entity = agent
                .commands
                .spawn(Screenshot::primary_window())
                .observe(capture)
                .id();
            agent.shots.list.push(PendingScreenshot {
                entity,
                reply: request.reply.clone(),
                frames: 0,
            });
        }
        Command::Quit => {
            info!("agent requested quit");
            agent.inject.exits.write(AppExit::Success);
            request.reply.ok(json!({}));
        }
    }
}

fn enqueue_cursor(agent: &mut AgentCtx, x: f32, y: f32) {
    // Behind anything already queued: an agent that pipelines click, click,
    // move must see its clicks where it put them.
    let Some((_, window)) = agent.windows.single().ok() else {
        return;
    };
    let scale = window.resolution.scale_factor();
    let physical = bevy::math::DVec2::new((x * scale) as f64, (y * scale) as f64);
    agent
        .queue
        .pending
        .push_back(Injection::Cursor { physical });
}

/// A key press queued for injection. `text` is what the keystroke types, if
/// anything — the field a focused text widget actually inserts from.
fn key_injection(key_code: KeyCode, logical: Key, text: Option<String>) -> Injection {
    Injection::KeyPress(KeyboardInput {
        key_code,
        logical_key: logical,
        state: bevy::input::ButtonState::Pressed,
        text: text.map(Into::into),
        repeat: false,
        window: Entity::PLACEHOLDER,
    })
}

/// The queue is drained strictly in order, one press per frame: when the
/// frame's press is spent, draining stops outright. Order is everything — an
/// agent that pipelines click A, click B must not have B's cursor position
/// overtake A's press, or A clicks where B aimed. A real mouse cannot emit
/// its events out of order, and neither do we.
fn inject_queued(agent: &mut AgentCtx) {
    let mut pressed = false;
    while let Some(injection) = agent.queue.pending.pop_front() {
        if pressed {
            agent.queue.pending.push_front(injection);
            break;
        }
        match injection {
            Injection::Cursor { physical } => inject_cursor(agent, physical),
            Injection::Press(button) => {
                press_mouse(agent, button, bevy::input::ButtonState::Pressed);
                agent.queue.next_frame.push(Injection::Release(button));
                pressed = true;
            }
            Injection::Release(button) => {
                press_mouse(agent, button, bevy::input::ButtonState::Released);
            }
            Injection::KeyPress(input) => {
                write_key(agent, &input, bevy::input::ButtonState::Pressed);
                let mut release = input;
                release.state = bevy::input::ButtonState::Released;
                agent.queue.next_frame.push(Injection::KeyRelease(release));
                pressed = true;
            }
            Injection::KeyRelease(input) => {
                write_key(agent, &input, bevy::input::ButtonState::Released);
            }
        }
    }
}

fn press_mouse(agent: &mut AgentCtx, button: MouseButton, state: bevy::input::ButtonState) {
    let Some(entity) = window_entity(agent) else {
        return;
    };
    let input = MouseButtonInput {
        button,
        state,
        window: entity,
    };
    agent.inject.mouse.write(input);
    agent
        .inject
        .window_events
        .write(WindowEvent::MouseButtonInput(input));
}

/// Set the window's cursor position and tell the app it moved — the same two
/// writes winit makes, minus the OS detour. The window component is mutated
/// with change detection off: `bevy_winit` reacts to a changed cursor
/// position by *warping the real mouse*, which on Wayland only logs an error
/// and everywhere else surprises whoever is holding it.
fn inject_cursor(agent: &mut AgentCtx, physical: bevy::math::DVec2) {
    let Ok((entity, mut window)) = agent.windows.single_mut() else {
        return;
    };
    let scale = window.resolution.scale_factor();
    let logical = (physical / scale as f64).as_vec2();
    let delta = agent
        .queue
        .last_physical
        .map(|last| (physical - last).as_vec2() / scale);

    window
        .bypass_change_detection()
        .set_physical_cursor_position(Some(physical));

    let moved = CursorMoved {
        window: entity,
        position: logical,
        delta,
    };
    agent.inject.cursor.write(moved.clone());
    agent
        .inject
        .window_events
        .write(WindowEvent::CursorMoved(moved));
    agent.queue.last_physical = Some(physical);
}

fn write_key(agent: &mut AgentCtx, input: &KeyboardInput, state: bevy::input::ButtonState) {
    let Some(entity) = window_entity(agent) else {
        return;
    };
    let mut event = input.clone();
    event.state = state;
    event.window = entity;
    agent.inject.keyboard.write(event.clone());
    agent
        .inject
        .window_events
        .write(WindowEvent::KeyboardInput(event));
}

fn window_entity(agent: &AgentCtx) -> Option<Entity> {
    agent.windows.single().ok().map(|(e, _)| e)
}

/// The captured PNG leaves through the request's reply, tagged with the
/// window size the agent sees.
fn capture(trigger: On<ScreenshotCaptured>, mut shots: ResMut<PendingScreenshots>) {
    let Some(pending) = shots.finish(trigger.entity) else {
        return;
    };
    match encode_png(&trigger.image) {
        Ok(png) => pending.reply.ok(json!({
            "width": trigger.image.width(),
            "height": trigger.image.height(),
            "png": base64::engine::general_purpose::STANDARD.encode(png),
        })),
        Err(e) => pending.reply.err(e),
    }
}

/// Raw screenshot bytes to PNG. Window captures arrive in the swapchain's
/// format — BGRA almost everywhere; RGBA is kept for completeness.
fn encode_png(image: &Image) -> Result<Vec<u8>, String> {
    let (width, height) = (image.width(), image.height());
    let data = image
        .data
        .as_deref()
        .ok_or_else(|| "screenshot carried no pixels".to_string())?;
    let rgba = match image.texture_descriptor.format {
        TextureFormat::Bgra8Unorm | TextureFormat::Bgra8UnormSrgb => data
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|px| [px[2], px[1], px[0], px[3]])
            .collect(),
        TextureFormat::Rgba8Unorm | TextureFormat::Rgba8UnormSrgb => data.to_vec(),
        other => return Err(format!("unsupported screenshot format {other:?}")),
    };
    let mut out = std::io::Cursor::new(Vec::new());
    image::RgbaImage::from_raw(width, height, rgba)
        .ok_or_else(|| "screenshot size does not match its data".to_string())?
        .write_to(&mut out, image::ImageFormat::Png)
        .map_err(|e| format!("png encoding failed: {e}"))?;
    Ok(out.into_inner())
}

// --- the game, as JSON -------------------------------------------------------

/// The whole visible game state: enough for an agent to decide its next click
/// without reading pixels.
fn state_json(agent: &AgentCtx) -> serde_json::Value {
    let session = &agent.session;
    let position = session.game.position();
    let board: Vec<_> = position
        .holes()
        .iter()
        .filter_map(|&c| position.occupant(c).map(|p| json!([c.q, c.r, p.index()])))
        .collect();
    let phase = agent
        .phase
        .as_ref()
        .map(|s| format!("{:?}", s.get()))
        .unwrap_or_default();
    json!({
        "phase": phase,
        "players": session.players.iter().map(|p| p.index()).collect::<Vec<_>>(),
        "ai_players": session.ai_players.iter().map(|p| p.index()).collect::<Vec<_>>(),
        "local_player": session.local_player().map(|p| p.index()),
        "spectating": session.spectating,
        "shared": session.shared,
        "turn": session.game.turn().index(),
        "outcome": outcome_json(session),
        "board": board,
        "selection": selection_json(session),
        "highlights": session.highlights().into_iter().map(cordon).collect::<Vec<_>>(),
        "may_act": session.may_act(),
        "can_confirm": session.can_confirm(),
        "is_jumping": session.is_jumping(),
        "moves_committed": session.history().len(),
        "message": session.message,
        "lobby": lobby_json(agent),
        "buttons": buttons_json(agent),
    })
}

/// The lobby half of `state`: the status line (where a refused press writes
/// its reason), the table's corners, and the corner a digit or wedge has
/// selected. Null outside the lobby.
fn lobby_json(agent: &AgentCtx) -> serde_json::Value {
    let table = agent
        .table
        .as_ref()
        .map(|t| t.0.iter().map(|c| format!("{c:?}")).collect::<Vec<_>>());
    json!({
        "status": agent.status.as_ref().map(|s| s.0.clone()),
        "table": table,
        "selected_corner": agent.selected.as_ref().and_then(|s| s.0),
    })
}

/// Every labelled button on screen, centre and size in the logical pixels a
/// `click` takes. The centre is the button transform's origin, mapped by the
/// inverse of the way a pointer click comes in — the same coordinates
/// picking resolves, so a click at `x, y` lands where this says.
fn buttons_json(agent: &AgentCtx) -> Vec<serde_json::Value> {
    let target_scale = agent
        .buttons
        .iter()
        .next()
        .map(|(_, _, target, _, _)| target.scale_factor())
        .unwrap_or(1.0);
    let to_logical = agent.ui_scale.0 / target_scale;
    let mut out = Vec::new();
    for (node, transform, _, visibility, children) in agent.buttons.iter() {
        // A hidden button would swallow the click and do nothing, and a
        // zero-sized one has not been laid out yet. Neither is worth
        // offering.
        if *visibility == Visibility::Hidden || node.size == Vec2::ZERO {
            continue;
        }
        let centre = transform.affine().transform_point2(Vec2::ZERO) * to_logical;
        out.push(json!({
            "label": label_of(agent, children),
            "x": centre.x,
            "y": centre.y,
            "w": node.size.x * to_logical,
            "h": node.size.y * to_logical,
        }));
    }
    out
}

/// The first text in a button's subtree, reading one branch at a time. Every
/// button in this app carries a single `ui_text` child, so one or two levels
/// is the whole story.
fn label_of(agent: &AgentCtx, children: &Children) -> Option<String> {
    for child in children.iter() {
        if let Ok(text) = agent.labels.get(child) {
            return Some(text.to_string());
        }
        if let Ok(grandchildren) = agent.branches.get(child) {
            for grandchild in grandchildren.iter() {
                if let Ok(text) = agent.labels.get(grandchild) {
                    return Some(text.to_string());
                }
            }
        }
    }
    None
}

fn outcome_json(session: &Session) -> serde_json::Value {
    use checkers_core::rules::Outcome;
    match session.game.outcome() {
        None => serde_json::Value::Null,
        Some(Outcome::Winner(p)) => json!(format!("winner({})", p.index())),
        Some(Outcome::Resigned(p)) => json!(format!("resigned({})", p.index())),
        Some(Outcome::Draw) => json!("draw"),
        Some(Outcome::Abandoned) => json!("abandoned"),
    }
}

fn selection_json(session: &Session) -> serde_json::Value {
    match &session.selection {
        crate::Selection::None => json!({ "kind": "none" }),
        crate::Selection::Piece { origin } => {
            json!({ "kind": "piece", "origin": cordon(*origin) })
        }
        crate::Selection::Pend { mv, .. } => json!({
            "kind": "pend",
            "origin": cordon(mv.origin),
            "destination": cordon(mv.destination),
        }),
        crate::Selection::Jumping { turn } => json!({
            "kind": "jumping",
            "current": cordon(turn.current()),
            "hops": turn.hops(),
        }),
    }
}

/// A hole as `[q, r]`, the array form the `state` board uses.
fn cordon(c: Coord) -> [i32; 2] {
    [c.q, c.r]
}

// --- key names ---------------------------------------------------------------

/// Physical codes for the letters and digits, in `char` order. `KeyCode` has
/// no arithmetic, so the tables are written out.
const LETTER_CODES: [KeyCode; 26] = [
    KeyCode::KeyA,
    KeyCode::KeyB,
    KeyCode::KeyC,
    KeyCode::KeyD,
    KeyCode::KeyE,
    KeyCode::KeyF,
    KeyCode::KeyG,
    KeyCode::KeyH,
    KeyCode::KeyI,
    KeyCode::KeyJ,
    KeyCode::KeyK,
    KeyCode::KeyL,
    KeyCode::KeyM,
    KeyCode::KeyN,
    KeyCode::KeyO,
    KeyCode::KeyP,
    KeyCode::KeyQ,
    KeyCode::KeyR,
    KeyCode::KeyS,
    KeyCode::KeyT,
    KeyCode::KeyU,
    KeyCode::KeyV,
    KeyCode::KeyW,
    KeyCode::KeyX,
    KeyCode::KeyY,
    KeyCode::KeyZ,
];
const DIGIT_CODES: [KeyCode; 10] = [
    KeyCode::Digit0,
    KeyCode::Digit1,
    KeyCode::Digit2,
    KeyCode::Digit3,
    KeyCode::Digit4,
    KeyCode::Digit5,
    KeyCode::Digit6,
    KeyCode::Digit7,
    KeyCode::Digit8,
    KeyCode::Digit9,
];

/// Parse a key name into the key code, the logical key, and the text it
/// types. `shift` uppercases letters; nothing else pretends to a layout.
fn parse_key(name: &str, shift: bool) -> Option<(KeyCode, Key, Option<String>)> {
    let named = |code, key| Some((code, key, None));
    match name {
        "enter" | "return" => named(KeyCode::Enter, Key::Enter),
        "numpadenter" => named(KeyCode::NumpadEnter, Key::Enter),
        "space" => Some((KeyCode::Space, Key::Space, Some(" ".into()))),
        "escape" => named(KeyCode::Escape, Key::Escape),
        "backspace" => named(KeyCode::Backspace, Key::Backspace),
        "delete" => named(KeyCode::Delete, Key::Delete),
        "tab" => named(KeyCode::Tab, Key::Tab),
        "home" => named(KeyCode::Home, Key::Home),
        "end" => named(KeyCode::End, Key::End),
        "pageup" => named(KeyCode::PageUp, Key::PageUp),
        "pagedown" => named(KeyCode::PageDown, Key::PageDown),
        "up" => named(KeyCode::ArrowUp, Key::ArrowUp),
        "down" => named(KeyCode::ArrowDown, Key::ArrowDown),
        "left" => named(KeyCode::ArrowLeft, Key::ArrowLeft),
        "right" => named(KeyCode::ArrowRight, Key::ArrowRight),
        "comma" => Some((KeyCode::Comma, character_key(','), Some(",".into()))),
        "period" => Some((KeyCode::Period, character_key('.'), Some(".".into()))),
        "minus" => Some((KeyCode::Minus, character_key('-'), Some("-".into()))),
        "equal" => Some((KeyCode::Equal, character_key('='), Some("=".into()))),
        other => {
            let mut chars = other.chars();
            let (c, rest) = (chars.next()?, chars.next());
            if rest.is_some() || c.is_uppercase() {
                return None; // multi-character or already-shifted names are not keys
            }
            char_key(if shift { c.to_ascii_uppercase() } else { c })
        }
    }
}

/// The logical key a printable character arrives as.
fn character_key(c: char) -> Key {
    Key::Character(c.to_string().into())
}

/// A single character as a keystroke: a physical code where one exists, the
/// character itself as the logical key, and the text a text field would
/// insert. `None` for control characters this interface refuses to invent.
fn char_key(c: char) -> Option<(KeyCode, Key, Option<String>)> {
    let typed = |code, c: char| Some((code, character_key(c), Some(c.to_string())));
    match c {
        'a'..='z' | 'A'..='Z' => {
            let i = (c.to_ascii_uppercase() as u8 - b'A') as usize;
            typed(LETTER_CODES[i], c)
        }
        '0'..='9' => typed(DIGIT_CODES[c as usize - '0' as usize], c),
        ' ' => Some((KeyCode::Space, Key::Space, Some(" ".into()))),
        '\n' | '\r' => Some((KeyCode::Enter, Key::Enter, None)),
        '\t' => Some((KeyCode::Tab, Key::Tab, None)),
        '.' => typed(KeyCode::Period, '.'),
        ',' => typed(KeyCode::Comma, ','),
        '-' => typed(KeyCode::Minus, '-'),
        '=' => typed(KeyCode::Equal, '='),
        '/' => typed(KeyCode::Slash, '/'),
        '\\' => typed(KeyCode::Backslash, '\\'),
        ';' => typed(KeyCode::Semicolon, ';'),
        '\'' => typed(KeyCode::Quote, '\''),
        '[' => typed(KeyCode::BracketLeft, '['),
        ']' => typed(KeyCode::BracketRight, ']'),
        '`' => typed(KeyCode::Backquote, '`'),
        // Anything else (accented letters, symbols) still types: no physical
        // code, but the text is what a text field reads.
        c if !c.is_control() => Some((
            KeyCode::Unidentified(NativeKeyCode::Unidentified),
            character_key(c),
            Some(c.to_string()),
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The key names the lobby and the round actually use must parse.
    #[test]
    fn every_key_the_game_uses_parses() {
        for name in [
            "enter",
            "backspace",
            "escape",
            "space",
            "r",
            "u",
            "t",
            "m",
            "a",
        ] {
            assert!(parse_key(name, false).is_some(), "{name} must parse");
        }
        assert!(parse_key("nope", false).is_none());
    }

    /// `shift` uppercases the typed letter but keeps the physical code.
    #[test]
    fn shift_uppercases_letters() {
        let (code, logical, text) = parse_key("a", true).expect("a with shift");
        assert_eq!(code, KeyCode::KeyA);
        assert_eq!(logical, Key::Character("A".into()));
        assert_eq!(text.as_deref(), Some("A"));
    }

    /// Requests parse from the exact JSON an agent sends.
    #[test]
    fn commands_parse_from_their_json() {
        let click: Command = serde_json::from_str(r#"{"cmd":"click","x":1.5,"y":2}"#).unwrap();
        assert!(matches!(click, Command::Click { x: 1.5, y: 2.0 }));

        let key: Command = serde_json::from_str(r#"{"cmd":"key","key":"enter"}"#).unwrap();
        assert!(matches!(key, Command::Key { ref key, shift: false } if key == "enter"));

        let text: Command = serde_json::from_str(r#"{"cmd":"text","text":"hi"}"#).unwrap();
        assert!(matches!(text, Command::Text { .. }));

        assert!(serde_json::from_str::<Command>(r#"{"cmd":"wat"}"#).is_err());
    }

    /// A synthetic BGRA frame survives encoding with colours unswizzled: the
    /// one pixel must leave as PNG-encoded red, not blue.
    #[test]
    fn png_encoding_unswizzles_bgra() {
        let image = Image::new(
            bevy::render::render_resource::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            bevy::render::render_resource::TextureDimension::D2,
            vec![0, 0, 255, 255], // BGRA red
            TextureFormat::Bgra8UnormSrgb,
            bevy::asset::RenderAssetUsages::MAIN_WORLD,
        );
        let png = encode_png(&image).expect("encodes");
        assert!(png.starts_with(&[0x89, b'P', b'N', b'G']));

        let decoded = image::load_from_memory(&png).expect("decodes again");
        assert_eq!(decoded.to_rgba8().into_raw(), vec![255, 0, 0, 255]);
    }
}
