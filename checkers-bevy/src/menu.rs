//! The main menu and the hotseat panel: the two screens before the lobby.
//!
//! The menu exists because the app used to open straight into the lobby, which
//! assumed you came to play with other people. Every offline path then lived
//! inside the lobby as an escape hatch (`S`), discoverable only by reading a
//! hint. Now the first screen names the two ways to play, and the lobby is
//! reached only by choosing multiplayer.
//!
//! Built with plain Bevy UI, like the lobby and the in-game controls — one
//! styling vocabulary across all screens, no dependency for four widgets.

use bevy::prelude::*;

use crate::AppState;
use crate::lobby::{CHOSEN, CHOSEN_DOWN, CHOSEN_HOVER, ChosenSeating, DOWN, HOVER, IDLE};
use crate::setup::Seating;

use checkers_core::position::Player;
use checkers_net::NetState;

/// The camps the computer plays, set from the hotseat panel and read when the
/// game is dealt. Local-only by design: a networked seat an engine plays
/// would need protocol support and an agreement about who runs it.
#[derive(Resource, Default, Debug, Clone)]
pub struct AiSeats(pub Vec<Player>);

pub fn plugin(app: &mut App) {
    app.init_resource::<AiSeats>()
        .add_systems(OnEnter(AppState::Menu), spawn_menu)
        .add_systems(OnExit(AppState::Menu), despawn)
        .add_systems(OnEnter(AppState::Hotseat), spawn_hotseat)
        .add_systems(OnExit(AppState::Hotseat), despawn)
        .add_systems(
            Update,
            (handle_buttons, sync_button_styles)
                .run_if(in_state(AppState::Menu).or_else(in_state(AppState::Hotseat))),
        );
}

/// Marker for everything spawned by the menu and the hotseat panel, so both
/// despawn together on leaving either.
#[derive(Component)]
struct MenuUi;

#[derive(Component, Clone, Copy, PartialEq)]
pub enum MenuButton {
    /// Open the multiplayer lobby.
    Lobby,
    /// Open the hotseat panel.
    Hotseat,
    /// Pick a seating for the hotseat game.
    Seats(Seating),
    /// Deal the hotseat game and start.
    Play,
    /// From the hotseat panel back to the menu.
    Back,
    /// Toggle the computer opponent (two-player hotseat only).
    Computer,
}

/// One button, styled exactly like the lobby's (the palette is imported, so
/// the screens share one vocabulary and cannot drift apart).
fn button(parent: &mut ChildSpawnerCommands, label: &str, tag: MenuButton) {
    parent
        .spawn((
            Button,
            Node {
                padding: UiRect::axes(Val::Px(18.0), Val::Px(10.0)),
                border_radius: BorderRadius::all(Val::Px(5.0)),
                ..default()
            },
            BackgroundColor(IDLE),
            tag,
        ))
        .with_child((
            Text::new(label),
            TextFont {
                font_size: FontSize::Px(17.0),
                ..default()
            },
            TextColor(Color::srgb(0.9, 0.9, 0.92)),
        ));
}

fn screen(commands: &mut Commands) -> Entity {
    // One centred column filling the window. Both screens are short, so they
    // are anchored to the middle rather than the top: the intro should read as
    // a title card, not as the top of a form.
    commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                row_gap: Val::Px(16.0),
                ..default()
            },
            MenuUi,
        ))
        .id()
}

