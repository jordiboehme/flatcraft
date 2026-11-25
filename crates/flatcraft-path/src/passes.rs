//! Stage 8: overcut and passes.
//!
//! Plan decision (R6): passes on a closed ring are chained laps. The ring
//! runs `passes` times without lifting, then the overcut once. The heading
//! at the end of a lap matches the first segment, so there is no start mark
//! between laps. Reverse never chains: a 180 deg turn with the blade down
//! pushes the blade.

use crate::overcut;
use crate::plan::Toolpath;

/// The passes of the ordered `paths`, with the overcut on closed rings.
///
/// - Closed, no reverse: one toolpath with `laps = passes`. The ring runs
///   `passes` times without lifting, then `overcut_mm` once. With
///   `passes = 1` this is one toolpath with `laps = 1` and the overcut once,
///   the same as before chaining.
/// - Closed, reverse: `passes` lifted laps as in [`separate`].
/// - Open: `passes` toolpaths in a row, each a new start. With
///   `reverse_alternate`, even passes run in reverse.
pub fn chain(
    paths: Vec<Toolpath>,
    passes: u32,
    reverse_alternate: bool,
    overcut_mm: f64,
) -> Vec<Toolpath> {
    debug_assert!(passes >= 1, "plan validates passes >= 1");
    let passes = passes.max(1);
    let mut out = Vec::with_capacity(paths.len());
    for t in paths {
        if t.closed && !reverse_alternate {
            out.push(chained(t, passes, overcut_mm));
        } else {
            out.extend(repeat(t, passes, reverse_alternate, overcut_mm));
        }
    }
    out
}

/// The passes as separate lifted laps (the M1 way): every closed lap gets
/// its own overcut, and with `reverse_alternate` even passes run in reverse
/// (and so does their `lap_range`). Passes are numbered from 1.
pub fn separate(
    paths: Vec<Toolpath>,
    passes: u32,
    reverse_alternate: bool,
    overcut_mm: f64,
) -> Vec<Toolpath> {
    debug_assert!(passes >= 1, "plan validates passes >= 1");
    let passes = passes.max(1);
    let mut out = Vec::with_capacity(paths.len() * passes as usize);
    for t in paths {
        out.extend(repeat(t, passes, reverse_alternate, overcut_mm));
    }
    out
}

/// `t` (a closed ring) with `passes` chained laps and the overcut after the
/// last one. `lap_range` is the first lap.
fn chained(mut t: Toolpath, passes: u32, overcut_mm: f64) -> Toolpath {
    t.lap_range = 0..t.points.len();
    t.points = overcut::apply(&t.points, passes, overcut_mm);
    t.pass = 1;
    t.laps = passes;
    t
}

