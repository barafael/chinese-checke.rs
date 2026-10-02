//! Inline SVG figures for the laws report.
//!
//! Every figure is computed from [`checkers_core::geometry`] — the same
//! predicates the laws are checked against — so a picture cannot drift from
//! the board the code plays on.

use checkers_core::geometry::{
    Coord, Dir, all_holes, camp_of, in_base_camp, in_camp, in_hex, in_inward_camp,
};

const SQRT3: f64 = 1.732_050_807_568_877_2;
const SUBS: [char; 6] = ['₀', '₁', '₂', '₃', '₄', '₅'];

/// Camp colours by camp index.
pub const CAMP_COLORS: [&str; 6] = [
    "#e53935", "#fb8c00", "#f9a825", "#43a047", "#1e88e5", "#8e24aa",
];

const INK: &str = "#37474f";
const MUTED: &str = "#90a4ae";
const FAINT: &str = "#eceff1";
const FAINT_STROKE: &str = "#cfd8dc";
const RED: &str = "#e53935";
const RED_DARK: &str = "#b71c1c";
const RED_TINT: &str = "#ffcdd2";
const GREEN: &str = "#2e7d32";
const GREEN_TINT: &str = "#c8e6c9";
const BLOCKER: &str = "#78909c";

/// Every coordinate within reach of the star that satisfies `pred`. A scan
/// of the grid rather than of [`all_holes`], because the inward camp is not on
/// the board.
fn holes_where(pred: impl Fn(Coord) -> bool) -> Vec<Coord> {
    (-8..=8)
        .flat_map(|q| (-8..=8).map(move |r| Coord::new(q, r)))
        .filter(|&c| pred(c))
        .collect()
}

fn hexes() -> Vec<Coord> {
    holes_where(in_hex)
}

fn camp_holes(camp: u32) -> Vec<Coord> {
    holes_where(|c| in_camp(c, camp))
}

fn hex_contacts(c: Coord) -> usize {
    Dir::ALL.iter().filter(|d| in_hex(c.neighbour(**d))).count()
}

fn contact_pairs_of(camp_holes: &[Coord]) -> Vec<(Coord, Coord)> {
    let mut v = Vec::new();
    for &c in camp_holes {
        for d in Dir::ALL {
            let n = c.neighbour(d);
            if in_hex(n) {
                v.push((c, n));
            }
        }
    }
    v
}

fn raw(scale: f64, c: Coord) -> (f64, f64) {
    (
        SQRT3 * scale * (c.q as f64 + 0.5 * c.r as f64),
        1.5 * scale * c.r as f64,
    )
}

struct Frame {
    scale: f64,
    pad: f64,
    min_x: f64,
    min_y: f64,
    w: f64,
    h: f64,
}

impl Frame {
    fn new(coords: &[Coord], scale: f64, pad: f64) -> Frame {
        let mut min_x = f64::MAX;
        let mut min_y = f64::MAX;
        let mut max_x = f64::MIN;
        let mut max_y = f64::MIN;
        for &c in coords {
            let (x, y) = raw(scale, c);
            min_x = min_x.min(x);
            max_x = max_x.max(x);
            min_y = min_y.min(y);
            max_y = max_y.max(y);
        }
        Frame {
            scale,
            pad,
            min_x,
            min_y,
            w: (max_x - min_x) + 2.0 * pad,
            h: (max_y - min_y) + 2.0 * pad,
        }
    }

    fn px(&self, c: Coord) -> (f64, f64) {
        let (x, y) = raw(self.scale, c);
        (x - self.min_x + self.pad, y - self.min_y + self.pad)
    }

    fn hole_r(&self) -> f64 {
        0.40 * self.scale
    }
}

fn wrap(f: &Frame, body: &str) -> String {
    let (w, h) = (f.w, f.h);
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {w:.0} {h:.0}\" width=\"{w:.0}\" height=\"{h:.0}\" class=\"figure-svg\">{body}</svg>"
    )
}

