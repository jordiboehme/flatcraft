//! Tool bars that fit small windows. A bar is a row of groups. As the
//! width shrinks it collapses in steps:
//!
//! 1. [`Level::Full`]: every group with icons and labels.
//! 2. [`Level::Icons`]: icon-only buttons; each keeps its name as its
//!    accessible name and shows it in its tooltip.
//! 3. [`Level::Collapsed`]: the groups that still do not fit collapse,
//!    from the right. A group with a tile (the ribbon's groups) becomes
//!    only its tile, whose list holds every action; a group without one
//!    (the Cut bar's steps) moves into a "»" button at the right end,
//!    which lists it with icons and labels ([`Mode::List`]).
//!
//! The widths are measured from what was drawn, never guessed, and a
//! tile's width comes from the function that sizes the tile: a level that
//! no longer fits steps down at once (the pass is discarded, so the person
//! never sees it), and a level steps up only when its measured width from
//! an earlier pass fits with [`MARGIN`] to spare, so a bar never jumps back
//! and forth. A change of content (another tab, other pins, a longer field
//! text) starts the measuring again from the top.
//!
//! A frame has at most three passes (`FlatCraftApp::new`): Full, then
//! Icons (measured), then the collapsed level computed from exact widths.
//! Should a change still be needed in the last pass, which cannot be
//! discarded, the bar takes the most collapsed level, which always fits as
//! well as anything can, and steps up from there in a later frame.

use egui::{Id, Ui, UiBuilder};

use super::settings::{painted_label, set_name};

/// How a group draws itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// On the bar, with labels.
    Full,
    /// On the bar, icon-only.
    Icons,
    /// On the bar, only its tile.
    Collapsed,
    /// In the overflow list: one item per row, icon and label.
    List,
}

/// One group of a bar.
pub struct Group<C> {
    /// Its heading in the overflow list (painted only, so it never clashes
    /// with an accessible name).
    pub title: &'static str,
    /// Never collapses.
    pub keep: bool,
    /// The width of its tile, when it collapses to one ([`Mode::Collapsed`]);
    /// `None`: it moves into the "»" list instead.
    pub tile: Option<f32>,
    pub draw: fn(&mut Ui, Mode, &mut C),
}

/// How far a bar is collapsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Level {
    #[default]
    Full,
    Icons,
    /// Icon-only, with this many groups collapsed from the right.
    Collapsed(usize),
}

impl Level {
    pub fn mode(self) -> Mode {
        match self {
            Level::Full => Mode::Full,
            _ => Mode::Icons,
        }
    }
}

/// Room a fuller level needs to spare before the bar steps up to it.
pub const MARGIN: f32 = 12.0;

/// The accessible name of the overflow button.
pub const MORE: &str = "More tools";

/// What a bar remembers between passes.
#[derive(Debug, Clone, Default)]
struct State {
    sig: u64,
    level: Level,
    /// Width of the whole row at [`Level::Full`].
    full: Option<f32>,
    /// Width of each group at [`Level::Icons`].
    icons: Vec<Option<f32>>,
    /// Room between two groups (the separator and its spacing).
    gap: f32,
}

/// The measures the level is chosen from.
#[derive(Debug, Clone, PartialEq)]
pub struct Measures<'a> {
    /// The room the bar has.
    pub budget: f32,
    /// Width of the row at [`Level::Full`], once measured.
    pub full: Option<f32>,
    /// Width of each group at [`Level::Icons`], once all are measured.
    pub icons: Option<&'a [f32]>,
    pub gap: f32,
    /// Width of the "»" button.
    pub more: f32,
    pub keep: &'a [bool],
    /// Width of each group's tile, if it has one.
    pub tiles: &'a [Option<f32>],
}

/// Which groups `level` collapses: `k` of those that may, from the right.
pub fn hidden(keep: &[bool], level: Level) -> Vec<bool> {
    let mut out = vec![false; keep.len()];
    if let Level::Collapsed(k) = level {
        let mut left = k;
        for i in (0..keep.len()).rev() {
            if left == 0 {
                break;
            }
            if !keep[i] {
                out[i] = true;
                left -= 1;
            }
        }
    }
    out
}

/// Width of the icon-only row with `k` groups collapsed: the shown groups,
/// the tiles, and the "»" button when a group without a tile is listed.
pub fn icons_width(m: &Measures, icons: &[f32], k: usize) -> f32 {
    let hide = hidden(m.keep, Level::Collapsed(k));
    let mut parts: Vec<f32> = Vec::new();
    let mut listed = false;
    for (i, w) in icons.iter().enumerate() {
        match (hide[i], m.tiles.get(i).copied().flatten()) {
            (false, _) => parts.push(*w),
            (true, Some(tile)) => parts.push(tile),
            (true, None) => listed = true,
        }
    }
    if listed {
        parts.push(m.more);
    }
    parts.iter().sum::<f32>() + m.gap * parts.len().saturating_sub(1) as f32
}

