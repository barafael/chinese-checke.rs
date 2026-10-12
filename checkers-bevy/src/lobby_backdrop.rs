//! The lobby's living backdrop: an endless triangle field that slowly turns
//! and drifts, carrying a Game of Life played out on its cells — and, now and
//! then, a cell hopping over a neighbour the way a piece hops over another on
//! the board.
//!
//! The field is one vertex-coloured [`Mesh2d`]: an equilateral triangle grid,
//! each triangle shrunk a little toward its centroid so the clear colour shows
//! through as the grid lines. The grid's parallelogram is sized so its
//! inscribed circle always swallows the whole viewport plus the pan and spin
//! margins, so no edge is ever seen — an infinite grid, as far as the lobby
//! can tell. The whole thing rotates about its centre and wanders on a slow
//! Lissajous path, which is the panning and turning.
//!
//! Life runs on the triangular tessellation itself: every triangle is a cell
//! with twelve neighbours — three across its edges, nine at its corners —
//! under Conway's B3/S23. Between generations, cells sometimes jump: over an
//! occupied same-orientation neighbour, into the empty cell exactly beyond,
//! along one of the six hex directions of that sublattice — the Chinese
//! checkers hop, which the same-orientation cells form for free.
//!
//! The colour gradient is adapted from cake's sky: from the page's deep
//! near-black (`#0f1014`) toward the violet of its nebula tint, brightening
//! along a diagonal so the field is never flat. A seed comes from the room
//! name, so everyone in a room watches the same sky live — the same trick the
//! nebula plays.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use bevy::asset::RenderAssetUsages;
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
            (drift, tick_life, ensure_coverage).run_if(in_state(AppState::Lobby)),
        );
}

/// The backdrop entity: one mesh holding every triangle, and the life living
/// on it. Gone with the lobby; the board deserves a quiet room.
#[derive(Component)]
#[require(DespawnOnExit<AppState> = DespawnOnExit(AppState::Lobby))]
struct Backdrop {
    mesh: Handle<Mesh>,
    grid: LifeGrid,
}

/// One revolution takes this many seconds.
const SPIN_PERIOD_SECS: f32 = 720.0;
/// How far the field wanders from centre, in world units. [`build`] leaves
/// at least this much slack beyond the viewport's half diagonal.
const PAN_AMPLITUDE: f32 = 30.0;
/// How far the breathing scale may swell the field, as a fraction.
const BREATH: f32 = 0.04;
/// Seconds between generations.
const TICK_SECS: f32 = 0.60;
/// The chance, per live cell per generation, of trying to hop.
const HOP_CHANCE: f32 = 0.01;
/// How many ticks a hop's landing cell stays lit.
const FLASH_TICKS: u8 = 4;
/// How triangles shrink toward their centroids; the remainder is the grid
/// line the clear colour shows through.
const GRID_LINE: f32 = 0.10;
/// About how many triangles the eye should find on screen at once.
const TARGET_VISIBLE: usize = 150;
/// Where the backdrop sits in the 2D stack: under everything the game or the
/// lobby will ever draw.
const LAYER: f32 = -10.0;
/// Cake's page background, the cool end of the gradient (`#0f1014`, eased a
/// touch darker so the grid lines read). The lobby's scrim shares it.
pub(crate) const DEEP: Srgba = Srgba::rgb(0.050, 0.055, 0.072);
/// The nebula's violet from cake's sky, the warm end.
const VIOLET: Srgba = Srgba::rgb(0.500, 0.400, 0.800);

/// The world-space half diagonal the camera shows right now.
fn half_diagonal(projection: &Projection) -> f32 {
    match projection {
        Projection::Orthographic(o) => o.area.half_size().length(),
        _ => 400.0,
    }
}

/// Spawn the field, sized to whatever the camera currently frames.
fn spawn(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    room: Res<RoomId>,
    cameras: Query<&Projection, With<Camera>>,
) {
    let half_diagonal = cameras.single().map(half_diagonal).unwrap_or(400.0);
    let (grid, mesh) = build(half_diagonal, seed_of(&room.0));
    let mesh = meshes.add(mesh);
    let material = materials.add(ColorMaterial::from_color(Color::WHITE));
    commands.spawn((
        Mesh2d(mesh.clone()),
        MeshMaterial2d(material),
        Transform::from_xyz(0.0, 0.0, LAYER),
        Pickable::IGNORE,
        Backdrop { mesh, grid },
    ));
}

