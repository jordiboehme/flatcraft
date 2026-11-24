use flatcraft_device::gpgl::*;
use flatcraft_device::*;
use kurbo::Point;
use proptest::prelude::*;

/// The built-in file output profile, with the physical mats of the gpgl.md section 9 test
/// input (304.8 x 304.8 and 304.8 x 609.6 mm), so the golden bytes stay the reference.
fn file_output() -> PlotterProfile {
    let mut p = real_file_output();
    p.media[0].height_mm = 304.8;
    p.media[1].height_mm = 609.6;
    p
}

/// The built-in file output profile as shipped.
fn real_file_output() -> PlotterProfile {
    let d = tempfile::tempdir().unwrap();
    let s = ProfileStore::load(d.path()).unwrap();
    s.plotter("file-output-gpgl-cameo2").unwrap()
}

fn settings() -> CutSettings {
    CutSettings {
        speed: 5,
        force: 10,
        track_enhancing: false,
    }
}

fn path(pts: &[(f64, f64)]) -> EmitPath {
    EmitPath {
        points: pts.iter().map(|&p| Point::from(p)).collect(),
        note: "t".into(),
    }
}

fn frame(cmds: &[&str]) -> Vec<u8> {
    let mut want = vec![0x1b, 0x04];
    for c in cmds {
        want.extend_from_slice(c.as_bytes());
        want.push(0x03);
    }
    want
}

// PROVISIONAL golden (spec 14a): re-check against captures
#[test]
fn square_job_golden() {
    // 10 mm square at (10,10), overcut point included, file-output profile, Mat 12x12,
    // speed 5, force 10, track enhancing off
    let pts = [
        (10., 10.),
        (20., 10.),
        (20., 20.),
        (10., 20.),
        (10., 10.),
        (10.5, 10.),
    ];
    let job = emit_job(
        &[EmitPath {
            points: pts.map(Point::from).to_vec(),
            note: "t".into(),
        }],
        &file_output(),
        "mat-12x12",
        &CutSettings {
            speed: 5,
            force: 10,
            track_enhancing: false,
        },
    )
    .unwrap();
    let want = frame(&[
        "!5",
        "FX10",
        "FC18",
        "FY1",
        "FN0",
        "TB50,0",
        "\\0,0",
        "Z6096,6096",
        "L0",
        "FE0,0",
        "FF0,0,0",
        "M200,200",
        "D200,400,400,400,400,200,200,200,200,210",
        "H",
    ]);
    assert_eq!(job.bytes, want);
}

#[test]
fn golden_job_equals_gpgl_md_byte_string() {
    // The single byte string from gpgl.md section 9, written out by hand.
    let want: &[u8] = b"\x1b\x04!5\x03FX10\x03FC18\x03FY1\x03FN0\x03TB50,0\x03\\0,0\x03Z6096,6096\x03L0\x03FE0,0\x03FF0,0,0\x03M200,200\x03D200,400,400,400,400,200,200,200,200,210\x03H\x03";
    let job = emit_job(
        &[path(&[
            (10., 10.),
            (20., 10.),
            (20., 20.),
            (10., 20.),
            (10., 10.),
            (10.5, 10.),
        ])],
        &file_output(),
        "mat-12x12",
        &settings(),
    )
    .unwrap();
    assert_eq!(job.bytes, want);
}

#[test]
fn listing_has_one_line_per_command() {
    let job = emit_job(
        &[path(&[(10., 10.), (20., 10.)])],
        &file_output(),
        "mat-12x12",
        &settings(),
    )
    .unwrap();
    assert_eq!(job.listing.len(), 15);
    assert_eq!(job.listing[0].command, "<ESC>\\x04");
    assert_eq!(job.listing[0].note, "init");
    assert_eq!(job.listing[1].command, "!5");
    assert_eq!(job.listing[1].note, "speed 5");
    assert_eq!(job.listing[2].note, "force 10");
    assert_eq!(job.listing[7].command, "\\0,0");
    assert_eq!(job.listing[12].command, "M200,200");
    assert_eq!(job.listing[12].note, "t");
    assert_eq!(job.listing[13].command, "D200,400");
    assert_eq!(job.listing[13].note, "t");
    assert_eq!(job.listing[14].note, "home");
    assert!(job.listing.iter().all(|l| !l.command.contains('\x03')));
    assert!(job.listing.iter().all(|l| !l.command.contains('\x1b')));
}

