//! The lobby: one setup screen for every way a game can be played.
//!
//! The star's six wedges are the six camps; a game is whatever corners got
//! filled:
//!
//! * **Solo** (no peers in the room): the local [`Table`] says which corners
//!   a human plays and which this device's engine drives. Presets fill the
//!   symmetric seatings.
//! * **Shared room**: a click on a free wedge *is* the claim; the host may
//!   seat engines on free corners, un-seat players, and remove engines. The
//!   host starts once two or more corners are claimed. Peers who claimed
//!   nothing watch.
//!
//! The room and the player's name are typed — on desktop they are the only way
//! in. The room field accepts with Enter, which rewrites the page URL (on the
//! web) and reopens the socket against the new room; the name field applies
//! with its button. Editing is modal: while a field holds the keyboard, no
//! other system in the app sees a key. The host is the peer with the
//! lexicographically smallest `PeerId`, recomputed every frame so host loss
//! self-heals. The host owns the roster: guests announce themselves with
//! [`NetMsg::Hello`] and claim with [`NetMsg::Claim`]; everything else is the
//! host broadcasting [`NetMsg::Roster`]. One authority, so no guest ever
//! reconciles two sources of truth about which player it commands.

use bevy::asset::RenderAssetUsages;
use bevy::ecs::system::SystemParam;
use bevy::image::Image;
use bevy::input::InputSystems;
use bevy::input::keyboard::KeyboardInput;
use bevy::input_focus::tab_navigation::TabIndex;
use bevy::input_focus::{
    AcquireFocus, FocusCause, FocusGained, FocusedInput, InputFocus, InputFocusSystems,
};
use bevy::picking::events::PointerClick;
use bevy::picking::hover::Hovered;
use bevy::picking::pointer::PointerButton;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::text::{EditableText, TextCursorStyle, TextEdit};
use bevy::ui::Pressed;
use bevy::ui::{ComputedUiRenderTargetInfo, RelativeCursorPosition, UiGlobalTransform};
use bevy::ui_widgets::{Button, TextInput};
use bevy_matchbox::prelude::*;
use checkers_net::{CH_RELIABLE, NetMsg, NetState, RoomId, Seat, broadcast, decode};

use crate::board_view::player_colour;
use crate::lobby_backdrop::DEEP;
use crate::setup::Seating;
use crate::ui::{button_messages, ui_text};
use crate::{AppState, Session};
use checkers_core::position::Player;
use checkers_core::rules::Variants;

/// Push the host's roster to every peer. The roster broadcast follows every
/// roster change, so it lives in one place.
fn publish_roster(socket: &mut MatchboxSocket, net: &NetState) {
    broadcast(socket, &net.peers, &NetMsg::Roster(net.seats.clone()));
}

/// Every clickable control on the screen bar the star's own hit area. A
/// press arrives as this same value, as a message (see [`button_messages`]).
#[derive(Component, Message, Clone, Copy, PartialEq, Eq)]
pub enum LobbyButton {
    Start,
    /// Fill the table with a symmetric setup. Solo-only: a shared table is
    /// claimed corner by corner, never overwritten by one peer's shortcut.
    Preset(Seating),
    /// Declare or switch off the "no piece may rest in a foreign camp" rule.
    ForeignCamps,
    /// Perform [`CornerCommand`] on the currently selected corner.
    CornerAction(CornerCommand),
}

/// Rows only the sequencing authority (the host, or any solo table) may use:
/// starting the game, the house rules, seating engines. Hidden from guests,
/// who could only press them to be refused.
#[derive(Component, Default, Clone)]
pub struct HostOnly;

/// Rows that only make sense before the room is shared: the seating presets.
/// In a shared room every corner is claimed on the star instead.
#[derive(Component, Default, Clone)]
pub struct SoloOnly;

/// What a [`LobbyButton::CornerAction`] press asks for, against the selected
/// corner.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub enum CornerCommand {
    Human,
    Cpu,
    Off,
}

/// One star petal: which camp it stands for. Clicking selects the corner.
#[derive(Component, Default, Clone)]
pub struct CornerPetal(pub usize);

/// One star petal's text, carrying its corner so the label can follow the
/// state without being rebuilt.
#[derive(Component, Default, Clone)]
pub struct CornerText(pub usize);

#[derive(Component, Default, Clone)]
struct RosterText;

/// Which value a text field edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    Room,
    Name,
}

/// A lobby text field: Bevy's [`TextInput`] over an [`EditableText`], editing
/// the room or the player's name. A click or the field's key focuses it, Enter
/// commits, Esc leaves.
///
/// While a field holds the keyboard every key is its own and the rest of the
/// app hears none of them (see [`hold_keys_for_field`]), because the
/// alternative is that typing a room named "solo" starts a game on the `s`.
/// Only a commit changes anything: a field that lets go of the keyboard shows
/// the committed value again, so an abandoned edit cannot leave the socket
/// pointing somewhere unintended.
#[derive(Component, Clone, Copy)]
pub struct InputField(pub FieldKind);

/// The name row's accept button: commits the name field.
#[derive(Component, Message, Default, Clone, Copy)]
pub struct ApplyName;

/// The error line under a field.
#[derive(Component, Clone)]
struct InputError(FieldKind);

/// Why the last commit was refused. Shown beneath its field for as long as
/// that field holds the keyboard, with what was typed intact, to be corrected.
#[derive(Resource, Debug, Default)]
pub struct FieldRefusal(pub Option<(FieldKind, String)>);

/// One corner's state, as configured locally. Only the solo setup writes this
/// resource; a networked table is read from the roster's seats instead.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum CornerState {
    /// No one sits here.
    #[default]
    Empty,
    /// A corner played by hand on this device.
    Human,
    /// A corner played by this device's engine.
    Cpu,
}

/// The six corners as configured by this device. Read when the game is dealt
/// on a solo setup; ignored while peers share the room.
#[derive(Resource, Debug, Clone, Default)]
pub struct Table(pub [CornerState; 6]);

/// The corner currently selected for action — which petal is highlighted.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SelectedCorner(pub Option<usize>);

/// The house-rule switches the host picked, set in the lobby and read when the
/// game starts. Same resource-over-field reasoning as [`Table`], and the same
/// rule: only the host decides, since the shared game must play under one rule
/// set for every peer.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ChosenVariants(pub Variants);

/// The one-line status message under the roster: why a press was refused,
/// what a press did. Lobby UI state, deliberately not part of [`NetState`] —
/// the wire state describes the room, not this screen.
#[derive(Resource, Debug, Clone, Default)]
pub struct LobbyStatus(pub String);

/// A guest's corner claim (or release, `Some(None)`) that the host has not
/// answered yet. The answer is the host's next roster that settles it, and
/// the status line says how it went — without this, "Claiming corner 3..."
/// stayed on screen long after the corner was granted.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct PendingClaim(pub Option<Option<u32>>);

/// Long enough for a name, short enough for a roster line and a corner label.
pub const NAME_MAX_LEN: usize = 24;

impl FieldKind {
    /// The most characters the field accepts: typing stops at the limit rather
    /// than being told afterwards that it was all too long.
    pub fn max_len(self) -> usize {
        match self {
            FieldKind::Room => RoomId::MAX_LEN,
            FieldKind::Name => NAME_MAX_LEN,
        }
    }
}

pub fn plugin(app: &mut App) {
    // The room comes from the URL — a share link lands you in the sender's
    // lobby, and a bare page is redirected to a fresh generated room so the
    // address bar is always shareable. Native builds read `CCHKRS_ROOM` and
    // simply generate when it is unset. Either may then be edited in the lobby:
    // the room field accepts with Enter and rewrites the URL, the name field
    // applies its own button.
    let room = crate::web::room_from_url().unwrap_or_else(crate::web::random_room);
    crate::web::share_room(&room);
    app.insert_resource(room)
        // The session's pet name is drawn at boot and can be changed in the
        // lobby.
        .insert_resource(NetState {
            name: crate::web::petname(),
            ..NetState::default()
        })
        .init_resource::<Table>()
        .init_resource::<SelectedCorner>()
        .init_resource::<HoveredCorner>()
        .init_resource::<SectorArt>()
        .init_resource::<ChosenVariants>()
        .init_resource::<LobbyStatus>()
        .init_resource::<PendingClaim>()
        .init_resource::<checkers_net::Signaling>()
        .add_plugins((clicks_plugin, fields_plugin))
        .add_systems(
            OnEnter(AppState::Lobby),
            // The socket opens once per room, not once per lobby entry:
            // `open_socket` is a no-op while one is open. The wedge art must
            // exist before the star can reference it.
            (checkers_net::open_socket, ensure_sector_art, spawn).chain(),
        )
        .add_systems(
            Update,
            (
                // Lobby machinery stays out of the game: ungated, `pump_socket`
                // raced `net::pump` for the same socket. Everything here is
                // lobby furniture.
                (
                    elect_host,
                    pump_socket,
                    // After the roster is fresh: a name that clashes with an
                    // earlier peer's is replaced before anyone sees two.
                    settle_name_clash,
                    select_corner,
                    handle_buttons,
                    broadcast_cursor,
                )
                    .chain()
                    .run_if(in_state(AppState::Lobby)),
                (
                    sync_button_styles,
                    sync_input_styles,
                    // Host-only and solo-only rows fold away for the players
                    // who could not use them; the corner rows follow the
                    // selection.
                    sync_host_rows,
                    sync_corner_actions,
                    hover_corner,
                    // The wedge tint follows the state, and the labels follow
                    // the roster; both after the hover is settled this frame.
                    (
                        sync_corner_styles,
                        draw_corner_labels,
                        draw_roster,
                        sync_remote_cursors,
                    )
                        .chain(),
                )
                    .chain()
                    .run_if(in_state(AppState::Lobby)),
            ),
        );
}

/// The lobby's buttons and the star, reported as messages: a press on a
/// [`LobbyButton`] arrives as that `LobbyButton`, a click on the star as the
/// [`StarClick`] naming its wedge. Its own plugin so the flow tests click
/// through the same path.
pub fn clicks_plugin(app: &mut App) {
    app.add_plugins(button_messages::<LobbyButton>)
        .add_message::<StarClick>();
}

/// The text fields: their keyboard, their commits, and the Apply button.
/// Registered as the app runs it, in its own plugin so the field tests run this
/// exact registration rather than a copy of it.
pub fn fields_plugin(app: &mut App) {
    app.add_plugins(button_messages::<ApplyName>)
        .init_resource::<FieldRefusal>()
        // The field is despawned with the lobby; the keyboard must not stay
        // with it.
        .add_systems(OnExit(AppState::Lobby), release_field)
        // After Bevy turns the window's key messages into `ButtonInput`, and
        // before the same keys are handed to the focused field: the presses a
        // field takes are gone before anything in `Update` looks.
        .add_systems(
            PreUpdate,
            hold_keys_for_field
                .after(InputSystems)
                .before(InputFocusSystems::Dispatch),
        )
        .add_systems(
            Update,
            (focus_input_fields, apply_name, draw_fields)
                .chain()
                .run_if(in_state(AppState::Lobby)),
        );
}

/// Paint every non-petal button: selected mode, hover, press.
///
/// Runs every frame and writes only real changes, so hover repaints
/// immediately without dirtying the UI otherwise.
fn sync_button_styles(
    variants: Res<ChosenVariants>,
    net: Res<NetState>,
    table: Res<Table>,
    selected: Res<SelectedCorner>,
    mut buttons: Query<(&Hovered, Has<Pressed>, &LobbyButton, &mut BackgroundColor)>,
) {
    let solo = net.peers.is_empty();
    let active = selected.0.map(|i| {
        if solo {
            match &table.0[i] {
                CornerState::Empty => CornerCommand::Off,
                CornerState::Human => CornerCommand::Human,
                CornerState::Cpu => CornerCommand::Cpu,
            }
        } else {
            match net.seat_at(i as u32) {
                // A free corner reads as its empty state.
                None => CornerCommand::Off,
                // An occupied corner is an engine seat or a human claim.
                Some(seat) if seat.engine => CornerCommand::Cpu,
                Some(_) => CornerCommand::Human,
            }
        }
    });

    for (hovered, pressed, button, mut bg) in buttons.iter_mut() {
        let selected = match button {
            LobbyButton::ForeignCamps => variants.0.forbid_foreign_camps,
            LobbyButton::CornerAction(cmd) => active == Some(*cmd),
            LobbyButton::Start | LobbyButton::Preset(_) => false,
        };
        let colour = if pressed {
            if selected { CHOSEN_DOWN } else { DOWN }
        } else if hovered.get() {
            if selected { CHOSEN_HOVER } else { HOVER }
        } else if selected {
            CHOSEN
        } else {
            IDLE
        };
        bg.set_if_neq(BackgroundColor(colour));
    }
}

/// Paint the text inputs: a focused field gets a green border and a darker
/// well so it is obvious the keyboard is captured; an unfocused one brightens
/// its border on hover, so the box reads as clickable.
fn sync_input_styles(
    focus: Res<InputFocus>,
    mut inputs: Query<(Entity, &Hovered, &mut BackgroundColor, &mut BorderColor), With<InputField>>,
) {
    for (field, hovered, mut bg, mut border) in inputs.iter_mut() {
        let (well, edge) = if focus.get() == Some(field) {
            (Color::srgb(0.15, 0.15, 0.19), CHOSEN)
        } else if hovered.get() {
            (IDLE, HOVER)
        } else {
            (IDLE, BORDER)
        };
        bg.set_if_neq(BackgroundColor(well));
        border.set_if_neq(BorderColor::all(edge));
    }
}

/// Fold rows away for the players who could only press them to be refused:
/// host-only rows (Start, the house rules, seating an engine) vanish for
/// guests, and the solo presets vanish once the room is shared.
fn sync_host_rows(
    net: Res<NetState>,
    mut rows: Query<(&mut Visibility, AnyOf<(&HostOnly, &SoloOnly)>)>,
) {
    let host = net.sequences();
    let solo = net.peers.is_empty();
    for (mut visibility, (host_only, solo_only)) in &mut rows {
        let shown = host_only.is_some() && host || solo_only.is_some() && solo;
        visibility.set_if_neq(visible_if(shown));
    }
}

/// A row's visibility: shown with its parent, or hidden.
fn visible_if(shown: bool) -> Visibility {
    if shown {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    }
}

/// One button. Factored out because the lobby spawns rows of them and the
/// padding, radius, and text styling must not drift between the rows. What
/// the button *is* — its [`LobbyButton`] or [`ApplyName`] tag — is patched on
/// by the caller.
fn button(label: impl Into<String>) -> impl Scene {
    sized_button(label, (14.0, 8.0), 15.0)
}

