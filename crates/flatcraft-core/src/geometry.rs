//! Cut geometry in millimetres.

use kurbo::{BezPath, Rect, Shape};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// One cuttable path in millimetres.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CutPath {
    #[serde(with = "svg_path")]
    pub path: BezPath,
    pub color: Option<[u8; 3]>,
    /// Id of the nearest enclosing SVG `<g id>`.
    pub label: Option<String>,
}

mod svg_path {
    use super::*;

    pub fn serialize<S: Serializer>(path: &BezPath, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&path.to_svg())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<BezPath, D::Error> {
        let text = String::deserialize(d)?;
        BezPath::from_svg(&text).map_err(serde::de::Error::custom)
    }
}

/// Union of the bounding boxes of all paths, `None` if there are no paths.
pub fn bounds(paths: &[CutPath]) -> Option<Rect> {
    paths
        .iter()
        .filter(|p| !p.path.elements().is_empty())
        .map(|p| p.path.bounding_box())
        .reduce(|a, b| a.union(b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Point;

    fn square(x: f64, y: f64, s: f64) -> CutPath {
        let mut path = BezPath::new();
        path.move_to(Point::new(x, y));
        path.line_to(Point::new(x + s, y));
        path.line_to(Point::new(x + s, y + s));
        path.line_to(Point::new(x, y + s));
        path.close_path();
        CutPath {
            path,
            color: None,
            label: None,
        }
    }

    #[test]
    fn bounds_of_empty_is_none() {
        assert_eq!(bounds(&[]), None);
    }

    #[test]
    fn bounds_unions_paths() {
        let b = bounds(&[square(0.0, 0.0, 10.0), square(20.0, 5.0, 10.0)]).unwrap();
        assert_eq!(b, Rect::new(0.0, 0.0, 30.0, 15.0));
    }

    #[test]
    fn serde_roundtrip_uses_svg_path_string() {
        let mut cp = square(1.5, 2.5, 10.0);
        cp.path.move_to(Point::new(0.0, 0.0));
        cp.path.curve_to(
            Point::new(1.0, 2.0),
            Point::new(3.0, 4.0),
            Point::new(5.0, 6.0),
        );
        cp.color = Some([255, 0, 128]);
        cp.label = Some("body".into());
        let json = serde_json::to_string(&cp).unwrap();
        assert!(json.contains("\"path\":\"M"), "{json}");
        let back: CutPath = serde_json::from_str(&json).unwrap();
        assert_eq!(back, cp);
    }
}
