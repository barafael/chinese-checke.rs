//! The backdrop: a procedural nebula that wavers very slowly, faint against
//! the dark clear colour, behind the lobby and the board alike.
//!
//! Ported from cake (`cake-bevy/src/nebula.rs`), whose algorithm is btl's
//! (github.com/barafael/btl, `btl-shared/src/nebula.rs`), itself after
//! randomfart. Three math expressions, one per colour channel, are grown at
//! random from a weighted grammar, compiled to stack-machine bytecode, and
//! evaluated per pixel over `(x, y, t)` in `[-1, 1]`. Moving `t` along a slow
//! sine makes the whole field drift back and forth.
//!
//! The texture is a small square stretched to cover the window, so the field
//! keeps its proportions whatever the window's shape; its look is
//! [`NebulaParams`], live-tunable from the [`crate::tweak`] pane.

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use std::f32::consts::TAU;

/// Texture resolution. Stretched over the whole window, and a nebula is soft
/// anyway, so low resolution is fine (and cheap to re-render).
const SIZE: u32 = 128;
/// Renders per waver period: `t` then moves at most ~0.01 between renders,
/// below what the eye catches at this opacity.
const RENDERS_PER_PERIOD: f32 = 600.0;
/// Bounds on the re-render interval, in seconds: no busier than 20 Hz for a
/// fast tweak-pane period, no lazier than twice a second for a slow one. A
/// render takes ~13 ms in a debug build, well under 1 ms in release.
const RENDER_INTERVAL_SECS: (f32, f32) = (0.05, 0.5);
/// Expression tree depth.
const DEPTH: u32 = 5;
/// Behind everything the camera draws: the board's holes sit at 0.
const NEBULA_Z: f32 = -500.0;

/// The nebula's look. Every field is a knob of the tweak pane.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct NebulaParams {
    pub shown: bool,
    /// Grows the field: same seed, same field.
    pub seed: u64,
    /// Alpha of the tint: how far the field shows through the clear colour.
    pub opacity: f32,
    /// Linear-ish RGB multiplier over the field's own colours.
    pub tint: [f32; 3],
    /// One full back-and-forth of the waver, in seconds.
    pub period_secs: f32,
}

impl Default for NebulaParams {
    fn default() -> Self {
        Self {
            shown: true,
            seed: 0x5EED,
            // Faint: just visible over the 0.09 grey clear colour.
            opacity: 0.05,
            // btl's tint: faint violet, so the board stays the brightest thing.
            tint: [0.5, 0.4, 0.8],
            // Very slow: five minutes there and back.
            period_secs: 300.0,
        }
    }
}

impl NebulaParams {
    fn colour(&self) -> Color {
        let [r, g, b] = self.tint;
        Color::srgba(r, g, b, self.opacity)
    }
}

pub fn plugin(app: &mut App) {
    app.init_resource::<NebulaParams>()
        .add_systems(Startup, spawn)
        .add_systems(Update, (follow_params, fit_to_view, waver));
}

/// Marks the nebula sprite.
#[derive(Component)]
struct NebulaSprite;

#[derive(Resource)]
struct Nebula {
    programs: Programs,
    /// The seed `programs` was grown from.
    seed: u64,
    image: Handle<Image>,
    /// Where the waver is along its sine, in radians. Accumulated rather than
    /// derived from the clock, so a new period changes the speed without a
    /// jump.
    phase: f32,
    /// When the texture was last rendered, in elapsed seconds.
    rendered_at: f32,
}

fn spawn(mut commands: Commands, params: Res<NebulaParams>, mut images: ResMut<Assets<Image>>) {
    let programs = Programs::generate(params.seed);
    let image = images.add(Image::new(
        Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        render(&programs, 0.0),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    ));
    commands.spawn((
        Sprite {
            image: image.clone(),
            color: params.colour(),
            ..default()
        },
        Transform::from_xyz(0.0, 0.0, NEBULA_Z),
        // Scenery: clicks go through it to the board and the lobby.
        Pickable::IGNORE,
        NebulaSprite,
    ));
    commands.insert_resource(Nebula {
        programs,
        seed: params.seed,
        image,
        phase: 0.0,
        rendered_at: 0.0,
    });
}

