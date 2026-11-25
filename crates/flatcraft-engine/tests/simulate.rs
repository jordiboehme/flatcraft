use flatcraft_device::ProfileStore;
use flatcraft_device::gpgl::quantize::to_device;
use flatcraft_engine::simulate::device_grid;
use kurbo::Point;

#[test]
fn device_grid_snaps_like_the_emitter_with_a_grid_origin() {
    let dir = tempfile::tempdir().unwrap();
    let store = ProfileStore::load(dir.path()).unwrap();
    let plotter = store.plotter("file-output-gpgl-cameo2").unwrap();
    assert!(!plotter.axis.mirror_x && !plotter.axis.mirror_y);
    assert_eq!((plotter.scale_x, plotter.scale_y), (1.0, 1.0));
    let mut media = plotter.media[0].clone();
    media.grid.origin_x_mm = 3.33;
    media.grid.origin_y_mm = 7.12;
    let grid = device_grid(&plotter, &media);
    assert_eq!(grid.units_per_mm, plotter.units_per_mm);
    // a deterministic spread of points over the mat, including half-step cases
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1u64 << 53) as f64
    };
    let mut points: Vec<Point> = (0..1000)
        .map(|_| Point::new(next() * 290.0, next() * 290.0))
        .collect();
    points.push(Point::new(0.0, 0.0));
    points.push(Point::new(0.025 - 0.03, 0.025 - 0.02));
    for p in points {
        let snapped = grid.snap(p);
        let dev = to_device(p, &plotter, &media);
        let upm = plotter.units_per_mm;
        let x = (snapped.x + media.grid.origin_x_mm) * upm;
        let y = (snapped.y + media.grid.origin_y_mm) * upm;
        assert!(
            (x - f64::from(dev.x)).abs() < 1e-9,
            "{p:?}: {snapped:?} vs {dev:?}"
        );
        assert!(
            (y - f64::from(dev.y)).abs() < 1e-9,
            "{p:?}: {snapped:?} vs {dev:?}"
        );
    }
}
