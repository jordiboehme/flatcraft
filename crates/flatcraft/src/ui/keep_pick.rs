//! "Pick what stays" (Task 59): people think in "this part stays, that
//! part goes", not in mode names. The picker shows the kept and waste
//! overlay on the Prepare canvas, highlights the region under the pointer
//! (the shape of the selection or the sheet around it, by the same
//! even-odd rule as the planner: [`KeptMaterial::region_at`]) and lets a
//! click mark it as "stays". The marks give the keep mode
//! ([`flatcraft_path::keep_for`]); Done sets it with `object.set_keep` in
//! one undo step, Cancel and Escape change nothing.
//!
//! The overlay of every mode Done can give is built on the kept and waste
//! worker ([`super::kept::WORKER`]) from the plan's own rings, one mask per
//! mode, so a click only switches between finished masks and the frame
//! never builds one.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use egui::{Color32, RichText, Ui};
use flatcraft_core::document::{KeepMode, ObjectId};
use flatcraft_engine::job::JobResult;
use flatcraft_path::{KeptMaterial, Region, RegionHit, keep_for};
use serde_json::json;

use super::canvas::CanvasView;
use super::groups::card;
use super::kept::{KeptCache, Variant};
use super::prepare::Snapshot;
use super::settings::keep_name;
use super::theme::Tokens;
use super::{Action, fonts};

/// The button next to the Keep combo.
pub const BUTTON: &str = "Pick on the mat...";
/// The item of the object context menu.
pub const MENU_ITEM: &str = "Pick what stays...";
/// What the button and the menu item do, for their hover cards.
pub const HINT: &str = "Not sure which Keep mode you need? Click the part that stays after cutting, right on the mat: the shape, the sheet around it, or both. FlatCraft sets Keep for you (Pieces, Stencil or Both) and shows kept and waste while you pick. Done sets it, Cancel or Esc leaves everything as it was.";
/// Why the button and the menu item are dimmed without a selection.
pub const NOTHING_SELECTED: &str = "Select an object first.";
/// Why they are dimmed when the selection is not cut.
pub const NOT_CUT: &str = "Hidden and skipped objects are not cut, so nothing of them stays or goes. Show them or cut them again first.";

/// The heading of the floating bar.
pub const TITLE: &str = "Pick what stays";
/// The bar before anything is marked, and why Done is dimmed then.
pub const NOTHING_MARKED: &str = "Click the part you want to keep.";
/// The bar when the job cannot be planned.
pub const NO_PLAN: &str =
    "Picking needs a job that can be planned. The Cut workspace says what to fix.";
/// The card of Cancel.
pub const CANCEL_HINT: &str = "Leave Keep as it was. Esc does the same.";
/// The bar after "the sheet is waste" when another object on the mat
/// keeps the shared sheet (Stencil or Both).
pub const SHEET_KEPT_ELSEWHERE: &str =
    "Another object keeps the sheet on this mat, so the sheet stays anyway.";
/// The pointer is on a cut line or off the cutting area.
pub const POINT_AT: &str = "Point at the shape or at the sheet around it.";

/// The bar's sentence for the mode the marks give.
pub fn result_text(mode: Option<KeepMode>) -> &'static str {
    match mode {
        None => NOTHING_MARKED,
        Some(KeepMode::Pieces) => {
            "The shapes stay, the sheet around them is waste: Pieces, like stickers."
        }
        Some(KeepMode::Stencil) => {
            "The sheet stays, the shapes fall out as waste: Stencil, like a paint stencil."
        }
        Some(KeepMode::Both) => {
            "The shapes and the sheet around them stay: Both. Nothing is waste, so the blade only turns on the spot at these cuts."
        }
    }
}