/// The backdrop's mesh goes with it: the next lobby entry builds a fresh one
/// against whatever the window has become.
fn teardown(mut meshes: ResMut<Assets<Mesh>>, backdrops: Query<&Backdrop>) {
    for backdrop in &backdrops {
        meshes.remove(&backdrop.mesh);
    }
}

/// Turn and wander. The rotation is slow enough to read as stillness that
/// stirs; the pan is a two-frequency drift so the path never repeats on
/// screen; the breathing scale keeps the whole field faintly alive.
fn drift(time: Res<Time>, mut field: Query<&mut Transform, With<Backdrop>>) {
    let t = time.elapsed_secs();
    for mut transform in &mut field {
        transform.rotation = Quat::from_rotation_z(t * std::f32::consts::TAU / SPIN_PERIOD_SECS);
        let pan = Vec2::new(
            0.6 * (t * 0.037).sin() + 0.4 * (t * 0.016).sin(),
            0.6 * (t * 0.028 + 1.7).sin() + 0.4 * (t * 0.010).cos(),
        ) * PAN_AMPLITUDE;
        transform.translation = pan.extend(LAYER);
        let breath = 1.0 + BREATH * (t * 0.014).sin();
        transform.scale = Vec3::splat(breath);
    }
}

/// Advance the life one generation every [`TICK_SECS`], and repaint the
/// vertex colours to match. The mesh is rewritten in place, so the draw
/// follows on the same frame.
fn tick_life(
    time: Res<Time>,
    mut backdrops: Query<&mut Backdrop>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut clock: Local<f32>,
) {
    let Ok(mut backdrop) = backdrops.single_mut() else {
        return;
    };
    *clock += time.delta_secs();
    if *clock < TICK_SECS {
        return;
    }
    *clock = 0.0;
    backdrop.grid.step();
    if let Some(mut mesh) = meshes.get_mut(&backdrop.mesh) {
        paint(&backdrop.grid, &mut mesh);
    }
}

/// Rebuild the field if the window has grown past the margins it was built
/// with — a maximise mid-lobby must not bare the grid's edge. Shrinking is
/// left alone: extra field off-screen costs nothing.
fn ensure_coverage(
    mut resized: MessageReader<WindowResized>,
    cameras: Query<&Projection, With<Camera>>,
    mut backdrops: Query<&mut Backdrop>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    if resized.read().next().is_none() {
        return;
    }
    let Ok(half_diagonal) = cameras.single().map(half_diagonal) else {
        return;
    };
    let Ok(mut backdrop) = backdrops.single_mut() else {
        return;
    };
    if half_diagonal + PAN_AMPLITUDE <= backdrop.grid.slack() {
        return;
    }
    let (grid, mesh) = build_with_side(half_diagonal, backdrop.grid.side, backdrop.grid.seed());
    backdrop.grid = grid;
    if let Some(mut existing) = meshes.get_mut(&backdrop.mesh) {
        *existing = mesh;
    }
}

// ---- the grid --------------------------------------------------------------

const SQRT3_2: f32 = 0.866_025_4;

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
/// Cells are indexed `k = orientation * w * h + v * w + u`; triangle `k` of
/// the mesh is cell `k`, its vertices at `3k .. 3k + 3`. The topology is a
/// torus: indices wrap, so the life never notices an edge even if the camera
/// somehow could.
pub(crate) struct LifeGrid {
    w: usize,
    h: usize,
    /// Triangle side, in world units.
    side: f32,
    /// The radius of open field guaranteed around the viewport's centre:
    /// the inscribed circle the grid was sized to.
    slack: f32,
    cells: Vec<u8>,
    /// Generations since birth; newborns glow brighter, elders settle.
    ages: Vec<u8>,
    /// Ticks of highlight left, set on a hop's landing cell.
    flash: Vec<u8>,
    /// Scratch for the generation step.
    next: Vec<u8>,
    /// The twelve neighbours of every cell, precomputed and wrapped.
    nbrs: Vec<[u32; 12]>,
    /// Where each cell sits on the deep-to-violet gradient, `0..1`.
    grad_t: Vec<f32>,
    rng: Rng,
    seed: u64,
}

impl LifeGrid {
    /// A field `w * h` parallelogram cells across, seeded from `seed` and
    /// sized to guarantee `slack` radius of coverage.
    pub(crate) fn new(w: usize, h: usize, side: f32, slack: f32, seed: u64) -> Self {
        let n = 2 * w * h;
        let mut grid = LifeGrid {
            w,
            h,
            side,
            slack,
            cells: vec![0; n],
            ages: vec![0; n],
            flash: vec![0; n],
            next: vec![0; n],
            nbrs: neighbor_table(w, h),
            grad_t: gradient(w, h, side),
            rng: Rng(seed.max(1)),
            seed,
        };
        grid.reseed(0.30);
        grid
    }