fn circle(x: f64, y: f64, r: f64, fill: &str, stroke: &str, width: f64) -> String {
    format!(
        "<circle cx=\"{x:.1}\" cy=\"{y:.1}\" r=\"{r:.1}\" fill=\"{fill}\" stroke=\"{stroke}\" stroke-width=\"{width:.1}\"/>"
    )
}

fn hole(f: &Frame, c: Coord, fill: &str, stroke: &str) -> String {
    let (x, y) = f.px(c);
    circle(x, y, f.hole_r(), fill, stroke, 1.5)
}

fn ring(f: &Frame, c: Coord, color: &str) -> String {
    let (x, y) = f.px(c);
    circle(x, y, f.hole_r() + 4.0, "none", color, 2.5)
}

fn piece(f: &Frame, c: Coord, fill: &str, stroke: &str) -> String {
    let (x, y) = f.px(c);
    circle(x, y, f.hole_r() * 0.72, fill, stroke, 1.5)
}

fn text(x: f64, y: f64, s: &str, size: f64, color: &str, weight: &str) -> String {
    format!(
        "<text x=\"{x:.1}\" y=\"{y:.1}\" font-size=\"{size:.1}\" fill=\"{color}\" font-weight=\"{weight}\" text-anchor=\"middle\" stroke=\"#fafafa\" stroke-width=\"3\" paint-order=\"stroke\">{s}</text>"
    )
}

fn line(x1: f64, y1: f64, x2: f64, y2: f64, color: &str, width: f64, dash: bool) -> String {
    let d = if dash {
        " stroke-dasharray=\"6 5\""
    } else {
        ""
    };
    format!(
        "<line x1=\"{x1:.1}\" y1=\"{y1:.1}\" x2=\"{x2:.1}\" y2=\"{y2:.1}\" stroke=\"{color}\" stroke-width=\"{width:.1}\"{d}/>"
    )
}

fn head(x: f64, y: f64, angle: f64, len: f64, color: &str) -> String {
    let spread = 0.42;
    let (a1, a2) = (angle + spread, angle - spread);
    let p1x = x - len * a1.cos();
    let p1y = y - len * a1.sin();
    let p2x = x - len * a2.cos();
    let p2y = y - len * a2.sin();
    format!(
        "<polygon points=\"{x:.1},{y:.1} {p1x:.1},{p1y:.1} {p2x:.1},{p2y:.1}\" fill=\"{color}\"/>"
    )
}

fn arrow_px(x1: f64, y1: f64, x2: f64, y2: f64, color: &str, width: f64) -> String {
    let angle = (y2 - y1).atan2(x2 - x1);
    let hl = 8.0 + 2.0 * width;
    let ex = x2 - hl * angle.cos();
    let ey = y2 - hl * angle.sin();
    let shaft = line(x1, y1, ex, ey, color, width, false);
    format!("{shaft}{}", head(x2, y2, angle, hl, color))
}

fn hop(f: &Frame, from: Coord, to: Coord, color: &str, width: f64) -> String {
    let (x1, y1) = f.px(from);
    let (x2, y2) = f.px(to);
    let (dx, dy) = (x2 - x1, y2 - y1);
    let len = (dx * dx + dy * dy).sqrt();
    let (nx, ny) = (dy / len, -dx / len);
    let bow = 0.55 * f.scale;
    let (cx, cy) = (0.5 * (x1 + x2) + nx * bow, 0.5 * (y1 + y2) + ny * bow);

    let (sx, sy) = {
        let (ux, uy) = ((cx - x1) / len, (cy - y1) / len);
        (x1 + ux * (f.hole_r() + 2.0), y1 + uy * (f.hole_r() + 2.0))
    };
    let angle = (y2 - cy).atan2(x2 - cx);
    let hl = 10.0;
    let (ex, ey) = (
        x2 - (hl + f.hole_r() * 0.4) * angle.cos(),
        y2 - (hl + f.hole_r() * 0.4) * angle.sin(),
    );
    let path = format!(
        "<path d=\"M {sx:.1} {sy:.1} Q {cx:.1} {cy:.1} {ex:.1} {ey:.1}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"{width:.1}\"/>"
    );
    format!("{path}{}", head(x2, y2, angle, hl, color))
}

