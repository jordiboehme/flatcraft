//! Object placement transform.

use kurbo::{Affine, Point, Rect};

/// Placement of an object on the mat, in millimetres and degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub center_mm: Point,
    pub scale_x: f64,
    pub scale_y: f64,
    /// Degrees, clockwise on screen (y points down).
    pub rotation_deg: f64,
    /// Mirrors along x, applied after scaling and before rotation.
    pub mirror: bool,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            center_mm: Point::ORIGIN,
            scale_x: 1.0,
            scale_y: 1.0,
            rotation_deg: 0.0,
            mirror: false,
        }
    }
}

impl Transform {
    /// `translate(center) * rotate(rotation) * scale(sx * mirror, sy) * translate(-local center)`.
    pub fn affine(&self, local_bounds: Rect) -> Affine {
        let flip = if self.mirror { -1.0 } else { 1.0 };
        Affine::translate(self.center_mm.to_vec2())
            * Affine::rotate(self.rotation_deg.to_radians())
            * Affine::scale_non_uniform(self.scale_x * flip, self.scale_y)
            * Affine::translate(-local_bounds.center().to_vec2())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_places_center() {
        let t = Transform {
            center_mm: Point::new(50.0, 40.0),
            ..Default::default()
        };
        let a = t.affine(Rect::new(0.0, 0.0, 20.0, 10.0));
        assert_eq!(a * Point::new(10.0, 5.0), Point::new(50.0, 40.0));
    }

    #[test]
    fn mirror_flips_x_only() {
        let t = Transform {
            mirror: true,
            ..Default::default()
        };
        let p = t.affine(Rect::new(0.0, 0.0, 20.0, 10.0)) * Point::new(20.0, 0.0);
        assert!((p.x + 10.0).abs() < 1e-9 && (p.y + 5.0).abs() < 1e-9);
    }

    #[test]
    fn rotate_90_is_clockwise_on_screen() {
        let t = Transform {
            rotation_deg: 90.0,
            ..Default::default()
        };
        let p = t.affine(Rect::new(0.0, 0.0, 20.0, 10.0)) * Point::new(20.0, 5.0);
        assert!(p.x.abs() < 1e-9 && (p.y - 10.0).abs() < 1e-9, "{p:?}");
    }

    #[test]
    fn scale_then_rotate_order() {
        let t = Transform {
            scale_x: 2.0,
            rotation_deg: 90.0,
            ..Default::default()
        };
        let p = t.affine(Rect::new(0.0, 0.0, 20.0, 10.0)) * Point::new(20.0, 5.0);
        assert!(p.x.abs() < 1e-9 && (p.y - 20.0).abs() < 1e-9, "{p:?}");
    }
}