#[test]
fn draw_is_chunked_at_16_points() {
    let pts: Vec<(f64, f64)> = (0..40).map(|i| (10.0 + i as f64, 10.0)).collect();
    let job = emit_job(&[path(&pts)], &file_output(), "mat-12x12", &settings()).unwrap();
    let cmds: Vec<&ListingLine> = job.listing.iter().skip(12).collect();
    assert!(cmds[0].command.starts_with('M'));
    let counts: Vec<usize> = cmds[1..cmds.len() - 1]
        .iter()
        .map(|l| {
            assert!(l.command.starts_with('D'));
            l.command[1..].split(',').count() / 2
        })
        .collect();
    assert_eq!(counts, vec![16, 16, 7]);
    assert_eq!(cmds.last().unwrap().command, "H");
}

#[test]
fn grid_origin_shifts_device_coordinates() {
    let p = real_file_output();
    let mut m = p.media[0].clone();
    m.grid.origin_x_mm = 10.0;
    m.grid.origin_y_mm = 12.0;
    assert_eq!(
        quantize::to_device(Point::new(0.0, 0.0), &p, &m),
        DevPoint { y: 240, x: 200 }
    );
}

#[test]
fn grid_origin_is_added_before_mirror() {
    let mut p = real_file_output();
    p.axis.mirror_x = true;
    let mut m = p.media[0].clone();
    m.grid.origin_x_mm = 10.0;
    // (width - (x + origin)) * 20 = (304.8 - 15) * 20
    assert_eq!(quantize::to_device(Point::new(5.0, 0.0), &p, &m).x, 5796);
}

#[test]
fn grid_origin_is_part_of_the_range_check() {
    let mut p = real_file_output();
    p.media[0].grid.origin_x_mm = 10.0;
    // x = 295 + 10 = 305 > 304.8 physical
    let err = emit_job(
        &[path(&[(10., 10.), (295., 10.)])],
        &p,
        "mat-12x12",
        &settings(),
    )
    .unwrap_err();
    assert!(
        matches!(err, EmitError::CoordinateOutOfRange { axis: "x", .. }),
        "{err:?}"
    );
    let msg = err.to_string();
    assert!(
        msg.contains("x = 305 mm (295 mm on the grid + 10 mm grid origin)"),
        "{msg}"
    );
    assert!(
        msg.contains("lies outside the mat (0 to 304.8 mm)"),
        "{msg}"
    );
}

#[test]
fn range_error_rounds_millimetres() {
    let mut p = real_file_output();
    p.media[0].grid.origin_x_mm = 0.1;
    let err = emit_job(
        &[path(&[(10., 10.), (304.8, 10.)])],
        &p,
        "mat-12x12",
        &settings(),
    )
    .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("x = 304.9 mm (304.8 mm on the grid + 0.1 mm grid origin)"),
        "{msg}"
    );
}

#[test]
fn work_area_uses_the_physical_size() {
    let p = real_file_output();
    let job = emit_job(
        &[path(&[(10., 10.), (20., 10.)])],
        &p,
        "mat-12x12",
        &settings(),
    )
    .unwrap();
    assert!(
        job.bytes
            .windows(b"Z6604,6096\x03".len())
            .any(|w| w == b"Z6604,6096\x03")
    );
}

#[test]
fn quantize_rounds_absolute() {
    let p = file_output();
    let m = p.media[0].clone();
    assert_eq!(
        quantize::to_device(Point::new(0.024, 0.026), &p, &m),
        DevPoint { y: 1, x: 0 }
    );
    let mut x = 0.0;
    let mut last = DevPoint { y: 0, x: 0 };
    for _ in 0..100 {
        x += 0.03;
        last = quantize::to_device(Point::new(x, 0.0), &p, &m);
    }
    assert_eq!(last.x, (3.0f64 * 20.0).round() as i32);
}

#[test]
fn quantize_rounds_ties_away_from_zero() {
    let p = file_output();
    let m = p.media[0].clone();
    // gpgl.md section 2: 10.025 mm * 20.0 = 200.5, sent as 201
    assert_eq!(quantize::to_device(Point::new(10.025, 0.0), &p, &m).x, 201);
}

#[test]
fn quantize_applies_mirror_and_scale() {
    let mut p = file_output();
    let m = p
        .media
        .iter()
        .find(|m| m.id == "mat-12x12")
        .unwrap()
        .clone();
    p.axis.mirror_x = true;
    assert_eq!(
        quantize::to_device(Point::new(10.0, 0.0), &p, &m).x,
        (294.8f64 * 20.0).round() as i32
    );
    p.axis.mirror_x = false;
    p.scale_y = 1.01;
    assert_eq!(quantize::to_device(Point::new(0.0, 100.0), &p, &m).y, 2020);
}