/// A [`button`] one size smaller, to sit level with a text field: the name
/// row's Apply.
fn small_button(label: impl Into<String>) -> impl Scene {
    sized_button(label, (10.0, 6.0), 14.0)
}

/// The shape every button shares, at a given padding and font size.
fn sized_button(
    label: impl Into<String>,
    (pad_x, pad_y): (f32, f32),
    font_size: f32,
) -> impl Scene {
    let label = label.into();
    bsn! {
        Button
        Hovered
        Node {
            padding: UiRect::axes(px(pad_x), px(pad_y)),
            border_radius: BorderRadius::all(px(5)),
        }
        BackgroundColor(IDLE)
        Children [ @ui_text(label, font_size, TEXT) ]
    }
}

/// A row of controls, centred on one line.
fn control_row() -> impl Scene {
    bsn! {
        Node {
            column_gap: px(10),
            align_items: AlignItems::Center,
        }
    }
}

/// A centred column of rows, `gap` pixels apart.
fn column(gap: f32) -> impl Scene {
    bsn! {
        Node {
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            row_gap: px(gap),
        }
    }
}

/// One labelled input row: the label, the text input box, and what closes it —
/// the focusing key for the room, the Apply button and focusing key for the
/// name — and beneath it the line for a refused commit. Enter commits, Esc
/// leaves; visuals follow in [`sync_input_styles`].
fn field_row(label: &str, kind: FieldKind, key: &str, apply: bool) -> impl SceneList {
    let apply: Vec<_> = apply
        .then(|| bsn! { @small_button("Apply") ApplyName on(keep_focus) })
        .into_iter()
        .collect();
    let error = InputError(kind);
    bsn_list! {
        @control_row()
        Children [
            @ui_text(label, 14.0, MUTED)
            --
            @input_field(kind)
            --
            {apply}
            --
            @ui_text(key, 14.0, MUTED)
        ]
        --
        @ui_text("", 14.0, Color::srgb(0.85, 0.35, 0.35))
        error
    }
}

/// A text field, shared by every field row: Bevy's text input, capped at what
/// the field can commit, so typing stops at the limit rather than being told
/// afterwards that it was all too long.
pub fn input_field(kind: FieldKind) -> impl Scene {
    let field = InputField(kind);
    let text = EditableText {
        max_characters: Some(kind.max_len()),
        ..default()
    };
    bsn! {
        field
        TextInput
        text
        // Pointer focus lands only on what has a tab index.
        TabIndex(0)
        Hovered
        TextLayout::no_wrap()
        TextFont { font_size: FontSize::Px(14.0) }
        TextColor(TEXT)
        TextCursorStyle
        Node {
            width: px(240),
            padding: UiRect::axes(px(10), px(6)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(5)),
        }
        BackgroundColor(IDLE)
        BorderColor::all(BORDER)
        on(commit_on_enter)
        on(announce_editing)
    }
}

/// Apply acts on the name field's text, so pressing it must not first take
/// the keyboard from the field, as a press anywhere else does.
fn keep_focus(mut acquire: On<AcquireFocus>) {
    acquire.propagate(false);
}

/// The shared palette. `CHOSEN` marks the button whose mode is active, with
/// its own hover and press.
pub(crate) const IDLE: Color = Color::srgb(0.22, 0.22, 0.27);
pub(crate) const HOVER: Color = Color::srgb(0.29, 0.29, 0.35);
pub(crate) const DOWN: Color = Color::srgb(0.17, 0.17, 0.21);
pub(crate) const CHOSEN: Color = Color::srgb(0.20, 0.45, 0.28);
pub(crate) const CHOSEN_HOVER: Color = Color::srgb(0.25, 0.53, 0.34);
pub(crate) const CHOSEN_DOWN: Color = Color::srgb(0.16, 0.37, 0.23);

/// Text on a button or in a field, the dimmer hints beside them, and an
/// unfocused field's border.
const TEXT: Color = Color::srgb(0.9, 0.9, 0.92);
const MUTED: Color = Color::srgb(0.62, 0.62, 0.68);
const BORDER: Color = Color::srgb(0.35, 0.35, 0.40);
/// How opaque the scrim over the backdrop is: dark enough that the controls
/// read, light enough that the field behind stays alive.
const SCRIM_ALPHA: f32 = 0.55;

/// The lobby is drawn over the animated backdrop: a flat scrim first —
/// translucent near-black in the manner of omdurman's panels, dark enough
/// that the controls read, light enough that the field behind stays alive —
/// and above it the two columns, exactly as they always were.
fn spawn(mut commands: Commands, art: Res<SectorArt>) {
    commands.spawn_scene(lobby(&art));
}

/// The full-window scrim: a flat translucent near-black — omdurman's panels
/// are flat too — that dims the animated field behind it so the lobby reads.
/// Ignore pointers: clicks belong to whatever the player sees through it.
fn scrim_layer() -> impl Scene {
    let shade = Color::from(DEEP.with_alpha(SCRIM_ALPHA));
    bsn! {
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            top: px(0),
            right: px(0),
            bottom: px(0),
        }
        BackgroundColor(shade)
        Pickable::IGNORE
    }
}

fn lobby(art: &SectorArt) -> impl Scene {
    let scope = DespawnOnExit(AppState::Lobby);
    bsn! {
        scope
        Node {
            width: percent(100),
            height: percent(100),
        }
        Children [
            @scrim_layer()
            --
            @lobby_columns(art)
        ]
    }
}

/// The lobby proper: a two-column flex row filling the window — the star on
/// the left, every control on the right. Side by side the columns stay short
/// enough to fit a 600px-tall window by construction, which a single stacked
/// column never could.
fn lobby_columns(art: &SectorArt) -> impl Scene {
    // Presets are shortcuts for the symmetric setups; they fill the whole
    // table, so what is configured and what the shortcut leaves can never
    // silently disagree.
    let presets: Vec<_> = Seating::ALL
        .into_iter()
        .map(|seating| {
            let preset = LobbyButton::Preset(seating);
            bsn! { @button(format!("Preset {}", seating.label())) preset }
        })
        .collect();
    let human = LobbyButton::CornerAction(CornerCommand::Human);
    let cpu = LobbyButton::CornerAction(CornerCommand::Cpu);
    let cancel = LobbyButton::CornerAction(CornerCommand::Off);
    let (foreign, start) = (LobbyButton::ForeignCamps, LobbyButton::Start);
    bsn! {
        Node {
            width: percent(100),
            height: percent(100),
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            column_gap: px(32),
            padding: UiRect::all(px(24)),
        }
        Children [
            // Left: the setup itself.
            @column(12.0)
            Children [
                @header("Lobby")
                --
                @ui_text(
                    "The star is the setup: click a corner, then set who sits there.",
                    14.0,
                    MUTED
                )
                --
                // The hex star. Wedges are out-facing triangle sectors over a
                // single hit area; digits 1..6 select the same corners.
                @star(art)
            ]
            --
            // Right: what to do with it.
            @column(10.0)
            Children [
                // Who I am and where I am: the room to join or share, and the
                // name the roster shows. Both are typed — on desktop they are
                // the only way in — so each row is a real text field.
                @header("General")
                --
                // The room field. Enter joins: the loopback to the socket is
                // a lobby re-enter, which also rewrites the page URL (web).
                {field_row("Room", FieldKind::Room, "R", false)}
                --
                // The player-name field, same pattern; Apply commits it.
                {field_row("Name", FieldKind::Name, "N", true)}
                --
                @header("Table")
                --
                // Solo-only: a shared table is claimed corner by corner on
                // the star.
                @control_row()
                SoloOnly
                Children [ {presets} ]
                --
                // What to do with the selected corner. [`sync_corner_actions`]
                // shows exactly one of these rows: a free corner offers
                // seating, a claimed corner offers cancelling it.
                @column(10.0)
                Children [
                    // Seat a free corner. Seating an engine is the host's
                    // call alone — the engine runs in the host's process —
                    // so guests never see the button.
                    @control_row()
                    SeatButtons
                    Children [
                        @button("Human") human
                        --
                        @button("Computer") cpu HostOnly
                    ]
                    --
                    // Cancel a claimed corner — yours (release), another
                    // player's (host un-seats), or an engine's (host removes
                    // it). [`corner_effect`] picks the right move.
                    @control_row()
                    CancelSeat
                    Children [ @button("Cancel Seat") cancel ]
                ]
                --
                // House rules, one toggle per switch. Only the host decides
                // them, so only the host even sees them.
                Node { margin: UiRect::top(px(6)) }
                HostOnly
                Children [ @ui_text("Rules", 16.0, Color::srgb(0.7, 0.7, 0.76)) ]
                --
                Node { column_gap: px(10) }
                HostOnly
                Children [ @button("No foreign rest") foreign ]
                --
                // The roster: who is here and where they sit.
                @ui_text("", 14.0, Color::srgb(0.88, 0.88, 0.9))
                RosterText
                --
                // The host starts the game — the one button guests must not
                // even see, since a shared start belongs to the host alone.
                Node {
                    column_gap: px(10),
                    margin: UiRect::top(px(4)),
                }
                HostOnly
                Children [ @button("Start (Enter)") start ]
            ]
        ]
    }
}

/// Is corner `i` claimed? Solo tables read the local [`Table`]; shared rooms
/// read the roster. The one occupancy rule every view and button row asks.
fn corner_is_filled(net: &NetState, table: &Table, i: usize) -> bool {
    if net.peers.is_empty() {
        table.0[i] != CornerState::Empty
    } else {
        net.seat_at(i as u32).is_some()
    }
}

/// Marker on the Human / Computer row: shown while the selected corner is
/// free, so it offers seating.
#[derive(Component, Default, Clone)]
struct SeatButtons;

/// Marker on the Cancel Seat row: shown while the selected corner is claimed.
#[derive(Component, Default, Clone)]
struct CancelSeat;

/// Show exactly one of the two corner rows, according to the selected corner:
/// a free corner offers seating, a claimed corner offers cancelling it. With
/// nothing selected both fold away — an empty star has nothing to act on, and
/// a guest that claimed nothing is a spectator to the sidebar.
fn sync_corner_actions(
    net: Res<NetState>,
    table: Res<Table>,
    selected: Res<SelectedCorner>,
    mut seat_rows: Query<&mut Visibility, (With<SeatButtons>, Without<CancelSeat>)>,
    mut cancel_rows: Query<&mut Visibility, (With<CancelSeat>, Without<SeatButtons>)>,
) {
    let claimed = selected
        .0
        .is_some_and(|i| corner_is_filled(&net, &table, i));
    let seat_wanted = selected.0.is_some() && !claimed;
    for mut visibility in &mut seat_rows {
        visibility.set_if_neq(visible_if(seat_wanted));
    }
    for mut visibility in &mut cancel_rows {
        visibility.set_if_neq(visible_if(claimed));
    }
}

/// Seconds between cursor broadcasts, and how long a silent cursor stays on
/// screen before it is hidden as gone.
const CURSOR_INTERVAL: f32 = 0.1;
const CURSOR_LINGER_SECS: f64 = 3.0;

/// A remote peer's pointer drawn over the lobby: where it was last reported
/// (already scaled into this window's logical pixels), where it is drawn
/// (eased toward the report so it glides rather than jumps), and when it was
/// last heard from.
#[derive(Component, Clone)]
pub struct RemoteCursor {
    peer: String,
    target: Vec2,
    display: Vec2,
    last_seen: f64,
}

/// A remote pointer: a dot in its peer's colour with the peer's name beside
/// it. Drawn over the lobby but never in its way — a click lands on whatever
/// is under the dot, not on the dot.
fn remote_cursor(cursor: RemoteCursor, net: &NetState) -> impl Scene {
    let (colour, label) = cursor_identity(net, &cursor.peer);
    let (left, top) = (px(cursor.display.x), px(cursor.display.y));
    let scope = DespawnOnExit(AppState::Lobby);
    bsn! {
        scope
        cursor
        Pickable::IGNORE
        Node {
            position_type: PositionType::Absolute,
            left: left,
            top: top,
            width: px(10),
            height: px(10),
            border_radius: BorderRadius::all(px(5)),
        }
        BackgroundColor(colour)
        Children [
            @ui_text(label, 12.0, colour)
            Pickable::IGNORE
            Node {
                position_type: PositionType::Absolute,
                left: px(12),
                top: px(-4),
            }
        ]
    }
}

/// Broadcast this pointer at a lazy 10 Hz while the lobby is up, as
/// **fractions of the window's width and height** (0..1, origin top-left).
/// Windows differ in size, so absolute pixels would drift apart across peers;
/// fractions land every pointer at the same relative spot, and each receiver
/// scales them back into its own logical pixels — the space bevy_ui lays out
/// in. Matchbox connects every peer to every peer, so a plain broadcast
/// reaches the whole room; the sender is the `from` on arrival and no origin
/// field is needed.
fn broadcast_cursor(
    time: Res<Time>,
    windows: Query<&Window>,
    net: Res<NetState>,
    socket: Option<ResMut<MatchboxSocket>>,
    mut next_at: Local<f32>,
) {
    let Some(mut socket) = socket else {
        return;
    };
    if net.peers.is_empty() || time.elapsed_secs() < *next_at {
        return;
    }
    *next_at = time.elapsed_secs() + CURSOR_INTERVAL;
    let Some(window) = windows.single().ok() else {
        return;
    };
    let Some(pos) = window.cursor_position() else {
        return;
    };
    let (width, height) = (window.width(), window.height());
    if width <= 0.0 || height <= 0.0 {
        return;
    }
    let fractions = [pos.x / width, pos.y / height];
    broadcast(&mut socket, &net.peers, &NetMsg::Cursor { pos: fractions });
}

/// A cursor's colour is the corner its peer claimed — the same colour the
/// wedge wears — and its label the roster name.
fn cursor_identity(net: &NetState, peer: &str) -> (Color, String) {
    match net.seats.iter().find(|s| s.peer == peer) {
        Some(seat) => {
            let player = seat.player.and_then(|i| Player::new(i as u8));
            (player.map_or(IDLE, player_colour), seat.name.clone())
        }
        None => (IDLE, format!("peer {}", &peer[..peer.len().min(4)])),
    }
}

