//! Interface pieces shared by the lobby and the in-game screens.

use bevy::prelude::*;

/// One line of UI text at a pixel size and colour.
pub fn ui_text(
    content: impl Into<String>,
    size: f32,
    colour: Color,
) -> (Text, TextFont, TextColor) {
    (
        Text::new(content),
        TextFont::from_font_size(size),
        TextColor(colour),
    )
}