fn centroid(f: &Frame, coords: &[Coord]) -> (f64, f64) {
    let n = coords.len() as f64;
    let (sx, sy) = coords.iter().fold((0.0, 0.0), |(ax, ay), &c| {
        let (x, y) = f.px(c);
        (ax + x, ay + y)
    });
    (sx / n, sy / n)
}

fn camp_label(f: &Frame, camp: u32, coords: &[Coord], color: &str) -> String {
    let (x, y) = centroid(f, coords);
    text(
        x,
        y + 0.3 * f.scale,
        &format!("C{}", SUBS[camp as usize]),
        0.95 * f.scale,
        color,
        "700",
    )
}

/// The six directions from the origin, labelled with their index and offset.
pub fn directions() -> String {
    let origin = Coord::ORIGIN;
    let mut coords = vec![origin];
    for d in Dir::ALL {
        coords.push(origin.neighbour(d));
    }
    let f = Frame::new(&coords, 30.0, 82.0);
    let mut b = String::new();
    b.push_str(&hole(&f, origin, FAINT, MUTED));
    for (i, d) in Dir::ALL.iter().enumerate() {
        let n = origin.neighbour(*d);
        b.push_str(&hole(&f, n, "#ffffff", MUTED));
        let (ox, oy) = f.px(origin);
        let (nx, ny) = f.px(n);
        let len = ((nx - ox) * (nx - ox) + (ny - oy) * (ny - oy)).sqrt();
        let (ux, uy) = ((nx - ox) / len, (ny - oy) / len);
        b.push_str(&arrow_px(
            ox + ux * (f.hole_r() + 3.0),
            oy + uy * (f.hole_r() + 3.0),
            nx - ux * (f.hole_r() + 7.0),
            ny - uy * (f.hole_r() + 7.0),
            INK,
            2.0,
        ));
        let label = format!("d{} = ({},{})", SUBS[i], d.dq(), d.dr());
        b.push_str(&text(
            nx + ux * (f.hole_r() + 22.0),
            ny + uy * (f.hole_r() + 22.0) + 4.0,
            &label,
            0.52 * f.scale,
            INK,
            "600",
        ));
    }
    wrap(&f, &b)
}

/// The central hexagon, one shade per ring of radius.
pub fn hexagon() -> String {
    const RING: [&str; 5] = ["#1e88e5", "#42a5f5", "#64b5f6", "#90caf9", "#bbdefb"];
    let coords = hexes();
    let f = Frame::new(&coords, 30.0, 26.0);
    let mut b = String::new();
    for &c in &coords {
        let k = (c.q.abs().max(c.r.abs()).max((c.q + c.r).abs())) as usize;
        let (x, y) = f.px(c);
        b.push_str(&circle(x, y, f.hole_r(), RING[k], "#ffffff", 1.2));
    }
    wrap(&f, &b)
}

/// The whole star, regions coloured and camps labelled.
pub fn board_regions() -> String {
    let holes = all_holes();
    let f = Frame::new(&holes, 20.0, 30.0);
    let mut b = String::new();
    for &c in &holes {
        match camp_of(c) {
            Some(i) => b.push_str(&circle(
                f.px(c).0,
                f.px(c).1,
                f.hole_r(),
                CAMP_COLORS[i as usize],
                "#ffffff",
                1.2,
            )),
            None => b.push_str(&hole(&f, c, FAINT, MUTED)),
        }
    }
    for i in 0..6u32 {
        b.push_str(&camp_label(&f, i, &camp_holes(i), INK));
    }
    wrap(&f, &b)
}