/// What happens to the middle of a letter like O, for the mode the marks
/// give (`None`: nothing marked yet) on a material with or without a
/// backing sheet. It follows the planner's rule (`KeptMaterial::kept_by`):
/// Pieces wastes the middle; Stencil keeps it on the sheet, where a
/// backing holds it and paper lets it fall out; Both keeps it, like
/// everything around its cuts.
pub fn hole_text(mode: Option<KeepMode>, backing: bool) -> &'static str {
    match (mode, backing) {
        (Some(KeepMode::Pieces), _) => {
            "The middle of a letter like O is waste: you weed it out of the letter."
        }
        (Some(KeepMode::Stencil), true) => {
            "The middle of a letter like O stays on the backing sheet, and the transfer tape lifts it with the stencil."
        }
        (Some(KeepMode::Stencil), false) => flatcraft_engine::report::STENCIL_ISLAND_HINT,
        (Some(KeepMode::Both), true) => {
            "The middle of a letter like O stays too, like everything around these cuts."
        }
        (Some(KeepMode::Both), false) => {
            "The middle of a letter like O stays too, as a loose piece: on paper it falls out of the letter, so pick it up."
        }
        (None, true) => {
            "A letter like O has a middle. Keep the shape and the middle is waste. Keep the sheet and the middle stays on the backing sheet with the stencil."
        }
        (None, false) => {
            "A letter like O has a middle. Keep the shape and the middle is waste. Keep the sheet and the middle falls out of this paper stencil."
        }
    }
}

/// The bar's line about the region under the pointer.
pub fn hover_text(hit: Option<RegionHit>, marked: bool) -> String {
    let Some(hit) = hit else {
        return POINT_AT.to_string();
    };
    let what = match (hit.region, hit.hole) {
        (Region::Shape, _) => "the shape",
        (Region::Sheet, true) => "the middle of a letter, which counts as sheet",
        (Region::Sheet, false) => "the sheet around it",
    };
    let verb = if marked { "let it go" } else { "keep it" };
    format!("Under the pointer: {what}. Click to {verb}.")
}

/// Why picking cannot start on the selection, if it cannot.
pub fn why_not(snap: &Snapshot, ids: &[u64]) -> Option<&'static str> {
    let objs: Vec<_> = ids.iter().filter_map(|id| snap.object(*id)).collect();
    if objs.is_empty() {
        return Some(NOTHING_SELECTED);
    }
    (!objs.iter().any(|o| o.visible && o.cut_enabled)).then_some(NOT_CUT)
}

/// What the floating bar asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Done,
    Cancel,
}

/// The keep picker of the Prepare canvas, while it is on.
#[derive(Clone)]
pub struct KeepPick {
    pub doc_id: u64,
    /// The selection when picking started: what Done sets.
    pub ids: BTreeSet<u64>,
    /// The objects Done changes: the selection and its linked copies.
    /// Their shape is "the shape", so a click on a linked copy of the
    /// selection means its shape too.
    reach: Arc<BTreeSet<ObjectId>>,
    /// The shape is marked as staying.
    pub shape: bool,
    /// The sheet around it is marked as staying.
    pub sheet: bool,
    /// The region under the pointer, last frame.
    pub hover: Option<RegionHit>,
    /// A letter with a hole was hovered: the bar explains its middle.
    pub hole_seen: bool,
    /// Which objects of `reach` have a hole, found on a worker once per
    /// plan.
    holes: HoleCache,
    /// The overlay of each mode Done can give: Pieces, Stencil, Both.
    pub previews: [KeptCache; 3],
    /// The shape of the selection, for the highlight.
    pub shapes: KeptCache,
    /// Where the floating bar was drawn last frame (screen).
    bar_rect: Option<egui::Rect>,
}

impl std::fmt::Debug for KeepPick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeepPick")
            .field("doc_id", &self.doc_id)
            .field("ids", &self.ids)
            .field("shape", &self.shape)
            .field("sheet", &self.sheet)
            .field("hover", &self.hover)
            .finish_non_exhaustive()
    }
}

/// The index of a mode in [`KeepPick::previews`].
fn slot(mode: KeepMode) -> usize {
    match mode {
        KeepMode::Pieces => 0,
        KeepMode::Stencil => 1,
        KeepMode::Both => 2,
    }
}

impl KeepPick {
    /// The picker for the objects `ids` (the selection), nothing marked.
    /// `None` when they cannot be picked ([`why_not`]).
    pub fn start(snap: &Snapshot, ids: &[u64]) -> Option<Self> {
        if why_not(snap, ids).is_some() {
            return None;
        }
        let ids: BTreeSet<u64> = ids.iter().copied().collect();
        let reach = reach_of(snap, &ids);
        Some(Self {
            doc_id: snap.doc_id,
            ids,
            reach: Arc::new(reach),
            shape: false,
            sheet: false,
            hover: None,
            hole_seen: false,
            holes: HoleCache::default(),
            previews: Default::default(),
            shapes: KeptCache::default(),
            bar_rect: None,
        })
    }

