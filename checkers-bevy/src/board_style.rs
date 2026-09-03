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

// --- Viewport fitting -------------------------------------------------------

/// How much room the framed board keeps off the screen edge, in both styles.
pub const FIT_MARGIN: f32 = 1.12;

/// The camera distance at which the framed board exactly fills the viewport.
///
/// Worst case is the top-down view: the screen's vertical axis then spans the
/// board's full depth, its horizontal axis the full width divided by the
/// aspect ratio. Oblique angles only foreshorten the depth, so fitting the
/// top-down view fits every pitch. `fov_y` is the camera's vertical field of
/// view, `aspect` the viewport's width over height — both live values, since
/// the window decides the framing and the framing decides the distance.
pub fn orbit_fit_radius(half_extent: Vec2, aspect: f32, fov_y: f32, margin: f32) -> f32 {
    let half = half_extent * margin;
    // The world height that must be visible for both axes to fit.
    let needed_height = half.y.max(half.x / aspect);
    // Visible height at distance `r` is `2 r tan(fov/2)`.
    needed_height / (fov_y / 2.0).tan()
}

/// Keep the 3D board framed as the window changes size or shape.
///
/// A perspective camera cannot be auto-fitted the way an orthographic one
/// can, so instead the system *rescales the orbit* by the ratio of fit
/// distances when the aspect ratio changes: the player's chosen zoom — how
/// tightly the board is framed — survives a window resize intact, and the
/// board's on-screen coverage follows the window rather than drifting off it.
/// Pure size changes need nothing here: with a fixed field of view, coverage
/// already follows the window.
///
/// Classic (2D) has no [`AmlahCamera`]; the query is empty and this is a no-op.
pub fn fit_orbit_to_window(
    windows: Query<&Window, Changed<Window>>,
    cameras: Query<&Projection, With<AmlahCamera>>,
    mut orbit: ResMut<OrbitCamera>,
    mut prev_aspect: Local<Option<f32>>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    // `window.height()` is never zero for a real window, but resize can
    // momentarily report degenerate sizes; the floor keeps the ratio sane.
    let aspect = (window.width() / window.height()).max(0.2);
    let Ok(Projection::Perspective(camera)) = cameras.single() else {
        return;
    };

    let half_extent = crate::board_view::BOARD_HALF_EXTENT * crate::board_amlah::SCALE;
    let fit = |aspect: f32| orbit_fit_radius(half_extent, aspect, camera.fov, FIT_MARGIN);

    let Some(prev) = *prev_aspect else {
        // First frame: adopt the window as-is, no correction.
        *prev_aspect = Some(aspect);
        return;
    };
    if (prev - aspect).abs() < 1e-3 {
        return;
    }

    orbit.radius = (orbit.radius * fit(aspect) / fit(prev))
        .clamp(ORBIT_RADIUS_MIN, ORBIT_RADIUS_MAX);
    *prev_aspect = Some(aspect);
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

    /// At a square viewport the board's depth (the larger half-extent) drives
    /// the fit, and the geometry checks out against the pinhole formula.
    #[test]
    fn fit_radius_matches_the_pinhole_geometry() {
        let half = Vec2::new(3.1, 3.55);
        let fov = std::f32::consts::FRAC_PI_4; // 45 degrees
        let r = orbit_fit_radius(half, 1.0, fov, 1.0);
        assert!((r - 3.55 / (fov / 2.0).tan()).abs() < 1e-4);
    }

    /// Extra width is free: once the depth drives the fit, a wider window
    /// needs no more distance. A narrower one does — the width divided by the
    /// aspect takes over — and a wider field of view needs less.
    #[test]
    fn fit_radius_tracks_the_aspect_ratio() {
        let half = Vec2::new(3.1, 3.55);
        let fov = std::f32::consts::FRAC_PI_4;
        let square = orbit_fit_radius(half, 1.0, fov, 1.0);
        let landscape = orbit_fit_radius(half, 2.0, fov, 1.0);
        let portrait = orbit_fit_radius(half, 0.5, fov, 1.0);
        let narrow = orbit_fit_radius(half, 0.25, fov, 1.0);
        let wide_fov = orbit_fit_radius(half, 1.0, fov * 2.0, 1.0);

        assert!((landscape - square).abs() < 1e-4, "width fits freely at 2:1");
        assert!(portrait > square, "portrait must back the camera up");
        assert!(narrow > portrait, "narrower still needs more distance");
        assert!(wide_fov < square, "a wider field of view needs less distance");
    }

    /// The margin is a plain multiplier, so a framed board keeps it off every
    /// edge at any aspect.
    #[test]
    fn fit_radius_applies_the_margin_evenly() {
        let half = Vec2::new(3.1, 3.55);
        let fov = std::f32::consts::FRAC_PI_4;
        let bare = orbit_fit_radius(half, 1.0, fov, 1.0);
        let framed = orbit_fit_radius(half, 1.0, fov, FIT_MARGIN);
        assert!((framed / bare - FIT_MARGIN).abs() < 1e-4);
    }
}