/// Draw every remote cursor where its peer's pointer is, and keep the label
/// and colour following the roster.
#[allow(clippy::type_complexity)]
fn sync_remote_cursors(
    time: Res<Time>,
    net: Res<NetState>,
    mut cursors: Query<(
        &mut RemoteCursor,
        &mut Node,
        &mut BackgroundColor,
        &mut Visibility,
        &Children,
    )>,
    mut labels: Query<(&mut Text, &mut TextColor), Without<RemoteCursor>>,
) {
    // Exponential smoothing: fast enough to follow, slow enough to hide
    // the 10 Hz steps.
    let alpha = 1.0 - (-6.0 * time.delta_secs()).exp();
    let now = time.elapsed_secs_f64();
    for (mut cursor, mut node, mut bg, mut vis, kids) in cursors.iter_mut() {
        cursor.display = cursor.display.lerp(cursor.target, alpha);
        node.left = px(cursor.display.x);
        node.top = px(cursor.display.y);
        let want = if now - cursor.last_seen > CURSOR_LINGER_SECS {
            Visibility::Hidden
        } else {
            Visibility::Visible
        };
        vis.set_if_neq(want);
        let (colour, label) = cursor_identity(&net, &cursor.peer);
        bg.set_if_neq(BackgroundColor(colour));
        for kid in kids.iter() {
            if let Ok((mut text, mut text_colour)) = labels.get_mut(kid) {
                if **text != label {
                    **text = label.clone();
                }
                text_colour.set_if_neq(TextColor(colour));
            }
        }
    }
}

/// A section heading in the lobby.
fn header(label: &str) -> impl Scene {
    ui_text(label, 20.0, Color::srgb(0.92, 0.92, 0.95))
}

/// Star geometry: the container the wedges and their labels live in.
const STAR_W: f32 = 420.0;
const STAR_H: f32 = 360.0;
const STAR_CENTRE: Vec2 = Vec2::new(STAR_W / 2.0, STAR_H / 2.0);

/// Wedge geometry: each corner is an **equilateral** triangle pointing
/// outward, like the board's camp triangles. With the apex at
/// [`WEDGE_OUTER`] on the camp's axis and the base corners on the ±30° rays
/// at `WEDGE_OUTER / √3`, the triangle is exactly equilateral and the six
/// wedges meet corner-to-corner in a rosette — the star's silhouette — while
/// the middle stays open. Labels may overhang the wedges.
const WEDGE_OUTER: f32 = 175.0;

/// Corner `i`'s direction, in container coordinates (y down), so the angle
/// arithmetic matches [`Window::cursor_position`] directly.
fn wedge_angle(i: usize) -> f32 {
    (60.0 * i as f32 - 90.0).to_radians()
}

/// The three corners of corner `i`'s wedge, in container-local pixels: the
/// outward tip first, then the two base corners. Apex on the axis at
/// [`WEDGE_OUTER`], base corners on the ±30° rays at `WEDGE_OUTER / √3` —
/// the arrangement that makes the triangle equilateral.
fn wedge_vertices(i: usize) -> [Vec2; 3] {
    let t = wedge_angle(i);
    let beta = 30f32.to_radians();
    let apex = STAR_CENTRE + WEDGE_OUTER * Vec2::new(t.cos(), t.sin());
    let base_r = WEDGE_OUTER / 3.0f32.sqrt();
    let left = STAR_CENTRE + base_r * Vec2::new((t - beta).cos(), (t - beta).sin());
    let right = STAR_CENTRE + base_r * Vec2::new((t + beta).cos(), (t + beta).sin());
    [apex, left, right]
}

/// Which side of the line `a -> b` the point `p` is on, as a signed area.
fn cross(a: Vec2, b: Vec2, p: Vec2) -> f32 {
    (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x)
}

/// Is `p` inside the triangle `v`? Sign tests, so an exact wedge hit needs no
/// rounding: the click area is the triangle, not its bounding box.
fn point_in_triangle(p: Vec2, v: [Vec2; 3]) -> bool {
    let (s1, s2, s3) = (
        cross(v[0], v[1], p),
        cross(v[1], v[2], p),
        cross(v[2], v[0], p),
    );
    (s1 >= 0.0 && s2 >= 0.0 && s3 >= 0.0) || (s1 <= 0.0 && s2 <= 0.0 && s3 <= 0.0)
}

/// Which corner a point on the star is over. `normalized` is the
/// centre-relative position [`RelativeCursorPosition`] reports (`-0.5 .. 0.5`,
/// y down); it is mapped back into container coordinates so the wedges' own
/// geometry can answer. `None` over the empty middle — the middle is the
/// star, not a button.
fn sector_at(normalized: Vec2) -> Option<usize> {
    let local = normalized * vec2(STAR_W, STAR_H) + STAR_CENTRE;
    (0..6).find(|&i| point_in_triangle(local, wedge_vertices(i)))
}

/// The centroid of corner `i`'s wedge, where its two label lines sit.
fn wedge_centroid(i: usize) -> Vec2 {
    let v = wedge_vertices(i);
    (v[0] + v[1] + v[2]) / 3.0
}

/// The star's textures, rasterized once on first lobby entry.
#[derive(Resource, Default)]
struct SectorArt(Option<SectorImages>);

struct SectorImages {
    /// Each wedge, white where the wedge is and transparent elsewhere, so the
    /// [`ImageNode`] tint paints its colour and hover stays a colour write.
    fills: [Handle<Image>; 6],
    /// Each wedge's selection outline, in its own colours, shown untinted.
    outlines: [Handle<Image>; 6],
}

/// The selection outline's two bands, in texture pixels: a dark outer band
/// and a light inner one, so the ring reads on every wedge colour — the dark
/// empty wedges and the white corner alike.
const OUTLINE_DARK: f32 = 2.0;
const OUTLINE_LIGHT: f32 = 3.0;

/// Rasterize corner `i`'s wedge into a full-star-size RGBA texture.
fn sector_image(i: usize) -> Image {
    let v = wedge_vertices(i);
    star_image(|p| point_in_triangle(p, v).then_some([255, 255, 255, 255]))
}

/// Rasterize corner `i`'s selection outline: the two-tone band just *inside*
/// the wedge's edges, so it never spills onto the neighbours the wedge
/// touches at its corners.
fn outline_image(i: usize) -> Image {
    let v = wedge_vertices(i);
    star_image(|p| {
        if !point_in_triangle(p, v) {
            return None;
        }
        let d = edge_distance(p, v);
        if d < OUTLINE_DARK {
            Some([0, 0, 0, 255])
        } else if d < OUTLINE_DARK + OUTLINE_LIGHT {
            Some([255, 255, 255, 255])
        } else {
            None
        }
    })
}

/// How far `p` is from the nearest edge of the triangle `v`.
fn edge_distance(p: Vec2, v: [Vec2; 3]) -> f32 {
    (0..3)
        .map(|k| {
            let (a, b) = (v[k], v[(k + 1) % 3]);
            cross(a, b, p).abs() / a.distance(b)
        })
        .fold(f32::INFINITY, f32::min)
}

/// A full-star-size RGBA texture, painted pixel by pixel: `paint` gives a
/// pixel centre its colour, or `None` for transparent.
fn star_image(paint: impl Fn(Vec2) -> Option<[u8; 4]>) -> Image {
    let (w, h) = (STAR_W as u32, STAR_H as u32);
    let mut data = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            if let Some(rgba) = paint(Vec2::new(x as f32 + 0.5, y as f32 + 0.5)) {
                let o = ((y * w + x) * 4) as usize;
                data[o..o + 4].copy_from_slice(&rgba);
            }
        }
    }
    Image::new(
        Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// Build [`SectorArt`] once; a no-op on every lobby re-entry.
fn ensure_sector_art(mut art: ResMut<SectorArt>, mut images: ResMut<Assets<Image>>) {
    if art.0.is_some() {
        return;
    }
    art.0 = Some(SectorImages {
        fills: std::array::from_fn(|i| images.add(sector_image(i))),
        outlines: std::array::from_fn(|i| images.add(outline_image(i))),
    });
}

/// The eight offsets the label's black border copies take around the white
/// fill (a ~1px ring). Bevy's text pipeline has no stroke, so a legible
/// border over the coloured wedges is a shadow ring of copies.
const OUTLINE_OFFSETS: [Vec2; 8] = [
    Vec2::new(1.0, 0.0),
    Vec2::new(-1.0, 0.0),
    Vec2::new(0.0, 1.0),
    Vec2::new(0.0, -1.0),
    Vec2::new(0.71, 0.71),
    Vec2::new(0.71, -0.71),
    Vec2::new(-0.71, 0.71),
    Vec2::new(-0.71, -0.71),
];

/// One outlined label line: the white fill copy on top, black copies offset
/// around it underneath. All copies carry the same string —
/// [`draw_corner_labels`] writes it into every one via the [`CornerText`]
/// marker on the stack. Each copy stretches the stack's width and centres its
/// glyphs, so the copies cannot drift apart.
fn outlined_line(tag: usize, font_size: f32, fill: Color) -> impl Scene {
    let copies: Vec<_> = OUTLINE_OFFSETS
        .into_iter()
        .map(|offset| outlined_copy(offset, font_size, Color::BLACK))
        .chain([outlined_copy(Vec2::ZERO, font_size, fill)])
        .collect();
    bsn! {
        Node {
            width: percent(100),
            height: px(font_size * 1.3),
        }
        CornerText(tag)
        Children [ {copies} ]
    }
}

/// One copy of an outlined label: a full-size flex box, shifted by `offset`,
/// with the text dead-centred inside it. Making the box fill the stack on
/// every side (`top: y` with `bottom: -y`) keeps it full-height while the
/// whole line slides by the outline offset, so the glyphs stay centred — the
/// flex centring below is exact and needs no knowledge of the font's line
/// metrics.
fn outlined_copy(offset: Vec2, font_size: f32, colour: Color) -> impl Scene {
    bsn! {
        Node {
            position_type: PositionType::Absolute,
            left: px(offset.x),
            right: px(-offset.x),
            top: px(offset.y),
            bottom: px(-offset.y),
            display: Display::Flex,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
        }
        Children [
            @ui_text("", font_size, colour)
            TextLayout::new(Justify::Center, LineBreak::NoWrap)
        ]
    }
}

/// The star: one button the size of the star, holding one out-facing wedge
/// per corner and its two label lines. The container itself is the only
/// button — hit-testing is angular, against [`sector_at`], and a press on a
/// wedge or a label bubbles up to it — and the middle stays empty, which is
/// what makes the star read as a star.
fn star(art: &SectorArt) -> impl Scene {
    let art = art
        .0
        .as_ref()
        .expect("sector art is built before the lobby spawns");
    let fills: Vec<_> = art
        .fills
        .iter()
        .cloned()
        .enumerate()
        .map(|(i, image)| bsn! { @full_star() ImageNode { image: image } CornerPetal(i) })
        .collect();
    // Over the fills, under the labels; only the selected one shows.
    let outlines: Vec<_> = art
        .outlines
        .iter()
        .cloned()
        .enumerate()
        .map(|(i, image)| {
            bsn! {
                @full_star()
                ImageNode { image: image }
                Visibility::Hidden
                CornerOutline(i)
            }
        })
        .collect();
    let labels: Vec<_> = (0..6).map(corner_label).collect();
    bsn! {
        StarHit
        RelativeCursorPosition
        Node {
            width: px(STAR_W),
            height: px(STAR_H),
        }
        on(click_star)
        Children [
            {fills}
            --
            {outlines}
            --
            {labels}
        ]
    }
}

/// A layer covering the whole star, for the wedge fills and outlines.
fn full_star() -> impl Scene {
    bsn! {
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            top: px(0),
            width: px(STAR_W),
            height: px(STAR_H),
        }
    }
}

/// Corner `i`'s two label lines, centred on its wedge: white fill over a
/// black border, so they stay legible over whichever colour the wedge wears.
fn corner_label(i: usize) -> impl Scene {
    let mid = wedge_centroid(i);
    bsn! {
        Node {
            position_type: PositionType::Absolute,
            left: px(mid.x - 55.0),
            top: px(mid.y - 18.0),
            width: px(110),
            height: px(36),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            row_gap: px(1),
        }
        Children [
            @outlined_line(i * 2, 14.0, Color::srgb(0.96, 0.96, 0.97))
            --
            @outlined_line(i * 2 + 1, 11.0, Color::srgb(0.85, 0.85, 0.88))
        ]
    }
}

/// One wedge's selection outline, visible while its corner is selected.
#[derive(Component, Default, Clone)]
struct CornerOutline(usize);

/// Marker on the star's single hit area: the whole container takes the
/// clicks, and they are resolved to a corner by angle, not by rectangles.
#[derive(Component, Default, Clone, Copy)]
pub struct StarHit;

/// A click on the star: the wedge it landed on, or `None` for the star's
/// empty middle.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct StarClick(pub Option<usize>);

/// Resolve a click on the star (or on a wedge or label inside it) to a wedge,
/// from where the click itself happened.
///
/// The click's own position rather than [`RelativeCursorPosition`]: a click
/// completes on release, and on a touch screen there is no cursor by then —
/// the finger has lifted, so the star would report no position at all.
fn click_star(
    click: On<PointerClick>,
    star: Query<
        (
            &ComputedNode,
            &UiGlobalTransform,
            &ComputedUiRenderTargetInfo,
        ),
        With<StarHit>,
    >,
    ui_scale: Res<UiScale>,
    mut clicks: MessageWriter<StarClick>,
) {
    if click.button != PointerButton::Primary {
        return;
    }
    let Ok((node, transform, target)) = star.get(click.entity) else {
        return;
    };
    // The pointer reports logical window pixels; the node's transform works
    // in the camera's physical ones, and `scale_factor` includes `UiScale`.
    let physical = click.pointer.position * target.scale_factor() / ui_scale.0;
    let at = node.normalize_point(*transform, physical);
    clicks.write(StarClick(at.and_then(sector_at)));
}

/// Which corner the cursor currently rests on, for wedge shading.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
struct HoveredCorner(Option<usize>);

/// What pressing Enter should do.
///
/// Split out from `handle_buttons` because it is the part that decides whether
/// a board can exist at all: solo or shared, the board is only spawned on
/// entering [`AppState::InGame`], so a refusal that cannot be acted on looks
/// like a blank screen.
#[derive(Debug, PartialEq, Eq)]
pub enum StartDecision {
    /// Nobody else is here: deal the configured corners on this device.
    Solo,
    /// The room is full enough and ready: tell the peers and begin.
    Multiplayer,
    /// Refuse, with a reason to show the player.
    Refuse(String),
}