    /// The keep mode the marks give, `None` with nothing marked.
    pub fn result(&self) -> Option<KeepMode> {
        keep_for(self.shape, self.sheet)
    }

    /// True while the picker still belongs to what is shown: the same
    /// document and the same selection. Anything else cancels it.
    pub fn still_on(&self, snap: &Snapshot) -> bool {
        snap.doc_id == self.doc_id && snap.selection == self.ids
    }

    /// Takes the linked copies of the selection again (an undo or an agent
    /// may have linked or unlinked them while picking): the previews, the
    /// hit test and Done follow.
    pub fn refresh(&mut self, snap: &Snapshot) {
        let reach = reach_of(snap, &self.ids);
        if *self.reach != reach {
            self.reach = Arc::new(reach);
        }
    }

    /// The objects Done changes: the selection and its linked copies.
    pub fn reach(&self) -> &BTreeSet<ObjectId> {
        &self.reach
    }

    /// True when an object outside the picker keeps the sheet (Stencil or
    /// Both): the sheet around the shape stays whatever the marks say.
    pub fn sheet_kept_elsewhere(&self, job: &JobResult) -> bool {
        job.plan
            .kept
            .sheet_kept_besides(|o| self.reach.contains(&o))
    }

    /// Marks `region` as staying, or unmarks it.
    pub fn toggle(&mut self, region: Region) {
        match region {
            Region::Shape => self.shape = !self.shape,
            Region::Sheet => self.sheet = !self.sheet,
        }
    }

    /// True when `region` is marked.
    pub fn marked(&self, region: Region) -> bool {
        match region {
            Region::Shape => self.shape,
            Region::Sheet => self.sheet,
        }
    }

    /// The variant of the plan the overlay shows for `mode`.
    pub fn variant(&self, mode: KeepMode) -> Variant {
        Variant::Modes(self.reach.clone(), mode)
    }

    /// The highlight's variant: the shape of the selection and its linked
    /// copies alone.
    pub fn shapes_variant(&self) -> Variant {
        Variant::Shapes(self.reach.clone())
    }

    /// True while a worker builds a mask or the hole flags of the picker.
    pub fn pending(&self) -> bool {
        self.shapes.pending() || self.previews.iter().any(KeptCache::pending) || self.holes.pending
    }

    /// The preview cache of `mode`.
    pub fn preview(&self, mode: KeepMode) -> &KeptCache {
        &self.previews[slot(mode)]
    }

    /// The region of the selection and its linked copies at `p` (mat mm)
    /// in `kept`.
    pub fn region_at(&self, kept: &KeptMaterial, p: kurbo::Point) -> Option<RegionHit> {
        kept.region_at(p, |o| self.reach.contains(&o))
    }

    /// True when the object of `hit` has a hole (the middle of an O). The
    /// flags come from the worker; until they are there only a point in
    /// the hole itself counts.
    fn has_hole(&mut self, ctx: &egui::Context, job: &Arc<JobResult>, hit: RegionHit) -> bool {
        if hit.hole {
            return true;
        }
        let holes = self.holes.of(ctx, job, &self.reach);
        hit.object.is_some_and(|o| holes.contains(&o))
    }

    /// The pointer on the canvas: what lies under it, and a click marks or
    /// unmarks it. `hover` is the pointer in screen points while it is
    /// over the canvas.
    pub fn pointer(
        &mut self,
        ctx: &egui::Context,
        job: Option<&Arc<JobResult>>,
        view: &CanvasView,
        hover: Option<egui::Pos2>,
        clicked: bool,
    ) {
        let hover = hover.filter(|at| !self.on_bar(*at));
        self.hover = match (job, hover) {
            (Some(job), Some(at)) => self.region_at(&job.plan.kept, view.screen_to_mm(at)),
            _ => None,
        };
        if let (Some(job), Some(hit)) = (job, self.hover)
            && self.has_hole(ctx, job, hit)
        {
            self.hole_seen = true;
        }
        if clicked && let Some(hit) = self.hover {
            self.toggle(hit.region);
        }
    }