/// The number of groups that may collapse.
fn movable(m: &Measures) -> usize {
    m.keep.iter().filter(|k| !**k).count()
}

/// The fullest icon-only level that fits `room`.
fn fitting_icons(m: &Measures, icons: &[f32], room: f32) -> Level {
    for k in 0..=movable(m) {
        if icons_width(m, icons, k) <= room {
            return if k == 0 {
                Level::Icons
            } else {
                Level::Collapsed(k)
            };
        }
    }
    Level::Collapsed(movable(m))
}

/// The level of the next pass, from the one just drawn and its measured
/// width `drawn`.
pub fn next_level(level: Level, drawn: f32, m: &Measures) -> Level {
    let movable = movable(m);
    if drawn > m.budget + 0.5 {
        // Step down.
        return match (level, m.icons) {
            (Level::Full, None) => Level::Icons,
            (_, Some(icons)) => {
                let fit = fitting_icons(m, icons, m.budget);
                // A stale measure may claim the level just drawn fits.
                match (level, fit) {
                    (Level::Icons, Level::Icons) => Level::Collapsed(1.min(movable)),
                    (Level::Collapsed(k), Level::Icons) => Level::Collapsed((k + 1).min(movable)),
                    (Level::Collapsed(k), Level::Collapsed(j)) if j <= k => {
                        Level::Collapsed((k + 1).min(movable))
                    }
                    (_, fit) => fit,
                }
            }
            (_, None) => level,
        };
    }
    // It fits: step up when a fuller level fits with room to spare.
    let room = m.budget - MARGIN;
    if level != Level::Full && m.full.is_some_and(|w| w <= room) {
        return Level::Full;
    }
    if let (Level::Collapsed(k), Some(icons)) = (level, m.icons) {
        let fit = fitting_icons(m, icons, room);
        let better = match fit {
            Level::Icons => true,
            Level::Collapsed(j) => j < k,
            Level::Full => false,
        };
        if better {
            return fit;
        }
    }
    level
}

/// The level a bar takes when it must change in a pass that cannot be
/// discarded: everything that may collapse is collapsed.
pub fn last_pass_level(m: &Measures) -> Level {
    match movable(m) {
        0 => Level::Icons,
        k => Level::Collapsed(k),
    }
}

/// The level of the next pass, or of the next frame when this is the last
/// pass of the frame (`last_pass`, it cannot be discarded): a step down
/// then goes straight to the most collapsed level, so the next frame fits
/// even if a measure was off.
pub fn decide(level: Level, drawn: f32, m: &Measures, last_pass: bool) -> Level {
    let next = next_level(level, drawn, m);
    if next != level && last_pass && drawn > m.budget + 0.5 {
        return last_pass_level(m);
    }
    next
}