#[test]
fn tall_mat_keeps_y_and_x_apart() {
    let p = file_output();
    let s = settings();
    let job = emit_job(&[path(&[(10., 400.), (20., 400.)])], &p, "mat-12x24", &s).unwrap();
    assert!(
        job.bytes
            .windows(b"Z12192,6096\x03".len())
            .any(|w| w == b"Z12192,6096\x03")
    );
    assert!(
        job.bytes
            .windows(b"M8000,200\x03".len())
            .any(|w| w == b"M8000,200\x03")
    );
    let err = emit_job(&[path(&[(400., 10.), (20., 10.)])], &p, "mat-12x24", &s).unwrap_err();
    assert!(
        matches!(err, EmitError::CoordinateOutOfRange { axis: "x", .. }),
        "{err:?}"
    );
    let msg = err.to_string();
    assert!(msg.contains("x = 400 mm"), "{msg}");
    assert!(msg.contains("304.8 mm"), "{msg}");
    assert!(msg.contains("device units 8000"), "{msg}");
}

#[test]
fn out_of_range_coordinates_are_refused() {
    let p = file_output();
    for bad in [
        (-0.1, 10.),
        (10., -0.1),
        (304.9, 10.),
        (10., 304.9),
        (f64::NAN, 10.),
        (10., f64::INFINITY),
    ] {
        let err = emit_job(&[path(&[(10., 10.), bad])], &p, "mat-12x12", &settings()).unwrap_err();
        assert!(
            matches!(err, EmitError::CoordinateOutOfRange { .. }),
            "{bad:?}: {err:?}"
        );
    }
    // Both ends are included.
    emit_job(
        &[path(&[(0., 0.), (304.8, 304.8)])],
        &p,
        "mat-12x12",
        &settings(),
    )
    .unwrap();
}

#[test]
fn out_of_range_force_is_refused() {
    let err = emit_job(
        &[path(&[(10., 10.), (20., 10.)])],
        &file_output(),
        "mat-12x12",
        &CutSettings {
            speed: 5,
            force: 34,
            track_enhancing: false,
        },
    )
    .unwrap_err();
    assert!(
        matches!(err, EmitError::OutOfRange { field: "force", .. }),
        "{err:?}"
    );
}

#[test]
fn out_of_range_speed_is_refused() {
    for speed in [0, 11] {
        let err = emit_job(
            &[path(&[(10., 10.), (20., 10.)])],
            &file_output(),
            "mat-12x12",
            &CutSettings {
                speed,
                force: 10,
                track_enhancing: false,
            },
        )
        .unwrap_err();
        assert!(
            matches!(err, EmitError::OutOfRange { field: "speed", .. }),
            "{err:?}"
        );
    }
}

#[test]
fn wide_profile_ranges_do_not_widen_protocol_limits() {
    let mut p = file_output();
    p.force_range = [0, 50];
    p.speed_range = [0, 30];
    for (speed, force, field) in [(5, 34, "force"), (5, 0, "force"), (11, 10, "speed")] {
        let err = emit_job(
            &[path(&[(10., 10.), (20., 10.)])],
            &p,
            "mat-12x12",
            &CutSettings {
                speed,
                force,
                track_enhancing: false,
            },
        )
        .unwrap_err();
        assert!(
            matches!(err, EmitError::OutOfRange { field: f, .. } if f == field),
            "{err:?}"
        );
    }
}

#[test]
fn out_of_range_fc_is_refused() {
    let mut p = file_output();
    p.knife_compensation = KnifeCompensation::Firmware { fc: 101 };
    let err = emit_job(
        &[path(&[(10., 10.), (20., 10.)])],
        &p,
        "mat-12x12",
        &settings(),
    )
    .unwrap_err();
    assert!(
        matches!(
            err,
            EmitError::OutOfRange {
                field: "fc",
                value: 101,
                ..
            }
        ),
        "{err:?}"
    );
}

#[test]
fn host_mode_is_refused_in_m1() {
    let mut p = file_output();
    p.knife_compensation = KnifeCompensation::Host {
        offset_mm: 0.25,
        pivot_threshold_deg: 10.0,
    };
    let err = emit_job(
        &[path(&[(10., 10.), (20., 10.)])],
        &p,
        "mat-12x12",
        &settings(),
    )
    .unwrap_err();
    assert_eq!(err, EmitError::HostCompensationNotYet);
}