    /// Draws the overlay of what Done would keep (the plan's own while
    /// nothing is marked), then the highlight of the region under the
    /// pointer. Every mask is asked for each frame, so all three modes
    /// are ready before the first click.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        ctx: &egui::Context,
        painter: &egui::Painter,
        view: &CanvasView,
        job: &Arc<JobResult>,
        area: kurbo::Rect,
        plan_overlay: &mut KeptCache,
        mat: [u8; 3],
        t: &Tokens,
    ) {
        for mode in [KeepMode::Pieces, KeepMode::Stencil, KeepMode::Both] {
            let v = self.variant(mode);
            self.previews[slot(mode)].mask_of(ctx, job, area, &v);
        }
        self.holes.of(ctx, job, &self.reach);
        match self.result() {
            Some(mode) => {
                let v = self.variant(mode);
                self.previews[slot(mode)].draw_of(ctx, painter, view, job, area, &v, mat, t);
            }
            None => plan_overlay.draw(ctx, painter, view, job, area, mat, t),
        }
        let v = self.shapes_variant();
        match self.hover {
            Some(hit) => {
                let tint = highlight(t);
                let waste = hit.region == Region::Sheet;
                self.shapes
                    .tint(ctx, painter, view, job, area, &v, waste, tint);
            }
            None => {
                self.shapes.mask_of(ctx, job, area, &v);
            }
        }
    }

    /// The action of Done: `object.set_keep` on the selection, `None`
    /// with nothing marked or when every object has the mode already.
    pub fn done_action(&self, snap: &Snapshot) -> Option<Action> {
        let mode = self.result()?;
        let same = self
            .reach
            .iter()
            .filter_map(|id| snap.object(id.0))
            .all(|o| o.keep == mode);
        (!same).then(|| {
            let ids: Vec<u64> = self.ids.iter().copied().collect();
            Action::run(
                self.doc_id,
                "object.set_keep",
                json!({ "ids": ids, "keep": mode.key() }),
            )
        })
    }

    /// The floating bar at the bottom right of the canvas `rect`: what the marks
    /// give in plain words, what lies under the pointer, the middle of a
    /// letter once one was hovered, and Done and Cancel.
    pub fn bar(
        &mut self,
        ui: &mut Ui,
        rect: egui::Rect,
        snap: &Snapshot,
        planned: bool,
        sheet_kept_elsewhere: bool,
        t: &Tokens,
    ) -> Option<Outcome> {
        let mut outcome = None;
        let width = BAR_WIDTH.min(rect.width() - 2.0 * t.spacing).max(160.0);
        let backing = snap.material.as_ref().is_some_and(|m| m.backing);
        let mode = self.result();
        // At the bottom right: designs mostly sit at the top left of the
        // mat, the pills at the bottom left. Laid out from the bottom up,
        // so the buttons stay where they are when a line above them
        // changes.
        let right = rect.right() - t.spacing;
        let max = egui::Rect::from_min_max(
            egui::pos2(right - width, rect.top()),
            egui::pos2(right, rect.bottom() - t.spacing),
        );
        let shown = ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(max)
                .layout(egui::Layout::bottom_up(egui::Align::Min)),
            |ui| {
                egui::Frame::new()
                    .fill(t.surface)
                    .stroke(egui::Stroke::new(1.0, t.accent_prepare))
                    .corner_radius(t.radius)
                    .inner_margin(t.spacing)
                    .show(ui, |ui| {
                        ui.set_width(width - 2.0 * t.spacing);
                        // Bottom up: the last line first.
                        ui.horizontal(|ui| {
                            let why = match (planned, mode) {
                                (false, _) => Some(NO_PLAN),
                                (true, None) => Some(NOTHING_MARKED),
                                (true, Some(_)) => None,
                            };
                            let r = ui.add_enabled(why.is_none(), egui::Button::new("Done"));
                            let what = match mode {
                                Some(m) => format!(
                                    "Set Keep to {} for the selection, in one step you can undo.",
                                    keep_name(m)
                                ),
                                None => "Set Keep to what you picked.".to_string(),
                            };
                            card(&r, "Done", &what, why, false);
                            if r.clicked() {
                                outcome = Some(Outcome::Done);
                            }
                            let cancel = egui::Button::new("Cancel").shortcut_text(
                                super::help::hint(super::help::PrepareKey::ClearSelection),
                            );
                            let r = ui.add(cancel);
                            card(&r, "Cancel", CANCEL_HINT, None, false);
                            if r.clicked() {
                                outcome = Some(Outcome::Cancel);
                            }
                        });
                        ui.add_space(t.spacing / 2.0);
                        if !planned {
                            ui.label(NO_PLAN);
                        } else {
                            if self.hole_seen {
                                ui.label(hole_text(mode, backing));
                            }
                            let marked = self.hover.is_some_and(|h| self.marked(h.region));
                            ui.label(
                                RichText::new(hover_text(self.hover, marked)).color(t.text_dim),
                            );
                            if sheet_kept_elsewhere && mode == Some(KeepMode::Pieces) {
                                ui.label(SHEET_KEPT_ELSEWHERE);
                            }
                            ui.label(result_text(mode));
                        }
                        ui.label(RichText::new(TITLE).font(fonts::semibold(14.0)));
                    })
                    .response
                    .rect
            },
        );
        self.bar_rect = Some(shown.inner);
        outcome
    }

    /// The floating bar's rect of the last frame (notices stay above it).
    pub fn bar_rect(&self) -> Option<egui::Rect> {
        self.bar_rect
    }

    /// True when `at` (screen) is on the floating bar of the last frame:
    /// the canvas below it neither highlights nor marks there.
    pub fn on_bar(&self, at: egui::Pos2) -> bool {
        self.bar_rect.is_some_and(|r| r.contains(at))
    }
}

