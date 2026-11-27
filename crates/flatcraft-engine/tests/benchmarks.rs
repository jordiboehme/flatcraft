//! Task 18: the quality benchmark suite against its committed baseline
//! (`tests/fixtures/bench/baseline.json`, see `flatcraft_engine::bench`
//! and `docs/quality/benchmarks.md`). `cargo xtask bench-quality --write`
//! writes the baseline and the page again.
//!
//! nextest runs every test in its own process, so only
//! `benchmarks_do_not_regress` runs the whole suite; the others run the
//! cases and variants they need.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use flatcraft_core::document::CutStrategy;
use flatcraft_engine::bench::{self, Baseline, Variant};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn baseline() -> Baseline {
    let path = root().join("tests/fixtures/bench/baseline.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "{}: {e} (run cargo xtask bench-quality --write)",
            path.display()
        )
    });
    Baseline::from_json(&text).expect("baseline.json parses")
}

/// The three strategies of every case, on threads.
fn strategy_rows(dir: &std::path::Path) -> Vec<bench::BenchRow> {
    let strategies: Vec<Variant> = Variant::ALL
        .into_iter()
        .filter(|v| v.is_strategy())
        .collect();
    let cases = bench::cases();
    std::thread::scope(|scope| {
        let handles: Vec<_> = cases
            .iter()
            .map(|c| {
                let strategies = &strategies;
                scope.spawn(move || bench::run_case(&dir.join(c.name), c, strategies))
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap())
            .collect()
    })
}

fn check(rows: &[bench::BenchRow], sweep: &bench::CrossingSweep) {
    println!("{}", bench::tables(rows, true));
    println!("{}", bench::sweep_table(sweep));
    let misses = bench::device_limit_misses(rows);
    assert!(
        misses.is_empty(),
        "device limit missed:\n{}",
        misses.join("\n")
    );
    let regressions = bench::regressions(&baseline(), rows, sweep);
    assert!(
        regressions.is_empty(),
        "regressions against tests/fixtures/bench/baseline.json:\n{}",
        regressions.join("\n")
    );
}

#[test]
fn benchmarks_do_not_regress() {
    // The strategy rows (the ones the rule applies to) and the crossing
    // sweep. The comparison rows run in `benchmarks_full_suite`.
    let dir = tempfile::tempdir().unwrap();
    let (rows, sweep) = std::thread::scope(|scope| {
        let sweep = scope.spawn(bench::crossing_sweep);
        (strategy_rows(dir.path()), sweep.join().unwrap())
    });
    check(&rows, &sweep);
}

#[test]
#[ignore = "the whole suite with the comparison variants: cargo xtask bench-quality runs it too"]
fn benchmarks_full_suite() {
    let dir = tempfile::tempdir().unwrap();
    let (rows, sweep) = std::thread::scope(|scope| {
        let sweep = scope.spawn(bench::crossing_sweep);
        (bench::run_all(dir.path()), sweep.join().unwrap())
    });
    check(&rows, &sweep);
}

/// Each case with `variants`, a thread per case, in case order.
fn compare(variants: &[Variant]) -> Vec<Vec<bench::BenchRow>> {
    let dir = tempfile::tempdir().unwrap();
    let cases = bench::cases();
    std::thread::scope(|scope| {
        let handles: Vec<_> = cases
            .iter()
            .map(|c| {
                let dir = dir.path();
                scope.spawn(move || bench::run_case(&dir.join(c.name), c, variants))
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    })
}

#[test]
fn order_improvements_do_not_cost_travel() {
    // R8: Or-opt, the entry DP and the crossing penalty on top of nearest
    // neighbour plus 2-opt never add travel. Fast has them with no turn
    // weight, so its travel is at most the baseline's on every case.
    let mut misses = Vec::new();
    for rows in compare(&[Variant::Strategy(CutStrategy::Fast), Variant::Baseline]) {
        let (f, b) = (&rows[0], &rows[1]);
        println!(
            "{}: fast travel {} vs baseline {}",
            f.case, f.travel_mm, b.travel_mm
        );
        if f.travel_mm > b.travel_mm {
            misses.push(format!(
                "{}: fast travel {} > baseline {}",
                f.case, f.travel_mm, b.travel_mm
            ));
        }
    }
    assert!(misses.is_empty(), "{}", misses.join("\n"));
}

#[test]
fn quality_order_beats_baseline_on_its_objective() {
    // R8: an optimiser must beat the baseline on its own objective, not on
    // one term. Quality's objective (travel + turn weight x entry turn +
    // crossing penalty x crossings, the constants of order::cost) is at
    // most the baseline's on every case, and it never has more crossings.
    let quality = Variant::Strategy(CutStrategy::Quality);
    let all: Vec<bench::BenchRow> = compare(&[quality, Variant::Baseline])
        .into_iter()
        .flatten()
        .collect();
    println!("{}", bench::objective_table(&all));
    let mut misses = Vec::new();
    for pair in all.chunks(2) {
        let (q, b) = (&pair[0], &pair[1]);
        // The margins are 47 mm and more; 1e-6 only keeps float noise out.
        if bench::objective(q) > bench::objective(b) + 1e-6 {
            misses.push(format!(
                "{}: quality objective {:.3} > baseline {:.3}",
                q.case,
                bench::objective(q),
                bench::objective(b)
            ));
        }
        if q.travel_crossings > b.travel_crossings {
            misses.push(format!(
                "{}: quality crossings {} > baseline {}",
                q.case, q.travel_crossings, b.travel_crossings
            ));
        }
    }
    assert!(misses.is_empty(), "{}", misses.join("\n"));
}

#[test]
fn dense_trace_finishes_in_time() {
    // A time budget, not a quality check: the trace, the plan and the
    // simulation of the dense picture with the default strategy stay under
    // 20 s in the CI debug build.
    let dir = tempfile::tempdir().unwrap();
    let case = bench::case(bench::DENSE_TRACE).unwrap();
    let started = Instant::now();
    let rows = bench::run_case(
        dir.path(),
        &case,
        &[Variant::Strategy(CutStrategy::Quality)],
    );
    let took = started.elapsed();
    println!("dense trace: {took:?}, {} lifts", rows[0].lifts);
    assert!(
        rows[0].lifts > 200,
        "{} lifts: the picture traced too few shapes",
        rows[0].lifts
    );
    assert!(took < Duration::from_secs(20), "{took:?}");
}

#[test]
fn rows_are_deterministic() {
    // The compared numbers come out the same on a second run (the
    // variants run on threads of their own). `cargo xtask bench-quality`
    // run twice gave the same baseline.json too.
    let case = bench::case("stickers-with-holes").unwrap();
    let run = || {
        let dir = tempfile::tempdir().unwrap();
        let variants = [Variant::Strategy(CutStrategy::Quality), Variant::Baseline];
        bench::run_case(dir.path(), &case, &variants)
            .into_iter()
            .map(|r| bench::BenchRow { millis: 0, ..r })
            .collect::<Vec<_>>()
    };
    assert_eq!(run(), run());
}

#[test]
fn dense_trace_fixture_is_the_generated_image() {
    let committed = std::fs::read(root().join("tests/fixtures/bench/dense-trace.png")).unwrap();
    assert!(
        committed == bench::dense_trace_png(),
        "tests/fixtures/bench/dense-trace.png differs from bench::dense_trace_png() (run cargo xtask bench-quality --write)"
    );
}

#[test]
fn fixtures_are_small() {
    for name in [
        "small-text.svg",
        "stickers-with-holes.svg",
        "tiny-dots.svg",
        "long-thin.svg",
        "round-holes.svg",
        "dense-trace.png",
    ] {
        let len = std::fs::metadata(root().join("tests/fixtures/bench").join(name))
            .unwrap()
            .len();
        assert!(len < 100 * 1024, "{name}: {len} bytes");
    }
}

#[test]
fn benchmarks_page_matches_the_baseline() {
    // A Windows checkout may have CRLF line ends.
    let page = std::fs::read_to_string(root().join("docs/quality/benchmarks.md"))
        .unwrap()
        .replace("\r\n", "\n");
    let want = bench::markdown(&baseline());
    if let Some((a, b)) = page.lines().zip(want.lines()).find(|(a, b)| a != b) {
        panic!(
            "docs/quality/benchmarks.md differs from the page of baseline.json (run cargo xtask bench-quality --write):\n  file: {a}\n  want: {b}"
        );
    }
    assert!(
        page == want,
        "docs/quality/benchmarks.md is not the page of baseline.json (run cargo xtask bench-quality --write)"
    );
    assert!(!page.contains('\u{2014}') && !page.contains('\u{2013}'));
}

#[test]
fn row_rule_catches_a_dropped_turn_weight_or_crossing_penalty() {
    // The committed rows, doctored: Quality's row with the entry turn the
    // order gets without a turn weight (the Fast row), and with the
    // crossings it gets without the crossing penalty (the Baseline row).
    // Both must count as regressions, on travel staying the same.
    let base = baseline();
    let row = |case: &str, variant: &str| {
        base.rows
            .iter()
            .find(|r| r.case == case && r.variant == variant)
            .unwrap()
            .clone()
    };
    for case in ["test-sheet", "small-text", "dense-trace"] {
        let quality = row(case, "quality");
        let no_turn_weight = bench::BenchRow {
            entry_turn_deg: row(case, "fast").entry_turn_deg,
            ..quality.clone()
        };
        let found = bench::row_regressions(&quality, &no_turn_weight);
        assert!(
            found.iter().any(|r| r.contains("entry_turn_deg")),
            "{case}: {found:?}"
        );
        let no_crossing_penalty = bench::BenchRow {
            travel_crossings: row(case, "baseline").travel_crossings,
            ..quality.clone()
        };
        let found = bench::row_regressions(&quality, &no_crossing_penalty);
        assert!(
            found.iter().any(|r| r.contains("travel_crossings")),
            "{case}: {found:?}"
        );
        assert!(bench::row_regressions(&quality, &quality).is_empty());
    }
}
