//! Tool bars that fit small windows. A bar is a row of groups. As the
//! width shrinks, the groups fold one step at a time, from the right.
//! Each group goes Full (icons and labels), then Icons (icon-only buttons;
//! each keeps its name as its accessible name and shows it in its
//! tooltip), then Tile (only its tile, whose list holds every action; a
//! group without a tile, like the Cut bar's steps, moves into a "»" button
//! at the right end, which lists it with icons and labels, [`Mode::List`]).
//! A group with the keep rule (the ribbon's Size) goes to Icons but never
//! to a tile.
//!
//! The order of the steps is the table [`steps`] (rightmost movable group
//! first). After every step the bar checks the fit and stops at the first
//! state that fits:
//!
//! | step | group (from the right) | goes to |
//! |------|------------------------|---------|
//! | 1    | 1st movable            | Icons   |
//! | 2    | 1st movable            | Tile    |
//! | 3..  | every other group, right to left (keep groups too) | Icons |
//! | then | every other movable group, right to left | Tile |
//!
//! For Add, Edit, Size (keep), Arrange and Linked files: Linked files
//! Icons, Linked files Tile, Arrange Icons, Size Icons, Edit Icons, Add
//! Icons, Arrange Tile, Edit Tile, Add Tile. So the group that is the
//! least used folds first, and a group at the left keeps its labels longest.
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
//! [`Level::Measure`] (every group icon-only, to measure the icon widths),
//! then the state chosen from exact widths. Should a change still be
//! needed in the last pass, which cannot be discarded, the bar takes the
//! most folded state, and steps up from there in a later frame.

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

/// How far a bar is folded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Level {
    /// Every group full.
    #[default]
    Full,
    /// Every group icon-only: drawn to measure the icon widths.
    Measure,
    /// The first `n` steps of [`steps`] taken.
    Folded(usize),
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
    /// Width of each group at [`Level::Full`].
    fulls: Vec<Option<f32>>,
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
    /// Width of each group at [`Level::Full`], once all are measured.
    pub fulls: Option<&'a [f32]>,
    /// Width of each group icon-only, once all are measured.
    pub icons: Option<&'a [f32]>,
    pub gap: f32,
    /// Width of the "»" button.
    pub more: f32,
    pub keep: &'a [bool],
    /// Width of each group's tile, if it has one.
    pub tiles: &'a [Option<f32>],
}

/// The steps of the fold table for groups with these keep rules, in order:
/// the group and the mode it takes.
pub fn steps(keep: &[bool]) -> Vec<(usize, Mode)> {
    let movable: Vec<usize> = (0..keep.len()).rev().filter(|i| !keep[*i]).collect();
    let mut out = Vec::new();
    let Some(&first) = movable.first() else {
        // Nothing may tile: the groups only go icon-only.
        return (0..keep.len()).rev().map(|i| (i, Mode::Icons)).collect();
    };
    out.push((first, Mode::Icons));
    out.push((first, Mode::Collapsed));
    for i in (0..keep.len()).rev().filter(|i| *i != first) {
        out.push((i, Mode::Icons));
    }
    for &i in &movable[1..] {
        out.push((i, Mode::Collapsed));
    }
    out
}

/// How many steps the table has.
fn last_step(keep: &[bool]) -> usize {
    steps(keep).len()
}

/// The mode of every group at `level`.
pub fn modes(keep: &[bool], level: Level) -> Vec<Mode> {
    match level {
        Level::Full => vec![Mode::Full; keep.len()],
        Level::Measure => vec![Mode::Icons; keep.len()],
        Level::Folded(n) => {
            let mut out = vec![Mode::Full; keep.len()];
            for (i, mode) in steps(keep).into_iter().take(n) {
                out[i] = mode;
            }
            out
        }
    }
}