pub fn start_decision(net: &NetState, table: &Table) -> StartDecision {
    let configured = table.0.iter().filter(|c| **c != CornerState::Empty).count();

    if net.peers.is_empty() {
        if configured < 2 {
            return StartDecision::Refuse(
                "At least two corners must be filled before the game can start.".into(),
            );
        }
        return StartDecision::Solo;
    }

    // A guest cannot start a *shared* game, and must be told so rather than
    // having Enter do nothing.
    if !net.sequences() {
        return StartDecision::Refuse(
            "Only the host can start a shared game. Claim a corner and wait.".into(),
        );
    }

    if seated(net) < 2 {
        return StartDecision::Refuse(
            "At least two corners must be claimed before the game can start.".into(),
        );
    }
    StartDecision::Multiplayer
}

/// Smallest `PeerId` hosts. Recomputed every frame so host loss self-heals.
///
/// `pub` so the multiplayer integration test can run the real election in a
/// headless instance, exactly as the app schedules it.
pub fn elect_host(socket: Option<ResMut<MatchboxSocket>>, mut net: ResMut<NetState>) {
    let Some(mut socket) = socket else {
        return;
    };

    crate::net::sync_peers(&mut socket, &mut net);

    // Read before writing, here and below: this runs every frame, and a write
    // marks the state changed for every system that redraws on a change.
    if net.my_id.is_none()
        && let Some(id) = socket.id()
    {
        net.my_id = Some(id);
    }
    let Some(me) = net.my_id else {
        return;
    };

    let is_host = net.peers.iter().all(|p| me.to_string() < p.to_string());
    if is_host == net.is_host {
        return;
    }
    net.is_host = is_host;
    if is_host {
        net.next_seq = net.last_applied_seq.map_or(0, |s| s + 1);
        info!(name = %net.name, "became host");
    } else {
        info!("relinquished the host");
    }
}

/// The world one lobby conversation reads and mutates: the roster and house
/// rules being negotiated, and the window/cursor furniture for peer pointers.
/// One parameter in place of eight: both lobby pumps would otherwise cross
/// the clippy argument limit, and being a unit of type keeps `pump_socket` and `handle_buttons` from
/// drifting apart. The socket is not here: it is an optional input, so each
/// system takes it as its own parameter.
#[derive(SystemParam)]
pub struct LobbyWorld<'w, 's> {
    pub net: ResMut<'w, NetState>,
    pub status: ResMut<'w, LobbyStatus>,
    pub pending: ResMut<'w, PendingClaim>,
    pub variants: ResMut<'w, ChosenVariants>,
    pub selected: ResMut<'w, SelectedCorner>,
    pub table: ResMut<'w, Table>,
    pub next_state: ResMut<'w, NextState<AppState>>,
    pub state: Res<'w, State<AppState>>,
    pub commands: Commands<'w, 's>,
    pub time: Res<'w, Time>,
    pub windows: Query<'w, 's, &'static Window>,
    pub cursors: Query<'w, 's, (Entity, &'static mut RemoteCursor)>,
}

/// The whole lobby conversation, one message at a time: greetings, corner
/// claims, the host's roster broadcasts, and the `Start` that moves everyone
/// into the game. Runs on the socket every frame.
///
/// `pub` so the multiplayer integration test can run the real pump in a
/// headless instance, exactly as the app schedules it.
pub fn pump_socket(socket: Option<ResMut<MatchboxSocket>>, lobby: LobbyWorld) {
    let LobbyWorld {
        mut net,
        mut status,
        mut pending,
        mut variants,
        mut next_state,
        state,
        mut commands,
        time,
        windows,
        mut cursors,
        ..
    } = lobby;

    let Some(mut socket) = socket else {
        return;
    };

    // Announce ourselves to peers we have not greeted yet: once when we first
    // have a name, and afterwards only to peers that join later.
    let me = net.my_id.map(|id| id.to_string()).unwrap_or_default();

    // The host prunes seats whose peer has left the mesh: a refresh (or a
    // crashed tab) mints a fresh peer id, so the old seat would otherwise sit
    // in the roster forever, and the list only ever grew. Checked before it
    // writes, and published only on an actual change, so an idle room costs
    // nothing — not even a changed `NetState`.
    if net.sequences() && has_departed(&net) {
        let seats = net.seats.len();
        prune_departed(&mut net);
        if net.seats.len() != seats {
            publish_roster(&mut socket, &net);
        }
    }

    if net.name.is_empty() && !me.is_empty() {
        net.name = format!("player-{}", &me[..me.len().min(4)]);
    }
    let unacquainted: Vec<PeerId> = net
        .peers
        .iter()
        .filter(|p| !net.greeted.contains(p))
        .copied()
        .collect();
    if !net.name.is_empty() && !unacquainted.is_empty() {
        let hello = NetMsg::Hello {
            name: net.name.clone(),
        };
        broadcast(&mut socket, &unacquainted, &hello);
        // The host's own seat is created here, by the Hello it never
        // receives. A re-greet is also a rename: the host edited its name and
        // cleared `greeted`, so the existing seat must follow suit — guests
        // ignore a Hello (only the sequencer acts on one), and nothing else
        // would republish the roster with the new name.
        if net.sequences() && sync_host_seat(&mut net, &me) {
            publish_roster(&mut socket, &net);
        }
        info!(name = %net.name, greeted = unacquainted.len(), "greeted the room");
        net.greeted.extend(unacquainted);
    }

    let inbox: Vec<(PeerId, Box<[u8]>)> = socket.channel_mut(CH_RELIABLE).receive();
    for (from, raw) in inbox {
        let Some(msg) = decode(&raw) else {
            continue;
        };
        match msg {
            NetMsg::Hello { name } => {
                info!(%from, %name, "peer greeted the room");
                if net.sequences() {
                    // A repeat Hello from a known peer is a rename, not a
                    // duplicate join: the name field re-greets precisely so
                    // this happens.
                    if let Some(seat) = net.seats.iter_mut().find(|s| s.peer == from.to_string()) {
                        if seat.name != name {
                            seat.name = name;
                            publish_roster(&mut socket, &net);
                        }
                    } else {
                        seat_for(&mut net, &from.to_string(), &name);
                        publish_roster(&mut socket, &net);
                    }
                }
            }
            NetMsg::Claim(claim) => {
                if net.sequences() {
                    let key = from.to_string();
                    let corner = claim.filter(|&c| c < 6);
                    // Read the roster before mutating it: the grant must see
                    // the corners as the peers do, and the sender's own hold.
                    let grantable = corner.is_some_and(|c| {
                        let free = net.seat_at(c).is_none();
                        let mine = net
                            .seats
                            .iter()
                            .any(|s| s.peer == key && s.player == Some(c));
                        free || mine
                    });
                    if let Some(seat) = net.seats.iter_mut().find(|s| s.peer == key) {
                        match corner {
                            // Grant a free corner, or the sender's own; a
                            // move to a new corner frees the old one on the
                            // same grant.
                            Some(corner) if grantable => {
                                seat.player = Some(corner);
                                status.0 = format!("{} claimed corner {corner}.", seat.name);
                                info!(name = %seat.name, corner, "claimed a corner");
                            }
                            // Someone else's corner: the seat stays as it was.
                            Some(_) => {}
                            // Releasing, or an impossible corner number: drop the claim.
                            None => seat.player = None,
                        }
                    }
                    publish_roster(&mut socket, &net);
                }
            }
            // Guests take the host's roster verbatim; it is the only authority.
            // A roster that settles this peer's claim answers it on the status
            // line.
            NetMsg::Roster(seats) => {
                net.seats = seats;
                if let Some(claim) = pending.0
                    && let Some(answer) = claim_answer(&net, claim)
                {
                    status.0 = answer;
                    pending.0 = None;
                }
            }
            // The host's rule switch, live. The host is the only authority, so
            // a sequencing peer ignores its own echo; guests take it verbatim.
            NetMsg::Variants {
                forbid_foreign_camps,
            } => {
                if !net.sequences() {
                    variants.0.forbid_foreign_camps = forbid_foreign_camps;
                }
            }
            // A peer's pointer, drawn over the lobby only — the game has its
            // own presentation, and leftover dots must not haunt it. The wire
            // carries fractions of the sender's window; scale them into this
            // window's pixels so the dot sits at the same relative spot.
            NetMsg::Cursor { pos } => {
                if *state.get() != AppState::Lobby {
                    continue;
                }
                let Some(window) = windows.single().ok() else {
                    continue;
                };
                let pos = Vec2::new(pos[0] * window.width(), pos[1] * window.height());
                let key = from.to_string();
                let now = time.elapsed_secs_f64();
                if let Some((_, mut cursor)) = cursors.iter_mut().find(|(_, c)| c.peer == key) {
                    cursor.target = pos;
                    cursor.last_seen = now;
                } else {
                    let cursor = RemoteCursor {
                        peer: key,
                        target: pos,
                        display: pos,
                        last_seen: now,
                    };
                    commands.spawn_scene(remote_cursor(cursor, &net));
                }
            }
            NetMsg::Start {
                seats,
                forbid_foreign_camps,
            } => accept_start(
                &mut net,
                &mut variants,
                &mut next_state,
                seats,
                forbid_foreign_camps,
            ),
            // Moves cannot arrive before the game starts, but a late duplicate
            // from a previous game in the same room could. Ignore rather than
            // mis-apply.
            NetMsg::Move(_) | NetMsg::Sequenced { .. } | NetMsg::Abandon => {}
        }
    }
}

/// Whether another peer in the room already goes by `name`. Engines are called
/// "Engine" and are not peers, so they never clash.
fn name_taken(net: &NetState, name: &str) -> bool {
    let me = net.my_id.map(|id| id.to_string());
    net.seats
        .iter()
        .any(|s| !s.engine && Some(&s.peer) != me.as_ref() && s.name == name)
}

/// Keep names unique in the room: the roster, the corner labels and the
/// cursors could not tell two peers of one name apart.
///
/// A rename to a taken name is refused (see `commit_name`), but pet names are
/// drawn before peers meet, so two can clash on joining. The peer with the
/// larger id then draws a free pet name and re-greets, so the host's roster
/// follows. The host has the smallest id and never has to give way, and of
/// two guests exactly one does.
pub fn settle_name_clash(mut net: ResMut<NetState>, mut status: ResMut<LobbyStatus>) {
    let Some(name) = clash_rename(&net) else {
        return;
    };
    status.0 = format!(
        "Someone here was already called \"{}\" - you are \"{name}\" now.",
        net.name
    );
    info!(from = %net.name, to = %name, "renamed to settle a name clash");
    net.name = name;
    net.greeted.clear();
}

/// The name to switch to when an earlier peer — one with a smaller id —
/// already goes by this peer's name; `None` while the name is this peer's
/// alone. Reads only, so an idle room leaves `NetState` untouched.
fn clash_rename(net: &NetState) -> Option<String> {
    let me = net.my_id?.to_string();
    let clash = net
        .seats
        .iter()
        .any(|s| !s.engine && s.peer < me && s.name == net.name);
    clash.then(|| {
        let taken: Vec<&str> = net.seats.iter().map(|s| s.name.as_str()).collect();
        crate::web::petname_avoiding(&taken)
    })
}

/// Take the host's `Start`: the final roster and the house rules, then deal the
/// round. Shared by the lobby's pump and the game's, because the socket
/// outlives the round: a peer still looking at the last game-over card is
/// pulled into the rematch exactly like one waiting in the lobby.
pub(crate) fn accept_start(
    net: &mut NetState,
    variants: &mut ChosenVariants,
    next_state: &mut NextState<AppState>,
    seats: Vec<Seat>,
    forbid_foreign_camps: bool,
) {
    net.seats = seats;
    variants.0.forbid_foreign_camps = forbid_foreign_camps;
    next_state.set(AppState::InGame);
    info!(seated = seated(net), "received Start - entering the game");
}

/// How many seats hold a corner: the players of the round the roster would
/// deal. Spectators have seats too, and are not counted.
fn seated(net: &NetState) -> usize {
    net.seats.iter().filter(|s| s.player.is_some()).count()
}

/// Drop seats whose peer has left the room, and greeting memory of peers that
/// are gone. A refresh mints a fresh `PeerId`, so without this the old seat
/// would sit in the roster forever. Engine seats have no peer behind them —
/// they belong to the host — so they survive, and so does the host's own
/// seat: matchbox's peer list is *other* peers and never names yourself.
/// Pure, so the rule is testable without a socket; the host runs it every
/// lobby frame.
fn prune_departed(net: &mut NetState) {
    let me = net.my_id.map(|id| id.to_string());
    net.seats
        .retain(|s| seat_present(s, me.as_deref(), &net.peers));
    net.greeted.retain(|p| net.peers.contains(p));
}

/// Whether [`prune_departed`] would change anything.
fn has_departed(net: &NetState) -> bool {
    let me = net.my_id.map(|id| id.to_string());
    net.seats
        .iter()
        .any(|s| !seat_present(s, me.as_deref(), &net.peers))
        || net.greeted.iter().any(|p| !net.peers.contains(p))
}

/// Whether a seat's owner is still in the room: an engine (the host's own),
/// this peer itself, or a connected peer.
fn seat_present(seat: &Seat, me: Option<&str>, peers: &[PeerId]) -> bool {
    seat.engine
        || Some(seat.peer.as_str()) == me
        || peers.iter().any(|p| p.to_string() == seat.peer)
}

/// Add a seat if this peer has none. A new seat claims nothing; the corner is
/// the seat's owner's to take, with [`NetMsg::Claim`].
fn seat_for(net: &mut NetState, peer: &str, name: &str) {
    if net.seats.iter().any(|s| s.peer == peer) {
        return;
    }
    net.seats.push(Seat::human(peer, name, None));
}

/// Mirror the host's current name into its own seat. The host's seat is only
/// ever *created* by its own greet; on a re-greet — a rename within the room —
/// it must be updated by hand, because guests ignore the host's Hello, so
/// nobody else would republish the roster. Modelling the room, not the
/// host's seat, is the caller's: return whether the roster changed and must
/// be broadcast.
fn sync_host_seat(net: &mut NetState, me: &str) -> bool {
    let Some(seat) = net.seats.iter_mut().find(|s| s.peer == me) else {
        seat_for(net, me, &net.name.clone());
        return false;
    };
    if seat.name != net.name {
        seat.name = net.name.clone();
        return true;
    }
    false
}

/// Seat an engine at a specific corner: a roster entry no peer commands,
/// ready from the moment it sits, which `Start` reads like any other claim.
/// Host-only — the engine is driven by the host's own process.
fn seat_engine_at(net: &mut NetState, corner: usize) {
    let n = net.seats.iter().filter(|s| s.engine).count();
    net.seats.push(Seat {
        peer: format!("engine-{n}"),
        name: "Engine".into(),
        player: Some(corner as u32),
        engine: true,
    });
}

/// Remove the engine from a corner, if one sits there.
fn remove_engine_at(net: &mut NetState, corner: usize) {
    net.seats
        .retain(|s| !(s.engine && s.player == Some(corner as u32)));
}

/// The camps this peer's own engine should drive, from the roster's engine
/// seats. Only the sequencing authority - the host - runs engines: its moves
/// reach every peer as ordinary sequenced moves, and a guest that also ran
/// them would drive the same seat twice.
pub fn engine_camps(net: &NetState) -> Vec<Player> {
    if !net.sequences() {
        return Vec::new();
    }
    net.seats
        .iter()
        .filter(|s| s.engine)
        .filter_map(|s| s.player)
        .filter_map(|i| Player::new(i as u8))
        .collect()
}

/// The `Start` the host broadcasts: the final roster and the house-rule
/// toggle. Receivers deal from `seats`, so the roster is the single source
/// for who sits where.
pub fn start_message(net: &NetState, variants: Variants) -> NetMsg {
    NetMsg::Start {
        seats: net.seats.clone(),
        forbid_foreign_camps: variants.forbid_foreign_camps,
    }
}

/// Which corner a keypress selects. `1`..`6` name the camps directly.
fn corner_from_keys(keys: &ButtonInput<KeyCode>, current: Option<usize>) -> Option<usize> {
    let pressed = CORNER_KEYS.iter().position(|&key| keys.just_pressed(key));
    pressed.or(current)
}

/// The key that selects each corner, in corner order.
const CORNER_KEYS: [KeyCode; 6] = [
    KeyCode::Digit1,
    KeyCode::Digit2,
    KeyCode::Digit3,
    KeyCode::Digit4,
    KeyCode::Digit5,
    KeyCode::Digit6,
];

/// What clicking corner `sector` does, given the room.
///
/// Solo setups select the corner for the sidebar buttons. Shared rooms claim
/// an empty corner on the spot — clicking the triangle *is* the claim — and a
/// claimed corner is selected for the sidebar, whoever holds it: your own to
/// cancel, another player's or an engine's for the host to cancel for them.
#[derive(Debug, PartialEq, Eq)]
pub enum SectorClick {
    Select(usize),
    Claim(u32),
}

pub fn sector_click(net: &NetState, sector: usize) -> SectorClick {
    if net.peers.is_empty() || net.seat_at(sector as u32).is_some() {
        SectorClick::Select(sector)
    } else {
        SectorClick::Claim(sector as u32)
    }
}

/// Apply a claim: the host edits its own roster and republishes; a guest asks
/// the host and waits for the roster. Returns the status line.
fn send_claim(
    socket: &mut MatchboxSocket,
    net: &mut NetState,
    pending: &mut PendingClaim,
    claim: Option<u32>,
    me: &str,
) -> String {
    if net.sequences() {
        if let Some(seat) = net.seats.iter_mut().find(|s| s.peer == me) {
            seat.player = claim;
        }
        publish_roster(socket, net);
        pending.0 = None;
        return match claim {
            Some(c) => format!("You hold corner {c}."),
            None => "Corner released.".into(),
        };
    }
    broadcast(socket, &net.peers, &NetMsg::Claim(claim));
    // Answered by the host's roster; see `claim_answer`.
    pending.0 = Some(claim);
    match claim {
        Some(c) => format!("Claiming corner {c}..."),
        None => "Releasing my corner...".into(),
    }
}

/// Whether this roster settles a guest's pending claim, and how to say so.
///
/// `None` while the roster says nothing either way: a roster the host sent
/// for someone else's change can arrive before the host has even seen this
/// claim, so only a roster that grants it — or shows the corner held by
/// someone else, which the host will not undo — is an answer.
pub fn claim_answer(net: &NetState, claim: Option<u32>) -> Option<String> {
    let mine = net.my_seat().and_then(|s| s.player);
    match claim {
        Some(c) if mine == Some(c) => Some(format!("You hold corner {c}.")),
        Some(c) => net
            .seat_at(c)
            .map(|seat| format!("Corner {c} went to {}.", seat.name)),
        None => mine.is_none().then(|| "Corner released.".into()),
    }
}

/// Select the corner a wedge click or digit key names.
///
/// A system of its own rather than a branch of [`handle_buttons`] so a headless
/// test can drive corner selection without a socket. The star is one hit
/// area; `click_star` resolves each click to a wedge by angle.
pub fn select_corner(
    mut clicks: MessageReader<StarClick>,
    keys: Res<ButtonInput<KeyCode>>,
    mut socket: Option<ResMut<MatchboxSocket>>,
    mut net: ResMut<NetState>,
    mut selected: ResMut<SelectedCorner>,
    mut status: ResMut<LobbyStatus>,
    mut pending: ResMut<PendingClaim>,
) {
    for sector in clicks.read().filter_map(|click| click.0) {
        let me = net.my_seat().map(|s| s.peer.clone()).unwrap_or_default();
        match sector_click(&net, sector) {
            SectorClick::Select(i) => selected.0 = Some(i),
            SectorClick::Claim(c) => {
                if let Some(s) = socket.as_mut() {
                    status.0 = send_claim(s, &mut net, &mut pending, Some(c), &me);
                }
                // The claimed corner is the selected one: the very next
                // Computer / Cancel Seat press acts on it without a second
                // wedge click. Without this the first press after a claim was
                // silently swallowed — nothing was selected to act on.
                selected.0 = Some(c as usize);
            }
        }
    }
    if let Some(next) = corner_from_keys(&keys, selected.0) {
        selected.0 = Some(next);
    }
}

/// Track which wedge the cursor rests on, for the hover shading.
fn hover_corner(
    hit: Query<&RelativeCursorPosition, With<StarHit>>,
    mut hovered: ResMut<HoveredCorner>,
) {
    let over = hit
        .iter()
        .next()
        .and_then(|rel| rel.normalized.and_then(sector_at));
    hovered.set_if_neq(HoveredCorner(over));
}

/// The concrete effect a [`CornerCommand`] presses on the selected corner.
/// Deciding it is a pure function so the rules are testable without a socket;
/// [`handle_buttons`] performs the result against the network.
#[derive(Debug, PartialEq, Eq)]
pub enum CornerEffect {
    /// A solo setup: the corner's new local state.
    Local(CornerState),
    /// Claim or release a corner in a shared room (guest -> host, or host
    /// directly).
    Claim(Option<u32>),
    /// The host seats an engine at this corner.
    AddEngine(u32),
    /// The host removes the engine from this corner.
    RemoveEngine(u32),
    /// The host un-seats the player holding this corner.
    Unseat(u32),
}

pub fn corner_effect(
    net: &NetState,
    me: &str,
    corner: u32,
    cmd: CornerCommand,
) -> Result<CornerEffect, String> {
    if !net.peers.is_empty() {
        // Shared room: the roster owns the corners.
        if let Some(owner) = net.seat_at(corner) {
            if owner.engine {
                if net.sequences() && cmd == CornerCommand::Off {
                    return Ok(CornerEffect::RemoveEngine(corner));
                }
                return Err(format!("An engine plays corner {corner}."));
            }
            if owner.peer == me {
                return match cmd {
                    CornerCommand::Off => Ok(CornerEffect::Claim(None)),
                    CornerCommand::Human => Err("You already hold this corner.".into()),
                    CornerCommand::Cpu => {
                        Err("Seat an engine on a free corner: cancel this seat first.".into())
                    }
                };
            }
            // The host may un-seat anyone: a stray claim, a peer that wandered
            // off, a table being rearranged before the start.
            if net.sequences() && cmd == CornerCommand::Off {
                return Ok(CornerEffect::Unseat(corner));
            }
            return Err(format!(
                "Corner {corner} is held by {}. Only the host can un-seat a player.",
                owner.name
            ));
        }
        return match cmd {
            // The human claiming this corner is me — any peer may.
            CornerCommand::Human => Ok(CornerEffect::Claim(Some(corner))),
            CornerCommand::Cpu if net.sequences() => Ok(CornerEffect::AddEngine(corner)),
            CornerCommand::Cpu => Err("Only the host can seat an engine.".into()),
            CornerCommand::Off => Err(format!("Corner {corner} is already free.")),
        };
    }

    // Solo setup: rewrite the local table.
    let state = match cmd {
        CornerCommand::Human => CornerState::Human,
        CornerCommand::Cpu => CornerState::Cpu,
        CornerCommand::Off => CornerState::Empty,
    };
    Ok(CornerEffect::Local(state))
}

/// Fill the whole table with a symmetric preset, replacing whatever was there,
/// so the shortcut and the deal can never silently disagree.
pub fn apply_preset(table: &mut Table, seating: Seating) {
    let seated = seating.players();
    for (corner, player) in table.0.iter_mut().zip(Player::ALL) {
        *corner = if seated.contains(&player) {
            CornerState::Human
        } else {
            CornerState::Empty
        };
    }
}

/// One corner of the lobby reacts to what the player clicks or types: presets,
/// corner-by-corner seats, the foreign-camps toggle, and the `Start` that
/// packs the table. The rules-only half lives in [`corner_effect`]; a network
/// conversation lives in [`pump_socket`].
pub fn handle_buttons(
    mut pressed: MessageReader<LobbyButton>,
    keys: Res<ButtonInput<KeyCode>>,
    mut socket: Option<ResMut<MatchboxSocket>>,
    lobby: LobbyWorld,
) {
    let LobbyWorld {
        mut net,
        mut table,
        mut selected,
        mut variants,
        mut status,
        mut pending,
        mut next_state,
        ..
    } = lobby;
    let mut start = keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::NumpadEnter);
    let mut foreign = keys.just_pressed(KeyCode::KeyF);
    let deselect = keys.just_pressed(KeyCode::Escape);
    let mut preset: Option<Seating> = None;
    let mut command: Option<CornerCommand> = None;

