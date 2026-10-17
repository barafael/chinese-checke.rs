//! The lobby's living backdrop: a triangle universe seen from far above —
//! the camera looms high and leans away from straight down, so the field
//! falls away toward the distance — carrying a Game of Life played out on
//! its cells, and, now and then, a cell hopping over a neighbour the way a
//! piece hops over another on the board.
//!
//! The camera is a fake: the app builds Bevy without the 3D pipeline, so the
//! [`Rig`] projects a ground plane through a tilted pinhole on the CPU, and
//! the result is drawn as one ordinary vertex-coloured [`Mesh2d`]. The rig
//! pans slowly over the plane and sways its lean; a gentle swell travels
//! through the surface.
//!
//! Each triangle is a cell and is shaded flat in its own colour — the life
//! reads as the triangular lattice itself, crisp and cell-by-cell, dissolving
//! into the clear colour in the far distance. Every generation the field is
//! cut into horizontal bands that advance at offset times, so the life
//! staggers across the field rather than changing all at once, and the hops
//! trickle in a slice at a time between generations.
//!
//! Life runs on the triangular tessellation itself: every triangle is a cell
//! with twelve neighbours — three across its edges, nine at its corners —
//! under Conway's B3/S23. Between generations, cells sometimes jump: over an
//! occupied same-orientation neighbour, into the empty cell exactly beyond,
//! along one of the six hex directions of that sublattice — the Chinese
//! checkers hop, which the same-orientation cells form for free.
//!
//! The colour gradient is adapted from cake's sky: from the page's deep
//! near-black (`#0f1014`) toward the violet of its nebula tint, drifting at a
//! small scale so the tint visibly varies across the field. A seed comes from
//! the room name — the same trick the nebula plays — so everyone in a room
//! starts from the same soup; the fields drift apart from there, since the
//! hops roll their dice once per frame.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use bevy::asset::RenderAssetUsages;
use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use bevy::render::render_resource::PrimitiveTopology;
use bevy::window::WindowResized;
use checkers_net::RoomId;

use crate::AppState;

pub fn plugin(app: &mut App) {
    app.add_systems(OnEnter(AppState::Lobby), spawn)
        .add_systems(OnExit(AppState::Lobby), teardown)
        .add_systems(
            Update,
            // A rebuild first, so the frame that resizes the sheet also
            // projects and paints it; never a frame of the blank new mesh.
            (ensure_coverage, loom, tick_life)
                .chain()
                .run_if(in_state(AppState::Lobby)),
        );
}

/// The backdrop entity: one continuous sheet of triangles, the life living
/// on it, and the rig it is seen through. Gone with the lobby; the board
/// deserves a quiet room.
#[derive(Component)]
#[require(DespawnOnExit<AppState> = DespawnOnExit(AppState::Lobby))]
struct Backdrop {
    mesh: Handle<Mesh>,
    rig: Rig,
    grid: LifeGrid,
    /// Scratch for the projected vertices, reused every frame.
    positions: Vec<[f32; 3]>,
    /// One generation clock per stagger band, phase-spread across the tick.
    clocks: Vec<f32>,
    /// The clock for the flashes and the reseed.
    upkeep: f32,
    /// The viewport half extent the field was built to cover, with the
    /// camera's whole roam. A window that outgrows it needs a wider sheet.
    built_for: Vec2,
}

/// How far the camera leans from straight down, in degrees. Straight down
/// would be a scan; this far over, the field reads as a universe seen from
/// very high up.
const TILT_DEG: f32 = 35.0;
/// How high the camera looms, in plane units. High and tilted: distance is
/// the whole look.
const HEIGHT: f32 = 700.0;
/// How far the camera wanders over the plane, in plane units.
const CAM_PAN: f32 = 200.0;
/// How far the lean sways either side of its heading, and how long a full
/// sway takes.
const YAW_SWAY: f32 = 9.0f32.to_radians();
const YAW_SWAY_PERIOD_SECS: f32 = 480.0;
/// Seconds between generations. The life is a slow bloom, not a busy one.
const TICK_SECS: f32 = 1.5;
/// The chance, per live cell per generation, of trying to hop.
const HOP_CHANCE: f32 = 0.01;
/// How many ticks a hop's landing cell stays lit.
const FLASH_TICKS: u8 = 2;
/// How many horizontal bands the field is cut into so its generations can
/// stagger: each band advances at its own offset within the tick, so the
/// whole field never changes in one simultaneous pulse.
const LIFE_BANDS: usize = 6;
/// The swell travelling through the surface: how high it lifts, how long a
/// wave is, and how long it takes to pass.
const WAVE_HEIGHT: f32 = 7.0;
const WAVE_LENGTH: f32 = 280.0;
const WAVE_PERIOD_SECS: f32 = 30.0;
/// Direction the swell travels across the plane, normalized by hand so it
/// can be a constant.
const WAVE_DIR: Vec2 = Vec2::new(0.330, 0.944);
/// About how many triangles the eye should find on screen at once.
const TARGET_VISIBLE: usize = 150;
/// Where the backdrop sits in the 2D stack: under everything the game or
/// the lobby will ever draw.
const LAYER: f32 = -10.0;
/// Cake's page background, the cool end of the gradient (`#0f1014`, eased a
/// touch darker so the field's depths read). The lobby's scrim shares it.
pub(crate) const DEEP: Srgba = Srgba::rgb(0.050, 0.055, 0.072);
/// The nebula's violet from cake's sky, the warm end.
const VIOLET: Srgba = Srgba::rgb(0.500, 0.400, 0.800);
/// The app's clear colour: distant triangles melt into it.
const CLEAR: Srgba = Srgba::rgb(0.09, 0.09, 0.11);
/// The nebula's wavelength across the plane: how many plane units one full
/// swing of the colour gradient takes. Small, so the tint varies several
/// times within a single screen instead of washing one uniform hue.
const NEBULA_WAVELENGTH: f32 = 420.0;

const SQRT3_2: f32 = 0.866_025_4;
/// The viewport assumed when there is no orthographic camera to ask.
const FALLBACK_VIEW_HALF: Vec2 = Vec2::new(450.0, 300.0);

/// The world-space half extent the camera shows right now.
fn view_half(projection: &Projection) -> Vec2 {
    match projection {
        Projection::Orthographic(o) => o.area.half_size(),
        _ => FALLBACK_VIEW_HALF,
    }
}