fn spawn_menu(mut commands: Commands) {
    let screen = screen(&mut commands);
    commands.entity(screen).with_children(|col| {
        col.spawn((
            Text::new("Chinese Checkers"),
            TextFont {
                font_size: FontSize::Px(34.0),
                ..default()
            },
            TextColor(Color::srgb(0.95, 0.95, 0.97)),
        ));
        col.spawn((
            Text::new("the machine-checked star"),
            TextFont {
                font_size: FontSize::Px(14.0),
                ..default()
            },
            TextColor(Color::srgb(0.6, 0.6, 0.66)),
        ));

        // A little air between the title and the choices.
        col.spawn(Node {
            height: Val::Px(24.0),
            ..default()
        });

        col.spawn(Node {
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(12.0),
            align_items: AlignItems::Center,
            ..default()
        })
        .with_children(|col| {
            // Buttons wide enough to read as the two paths, stacked so the eye
            // picks one without scanning a row.
            for (label, tag) in [
                ("Multiplayer", MenuButton::Lobby),
                ("Hotseat", MenuButton::Hotseat),
            ] {
                col.spawn((
                    Button,
                    Node {
                        width: Val::Px(220.0),
                        justify_content: JustifyContent::Center,
                        padding: UiRect::axes(Val::Px(18.0), Val::Px(10.0)),
                        border_radius: BorderRadius::all(Val::Px(5.0)),
                        ..default()
                    },
                    BackgroundColor(IDLE),
                    tag,
                ))
                .with_child((
                    Text::new(label),
                    TextFont {
                        font_size: FontSize::Px(17.0),
                        ..default()
                    },
                    TextColor(Color::srgb(0.9, 0.9, 0.92)),
                ));
            }
        });
    });
}

fn spawn_hotseat(mut commands: Commands, chosen: Res<ChosenSeating>) {
    let screen = screen(&mut commands);
    commands.entity(screen).with_children(|col| {
        col.spawn((
            Text::new("Hotseat"),
            TextFont {
                font_size: FontSize::Px(28.0),
                ..default()
            },
            TextColor(Color::srgb(0.95, 0.95, 0.97)),
        ));
        col.spawn((
            Text::new("everyone plays on this device - pass it as turns pass"),
            TextFont {
                font_size: FontSize::Px(14.0),
                ..default()
            },
            TextColor(Color::srgb(0.6, 0.6, 0.66)),
        ));

        col.spawn(Node {
            height: Val::Px(16.0),
            ..default()
        });

        col.spawn(Node {
            column_gap: Val::Px(10.0),
            align_items: AlignItems::Center,
            ..default()
        })
        .with_children(|row| {
            row.spawn((
                Text::new("Players"),
                TextFont {
                    font_size: FontSize::Px(14.0),
                    ..default()
                },
                TextColor(Color::srgb(0.62, 0.62, 0.68)),
            ));
            for seating in Seating::ALL {
                button(row, seating.label(), MenuButton::Seats(seating));
            }
        });

        col.spawn(Node {
            column_gap: Val::Px(10.0),
            margin: UiRect::top(Val::Px(8.0)),
            ..default()
        })
        .with_children(|row| {
            button(row, "Play", MenuButton::Play);
            button(row, "Computer opponent", MenuButton::Computer);
            button(row, "Back", MenuButton::Back);
        });

        // Which seating is currently picked, shown in place rather than as a
        // separate status line to read.
        col.spawn((
            Text::new(format!("Deals {}", chosen.0.label())),
            TextFont {
                font_size: FontSize::Px(14.0),
                ..default()
            },
            TextColor(Color::srgb(0.62, 0.62, 0.68)),
            DealsText,
        ));
    });

    // Repaint the highlight before the first interaction.
    commands.insert_resource(*chosen);
}

/// The hotseat panel's "Deals ..." line, so picking a player count is visible
/// without waiting for anything to change elsewhere.
#[derive(Component)]
struct DealsText;

fn despawn(mut commands: Commands, ui: Query<Entity, With<MenuUi>>) {
    for e in ui.iter() {
        commands.entity(e).despawn();
    }
}

