//! Random layouts for the pipeline property tests (Task 16): star-shaped
//! polygons, nestings of them and open polylines, one item per cell of a
//! 4 x 4 grid on a 12x12 mat. `flatcraft-path/tests/properties.rs` and
//! `flatcraft-sim/tests/properties.rs` both use it.
//!
//! The ranges, and why:
//! - A star has 6 to 20 vertices at angles `(k + j_k) * 360 / n` with
//!   `j_k` in [0, 0.5), so no two neighbours are more than 90 deg apart,
//!   and radii of 0.6 to 1 times its size. That keeps it simple and
//!   star-shaped, with every edge at least 0.6 * cos(45 deg) = 0.42 times
//!   the size from the centre. Its corners turn anything from a few
//!   degrees to sharp spikes.
//! - A nest is 1 to 3 rings, each [`NEST_SCALE`] the size of the one
//!   around it (0.35 < 0.42, so a ring never touches its parent), the
//!   outer one 10 to 28 mm: the innermost ring is at least 1.2 mm. It may
//!   hold an open polyline in its innermost ring (within 0.29 of that
//!   ring's size from the centre).
//! - A free polyline has 2 to 6 points within 25 mm of its cell centre.
//! - Neighbouring points of a polyline are at least [`MIN_STEP`] of its
//!   scale apart (0.06 mm in the smallest ring), so no segment is shorter
//!   than the join tolerance and no line is shorter than the shortest cut:
//!   the plan keeps every input path.

#![allow(dead_code)]

use flatcraft_core::document::ObjectId;
use flatcraft_path::{PathRef, PlacedPath};
use kurbo::{BezPath, Point};
use proptest::collection::vec;
use proptest::prelude::*;

/// The size of one grid cell, mm.
pub const CELL_MM: f64 = 70.0;
/// Cells along each side.
pub const CELLS: usize = 4;
/// Where the first cell starts, mm.
pub const ORIGIN_MM: f64 = 10.0;
/// The least distance of neighbouring polyline points, in unit coordinates.
pub const MIN_STEP: f64 = 0.25;
/// The size of a nested ring against the ring around it.
pub const NEST_SCALE: f64 = 0.35;

#[derive(Debug, Clone)]
pub struct Star {
    pub jitter: Vec<f64>,
    pub radius: Vec<f64>,
}

impl Star {
    /// The vertices around `c`, `size` mm at most from it.
    pub fn points(&self, c: Point, size: f64) -> Vec<Point> {
        let n = self.radius.len();
        (0..n)
            .map(|k| {
                let a = std::f64::consts::TAU * (k as f64 + self.jitter[k]) / n as f64;
                c + size * self.radius[k] * kurbo::Vec2::from_angle(a)
            })
            .collect()
    }
}

pub fn star() -> impl Strategy<Value = Star> {
    (6usize..=20).prop_flat_map(|n| {
        (vec(0.0f64..0.5, n), vec(0.6f64..=1.0, n))
            .prop_map(|(jitter, radius)| Star { jitter, radius })
    })
}

#[derive(Debug, Clone)]
pub enum Item {
    Nest {
        size: f64,
        rings: Vec<Star>,
        /// Points in [-1, 1]^2, scaled to 0.2 of the innermost ring.
        line: Option<Vec<(f64, f64)>>,
    },
    /// Points in [-1, 1]^2, scaled to 25 mm.
    Line(Vec<(f64, f64)>),
}

/// Points in [-1, 1]^2, neighbours at least [`MIN_STEP`] apart.
fn unit_points(n: std::ops::RangeInclusive<usize>) -> impl Strategy<Value = Vec<(f64, f64)>> {
    vec((-1.0f64..=1.0, -1.0f64..=1.0), n).prop_filter("neighbours apart", |pts| {
        pts.windows(2)
            .all(|w| (w[0].0 - w[1].0).hypot(w[0].1 - w[1].1) >= MIN_STEP)
    })
}

pub fn item() -> impl Strategy<Value = Item> {
    prop_oneof![
        3 => (10.0f64..=28.0, vec(star(), 1..=3), proptest::option::of(unit_points(2..=5)))
            .prop_map(|(size, rings, line)| Item::Nest { size, rings, line }),
        1 => unit_points(2..=6).prop_map(Item::Line),
    ]
}

/// 1 to `max` items, each in its own cell.
pub fn layout(max: usize) -> impl Strategy<Value = Vec<(usize, Item)>> {
    proptest::sample::subsequence((0..CELLS * CELLS).collect::<Vec<_>>(), 1..=max).prop_flat_map(
        |cells| {
            let n = cells.len();
            vec(item(), n).prop_map(move |items| cells.iter().copied().zip(items).collect())
        },
    )
}

fn cell_centre(cell: usize) -> Point {
    let (i, j) = (cell % CELLS, cell / CELLS);
    Point::new(
        ORIGIN_MM + CELL_MM * (i as f64 + 0.5),
        ORIGIN_MM + CELL_MM * (j as f64 + 0.5),
    )
}

pub fn polygon(pts: &[Point]) -> BezPath {
    let mut b = BezPath::new();
    b.move_to(pts[0]);
    for &q in &pts[1..] {
        b.line_to(q);
    }
    b.close_path();
    b
}

pub fn polyline(pts: &[Point]) -> BezPath {
    let mut b = BezPath::new();
    b.move_to(pts[0]);
    for &q in &pts[1..] {
        b.line_to(q);
    }
    b
}

/// One input path of a built layout.
#[derive(Debug, Clone)]
pub struct Built {
    pub placed: PlacedPath,
    /// The path as points (closed rings repeat their first point).
    pub points: Vec<Point>,
    pub closed: bool,
    /// The input path that holds this one directly, if any.
    pub parent: Option<PathRef>,
}

/// The input paths of `layout`: item `k` is object `k + 1`, its paths
/// numbered from 0 (outer ring first).
pub fn build(layout: &[(usize, Item)]) -> Vec<Built> {
    let mut out = Vec::new();
    for (k, (cell, item)) in layout.iter().enumerate() {
        let c = cell_centre(*cell);
        let object = ObjectId(k as u64 + 1);
        let mut index = 0;
        let mut next = |parent: Option<PathRef>, pts: Vec<Point>, closed: bool| {
            let source = PathRef {
                object,
                path_index: index,
            };
            index += 1;
            let path = if closed {
                polygon(&pts)
            } else {
                polyline(&pts)
            };
            let mut points = pts;
            if closed {
                points.push(points[0]);
            }
            out.push(Built {
                placed: PlacedPath { path, source },
                points,
                closed,
                parent,
            });
            source
        };
        match item {
            Item::Nest { size, rings, line } => {
                let mut parent = None;
                let mut s = *size;
                for r in rings {
                    parent = Some(next(parent, r.points(c, s), true));
                    s *= NEST_SCALE;
                }
                if let Some(line) = line {
                    let inner = s / NEST_SCALE;
                    let pts = line
                        .iter()
                        .map(|&(x, y)| c + 0.2 * inner * kurbo::Vec2::new(x, y))
                        .collect();
                    next(parent, pts, false);
                }
            }
            Item::Line(line) => {
                let pts = line
                    .iter()
                    .map(|&(x, y)| c + 25.0 * kurbo::Vec2::new(x, y))
                    .collect();
                next(None, pts, false);
            }
        }
    }
    out
}

pub fn placed(built: &[Built]) -> Vec<PlacedPath> {
    built.iter().map(|b| b.placed.clone()).collect()
}