// ---- the camera ------------------------------------------------------------

/// A tilted pinhole over the plane, projected on the CPU into the same 2D
/// world space the lobby renders in.
///
/// The camera hangs at [`HEIGHT`] over the ground, leaning [`TILT_DEG`] from
/// straight down toward the plane's north. The focal length is chosen so one
/// plane unit at the focus point — the ground the screen centre looks at,
/// `HEIGHT * tan(tilt)` north of the camera — draws as one world unit: the
/// field's on-screen grain matches the lobby's, and perspective does the
/// rest. Closer ground draws larger, distant ground falls away.
struct Rig {
    sin_tilt: f32,
    cos_tilt: f32,
    focal: f32,
    focus_d: f32,
    /// Where the camera hangs over the plane (it pans), and where its lean
    /// points (it sways).
    pos: Vec2,
    yaw: f32,
}

impl Rig {
    fn new() -> Self {
        let (sin_tilt, cos_tilt) = TILT_DEG.to_radians().sin_cos();
        Self {
            sin_tilt,
            cos_tilt,
            focal: HEIGHT / (cos_tilt * cos_tilt),
            focus_d: HEIGHT * sin_tilt / cos_tilt,
            pos: Vec2::ZERO,
            yaw: 0.0,
        }
    }

    /// Project a ground point: where it lands on screen, and the local
    /// scale (one plane unit draws as this many world units here).
    fn project(&self, p: Vec2) -> (Vec2, f32) {
        let rel = rotate(p - self.pos, -self.yaw);
        let depth = rel.y * self.sin_tilt + HEIGHT * self.cos_tilt;
        let screen = Vec2::new(
            self.focal * rel.x / depth,
            self.focal * (rel.y * self.cos_tilt - HEIGHT * self.sin_tilt) / depth,
        );
        (screen, self.focal * HEIGHT / (depth * depth))
    }

    /// Where a screen point's sight-line meets the ground. A sight-line at
    /// or above the horizon never does, so those are bent down to a line
    /// well under it: a freakishly tall window then sizes a huge field, not
    /// a nonsensical one.
    fn unproject(&self, at: Vec2) -> Vec2 {
        let horizon = self.focal * self.cos_tilt / self.sin_tilt;
        let y = at.y.min(0.8 * horizon);
        let depth = HEIGHT / (self.cos_tilt - y * self.sin_tilt / self.focal);
        let rel = Vec2::new(
            at.x * depth / self.focal,
            (y * depth / self.focal + HEIGHT * self.sin_tilt) / self.cos_tilt,
        );
        self.pos + rotate(rel, self.yaw)
    }
}

/// Rotate `v` by `a` radians.
fn rotate(v: Vec2, a: f32) -> Vec2 {
    let (s, c) = a.sin_cos();
    Vec2::new(v.x * c - v.y * s, v.x * s + v.y * c)
}

/// Spawn the field, sized to whatever the camera currently frames.
fn spawn(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    room: Res<RoomId>,
    cameras: Query<&Projection, With<Camera>>,
) {
    let half = cameras
        .single()
        .map(view_half)
        .unwrap_or(FALLBACK_VIEW_HALF);
    let (rig, grid, mesh) = build(half, seed_of(&room.0));
    let mesh = meshes.add(mesh);
    let material = materials.add(ColorMaterial::from_color(Color::WHITE));
    let positions = vec![[0.0; 3]; grid.sheet.corners.len()];
    commands.spawn((
        Mesh2d(mesh.clone()),
        MeshMaterial2d(material),
        Transform::from_xyz(0.0, 0.0, LAYER),
        Pickable::IGNORE,
        Backdrop {
            mesh,
            rig,
            grid,
            positions,
            clocks: band_clocks(LIFE_BANDS, TICK_SECS),
            upkeep: 0.0,
            built_for: half,
        },
    ));
}

/// The backdrop's mesh goes with it: the next lobby entry builds a fresh one
/// against whatever the window has become.
fn teardown(mut meshes: ResMut<Assets<Mesh>>, backdrops: Query<&Backdrop>) {
    for backdrop in &backdrops {
        meshes.remove(&backdrop.mesh);
    }
}

/// Pan the camera and sway its lean, then re-project every vertex through
/// the rig, with the swell riding on top. The positions are rewritten in
/// place, so the draw follows on the same frame.
fn loom(time: Res<Time>, mut backdrops: Query<&mut Backdrop>, mut meshes: ResMut<Assets<Mesh>>) {
    let Ok(mut backdrop) = backdrops.single_mut() else {
        return;
    };
    let t = time.elapsed_secs();
    let Backdrop {
        mesh,
        rig,
        grid,
        positions,
        ..
    } = &mut *backdrop;

    rig.pos = Vec2::new(
        0.6 * (t * 0.025).sin() + 0.4 * (t * 0.011).sin(),
        0.6 * (t * 0.019 + 1.7).sin() + 0.4 * (t * 0.008).cos(),
    ) * CAM_PAN;
    rig.yaw = YAW_SWAY * (t * std::f32::consts::TAU / YAW_SWAY_PERIOD_SECS).sin();

    let phase = t * std::f32::consts::TAU / WAVE_PERIOD_SECS;
    for (i, &li) in grid.sheet.corners.iter().enumerate() {
        let p = grid.sheet.lattice[li as usize];
        let (screen, scale) = rig.project(p);
        let swell =
            WAVE_HEIGHT * (std::f32::consts::TAU * p.dot(WAVE_DIR) / WAVE_LENGTH - phase).sin();
        positions[i] = [screen.x, screen.y + swell * scale, 0.0];
    }

    if let Some(mut mesh) = meshes.get_mut(mesh.id()) {
        match mesh.try_attribute_mut(Mesh::ATTRIBUTE_POSITION) {
            Ok(VertexAttributeValues::Float32x3(slot)) if slot.len() == positions.len() => {
                slot.clone_from(positions);
            }
            _ => {
                mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions.clone());
            }
        }
    }
}