/// The board with camp 0's contact pairs against the hexagon.
pub fn contact_pairs() -> String {
    let holes = all_holes();
    let f = Frame::new(&holes, 20.0, 30.0);
    let mut b = String::new();
    for &c in &holes {
        let fill = if in_base_camp(c) {
            RED_TINT
        } else if in_hex(c) {
            FAINT
        } else {
            "#f5f5f5"
        };
        let stroke = if in_base_camp(c) { RED } else { MUTED };
        b.push_str(&hole(&f, c, fill, stroke));
    }
    let base = camp_holes(0);
    b.push_str(&contact_marks(&f, &base));
    b.push_str(&camp_label(&f, 0, &base, RED_DARK));
    let (hx, hy) = f.px(Coord::ORIGIN);
    b.push_str(&text(
        hx,
        hy + 0.3 * f.scale,
        "H₄",
        0.95 * f.scale,
        MUTED,
        "700",
    ));
    wrap(&f, &b)
}

/// A camp's contact pairs with the hexagon as red links, and its contact
/// holes filled dark and ringed.
fn contact_marks(f: &Frame, camp_holes: &[Coord]) -> String {
    let mut b = String::new();
    for (from, to) in contact_pairs_of(camp_holes) {
        let (x1, y1) = f.px(from);
        let (x2, y2) = f.px(to);
        let len = ((x2 - x1) * (x2 - x1) + (y2 - y1) * (y2 - y1)).sqrt();
        let (ux, uy) = ((x2 - x1) / len, (y2 - y1) / len);
        b.push_str(&line(
            x1 + ux * (f.hole_r() + 2.0),
            y1 + uy * (f.hole_r() + 2.0),
            x2 - ux * (f.hole_r() + 2.0),
            y2 - uy * (f.hole_r() + 2.0),
            RED,
            2.5,
            false,
        ));
    }
    for &c in camp_holes {
        if hex_contacts(c) > 0 {
            b.push_str(&hole(f, c, RED, RED_DARK));
            b.push_str(&ring(f, c, RED_DARK));
        }
    }
    b
}

fn camp_panel(camp_holes: &[Coord]) -> String {
    let mut region = hexes();
    region.extend(camp_holes);
    let f = Frame::new(&region, 19.0, 26.0);
    let mut b = String::new();
    for c in hexes() {
        b.push_str(&hole(&f, c, FAINT, MUTED));
    }
    for &c in camp_holes {
        b.push_str(&hole(&f, c, RED_TINT, RED));
    }
    b.push_str(&contact_marks(&f, camp_holes));
    wrap(&f, &b)
}

/// The correct outward-pointing camp 0.
pub fn proper_camp() -> String {
    camp_panel(&camp_holes(0))
}

/// The draft's inward-pointing camp, which touches the hexagon at one hole.
pub fn inward_camp() -> String {
    camp_panel(&holes_where(in_inward_camp))
}

/// Camps numbered around the board, with the R arrow and the R³ reflection.
pub fn rotation() -> String {
    let holes = all_holes();
    let f = Frame::new(&holes, 20.0, 32.0);
    let mut b = String::new();
    for &c in &holes {
        match camp_of(c) {
            Some(0) => b.push_str(&hole(&f, c, RED_TINT, RED)),
            Some(3) => b.push_str(&hole(&f, c, GREEN_TINT, GREEN)),
            _ => b.push_str(&hole(&f, c, "#f5f5f5", FAINT_STROKE)),
        }
    }
    for i in 0..6u32 {
        let holes_i = camp_holes(i);
        let color = match i {
            0 => RED_DARK,
            3 => GREEN,
            _ => MUTED,
        };
        b.push_str(&camp_label(&f, i, &holes_i, color));
    }
    let c0 = centroid(&f, &camp_holes(0));
    let c1 = centroid(&f, &camp_holes(1));
    let c3 = centroid(&f, &camp_holes(3));
    let shrink = f.hole_r() + 6.0;
    let len = ((c1.0 - c0.0) * (c1.0 - c0.0) + (c1.1 - c0.1) * (c1.1 - c0.1)).sqrt();
    let (ux, uy) = ((c1.0 - c0.0) / len, (c1.1 - c0.1) / len);
    b.push_str(&arrow_px(
        c0.0 + ux * shrink * 1.4,
        c0.1 + uy * shrink * 1.4,
        c1.0 - ux * shrink * 1.4,
        c1.1 - uy * shrink * 1.4,
        INK,
        2.5,
    ));
    let (mx, my) = (0.5 * (c0.0 + c1.0), 0.5 * (c0.1 + c1.1));
    b.push_str(&text(
        mx - uy * 14.0,
        my + ux * 14.0,
        "R",
        1.0 * f.scale,
        INK,
        "700",
    ));

    b.push_str(&line(c0.0, c0.1, c3.0, c3.1, INK, 2.0, true));
    b.push_str(&text(
        0.5 * (c0.0 + c3.0),
        0.5 * (c0.1 + c3.1) + 0.4 * f.scale,
        "R³ = −id",
        0.8 * f.scale,
        INK,
        "700",
    ));
    wrap(&f, &b)
}

