//! Stage 7: repeat each path for multiple passes.

use crate::plan::Toolpath;

/// Repeat each toolpath `passes` times, in a row. Passes are numbered from 1.
/// With `reverse_alternate`, even passes run in reverse.
pub fn expand(paths: Vec<Toolpath>, passes: u32, reverse_alternate: bool) -> Vec<Toolpath> {
    let mut out = Vec::with_capacity(paths.len() * passes.max(1) as usize);
    for t in paths {
        for pass in 1..=passes.max(1) {
            let mut copy = t.clone();
            copy.pass = pass;
            if reverse_alternate && pass % 2 == 0 {
                copy.points.reverse();
            }
            out.push(copy);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::PathRef;
    use flatcraft_core::document::ObjectId;
    use kurbo::Point;

    fn tp(id: u64) -> Toolpath {
        Toolpath {
            points: vec![Point::new(id as f64, 0.0), Point::new(id as f64, 5.0)],
            closed: false,
            source: PathRef {
                object: ObjectId(id),
                path_index: 0,
            },
            pass: 1,
        }
    }

    #[test]
    fn passes_repeat_per_path_with_reverse() {
        let out = expand(vec![tp(1), tp(2)], 2, true);
        let got: Vec<(u64, u32, f64)> = out
            .iter()
            .map(|t| (t.source.object.0, t.pass, t.points[0].y))
            .collect();
        assert_eq!(
            got,
            vec![(1, 1, 0.0), (1, 2, 5.0), (2, 1, 0.0), (2, 2, 5.0)]
        );
    }

    #[test]
    fn passes_without_reverse_repeat_identically() {
        let out = expand(vec![tp(1)], 3, false);
        assert_eq!(out.len(), 3);
        assert!(out.iter().all(|t| t.points[0].y == 0.0));
        assert_eq!(
            out.iter().map(|t| t.pass).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
    }

    #[test]
    fn zero_passes_counts_as_one() {
        assert_eq!(expand(vec![tp(1)], 0, true).len(), 1);
    }
}