/// Advance the life and repaint the cell colours to match. The generations
/// are staggered: the field is cut into [`LIFE_BANDS`] horizontal bands, each
/// advancing at its own offset within the tick, so the life ripples across the
/// field instead of pulsing as one. The hops are not saved up for the tick
/// either — a slice flies every frame — and the upkeep (flashes, reseed) runs
/// on its own clock. The mesh is rewritten in place, so the draw follows on
/// the same frame.
fn tick_life(
    time: Res<Time>,
    mut backdrops: Query<&mut Backdrop>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let Ok(mut backdrop) = backdrops.single_mut() else {
        return;
    };
    let dt = time.delta_secs();
    // This frame's share of the tick's hop attempts: frame-rate independent
    // in expectation, and sparse enough that the hops trickle in.
    let chance = (HOP_CHANCE * dt / TICK_SECS).min(1.0);
    let mut dirty = backdrop.grid.hops(chance);
    for band in 0..LIFE_BANDS {
        backdrop.clocks[band] += dt;
        if backdrop.clocks[band] >= TICK_SECS {
            backdrop.clocks[band] -= TICK_SECS;
            backdrop.grid.generation_band(band, LIFE_BANDS);
            dirty = true;
        }
    }
    backdrop.upkeep += dt;
    if backdrop.upkeep >= TICK_SECS {
        backdrop.upkeep -= TICK_SECS;
        backdrop.grid.upkeep();
        dirty = true;
    }
    if !dirty {
        return;
    }
    if let Some(mut mesh) = meshes.get_mut(&backdrop.mesh) {
        paint(&backdrop.grid, &mut mesh);
    }
}

/// The per-band generation clocks, phase-spread across one tick: the last
/// band advances first, band 0 a full tick in, and every band thereafter once
/// a tick — so no two bands tick together and the field never pulses as one.
fn band_clocks(bands: usize, tick: f32) -> Vec<f32> {
    (0..bands)
        .map(|band| band as f32 / bands as f32 * tick)
        .collect()
}

/// Rebuild the field if the window has grown past the ground it was built
/// to cover — a maximise mid-lobby must not bare the sheet's edge.
/// Shrinking is left alone: extra field off-screen costs nothing.
fn ensure_coverage(
    mut resized: MessageReader<WindowResized>,
    cameras: Query<&Projection, With<Camera>>,
    mut backdrops: Query<&mut Backdrop>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    if resized.read().next().is_none() {
        return;
    }
    let Ok(half) = cameras.single().map(view_half) else {
        return;
    };
    let Ok(mut backdrop) = backdrops.single_mut() else {
        return;
    };
    if half.x <= backdrop.built_for.x && half.y <= backdrop.built_for.y {
        return;
    }
    let (rig, grid, mesh) = build_with_side(half, backdrop.grid.side, backdrop.grid.seed());
    backdrop.rig = rig;
    backdrop.grid = grid;
    backdrop.built_for = half;
    backdrop.positions = vec![[0.0; 3]; backdrop.grid.sheet.corners.len()];
    if let Some(mut existing) = meshes.get_mut(&backdrop.mesh) {
        *existing = mesh;
    }
}

// ---- the grid --------------------------------------------------------------

/// One cell of the field: an up-pointing or down-pointing triangle.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Orientation {
    Up,
    Down,
}

/// The six directions of the same-orientation sublattice — the hex
/// directions a checkers hop flies along, in `(du, dv)` index steps.
const HOP_DIRECTIONS: [(i32, i32); 6] = [(1, 0), (-1, 0), (0, 1), (0, -1), (1, -1), (-1, 1)];

/// The triangular field and the life on it.
///
/// Cells are indexed `k = orientation * w * h + v * w + u`. The drawn mesh
/// gives every cell its own three vertices, at the triangle's own corners —
/// nothing is shared between neighbours — so each triangle can carry one flat
/// colour, the cell's.
pub(crate) struct LifeGrid {
    w: usize,
    h: usize,
    /// Triangle side, in plane units.
    side: f32,
    cells: Vec<u8>,
    /// Ticks of highlight left, set on a hop's landing cell.
    flash: Vec<u8>,
    /// Scratch for the generation step.
    next: Vec<u8>,
    /// The twelve neighbours of every cell, precomputed and wrapped.
    nbrs: Vec<[u32; 12]>,
    /// The sheet's static painting geometry.
    sheet: Sheet,
    rng: Rng,
    seed: u64,
}

/// The drawn sheet's static geometry: the lattice points the triangles meet
/// at, where each sits on the gradient and how far it has melted toward the
/// clear colour, and, for every mesh vertex, which lattice point it stands on.
struct Sheet {
    lattice: Vec<Vec2>,
    grad: Vec<f32>,
    fade: Vec<f32>,
    /// Three per cell, six per mesh triangle pair: the lattice vertex under
    /// each mesh vertex, in mesh order.
    corners: Vec<u32>,
}

impl LifeGrid {
    /// A field of `w * h` cells anchored at `origin` in plane space, its
    /// far distances melting out over `fade`.
    pub(crate) fn new(
        w: usize,
        h: usize,
        side: f32,
        origin: Vec2,
        fade_start: f32,
        fade_end: f32,
        seed: u64,
    ) -> Self {
        let n = 2 * w * h;
        let mut grid = LifeGrid {
            w,
            h,
            side,
            cells: vec![0; n],
            flash: vec![0; n],
            next: vec![0; n],
            nbrs: neighbor_table(w, h),
            sheet: sheet(w, h, side, origin, fade_start, fade_end),
            rng: Rng(seed.max(1)),
            seed,
        };
        grid.reseed(0.30);
        grid
    }

    /// The cell index of orientation `o` at `(u, v)`, wrapped on the torus.
    pub(crate) fn index(&self, u: i32, v: i32, o: Orientation) -> usize {
        let u = u.rem_euclid(self.w as i32) as usize;
        let v = v.rem_euclid(self.h as i32) as usize;
        let base = match o {
            Orientation::Up => 0,
            Orientation::Down => self.w * self.h,
        };
        base + v * self.w + u
    }

    pub(crate) fn population(&self) -> u32 {
        self.cells.iter().map(|&c| u32::from(c)).sum()
    }

    /// Fresh soup. Called at birth, and by `upkeep` whenever the population
    /// escapes its band: burned out, or grown to a churning near-solid.
    pub(crate) fn reseed(&mut self, p: f32) {
        for (cell, flash) in self.cells.iter_mut().zip(self.flash.iter_mut()) {
            *cell = u8::from(self.rng.chance(p));
            *flash = 0;
        }
    }