    for button in pressed.read() {
        match button {
            LobbyButton::Start => start = true,
            LobbyButton::ForeignCamps => foreign = true,
            LobbyButton::Preset(s) => preset = Some(*s),
            LobbyButton::CornerAction(cmd) => command = Some(*cmd),
        }
    }

    if deselect {
        selected.0 = None;
    }

    let solo = net.peers.is_empty();
    let me = net.my_seat().map(|s| s.peer.clone()).unwrap_or_default();

    // Presets are solo-only furniture ([`SoloOnly`] hides the row once the
    // room is shared, and no key names them).
    if let Some(seating) = preset
        && solo
    {
        apply_preset(&mut table, seating);
        status.0 = format!("Table filled: {}.", seating.label());
    }

    if let (Some(corner), Some(cmd)) = (selected.0, command) {
        let corner = corner as u32;
        match corner_effect(&net, &me, corner, cmd) {
            Ok(CornerEffect::Local(state)) => {
                table.0[corner as usize] = state;
                status.0 = match cmd {
                    CornerCommand::Human => format!("Corner {corner}: human."),
                    CornerCommand::Cpu => format!("Corner {corner}: computer."),
                    CornerCommand::Off => format!("Corner {corner}: empty."),
                };
            }
            Ok(CornerEffect::Claim(claim)) => {
                if let Some(s) = socket.as_mut() {
                    status.0 = send_claim(s, &mut net, &mut pending, claim, &me);
                }
            }
            Ok(CornerEffect::AddEngine(c)) => {
                seat_engine_at(&mut net, c as usize);
                if let Some(s) = socket.as_mut() {
                    publish_roster(s, &net);
                }
                status.0 = format!("Engine seated at corner {c}.");
            }
            Ok(CornerEffect::RemoveEngine(c)) => {
                remove_engine_at(&mut net, c as usize);
                if let Some(s) = socket.as_mut() {
                    publish_roster(s, &net);
                }
                status.0 = format!("Corner {c}: engine removed.");
            }
            Ok(CornerEffect::Unseat(c)) => {
                if let Some(seat) = net
                    .seats
                    .iter_mut()
                    .find(|s| s.player == Some(c) && !s.engine)
                {
                    seat.player = None;
                    status.0 = format!("{} was un-seated from corner {c}.", seat.name);
                    if let Some(s) = socket.as_mut() {
                        publish_roster(s, &net);
                    }
                }
            }
            Err(why) => status.0 = why,
        }
    }

    if foreign {
        // Only the host decides the rules, for the same reason only the host
        // decides the table: the shared game must play under one rule set.
        if net.sequences() {
            variants.0.forbid_foreign_camps = !variants.0.forbid_foreign_camps;
            // Broadcast live, so every lobby shows one game before the Start
            // repeats it; with no peers this is a no-op.
            if let Some(s) = socket.as_mut() {
                broadcast(
                    s,
                    &net.peers,
                    &NetMsg::Variants {
                        forbid_foreign_camps: variants.0.forbid_foreign_camps,
                    },
                );
            }
            status.0 = if variants.0.forbid_foreign_camps {
                "Rule on: no piece rests in a foreign triangle.".into()
            } else {
                "Rule off: the specification's game.".into()
            };
        } else {
            status.0 = "Only the host chooses the rules.".into();
        }
    }

    if start {
        match start_decision(&net, &table) {
            StartDecision::Solo => {
                info!("starting a solo game");
                next_state.set(AppState::InGame);
            }
            StartDecision::Multiplayer => {
                info!(players = seated(&net), "starting a shared game");
                if let Some(s) = socket.as_mut() {
                    broadcast(s, &net.peers, &start_message(&net, variants.0));
                }
                next_state.set(AppState::InGame);
            }
            StartDecision::Refuse(why) => status.0 = why,
        }
    }
}

/// Focus a field with its key: `R` the room, `N` the name. A click focuses
/// one natively; this is the keyboard's way in. The field already holds the
/// current value, and the caret goes to its end, so a small change does not
/// mean retyping the whole thing.
fn focus_input_fields(
    keys: Res<ButtonInput<KeyCode>>,
    mut fields: Query<(Entity, &InputField, &mut EditableText)>,
    mut focus: ResMut<InputFocus>,
) {
    let kind = if keys.just_pressed(KeyCode::KeyR) {
        FieldKind::Room
    } else if keys.just_pressed(KeyCode::KeyN) {
        FieldKind::Name
    } else {
        return;
    };
    let Some((field, _, mut text)) = fields.iter_mut().find(|(_, f, _)| f.0 == kind) else {
        return;
    };
    focus.set(field, FocusCause::Navigated);
    text.queue_edit(TextEdit::TextEnd(false));
}

