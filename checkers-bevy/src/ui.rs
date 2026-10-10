//! Interface pieces shared by the lobby and the in-game screens.

use bevy::prelude::*;
use bevy::ui_widgets::Activate;

/// One line of UI text at a pixel size and colour.
///
/// Sized in pixels rather than left at Bevy's default `Rem(1.)`: `UiScale`
/// already scales the whole interface with the window, and these sizes are
/// chosen against that.
pub fn ui_text(content: impl Into<String>, size: f32, colour: Color) -> impl Scene {
    let content = content.into();
    bsn! {
        Text(content)
        TextFont { font_size: FontSize::Px(size) }
        TextColor(colour)
    }
}

/// Report every activated button tagged `T` as a `T` message.
///
/// A button fires [`Activate`] when a click completes on it (or Enter/Space
/// while it holds focus). The systems that act on clicks also act on keys, in
/// a fixed place in the frame — Enter starts the game exactly as the Start
/// button does — so the activation is handed to them as a message rather than
/// acted on in an observer of its own, outside that order.
pub fn button_messages<T: Component + Message + Clone>(app: &mut App) {
    app.add_message::<T>().add_observer(forward_activation::<T>);
}

fn forward_activation<T: Component + Message + Clone>(
    activate: On<Activate>,
    tags: Query<&T>,
    mut pressed: MessageWriter<T>,
) {
    if let Ok(tag) = tags.get(activate.entity) {
        pressed.write(tag.clone());
    }
}