    /// The B3/S23 generation over the 12-neighbourhood, restricted to the
    /// cells of band `band` of `bands` horizontal strips. Each band is
    /// advanced at its own offset in time, so the field's life staggers
    /// instead of pulsing as one. The rule still reads the previous state
    /// everywhere, so a band never sees its own births mid-pass.
    pub(crate) fn generation_band(&mut self, band: usize, bands: usize) {
        let rows = self.h;
        let v0 = rows * band / bands;
        let v1 = rows * (band + 1) / bands;
        for v in v0..v1 {
            for u in 0..self.w {
                for o in [Orientation::Up, Orientation::Down] {
                    let i = self.index(u as i32, v as i32, o);
                    let mut count = 0;
                    for &n in &self.nbrs[i] {
                        count += self.cells[n as usize];
                    }
                    // Conway's: three neighbours birth, two or three keep alive.
                    let lives = matches!((self.cells[i], count), (1, 2 | 3) | (0, 3));
                    self.next[i] = u8::from(lives);
                }
            }
        }
        for v in v0..v1 {
            for u in 0..self.w {
                for o in [Orientation::Up, Orientation::Down] {
                    let i = self.index(u as i32, v as i32, o);
                    self.cells[i] = self.next[i];
                }
            }
        }
    }

    /// The checkers moves: each live cell may leap over an occupied
    /// same-orientation neighbour into the empty cell exactly beyond. The
    /// mover's cell empties; the jumped cell stays — a hop is a hop, not a
    /// capture. Called with a small `chance` every frame, the hops stagger in
    /// across the tick instead of all flying at once. True if any piece moved.
    pub(crate) fn hops(&mut self, chance: f32) -> bool {
        let (w, h) = (self.w as i32, self.h as i32);
        let mut moved = false;
        for u in 0..w {
            for v in 0..h {
                for o in [Orientation::Up, Orientation::Down] {
                    if self.cells[self.index(u, v, o)] == 0 || !self.rng.chance(chance) {
                        continue;
                    }
                    let dir = HOP_DIRECTIONS[self.rng.below(HOP_DIRECTIONS.len())];
                    moved |= self.try_hop(u, v, o, dir);
                }
            }
        }
        moved
    }

    /// One hop attempt from `(u, v, o)` along `dir`: it flies when the
    /// neighbour beyond is occupied and the cell beyond that is empty.
    /// True when the cell moved.
    pub(crate) fn try_hop(&mut self, u: i32, v: i32, o: Orientation, (du, dv): (i32, i32)) -> bool {
        let from = self.index(u, v, o);
        if self.cells[from] == 0 {
            return false;
        }
        let over = self.index(u + du, v + dv, o);
        let landing = self.index(u + 2 * du, v + 2 * dv, o);
        if self.cells[over] == 0 || self.cells[landing] == 1 {
            return false;
        }
        self.cells[from] = 0;
        self.cells[landing] = 1;
        self.flash[landing] = FLASH_TICKS;
        true
    }

    /// The flashes decay; a population burnt out or grown past its band is
    /// reseeded.
    fn upkeep(&mut self) {
        for flash in self.flash.iter_mut() {
            *flash = flash.saturating_sub(1);
        }
        let n = self.cells.len() as u32;
        let pop = self.population();
        if pop < n / 20 {
            self.reseed(0.34);
        } else if pop > n * 9 / 10 {
            self.reseed(0.22);
        }
    }

    /// The seed the grid was grown from — so a rebuilt field is the same
    /// field, not a new one.
    pub(crate) fn seed(&self) -> u64 {
        self.seed
    }

    /// The life on cell `k`: one for a live cell, a little extra while its
    /// landing flash lasts. Flat per-cell shading: the neighbours have no say.
    fn cell_lit(&self, k: usize) -> f32 {
        (f32::from(self.cells[k]) + 0.35 * f32::from(self.flash[k])).min(1.0)
    }
}

/// The colour of a cell at gradient `grad` (0..1 across the nebula) with life
/// `lit` (0..1), melted toward the clear colour by `fade`. One flat colour per
/// triangle; the far field dissolves into the clear.
fn shade(lit: f32, grad: f32, fade: f32) -> [f32; 4] {
    let still = DEEP
        .mix(&VIOLET, 0.12 + 0.32 * grad)
        .mix(&Srgba::BLACK, 0.28);
    let alive = DEEP
        .mix(&VIOLET, 0.55 + 0.40 * grad)
        .mix(&Srgba::WHITE, 0.08 + 0.30 * lit);
    let bloom = smoothstep(0.15, 0.5, lit);
    let c = LinearRgba::from(still.mix(&alive, bloom));
    let clear = LinearRgba::from(CLEAR);
    [
        c.red + (clear.red - c.red) * fade,
        c.green + (clear.green - c.green) * fade,
        c.blue + (clear.blue - c.blue) * fade,
        1.0,
    ]
}

/// `smoothstep(a, b, x)`, the tasteful climb from zero to one.
fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Where a plane point sits on the nebula gradient, as a mix fraction from
/// the deep near-black toward the violet. A small-scale drift summed from a
/// few non-parallel sines, so it reads as soft blotches of tint rather than
/// stripes, and the colour varies several times across a screen.
fn nebula(p: Vec2) -> f32 {
    let s = std::f32::consts::TAU / NEBULA_WAVELENGTH;
    let n = (p.x * s).sin() + (p.y * s * 1.3 + 0.7).sin() + ((p.x - p.y) * s * 0.6 - 0.9).sin();
    (0.5 + n / 6.0).clamp(0.0, 1.0)
}

/// The lattice vertex of `(u, v)` in a `w`-wide sheet.
fn lattice_index(w: usize, u: usize, v: usize) -> usize {
    v * (w + 1) + u
}

/// The sheet's geometry over a field of `w * h` cells: the lattice points
/// the triangles meet at, their gradient and fade fractions, and the lattice
/// vertex under each mesh vertex. Everything static about the painting lives
/// here.
fn sheet(w: usize, h: usize, side: f32, origin: Vec2, fade_start: f32, fade_end: f32) -> Sheet {
    let a = Vec2::new(side, 0.0);
    let b = Vec2::new(0.5 * side, side * SQRT3_2);
    let at = |u: f32, v: f32| origin + u * a + v * b;

    let mut lattice = Vec::with_capacity((w + 1) * (h + 1));
    let mut grad = Vec::with_capacity(lattice.capacity());
    let mut fade = Vec::with_capacity(lattice.capacity());
    for v in 0..=h {
        for u in 0..=w {
            let p = at(u as f32, v as f32);
            lattice.push(p);
            grad.push(nebula(p));
            fade.push(smoothstep(fade_start, fade_end, p.y));
        }
    }
    let corners = mesh_corners(w, h);
    Sheet {
        lattice,
        grad,
        fade,
        corners,
    }
}