/// Width of the row with these group modes: the shown groups, the tiles,
/// and the "»" button when a group without a tile is listed.
pub fn row_width(m: &Measures, modes: &[Mode]) -> Option<f32> {
    let mut parts: Vec<f32> = Vec::new();
    let mut listed = false;
    for (i, mode) in modes.iter().enumerate() {
        match (mode, m.tiles.get(i).copied().flatten()) {
            (Mode::Full, _) => parts.push(*m.fulls?.get(i)?),
            (Mode::Icons, _) => parts.push(*m.icons?.get(i)?),
            (_, Some(tile)) => parts.push(tile),
            (_, None) => listed = true,
        }
    }
    if listed {
        parts.push(m.more);
    }
    Some(parts.iter().sum::<f32>() + m.gap * parts.len().saturating_sub(1) as f32)
}

/// The place of a level on the ladder: Full 0, Measure 1, Folded(n) n + 1.
fn rank(level: Level) -> usize {
    match level {
        Level::Full => 0,
        Level::Measure => 1,
        Level::Folded(n) => n + 1,
    }
}

/// The level at a place on the ladder (see [`rank`]).
fn at_rank(r: usize) -> Level {
    match r {
        0 => Level::Full,
        1 => Level::Measure,
        r => Level::Folded(r - 1),
    }
}

/// The least folded level that fits `room`, from what is measured: the
/// first state of the fold table that fits. Until the icon widths are
/// measured, the answer after Full is [`Level::Measure`], which measures
/// them.
pub fn choose_level(m: &Measures, room: f32) -> Level {
    if m.full.is_some_and(|w| w <= room) {
        return Level::Full;
    }
    let n = last_step(m.keep);
    if m.icons.is_none() || m.fulls.is_none() {
        return Level::Measure;
    }
    for k in 1..=n {
        if row_width(m, &modes(m.keep, Level::Folded(k))).is_some_and(|w| w <= room) {
            return Level::Folded(k);
        }
    }
    Level::Folded(n)
}

/// The level of the next pass, from the one just drawn and its measured
/// width `drawn`.
pub fn next_level(level: Level, drawn: f32, m: &Measures) -> Level {
    let n = last_step(m.keep);
    if drawn > m.budget + 0.5 {
        // Step down. A stale measure may claim the level just drawn fits:
        // then go one step further.
        let fit = choose_level(m, m.budget);
        if rank(fit) > rank(level) || (level == Level::Full && fit == Level::Measure) {
            return fit;
        }
        return at_rank((rank(level) + 1).min(n + 1));
    }
    // The measuring pass is never a resting level: go to the state the
    // measures pick.
    if level == Level::Measure {
        let fit = choose_level(m, m.budget);
        if fit != Level::Measure {
            return fit;
        }
    }
    // It fits: step up when a less folded level fits with room to spare.
    let fit = choose_level(m, m.budget - MARGIN);
    if rank(fit) < rank(level) {
        return fit;
    }
    level
}

