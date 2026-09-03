//! Which visualization is showing the board.
//!
//! [`Session`](crate::Session) is board state; visuals are a pure function
//! of session + [`BoardStyle`], rebuilt wholesale on change, so switching
//! (`V`) never touches play. Adding a style: a variant here, a spawn
//! function, branches in the piece/highlight/click systems.

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
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

// --- Orbital 3D camera -----------------------------------------------------
//
// The 3D (amlah) style keeps the board flat on the ground while the camera
// floats free: the player orbits the board centre and zooms, and mouse picking
// keeps working from any angle because it always casts the cursor ray onto the
// board plane. The state below is view-only — it never touches the session.

/// How slowly a right-drag pixel moves the camera around the board, in radians.
pub const ORBIT_YAW_SPEED: f32 = 0.006;
/// Same, for the camera's elevation (pitch).
pub const ORBIT_PITCH_SPEED: f32 = 0.005;
/// Zoom gain per scroll notch (`radius *= exp(` this × delta `)`).
pub const ORBIT_WHEEL_SPEED: f32 = 0.12;
/// The camera may dip near-flat, but never quite horizontal, or the
/// `look_at` up-vector degenerates.
pub const ORBIT_PITCH_MIN: f32 = 0.03;
/// Nor may it go fully overhead, for the same reason.
pub const ORBIT_PITCH_MAX: f32 = 1.5;
/// Zoom clamp in 3D units: close enough to read the camps, far enough to take
/// the whole star in.
pub const ORBIT_RADIUS_MIN: f32 = 5.0;
pub const ORBIT_RADIUS_MAX: f32 = 40.0;

/// The orbit state of the 3D camera.
///
/// The camera sits on a sphere around the board centre (`Vec3::ZERO`) at
/// [`OrbitCamera::radius`], with [`OrbitCamera::yaw`] around the vertical axis
/// and [`OrbitCamera::pitch`] above the board plane. The default reproduces
/// the fixed tilted camera the amlah style used to spawn, so nothing moves
/// until the player drags or scrolls.
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct OrbitCamera {
    /// Angle around the vertical `Y` axis, in radians.
    pub yaw: f32,
    /// Elevation above the board plane, in radians.
    pub pitch: f32,
    /// Distance from the board centre, in 3D units.
    pub radius: f32,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        // Match the fixed camera the amlah style spawned before orbits existed.
        let pos = crate::board_amlah::CAMERA_POS;
        let radius = pos.length();
        Self {
            yaw: 0.0, // CAMERA_POS sits at x = 0, on the +Z side.
            pitch: (pos.y / radius).asin(),
            radius,
        }
    }
}

impl OrbitCamera {
    /// The camera position on the sphere around the board centre.
    pub fn eye(self) -> Vec3 {
        let y = self.radius * self.pitch.sin();
        let horiz = self.radius * self.pitch.cos();
        Vec3::new(horiz * self.yaw.sin(), y, horiz * self.yaw.cos())
    }
}

/// Drive the 3D camera: right-drag orbits the board, the wheel zooms.
///
/// The classic (2D) style has no [`AmlahCamera`], so the query is empty there
/// and this is a no-op. Picking needs no change: `handle_clicks` casts each
/// cursor ray onto the board plane from the camera's current transform, which
/// is exactly what the classic style already does for its fixed camera.
///
/// The transform is applied every frame rather than only on input, so the
/// player's orbit survives a style switch out and back: `apply_style` respawns
/// the camera at the fixed initial position, and this system silently returns
/// it to the remembered view on the same frame.
pub fn orbit_camera(
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut orbit: ResMut<OrbitCamera>,
    mut cam: Query<&mut Transform, With<AmlahCamera>>,
) {
    let Ok(mut transform) = cam.single_mut() else {
        return;
    };

    // Right-drag orbits. Dragging up raises the camera; dragging right sweeps
    // it around the board (a "grab the table and turn it" feel).
    if mouse.pressed(MouseButton::Right) && motion.delta != Vec2::ZERO {
        let d = motion.delta;
        orbit.yaw += d.x * ORBIT_YAW_SPEED;
        orbit.pitch =
            (orbit.pitch - d.y * ORBIT_PITCH_SPEED).clamp(ORBIT_PITCH_MIN, ORBIT_PITCH_MAX);
    }

    // The wheel zooms in and out.
    if scroll.delta.y != 0.0 {
        let factor = (scroll.delta.y * ORBIT_WHEEL_SPEED).exp();
        orbit.radius = (orbit.radius * factor).clamp(ORBIT_RADIUS_MIN, ORBIT_RADIUS_MAX);
    }

    transform.translation = orbit.eye();
    transform.look_at(Vec3::ZERO, Vec3::Y);
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

    /// The default orbit reproduces the fixed camera the amlah style spawned,
    /// so switching to it does not jerk the view on the first frame.
    #[test]
    fn default_orbit_lands_on_the_legacy_camera_position() {
        let orbit = OrbitCamera::default();
        let eye = orbit.eye();
        let wanted = crate::board_amlah::CAMERA_POS;
        assert!(
            (eye - wanted).length() < 1e-3,
            "default orbit put the camera at {eye:?}, expected {wanted:?}"
        );
    }

    /// The camera eye must always sit above the board with a usable up-axis.
    /// The pitch clamp keeps the eye from ever going flat-or-overhead, and any
    /// positive pitch yields a positive height, so `look_at` (which needs a
    /// non-parallel up-vector) is always well defined.
    #[test]
    fn pitch_clamp_keeps_the_view_above_the_board() {
        // The lowest allowed pitch still looks down on the board.
        let low = OrbitCamera {
            pitch: ORBIT_PITCH_MIN,
            ..OrbitCamera::default()
        };
        let high = OrbitCamera {
            pitch: ORBIT_PITCH_MAX,
            ..OrbitCamera::default()
        };
        assert!(low.eye().y > 0.0);
        assert!(high.eye().y > high.radius * 0.9); // steepest, not quite overhead
        // And the up-vector stays usable: the eye is never straight above the
        // origin, which is the one pose `look_at(..., Vec3::Y)` cannot take.
        let horiz = Vec2::new(high.eye().x, high.eye().z).length();
        assert!(horiz > 0.001, "eye must not sit on the vertical axis");
    }
}