/// The lattice vertex under every mesh vertex, in the order [`field_mesh`]
/// emits them: for each cell in index order, its three corners. Up cells
/// (indices `0..w*h`) use their low triangle, down cells their high one.
fn mesh_corners(w: usize, h: usize) -> Vec<u32> {
    let mut corners = Vec::with_capacity(6 * w * h);
    for o in [Orientation::Up, Orientation::Down] {
        for v in 0..h {
            for u in 0..w {
                let v00 = lattice_index(w, u, v) as u32;
                let v10 = lattice_index(w, u + 1, v) as u32;
                let v01 = lattice_index(w, u, v + 1) as u32;
                let v11 = lattice_index(w, u + 1, v + 1) as u32;
                corners.extend(match o {
                    Orientation::Up => [v00, v10, v01],
                    Orientation::Down => [v10, v11, v01],
                });
            }
        }
    }
    corners
}

/// One cell's twelve neighbours as `(du, dv, orientation)` offsets, edge
/// neighbours first. Derived once, here, from the geometry:
///
/// `U(u,v)` — vertices `(u,v)`, `(u+1,v)`, `(u,v+1)` — shares each edge with
/// a down cell: `D(u,v-1)` across the base, `D(u,v)` across the diagonal,
/// `D(u-1,v)` across the upright. Each of its three vertices then carries
/// three more triangles that touch only the corner; six of the nine are up
/// cells, and they are exactly the six hex directions the hops fly along.
///
/// `D(u,v)` — vertices `(u+1,v)`, `(u+1,v+1)`, `(u,v+1)` — is the same read
/// off its own corners: edges out to `U(u+1,v)`, `U(u,v+1)`, `U(u,v)`.
const fn neighbour_offsets(o: Orientation) -> [((i32, i32), Orientation); 12] {
    use Orientation::{Down, Up};
    match o {
        Orientation::Up => [
            ((0, -1), Down),
            ((0, 0), Down),
            ((-1, 0), Down),
            ((-1, 0), Up),
            ((-1, -1), Down),
            ((0, -1), Up),
            ((1, -1), Up),
            ((1, -1), Down),
            ((1, 0), Up),
            ((-1, 1), Up),
            ((-1, 1), Down),
            ((0, 1), Up),
        ],
        Orientation::Down => [
            ((1, 0), Up),
            ((0, 1), Up),
            ((0, 0), Up),
            ((1, -1), Up),
            ((1, -1), Down),
            ((0, -1), Down),
            ((1, 0), Down),
            ((1, 1), Up),
            ((0, 1), Down),
            ((-1, 0), Down),
            ((-1, 1), Up),
            ((-1, 1), Down),
        ],
    }
}

/// The twelve neighbours of every cell, precomputed and wrapped: three
/// across edges, nine at corners.
pub(crate) fn neighbor_table(w: usize, h: usize) -> Vec<[u32; 12]> {
    let mut table = Vec::with_capacity(2 * w * h);
    for o in [Orientation::Up, Orientation::Down] {
        for v in 0..h as i32 {
            for u in 0..w as i32 {
                let nbrs = neighbour_offsets(o)
                    .map(|((du, dv), orient)| index(u + du, v + dv, orient, w, h) as u32);
                table.push(nbrs);
            }
        }
    }
    table
}

fn index(u: i32, v: i32, o: Orientation, w: usize, h: usize) -> usize {
    let u = u.rem_euclid(w as i32) as usize;
    let v = v.rem_euclid(h as i32) as usize;
    let base = match o {
        Orientation::Up => 0,
        Orientation::Down => w * h,
    };
    base + v * w + u
}

/// Build a field covering everything the camera can see of a viewport this
/// many world units in half-extent, plus its whole roam. Returns the rig,
/// the life, and the mesh (already painted).
fn build(view_half: Vec2, seed: u64) -> (Rig, LifeGrid, Mesh) {
    let side = default_side(view_half);
    build_with_side(view_half, side, seed)
}

/// As [`build`], but at a given triangle size — the resize path keeps the
/// field's grain and grows its reach instead.
fn build_with_side(view_half: Vec2, side: f32, seed: u64) -> (Rig, LifeGrid, Mesh) {
    let rig = Rig::new();
    // The ground rect the viewport can show, with the camera at home.
    let (mut min, mut max) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
    for corner in [Vec2::NEG_ONE, Vec2::X, Vec2::Y, Vec2::ONE] {
        let g = rig.unproject(corner * view_half);
        min = min.min(g);
        max = max.max(g);
    }
    // The camera roams over its pan and sways its lean: every rect corner
    // shoved around the pan, every corner swung both ways about the origin,
    // and the bbox of all of it is what the field must cover.
    let mut corners = Vec::new();
    for x in [min.x, max.x] {
        for y in [min.y, max.y] {
            let c = Vec2::new(x, y);
            corners.push(c);
            corners.push(rotate(c, YAW_SWAY));
            corners.push(rotate(c, -YAW_SWAY));
        }
    }
    min = Vec2::splat(f32::MAX);
    max = Vec2::splat(f32::MIN);
    for c in corners {
        min = min.min(c);
        max = max.max(c);
    }
    let margin = Vec2::splat(CAM_PAN + WAVE_HEIGHT + side);
    min -= margin;
    max += margin;

    let centre = (min + max) / 2.0;
    let span = max - min;

    // The sheet is a parallelogram of cells; its slanted sides mean the
    // largest axis-aligned rectangle inside it is narrower than its bounding
    // box by the rectangle's own height over √3. Size for that, or a slanted
    // edge of missing sheet shows up mid-view.
    let h = (span.y / (side * SQRT3_2)).ceil() as usize;
    let w = (((span.x + span.y / SQRT3_2) / side).ceil() as usize).max(1);
    // Anchor the parallelogram so the visible rect hugs its wide part: the
    // slanted left side starts a full inset west of the rect's left edge.
    let origin = Vec2::new(
        centre.x + span.x / 2.0 - w as f32 * side,
        centre.y - span.y / 2.0,
    );

    // The melt lives past the ground the screen's top edge can reach at
    // home, and finishes a view-height further out: a whisper of dissolve
    // at the top of the frame, never a bare edge.
    let fade_start = rig.focus_d + view_half.y;
    let fade_end = fade_start + view_half.y * 1.2;
    let grid = LifeGrid::new(w, h, side, origin, fade_start, fade_end, seed);
    let mut mesh = field_mesh(w, h);
    paint(&grid, &mut mesh);
    (rig, grid, mesh)
}

