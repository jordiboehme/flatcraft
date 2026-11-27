//! `common::capture` returns only what one action sent, so a command from
//! an earlier action can never let a check pass.

mod common;
use common::{capture, harness, session};

#[test]
fn a_capture_holds_only_what_its_action_sent() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(&dir, session(&dir), false);
    let ctx = h.ctx.clone();
    let first = capture(&mut h, |h| {
        ctx.copy_text("a".into());
        h.step();
        // An extra frame must not wipe the command.
        h.step();
    });
    assert_eq!(first.copied().as_deref(), Some("a"));
    // The same copy that silently fails: the check must fail, not see "a".
    let second = capture(&mut h, |h| h.step());
    assert_eq!(second.copied(), None, "the first copy is not seen again");
    assert!(second.is_empty());
    // A real second copy of the same text is seen.
    let third = capture(&mut h, |h| {
        ctx.copy_text("a".into());
        h.step();
    });
    assert_eq!(third.copied().as_deref(), Some("a"));
}

#[test]
fn a_close_or_a_url_from_before_is_not_seen_by_the_next_capture() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(&dir, session(&dir), false);
    let ctx = h.ctx.clone();
    let first = capture(&mut h, |h| {
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        ctx.open_url(egui::OpenUrl::new_tab("https://example.com/"));
        h.step();
    });
    assert!(first.closes());
    assert_eq!(first.urls(), ["https://example.com/"]);
    let next = capture(&mut h, |h| h.step());
    assert!(!next.closes());
    assert!(next.urls().is_empty());
}
