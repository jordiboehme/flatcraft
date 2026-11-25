//! File Output transport and byte log.

use flatcraft_device::gpgl::*;
use flatcraft_device::*;
use kurbo::Point;

fn file_output() -> PlotterProfile {
    let d = tempfile::tempdir().unwrap();
    let s = ProfileStore::load(d.path()).unwrap();
    s.plotter("file-output-gpgl-cameo2").unwrap()
}

fn square_job() -> EmittedJob {
    let pts = [
        (10.0, 10.0),
        (20.0, 10.0),
        (20.0, 20.0),
        (10.0, 20.0),
        (10.0, 10.0),
    ];
    let path = EmitPath {
        points: pts.iter().map(|&p| Point::from(p)).collect(),
        note: "object 1 'square', path 0, pass 1".into(),
    };
    let settings = CutSettings {
        speed: 5,
        force: 10,
        track_enhancing: false,
    };
    emit_job(&[path], &file_output(), "mat-12x12", &settings).unwrap()
}

fn names(dir: &std::path::Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn only_checked_jobs_reach_a_transport() {
    // FB, or a job without the final H, never becomes a CheckedJob, so no
    // transport can be handed one.
    for bad in [&b"FB1\x03H\x03"[..], &b"\x1b\x04!5\x03"[..]] {
        let job = EmittedJob {
            bytes: bad.to_vec(),
            listing: vec![],
        };
        assert!(CheckedJob::check(job).is_err());
    }
    let e = CheckedJob::check(EmittedJob {
        bytes: b"\x1b\x04!5\x03".to_vec(),
        listing: vec![],
    })
    .unwrap_err();
    assert_eq!(e, SafetyError::NotTerminated);
    let good = CheckedJob::check(square_job()).unwrap();
    assert_eq!(good.bytes, square_job().bytes);
}

#[test]
fn file_transport_writes_three_files_and_suffixes() {
    let dir = tempfile::tempdir().unwrap();
    let job = square_job();
    let mut log = ByteLog::new();
    let mut t = FileTransport::new(dir.path().to_path_buf(), "gpgl".into());

    let r = t
        .send_job(&CheckedJob::check(job.clone()).unwrap(), "square", &mut log)
        .unwrap();
    assert_eq!(r.bytes_sent, job.bytes.len());
    let gpgl = dir.path().join("square.gpgl");
    let txt = dir.path().join("square.gpgl.txt");
    let jsonl = dir.path().join("square.bytelog.jsonl");
    assert_eq!(r.files, vec![gpgl.clone(), txt.clone(), jsonl.clone()]);
    assert_eq!(std::fs::read(&gpgl).unwrap(), job.bytes);

    let listing = std::fs::read_to_string(&txt).unwrap();
    let lines: Vec<&str> = listing.lines().collect();
    assert_eq!(lines.len(), job.listing.len());
    for (line, l) in lines.iter().zip(&job.listing) {
        assert_eq!(*line, format!("{}\t# {}", l.command, l.note));
    }
    assert!(listing.contains("object 1 'square', path 0, pass 1"));

    // One log entry per command, endpoint 0x01, and together they are the job bytes.
    let text = std::fs::read_to_string(&jsonl).unwrap();
    let rows: Vec<serde_json::Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(rows.len(), job.listing.len());
    let mut all = String::new();
    for row in &rows {
        assert_eq!(row["dir"], "out");
        assert_eq!(row["ep"], "0x01");
        assert!(row["t"].as_f64().unwrap() >= 0.0);
        let h = row["hex"].as_str().unwrap();
        assert_eq!(h, h.to_lowercase());
        all.push_str(h);
    }
    assert_eq!(all, hex(&job.bytes));
    // Key order matches the capture fixtures.
    assert!(
        text.lines().next().unwrap().starts_with("{\"t\":"),
        "{text}"
    );

    // The session log got the same entries.
    assert_eq!(log.len(), job.listing.len());
    let joined: Vec<u8> = log.entries().flat_map(|e| e.bytes.clone()).collect();
    assert_eq!(joined, job.bytes);
    assert!(log.entries().all(|e| e.dir == Dir::Out));

    // Collisions get -2, -3.
    let r2 = t
        .send_job(&CheckedJob::check(job.clone()).unwrap(), "square", &mut log)
        .unwrap();
    assert_eq!(r2.files[0], dir.path().join("square-2.gpgl"));
    assert_eq!(r2.files[2], dir.path().join("square-2.bytelog.jsonl"));
    let r3 = t
        .send_job(&CheckedJob::check(job.clone()).unwrap(), "square", &mut log)
        .unwrap();
    assert_eq!(r3.files[0], dir.path().join("square-3.gpgl"));

    // The per-job byte log holds only its own job.
    let text = std::fs::read_to_string(&r3.files[2]).unwrap();
    assert_eq!(text.lines().count(), job.listing.len());

    // No temp files are left.
    assert_eq!(
        names(dir.path()),
        vec![
            "square-2.bytelog.jsonl",
            "square-2.gpgl",
            "square-2.gpgl.txt",
            "square-3.bytelog.jsonl",
            "square-3.gpgl",
            "square-3.gpgl.txt",
            "square.bytelog.jsonl",
            "square.gpgl",
            "square.gpgl.txt",
        ]
    );
}

#[test]
fn collision_on_any_of_the_three_files_moves_on() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("job.bytelog.jsonl"), "keep").unwrap();
    let mut log = ByteLog::new();
    let mut t = FileTransport::new(dir.path().to_path_buf(), "gpgl".into());
    let r = t
        .send_job(&CheckedJob::check(square_job()).unwrap(), "job", &mut log)
        .unwrap();
    assert_eq!(r.files[0], dir.path().join("job-2.gpgl"));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("job.bytelog.jsonl")).unwrap(),
        "keep"
    );
}