/// The triangle size that puts about [`TARGET_VISIBLE`] triangles on a
/// viewport of this half extent.
fn default_side(view_half: Vec2) -> f32 {
    // A typical viewport area from its half extent, at a plump aspect.
    let area = 4.0 * view_half.x * view_half.y * 0.62;
    let per_triangle = SQRT3_2 / 2.0;
    (area / (TARGET_VISIBLE as f32 * per_triangle))
        .sqrt()
        .clamp(48.0, 110.0)
}

/// The sheet as a flat-shaded triangle list: each cell owns its three
/// vertices, at the triangle's own corners, so the cell can carry one flat
/// colour. Nothing is shared, so the triangles are crisp.
fn field_mesh(w: usize, h: usize) -> Mesh {
    let vertices = 6 * w * h;
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0; 3]; vertices]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0; 2]; vertices]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.0; 4]; vertices]);
    mesh
}

/// Write the sheet's colours: one flat colour per cell, from its own life and
/// the nebula at its corners, the far distances melted into the clear colour.
fn paint(grid: &LifeGrid, mesh: &mut Mesh) {
    let cells = 2 * grid.w * grid.h;
    let mut colours = Vec::with_capacity(grid.sheet.corners.len());
    for k in 0..cells {
        let lit = grid.cell_lit(k);
        for c in 0..3 {
            let vi = k * 3 + c;
            let li = grid.sheet.corners[vi] as usize;
            colours.push(shade(lit, grid.sheet.grad[li], grid.sheet.fade[li]));
        }
    }
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colours);
}

/// A room's seed: the same room, the same sky.
fn seed_of(room: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    room.hash(&mut hasher);
    hasher.finish()
}