/// Apply the params: visibility and tint at once, and a new seed by growing
/// new programs (rendered by the next [`waver`]).
fn follow_params(
    params: Res<NebulaParams>,
    mut nebula: ResMut<Nebula>,
    mut sprites: Query<(&mut Sprite, &mut Visibility), With<NebulaSprite>>,
) {
    if !params.is_changed() {
        return;
    }
    let want = if params.shown {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for (mut sprite, mut visible) in &mut sprites {
        visible.set_if_neq(want);
        sprite.color = params.colour();
    }
    if params.seed != nebula.seed {
        nebula.programs = Programs::generate(params.seed);
        nebula.seed = params.seed;
        // Due now, not at the next interval.
        nebula.rendered_at = f32::NEG_INFINITY;
    }
}

/// Cover the camera's view: a square as large as the view's longer side, so
/// the field is cropped rather than stretched. The camera zooms with the
/// window (see `fit_projection`), so this follows it every frame.
fn fit_to_view(
    cameras: Query<&Projection, With<Camera2d>>,
    mut sprites: Query<&mut Sprite, With<NebulaSprite>>,
) {
    let Some(Projection::Orthographic(ortho)) = cameras.iter().next() else {
        return;
    };
    let side = ortho.area.width().max(ortho.area.height());
    let want = Some(Vec2::splat(side));
    for mut sprite in &mut sprites {
        if sprite.custom_size != want {
            sprite.custom_size = want;
        }
    }
}

/// Advance the waver and re-render the texture. The image is rewritten in
/// place on the GPU, so the sprite follows it.
fn waver(
    params: Res<NebulaParams>,
    time: Res<Time>,
    mut nebula: ResMut<Nebula>,
    mut images: ResMut<Assets<Image>>,
) {
    if !params.shown {
        return;
    }
    nebula.phase = (nebula.phase + time.delta_secs() * TAU / params.period_secs) % TAU;
    let now = time.elapsed_secs();
    let (fastest, slowest) = RENDER_INTERVAL_SECS;
    let interval = (params.period_secs / RENDERS_PER_PERIOD).clamp(fastest, slowest);
    if now - nebula.rendered_at < interval {
        return;
    }
    nebula.rendered_at = now;
    let pixels = render(&nebula.programs, nebula.phase.sin());
    if let Some(mut image) = images.get_mut(&nebula.image) {
        match &mut image.data {
            Some(data) if data.len() == pixels.len() => data.copy_from_slice(&pixels),
            slot => *slot = Some(pixels),
        }
    }
}

/// The opaque RGBA texture at time `t`.
fn render(programs: &Programs, t: f32) -> Vec<u8> {
    let mut pixels = vec![255u8; (SIZE * SIZE * 4) as usize];
    for (i, px) in pixels.chunks_mut(4).enumerate() {
        let i = i as u32;
        // Sample texel centres. Image rows run top to bottom; y runs up.
        let y = 1.0 - ((i / SIZE) as f32 + 0.5) / SIZE as f32 * 2.0;
        let x = ((i % SIZE) as f32 + 0.5) / SIZE as f32 * 2.0 - 1.0;
        px[0] = channel(eval(&programs.r, x, y, t));
        px[1] = channel(eval(&programs.g, x, y, t));
        px[2] = channel(eval(&programs.b, x, y, t));
    }
    pixels
}

/// Map `[-1, 1]` to a byte.
fn channel(v: f32) -> u8 {
    (((v + 1.0) * 0.5).clamp(0.0, 1.0) * 255.0) as u8
}

// ---- Bytecode --------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
enum Op {
    X,
    Y,
    T,
    Num(f32),
    Add,
    Mult,
    Sqrt,
    Abs,
    Sin,
    Mix,
}

/// Evaluate a compiled program at `(x, y, t)`.
fn eval(ops: &[Op], x: f32, y: f32, t: f32) -> f32 {
    let mut stack = [0f32; 32];
    let mut sp = 0usize;
    for &op in ops {
        match op {
            Op::X => {
                stack[sp] = x;
                sp += 1;
            }
            Op::Y => {
                stack[sp] = y;
                sp += 1;
            }
            Op::T => {
                stack[sp] = t;
                sp += 1;
            }
            Op::Num(n) => {
                stack[sp] = n;
                sp += 1;
            }
            Op::Abs => stack[sp - 1] = stack[sp - 1].abs(),
            Op::Sqrt => stack[sp - 1] = stack[sp - 1].abs().sqrt(),
            Op::Sin => stack[sp - 1] = (stack[sp - 1] * std::f32::consts::PI).sin(),
            Op::Add => {
                sp -= 1;
                stack[sp - 1] += stack[sp];
            }
            Op::Mult => {
                sp -= 1;
                stack[sp - 1] *= stack[sp];
            }
            Op::Mix => {
                sp -= 2;
                let w = ((stack[sp + 1] + 1.0) * 0.5).clamp(0.0, 1.0);
                stack[sp - 1] = stack[sp - 1] * (1.0 - w) + stack[sp] * w;
            }
        }
    }
    stack[0]
}

/// One compiled program per colour channel.
#[derive(Debug, PartialEq)]
struct Programs {
    r: Vec<Op>,
    g: Vec<Op>,
    b: Vec<Op>,
}

impl Programs {
    /// Same seed, same nebula.
    fn generate(seed: u64) -> Programs {
        // xorshift gets stuck at zero.
        let mut rng = Rng(seed.max(1));
        let compile = |rng: &mut Rng| {
            let mut ops = Vec::new();
            Expr::generate(rng, DEPTH).compile(&mut ops);
            ops
        };
        let r = compile(&mut rng);
        let g = compile(&mut rng);
        let b = compile(&mut rng);
        Programs { r, g, b }
    }
}

// ---- Expression grammar ----------------------------------------------------

enum Expr {
    X,
    Y,
    T,
    Num(f32),
    Add(Box<Expr>, Box<Expr>),
    Mult(Box<Expr>, Box<Expr>),
    Sqrt(Box<Expr>),
    Abs(Box<Expr>),
    Sin(Box<Expr>),
    Mix(Box<Expr>, Box<Expr>, Box<Expr>),
}

impl Expr {
    fn compile(&self, ops: &mut Vec<Op>) {
        match self {
            Expr::X => ops.push(Op::X),
            Expr::Y => ops.push(Op::Y),
            Expr::T => ops.push(Op::T),
            Expr::Num(n) => ops.push(Op::Num(*n)),
            Expr::Abs(e) => {
                e.compile(ops);
                ops.push(Op::Abs);
            }
            Expr::Sqrt(e) => {
                e.compile(ops);
                ops.push(Op::Sqrt);
            }
            Expr::Sin(e) => {
                e.compile(ops);
                ops.push(Op::Sin);
            }
            Expr::Add(a, b) => {
                a.compile(ops);
                b.compile(ops);
                ops.push(Op::Add);
            }
            Expr::Mult(a, b) => {
                a.compile(ops);
                b.compile(ops);
                ops.push(Op::Mult);
            }
            Expr::Mix(a, b, c) => {
                a.compile(ops);
                b.compile(ops);
                c.compile(ops);
                ops.push(Op::Mix);
            }
        }
    }

    /// btl's weights, tuned for smooth, flowing fields: plenty of sin, add
    /// and mix for undulation and blending, and no modulo, so no tiling.
    fn generate(rng: &mut Rng, depth: u32) -> Expr {
        const W_TERMINAL: u32 = 2;
        const W_ADD: u32 = 3;
        const W_MULT: u32 = 2;
        const W_SQRT: u32 = 1;
        const W_SIN: u32 = 3;
        const W_MIX: u32 = 2;
        const TOTAL: u32 = W_TERMINAL + W_ADD + W_MULT + W_SQRT + W_SIN + W_MIX;

        let roll = rng.next_u32() % TOTAL;
        if depth == 0 || roll < W_TERMINAL {
            return Self::terminal(rng);
        }
        let mut cursor = W_TERMINAL + W_ADD;
        if roll < cursor {
            return Expr::Add(
                Box::new(Self::generate(rng, depth - 1)),
                Box::new(Self::generate(rng, depth - 1)),
            );
        }
        cursor += W_MULT;
        if roll < cursor {
            return Expr::Mult(
                Box::new(Self::generate(rng, depth - 1)),
                Box::new(Self::generate(rng, depth - 1)),
            );
        }
        cursor += W_SQRT;
        if roll < cursor {
            return Expr::Sqrt(Box::new(Expr::Abs(Box::new(Self::generate(
                rng,
                depth - 1,
            )))));
        }
        cursor += W_SIN;
        if roll < cursor {
            return Expr::Sin(Box::new(Self::generate(rng, depth - 1)));
        }
        Expr::Mix(
            Box::new(Self::generate(rng, depth - 1)),
            Box::new(Self::generate(rng, depth - 1)),
            Box::new(Self::generate(rng, depth - 1)),
        )
    }

    fn terminal(rng: &mut Rng) -> Expr {
        match rng.next_u32() % 7 {
            0 => Expr::Num(rng.next_f32() * 2.0 - 1.0),
            1 => Expr::X,
            2 => Expr::Y,
            3 => Expr::Abs(Box::new(Expr::X)),
            4 => Expr::Abs(Box::new(Expr::Y)),
            5 => Expr::Sqrt(Box::new(Expr::Add(
                Box::new(Expr::Mult(Box::new(Expr::X), Box::new(Expr::X))),
                Box::new(Expr::Mult(Box::new(Expr::Y), Box::new(Expr::Y))),
            ))),
            _ => Expr::T,
        }
    }
}

/// btl's xorshift64.
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 16) as u32
    }

    /// In `[0, 1)`.
    fn next_f32(&mut self) -> f32 {
        (self.next_u32() & 0x00FF_FFFF) as f32 / 16_777_216.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_seed_grows_the_same_nebula() {
        assert_eq!(Programs::generate(42), Programs::generate(42));
        assert_ne!(Programs::generate(42), Programs::generate(43));
    }

    /// Depth 5 bounds the stack well inside the evaluator's 32 slots, for
    /// every seed; a program that overflowed would panic mid-frame.
    #[test]
    fn every_program_fits_the_stack_and_leaves_one_value() {
        for seed in 1..2000u64 {
            let p = Programs::generate(seed);
            for ops in [&p.r, &p.g, &p.b] {
                let mut depth: i32 = 0;
                let mut max = 0;
                for op in ops.iter() {
                    depth += match op {
                        Op::X | Op::Y | Op::T | Op::Num(_) => 1,
                        Op::Abs | Op::Sqrt | Op::Sin => 0,
                        Op::Add | Op::Mult => -1,
                        Op::Mix => -2,
                    };
                    assert!(depth >= 1, "seed {seed}: stack underflow");
                    max = max.max(depth);
                }
                assert_eq!(depth, 1, "seed {seed}");
                assert!(max <= 32, "seed {seed}: needs {max} slots");
            }
        }
    }

    #[test]
    fn the_texture_is_opaque_everywhere() {
        let pixels = render(&Programs::generate(7), 0.3);
        assert_eq!(pixels.len(), (SIZE * SIZE * 4) as usize);
        assert!(pixels.chunks(4).all(|px| px[3] == 255));
    }

    #[test]
    fn time_moves_the_field() {
        // Some seed among the first few must use `t`; each one that does
        // must change with it.
        let moved = (1..20u64).any(|seed| {
            let p = Programs::generate(seed);
            render(&p, -0.5) != render(&p, 0.5)
        });
        assert!(moved);
    }

    /// Not an assertion: how long one re-render takes in this build.
    #[test]
    fn render_cost() {
        let p = Programs::generate(0x5EED);
        let start = std::time::Instant::now();
        for k in 0..10 {
            std::hint::black_box(render(&p, k as f32 / 10.0));
        }
        eprintln!("one render: {:?}", start.elapsed() / 10);
    }
}
