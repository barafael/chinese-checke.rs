//! The tweak pane (a dev aid): **Ctrl+Shift** opens a small pane over the
//! lobby or the board that sets the backdrop's parameters live, as
//! omdurman's title-screen tuning pane does. Everything starts from
//! [`NebulaParams::default`]; each finished change logs the whole set, ready
//! to be copied back there.
//!
//! Built from Bevy's own widgets: a feathers-styled [`ModalDialog`] in the
//! top right corner over a clear [`ModalDialogBarrier`] that covers the
//! window. The barrier takes every click meant for the interface behind it,
//! and a click on it, Escape, the close box or Ctrl+Shift again closes the
//! pane. The board polls the mouse and the keyboard rather than being
//! picked, so while the pane is open their presses are cleared too.

use bevy::feathers::FeathersPlugins;
use bevy::feathers::controls::{
    FeathersButton, FeathersCheckbox, FeathersDialogBody, FeathersDialogClose,
    FeathersDialogHeader, FeathersSlider,
};
use bevy::feathers::dark_theme::create_dark_theme;
use bevy::feathers::display::{caption, label, label_small};
use bevy::feathers::theme::{
    InheritableThemeTextColor, ThemeBackgroundColor, ThemeBorderColor, UiTheme,
};
use bevy::feathers::tokens;
use bevy::input::InputSystems;
use bevy::input_focus::tab_navigation::TabNavigationPlugin;
use bevy::input_focus::{InputFocus, InputFocusSystems};
use bevy::picking::PickingSystems;
use bevy::prelude::*;
use bevy::scene::on;
use bevy::ui::{Checked, FixedNode, OverrideClip};
use bevy::ui_widgets::{
    Activate, ModalDialog, ModalDialogBarrier, RequestClose, SliderPrecision, SliderStep,
    SliderValue, ValueChange,
};

use crate::nebula::NebulaParams;

/// Whether the pane is open.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct TweakPane {
    pub open: bool,
}

/// Run condition: the pane is closed, so the game may take input.
pub fn pane_closed(pane: Res<TweakPane>) -> bool {
    !pane.open
}

pub fn plugin(app: &mut App) {
    if !app.is_plugin_added::<TabNavigationPlugin>() {
        app.add_plugins(FeathersPlugins);
    }
    app.insert_resource(UiTheme(create_dark_theme()))
        .init_resource::<TweakPane>()
        .add_systems(
            PreUpdate,
            (toggle, hold_input)
                .chain()
                .after(InputSystems)
                .before(InputFocusSystems::Dispatch)
                .before(PickingSystems::ProcessInput),
        )
        .add_systems(
            Update,
            (
                show_pane,
                sync_widgets.run_if(|pane: Res<TweakPane>| pane.open),
            )
                .chain(),
        );
}

/// Ctrl+Shift opens and closes the pane: both held, one of them just
/// pressed, in either order.
fn toggle(keys: Res<ButtonInput<KeyCode>>, mut pane: ResMut<TweakPane>) {
    let ctrl = [KeyCode::ControlLeft, KeyCode::ControlRight];
    let shift = [KeyCode::ShiftLeft, KeyCode::ShiftRight];
    let held = keys.any_pressed(ctrl) && keys.any_pressed(shift);
    let fresh = keys.any_just_pressed(ctrl) || keys.any_just_pressed(shift);
    if held && fresh {
        pane.open = !pane.open;
    }
}

/// While the pane is open, the board and the game's shortcuts hear nothing:
/// this frame's presses are cleared before any game or lobby system polls
/// them. Held keys stay held, so Ctrl+Shift can still close the pane. The
/// pane's widgets take their input from picking and keyboard focus, not from
/// these, so they are unaffected.
///
/// Ahead of focus dispatch and picking, which is where an Escape or a click
/// closes the pane: the press that closed it was cleared before it could
/// reach anything behind it.
fn hold_input(
    pane: Res<TweakPane>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
) {
    if pane.open {
        keys.clear();
        mouse.clear();
    }
}

/// The pane's root: the barrier over the whole window.
#[derive(Component, Default, Clone)]
struct PaneRoot;

/// A slider bound to one [`Knob`].
#[derive(Component, Clone, Copy)]
struct KnobSlider(Knob);

/// The checkbox that shows or hides the nebula.
#[derive(Component, Default, Clone)]
struct ShownBox;

/// The text naming the current seed.
#[derive(Component, Default, Clone)]
struct SeedText;

