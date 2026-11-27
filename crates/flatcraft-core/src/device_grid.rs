//! The app-to-device transform: the one place that maps app millimetres
//! (origin at the mat's printed grid 0,0) to plotter units and back.
//!
//! The emitter (`flatcraft-device`), the knife stage's rounding
//! (`flatcraft-path::quantize`) and the simulator's device mode all use it,
//! so a point the knife stage rounds is exactly the point the plotter gets.

use kurbo::{Point, Size, Vec2};

/// The device units of one plotter on one medium, as the emitter sends
/// them (gpgl.md section 2). In this order: add the grid `origin`, mirror
/// inside the medium (`media_size`) where asked, multiply by `scale` and
/// by `units_per_mm`, and round each absolute coordinate on its own.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeviceGrid {
    pub units_per_mm: f64,
    /// The grid origin of the mat: app 0,0 lies at `origin` in device
    /// millimetres.
    pub origin: Point,
    /// The physical medium, which the mirror flags mirror inside.
    pub media_size: Size,
    pub mirror_x: bool,
    pub mirror_y: bool,
    /// The calibration scale per axis (1.0 means none).
    pub scale: Vec2,
}

impl DeviceGrid {
    /// A grid with no mirror and no scale correction.
    pub fn new(units_per_mm: f64, origin: Point) -> Self {
        Self {
            units_per_mm,
            origin,
            media_size: Size::ZERO,
            mirror_x: false,
            mirror_y: false,
            scale: Vec2::new(1.0, 1.0),
        }
    }

    /// The exact device units of the app point `p`, as (x, y), not rounded.
    pub fn to_units(&self, p: Point) -> Point {
        let mut x = p.x + self.origin.x;
        let mut y = p.y + self.origin.y;
        if self.mirror_x {
            x = self.media_size.width - x;
        }
        if self.mirror_y {
            y = self.media_size.height - y;
        }
        Point::new(
            x * self.scale.x * self.units_per_mm,
            y * self.scale.y * self.units_per_mm,
        )
    }

    /// The device units the plotter gets for `p`: [`Self::to_units`],
    /// each coordinate rounded half away from zero (`f64::round`).
    pub fn rounded_units(&self, p: Point) -> Point {
        let u = self.to_units(p);
        Point::new(u.x.round(), u.y.round())
    }

    /// The app point of the device units `u`: the inverse of
    /// [`Self::to_units`].
    pub fn from_units(&self, u: Point) -> Point {
        let mut x = u.x / self.units_per_mm / self.scale.x;
        let mut y = u.y / self.units_per_mm / self.scale.y;
        if self.mirror_x {
            x = self.media_size.width - x;
        }
        if self.mirror_y {
            y = self.media_size.height - y;
        }
        Point::new(x - self.origin.x, y - self.origin.y)
    }

    /// Where the plotter really puts `p`, in app millimetres: the rounded
    /// units mapped back. A snapped point rounds to the same units again,
    /// so sending it moves it no further.
    pub fn snap(&self, p: Point) -> Point {
        self.from_units(self.rounded_units(p))
    }

    /// The first value that makes the grid unusable: units or scale that
    /// are not positive and finite, or an origin or medium size that is not
    /// finite.
    pub fn invalid_value(&self) -> Option<f64> {
        let positive = |v: f64| v.is_finite() && v > 0.0;
        [
            (self.units_per_mm, positive(self.units_per_mm)),
            (self.scale.x, positive(self.scale.x)),
            (self.scale.y, positive(self.scale.y)),
            (self.origin.x, self.origin.x.is_finite()),
            (self.origin.y, self.origin.y.is_finite()),
            (self.media_size.width, self.media_size.width.is_finite()),
            (self.media_size.height, self.media_size.height.is_finite()),
        ]
        .into_iter()
        .find(|&(_, ok)| !ok)
        .map(|(v, _)| v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calibrated(mirror_x: bool, mirror_y: bool) -> DeviceGrid {
        DeviceGrid {
            units_per_mm: 20.0,
            origin: Point::new(0.37, 0.013),
            media_size: Size::new(304.8, 330.2),
            mirror_x,
            mirror_y,
            scale: Vec2::new(1.002, 0.998),
        }
    }

    /// A snapped point rounds to the same units again: no second rounding
    /// move, with a scale, mirrors and a grid origin.
    #[test]
    fn snap_is_a_round_trip() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        for mx in [false, true] {
            for my in [false, true] {
                let g = calibrated(mx, my);
                for _ in 0..2000 {
                    let p = Point::new(next() * 300.0, next() * 300.0);
                    let s = g.snap(p);
                    assert_eq!(g.rounded_units(s), g.rounded_units(p), "{p:?}");
                    assert_eq!(g.snap(s), s, "{p:?}");
                    let u = g.to_units(s);
                    assert!((u.x - u.x.round()).abs() < 1e-6 && (u.y - u.y.round()).abs() < 1e-6);
                    // The snap moves a point by at most half a unit per axis.
                    let half = 0.5 / 20.0 / 0.998 + 1e-9;
                    assert!((s.x - p.x).abs() <= half && (s.y - p.y).abs() <= half);
                }
            }
        }
    }

    #[test]
    fn plain_grid_rounds_to_twentieths_from_the_origin() {
        let g = DeviceGrid::new(20.0, Point::new(0.3, 0.0));
        assert_eq!(
            g.rounded_units(Point::new(10.01, 10.0)),
            Point::new(206.0, 200.0)
        );
        assert!(
            g.snap(Point::new(10.01, 10.0))
                .distance(Point::new(10.0, 10.0))
                < 1e-12
        );
    }

    #[test]
    fn invalid_values_are_named() {
        let mut g = DeviceGrid::new(20.0, Point::ZERO);
        assert_eq!(g.invalid_value(), None);
        g.origin.y = f64::NAN;
        assert!(g.invalid_value().unwrap().is_nan());
        g = DeviceGrid::new(0.0, Point::ZERO);
        assert_eq!(g.invalid_value(), Some(0.0));
        g = DeviceGrid::new(20.0, Point::ZERO);
        g.scale.x = -1.0;
        assert_eq!(g.invalid_value(), Some(-1.0));
    }
}