/// Draws the bar into the rest of `ui`'s row, `height` high. `sig`
/// identifies the content (the tab, the pins, anything that changes the
/// widths): when it changes, the bar measures again from the top.
pub fn show<C>(ui: &mut Ui, id: Id, sig: u64, height: f32, groups: &[Group<C>], cx: &mut C) {
    let mut st: State = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    if st.sig != sig || st.icons.len() != groups.len() {
        st = State {
            sig,
            icons: vec![None; groups.len()],
            ..State::default()
        };
    }
    let keep: Vec<bool> = groups.iter().map(|g| g.keep).collect();
    let tiles: Vec<Option<f32>> = groups.iter().map(|g| g.tile).collect();
    let mut rect = ui.available_rect_before_wrap();
    rect.set_height(height);
    let budget = rect.width();
    let hide = hidden(&keep, st.level);
    let mode = st.level.mode();
    let more_w = more_width(ui);

    let mut widths: Vec<Option<f32>> = vec![None; groups.len()];
    let mut ends: Vec<(f32, f32)> = Vec::new();
    let mut right = rect.left();
    let listed: Vec<bool> = (0..groups.len())
        .map(|i| hide[i] && tiles[i].is_none())
        .collect();
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            // Clip only at the right: a group may be taller than the row.
            let clip = ui.clip_rect();
            ui.set_clip_rect(egui::Rect::from_x_y_ranges(
                clip.left()..=rect.right().min(clip.right()),
                clip.y_range(),
            ));
            let mut first = true;
            for (i, g) in groups.iter().enumerate() {
                if listed[i] {
                    continue;
                }
                if !first {
                    ui.separator();
                }
                first = false;
                let m = if hide[i] { Mode::Collapsed } else { mode };
                let r = ui.scope(|ui| (g.draw)(ui, m, cx)).response.rect;
                if !hide[i] {
                    widths[i] = Some(r.width());
                }
                ends.push((r.left(), r.right()));
                right = r.right();
            }
            if listed.iter().any(|h| *h) {
                if !first {
                    ui.separator();
                }
                let resp = ui
                    .add(egui::Button::new("»"))
                    .on_hover_text("More tools that do not fit the window");
                set_name(ui, &resp, MORE);
                right = resp.rect.right();
                egui::Popup::menu(&resp)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .show(|ui| {
                        ui.set_min_width(220.0);
                        let mut first = true;
                        for (i, g) in groups.iter().enumerate() {
                            if !listed[i] {
                                continue;
                            }
                            if !first {
                                ui.separator();
                            }
                            first = false;
                            ui.scope(|ui| {
                                ui.style_mut().override_text_style = Some(egui::TextStyle::Small);
                                ui.visuals_mut().override_text_color =
                                    Some(ui.visuals().weak_text_color());
                                painted_label(ui, g.title);
                            });
                            ui.vertical(|ui| (g.draw)(ui, Mode::List, cx));
                        }
                    });
            }
        },
    );
    // Measure what was drawn.
    if let [(_, a_end), (b_start, _), ..] = ends[..] {
        st.gap = b_start - a_end;
    }
    let drawn = right - rect.left();
    match st.level {
        Level::Full => st.full = Some(drawn),
        _ => {
            for (slot, w) in st.icons.iter_mut().zip(&widths) {
                if let Some(w) = w {
                    *slot = Some(*w);
                }
            }
        }
    }
    let icons: Option<Vec<f32>> = st.icons.iter().copied().collect();
    let m = Measures {
        budget,
        full: st.full,
        icons: icons.as_deref(),
        gap: st.gap,
        more: more_w,
        keep: &keep,
        tiles: &tiles,
    };
    let ctx = ui.ctx();
    let last_pass = ctx.current_pass_index() + 1 >= ctx.options(|o| o.max_passes.get());
    let next = decide(st.level, drawn, &m, last_pass);
    if next != st.level {
        st.level = next;
        ctx.request_discard("a tool bar changed how far it is collapsed");
        ctx.request_repaint();
    }
    ui.data_mut(|d| d.insert_temp(id, st));
}

/// Width of the "»" button.
fn more_width(ui: &Ui) -> f32 {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap("»".into(), font, egui::Color32::PLACEHOLDER);
    galley.size().x + 2.0 * ui.spacing().button_padding.x
}