/// Say how editing ends, however the field came to hold the keyboard.
fn announce_editing(_gained: On<FocusGained>, mut status: ResMut<LobbyStatus>) {
    status.0 = "Editing. Enter accepts, Esc cancels.".into();
}

/// While a field holds the keyboard, its keys are its own.
///
/// Runs between Bevy turning the window's key messages into [`ButtonInput`]
/// and handing the same keys to the focused field, and empties the former:
/// no system that reads the keyboard sees a key typed into the field — not
/// the lobby's Enter and digits, and not the app-wide sound toggle. That
/// includes the key that ends the edit: the field still held the keyboard
/// when it was pressed, so the Enter that commits a room does not also start
/// the game, and the Esc that leaves does not also clear the selected corner.
pub fn hold_keys_for_field(
    focus: Res<InputFocus>,
    fields: Query<(), With<InputField>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
) {
    if focus.get().is_some_and(|focused| fields.contains(focused)) {
        keys.clear();
    }
}

/// What committing a field writes: the room (and the socket bound to it), the
/// player's name, the status line, the state that re-enters the lobby, and
/// the field's own focus and refusal.
#[derive(SystemParam)]
pub struct FieldTargets<'w, 's> {
    commands: Commands<'w, 's>,
    room: ResMut<'w, RoomId>,
    net: ResMut<'w, NetState>,
    status: ResMut<'w, LobbyStatus>,
    next_state: ResMut<'w, NextState<AppState>>,
    focus: ResMut<'w, InputFocus>,
    refusal: ResMut<'w, FieldRefusal>,
}

/// Enter commits the field it was typed into.
fn commit_on_enter(
    input: On<FocusedInput<KeyboardInput>>,
    fields: Query<(&InputField, &EditableText)>,
    mut targets: FieldTargets,
) {
    let key = &input.input;
    let enter = matches!(key.key_code, KeyCode::Enter | KeyCode::NumpadEnter);
    if !enter || !key.state.is_pressed() || key.repeat {
        return;
    }
    // A frame's keys all go to the field that held the keyboard when the
    // frame began. Once this frame's Enter or Esc has let go of it, a later
    // Enter in the same frame is no longer the field's to commit.
    let field = input.focused_entity;
    if targets.focus.get() != Some(field) {
        return;
    }
    let Ok((InputField(kind), text)) = fields.get(field) else {
        return;
    };
    // An Enter that confirms an IME composition belongs to the IME.
    if text.is_composing() {
        return;
    }
    commit(*kind, &text.value().to_string(), &mut targets);
}

/// Commit a field's text. On success the field lets go of the keyboard; on a
/// refusal it keeps it, with the reason beneath it and what was typed intact,
/// to be corrected.
fn commit(kind: FieldKind, text: &str, targets: &mut FieldTargets) {
    let result = match kind {
        FieldKind::Room => commit_room(text, targets),
        FieldKind::Name => commit_name(text, targets),
    };
    match result {
        Ok(()) => {
            targets.focus.clear();
            targets.refusal.0 = None;
        }
        Err(why) => targets.refusal.0 = Some((kind, why)),
    }
}

/// Change the room, and rejoin.
///
/// Changing the room means **reopening the socket**: the room is baked into the
/// signaling URL, so editing [`RoomId`] alone would change the label and
/// nothing else. [`checkers_net::close_socket`] drops the socket together with
/// everything it told us, and re-entering the lobby opens a fresh one against
/// the new room. On the web the new room is written into the page URL, so the
/// address bar always points where this peer actually is.
fn commit_room(buffer: &str, targets: &mut FieldTargets) -> Result<(), String> {
    let parsed = RoomId::parse(buffer).map_err(|why| why.to_string())?;
    if parsed == *targets.room {
        // Same room: rejoining would drop the peers already here for no
        // reason.
        targets.status.0 = format!("Already in room \"{}\".", targets.room.0);
        return Ok(());
    }
    info!(from = %targets.room.0, to = %parsed.0, "changing room");
    *targets.room = parsed;
    crate::web::share_room(&targets.room);
    targets.status.0 = format!("Joining room \"{}\"...", targets.room.0);
    checkers_net::close_socket(&mut targets.commands, &mut targets.net);
    targets.next_state.set(AppState::Lobby);
    Ok(())
}

/// Change the display name: trimmed, and refused when empty like
/// [`RoomId::parse`] refuses an empty room. Everyone is re-greeted, because the
/// roster is only ever exchanged on Hello — a rename without a re-greet would
/// leave every peer looking at the old name.
fn commit_name(buffer: &str, targets: &mut FieldTargets) -> Result<(), String> {
    let trimmed = buffer.trim();
    if trimmed.is_empty() {
        return Err("A name cannot be empty.".into());
    }
    if name_taken(&targets.net, trimmed) {
        return Err(format!("Someone here is already called \"{trimmed}\"."));
    }
    if trimmed != targets.net.name {
        targets.net.name = trimmed.to_string();
        targets.net.greeted.clear();
        targets.status.0 = "Name updated.".into();
    }
    Ok(())
}

/// The name row's Apply button.
///
/// This is what the button is *for*: a click commits the name being typed.
/// Pressing it leaves the keyboard with the field (see `keep_focus`), so a
/// click on Apply while the name is focused commits it; with the name field
/// not focused, the click focuses it — the next Apply, or Enter, applies. One
/// modal at a time: while the room field holds the keyboard, Apply stands
/// down.
pub fn apply_name(
    mut pressed: MessageReader<ApplyName>,
    fields: Query<(Entity, &InputField, &EditableText)>,
    mut targets: FieldTargets,
) {
    if pressed.is_empty() {
        return;
    }
    pressed.clear();
    let Some((name, _, text)) = fields.iter().find(|(_, f, _)| f.0 == FieldKind::Name) else {
        return;
    };
    match targets.focus.get() {
        Some(focused) if focused == name => {
            commit(FieldKind::Name, &text.value().to_string(), &mut targets);
        }
        Some(focused) if fields.contains(focused) => {}
        _ => targets.focus.set(name, FocusCause::Navigated),
    }
}

/// Let go of the keyboard on leaving the lobby: the field goes with the
/// lobby, and a field left focused when the game starts must not come back
/// holding the keys of the lobby the player returns to.
pub fn release_field(
    mut focus: ResMut<InputFocus>,
    fields: Query<(), With<InputField>>,
    mut refusal: ResMut<FieldRefusal>,
) {
    if focus.get().is_some_and(|focused| fields.contains(focused)) {
        focus.clear();
    }
    refusal.0 = None;
}

/// Every field that does not hold the keyboard shows the committed value: an
/// edit left with Esc or a click elsewhere is dropped, and a value changed
/// from outside — a rename after a name clash — shows at once. A refusal shows
/// beneath its field while that field still holds the keyboard.
///
/// Compares before it writes instead of asking whether anything changed, so
/// the fields are right on the first frame of a rebuilt lobby too.
fn draw_fields(
    room: Res<RoomId>,
    net: Res<NetState>,
    focus: Res<InputFocus>,
    mut refusal: ResMut<FieldRefusal>,
    mut fields: Query<(Entity, &InputField, &mut EditableText)>,
    mut errors: Query<(&mut Text, &InputError)>,
) {
    for (field, InputField(kind), mut text) in &mut fields {
        if focus.get() == Some(field) {
            continue;
        }
        if refusal
            .0
            .as_ref()
            .is_some_and(|(refused, _)| refused == kind)
        {
            refusal.0 = None;
        }
        let committed = match kind {
            FieldKind::Room => room.0.as_str(),
            FieldKind::Name => net.name.as_str(),
        };
        if text.value() != committed {
            text.editor_mut().set_text(committed);
            text.queue_edit(TextEdit::TextEnd(false));
        }
    }
    for (mut line, InputError(kind)) in &mut errors {
        let wanted = match &refusal.0 {
            Some((refused, why)) if refused == kind => why.as_str(),
            _ => "",
        };
        if line.as_str() != wanted {
            **line = wanted.to_string();
        }
    }
}

/// Draw the roster and the running status line.
fn draw_roster(
    net: Res<NetState>,
    table: Res<Table>,
    status: Res<LobbyStatus>,
    mut text: Query<&mut Text, With<RosterText>>,
    fresh: Query<(), Added<RosterText>>,
) {
    // A rebuilt lobby is drawn whether or not anything changed meanwhile.
    if !net.is_changed() && !table.is_changed() && !status.is_changed() && fresh.is_empty() {
        return;
    }
    let Ok(mut text) = text.single_mut() else {
        return;
    };

    let me = net.my_seat().map(|s| s.peer.clone()).unwrap_or_default();
    let mut out;
    if net.peers.is_empty() {
        let filled = table.0.iter().filter(|c| **c != CornerState::Empty).count();
        let cpu = table.0.iter().filter(|c| **c == CornerState::Cpu).count();
        out = format!("local setup  |  {filled} corner(s) filled, {cpu} by computer\n\n");
        if filled == 0 {
            out.push_str(
                "The star is empty. Click a petal (or 1-6), then choose Human / Computer.\n",
            );
        } else if cpu == filled {
            out.push_str("Every corner is an engine - Enter starts as a spectator.\n");
        }
    } else {
        let role = if net.sequences() { "host" } else { "guest" };
        out = format!("{role}  |  {} peer(s) here\n\n", net.peers.len());
        if net.seats.is_empty() {
            out.push_str("No one here yet - share the room name.\n");
        }
        for seat in &net.seats {
            let corner = seat
                .player
                .map_or_else(|| "no corner".into(), |p| format!("corner {p}"));
            let engine = if seat.engine { " (computer)" } else { "" };
            let pointer = if seat.peer == me { ">" } else { " " };
            out.push_str(&format!("  {pointer} {}  {corner}{engine}\n", seat.name));
        }
    }

    out.push_str(if net.peers.is_empty() {
        "\nEnter starts this table locally.\nDigits 1-6 select a corner."
    } else {
        "\nClaim a corner on the star; the host starts the game with Enter."
    });
    if !status.0.is_empty() {
        out.push_str(&format!("\n\n{}", status.0));
    }
    **text = out;
}

/// Keep the star's petals in step with the table/roster: the wedge's colour,
/// lifted under the cursor, and the outline around the selected one.
///
/// Selection is an outline rather than a brighter fill. Brightening scaled
/// each channel and clipped it, which shifted the hue — the yellow corner
/// turned pure yellow, another corner cyan — and left no headroom on the
/// white corner, where selected and hovered looked the same.
fn sync_corner_styles(
    net: Res<NetState>,
    table: Res<Table>,
    selected: Res<SelectedCorner>,
    hovered: Res<HoveredCorner>,
    mut petals: Query<(&CornerPetal, &mut ImageNode)>,
    mut outlines: Query<(&CornerOutline, &mut Visibility)>,
) {
    for (petal, mut img) in petals.iter_mut() {
        let p = Player::new(petal.0 as u8).expect("corner indices are below six");
        let base = if corner_is_filled(&net, &table, petal.0) {
            player_colour(p)
        } else {
            IDLE
        };
        let colour = if hovered.0 == Some(petal.0) {
            lift(base)
        } else {
            base
        };
        if img.color != colour {
            img.color = colour;
        }
    }
    for (outline, mut visibility) in &mut outlines {
        visibility.set_if_neq(visible_if(selected.0 == Some(outline.0)));
    }
}

/// Draw the two lines of every star petal.
///
/// Each line is a stack of copies (white fill over black border copies, see
/// [`outlined_line`]); the marker sits on the stack and the string goes into
/// every copy, so fill and border can never disagree.
fn draw_corner_labels(
    net: Res<NetState>,
    table: Res<Table>,
    lines: Query<(&CornerText, &Children)>,
    boxes: Query<&Children>,
    mut texts: Query<&mut Text>,
    fresh: Query<(), Added<CornerText>>,
) {
    // A rebuilt lobby is drawn whether or not anything changed meanwhile.
    if !net.is_changed() && !table.is_changed() && fresh.is_empty() {
        return;
    }
    let solo = net.peers.is_empty();
    for (label, copies) in &lines {
        let i = label.0 / 2;
        let line = label.0 % 2;
        let (title, sub) = if solo {
            match &table.0[i] {
                CornerState::Empty => ("Empty".into(), "click to select".into()),
                CornerState::Human => (format!("P{i}"), "human".into()),
                CornerState::Cpu => ("Computer".into(), "CPU".into()),
            }
        } else if let Some(seat) = net.seat_at(i as u32) {
            if seat.engine {
                ("Engine".into(), "CPU".into())
            } else {
                (seat.name.clone(), "human".into())
            }
        } else {
            ("Empty".into(), "click to claim".into())
        };
        let wanted = if line == 0 { title } else { sub };
        for copy in copies.iter() {
            // Each copy is a flex box; its one child is the text leaf.
            if let Ok(leaf) = boxes.get(copy)
                && let Some(text_entity) = leaf.iter().next()
                && let Ok(mut text) = texts.get_mut(text_entity)
                && **text != wanted
            {
                **text = wanted.clone();
            }
        }
    }
}

/// Lift a wedge's colour under the cursor: a step toward white on the sRGB
/// scale, which keeps the colour itself rather than shifting its hue.
fn lift(c: Color) -> Color {
    Color::Srgba(c.to_srgba().mix(&Srgba::WHITE, 0.15))
}

/// The camps that sit down in the current room: which corners are filled,
/// which of them this device's engine drives, which corner this peer
/// commands, and whether this peer only watches. Pure, so the deal is
/// testable without a world.
///
/// Solo setups read the local [`Table`] — every configured corner, whether
/// played by hand (`CornerState::Human`) or by the engine. Shared rooms read
/// the roster's claims instead, so every peer deals the same board no matter
/// what corners each device configured.
pub fn deal_for(net: &NetState, table: &Table) -> (Vec<Player>, Vec<Player>, Option<Player>, bool) {
    if net.peers.is_empty() {
        let players: Vec<Player> = (0..6)
            .filter(|&i| table.0[i] != CornerState::Empty)
            .filter_map(|i| Player::new(i as u8))
            .collect();
        let ai: Vec<Player> = (0..6)
            .filter(|&i| table.0[i] == CornerState::Cpu)
            .filter_map(|i| Player::new(i as u8))
            .collect();
        let spectating = !ai.is_empty() && ai.len() >= players.len();
        (players, ai, None, spectating)
    } else {
        let players: Vec<Player> = net
            .seats
            .iter()
            .filter_map(|s| s.player)
            .filter_map(|i| Player::new(i as u8))
            .collect();
        let ai = engine_camps(net);
        let local = net.my_player();
        let spectating = net.my_seat().is_none_or(|s| s.player.is_none());
        (players, ai, local, spectating)
    }
}