    /// The cell index of orientation `o` at `(u, v)`, unwrapped.
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

    /// Fresh soup. Called at birth, and by `step` whenever the population
    /// escapes its band: burned out, or grown to a churning near-solid.
    pub(crate) fn reseed(&mut self, p: f32) {
        for ((cell, age), flash) in self
            .cells
            .iter_mut()
            .zip(self.ages.iter_mut())
            .zip(self.flash.iter_mut())
        {
            *cell = u8::from(self.rng.chance(p));
            *age = 0;
            *flash = 0;
        }
    }

    /// One generation: Conway's birth and survival over the twelve
    /// neighbours, then the hops, then the upkeep.
    pub(crate) fn step(&mut self) {
        self.generation();
        self.hops(HOP_CHANCE);
        self.upkeep();
    }

    /// The B3/S23 generation over the 12-neighbourhood.
    pub(crate) fn generation(&mut self) {
        for i in 0..self.cells.len() {
            let mut count = 0;
            for &n in &self.nbrs[i] {
                count += self.cells[n as usize];
            }
            // Conway's: three neighbours birth, two or three keep alive.
            let lives = matches!((self.cells[i], count), (1, 2 | 3) | (0, 3));
            self.next[i] = u8::from(lives);
        }
        std::mem::swap(&mut self.cells, &mut self.next);
    }

    /// The checkers moves: each live cell may leap over an occupied
    /// same-orientation neighbour into the empty cell exactly beyond. The
    /// mover's cell empties; the jumped cell stays — a hop is a hop, not a
    /// capture.
    pub(crate) fn hops(&mut self, chance: f32) {
        let (w, h) = (self.w as i32, self.h as i32);
        for u in 0..w {
            for v in 0..h {
                for o in [Orientation::Up, Orientation::Down] {
                    if self.cells[self.index(u, v, o)] == 0 || !self.rng.chance(chance) {
                        continue;
                    }
                    let dir = HOP_DIRECTIONS[self.rng.below(HOP_DIRECTIONS.len())];
                    self.try_hop(u, v, o, dir);
                }
            }
        }
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
        self.ages[landing] = 0;
        self.flash[landing] = FLASH_TICKS;
        true
    }