/// Spawn the pane when it opens, despawn it when it closes. Opening takes
/// the keyboard from whatever text field held it; the dialog then moves
/// focus into itself.
fn show_pane(
    pane: Res<TweakPane>,
    roots: Query<Entity, With<PaneRoot>>,
    mut focus: ResMut<InputFocus>,
    mut commands: Commands,
) {
    if !pane.is_changed() {
        return;
    }
    match (pane.open, roots.is_empty()) {
        (true, true) => {
            focus.clear();
            commands.spawn_scene(pane_scene());
        }
        (false, false) => {
            for root in &roots {
                commands.entity(root).despawn();
            }
        }
        _ => {}
    }
}

/// The pane's width, logical pixels.
const PANE_WIDTH: f32 = 380.0;

fn pane_scene() -> impl Scene {
    let knobs: Vec<_> = Knob::ALL.into_iter().map(knob_row).collect();
    bsn! {
        PaneRoot
        Node {
            display: Display::Flex,
            position_type: PositionType::Absolute,
            left: px(0),
            right: px(0),
            top: px(0),
            bottom: px(0),
            justify_content: JustifyContent::FlexEnd,
            align_items: AlignItems::FlexStart,
            padding: UiRect::all(px(12)),
        }
        ModalDialogBarrier
        FixedNode
        OverrideClip
        GlobalZIndex(99)
        // A click on the barrier, Escape, or the close box.
        on(|_: On<RequestClose>, mut pane: ResMut<TweakPane>| pane.open = false)
        Children [
            Node {
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Stretch,
                border_radius: BorderRadius::all(px(4)),
                border: UiRect::all(px(1)),
                width: px(PANE_WIDTH),
            }
            ModalDialog
            ThemeBackgroundColor(tokens::DIALOG_BG)
            ThemeBorderColor(tokens::DIALOG_BORDER)
            InheritableThemeTextColor(tokens::DIALOG_TEXT)
            Children [
                @FeathersDialogHeader
                InheritableThemeTextColor(tokens::DIALOG_HEADER_TEXT)
                Children [
                    @caption("Backdrop  (Ctrl+Shift)")
                    --
                    @FeathersDialogClose
                ]
                --
                @FeathersDialogBody
                Node { row_gap: px(6) }
                Children [
                    @FeathersCheckbox { @caption: bsn! { @caption("Nebula") } }
                    ShownBox
                    on(shown_changed)
                    --
                    Node {
                        flex_direction: FlexDirection::Row,
                        align_items: AlignItems::Center,
                        column_gap: px(6),
                    }
                    Children [
                        @FeathersButton { @caption: bsn! { @caption("Seed -") } }
                        on(|_: On<Activate>, mut p: ResMut<NebulaParams>| step_seed(&mut p, -1))
                        --
                        @FeathersButton { @caption: bsn! { @caption("Seed +") } }
                        on(|_: On<Activate>, mut p: ResMut<NebulaParams>| step_seed(&mut p, 1))
                        --
                        @label("") SeedText
                    ]
                    --
                    {knobs}
                ]
            ]
        ]
    }
}

/// One knob: its name above its slider.
fn knob_row(knob: Knob) -> impl Scene {
    let (min, max, decimals) = knob.range();
    let tag = KnobSlider(knob);
    let precision = SliderPrecision(decimals as i32);
    let step = SliderStep(10f32.powi(-(decimals as i32)));
    bsn! {
        Node {
            flex_direction: FlexDirection::Column,
            row_gap: px(2),
        }
        Children [
            @label_small(knob.label())
            --
            @FeathersSlider { @min: {min}, @max: {max} }
            tag
            precision
            step
            on(knob_changed)
        ]
    }
}

/// Seeds step by one either way, and never reach zero (the generator's
/// xorshift would be stuck there).
fn step_seed(params: &mut NebulaParams, by: i64) {
    params.seed = params.seed.wrapping_add_signed(by).max(1);
    info!("tweak: {}", describe(params));
}

fn shown_changed(change: On<ValueChange<bool>>, mut params: ResMut<NebulaParams>) {
    params.shown = change.value;
    info!("tweak: {}", describe(&params));
}

/// A slider moved: set its knob, and log the set once the drag is done.
fn knob_changed(
    change: On<ValueChange<f32>>,
    knobs: Query<&KnobSlider>,
    mut params: ResMut<NebulaParams>,
) {
    let Ok(KnobSlider(knob)) = knobs.get(change.source) else {
        return;
    };
    knob.set(&mut params, change.value);
    if change.is_final {
        info!("tweak: {}", describe(&params));
    }
}