/// Build the round's session for the configured table and variants, seating
/// the local player. Pure, so the entry system and the "deal a new game" key
/// share exactly one deal path.
pub fn deal_session(net: &NetState, table: &Table, variants: Variants) -> Session {
    let (players, ai, local_player, spectating) = deal_for(net, table);
    let wording = if spectating {
        if net.peers.is_empty() {
            "Spectating - the engines play each other.".into()
        } else {
            "Spectating - watch, but do not touch.".into()
        }
    } else if let Some(p) = local_player {
        format!("You are player {} of {}", p.index(), players.len())
    } else {
        format!("Playing all {} corners on this device", players.len())
    };

    let mut session = Session::for_players(&players, variants);
    session.ai_players = ai;
    session.local_player = local_player;
    session.spectating = spectating;
    // The same test `deal_for` used to pick the roster over the table.
    session.shared = !net.peers.is_empty();
    session.message = wording;
    session
}

/// Build the game for the configured table and seat the local player.
///
/// The session is *rebuilt* here rather than mutated, because the players
/// determine the starting position and there is no meaningful way to reseat a
/// board that has already been dealt. Runs on entering the game, before the
/// board is spawned.
pub fn apply_seats(
    net: Res<NetState>,
    table: Res<Table>,
    variants: Res<ChosenVariants>,
    mut session: ResMut<Session>,
) {
    *session = deal_session(&net, &table, variants.0);
    info!(
        players = session.players.len(),
        engines = session.ai_players.len(),
        "round dealt",
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seat(name: &str, player: Option<u32>) -> Seat {
        Seat::human(name, name, player)
    }

    fn engine_seat(name: &str, player: Option<u32>) -> Seat {
        Seat {
            engine: true,
            ..seat(name, player)
        }
    }

    /// A room this peer shares with one other, holding `seats`.
    fn shared(is_host: bool, seats: Vec<Seat>) -> NetState {
        NetState {
            peers: vec![fake_peer()],
            is_host,
            seats,
            ..NetState::default()
        }
    }

    /// A container-local point as [`RelativeCursorPosition`] reports it: the
    /// inverse of the mapping in `sector_at`.
    fn normalized(p: Vec2) -> Vec2 {
        (p - STAR_CENTRE) / vec2(STAR_W, STAR_H)
    }

    /// Reads a star texture's alpha at the pixel under a point.
    fn alpha_of(image: &Image) -> impl Fn(Vec2) -> u8 + '_ {
        let Some(data) = &image.data else {
            panic!("star textures keep their pixel data");
        };
        move |p| {
            let (x, y) = (p.x as u32, p.y as u32);
            data[((y * STAR_W as u32 + x) * 4 + 3) as usize]
        }
    }

    /// The host's rename must reach its own seat and republish the roster:
    /// guests ignore a Hello, so no other path carries a renamed host.
    #[test]
    fn a_host_rename_updates_its_own_seat() {
        let mut net = NetState {
            name: "new-host".into(),
            seats: vec![seat("host", Some(0))],
            ..NetState::default()
        };
        assert!(sync_host_seat(&mut net, "host"), "a rename republishes");
        assert_eq!(net.seats[0].peer, "host");
        assert_eq!(net.seats[0].name, "new-host");
    }

    /// On the host's first greet its seat does not exist yet (the host never
    /// receives its own Hello), so it must be added — without a republish,
    /// which only a rename deserves.
    #[test]
    fn a_hosts_first_greet_creates_its_own_seat() {
        let mut net = NetState {
            name: "ada".into(),
            ..NetState::default()
        };
        assert!(!sync_host_seat(&mut net, "host"));
        assert_eq!(net.seats.len(), 1);
        assert_eq!(net.seats[0].peer, "host");
        assert_eq!(net.seats[0].name, "ada");
    }

    /// An idle room re-greets nobody and nothing changes: no republish.
    #[test]
    fn an_unchanged_name_reports_no_change() {
        let mut net = NetState {
            name: "host".into(),
            seats: vec![seat("host", Some(0))],
            ..NetState::default()
        };
        assert!(!sync_host_seat(&mut net, "host"));
        assert_eq!(net.seats[0].name, "host");
    }

    /// A lone device with no peers deals whatever corners it configured.
    #[test]
    fn a_solo_table_starts_with_the_configured_corners() {
        let net = NetState::default();
        let mut table = Table::default();
        table.0[0] = CornerState::Human;
        table.0[3] = CornerState::Cpu;
        assert_eq!(start_decision(&net, &table), StartDecision::Solo);
    }

    /// One corner cannot be a game: the turn would visit a player's camp with
    /// nobody else to play.
    #[test]
    fn one_corner_cannot_start() {
        let net = NetState::default();
        let mut table = Table::default();
        table.0[0] = CornerState::Human;
        match start_decision(&net, &table) {
            StartDecision::Refuse(why) => assert!(why.contains("two corners"), "{why}"),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    /// A guest cannot start a shared game, and must be told so rather than
    /// having Enter do nothing.
    #[test]
    fn a_guest_is_told_only_the_host_can_start() {
        let net = shared(false, vec![]);
        match start_decision(&net, &Table::default()) {
            StartDecision::Refuse(why) => assert!(why.contains("host"), "{why}"),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    /// Fewer than two claimed corners cannot start.
    #[test]
    fn a_shared_start_needs_two_claimed_corners() {
        let net = shared(true, vec![seat("ada", Some(0))]);
        assert!(
            matches!(
                start_decision(&net, &Table::default()),
                StartDecision::Refuse(_)
            ),
            "one claim cannot be a game"
        );
    }

    #[test]
    fn the_host_starts_two_claimed_corners() {
        let net = shared(true, vec![seat("ada", Some(0)), seat("grace", Some(3))]);
        assert_eq!(
            start_decision(&net, &Table::default()),
            StartDecision::Multiplayer
        );
    }

    /// A claimed corner plus an engine seat reaches the two-corner minimum.
    #[test]
    fn engines_count_toward_the_two_corners() {
        let seats = vec![seat("ada", Some(0)), engine_seat("engine-0", Some(3))];
        let net = shared(true, seats);
        assert_eq!(
            start_decision(&net, &Table::default()),
            StartDecision::Multiplayer
        );
    }

    /// The `Start` broadcast carries the roster as it stands, so every peer
    /// deals from the same seats.
    #[test]
    fn the_start_message_carries_the_roster() {
        let net = NetState {
            seats: vec![seat("ada", Some(3)), seat("grace", Some(0))],
            ..Default::default()
        };
        let NetMsg::Start { seats, .. } = start_message(&net, Variants::default()) else {
            panic!("start_message must build a Start");
        };
        assert_eq!(seats, net.seats);
    }

    /// The host seats an engine on a free corner; it claims that corner and
    /// reads as ready so it never blocks a start.
    #[test]
    fn an_engine_seat_claims_its_corner() {
        // A friend is here: engine seating is a shared-room action, never
        // a solo rewrite of the local table.
        let mut net = shared(true, vec![]);
        let effect = corner_effect(&net, "", 2, CornerCommand::Cpu);
        assert_eq!(effect, Ok(CornerEffect::AddEngine(2)));
        seat_engine_at(&mut net, 2);
        assert!(net.seats[0].engine);
        assert_eq!(net.seats[0].player, Some(2));

        // The host removes it again.
        assert_eq!(
            corner_effect(&net, "", 2, CornerCommand::Off),
            Ok(CornerEffect::RemoveEngine(2))
        );
        remove_engine_at(&mut net, 2);
        assert!(net.seats.is_empty());
    }

    /// Only the sequencing authority runs engines.
    #[test]
    fn engines_drive_on_the_host_alone() {
        let mut host = NetState {
            is_host: true,
            ..Default::default()
        };
        seat_engine_at(&mut host, 0);
        let guest = NetState {
            peers: vec![fake_peer()],
            seats: host.seats.clone(),
            ..Default::default()
        };
        assert_eq!(engine_camps(&host), vec![Player::ALL[0]]);
        assert!(engine_camps(&guest).is_empty());
    }

    /// A guest claims a free corner; the host applies it. The pure decision is
    /// a claim message, and the host's own application moves the seat.
    #[test]
    fn a_claim_moves_the_seat() {
        let mut net = shared(false, vec![]);
        seat_for(&mut net, "host", "ada");
        assert_eq!(net.seats[0].player, None, "no corner until claimed");

        let effect = corner_effect(&net, "host", 1, CornerCommand::Human)
            .expect("a free corner is claimable");
        assert_eq!(effect, CornerEffect::Claim(Some(1)));
        net.seats[0].player = Some(1);
        assert_eq!(net.seats[0].player, Some(1));
    }

    /// A corner already held by someone else is not claimable, and the reason
    /// names the holder.
    #[test]
    fn an_occupied_corner_is_not_claimable() {
        let net = shared(false, vec![seat("ada", Some(2))]);
        let err = corner_effect(&net, "grace", 2, CornerCommand::Human).expect_err("taken");
        assert!(err.contains("ada"), "must name the holder: {err}");
    }

    /// Pressing "Off" on my own corner releases it; anything else is refused
    /// with the reason attached.
    #[test]
    fn releasing_my_own_corner() {
        let net = shared(false, vec![seat("ada", Some(2))]);
        assert_eq!(
            corner_effect(&net, "ada", 2, CornerCommand::Off),
            Ok(CornerEffect::Claim(None))
        );
        assert!(corner_effect(&net, "ada", 2, CornerCommand::Human).is_err());
    }

    /// A guest cannot place an engine; that is the host's call.
    #[test]
    fn only_the_host_seats_engines() {
        let net = shared(false, vec![]);
        assert!(corner_effect(&net, "grace", 1, CornerCommand::Cpu).is_err());
        let host = shared(true, vec![]);
        assert_eq!(
            corner_effect(&host, "host", 1, CornerCommand::Cpu),
            Ok(CornerEffect::AddEngine(1))
        );
    }

    /// Solo commands map straight onto corner states.
    #[test]
    fn solo_commands_rewrite_the_table() {
        let net = NetState::default();
        for (cmd, state) in [
            (CornerCommand::Cpu, CornerState::Cpu),
            (CornerCommand::Human, CornerState::Human),
            (CornerCommand::Off, CornerState::Empty),
        ] {
            assert_eq!(
                corner_effect(&net, "", 0, cmd),
                Ok(CornerEffect::Local(state))
            );
        }
    }

    /// The preset fills exactly the seating's corners, presenting the
    /// symmetric game the shortcut claims.
    #[test]
    fn presets_fill_the_seatings_corners() {
        let mut table = Table::default();
        apply_preset(&mut table, Seating::Two);
        assert_eq!(table.0[0], CornerState::Human);
        assert_eq!(table.0[3], CornerState::Human);
        assert_eq!(table.0[1], CornerState::Empty);
        assert_eq!(table.0[2], CornerState::Empty);
    }

    /// Digit keys select their corner; unrelated keys change nothing.
    #[test]
    fn the_digits_select_their_corner() {
        for (corner, key) in CORNER_KEYS.into_iter().enumerate() {
            let mut keys = ButtonInput::default();
            keys.press(key);
            assert_eq!(corner_from_keys(&keys, None), Some(corner), "{key:?}");
        }
        let mut keys = ButtonInput::default();
        keys.press(KeyCode::KeyX);
        assert_eq!(corner_from_keys(&keys, None), None);
    }

    /// The deal must run on the configured corners: `apply_seats` rebuilds the
    /// session from them, and a session built for two corners must not be
    /// holding a board nobody set up. Solo setups read the local table; the
    /// engine camps match; all-engine tables are spectated.
    #[test]
    fn the_configured_corners_build_the_session() {
        let net = NetState::default();
        let mut table = Table::default();
        table.0[0] = CornerState::Human;
        table.0[2] = CornerState::Cpu;
        table.0[4] = CornerState::Human;
        let (players, ai, local, spectating) = deal_for(&net, &table);
        assert_eq!(
            players,
            vec![Player::ALL[0], Player::ALL[2], Player::ALL[4]]
        );
        assert_eq!(ai, vec![Player::ALL[2]]);
        assert_eq!(local, None);
        assert!(!spectating);

        // All engines: the table starts as a watched race.
        let mut table = Table::default();
        table.0[0] = CornerState::Cpu;
        table.0[3] = CornerState::Cpu;
        let (players, ai, _local, spectating) = deal_for(&net, &table);
        assert_eq!(players, vec![Player::ALL[0], Player::ALL[3]]);
        assert_eq!(ai, vec![Player::ALL[0], Player::ALL[3]]);
        assert!(spectating, "all-engine tables are spectated");
    }

    /// A shared start deals the roster's claims, not the local table.
    #[test]
    fn a_shared_start_deals_the_claims() {
        let mut net = NetState::default();
        let me = PeerId(uuid::Uuid::from_u128(1));
        net.my_id = Some(me);
        net.peers = vec![me, fake_peer()];
        net.is_host = true;
        let host = me.to_string();
        net.seats = vec![seat(&host, Some(0)), seat("grace", Some(3))];
        let mut table = Table::default();
        table.0[5] = CornerState::Cpu;

        let (players, ai, local, spectating) = deal_for(&net, &table);
        assert_eq!(players, vec![Player::ALL[0], Player::ALL[3]]);
        assert!(ai.is_empty(), "the local table is ignored in a shared room");
        assert_eq!(local, Some(Player::ALL[0]));
        assert!(!spectating);
    }

    /// A round remembers whether it was dealt from a shared room, and only a
    /// shared round names its players after the roster. After the guests of a
    /// shared round leave, the host's own seat and its engines linger in the
    /// roster — a solo round dealt then must not borrow their names, nor hide
    /// the solo controls.
    #[test]
    fn only_a_shared_round_reads_the_roster() {
        let mut net = NetState::default();
        let me = PeerId(uuid::Uuid::from_u128(1));
        net.my_id = Some(me);
        net.peers = vec![me, fake_peer()];
        net.is_host = true;
        net.seats = vec![seat(&me.to_string(), Some(0)), seat("grace", Some(3))];
        let mut table = Table::default();
        table.0[0] = CornerState::Human;
        table.0[3] = CornerState::Human;

        let shared = deal_session(&net, &table, Variants::default());
        assert!(shared.shared);
        assert_eq!(shared.roster_name(&net, Player::ALL[3]), Some("grace"));

        // The guests leave; their seats may linger until the next prune.
        net.peers.clear();
        let solo = deal_session(&net, &table, Variants::default());
        assert!(!solo.shared);
        assert_eq!(
            solo.roster_name(&net, Player::ALL[3]),
            None,
            "a solo corner is not named after a leftover seat"
        );
    }

    fn fake_peer() -> PeerId {
        PeerId(uuid::Uuid::from_u128(2))
    }

    /// Claiming a corner on the star must also select it: otherwise the first
    /// sidebar press after a claim was silently swallowed, acting on nothing.
    #[test]
    fn claiming_a_corner_selects_it() {
        use bevy::ecs::system::RunSystemOnce;

        let mut world = World::new();
        let me = PeerId(uuid::Uuid::from_u128(1));
        let mut net = shared(true, vec![seat(&me.to_string(), None)]);
        net.my_id = Some(me);
        world.insert_resource(net);
        world.insert_resource(SelectedCorner(None));
        world.init_resource::<LobbyStatus>();
        world.init_resource::<PendingClaim>();
        world.init_resource::<ButtonInput<KeyCode>>();
        world.init_resource::<Messages<StarClick>>();

        // A click on corner 1's centroid — free, so a claim.
        let wedge = sector_at(normalized(wedge_centroid(1)));
        assert_eq!(wedge, Some(1));
        world.write_message(StarClick(wedge));

        world.run_system_once(select_corner).unwrap();

        assert_eq!(
            world.resource::<SelectedCorner>().0,
            Some(1),
            "the claimed corner must be the selected one"
        );
    }

    /// A click names the wedge it landed on, read from the click itself: a
    /// tap completes once the finger has lifted, when there is no cursor left
    /// to ask where it was.
    #[test]
    fn a_click_on_the_star_names_the_wedge_under_it() {
        use bevy::camera::NormalizedRenderTarget;
        use bevy::picking::backend::HitData;
        use bevy::picking::events::Pointer;
        use bevy::picking::pointer::{Location, PointerId};
        use std::time::Duration;

        let mut world = World::new();
        world.init_resource::<UiScale>();
        world.init_resource::<Messages<StarClick>>();
        // The star laid out with its top-left corner at `origin`. A UI node's
        // transform sits at its centre.
        let origin = vec2(100.0, 50.0);
        let size = vec2(STAR_W, STAR_H);
        let star = world
            .spawn((
                StarHit,
                ComputedNode { size, ..default() },
                UiGlobalTransform::from_translation(origin + size / 2.0),
                ComputedUiRenderTargetInfo::default(),
            ))
            .observe(click_star)
            .id();

        for at in [wedge_centroid(4), STAR_CENTRE] {
            let location = Location {
                target: NormalizedRenderTarget::None {
                    width: 800,
                    height: 600,
                },
                position: origin + at,
            };
            world.trigger(PointerClick {
                entity: star,
                pointer: Pointer::new(PointerId::Touch(1), location),
                button: PointerButton::Primary,
                hit: HitData::new(star, 0.0, None, None),
                duration: Duration::from_millis(80),
                count: 1,
            });
        }

        let clicks: Vec<_> = world
            .resource_mut::<Messages<StarClick>>()
            .drain()
            .collect();
        assert_eq!(
            clicks,
            [StarClick(Some(4)), StarClick(None)],
            "a wedge click names its wedge; the middle names none"
        );
    }

    /// The wedges leave the middle of the star clear — that is the point of
    /// them — and each points at its own camp's direction.
    #[test]
    fn the_centre_is_not_a_corner_and_each_wedge_is_its_own() {
        assert_eq!(
            sector_at(Vec2::ZERO),
            None,
            "the middle of the star selects nothing"
        );
        for i in 0..6 {
            // The centroid, as a normalized container position, hits corner i.
            assert_eq!(
                sector_at(normalized(wedge_centroid(i))),
                Some(i),
                "wedge {i}'s own centroid must resolve to corner {i}"
            );
        }
        // A point between two wedges — straight out from the centre between
        // corners 0 and 1 — belongs to neither.
        let angle = (-60.0f32).to_radians();
        let between = STAR_CENTRE + 120.0 * Vec2::new(angle.cos(), angle.sin());
        assert_eq!(sector_at(normalized(between)), None);
    }

    /// The wedges are equilateral, like the board's camp triangles: all three
    /// sides equal, and the tip the outermost point of its wedge.
    #[test]
    fn the_wedges_are_equilateral() {
        for i in 0..6 {
            let [apex, left, right] = wedge_vertices(i);
            let side = left.distance(right);
            assert!(
                (apex.distance(left) - side).abs() < 1e-3
                    && (apex.distance(right) - side).abs() < 1e-3,
                "wedge {i} is not equilateral"
            );
            assert!(
                apex.distance(STAR_CENTRE) > left.distance(STAR_CENTRE),
                "wedge {i}'s tip must face outward"
            );
        }
    }

    /// The rasterized wedge is opaque inside the triangle and transparent at
    /// the centre, so the tint paints a sector and the star stays visible.
    #[test]
    fn the_wedge_texture_paints_the_triangle_only() {
        let image = sector_image(0);
        let alpha_at = alpha_of(&image);
        assert_eq!(alpha_at(wedge_centroid(0)), 255);
        assert_eq!(alpha_at(STAR_CENTRE), 0);
    }

    /// The selection outline is a band just inside the wedge's edges: there
    /// right inside each edge, clear in the middle and outside the wedge.
    #[test]
    fn the_outline_rings_the_inside_of_the_wedge() {
        let image = outline_image(0);
        let alpha_at = alpha_of(&image);
        let v = wedge_vertices(0);
        let mid = wedge_centroid(0);
        assert_eq!(alpha_at(mid), 0, "the middle of the wedge stays clear");
        for k in 0..3 {
            let edge_mid = (v[k] + v[(k + 1) % 3]) / 2.0;
            let inward = (mid - edge_mid).normalize();
            assert_eq!(alpha_at(edge_mid + inward), 255, "edge {k} is outlined");
            assert_eq!(
                alpha_at(edge_mid - inward * 3.0),
                0,
                "the outline stays inside edge {k}"
            );
        }
    }

    /// Hovering keeps a wedge's colour: every channel moves toward white by
    /// the same share, so the channels keep their order and nothing clips.
    #[test]
    fn hover_lifts_without_shifting_the_hue() {
        let base = Color::srgb(0.95, 0.72, 0.20);
        let lifted = lift(base).to_srgba();
        assert!(lifted.red > 0.95 && lifted.green > 0.72 && lifted.blue > 0.20);
        assert!(lifted.red > lifted.green && lifted.green > lifted.blue);
        assert!(lifted.red < 1.0, "nothing clips to full");
    }

    /// Solo: a wedge click only selects; the sidebar buttons configure.
    #[test]
    fn a_solo_wedge_click_selects() {
        let net = NetState::default();
        assert_eq!(sector_click(&net, 3), SectorClick::Select(3));
    }

    /// Shared: an empty wedge is claimed on the spot — the click is the claim.
    #[test]
    fn a_shared_click_on_an_empty_corner_claims_it() {
        let mut net = NetState::default();
        let me = PeerId(uuid::Uuid::from_u128(1));
        net.my_id = Some(me);
        net.peers = vec![me, fake_peer()];
        net.seats = vec![seat(&me.to_string(), None)];
        assert_eq!(sector_click(&net, 5), SectorClick::Claim(5));
    }

    /// A peer that leaves takes its seat with it: a refresh mints a new peer
    /// id, and without pruning the roster only ever grew. Note `net.peers`
    /// never names the host itself — matchbox lists only *other* peers — so
    /// the host's own seat is exempted explicitly.
    #[test]
    fn departed_peers_lose_their_seats() {
        let mut net = NetState::default();
        let me = PeerId(uuid::Uuid::from_u128(1));
        let gone = PeerId(uuid::Uuid::from_u128(2));
        let here = PeerId(uuid::Uuid::from_u128(3));
        net.my_id = Some(me);
        net.is_host = true;
        // Others only: the mesh never reports the host's own id as a peer.
        net.peers = vec![here];
        net.greeted = vec![gone, here];
        net.seats = vec![
            seat(&me.to_string(), Some(0)),
            seat(&gone.to_string(), Some(3)),
            engine_seat("engine-0", Some(4)),
            seat(&here.to_string(), None),
        ];

        prune_departed(&mut net);

        assert!(
            net.seats.iter().any(|s| s.peer == me.to_string()),
            "the host's own seat must survive the prune"
        );
        assert!(
            !net.seats.iter().any(|s| s.peer == gone.to_string()),
            "the departed peer's seat must go"
        );
        assert!(
            net.seats.iter().any(|s| s.peer == here.to_string()),
            "connected peers keep their seats"
        );
        assert!(
            net.seats.iter().any(|s| s.engine),
            "engines have no peer behind them and survive"
        );
        assert!(
            !net.greeted.contains(&gone),
            "greeting memory of the departed is dropped"
        );
        assert!(net.greeted.contains(&here));
    }

    /// A guest's claim is answered by the roster that settles it — granted,
    /// or the corner held by someone else — and by no other: a roster the
    /// host sent for another change may arrive before the claim was seen.
    #[test]
    fn a_roster_answers_a_pending_claim() {
        let me = PeerId(uuid::Uuid::from_u128(2));
        let mut net = NetState {
            my_id: Some(me),
            seats: vec![seat(&me.to_string(), None), seat("ada", Some(0))],
            ..NetState::default()
        };
        assert_eq!(claim_answer(&net, Some(3)), None, "not answered yet");
        assert_eq!(
            claim_answer(&net, Some(0)).as_deref(),
            Some("Corner 0 went to ada.")
        );
        assert_eq!(
            claim_answer(&net, None).as_deref(),
            Some("Corner released.")
        );

        net.seats[0].player = Some(3);
        assert_eq!(
            claim_answer(&net, Some(3)).as_deref(),
            Some("You hold corner 3.")
        );
        assert_eq!(claim_answer(&net, None), None, "still holding it");
    }

    /// Two peers who drew the same pet name: the one with the larger id gives
    /// way, to a name nobody here has; the other — the host, whenever it is
    /// involved — keeps its own. Engines never clash.
    #[test]
    fn the_later_of_two_same_named_peers_gives_way() {
        let host = PeerId(uuid::Uuid::from_u128(1));
        let guest = PeerId(uuid::Uuid::from_u128(2));
        let gecko = |id: PeerId| Seat {
            name: "gecko".into(),
            ..seat(&id.to_string(), None)
        };
        let roster = vec![gecko(host), gecko(guest)];

        let guest_net = NetState {
            my_id: Some(guest),
            name: "gecko".into(),
            seats: roster.clone(),
            ..NetState::default()
        };
        let renamed = clash_rename(&guest_net).expect("the guest gives way");
        assert_ne!(renamed, "gecko");

        let host_net = NetState {
            my_id: Some(host),
            name: "gecko".into(),
            seats: roster,
            ..NetState::default()
        };
        assert_eq!(clash_rename(&host_net), None, "the host keeps its name");

        let engine_net = NetState {
            my_id: Some(guest),
            name: "Engine".into(),
            seats: vec![engine_seat("engine-0", Some(4))],
            ..NetState::default()
        };
        assert_eq!(clash_rename(&engine_net), None, "engines are not peers");
    }

    /// An idle room has nobody to prune, so the host's per-frame check leaves
    /// the roster untouched; a peer that leaves is noticed.
    #[test]
    fn only_a_departure_triggers_the_prune() {
        let mut net = NetState::default();
        let me = PeerId(uuid::Uuid::from_u128(1));
        let here = PeerId(uuid::Uuid::from_u128(3));
        net.my_id = Some(me);
        net.peers = vec![here];
        net.greeted = vec![here];
        net.seats = vec![
            seat(&me.to_string(), Some(0)),
            seat(&here.to_string(), Some(3)),
            engine_seat("engine-0", Some(4)),
        ];
        assert!(!has_departed(&net), "everyone here is still here");

        net.peers.clear();
        assert!(has_departed(&net), "the peer that left must be pruned");
    }

    /// A rebuilt lobby shows its roster at once, even when nothing changed
    /// since the last draw — as when a round ends and the lobby is spawned
    /// anew over a roster that has been quiet all along.
    #[test]
    fn a_rebuilt_roster_is_drawn_without_a_change() {
        let mut world = World::new();
        world.init_resource::<NetState>();
        world.init_resource::<Table>();
        world.init_resource::<LobbyStatus>();
        let draw = world.register_system(draw_roster);
        world.run_system(draw).expect("first draw");
        world.run_system(draw).expect("an idle draw");

        let text = world.spawn((Text::default(), RosterText)).id();
        world.run_system(draw).expect("the draw after the rebuild");

        let drawn = world.get::<Text>(text).expect("the roster text");
        assert!(!drawn.0.is_empty(), "the fresh roster was left blank");
    }

    /// Shared: clicking a claimed corner selects it — yours, another
    /// player's, or an engine's — so the sidebar's Cancel Seat can act on it;
    /// a free corner is claimed on the spot.
    #[test]
    fn a_shared_click_selects_any_claimed_corner() {
        let mut net = NetState::default();
        let me = PeerId(uuid::Uuid::from_u128(1));
        let other = fake_peer();
        net.my_id = Some(me);
        net.peers = vec![me, other];
        net.seats = vec![
            seat(&me.to_string(), Some(2)),
            seat("grace", Some(1)),
            engine_seat("bot", Some(4)),
        ];
        assert_eq!(sector_click(&net, 2), SectorClick::Select(2));
        assert_eq!(sector_click(&net, 1), SectorClick::Select(1));
        assert_eq!(sector_click(&net, 4), SectorClick::Select(4));
        assert_eq!(sector_click(&net, 3), SectorClick::Claim(3));
    }

    /// The host can un-seat another player's corner; a guest cannot touch a
    /// corner held by someone else.
    #[test]
    fn the_host_unseats_a_player() {
        let host = "host";
        let mut net = shared(true, vec![seat(host, Some(0)), seat("grace", Some(3))]);

        assert_eq!(
            corner_effect(&net, host, 3, CornerCommand::Off),
            Ok(CornerEffect::Unseat(3)),
            "the host un-seats anyone"
        );
        assert_eq!(
            corner_effect(&net, host, 0, CornerCommand::Off),
            Ok(CornerEffect::Claim(None)),
            "releasing one's own corner is a release, not an un-seat"
        );

        net.is_host = false;
        let effect = corner_effect(&net, "grace", 0, CornerCommand::Off);
        assert!(effect.is_err(), "a guest cannot un-seat another player");
    }
}