    /// Ages and flashes advance; a population burnt out or grown past its
    /// band is reseeded.
    fn upkeep(&mut self) {
        for i in 0..self.cells.len() {
            if self.cells[i] == 1 {
                self.ages[i] = self.ages[i].saturating_add(1);
            }
            self.flash[i] = self.flash[i].saturating_sub(1);
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

    /// The coverage radius the grid guarantees around the viewport centre.
    pub(crate) fn slack(&self) -> f32 {
        self.slack
    }

    /// The colour one cell wears this tick, as linear RGBA for the vertex
    /// buffer.
    fn cell_colour(&self, k: usize) -> [f32; 4] {
        let t = self.grad_t[k];
        let srgba = if self.cells[k] == 1 {
            let base = DEEP.mix(&VIOLET, 0.45 + 0.45 * t);
            // Newborns glow, elders settle, a fresh landing flashes.
            let youth = 1.0 - (self.ages[k].min(16) as f32) / 16.0;
            let lit = (0.08 + 0.16 * youth + 0.07 * self.flash[k] as f32).min(0.55);
            base.mix(&Srgba::WHITE, lit)
        } else {
            // The empty field whispers: near cake's deep at the cool end,
            // breathing faint violet toward the warm one.
            DEEP.mix(&VIOLET, 0.10 + 0.30 * t).mix(&Srgba::BLACK, 0.30)
        };
        let lin = LinearRgba::from(srgba);
        [lin.red, lin.green, lin.blue, 1.0]
    }
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

/// Where each cell sits on the deep-to-violet gradient: a diagonal sweep,
/// `0` at the grid's low corner, `1` at its high one.
fn gradient(w: usize, h: usize, side: f32) -> Vec<f32> {
    let span_x = side * (w as f32 + 0.5 * h as f32);
    let span_y = side * SQRT3_2 * h as f32;
    let mut ts = Vec::with_capacity(2 * w * h);
    for o in [Orientation::Up, Orientation::Down] {
        for v in 0..h {
            for u in 0..w {
                let c = cell_centroid(u, v, o, side);
                let t = (c.x + c.y + 0.5 * (span_x + span_y)) / (span_x + span_y);
                ts.push(t.clamp(0.0, 1.0));
            }
        }
    }
    ts
}

/// A cell's centroid, in grid-local world units, before centring.
fn cell_centroid(u: usize, v: usize, o: Orientation, side: f32) -> Vec2 {
    let a = Vec2::new(side, 0.0);
    let b = Vec2::new(0.5 * side, side * SQRT3_2);
    let p = |du: f32, dv: f32| du * a + dv * b;
    match o {
        Orientation::Up => {
            (p(u as f32, v as f32) + p(u as f32 + 1.0, v as f32) + p(u as f32, v as f32 + 1.0))
                / 3.0
        }
        Orientation::Down => {
            (p(u as f32 + 1.0, v as f32)
                + p(u as f32 + 1.0, v as f32 + 1.0)
                + p(u as f32, v as f32 + 1.0))
                / 3.0
        }
    }
}

/// Build a field covering a viewport of `half_diagonal` world-unit radius,
/// plus the pan, spin, and breathing margins. Returns the life and its mesh.
pub(crate) fn build(half_diagonal: f32, seed: u64) -> (LifeGrid, Mesh) {
    let (w, h, side, slack, mesh) = geometry(half_diagonal, default_side(half_diagonal));
    (LifeGrid::new(w, h, side, slack, seed), mesh)
}

/// As [`build`], but at a given triangle size — the resize path keeps the
/// field's grain and grows its reach instead.
fn build_with_side(half_diagonal: f32, side: f32, seed: u64) -> (LifeGrid, Mesh) {
    let (w, h, side, slack, mesh) = geometry(half_diagonal, side);
    (LifeGrid::new(w, h, side, slack, seed), mesh)
}

/// The triangle size that puts about [`TARGET_VISIBLE`] triangles on a
/// viewport of this half diagonal, assuming a 16:10-ish frame.
fn default_side(half_diagonal: f32) -> f32 {
    // A typical viewport area from its half diagonal, at a plump aspect.
    let area = 4.0 * half_diagonal * half_diagonal * 0.62;
    let per_triangle = SQRT3_2 / 2.0;
    (area / (TARGET_VISIBLE as f32 * per_triangle))
        .sqrt()
        .clamp(48.0, 110.0)
}

/// The grid dimensions whose inscribed circle swallows the viewport plus
/// every motion margin, and the mesh itself.
fn geometry(half_diagonal: f32, side: f32) -> (usize, usize, f32, f32, Mesh) {
    // Room for the pan, one triangle of spin sweep, and the breath.
    let slack = half_diagonal * (1.0 + BREATH) + PAN_AMPLITUDE + side;
    // An inscribed circle of radius `slack` needs the parallelogram's two
    // edge separations to be at least twice that.
    let n = (2.0 * slack / (side * SQRT3_2)).ceil() as usize;
    let (w, h) = (n, n);
    (w, h, side, slack, field_mesh(w, h, side))
}

/// The whole field as one triangle list, every triangle shrunk toward its
/// centroid so the gaps read as grid lines.
pub(crate) fn field_mesh(w: usize, h: usize, side: f32) -> Mesh {
    let a = Vec2::new(side, 0.0);
    let b = Vec2::new(0.5 * side, side * SQRT3_2);
    // Centre the covered parallelogram on the origin.
    let origin = -Vec2::new(
        side * (w as f32 + 0.5 * h as f32) / 2.0,
        side * SQRT3_2 * h as f32 / 2.0,
    );
    let shrink = 1.0 - GRID_LINE;

    let mut positions = Vec::with_capacity(6 * w * h);
    let mut uvs = Vec::with_capacity(6 * w * h);
    for v in 0..h {
        for u in 0..w {
            let at = |du: f32, dv: f32| origin + (u as f32 + du) * a + (v as f32 + dv) * b;
            for (p0, p1, p2) in [
                // Up: (u,v), (u+1,v), (u,v+1).
                (at(0.0, 0.0), at(1.0, 0.0), at(0.0, 1.0)),
                // Down: (u+1,v), (u+1,v+1), (u,v+1).
                (at(1.0, 0.0), at(1.0, 1.0), at(0.0, 1.0)),
            ] {
                let centroid = (p0 + p1 + p2) / 3.0;
                for p in [p0, p1, p2] {
                    let p = centroid + (p - centroid) * shrink;
                    positions.push([p.x, p.y, 0.0]);
                    uvs.push([0.0, 0.0]);
                }
            }
        }
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    // Painted by `paint` before the first draw; white stands in until then.
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0, 1.0, 1.0, 1.0]; 6 * w * h]);
    mesh
}

/// Write every cell's colour into the mesh's vertex colours.
pub(crate) fn paint(grid: &LifeGrid, mesh: &mut Mesh) {
    let colours: Vec<[f32; 4]> = (0..grid.cells.len())
        .flat_map(|k| std::iter::repeat_n(grid.cell_colour(k), 3))
        .collect();
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
        let mut grid = LifeGrid::new(6, 6, 10.0, 100.0, 7);
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
        grid.generation();
        assert_eq!(grid.cells[born], 1, "three neighbours birth");
        assert_eq!(grid.cells[kept], 1, "two neighbours survive");
        assert_eq!(grid.cells[lone], 0, "a lone cell dies");
    }

