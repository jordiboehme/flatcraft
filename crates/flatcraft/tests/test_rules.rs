//! Rules for the window tests themselves, so a busy machine cannot make
//! them flaky:
//!
//! - No bare `Harness::run` (`run`, `try_run`, `run_ok`, `run_steps`): it
//!   fails, or steps a fixed count, when a worker lands within its few
//!   frames. Use `run_plan` or `common::wait_until`, which wait for the
//!   worker's event. A line that truly needs it says why in a `// run:`
//!   comment on it or just above.
//! - No click with a wait between press and release: egui counts a press
//!   held longer than 0.8 s (about three test frames) as no click, and a
//!   wait steps as many frames as a worker needs. Use `common::click_at`.
//!   A press that is meant to be held says so in a `// held:` comment.
//!
//! - No read of a harness's last frame output (`.output()` and its
//!   `platform_output`, `viewport_output` or `shapes`): the last frame is
//!   whichever frame a busy machine ran last, so a command sent by a click
//!   (open a URL, copy, close) is gone after one extra frame. Read
//!   `common::capture`, which records every frame of one action. State that every
//!   frame sets again (the cursor, the painted shapes, the repaint delay)
//!   is fine, with a `// output:` comment on the line or just above.
//!   Known gap: `let o = h.output();` followed by a field read on a later
//!   line is not seen by the scan; review catches it.
//!
//! The scan works on the code with comments and string contents removed
//! and all whitespace dropped, so split calls and arguments over several
//! lines match too. It cannot see everything: a press and a release of the
//! same point written as different expressions, or more than
//! [`PAIR_LINES`] lines apart, are left to review.

/// How many lines after a press its release is looked for.
const PAIR_LINES: usize = 12;

/// The source as one string of code without comments, string contents or
/// whitespace, the line (1-based) of each of its bytes, and the comment
/// text of each line.
struct Code {
    code: String,
    line: Vec<usize>,
    comments: Vec<String>,
}

impl Code {
    fn of(src: &str) -> Self {
        let mut code = String::new();
        let mut line = Vec::new();
        let mut comments = vec![String::new()];
        let mut n = 1;
        let mut chars = src.chars().peekable();
        let mut in_str = false;
        let mut push = |c: char, n: usize, code: &mut String| {
            code.push(c);
            line.extend(std::iter::repeat_n(n, c.len_utf8()));
        };
        while let Some(c) = chars.next() {
            if c == '\n' {
                n += 1;
                comments.push(String::new());
                continue;
            }
            if in_str {
                match c {
                    '\\' => {
                        chars.next();
                    }
                    '"' => {
                        in_str = false;
                        push('"', n, &mut code);
                    }
                    _ => {}
                }
                continue;
            }
            match c {
                '"' => {
                    in_str = true;
                    push('"', n, &mut code);
                }
                // A char literal: '"' must not open a string.
                '\'' if chars.clone().nth(1) == Some('\'') => {
                    chars.next();
                    chars.next();
                }
                '/' if chars.peek() == Some(&'/') => {
                    let mut text = String::from("/");
                    while let Some(&d) = chars.peek() {
                        if d == '\n' {
                            break;
                        }
                        text.push(d);
                        chars.next();
                    }
                    comments[n - 1].push_str(&text);
                }
                '/' if chars.peek() == Some(&'*') => {
                    chars.next();
                    let mut last = ' ';
                    for d in chars.by_ref() {
                        if d == '\n' {
                            n += 1;
                            comments.push(String::new());
                        }
                        if last == '*' && d == '/' {
                            break;
                        }
                        last = d;
                    }
                }
                c if c.is_whitespace() => {}
                c => push(c, n, &mut code),
            }
        }
        Self {
            code,
            line,
            comments,
        }
    }

    /// The line of the byte at `at`.
    fn line_at(&self, at: usize) -> usize {
        self.line.get(at).copied().unwrap_or(0)
    }

    /// Whether line `n` or the line above has a comment starting with `tag`.
    fn allowed(&self, n: usize, tag: &str) -> bool {
        let has = |n: usize| {
            n >= 1
                && self
                    .comments
                    .get(n - 1)
                    .is_some_and(|c| c.contains(&format!("// {tag}:")))
        };
        has(n) || has(n.saturating_sub(1))
    }