/// Paint every button: chosen seating, hover, press. The hotseat panel has no
/// keys, so this is also what makes picking a player count visible — before
/// this, the seat buttons never highlighted at all.
fn sync_button_styles(
    chosen: Res<ChosenSeating>,
    mut buttons: Query<(&Interaction, &MenuButton, &mut BackgroundColor)>,
) {
    for (interaction, button, mut bg) in buttons.iter_mut() {
        let selected = matches!(button, MenuButton::Seats(s) if *s == chosen.0);
        let colour = match interaction {
            Interaction::Pressed if selected => CHOSEN_DOWN,
            Interaction::Pressed => DOWN,
            Interaction::Hovered if selected => CHOSEN_HOVER,
            Interaction::Hovered => HOVER,
            Interaction::None if selected => CHOSEN,
            Interaction::None => IDLE,
        };
        if bg.0 != colour {
            bg.0 = colour;
        }
    }
}

/// What a menu button press means, split out of the system so the rules are
/// testable headlessly.
#[derive(Debug, PartialEq, Eq)]
pub enum MenuAction {
    None,
    ToLobby,
    ToHotseat,
    ToMenu,
    /// Deal the hotseat game with the chosen seating.
    Play,
    /// Toggle the computer opponent.
    Computer,
    Pick(Seating),
}

pub fn action_for(button: MenuButton) -> MenuAction {
    match button {
        MenuButton::Lobby => MenuAction::ToLobby,
        MenuButton::Hotseat => MenuAction::ToHotseat,
        MenuButton::Back => MenuAction::ToMenu,
        MenuButton::Play => MenuAction::Play,
        MenuButton::Computer => MenuAction::Computer,
        MenuButton::Seats(s) => MenuAction::Pick(s),
    }
}

fn handle_buttons(
    buttons: Query<(&Interaction, &MenuButton), Changed<Interaction>>,
    mut chosen: ResMut<ChosenSeating>,
    mut net: ResMut<NetState>,
    mut ai_seats: ResMut<AiSeats>,
    mut next_state: ResMut<NextState<AppState>>,
    mut deals: Query<&mut Text, With<DealsText>>,
) {
    let mut action = MenuAction::None;
    for (interaction, button) in buttons.iter() {
        if *interaction == Interaction::Pressed {
            action = action_for(*button);
        }
    }

    match action {
        MenuAction::None => {}
        // Multiplayer seats humans only; the computer is a hotseat feature.
        MenuAction::ToLobby => {
            ai_seats.0.clear();
            next_state.set(AppState::Lobby);
        }
        MenuAction::ToHotseat => next_state.set(AppState::Hotseat),
        MenuAction::ToMenu => next_state.set(AppState::Menu),
        MenuAction::Pick(s) => chosen.0 = s,
        MenuAction::Computer => {
            // The computer takes the second seat of the two-player deal. Any
            // other seating is played by the humans present.
            if chosen.0 == Seating::Two && ai_seats.0 == vec![Player::ALL[3]] {
                ai_seats.0.clear();
            } else if chosen.0 == Seating::Two {
                ai_seats.0 = vec![Player::ALL[3]];
            }
        }
        MenuAction::Play => {
            // Hotseat is offline by definition: drop any seat binding the
            // lobby's greetings left behind, or the solo player would be
            // pinned to one camp. The session itself is rebuilt on entering
            // the game.
            net.unbind_players();
            info!("starting a hotseat game");
            next_state.set(AppState::InGame);
        }
    }

    // Keep the "Deals ..." line honest. The early-out is the same pattern as
    // the lobby's sync systems: repaint only when the choice moved.
    if chosen.is_changed()
        && let Ok(mut text) = deals.single_mut()
    {
        **text = format!("Deals {}", chosen.0.label());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The menu is the front door; a button that lands nowhere would strand a
    /// player before any screen has even appeared.
    #[test]
    fn every_button_lands_somewhere() {
        assert_eq!(action_for(MenuButton::Lobby), MenuAction::ToLobby);
        assert_eq!(action_for(MenuButton::Hotseat), MenuAction::ToHotseat);
        assert_eq!(action_for(MenuButton::Back), MenuAction::ToMenu);
        assert_eq!(action_for(MenuButton::Play), MenuAction::Play);
        assert_eq!(
            action_for(MenuButton::Seats(Seating::Three)),
            MenuAction::Pick(Seating::Three)
        );
    }
}