/// The level a bar drew last, for tests.
pub fn level(ctx: &egui::Context, id: Id) -> Option<Level> {
    ctx.data(|d| d.get_temp::<State>(id)).map(|s| s.level)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m<'a>(
        budget: f32,
        full: Option<f32>,
        icons: Option<&'a [f32]>,
        keep: &'a [bool],
    ) -> Measures<'a> {
        Measures {
            budget,
            full,
            icons,
            gap: 10.0,
            more: 20.0,
            keep,
            tiles: &NO_TILES,
        }
    }

    const KEEP: [bool; 3] = [false, true, false];
    const NO_TILES: [Option<f32>; 3] = [None; 3];

    #[test]
    fn overflow_moves_groups_from_the_right_but_never_a_kept_one() {
        assert_eq!(hidden(&KEEP, Level::Collapsed(1)), [false, false, true]);
        assert_eq!(hidden(&KEEP, Level::Collapsed(2)), [true, false, true]);
        assert_eq!(hidden(&KEEP, Level::Collapsed(5)), [true, false, true]);
        assert_eq!(hidden(&KEEP, Level::Icons), [false, false, false]);
    }

    #[test]
    fn steps_down_in_order() {
        let icons = [100.0, 50.0, 100.0];
        // Full too wide, icons not measured yet: try icons.
        assert_eq!(
            next_level(Level::Full, 500.0, &m(400.0, None, None, &KEEP)),
            Level::Icons
        );
        // Icons measured: 100 + 50 + 100 + 2 gaps = 270.
        let mm = m(260.0, Some(500.0), Some(&icons), &KEEP);
        assert_eq!(next_level(Level::Icons, 270.0, &mm), Level::Collapsed(1));
        // Without the last group: 100 + 50 + gap + gap + more = 190.
        let mm = m(180.0, Some(500.0), Some(&icons), &KEEP);
        assert_eq!(next_level(Level::Icons, 270.0, &mm), Level::Collapsed(2));
        // Straight from Full when the icon widths are known.
        let mm = m(300.0, Some(500.0), Some(&icons), &KEEP);
        assert_eq!(next_level(Level::Full, 500.0, &mm), Level::Icons);
    }

    #[test]
    fn steps_up_only_with_room_to_spare() {
        let icons = [100.0, 50.0, 100.0];
        // 270 wide icons: exactly fitting is not enough to leave Overflow.
        let mm = m(270.0, Some(500.0), Some(&icons), &KEEP);
        assert_eq!(
            next_level(Level::Collapsed(1), 190.0, &mm),
            Level::Collapsed(1)
        );
        let mm = m(270.0 + MARGIN, Some(500.0), Some(&icons), &KEEP);
        assert_eq!(next_level(Level::Collapsed(1), 190.0, &mm), Level::Icons);
        // Full needs its own width plus the margin.
        let mm = m(505.0, Some(500.0), Some(&icons), &KEEP);
        assert_eq!(next_level(Level::Icons, 270.0, &mm), Level::Icons);
        let mm = m(500.0 + MARGIN, Some(500.0), Some(&icons), &KEEP);
        assert_eq!(next_level(Level::Icons, 270.0, &mm), Level::Full);
    }

    #[test]
    fn collapsed_groups_count_their_tiles_and_need_no_list() {
        let icons = [100.0, 50.0, 100.0];
        let tiles = [Some(40.0), None, Some(30.0)];
        let mm = Measures {
            tiles: &tiles,
            ..m(1000.0, Some(500.0), Some(&icons), &KEEP)
        };
        // The last group becomes its 30 wide tile: 100 + 50 + 30 + 2 gaps.
        assert_eq!(icons_width(&mm, &icons, 1), 200.0);
        // Both movable groups as tiles, no » (they all have tiles).
        assert_eq!(icons_width(&mm, &icons, 2), 40.0 + 50.0 + 30.0 + 20.0);
        // Without tiles, the » comes in.
        let mm = m(1000.0, Some(500.0), Some(&icons), &KEEP);
        assert_eq!(icons_width(&mm, &icons, 2), 50.0 + 20.0 + 10.0);
    }

    #[test]
    fn a_change_in_the_last_pass_goes_to_the_most_collapsed_level() {
        let icons = [100.0, 50.0, 100.0];
        let tiles = [Some(40.0), None, Some(30.0)];
        // A stale measure says Icons fits; the drawn row did not.
        let mm = Measures {
            tiles: &tiles,
            ..m(260.0, Some(500.0), Some(&icons), &KEEP)
        };
        assert_eq!(decide(Level::Full, 500.0, &mm, false), Level::Collapsed(1));
        assert_eq!(decide(Level::Full, 500.0, &mm, true), Level::Collapsed(2));
        assert!(icons_width(&mm, &icons, 2) <= 260.0);
        // Stepping up in the last pass is safe: the shown pass fits.
        let wide = Measures {
            tiles: &tiles,
            ..m(1000.0, Some(500.0), Some(&icons), &KEEP)
        };
        assert_eq!(decide(Level::Collapsed(2), 120.0, &wide, true), Level::Full);
    }

    #[test]
    fn no_width_flips_between_two_levels() {
        // For every budget, the level reached from either side is stable.
        let icons = [100.0, 50.0, 100.0];
        for budget in (100..700).map(|b| b as f32) {
            let mm = m(budget, Some(500.0), Some(&icons), &KEEP);
            let width = |l: Level| match l {
                Level::Full => 500.0,
                Level::Icons => icons_width(&mm, &icons, 0),
                Level::Collapsed(k) => icons_width(&mm, &icons, k),
            };
            for start in [
                Level::Full,
                Level::Icons,
                Level::Collapsed(1),
                Level::Collapsed(2),
            ] {
                let mut l = start;
                let mut seen = Vec::new();
                for _ in 0..8 {
                    seen.push(l);
                    l = next_level(l, width(l), &mm);
                }
                let last = seen[seen.len() - 1];
                assert_eq!(
                    seen[seen.len() - 2],
                    last,
                    "budget {budget} from {start:?}: {seen:?}"
                );
                if last != Level::Collapsed(2) {
                    assert!(
                        width(last) <= budget,
                        "budget {budget}: {last:?} does not fit"
                    );
                }
            }
        }
    }
}