    /// The text between the bracket opened just before `open` and its
    /// closing one, and the index after it.
    fn group(&self, open: usize, open_c: char, close_c: char) -> (&str, usize) {
        let mut depth = 1;
        for (j, c) in self.code[open..].char_indices() {
            if c == open_c {
                depth += 1;
            } else if c == close_c {
                depth -= 1;
                if depth == 0 {
                    return (&self.code[open..open + j], open + j + 1);
                }
            }
        }
        (&self.code[open..], self.code.len())
    }
}

/// Lines (1-based) of `src` that call `Harness::run` without a `// run:`
/// comment on the line or the line above.
fn bare_runs(src: &str) -> Vec<usize> {
    let c = Code::of(src);
    let mut found = Vec::new();
    for call in [
        ".run()",
        ".try_run()",
        ".run_ok()",
        ".run_steps(",
        "Harness::run(",
    ] {
        for (at, _) in c.code.match_indices(call) {
            let n = c.line_at(at);
            if !c.allowed(n, "run") {
                found.push(n);
            }
        }
    }
    found.sort_unstable();
    found.dedup();
    found
}

/// Lines (1-based) of `src` that read the last frame's output without an
/// `// output:` comment on the line or the line above.
fn last_frame_reads(src: &str) -> Vec<usize> {
    let c = Code::of(src);
    let mut found = Vec::new();
    for field in [
        ".platform_output",
        ".viewport_output",
        ".shapes",
        ".textures_delta",
    ] {
        let call = format!(".output(){field}");
        for (at, _) in c.code.match_indices(&call) {
            let n = c.line_at(at);
            if !c.allowed(n, "output") {
                found.push(n);
            }
        }
    }
    found.sort_unstable();
    found.dedup();
    found
}

/// What happens to the pointer, in order.
#[derive(Debug, PartialEq)]
enum Step {
    Press(String),
    Release(String),
    Move,
    Wait,
}

/// A press argument without a trailing comma.
fn point(arg: &str) -> String {
    arg.trim_end_matches(',').to_string()
}

/// The pointer steps of the code, with the line each starts on.
fn steps(c: &Code) -> Vec<(usize, Step)> {
    let code = &c.code;
    let mut out = Vec::new();
    let mut i = 0;
    while i < code.len() {
        let rest = &code[i..];
        let mut next = i + rest.chars().next().map_or(1, char::len_utf8);
        if let Some(arg_at) = [".drag_at(", ".drop_at("]
            .iter()
            .find(|p| rest.starts_with(**p))
            .map(|p| i + p.len())
        {
            let (arg, end) = c.group(arg_at, '(', ')');
            out.push((
                c.line_at(i),
                if rest.starts_with(".drag_at(") {
                    Step::Press(point(arg))
                } else {
                    Step::Release(point(arg))
                },
            ));
            next = end;
        } else if rest.starts_with("PointerButton{") {
            let (fields, end) = c.group(i + "PointerButton{".len(), '{', '}');
            let pos = fields
                .split(',')
                .find_map(|f| f.strip_prefix("pos:"))
                .map(point);
            if let Some(pos) = pos {
                if fields.contains("pressed:true") {
                    out.push((c.line_at(i), Step::Press(pos)));
                } else if fields.contains("pressed:false") {
                    out.push((c.line_at(i), Step::Release(pos)));
                }
            }
            next = end;
        } else if rest.starts_with(".hover_at(") || rest.starts_with("PointerMoved(") {
            out.push((c.line_at(i), Step::Move));
        } else if [
            "run_plan(",
            "wait_until(",
            "wait_for_",
            "settle(",
            ".run_steps(",
        ]
        .iter()
        .any(|w| rest.starts_with(w))
        {
            out.push((c.line_at(i), Step::Wait));
        } else if let Some(range) = rest.strip_prefix("for_in0..") {
            // A loop of more than three frames is a wait too.
            let digits: String = range.chars().take_while(char::is_ascii_digit).collect();
            let frames = digits.parse::<usize>().unwrap_or(usize::MAX);
            if let Some(open) = rest.find('{') {
                let (body, _) = c.group(i + open + 1, '{', '}');
                if body.contains(".step()") && frames > 3 {
                    out.push((c.line_at(i), Step::Wait));
                }
            }
        }
        i = next;
    }
    out
}