    /// With a lane open, a hop leaps over the occupied neighbour into the
    /// empty cell exactly beyond, which stays alive — a hop is not a capture.
    #[test]
    fn a_hop_leaps_over_an_occupied_neighbour_into_an_empty_landing() {
        let mut grid = LifeGrid::new(8, 8, 10.0, 200.0, 11);
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
        let mut grid = LifeGrid::new(8, 8, 10.0, 200.0, 13);
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
        let mut grid = LifeGrid::new(6, 6, 10.0, 100.0, 17);
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
        let mut grid = LifeGrid::new(14, 10, 10.0, 150.0, 23);
        let n = grid.cells.len() as u32;
        for _ in 0..400 {
            grid.step();
            let pop = grid.population();
            assert!(pop >= n / 20, "burned out: {pop}/{n}");
            assert!(pop <= n * 9 / 10, "saturated: {pop}/{n}");
            assert!(pop > 0);
        }
    }

    /// The mesh holds two triangles per cell, centred on the origin, and the
    /// vertex colour slots line up with the cells.
    #[test]
    fn the_mesh_holds_two_centred_triangles_per_cell() {
        let (w, h) = (6, 5);
        let mesh = field_mesh(w, h, 12.0);
        let positions = mesh
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .expect("positions present")
            .as_float3()
            .expect("positions are float3");
        assert_eq!(positions.len(), 6 * w * h);
        let centre: Vec2 = positions
            .iter()
            .map(|[x, y, _]| Vec2::new(*x, *y))
            .sum::<Vec2>()
            / (positions.len() as f32);
        assert!(centre.length() < 1.0, "field off centre: {centre}");

        let mut grid = LifeGrid::new(w, h, 12.0, 100.0, 29);
        let mut mesh = mesh;
        // Triangle k is cell k: a dead field paints exactly the dead colour.
        grid.reseed(0.0);
        paint(&grid, &mut mesh);
        let VertexAttributeValues::Float32x4(colours) = mesh
            .attribute(Mesh::ATTRIBUTE_COLOR)
            .expect("colours present")
        else {
            panic!("colours are float4");
        };
        assert_eq!(colours.len(), 6 * w * h);
        // Triangle k is cell k: each vertex triple carries exactly the
        // colour its cell was painted.
        for k in 0..grid.cells.len() {
            let wanted = grid.cell_colour(k);
            for c in &colours[3 * k..3 * k + 3] {
                assert_eq!(*c, wanted, "triangle {k} mismatched");
            }
        }
    }

    /// A pass of hops never changes the population: every hop is a move.
    #[test]
    fn hops_conserve_the_population() {
        let mut grid = LifeGrid::new(16, 12, 10.0, 150.0, 31);
        for _ in 0..10 {
            let before = grid.population();
            grid.hops(1.0);
            assert_eq!(grid.population(), before, "a hop is a move, not a birth");
            grid.step();
        }
    }

    /// The gradient is cake's: the cool end sits near the page's deep, the
    /// warm end well toward the nebula's violet.
    #[test]
    fn the_gradient_runs_from_deep_to_violet() {
        let cool = DEEP.mix(&VIOLET, 0.12);
        let warm = DEEP.mix(&VIOLET, 0.54);
        let toward = |c: Srgba| {
            let v = Vec3::new(c.red, c.green, c.blue);
            let violet = Vec3::new(VIOLET.red, VIOLET.green, VIOLET.blue);
            v.distance(violet)
        };
        assert!(
            toward(warm) < toward(cool),
            "warm end must be nearer violet"
        );
    }

    /// The same room seeds the same field.
    #[test]
    fn the_same_room_sees_the_same_sky() {
        assert_eq!(seed_of("parlor"), seed_of("parlor"));
        assert_ne!(seed_of("parlor"), seed_of("cupboard"));
    }
}