/// A tiny xorshift, after the one cake's nebula uses. Deterministic, `rand`-free.
pub(crate) struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// True with probability `p`.
    pub(crate) fn chance(&mut self, p: f32) -> bool {
        ((self.next_u64() >> 11) as f32 / (1u64 << 53) as f32) < p
    }

    /// A uniform index below `n`.
    pub(crate) fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::mesh::VertexAttributeValues;

    /// A small grid away from any fade, for the life tests.
    fn test_grid(w: usize, h: usize, seed: u64) -> LifeGrid {
        LifeGrid::new(w, h, 10.0, Vec2::ZERO, 1.0e30, 2.0e30, seed)
    }

    /// One generation the way the lobby runs it: the rule, a pass of hops,
    /// then the upkeep.
    fn step(grid: &mut LifeGrid) {
        grid.generation_band(0, 1);
        grid.hops(HOP_CHANCE);
        grid.upkeep();
    }

    /// Every cell's twelve neighbours: all distinct, all in range.
    #[test]
    fn every_cell_has_twelve_distinct_wrapped_neighbours() {
        let (w, h) = (5, 4);
        let table = neighbor_table(w, h);
        assert_eq!(table.len(), 2 * w * h);
        for nbrs in &table {
            for i in 0..12 {
                assert!(nbrs[i] < (2 * w * h) as u32);
                for j in i + 1..12 {
                    assert_ne!(nbrs[i], nbrs[j], "neighbour repeated in {nbrs:?}");
                }
            }
        }
    }

    /// An up cell's edge neighbours are exactly the three down cells its
    /// derivation names — here at `(0,0)`, where the wrap is exercised.
    #[test]
    fn an_up_cells_edge_neighbours_are_the_adjacent_downs() {
        let (w, h) = (4, 5);
        let table = neighbor_table(w, h);
        let idx = |u: i32, v: i32, o: Orientation| {
            index(u.rem_euclid(w as i32), v.rem_euclid(h as i32), o, w, h)
        };
        let up00 = idx(0, 0, Orientation::Up);
        let expected = [
            idx(0, -1, Orientation::Down),
            idx(0, 0, Orientation::Down),
            idx(-1, 0, Orientation::Down),
        ];
        for e in expected {
            assert!(table[up00].contains(&(e as u32)), "edge neighbour {e}");
        }
    }

    /// The corner ring of an up cell at `(0,0)`: the six same-orientation
    /// hex directions (the hop lanes) must all be present.
    #[test]
    fn an_up_cells_corner_ring_holds_the_six_hop_directions() {
        let (w, h) = (4, 5);
        let table = neighbor_table(w, h);
        let idx = |u: i32, v: i32, o: Orientation| {
            index(u.rem_euclid(w as i32), v.rem_euclid(h as i32), o, w, h)
        };
        let up00 = idx(0, 0, Orientation::Up);
        for (du, dv) in HOP_DIRECTIONS {
            let hop = idx(du, dv, Orientation::Up);
            assert!(
                table[up00].contains(&(hop as u32)),
                "hop lane ({du},{dv}) missing"
            );
        }
    }

    /// Conway's own: three neighbours birth, two keep alive, solitude dies.
    #[test]
    fn the_generation_rule_is_conways() {
        let mut grid = test_grid(6, 6, 7);
        grid.reseed(0.0);
        // A birth: three neighbours around an empty cell.
        let born = grid.index(3, 3, Orientation::Up);
        for (du, dv) in [(1, 0), (0, 1), (1, -1)] {
            let i = grid.index(3 + du, 3 + dv, Orientation::Up);
            grid.cells[i] = 1;
        }
        assert_eq!(grid.cells[born], 0);
        // A survivor: alive, with two neighbours.
        let kept = grid.index(0, 0, Orientation::Down);
        grid.cells[kept] = 1;
        for (du, dv) in [(1, 0), (0, 1)] {
            let i = grid.index(du, dv, Orientation::Down);
            grid.cells[i] = 1;
        }
        // A lonely death.
        let lone = grid.index(5, 5, Orientation::Up);
        grid.cells[lone] = 1;
        // No upkeep: the population band must not interfere.
        grid.generation_band(0, 1);
        assert_eq!(grid.cells[born], 1, "three neighbours birth");
        assert_eq!(grid.cells[kept], 1, "two neighbours survive");
        assert_eq!(grid.cells[lone], 0, "a lone cell dies");
    }

    /// The stagger: a generation runs only over its own band's rows, so a
    /// birth in another band's rows is left for that band's turn.
    #[test]
    fn a_generation_band_only_advances_its_own_rows() {
        let (w, h, bands) = (8, 4, 2);
        let mut grid = test_grid(w, h, 17);
        grid.reseed(0.0);
        // A birth in band 0's rows (v=0) and one in band 1's (v=3).
        let low = grid.index(3, 0, Orientation::Up);
        let high = grid.index(3, 3, Orientation::Up);
        for target in [low, high] {
            let (u, v) = (target % w, target / w);
            for (du, dv) in [(1, 0), (0, 1), (1, -1)] {
                let i = grid.index(u as i32 + du, v as i32 + dv, Orientation::Up);
                grid.cells[i] = 1;
            }
        }
        grid.generation_band(0, bands);
        assert_eq!(grid.cells[low], 1, "band 0's birth happens");
        assert_eq!(grid.cells[high], 0, "band 1's birth waits its turn");
        grid.generation_band(1, bands);
        assert_eq!(grid.cells[high], 1, "band 1's birth happens on its turn");
    }

    /// With a lane open, a hop leaps over the occupied neighbour into the
    /// empty cell exactly beyond, which stays alive — a hop is not a capture.
    #[test]
    fn a_hop_leaps_over_an_occupied_neighbour_into_an_empty_landing() {
        let mut grid = test_grid(8, 8, 11);
        grid.reseed(0.0);
        let (from, over, landing) = (
            grid.index(3, 3, Orientation::Up),
            grid.index(4, 3, Orientation::Up),
            grid.index(5, 3, Orientation::Up),
        );
        grid.cells[from] = 1;
        grid.cells[over] = 1;
        assert!(grid.try_hop(3, 3, Orientation::Up, (1, 0)));
        assert_eq!(grid.cells[from], 0, "the mover leaves");
        assert_eq!(grid.cells[over], 1, "a hop never captures");
        assert_eq!(grid.cells[landing], 1, "the hop lands two lanes out");
        assert_eq!(grid.flash[landing], FLASH_TICKS, "the landing lights up");
    }

    /// A hop with nowhere to land stays home: an empty neighbour under it,
    /// or an occupied landing, and the cell does not move.
    #[test]
    fn a_hop_needs_an_occupied_neighbour_and_an_empty_landing() {
        let mut grid = test_grid(8, 8, 13);
        grid.reseed(0.0);
        let (from, empty, landing) = (
            grid.index(3, 3, Orientation::Up),
            grid.index(4, 3, Orientation::Up),
            grid.index(5, 3, Orientation::Up),
        );
        grid.cells[from] = 1;
        assert!(!grid.try_hop(3, 3, Orientation::Up, (1, 0)));
        assert_eq!(grid.cells[from], 1, "nothing to hop over, no hop");

        grid.cells[empty] = 1;
        grid.cells[landing] = 1;
        assert!(!grid.try_hop(3, 3, Orientation::Up, (1, 0)));
        assert_eq!(grid.cells[from], 1, "no room to land, no hop");
    }

    /// A landing's flash decays away over a few ticks.
    #[test]
    fn a_landing_flashes_and_fades() {
        let mut grid = test_grid(6, 6, 17);
        grid.reseed(0.0);
        let (a, b, c) = (
            grid.index(2, 2, Orientation::Up),
            grid.index(3, 2, Orientation::Up),
            grid.index(4, 2, Orientation::Up),
        );
        grid.cells[a] = 1;
        grid.cells[b] = 1;
        assert!(grid.try_hop(2, 2, Orientation::Up, (1, 0)));
        assert_eq!(grid.cells[c], 1);
        assert!(grid.flash[c] > 0, "a landing flashes");
        for _ in 0..FLASH_TICKS {
            grid.upkeep();
        }
        assert_eq!(grid.flash[c], 0, "the flash fades");
    }

    /// Long-run stability: the population stays inside its band forever —
    /// reseeding rescues a burnt-out field and prunes a churning solid one.
    #[test]
    fn the_population_stays_in_its_band() {
        let mut grid = test_grid(14, 10, 23);
        let n = grid.cells.len() as u32;
        for _ in 0..400 {
            step(&mut grid);
            let pop = grid.population();
            assert!(pop >= n / 20, "burned out: {pop}/{n}");
            assert!(pop <= n * 9 / 10, "saturated: {pop}/{n}");
            assert!(pop > 0);
        }
    }

    /// A pass of hops never changes the population: every hop is a move. The
    /// lobby slices these passes thin across the tick, but each slice is still
    /// only a move.
    #[test]
    fn hops_conserve_the_population() {
        let mut grid = test_grid(16, 12, 31);
        for _ in 0..10 {
            let before = grid.population();
            // One tick's hops, sliced the way the lobby staggers them.
            for _ in 0..90 {
                grid.hops(HOP_CHANCE / 90.0);
            }
            assert_eq!(grid.population(), before, "a hop is a move, not a birth");
            step(&mut grid);
        }
    }

    /// `hops` reports whether any piece moved — the lobby repaints only when
    /// a hop lands, and an empty field moves nothing.
    #[test]
    fn hops_report_whether_anything_moved() {
        let mut empty = test_grid(16, 12, 31);
        empty.reseed(0.0);
        assert!(!empty.hops(1.0), "an empty field hops nowhere");
    }

    /// The camera looks down from far away: the ground at the focus lands
    /// dead centre at unit scale, and equal steps either side of the focus
    /// are compressed the further they are.
    #[test]
    fn the_rig_looms_rather_than_scans() {
        let rig = Rig::new();
        let (at, scale) = rig.project(rig.pos + Vec2::new(0.0, rig.focus_d));
        assert!(at.distance(Vec2::ZERO) < 0.01, "focus off centre: {at}");
        assert!((scale - 1.0).abs() < 0.01, "focus scale {scale}");

        let near = rig
            .project(rig.pos + Vec2::new(0.0, rig.focus_d - 100.0))
            .0
            .y;
        let far = rig
            .project(rig.pos + Vec2::new(0.0, rig.focus_d + 100.0))
            .0
            .y;
        assert!(near < 0.0 && far > 0.0, "near {near}, far {far}");
        assert!(
            far.abs() < near.abs(),
            "the far step must travel less on screen: {far} vs {near}"
        );
    }

    /// A sight-line at or above the horizon is bent down to the ground, not
    /// sent off to infinity or behind the camera.
    #[test]
    fn unprojecting_above_the_horizon_stays_on_the_ground() {
        let rig = Rig::new();
        let under = rig.unproject(Vec2::new(0.0, 500.0));
        for sky in [2_000.0, 1.0e6, f32::MAX] {
            let g = rig.unproject(Vec2::new(0.0, sky));
            assert!(g.is_finite(), "{sky} unprojects to {g}");
            assert!(g.y > under.y, "{sky} lands at {g}, nearer than {under}");
        }
    }

    /// Every screen point's sight-line meets the ground where projecting
    /// that ground point put it.
    #[test]
    fn projection_round_trips_through_the_rig() {
        let rig = Rig::new();
        for p in [
            Vec2::ZERO,
            Vec2::new(300.0, -200.0),
            Vec2::new(-500.0, 400.0),
            Vec2::new(600.0, rig.focus_d + 300.0),
        ] {
            let (screen, _) = rig.project(p);
            assert!(
                rig.unproject(screen).distance(p) < 0.5,
                "{p} -> {screen:?} does not round-trip"
            );
        }
    }

    /// Flat shading: a cell's life lights all three of its vertices, and
    /// lights them by the same amount — the cell is one flat glow, only the
    /// nebula varying across its corners.
    #[test]
    fn flat_shading_paints_each_cell_by_its_own_life() {
        let mut grid = test_grid(9, 9, 5);
        let k = grid.index(4, 4, Orientation::Up);
        let paint_cell = |grid: &mut LifeGrid, alive: bool| {
            grid.reseed(0.0);
            grid.cells[k] = u8::from(alive);
            let mut mesh = field_mesh(9, 9);
            paint(grid, &mut mesh);
            match mesh.attribute(Mesh::ATTRIBUTE_COLOR) {
                Some(VertexAttributeValues::Float32x4(colours)) => colours.clone(),
                _ => panic!("colours are float4"),
            }
        };
        let lum = |c: [f32; 4]| c[0] + c[1] + c[2];
        let lit = paint_cell(&mut grid, true);
        let dark = paint_cell(&mut grid, false);
        let gains: Vec<f32> = (0..3)
            .map(|c| lum(lit[3 * k + c]) - lum(dark[3 * k + c]))
            .collect();
        assert!(
            gains.iter().all(|&g| g > 0.0),
            "a live cell glows at all three corners: {gains:?}"
        );
        let (lo, hi) = (
            gains.iter().cloned().fold(f32::INFINITY, f32::min),
            gains.iter().cloned().fold(f32::NEG_INFINITY, f32::max),
        );
        assert!(
            hi - lo < 0.05,
            "the cell's life is flat across it: {gains:?}"
        );
    }

    /// Every cell owns its three corners: the mesh is a flat-shaded triangle
    /// list with no shared vertices, and the corner table maps each cell's
    /// mesh vertices to the lattice points its own triangle spans.
    #[test]
    fn each_cell_owns_its_three_vertices() {
        let (w, h) = (6, 5);
        let mesh = field_mesh(w, h);
        assert!(mesh.indices().is_none(), "flat shading shares no vertices");
        assert_eq!(
            mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap().len(),
            6 * w * h
        );

        let grid = test_grid(w, h, 3);
        assert_eq!(grid.sheet.corners.len(), 6 * w * h);
        let corner = |vi: usize| grid.sheet.corners[vi];
        for v in 0..h {
            for u in 0..w {
                let k = v * w + u;
                assert_eq!(
                    [corner(3 * k), corner(3 * k + 1), corner(3 * k + 2)],
                    [
                        lattice_index(w, u, v) as u32,
                        lattice_index(w, u + 1, v) as u32,
                        lattice_index(w, u, v + 1) as u32,
                    ],
                    "up cell ({u},{v}) owns its low triangle"
                );
                let kd = w * h + v * w + u;
                assert_eq!(
                    [corner(3 * kd), corner(3 * kd + 1), corner(3 * kd + 2)],
                    [
                        lattice_index(w, u + 1, v) as u32,
                        lattice_index(w, u + 1, v + 1) as u32,
                        lattice_index(w, u, v + 1) as u32,
                    ],
                    "down cell ({u},{v}) owns its high triangle"
                );
            }
        }
    }

    /// The field covers everything the camera can see, even pushed to the
    /// far corner of its roam: every seen ground point lands inside the
    /// sheet, in lattice coordinates, with room to spare.
    #[test]
    fn the_field_covers_everything_the_camera_can_see() {
        let view = Vec2::new(450.0, 300.0);
        let (rig, grid, _mesh) = build(view, 3);
        let a = Vec2::new(grid.side, 0.0);
        let b = Vec2::new(0.5 * grid.side, grid.side * SQRT3_2);
        // The sheet's anchor: back it out of any lattice vertex.
        let origin = grid.sheet.lattice[0];
        for corner in [Vec2::NEG_ONE, Vec2::X, Vec2::Y, Vec2::ONE] {
            for offset in [
                Vec2::splat(CAM_PAN),
                Vec2::splat(-CAM_PAN),
                Vec2::new(CAM_PAN, -CAM_PAN),
                Vec2::ZERO,
            ] {
                let d = rig.unproject(corner * view) + offset - origin;
                let v = d.y / b.y;
                let u = (d.x - v * b.x) / a.x;
                assert!(
                    (-1.0..=grid.w as f32 + 1.0).contains(&u)
                        && (-1.0..=grid.h as f32 + 1.0).contains(&v),
                    "corner {corner:?} at {offset:?} sees past the field: u={u}, v={v}"
                );
            }
        }
    }

    /// The same room seeds the same field.
    #[test]
    fn the_same_room_sees_the_same_sky() {
        assert_eq!(seed_of("parlor"), seed_of("parlor"));
        assert_ne!(seed_of("parlor"), seed_of("cupboard"));
    }
}
