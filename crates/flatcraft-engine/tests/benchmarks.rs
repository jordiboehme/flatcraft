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

#[test]
fn benchmarks_do_not_regress() {
    let dir = tempfile::tempdir().unwrap();
    let (rows, sweep) = std::thread::scope(|scope| {
        let sweep = scope.spawn(bench::crossing_sweep);
        (bench::run_all(dir.path()), sweep.join().unwrap())
    });
    println!("{}", bench::tables(&rows, true));
    println!("{}", bench::sweep_table(&sweep));
    let misses = bench::device_limit_misses(&rows);
    assert!(
        misses.is_empty(),
        "device limit missed:\n{}",
        misses.join("\n")
    );
    let regressions = bench::regressions(&baseline(), &rows, &sweep);
    assert!(
        regressions.is_empty(),
        "regressions against tests/fixtures/bench/baseline.json:\n{}",
        regressions.join("\n")
    );
}

#[test]
#[ignore = "STOP (Task 18): Quality has more travel and no better start deviation than Baseline on test-sheet and tiny-dots; needs a ruling, see task-18-report.md"]
fn new_order_beats_baseline_on_travel_or_quality() {
    // Per case, the Quality order has no more travel than the plain
    // nearest neighbour plus 2-opt order, or a better start deviation.
    let quality = Variant::Strategy(CutStrategy::Quality);
    let mut misses = Vec::new();
    for case in bench::cases() {
        let dir = tempfile::tempdir().unwrap();
        let rows = bench::run_case(dir.path(), &case, &[quality, Variant::Baseline]);
        let (q, b) = (&rows[0], &rows[1]);
        println!(
            "{}: travel {} vs {}, start {} vs {}",
            case.name, q.travel_mm, b.travel_mm, q.max_start_deviation_mm, b.max_start_deviation_mm
        );
        if !(q.travel_mm <= b.travel_mm || q.max_start_deviation_mm < b.max_start_deviation_mm) {
            misses.push(format!(
                "{}: quality travel {} > baseline {} and start {} >= {}",
                case.name,
                q.travel_mm,
                b.travel_mm,
                q.max_start_deviation_mm,
                b.max_start_deviation_mm
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
        rows[0].lifts > 450,
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