/// Steps from a piece: legal onto empty neighbours, not onto occupied ones.
pub fn steps() -> String {
    let origin = Coord::ORIGIN;
    let mut coords = vec![origin];
    for d in Dir::ALL {
        coords.push(origin.neighbour(d));
    }
    let f = Frame::new(&coords, 30.0, 56.0);
    let mut b = String::new();
    b.push_str(&hole(&f, origin, FAINT, MUTED));
    for d in Dir::ALL {
        b.push_str(&hole(&f, origin.neighbour(d), "#ffffff", MUTED));
    }
    b.push_str(&piece(&f, origin, BLOCKER, INK));
    b.push_str(&piece(&f, origin.neighbour(Dir::E), BLOCKER, INK));

    let (ox, oy) = f.px(origin);
    for d in Dir::ALL {
        let n = origin.neighbour(d);
        let (nx, ny) = f.px(n);
        let len = ((nx - ox) * (nx - ox) + (ny - oy) * (ny - oy)).sqrt();
        let (ux, uy) = ((nx - ox) / len, (ny - oy) / len);
        if d == Dir::E {
            let stop = f.hole_r() + 16.0;
            b.push_str(&arrow_px(
                ox + ux * (f.hole_r() + 3.0),
                oy + uy * (f.hole_r() + 3.0),
                nx - ux * stop,
                ny - uy * stop,
                RED,
                2.0,
            ));
            b.push_str(&text(
                nx - ux * (f.hole_r() + 2.0),
                ny - uy * (f.hole_r() + 2.0) + 5.0,
                "✕",
                0.7 * f.scale,
                RED,
                "700",
            ));
        } else {
            b.push_str(&arrow_px(
                ox + ux * (f.hole_r() + 3.0),
                oy + uy * (f.hole_r() + 3.0),
                nx - ux * (f.hole_r() + 7.0),
                ny - uy * (f.hole_r() + 7.0),
                GREEN,
                2.0,
            ));
        }
    }
    wrap(&f, &b)
}

/// One jump: over the occupied midpoint, onto the empty landing hole.
pub fn jump() -> String {
    let (o, m, t) = (Coord::new(0, 0), Coord::new(1, 0), Coord::new(2, 0));
    let coords = [o, m, t];
    let f = Frame::new(&coords, 36.0, 34.0);
    let mut b = String::new();
    for c in coords {
        b.push_str(&hole(&f, c, "#ffffff", MUTED));
    }
    b.push_str(&piece(&f, o, RED, RED_DARK));
    b.push_str(&piece(&f, m, BLOCKER, INK));
    b.push_str(&ring(&f, t, GREEN));
    b.push_str(&hop(&f, o, t, RED, 2.5));

    for (c, s) in [(o, "x"), (m, "x+d"), (t, "x+2d")] {
        let (x, y) = f.px(c);
        b.push_str(&text(
            x,
            y + f.hole_r() + 0.55 * f.scale,
            s,
            0.55 * f.scale,
            INK,
            "600",
        ));
    }
    wrap(&f, &b)
}