#[test]
fn none_compensation_emits_fc0() {
    let mut p = file_output();
    p.knife_compensation = KnifeCompensation::None;
    let job = emit_job(
        &[path(&[(10., 10.), (20., 10.)])],
        &p,
        "mat-12x12",
        &settings(),
    )
    .unwrap();
    assert!(job.bytes.windows(4).any(|w| w == b"FC0\x03"));
}

#[test]
fn track_enhancing_on_sends_fy0() {
    let job = emit_job(
        &[path(&[(10., 10.), (20., 10.)])],
        &file_output(),
        "mat-12x12",
        &CutSettings {
            speed: 5,
            force: 20,
            track_enhancing: true,
        },
    )
    .unwrap();
    assert!(job.bytes.windows(4).any(|w| w == b"FY0\x03"));
    assert!(!job.bytes.windows(4).any(|w| w == b"FY1\x03"));
}

#[test]
fn unknown_media_is_refused() {
    let err = emit_job(
        &[path(&[(10., 10.), (20., 10.)])],
        &file_output(),
        "no-such-mat",
        &settings(),
    )
    .unwrap_err();
    assert!(matches!(err, EmitError::UnknownMedia { .. }), "{err:?}");
}

#[test]
fn cameo3_dialect_is_refused() {
    let mut p = file_output();
    p.protocol = Protocol::Gpgl {
        dialect: GpglDialect::Cameo3Plus,
    };
    let err = emit_job(
        &[path(&[(10., 10.), (20., 10.)])],
        &p,
        "mat-12x12",
        &settings(),
    )
    .unwrap_err();
    assert_eq!(err, EmitError::DialectNotSupported);
    assert_eq!(
        GpglCommand::Home.to_bytes(GpglDialect::Cameo3Plus),
        Err(EmitError::DialectNotSupported)
    );
}

#[test]
fn short_paths_are_skipped_and_empty_jobs_refused() {
    let p = file_output();
    let s = settings();
    assert_eq!(emit_job(&[], &p, "mat-12x12", &s), Err(EmitError::EmptyJob));
    assert_eq!(
        emit_job(&[path(&[]), path(&[(10., 10.)])], &p, "mat-12x12", &s),
        Err(EmitError::EmptyJob)
    );
    let job = emit_job(
        &[path(&[(1., 1.)]), path(&[(10., 10.), (20., 10.)])],
        &p,
        "mat-12x12",
        &s,
    )
    .unwrap();
    assert_eq!(job.listing.len(), 15);
}

#[test]
fn error_messages_are_readable() {
    let msgs = [
        EmitError::EmptyJob.to_string(),
        EmitError::HostCompensationNotYet.to_string(),
        EmitError::DialectNotSupported.to_string(),
        EmitError::OutOfRange {
            field: "force",
            value: 34,
            min: 1,
            max: 33,
        }
        .to_string(),
    ];
    for m in msgs {
        assert!(!m.is_empty());
        assert!(!m.contains('\u{2014}'), "{m}");
    }
}

#[test]
fn check_safe_rejects_fb_and_tb72() {
    for bad in [
        &b"FB1,2\x03H\x03"[..],
        b"TB72,1\x03H\x03",
        b"\x1b\x04FB1\x03H\x03",
        b"\x1b\x04\x1b\x05TB72\x03H\x03",
        b"M1,1\x03FB1",
    ] {
        assert!(
            matches!(check_safe(bad), Err(SafetyError::Forbidden { .. })),
            "{bad:?}"
        );
        assert!(
            matches!(check_safe_fragment(bad), Err(SafetyError::Forbidden { .. })),
            "{bad:?}"
        );
    }
}

// Review fix round 1: inputs a blocklist let through. Each one must be rejected.
const BYPASSES: &[&[u8]] = &[
    b"TB072,1\x03H\x03",
    b"TB 72,1\x03H\x03",
    b"TB+72,1\x03H\x03",
    b"L0FB1\x03H\x03",
    b"M1,1FB1\x03H\x03",
    b"F\x00B1\x03H\x03",
    b"F B1\x03H\x03",
    b"F\x1b\x15B1\x03H\x03",
    b"F\x1bB1\x03H\x03",
    b"\x7fFB1\x03H\x03",
    b"\x80FB1\x03H\x03",
    b"\xc6\xc21\x03H\x03",
    b"H\x1b\x05\x03",
    b"H\x03\x1b\x05",
    b"M1,1\x03H\x03\x1b\x15",
    b"\x1b\x1bFB1\x03H\x03",
    b"\xff\xfe\x00\x03H\x03",
];