/// Show the params in the widgets: the sliders and the checkbox hold no
/// state of their own, so they follow [`NebulaParams`], whether it changed
/// from the pane or from anywhere else. A handful of widgets, and only while
/// the pane is open, so it simply runs every frame, writing only what
/// differs.
fn sync_widgets(
    params: Res<NebulaParams>,
    sliders: Query<(Entity, &KnobSlider, &SliderValue)>,
    boxes: Query<(Entity, Has<Checked>), With<ShownBox>>,
    mut seed: Query<&mut Text, With<SeedText>>,
    mut commands: Commands,
) {
    for (entity, KnobSlider(knob), value) in &sliders {
        let want = knob.get(&params);
        if value.0 != want {
            commands.entity(entity).insert(SliderValue(want));
        }
    }
    for (entity, checked) in &boxes {
        match (params.shown, checked) {
            (true, false) => {
                commands.entity(entity).insert(Checked);
            }
            (false, true) => {
                commands.entity(entity).remove::<Checked>();
            }
            _ => {}
        }
    }
    for mut text in &mut seed {
        let want = format!("{:#x}", params.seed);
        if text.0 != want {
            text.0 = want;
        }
    }
}

/// One slider of the pane.
#[derive(Clone, Copy, Debug)]
enum Knob {
    Opacity,
    Period,
    Red,
    Green,
    Blue,
}

impl Knob {
    const ALL: [Knob; 5] = [
        Knob::Opacity,
        Knob::Period,
        Knob::Red,
        Knob::Green,
        Knob::Blue,
    ];

    fn label(self) -> &'static str {
        match self {
            Knob::Opacity => "Opacity",
            Knob::Period => "Waver period (s, there and back)",
            Knob::Red => "Tint red",
            Knob::Green => "Tint green",
            Knob::Blue => "Tint blue",
        }
    }

    /// Range and decimals.
    fn range(self) -> (f32, f32, usize) {
        match self {
            Knob::Opacity => (0.0, 0.5, 3),
            Knob::Period => (10.0, 1200.0, 0),
            Knob::Red | Knob::Green | Knob::Blue => (0.0, 1.0, 2),
        }
    }

    fn get(self, p: &NebulaParams) -> f32 {
        match self {
            Knob::Opacity => p.opacity,
            Knob::Period => p.period_secs,
            Knob::Red => p.tint[0],
            Knob::Green => p.tint[1],
            Knob::Blue => p.tint[2],
        }
    }

    fn set(self, p: &mut NebulaParams, value: f32) {
        match self {
            Knob::Opacity => p.opacity = value,
            Knob::Period => p.period_secs = value,
            Knob::Red => p.tint[0] = value,
            Knob::Green => p.tint[1] = value,
            Knob::Blue => p.tint[2] = value,
        }
    }
}

/// The values as [`NebulaParams::default`] spells them.
fn describe(p: &NebulaParams) -> String {
    format!(
        "NebulaParams {{ shown: {}, seed: {:#x}, opacity: {:.3}, tint: [{:.2}, {:.2}, {:.2}], \
         period_secs: {:.0} }}",
        p.shown, p.seed, p.opacity, p.tint[0], p.tint[1], p.tint[2], p.period_secs,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every knob starts inside its slider's range, and sets exactly what it
    /// reads.
    #[test]
    fn the_knobs_start_in_range_and_round_trip() {
        let defaults = NebulaParams::default();
        for (i, knob) in Knob::ALL.into_iter().enumerate() {
            let (min, max, _) = knob.range();
            let value = knob.get(&defaults);
            assert!((min..=max).contains(&value), "{knob:?} {value}");
            let mut params = defaults;
            let other = (min + max) / 2.0 + 0.125;
            knob.set(&mut params, other);
            assert_eq!(knob.get(&params), other, "{knob:?}");
            for (j, untouched) in Knob::ALL.into_iter().enumerate() {
                if i != j {
                    assert_eq!(untouched.get(&params), untouched.get(&defaults));
                }
            }
        }
    }

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<TweakPane>()
            .add_systems(PreUpdate, (toggle, hold_input).chain());
        app
    }

    fn open(app: &App) -> bool {
        app.world().resource::<TweakPane>().open
    }

    /// Shift then Ctrl opens the pane, Ctrl then Shift closes it; and while
    /// it is open, a press reaches nothing behind it.
    #[test]
    fn ctrl_shift_opens_the_pane_and_holds_the_input() {
        let mut app = app();
        let press = |app: &mut App, key: KeyCode| {
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(key);
            app.update();
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .clear();
        };
        let release_all = |app: &mut App| {
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .release_all();
        };

        press(&mut app, KeyCode::ShiftLeft);
        assert!(!open(&app), "Shift alone does nothing");
        press(&mut app, KeyCode::ControlLeft);
        assert!(open(&app), "Shift, then Ctrl, opens it");

        release_all(&mut app);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Enter);
        app.update();
        let keys = app.world().resource::<ButtonInput<KeyCode>>();
        assert!(!keys.just_pressed(KeyCode::Enter), "the game never sees it");
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear();

        release_all(&mut app);
        press(&mut app, KeyCode::ControlRight);
        press(&mut app, KeyCode::ShiftRight);
        assert!(!open(&app), "Ctrl, then Shift, closes it again");
    }
}