/// The selection `ids` and the linked copies of its objects.
fn reach_of(snap: &Snapshot, ids: &BTreeSet<u64>) -> BTreeSet<ObjectId> {
    let groups: BTreeSet<u64> = ids
        .iter()
        .filter_map(|id| snap.object(*id)?.link_group)
        .collect();
    snap.objects
        .iter()
        .filter(|o| ids.contains(&o.id) || o.link_group.is_some_and(|g| groups.contains(&g)))
        .map(|o| ObjectId(o.id))
        .collect()
}

/// The objects with a hole, found on the kept and waste worker once per
/// plan and set of objects: the scan reads every ring, too slow for a
/// frame with a traced picture.
#[derive(Clone, Default)]
struct HoleCache {
    key: Option<(Arc<JobResult>, Arc<BTreeSet<ObjectId>>)>,
    slot: Arc<Mutex<Option<BTreeSet<ObjectId>>>>,
    pending: bool,
    holes: BTreeSet<ObjectId>,
}

impl HoleCache {
    /// The objects of `reach` with a hole in `job`'s plan, as far as known.
    fn of(
        &mut self,
        ctx: &egui::Context,
        job: &Arc<JobResult>,
        reach: &Arc<BTreeSet<ObjectId>>,
    ) -> &BTreeSet<ObjectId> {
        let same = self
            .key
            .as_ref()
            .is_some_and(|(j, r)| Arc::ptr_eq(j, job) && r == reach);
        if !same {
            self.key = Some((job.clone(), reach.clone()));
            self.holes.clear();
            self.pending = true;
            let slot: Arc<Mutex<Option<BTreeSet<ObjectId>>>> = Arc::default();
            self.slot = slot.clone();
            let kept = job.plan.kept.clone();
            let reach = reach.clone();
            let repaint = ctx.clone();
            let find = move || {
                let holes = reach
                    .iter()
                    .copied()
                    .filter(|o| kept.has_holes(*o))
                    .collect();
                if let Ok(mut s) = slot.lock() {
                    *s = Some(holes);
                }
                repaint.request_repaint();
            };
            if let Err(e) = std::thread::Builder::new()
                .name(super::kept::WORKER.into())
                .spawn(find)
            {
                eprintln!("flatcraft: cannot start the hole finder: {e}");
                self.pending = false;
            }
        }
        if self.pending
            && let Some(holes) = self.slot.lock().ok().and_then(|mut s| s.take())
        {
            self.holes = holes;
            self.pending = false;
        }
        &self.holes
    }
}

/// The widest the floating bar gets, in points.
pub const BAR_WIDTH: f32 = 420.0;

/// The colour of the highlighted region.
pub fn highlight(t: &Tokens) -> Color32 {
    t.accent_prepare.gamma_multiply(0.2)
}
