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

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

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
            "Both sides of every cut stay: Both. Nothing right next to these cuts is waste, so the blade only turns on the spot there."
        }
    }
}

/// What happens to the middle of a letter like O, for the mode the marks
/// give (`None`: nothing marked yet) on a material with or without a
/// backing sheet. It follows the planner's rule (`KeptMaterial::kept_by`):
/// Pieces wastes the middle; Stencil keeps it on the sheet, where a
/// backing holds it and paper lets it fall out; Both keeps only a band
/// along its cut.
pub fn hole_text(mode: Option<KeepMode>, backing: bool) -> &'static str {
    match (mode, backing) {
        (Some(KeepMode::Pieces), _) => {
            "The middle of a letter like O is waste: you weed it out of the letter."
        }
        (Some(KeepMode::Stencil), true) => {
            "The middle of a letter like O stays on the backing sheet, and the transfer tape lifts it with the stencil."
        }
        (Some(KeepMode::Stencil), false) => flatcraft_engine::report::STENCIL_ISLAND_HINT,
        (Some(KeepMode::Both), _) => {
            "The middle of a letter like O is waste, apart from a 1 mm band along its cut."
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
    /// The selected objects: their shape is "the shape".
    picked: Arc<BTreeSet<ObjectId>>,
    /// The objects Done changes: the selection and its linked copies.
    reach: Arc<BTreeSet<ObjectId>>,
    /// The shape is marked as staying.
    pub shape: bool,
    /// The sheet around it is marked as staying.
    pub sheet: bool,
    /// The region under the pointer, last frame.
    pub hover: Option<RegionHit>,
    /// A letter with a hole was hovered: the bar explains its middle.
    pub hole_seen: bool,
    /// Which objects have a hole, for the plan they were asked of.
    holes: Option<(Arc<JobResult>, HashMap<ObjectId, bool>)>,
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
        let groups: BTreeSet<u64> = ids
            .iter()
            .filter_map(|id| snap.object(*id)?.link_group)
            .collect();
        let reach = snap
            .objects
            .iter()
            .filter(|o| ids.contains(&o.id) || o.link_group.is_some_and(|g| groups.contains(&g)))
            .map(|o| ObjectId(o.id))
            .collect();
        Some(Self {
            doc_id: snap.doc_id,
            picked: Arc::new(ids.iter().map(|i| ObjectId(*i)).collect()),
            ids,
            reach: Arc::new(reach),
            shape: false,
            sheet: false,
            hover: None,
            hole_seen: false,
            holes: None,
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

    /// The highlight's variant: the selection's shape alone.
    pub fn shapes_variant(&self) -> Variant {
        Variant::Shapes(self.picked.clone())
    }

    /// True while a worker builds a mask of the picker.
    pub fn pending(&self) -> bool {
        self.shapes.pending() || self.previews.iter().any(KeptCache::pending)
    }

    /// The preview cache of `mode`.
    pub fn preview(&self, mode: KeepMode) -> &KeptCache {
        &self.previews[slot(mode)]
    }

    /// The region of the selection at `p` (mat mm) in `kept`.
    pub fn region_at(&self, kept: &KeptMaterial, p: kurbo::Point) -> Option<RegionHit> {
        kept.region_at(p, |o| self.picked.contains(&o))
    }

    /// True when the object of `hit` has a hole (the middle of an O),
    /// asked once per object and plan.
    fn has_hole(&mut self, job: &Arc<JobResult>, hit: RegionHit) -> bool {
        if hit.hole {
            return true;
        }
        let Some(o) = hit.object else {
            return false;
        };
        if !self
            .holes
            .as_ref()
            .is_some_and(|(j, _)| Arc::ptr_eq(j, job))
        {
            self.holes = Some((job.clone(), HashMap::new()));
        }
        let (_, holes) = self.holes.as_mut().expect("set just above");
        *holes.entry(o).or_insert_with(|| job.plan.kept.has_holes(o))
    }

    /// The pointer on the canvas: what lies under it, and a click marks or
    /// unmarks it. `hover` is the pointer in screen points while it is
    /// over the canvas.
    pub fn pointer(
        &mut self,
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
            && self.has_hole(job, hit)
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

    /// True when `at` (screen) is on the floating bar of the last frame:
    /// the canvas below it neither highlights nor marks there.
    pub fn on_bar(&self, at: egui::Pos2) -> bool {
        self.bar_rect.is_some_and(|r| r.contains(at))
    }
}

/// The widest the floating bar gets, in points.
pub const BAR_WIDTH: f32 = 420.0;

/// The colour of the highlighted region.
pub fn highlight(t: &Tokens) -> Color32 {
    t.accent_prepare.gamma_multiply(0.2)
}