#[test]
fn check_safe_rejects_review_bypasses() {
    let accepted: Vec<String> = BYPASSES
        .iter()
        .filter(|bad| check_safe(bad).is_ok())
        .map(|bad| bad.escape_ascii().to_string())
        .collect();
    assert!(accepted.is_empty(), "accepted: {accepted:#?}");
}

#[test]
fn check_safe_accepts_tb50() {
    assert_eq!(check_safe(b"\x1b\x04TB50,0\x03TB50,1\x03H\x03"), Ok(()));
}

#[test]
fn check_safe_requires_home() {
    assert_eq!(check_safe(b""), Err(SafetyError::NotTerminated));
    assert_eq!(check_safe(b"M1,1\x03"), Err(SafetyError::NotTerminated));
    assert!(check_safe(b"H\x03M1,1").is_err());
    assert!(check_safe(b"H").is_err());
    assert_eq!(
        check_safe(b"H\x03\x1b\x05"),
        Err(SafetyError::NotTerminated)
    );
    assert_eq!(check_safe(b"M1,1\x03H\x03"), Ok(()));
    assert_eq!(check_safe_fragment(b"\x1b\x05"), Ok(()));
    assert_eq!(check_safe_fragment(b"M1,1\x03"), Ok(()));
}

#[test]
fn decode_explains_square_job() {
    let job = emit_job(
        &[path(&[
            (10., 10.),
            (20., 10.),
            (20., 20.),
            (10., 20.),
            (10., 10.),
            (10.5, 10.),
        ])],
        &file_output(),
        "mat-12x12",
        &settings(),
    )
    .unwrap();
    let d = decode::decode(&job.bytes);
    assert_eq!(d.len(), job.listing.len());
    let meanings: Vec<&str> = d.iter().map(|c| c.meaning.as_str()).collect();
    for want in ["init", "speed 5", "force 10", "home"] {
        assert!(meanings.contains(&want), "{want} missing in {meanings:?}");
    }
    assert!(d.iter().all(|c| c.meaning != "unknown"), "{d:?}");
    for (c, l) in d.iter().zip(&job.listing) {
        assert_eq!(c.raw, l.command);
    }
}

#[test]
fn decode_marks_unknown_and_forbidden() {
    let d = decode::decode(b"QQ\x03FB1\x03TB72,1\x03");
    assert_eq!(d[0].meaning, "unknown");
    assert!(d[1].meaning.contains("FORBIDDEN"));
    assert!(d[2].meaning.contains("FORBIDDEN"));
    for raw in [&b"TB072,1\x03"[..], b"TB 72,1\x03", b"TB+72,1\x03"] {
        assert!(
            decode::decode(raw)[0].meaning.contains("FORBIDDEN"),
            "{raw:?}"
        );
    }
}

fn arb_paths() -> impl Strategy<Value = Vec<Vec<(f64, f64)>>> {
    prop::collection::vec(
        prop::collection::vec((0.0..=304.8f64, 0.0..=304.8f64), 2..40),
        1..5,
    )
}

proptest! {
    #[test]
    fn every_emitted_job_passes_check_safe(
        paths in arb_paths(),
        speed in 1u8..=10,
        force in 1u8..=33,
        track in any::<bool>(),
    ) {
        let eps: Vec<EmitPath> = paths.iter().map(|p| path(p)).collect();
        let job = emit_job(
            &eps,
            &file_output(),
            "mat-12x12",
            &CutSettings { speed, force, track_enhancing: track },
        ).unwrap();
        prop_assert_eq!(check_safe(&job.bytes), Ok(()));
        prop_assert_eq!(decode::decode(&job.bytes).len(), job.listing.len());
    }

    #[test]
    fn decode_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..256)) {
        let _ = decode::decode(&bytes);
    }

    #[test]
    fn decoded_forbidden_is_always_rejected(
        tokens in prop::collection::vec(
            prop::sample::select(vec![
                &b"\x1b"[..], b"\x04", b"\x05", b"\x15", b"\x03", b"FB1", b"TB72", b"H",
                b"M1,1", b" ", b"\n",
            ]),
            0..24,
        )
    ) {
        let bytes: Vec<u8> = tokens.concat();
        if decode::decode(&bytes).iter().any(|c| c.meaning.contains("FORBIDDEN")) {
            prop_assert!(check_safe_fragment(&bytes).is_err(), "{:?}", bytes);
            prop_assert!(check_safe(&bytes).is_err(), "{:?}", bytes);
        }
    }

    #[test]
    fn check_safe_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..256)) {
        let _ = check_safe(&bytes);
        let _ = check_safe_fragment(&bytes);
    }
}