/// A two-hop route: the same piece chaining jumps around fixed blockers.
pub fn route() -> String {
    let pts = [
        Coord::new(0, 0),
        Coord::new(1, 0),
        Coord::new(2, 0),
        Coord::new(2, 1),
        Coord::new(2, 2),
    ];
    let f = Frame::new(&pts, 36.0, 36.0);
    let mut b = String::new();
    for &c in &pts {
        b.push_str(&hole(&f, c, "#ffffff", MUTED));
    }
    b.push_str(&piece(&f, pts[0], RED, RED_DARK));
    b.push_str(&piece(&f, pts[1], BLOCKER, INK));
    b.push_str(&piece(&f, pts[3], BLOCKER, INK));
    b.push_str(&ring(&f, pts[2], GREEN));
    b.push_str(&ring(&f, pts[4], GREEN));
    b.push_str(&hop(&f, pts[0], pts[2], RED, 2.5));
    b.push_str(&hop(&f, pts[2], pts[4], RED, 2.5));

    for (from, to, n) in [(pts[0], pts[2], "1"), (pts[2], pts[4], "2")] {
        let (x1, y1) = f.px(from);
        let (x2, y2) = f.px(to);
        let (dx, dy) = (x2 - x1, y2 - y1);
        let len = (dx * dx + dy * dy).sqrt();
        let (nx, ny) = (dy / len, -dx / len);
        let bow = 0.55 * f.scale;
        b.push_str(&text(
            0.5 * (x1 + x2) + nx * (bow + 0.35 * f.scale),
            0.5 * (y1 + y2) + ny * (bow + 0.35 * f.scale) + 4.0,
            n,
            0.6 * f.scale,
            RED_DARK,
            "700",
        ));
    }
    wrap(&f, &b)
}

/// Camp 0 start versus the opposite camp 3 target.
pub fn start_target() -> String {
    start_target_inner(false)
}

/// A won position: every hole of the target camp holds the mover's pieces.
pub fn win() -> String {
    start_target_inner(true)
}

fn start_target_inner(won: bool) -> String {
    let holes = all_holes();
    let f = Frame::new(&holes, 20.0, 32.0);
    let mut b = String::new();
    for &c in &holes {
        match camp_of(c) {
            Some(0) => b.push_str(&hole(&f, c, RED_TINT, RED)),
            Some(3) => b.push_str(&hole(&f, c, GREEN_TINT, GREEN)),
            _ => b.push_str(&hole(&f, c, "#f5f5f5", FAINT_STROKE)),
        }
    }
    // Before the game the pieces fill the start camp; once won, the target.
    let (camp, fill, stroke) = if won {
        (3, "#43a047", GREEN)
    } else {
        (0, RED, RED_DARK)
    };
    for c in camp_holes(camp) {
        b.push_str(&piece(&f, c, fill, stroke));
    }
    let c0 = centroid(&f, &camp_holes(0));
    let c3 = centroid(&f, &camp_holes(3));
    if !won {
        b.push_str(&line(c0.0, c0.1, c3.0, c3.1, INK, 2.0, true));
        let (sx, sy) = (c3.0 - c0.0, c3.1 - c0.1);
        let len = (sx * sx + sy * sy).sqrt();
        let (ux, uy) = (sx / len, sy / len);
        b.push_str(&arrow_px(
            0.5 * (c0.0 + c3.0) + ux * 8.0,
            0.5 * (c0.1 + c3.1) + uy * 8.0,
            c3.0 - ux * (f.hole_r() + 8.0),
            c3.1 - uy * (f.hole_r() + 8.0),
            INK,
            2.5,
        ));
        b.push_str(&text(
            c0.0,
            c0.1 - 1.1 * f.scale,
            "start",
            0.75 * f.scale,
            RED_DARK,
            "700",
        ));
        b.push_str(&text(
            c3.0,
            c3.1 - 1.1 * f.scale,
            "target",
            0.75 * f.scale,
            GREEN,
            "700",
        ));
    }
    wrap(&f, &b)
}
