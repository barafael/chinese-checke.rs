//! Everything a view-rebuilding system needs in order to draw, bundled so
//! system signatures stay short.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

/// Commands plus the two asset stores the board draws with, shared by every
/// rebuilding system (pieces, highlights, camp rings, the opponent-move
/// trace).
#[derive(SystemParam)]
pub struct DrawContext<'w, 's> {
    pub commands: Commands<'w, 's>,
    pub meshes: ResMut<'w, Assets<Mesh>>,
    pub materials: ResMut<'w, Assets<ColorMaterial>>,
}