/// Lines (1-based) of `src` where a press at a point is followed by a wait
/// and then a release at the same point, with no move in between, unless
/// the press line or the line above has a `// held:` comment.
fn held_clicks(src: &str) -> Vec<usize> {
    let c = Code::of(src);
    let steps = steps(&c);
    let mut found = Vec::new();
    for (k, (n, step)) in steps.iter().enumerate() {
        let Step::Press(at) = step else { continue };
        if c.allowed(*n, "held") {
            continue;
        }
        let mut waited = false;
        for (m, next) in &steps[k + 1..] {
            if *m > n + PAIR_LINES {
                break;
            }
            match next {
                Step::Wait => waited = true,
                Step::Release(to) => {
                    if waited && to == at {
                        found.push(*n);
                    }
                    break;
                }
                Step::Press(_) | Step::Move => break,
            }
        }
    }
    found
}

#[test]
fn the_rules_find_what_they_look_for() {
    let src = r#"h.run();
r.h.run_ok();
// run: no worker in this harness
h.run();
h.run(); // run: no worker
h.run_plan();
// h.run();
h.run_steps(2);
egui_kittest::Harness::run(&mut h);
h
    .run();
let s = "// not a comment"; h.run();
let t = "h.run()";
"#;
    assert_eq!(bare_runs(src), vec![1, 2, 8, 9, 11, 12]);
    let src = r#"h.drag_at(at);
h.run_plan();
h.drop_at(at);
h.drag_at(p);
h.step();
h.drop_at(p);
h.drag_at(from);
h.run_plan();
h.hover_at(to);
h.run_plan();
h.drop_at(to);
// held: a long press is the point
h.drag_at(at);
h.run_plan();
h.drop_at(at);
h.drag_at(
    egui::pos2(1.0, 2.0),
);
h.run_plan();
h.drop_at(egui::pos2(1.0, 2.0));
h.drag_at(q);
for _ in 0..8 {
    h.step();
}
h.drop_at(q);
h.drag_at(q);
for _ in 0..2 {
    h.step();
}
h.drop_at(q);
h.event(egui::Event::PointerButton {
    pos: at,
    button: egui::PointerButton::Primary,
    pressed: true,
    modifiers: egui::Modifiers::NONE,
});
h.run_plan();
h.event(egui::Event::PointerButton {
    pos: at,
    button: egui::PointerButton::Primary,
    pressed: false,
    modifiers: egui::Modifiers::NONE,
});
"#;
    assert_eq!(held_clicks(src), vec![1, 16, 21, 31]);
    let src = r#"h.output().platform_output.commands;
h
    .output()
    .viewport_output;
// output: state of the last frame
h.output().platform_output.cursor_icon;
h.output().shapes; // output: painted
let out = cmd.output().expect("runs");
let t = "h.output().shapes";
// h.output().shapes
for c in &h.output().shapes {}
"#;
    assert_eq!(last_frame_reads(src), vec![1, 3, 11]);
}

#[test]
fn window_tests_wait_by_event_and_click_in_fixed_frames() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut files: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .collect();
    files.push(root.join("common/mod.rs"));
    assert!(files.len() > 30, "scanned {} files", files.len());
    let mut bad = Vec::new();
    for f in &files {
        let name = f
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if name == "test_rules.rs" {
            continue;
        }
        let src = std::fs::read_to_string(f).unwrap();
        for l in bare_runs(&src) {
            bad.push(format!(
                "{name}:{l}: Harness::run (use run_plan or wait_until)"
            ));
        }
        for l in last_frame_reads(&src) {
            bad.push(format!(
                "{name}:{l}: last frame output (read common::capture)"
            ));
        }
        for l in held_clicks(&src) {
            bad.push(format!(
                "{name}:{l}: a wait between press and release (use click_at)"
            ));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
