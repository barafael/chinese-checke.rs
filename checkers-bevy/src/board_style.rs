//! Which visualization is showing the board, and the seam that keeps it
//! separate from the game.
//!
//! [`Session`](crate::Session) is the only board state: game, selection,
//! messages. It knows nothing about how it is drawn. Everything visual —
//! camera, board geometry, pieces, highlights — is a pure function of
//! `Session` + [`BoardStyle`], rebuilt wholesale whenever either changes, so
//! switching style mid-play (`V`) never touches the position, the staged
//! turn, or the network state.
//!
//! Adding a visualization means: a variant here, a spawn function for its
//! board geometry, and a branch in the piece/highlight/click systems in
//! `main.rs`. Nothing else.

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
/// geometry. Despawned wholesale when the style changes — pieces and
/// highlights are *not* marked with this, because the sync systems rebuild
/// them from the session on their own.
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