/// The level a bar takes when it must change in a pass that cannot be
/// discarded: everything is folded as far as it goes.
pub fn last_pass_level(m: &Measures) -> Level {
    Level::Folded(last_step(m.keep))
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
    if st.sig != sig || st.icons.len() != groups.len() || st.fulls.len() != groups.len() {
        st = State {
            sig,
            fulls: vec![None; groups.len()],
            icons: vec![None; groups.len()],
            ..State::default()
        };
    }
    let keep: Vec<bool> = groups.iter().map(|g| g.keep).collect();
    let tiles: Vec<Option<f32>> = groups.iter().map(|g| g.tile).collect();
    let mut rect = ui.available_rect_before_wrap();
    rect.set_height(height);
    let budget = rect.width();
    let group_modes = modes(&keep, st.level);
    let hide: Vec<bool> = group_modes.iter().map(|m| *m == Mode::Collapsed).collect();
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
                let m = group_modes[i];
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
    if st.level == Level::Full {
        st.full = Some(drawn);
    }
    for (i, w) in widths.iter().enumerate() {
        if let Some(w) = w {
            let slot = if group_modes[i] == Mode::Full {
                &mut st.fulls[i]
            } else {
                &mut st.icons[i]
            };
            *slot = Some(*w);
        }
    }
    let icons: Option<Vec<f32>> = st.icons.iter().copied().collect();
    let fulls: Option<Vec<f32>> = st.fulls.iter().copied().collect();
    let m = Measures {
        budget,
        full: st.full,
        fulls: fulls.as_deref(),
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

    // Add, Edit, Size (keep), Arrange, Linked files, like the Layout tab.
    const KEEP: [bool; 5] = [false, false, true, false, false];
    const FULLS: [f32; 5] = [150.0, 100.0, 120.0, 140.0, 90.0];
    const ICONS: [f32; 5] = [100.0, 70.0, 120.0, 60.0, 50.0];
    const TILES: [Option<f32>; 5] = [Some(40.0), Some(40.0), None, Some(40.0), Some(40.0)];

    fn mk(budget: f32) -> Measures<'static> {
        // Full row: 600 + 4 gaps of 10.
        Measures {
            budget,
            full: Some(640.0),
            fulls: Some(&FULLS),
            icons: Some(&ICONS),
            gap: 10.0,
            more: 20.0,
            keep: &KEEP,
            tiles: &TILES,
        }
    }

    fn name(i: usize) -> &'static str {
        ["Add", "Edit", "Size", "Arrange", "Files"][i]
    }

    #[test]
    fn the_fold_order_is_the_table() {
        let order: Vec<String> = steps(&KEEP)
            .into_iter()
            .map(|(i, m)| format!("{} {:?}", name(i), m))
            .collect();
        assert_eq!(
            order,
            [
                "Files Icons",
                "Files Collapsed",
                "Arrange Icons",
                "Size Icons",
                "Edit Icons",
                "Add Icons",
                "Arrange Collapsed",
                "Edit Collapsed",
                "Add Collapsed",
            ]
        );
        // A keep group never tiles.
        assert!(
            steps(&KEEP)
                .iter()
                .all(|(i, m)| !(KEEP[*i] && *m == Mode::Collapsed))
        );
        // Modes after n steps.
        assert_eq!(
            modes(&KEEP, Level::Folded(3)),
            [
                Mode::Full,
                Mode::Full,
                Mode::Full,
                Mode::Icons,
                Mode::Collapsed
            ]
        );
        assert_eq!(modes(&KEEP, Level::Full), vec![Mode::Full; 5]);
        assert_eq!(modes(&KEEP, Level::Measure), vec![Mode::Icons; 5]);
    }

    #[test]
    fn the_first_state_that_fits_wins() {
        // Full width minus 30: the last group is icon-only (50 instead of 90).
        assert_eq!(choose_level(&mk(610.0), 610.0), Level::Folded(1));
        assert_eq!(choose_level(&mk(640.0), 640.0), Level::Full);
        // A tile is 40: Files Tile saves another 10.
        assert_eq!(choose_level(&mk(595.0), 595.0), Level::Folded(2));
        // Arrange goes icon-only before Edit or Add lose their labels.
        let w2 = row_width(&mk(0.0), &modes(&KEEP, Level::Folded(2))).unwrap();
        let w3 = row_width(&mk(0.0), &modes(&KEEP, Level::Folded(3))).unwrap();
        assert_eq!(choose_level(&mk(w3), w3), Level::Folded(3));
        assert!(w3 < w2);
        assert_eq!(
            modes(&KEEP, Level::Folded(3))[0],
            Mode::Full,
            "Add keeps its labels"
        );
        // Nothing fits: the most folded state.
        assert_eq!(choose_level(&mk(10.0), 10.0), Level::Folded(9));
    }

    #[test]
    fn icons_unmeasured_means_a_measuring_pass() {
        let mm = Measures {
            icons: None,
            ..mk(400.0)
        };
        assert_eq!(choose_level(&mm, 400.0), Level::Measure);
        assert_eq!(next_level(Level::Full, 640.0, &mm), Level::Measure);
    }

    #[test]
    fn a_wider_budget_never_gives_a_more_folded_level() {
        let mut last = usize::MAX;
        for budget in (50..700).map(|b| b as f32) {
            let r = rank(choose_level(&mk(budget), budget));
            assert!(r <= last, "budget {budget}: rank {r} after {last}");
            last = r;
        }
        assert_eq!(last, 0);
    }

    #[test]
    fn the_ladder_settles_without_flipping() {
        for budget in (50..700).map(|b| b as f32) {
            let mm = mk(budget);
            let width = |l: Level| match l {
                Level::Full => 640.0,
                other => row_width(&mm, &modes(&KEEP, other)).unwrap(),
            };
            for start in [
                Level::Full,
                Level::Measure,
                Level::Folded(1),
                Level::Folded(4),
                Level::Folded(9),
            ] {
                let mut l = start;
                let mut seen = Vec::new();
                for _ in 0..12 {
                    seen.push(l);
                    l = next_level(l, width(l), &mm);
                }
                assert_eq!(
                    seen[10], seen[11],
                    "budget {budget} from {start:?}: {seen:?}"
                );
                let last = seen[11];
                if last != Level::Folded(9) {
                    assert!(width(last) <= budget, "budget {budget}: {last:?}");
                }
            }
        }
    }

    #[test]
    fn a_stale_step_down_from_measure_goes_to_the_first_fold() {
        // The measures claim the measuring pass fits although it did not
        // (drawn is wider than the budget): the next level is Folded(1).
        let mm = mk(10_000.0);
        assert_eq!(next_level(Level::Measure, 20_000.0, &mm), Level::Folded(1));
        assert_eq!(next_level(Level::Full, 20_000.0, &mm), Level::Measure);
        assert_eq!(rank(Level::Measure) + 1, rank(Level::Folded(1)));
    }

    #[test]
    fn the_measuring_pass_is_not_a_resting_level() {
        let mm = mk(610.0);
        let drawn = row_width(&mm, &modes(&KEEP, Level::Measure)).unwrap();
        assert!(drawn <= 610.0);
        assert_eq!(next_level(Level::Measure, drawn, &mm), Level::Folded(1));
    }

    #[test]
    fn steps_up_only_with_room_to_spare() {
        let at = 640.0 - 40.0 - 10.0; // the Folded(2) width
        let w2 = row_width(&mk(0.0), &modes(&KEEP, Level::Folded(2))).unwrap();
        assert_eq!(w2, at);
        // Exactly fitting is not enough to step up from Folded(3).
        let w3 = row_width(&mk(0.0), &modes(&KEEP, Level::Folded(3))).unwrap();
        assert_eq!(next_level(Level::Folded(3), w3, &mk(w2)), Level::Folded(3));
        assert_eq!(
            next_level(Level::Folded(3), w3, &mk(w2 + MARGIN)),
            Level::Folded(2)
        );
    }

    #[test]
    fn a_change_in_the_last_pass_goes_to_the_most_folded_level() {
        let mm = mk(450.0);
        assert_eq!(decide(Level::Full, 640.0, &mm, true), Level::Folded(9));
        assert_ne!(decide(Level::Full, 640.0, &mm, false), Level::Folded(9));
        // Stepping up in the last pass is safe: the shown pass fits.
        assert_eq!(
            decide(Level::Folded(9), 200.0, &mk(700.0), true),
            Level::Full
        );
    }

    #[test]
    fn groups_without_a_tile_are_listed_behind_one_more_button() {
        let keep = [false, false];
        let fulls = [100.0, 100.0];
        let icons = [50.0, 50.0];
        let tiles = [None, None];
        let mm = Measures {
            budget: 0.0,
            full: Some(210.0),
            fulls: Some(&fulls),
            icons: Some(&icons),
            gap: 10.0,
            more: 20.0,
            keep: &keep,
            tiles: &tiles,
        };
        // Both listed: only the » button.
        assert_eq!(
            row_width(&mm, &[Mode::Collapsed, Mode::Collapsed]),
            Some(20.0)
        );
        assert_eq!(
            row_width(&mm, &[Mode::Full, Mode::Collapsed]),
            Some(100.0 + 20.0 + 10.0)
        );
    }
}
