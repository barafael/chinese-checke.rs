//! Everything a view-rebuilding system needs in order to draw, bundled so
//! system signatures stay short.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

/// Commands plus the three asset stores the board styles draw with. One
/// parameter instead of four: rebuilding systems (pieces, highlights, the
/// opponent-move trace) would otherwise all cross the clippy argument limit.
#[derive(SystemParam)]
pub struct DrawContext<'w, 's> {
    pub commands: Commands<'w, 's>,
    pub meshes: ResMut<'w, Assets<Mesh>>,
    pub materials: ResMut<'w, Assets<ColorMaterial>>,
    pub std_materials: ResMut<'w, Assets<StandardMaterial>>,
}