fn repeat(
    mut t: Toolpath,
    passes: u32,
    reverse_alternate: bool,
    overcut_mm: f64,
) -> impl Iterator<Item = Toolpath> {
    if t.closed {
        t.lap_range = 0..t.points.len();
        t.points = overcut::apply(&t.points, 1, overcut_mm);
    }
    (1..=passes).map(move |pass| {
        let mut copy = t.clone();
        copy.pass = pass;
        copy.laps = 1;
        if reverse_alternate && pass % 2 == 0 {
            copy.points.reverse();
            let (n, r) = (copy.points.len(), &copy.lap_range);
            copy.lap_range = n - r.end..n - r.start;
        }
        copy
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clean::length;
    use crate::plan::PathRef;
    use flatcraft_core::document::ObjectId;
    use kurbo::Point;

    fn tp(id: u64) -> Toolpath {
        Toolpath::new(
            vec![Point::new(id as f64, 0.0), Point::new(id as f64, 5.0)],
            false,
            PathRef {
                object: ObjectId(id),
                path_index: 0,
            },
        )
    }

    fn square() -> Toolpath {
        Toolpath::new(
            [
                (0.0, 0.0),
                (10.0, 0.0),
                (10.0, 10.0),
                (0.0, 10.0),
                (0.0, 0.0),
            ]
            .map(Point::from)
            .to_vec(),
            true,
            PathRef {
                object: ObjectId(7),
                path_index: 0,
            },
        )
    }

    #[test]
    fn passes_repeat_per_path_with_reverse() {
        // Open paths never chain: each pass is its own toolpath, even ones
        // reversed, under both `chain` and `separate`.
        for out in [
            chain(vec![tp(1), tp(2)], 2, true, 0.5),
            separate(vec![tp(1), tp(2)], 2, true, 0.5),
        ] {
            let got: Vec<(u64, u32, u32, f64)> = out
                .iter()
                .map(|t| (t.source.object.0, t.pass, t.laps, t.points[0].y))
                .collect();
            assert_eq!(
                got,
                vec![
                    (1, 1, 1, 0.0),
                    (1, 2, 1, 5.0),
                    (2, 1, 1, 0.0),
                    (2, 2, 1, 5.0)
                ]
            );
        }
    }

    #[test]
    fn closed_passes_chain_without_lifting() {
        let out = chain(vec![square()], 3, false, 0.5);
        assert_eq!(out.len(), 1);
        let t = &out[0];
        assert_eq!((t.pass, t.laps), (1, 3));
        assert!(
            (length(&t.points) - 120.5).abs() < 1e-9,
            "{}",
            length(&t.points)
        );
        // No repeated point at the joins between laps.
        assert!(t.points.windows(2).all(|w| w[0] != w[1]));
        assert_eq!(*t.points.last().unwrap(), Point::new(0.5, 0.0));
        // The first lap, closed, without the other laps and the overcut.
        assert_eq!(t.lap(), &square().points[..]);
    }

    #[test]
    fn closed_passes_with_reverse_are_separate_laps() {
        let out = chain(vec![square()], 3, true, 0.5);
        assert_eq!(out.len(), 3);
        for t in &out {
            assert_eq!(t.laps, 1);
            assert!((length(&t.points) - 40.5).abs() < 1e-9);
            assert!((length(t.lap()) - 40.0).abs() < 1e-9);
            assert_eq!(t.lap().first(), t.lap().last());
        }
        assert_eq!(out[1].points[0], *out[0].points.last().unwrap());
        assert_eq!(*out[1].points.last().unwrap(), Point::new(0.0, 0.0));
    }

    #[test]
    fn separate_keeps_closed_laps_apart() {
        let out = separate(vec![square()], 3, false, 0.5);
        assert_eq!(out.len(), 3);
        assert_eq!(out.iter().map(|t| t.pass).collect::<Vec<_>>(), [1, 2, 3]);
        for t in &out {
            assert_eq!(t.laps, 1);
            assert!((length(&t.points) - 40.5).abs() < 1e-9);
            assert_eq!(t.points[0], Point::new(0.0, 0.0));
        }
    }

    #[test]
    fn open_passes_without_reverse_restart_at_the_start() {
        let out = chain(vec![tp(1)], 3, false, 0.5);
        assert_eq!(out.len(), 3);
        assert!(out.iter().all(|t| t.points == tp(1).points && t.laps == 1));
        assert_eq!(
            out.iter().map(|t| t.pass).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
    }

    #[test]
    fn open_passes_with_reverse_alternate() {
        let out = chain(vec![tp(1)], 3, true, 0.5);
        assert_eq!(out.len(), 3);
        let ys: Vec<(f64, f64)> = out
            .iter()
            .map(|t| (t.points[0].y, t.points.last().unwrap().y))
            .collect();
        assert_eq!(ys, vec![(0.0, 5.0), (5.0, 0.0), (0.0, 5.0)]);
        // No overcut on open paths.
        assert!(out.iter().all(|t| length(&t.points) == 5.0));
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "passes >= 1")]
    fn zero_passes_is_a_bug_in_debug() {
        chain(vec![tp(1)], 0, true, 0.0);
    }

    #[test]
    #[cfg(not(debug_assertions))]
    fn zero_passes_counts_as_one() {
        assert_eq!(chain(vec![tp(1)], 0, true, 0.0).len(), 1);
    }
}
