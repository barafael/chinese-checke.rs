//! Which visualization is showing the board.
//!
//! [`Session`](crate::Session) is board state; visuals are a pure function
//! of session + [`BoardStyle`], rebuilt wholesale on change, so switching
//! (`V`) never touches play. Adding a style: a variant here, a spawn
//! function, branches in the piece/highlight/click systems.

use bevy::prelude::*;

/// The board visualization currently on screen.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoardStyle {
    /// The original flat view: dark board, one entity per hole, 2D camera.
    #[default]
    Classic,
    /// The amlah look: cream hex plate with accent camp triangles, black
    /// holes and connection lines baked into three meshes, cone pieces, and
    /// a fixed tilted 3D camera.
    Amlah,
}

impl BoardStyle {
    /// The next style in the cycle, for the `V` key.
    pub fn next(self) -> Self {
        match self {
            Self::Classic => Self::Amlah,
            Self::Amlah => Self::Classic,
        }
    }

    /// Human-readable name for the status line.
    pub fn label(self) -> &'static str {
        match self {
            Self::Classic => "classic (2D)",
            Self::Amlah => "amlah (3D)",
        }
    }
}

/// Structure owned by the active visualization: its camera and board
/// geometry. Pieces and highlights are rebuilt by the sync systems instead.
#[derive(Component)]
pub struct BoardVisual;

/// The 2D camera of the classic style.
#[derive(Component)]
pub struct ClassicCamera;

/// The 3D camera of the amlah style.
#[derive(Component)]
pub struct AmlahCamera;

/// `V` cycles the visualization. Deliberately never touches the session:
/// switching is a view operation, and the new style re-derives everything
/// from the unchanged board state.
pub fn handle_style_key(keys: Res<ButtonInput<KeyCode>>, mut style: ResMut<BoardStyle>) {
    if keys.just_pressed(KeyCode::KeyV) {
        *style = style.next();
        info!("board style: {}", style.label());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two styles for now; `V` must keep cycling between them forever.
    #[test]
    fn styles_cycle() {
        assert_eq!(BoardStyle::default(), BoardStyle::Classic);
        assert_eq!(BoardStyle::Classic.next(), BoardStyle::Amlah);
        assert_eq!(BoardStyle::Amlah.next(), BoardStyle::Classic);
    }
}