#[test]
fn byte_log_jsonl_format_and_ring() {
    let mut log = ByteLog::new();
    log.push(Dir::Out, b"\x1b\x04");
    log.push(Dir::In, b"0\x03");
    let text = log.to_jsonl();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2);
    let a: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(a["dir"], "out");
    assert_eq!(a["ep"], "0x01");
    assert_eq!(a["hex"], "1b04");
    let b: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(b["dir"], "in");
    assert_eq!(b["ep"], "0x82");
    assert_eq!(b["hex"], "3003");
    assert!(b["t"].is_f64() || b["t"].is_u64());
    assert!(lines[0].starts_with("{\"t\":"));

    for i in 0..bytelog::CAPACITY {
        log.push(Dir::Out, &[(i % 256) as u8]);
    }
    assert_eq!(log.len(), bytelog::CAPACITY);
    // The two oldest entries were dropped.
    assert_eq!(log.entries().next().unwrap().bytes, vec![0u8]);
    assert_eq!(log.entries().next().unwrap().dir, Dir::Out);
}

#[test]
fn leftover_temp_files_do_not_block_the_job_name_and_are_kept() {
    let dir = tempfile::tempdir().unwrap();
    let old_style = dir.path().join(".job.gpgl.tmp");
    let other = std::process::id().wrapping_add(1);
    let fresh = dir.path().join(format!(".job.gpgl.{other}.0.tmp"));
    for f in [&old_style, &fresh] {
        std::fs::write(f, "someone else's").unwrap();
    }
    let mut log = ByteLog::new();
    let mut t = FileTransport::new(dir.path().to_path_buf(), "gpgl".into());
    let r = t
        .send_job(&CheckedJob::check(square_job()).unwrap(), "job", &mut log)
        .unwrap();
    assert_eq!(r.files[0], dir.path().join("job.gpgl"));
    for f in [&old_style, &fresh] {
        assert_eq!(std::fs::read_to_string(f).unwrap(), "someone else's");
    }
    let tmps: Vec<String> = names(dir.path())
        .into_iter()
        .filter(|n| n.ends_with(".tmp"))
        .collect();
    assert_eq!(tmps.len(), 2, "no temp file of ours is left: {tmps:?}");
}
