//! "It didn't cut right": what to check, in order, for what went wrong.
//!
//! The person (or an agent) names the [`Symptom`]; [`checklist`] gives the
//! ordered [`Item`]s for it, worded for this job: the speed, force and
//! passes it cuts with now, the material's blade depth hint, the preset
//! notes, the last cut with this material that went well, the mirror
//! state of the objects and the mat. Items that FlatCraft can change carry
//! fixes in the shape of the import report ([`crate::report::Fix`]): one
//! registry command with absolute arguments, so the window's button and an
//! agent's `execute` do exactly the same, a stale click does no harm, and
//! each one is one undo step. Speed, force and passes fixes change the
//! job's overrides, never the material preset. A fix that cannot run now
//! is listed in `blocked` with the reason, so the window can dim it and
//! say why.
//!
//! `job.troubleshoot` returns the same checklist to agents.

use flatcraft_core::document::CutStrategy;
use flatcraft_device::{MaterialGroup, MaterialProfile, MediaKind};
use kurbo::{Rect, Vec2};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::cutlog::CutEntry;
use crate::job::{JobInput, JobObject};
use crate::patterns::{TEST_HOLE_MM, TEST_SQUARE_MM, is_test_shape};
use crate::report::{self, Code, Fix};

/// How far the test shape keeps from every object, mm.
pub const TEST_SHAPE_GAP_MM: f64 = 5.0;
/// How much "Force +2" and "Force -2" change the force.
pub const FORCE_STEP: u32 = 2;
/// How much "Slower" lowers the speed.
pub const SPEED_STEP: u32 = 2;

/// What went wrong. The names are stable: agents pass them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Symptom {
    /// The knife left lines, but the pieces do not come out.
    NotThrough,
    /// Ragged edges, torn corners, or the knife pulled the material.
    #[serde(rename = "tore_or_dragged")]
    TornOrDragged,
    /// The material or pieces shifted or lifted during the cut.
    PiecesMoved,
    /// The cut landed in the wrong place, at the wrong size, or backwards.
    WrongPlaceOrSize,
}

impl Symptom {
    pub const ALL: [Symptom; 4] = [
        Symptom::NotThrough,
        Symptom::TornOrDragged,
        Symptom::PiecesMoved,
        Symptom::WrongPlaceOrSize,
    ];

    /// The name agents use, like `not_through`.
    pub fn as_str(self) -> &'static str {
        match self {
            Symptom::NotThrough => "not_through",
            Symptom::TornOrDragged => "tore_or_dragged",
            Symptom::PiecesMoved => "pieces_moved",
            Symptom::WrongPlaceOrSize => "wrong_place_or_size",
        }
    }

    /// The button text.
    pub fn label(self) -> &'static str {
        match self {
            Symptom::NotThrough => "It did not cut through",
            Symptom::TornOrDragged => "It tore or dragged",
            Symptom::PiecesMoved => "Pieces moved",
            Symptom::WrongPlaceOrSize => "Wrong place or size",
        }
    }

    /// What the symptom looks like, for the button's hover card.
    pub fn description(self) -> &'static str {
        match self {
            Symptom::NotThrough => {
                "The knife left lines, but the pieces do not come out, or only in places."
            }
            Symptom::TornOrDragged => {
                "The edges are ragged, corners ripped, or the knife pulled the material along."
            }
            Symptom::PiecesMoved => {
                "The material or small pieces shifted or lifted while it cut, so later lines miss."
            }
            Symptom::WrongPlaceOrSize => {
                "The cut sits in the wrong spot on the material, has the wrong size, or comes out backwards."
            }
        }
    }
}

impl Symptom {
    /// The symptom with this name, as `job.troubleshoot` takes it.
    pub fn parse(name: &str) -> Option<Symptom> {
        Symptom::ALL.into_iter().find(|s| s.as_str() == name)
    }
}

/// What an item checks. The names are stable: agents match on them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Check {
    /// The person's notes on the plotter and material presets.
    Notes,
    /// The last cut with this material that went well, from the cut log.
    LastGood,
    /// The blade depth against the material's hint (a hint only: it is
    /// set by hand and never sent).
    BladeDepth,
    Force,
    Passes,
    Speed,
    /// A worn or dirty blade.
    Blade,
    /// The mat's grip (or the rollers without a mat).
    Mat,
    /// Which side of the material faces up, and which way it goes in.
    MaterialFace,
    /// Shapes and strips too small for the blade.
    TinyDetails,
    /// The cut order: holes and inner pieces before their outline.
    CutOrder,
    /// Mirror for iron-on.
    Mirror,
    /// The size of the design and the plotter's scale.
    Scale,
    /// Where 0,0 is, and the mat's calibration.
    Position,
    /// Cut a small test shape first.
    TestShape,
}

impl Check {
    /// The name agents see, like `blade_depth`.
    pub fn as_str(self) -> &'static str {
        match self {
            Check::Notes => "notes",
            Check::LastGood => "last_good",
            Check::BladeDepth => "blade_depth",
            Check::Force => "force",
            Check::Passes => "passes",
            Check::Speed => "speed",
            Check::Blade => "blade",
            Check::Mat => "mat",
            Check::MaterialFace => "material_face",
            Check::TinyDetails => "tiny_details",
            Check::CutOrder => "cut_order",
            Check::Mirror => "mirror",
            Check::Scale => "scale",
            Check::Position => "position",
            Check::TestShape => "test_shape",
        }
    }
}

/// A fix that cannot run now, and why: the window shows it dimmed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Blocked {
    pub label: String,
    pub reason: String,
}

/// One line of the checklist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    pub check: Check,
    /// A short title, like "Force".
    pub title: String,
    /// What to check, with this job's values, in plain words.
    pub message: String,
    /// Why it matters and any term explained, for the hover card.
    pub hint: String,
    /// True when FlatCraft sees that this is probably the cause (iron-on
    /// vinyl that is not mirrored, tiny shapes in the design).
    pub likely: bool,
    /// The objects it is about, for "Show me" (empty for the job).
    pub object_ids: Vec<u64>,
    /// What FlatCraft can change, ready to run.
    pub fixes: Vec<Fix>,
    /// Fixes that cannot run now, with the reason.
    pub blocked: Vec<Blocked>,
}

/// The last cut with this material that the person rated good.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastGood {
    /// The cut log entry.
    pub entry: String,
    /// When it was cut, RFC 3339 UTC.
    pub date: String,
    pub plotter_id: String,
    pub plotter: String,
    pub speed: u8,
    pub force: u8,
    pub passes: u32,
    /// The person's note on it (may be empty).
    pub note: String,
}

impl LastGood {
    pub fn of(e: &CutEntry) -> Self {
        Self {
            entry: e.id.clone(),
            date: e.date.clone(),
            plotter_id: e.plotter_id.clone(),
            plotter: e.plotter.clone(),
            speed: e.speed,
            force: e.force,
            passes: e.passes,
            note: e
                .outcome
                .as_ref()
                .map(|o| o.note.clone())
                .unwrap_or_default(),
        }
    }
}

/// The values the job cuts with now, and what the plotter takes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    pub material_id: String,
    pub material: String,
    pub plotter_id: String,
    pub plotter: String,
    pub speed: u32,
    pub force: u32,
    pub passes: u32,
    /// The material's own values, before the job overrides.
    pub material_speed: u32,
    pub material_force: u32,
    pub material_passes: u32,
    pub speed_range: [u32; 2],
    pub force_range: [u32; 2],
    pub passes_range: [u32; 2],
    pub blade_depth_hint: Option<String>,
}

/// The checklist for one symptom.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checklist {
    pub symptom: Symptom,
    /// The symptom's label, like "It did not cut through".
    pub title: String,
    /// One line on how to use the list.
    pub lead: String,
    pub settings: Settings,
    pub last_good: Option<LastGood>,
    /// What to check, the most likely first.
    pub items: Vec<Item>,
}

/// The mirror item's hint when some objects are mirrored and some not.
pub const MIXED_MIRROR: &str = "Some objects are mirrored and some are not. Turn the mirrored ones back (Mirror horizontally on them), then use Mirror for iron-on so the whole layout flips together.";

/// The mirror item's hint for a mixed layout on material that is not
/// iron-on.
pub const MIXED_MIRROR_PLAIN: &str = "Some objects are mirrored and some are not. Turn the mirrored ones back with Mirror horizontally.";
/// The mirror item when only the test shape is cut.
pub const ONLY_TEST_SHAPE: &str =
    "Only the test shape is set to cut, and it reads the same either way.";

/// Why the mirror fix waits while part of the layout is off the mat.
pub const OFF_MAT_MIRROR: &str =
    "Part of the design is outside the cutting area. Move it onto the mat in Prepare first.";

/// The lead line of every checklist.
pub const LEAD: &str = "Work from the top: the first things are the most likely. Change one thing at a time, then cut a test shape.";

/// The words every override fix ends with.
const OVERRIDE_TAIL: &str = "Only this job changes, not the material preset. Undo takes it back.";

/// The checks for `symptom`, in the order to work through them. Notes and
/// the last good cut come first when there are any.
pub fn order(symptom: Symptom) -> &'static [Check] {
    use Check::*;
    match symptom {
        Symptom::NotThrough => &[
            Notes,
            LastGood,
            BladeDepth,
            Force,
            Passes,
            Blade,
            Speed,
            Mat,
            MaterialFace,
            TestShape,
        ],
        Symptom::TornOrDragged => &[
            Notes,
            LastGood,
            BladeDepth,
            Force,
            Speed,
            Blade,
            TinyDetails,
            Mat,
            MaterialFace,
            TestShape,
        ],
        Symptom::PiecesMoved => &[
            Notes,
            LastGood,
            Mat,
            Speed,
            CutOrder,
            TinyDetails,
            Blade,
            TestShape,
        ],
        Symptom::WrongPlaceOrSize => {
            &[Notes, Scale, Position, Mirror, MaterialFace, Mat, TestShape]
        }
    }
}

/// The settings the job cuts with: the same rule as the job build (the
/// override, else the material for this plotter).
pub fn settings(input: &JobInput) -> Settings {
    let (job, m, p) = (&input.job, &input.material, &input.plotter);
    Settings {
        material_id: m.id.clone(),
        material: m.name.clone(),
        plotter_id: p.id.clone(),
        plotter: p.name.clone(),
        speed: u32::from(job.speed.unwrap_or(m.speed)),
        force: u32::from(job.force.unwrap_or(m.force)),
        passes: job.passes.unwrap_or(m.passes),
        material_speed: u32::from(m.speed),
        material_force: u32::from(m.force),
        material_passes: m.passes,
        speed_range: p.speed_range.map(u32::from),
        force_range: p.force_range.map(u32::from),
        passes_range: [p.passes_range[0].max(1), p.passes_range[1]],
        blade_depth_hint: m.blade_depth_hint.clone(),
    }
}

/// True for heat transfer (iron-on) vinyl, which is cut from the back and
/// so mirrored: a preset in the Iron-on group (the built-in ones and every
/// preset made from them inherit it), or a name that says so.
pub fn iron_on(m: &MaterialProfile) -> bool {
    let name = m.name.to_lowercase();
    m.group == MaterialGroup::IronOn
        || ["heat transfer", "iron-on", "iron on", "htv"]
            .iter()
            .any(|w| name.contains(w))
}

/// The Cut stage's reminder for iron-on material: the checklist's mirror
/// item when the material is iron-on and some object that is cut is not
/// mirrored (nothing mirrored, or a mixed layout). `None` otherwise. It
/// never blocks a cut: heat transfer vinyl cut the right way round is a
/// mistake only on the shirt.
pub fn iron_on_reminder(input: &JobInput) -> Option<Item> {
    if !iron_on(&input.material) {
        return None;
    }
    let it = mirror(input);
    it.likely.then_some(it)
}

/// Where the test shape would go now: the top-left of a free spot in the
/// cutting area, `None` when there is no room. Every object counts (also
/// hidden and skipped ones: they may be cut later), except the test shape
/// itself, which moves there.
pub fn test_shape_spot(input: &JobInput) -> Option<kurbo::Point> {
    let area = cutting_area(input)?;
    let taken: Vec<Rect> = input
        .objects
        .iter()
        .filter(|o| !is_test_shape(&o.name, &o.geometry))
        .filter_map(|o| report::object_bounds(input, o))
        .collect();
    crate::placement::free_spot(
        area,
        &taken,
        Vec2::new(TEST_SQUARE_MM, TEST_SQUARE_MM),
        TEST_SHAPE_GAP_MM,
    )
}

fn cutting_area(input: &JobInput) -> Option<Rect> {
    crate::commands::effective_media(&input.plotter, &input.job)
        .ok()
        .and_then(|(m, _)| input.plotter.cutting_area(&m.id))
}

/// "1 pass", "3 passes".
fn passes_text(n: u32) -> String {
    if n == 1 {
        "1 pass".into()
    } else {
        format!("{n} passes")
    }
}

/// The name of a cut strategy, as the window shows it.
pub fn strategy_name(s: CutStrategy) -> &'static str {
    match s {
        CutStrategy::Quality => "Quality",
        CutStrategy::Fast => "Fast",
        CutStrategy::OutlinesLast => "Outlines last",
    }
}

fn item(check: Check, title: &str, message: String, hint: &str) -> Item {
    Item {
        check,
        title: title.to_string(),
        message,
        hint: hint.to_string(),
        likely: false,
        object_ids: Vec::new(),
        fixes: Vec::new(),
        blocked: Vec::new(),
    }
}

fn override_fix(label: &str, field: &str, value: u32, what: &str) -> Fix {
    Fix {
        label: label.to_string(),
        description: format!("Sets the {what} of this job to {value}. {OVERRIDE_TAIL}"),
        command: "job.set_overrides".into(),
        args: json!({ field: value }),
    }
}

fn blocked(label: &str, reason: String) -> Blocked {
    Blocked {
        label: label.to_string(),
        reason,
    }
}

/// The checklist for `symptom` on this job. `last_good` is the newest cut
/// with this material rated good ([`crate::cutlog::CutLog::last_good`]).
pub fn checklist(input: &JobInput, symptom: Symptom, last_good: Option<&CutEntry>) -> Checklist {
    let set = settings(input);
    let last_good = last_good.map(LastGood::of);
    let mut items: Vec<Item> = order(symptom)
        .iter()
        .filter_map(|c| make(*c, symptom, input, &set, last_good.as_ref()))
        .collect();
    // Iron-on that is not mirrored is the most likely cause of a design
    // that comes out backwards: it goes first.
    if symptom == Symptom::WrongPlaceOrSize
        && iron_on(&input.material)
        && let Some(i) = items
            .iter()
            .position(|it| it.check == Check::Mirror && it.likely)
    {
        let it = items.remove(i);
        items.insert(0, it);
    }
    Checklist {
        symptom,
        title: symptom.label().to_string(),
        lead: LEAD.to_string(),
        settings: set,
        last_good,
        items,
    }
}

fn make(
    check: Check,
    symptom: Symptom,
    input: &JobInput,
    set: &Settings,
    last_good: Option<&LastGood>,
) -> Option<Item> {
    Some(match check {
        Check::Notes => notes(input)?,
        Check::LastGood => last_good_item(set, last_good?),
        Check::BladeDepth => blade_depth(symptom, set),
        Check::Force => force(symptom, set),
        Check::Passes => passes(set),
        Check::Speed => speed(symptom, set),
        Check::Blade => item(
            Check::Blade,
            "Blade worn or dirty",
            "Take the blade out and look at the tip. Bits of material in the cap stop it from turning, and a worn tip drags and tears. Clean it, or put in a new blade.".into(),
            "FlatCraft cannot see the blade, so this one is up to you. A blade that cannot turn freely cuts like a butter knife.",
        ),
        Check::Mat => mat(input),
        Check::MaterialFace => item(
            Check::MaterialFace,
            "Material face and direction",
            "Put the material on the right side up: vinyl with its colour up, heat transfer vinyl with the shiny carrier sheet down. The top edge of the mat, where the grid's 0,0 is, goes into the plotter first, and the canvas shows the mat the same way up.".into(),
            "The face is the side the knife meets first. Cut from the wrong side, the knife goes into the backing or the carrier sheet instead of the material.",
        ),
        Check::TinyDetails => tiny_details(input),
        Check::CutOrder => cut_order(input),
        Check::Mirror => mirror(input),
        Check::Scale => scale(input),
        Check::Position => position(input),
        Check::TestShape => test_shape(input),
    })
}

fn notes(input: &JobInput) -> Option<Item> {
    let mut lines = Vec::new();
    let clean = |n: &Option<String>| {
        n.as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(str::to_string)
    };
    if let Some(n) = clean(&input.material.notes) {
        lines.push(format!("Your notes on {}: {n}", input.material.name));
    }
    if let Some(n) = clean(&input.plotter.notes) {
        lines.push(format!("Your notes on {}: {n}", input.plotter.name));
    }
    if lines.is_empty() {
        return None;
    }
    Some(item(
        Check::Notes,
        "Your notes",
        lines.join("\n"),
        "What you wrote on the plotter and material presets. Change the notes in Settings, in the presets.",
    ))
}

fn last_good_item(set: &Settings, g: &LastGood) -> Item {
    let on = if g.plotter_id == set.plotter_id {
        String::new()
    } else {
        format!(" (on {})", g.plotter)
    };
    let mut message = format!(
        "Last good cut with this material: force {}, {}, speed {}{on}.",
        g.force,
        passes_text(g.passes),
        g.speed
    );
    if !g.note.trim().is_empty() {
        message.push_str(&format!(" You noted: {}", g.note.trim()));
    }
    let mut it = item(
        Check::LastGood,
        "Last good cut",
        message,
        "From the cut log on this computer: the newest cut with this material that you rated as good. Rate a cut in the job settings, with the Rate button.",
    );
    let label = "Use these settings";
    let (s, f, p) = (u32::from(g.speed), u32::from(g.force), g.passes);
    let within = |v: u32, r: [u32; 2]| v >= r[0] && v <= r[1];
    if s == set.speed && f == set.force && p == set.passes {
        it.blocked.push(blocked(
            label,
            "This job already cuts with these settings.".into(),
        ));
    } else if !(within(s, set.speed_range)
        && within(f, set.force_range)
        && within(p, set.passes_range))
    {
        it.blocked.push(blocked(
            label,
            format!(
                "They do not fit {}: it takes speed {} to {}, force {} to {} and {} to {} passes.",
                set.plotter,
                set.speed_range[0],
                set.speed_range[1],
                set.force_range[0],
                set.force_range[1],
                set.passes_range[0],
                set.passes_range[1]
            ),
        ));
    } else {
        it.fixes.push(Fix {
            label: label.into(),
            description: format!(
                "Sets this job to speed {s}, force {f} and {}, as in your last good cut with {}. {OVERRIDE_TAIL}",
                passes_text(p),
                set.material
            ),
            command: "job.set_overrides".into(),
            args: json!({ "speed": s, "force": f, "passes": p }),
        });
    }
    it
}

fn blade_depth(symptom: Symptom, set: &Settings) -> Item {
    let mut message = match &set.blade_depth_hint {
        Some(h) => format!(
            "{} says {h}. Check the number on your blade's cap.",
            set.material
        ),
        None => format!(
            "{} has no blade depth hint. Start low and go up one step at a time.",
            set.material
        ),
    };
    message.push_str(match symptom {
        Symptom::TornOrDragged => " Too deep, and the tip tears thin material and drags.",
        _ => " Too shallow, and the tip never gets through.",
    });
    item(
        Check::BladeDepth,
        "Blade depth",
        message,
        "The blade depth is how far the tip sticks out of the blade. You set it by hand on the blade: FlatCraft cannot set it and sends nothing about it. The material's hint is where to start.",
    )
}

fn force(symptom: Symptom, set: &Settings) -> Item {
    let [lo, hi] = set.force_range;
    let now = format!(
        "Force is {} now (the material says {}, this plotter takes {lo} to {hi}).",
        set.force, set.material_force
    );
    let hint = "Force is how hard the blade presses. Too little and the cut does not go through; too much and it cuts into the backing and tears thin material.";
    if symptom == Symptom::TornOrDragged {
        let mut it = item(
            Check::Force,
            "Force",
            format!("{now} Too much force tears and drags. Lower it by {FORCE_STEP}."),
            hint,
        );
        let label = "Force -2";
        if set.force <= lo {
            it.blocked.push(blocked(
                label,
                format!("Force is at this plotter's minimum, {lo}. Check the blade depth: a blade set too deep tears even with little force."),
            ));
        } else {
            let v = set.force.saturating_sub(FORCE_STEP).max(lo);
            it.fixes.push(override_fix(label, "force", v, "force"));
        }
        it
    } else {
        let mut it = item(
            Check::Force,
            "Force",
            format!("{now} Raise it by {FORCE_STEP} and cut a test shape."),
            hint,
        );
        let label = "Force +2";
        if set.force >= hi {
            it.blocked.push(blocked(
                label,
                format!("Force is at this plotter's maximum, {hi}. Add a pass, or check the blade depth."),
            ));
        } else {
            let v = (set.force + FORCE_STEP).min(hi);
            it.fixes.push(override_fix(label, "force", v, "force"));
        }
        it
    }
}

fn passes(set: &Settings) -> Item {
    let [_, hi] = set.passes_range;
    let mut it = item(
        Check::Passes,
        "Passes",
        format!(
            "Cutting {} now (the material says {}). Thick material often needs 2 or 3: each pass cuts deeper in the same line.",
            passes_text(set.passes),
            set.material_passes
        ),
        "A pass is one trip of the knife along every line. More passes cut deeper without pressing harder, so thin details tear less than with more force.",
    );
    let label = "Add a pass";
    if set.passes >= hi {
        it.blocked.push(blocked(
            label,
            format!("This plotter takes at most {hi} passes."),
        ));
    } else {
        it.fixes.push(override_fix(
            label,
            "passes",
            set.passes + 1,
            "number of passes",
        ));
    }
    it
}

fn speed(symptom: Symptom, set: &Settings) -> Item {
    let [lo, hi] = set.speed_range;
    let why = match symptom {
        Symptom::NotThrough => "Thick or dense material cuts deeper when the blade moves slower.",
        Symptom::PiecesMoved => {
            "A fast blade can push light material and small pieces around. Slower holds them better."
        }
        _ => "Slower cuts tear less at corners and small details.",
    };
    let mut it = item(
        Check::Speed,
        "Speed",
        format!(
            "Speed is {} now (the material says {}, this plotter takes {lo} to {hi}). {why}",
            set.speed, set.material_speed
        ),
        "Speed is how fast the blade moves while it cuts. Slower takes longer but is gentler and more exact.",
    );
    let label = "Slower";
    if set.speed <= lo {
        it.blocked.push(blocked(
            label,
            format!("Speed is at this plotter's slowest, {lo}."),
        ));
    } else {
        let v = set.speed.saturating_sub(SPEED_STEP).max(lo);
        it.fixes.push(override_fix(label, "speed", v, "speed"));
    }
    it
}

fn media_name_kind(input: &JobInput) -> Option<(String, MediaKind, bool)> {
    crate::commands::effective_media(&input.plotter, &input.job)
        .ok()
        .map(|(m, _)| (m.name.clone(), m.kind, m.calibrated))
}

fn mat(input: &JobInput) -> Item {
    let hint =
        "The mat's grip (its tack) is what holds the material still while the knife pulls on it.";
    match media_name_kind(input) {
        Some((_, MediaKind::NoMat, _)) => item(
            Check::Mat,
            "Rollers",
            "There is no mat: the rollers move the sheet itself. Feed it in straight and check that both rollers sit on the material.".into(),
            "Without a mat nothing sticks the material down, so only the rollers hold it.",
        ),
        Some((name, MediaKind::Mat, _)) => item(
            Check::Mat,
            "Mat grip",
            format!("Press the material flat onto the {name}. If it lifts at a corner or slides, the mat has lost its grip: clean it, or use a new one."),
            hint,
        ),
        None => item(
            Check::Mat,
            "Mat grip",
            "Press the material flat onto the mat. If it lifts at a corner or slides, the mat has lost its grip: clean it, or use a new one.".into(),
            hint,
        ),
    }
}

fn tiny_details(input: &JobInput) -> Item {
    let min = report::min_feature_mm(input.plotter.blade_offset_mm);
    let found: Vec<report::Issue> = report::check(input, None)
        .issues
        .into_iter()
        .filter(|i| matches!(i.code, Code::Hairline | Code::BelowMinFeature))
        .collect();
    let base = format!(
        "Shapes smaller than {} and strips thinner than that tear or stay stuck with this blade.",
        report::mm(min)
    );
    let message = match found.as_slice() {
        [] => format!("{base} FlatCraft finds none in this design."),
        list => {
            let lines: Vec<&str> = list.iter().map(|i| i.message.as_str()).collect();
            format!("{base} FlatCraft finds these: {}", lines.join(" "))
        }
    };
    let mut it = item(
        Check::TinyDetails,
        "Tiny details",
        message,
        "The smallest shape a blade cuts cleanly depends on how far its tip sits from the middle of the blade (the blade offset). Make tiny parts bigger, or cut thin lines as single lines.",
    );
    it.likely = !found.is_empty();
    for i in found {
        for id in &i.object_ids {
            if !it.object_ids.contains(id) {
                it.object_ids.push(*id);
            }
        }
        it.fixes.extend(i.fixes);
    }
    it
}

fn cut_order(input: &JobInput) -> Item {
    let strategy = input.job.strategy;
    let mut message = format!(
        "FlatCraft cuts the holes and inner pieces of a shape before its outline, so a piece is still held while its inside is cut. This document cuts with the strategy {}.",
        strategy_name(strategy)
    );
    if strategy != CutStrategy::OutlinesLast {
        message.push_str(" The Outlines last strategy cuts every outline after every hole and inner piece on a mat, so the whole sheet holds together until the end; without a mat it does this per 50 mm feed band.");
    }
    let mut it = item(
        Check::CutOrder,
        "Cut order",
        message,
        "The strategy decides the order of the cuts. The Path stage shows the order with numbers on every line.",
    );
    let label = "Cut outlines last";
    if strategy == CutStrategy::OutlinesLast {
        it.blocked.push(blocked(
            label,
            "This document already uses Outlines last: every outline is cut after the holes and inner pieces.".into(),
        ));
    } else {
        it.fixes.push(Fix {
            label: label.into(),
            description: format!(
                "Sets the strategy of this document to Outlines last: on a mat every outline comes after every hole and inner piece; without a mat, per 50 mm feed band. It was {}. New documents still start with the strategy in Settings, Cutting. Undo takes it back.",
                strategy_name(strategy)
            ),
            command: "job.set_strategy".into(),
            args: json!({ "strategy": "outlines_last" }),
        });
    }
    it
}

fn mirror(input: &JobInput) -> Item {
    // The test shape is the same either way round: it never counts.
    let cut: Vec<_> = input
        .objects
        .iter()
        .filter(|o| o.visible && o.cut_enabled && !is_test_shape(&o.name, &o.geometry))
        .collect();
    let n = cut.len();
    let mirrored: Vec<u64> = cut
        .iter()
        .filter(|o| o.transform.mirror)
        .map(|o| o.id.0)
        .collect();
    let plain: Vec<u64> = cut
        .iter()
        .filter(|o| !o.transform.mirror)
        .map(|o| o.id.0)
        .collect();
    let m = mirrored.len();
    let hot = iron_on(&input.material);
    let objects = |k: usize| {
        if k == 1 {
            "1 object".to_string()
        } else {
            format!("{k} objects")
        }
    };
    let (message, likely, ids) = if n == 0 {
        (
            "Nothing is set to cut yet. Heat transfer vinyl (iron-on) is cut mirrored, because you cut it from the back; everything else is not.".to_string(),
            false,
            Vec::new(),
        )
    } else if hot && !plain.is_empty() {
        (
            format!(
                "{} is cut from the back, but {}, so {} backwards when you iron {} on. Mirror {} with the button, or in Prepare with Mirror horizontally.",
                input.material.name,
                if plain.len() == 1 && n == 1 {
                    "the object is not mirrored".to_string()
                } else if plain.len() == 1 {
                    format!("1 of {n} is not mirrored")
                } else {
                    format!("{} of {n} are not mirrored", plain.len())
                },
                if plain.len() == 1 {
                    "it comes out"
                } else {
                    "they come out"
                },
                if plain.len() == 1 { "it" } else { "them" },
                if plain.len() == 1 { "it" } else { "them" },
            ),
            true,
            plain,
        )
    } else if hot {
        (
            if n == 1 {
                "The object is mirrored, as heat transfer vinyl needs.".to_string()
            } else {
                format!(
                    "All {} are mirrored, as heat transfer vinyl needs.",
                    objects(n)
                )
            },
            false,
            Vec::new(),
        )
    } else if m > 0 {
        (
            format!(
                "{} of {n} {} mirrored. Only heat transfer vinyl (iron-on) is cut mirrored; on {} a mirrored design comes out backwards.",
                m,
                if m == 1 { "is" } else { "are" },
                input.material.name
            ),
            true,
            mirrored,
        )
    } else {
        (
            "Nothing is mirrored. Only heat transfer vinyl (iron-on) is cut mirrored, because you cut it from the back.".to_string(),
            false,
            Vec::new(),
        )
    };
    let mut it = item(
        Check::Mirror,
        "Mirror for iron-on",
        message,
        "Mirror flips a design left and right. Iron-on vinyl lies face down on the mat, so its design must be cut mirrored to read right on the shirt.",
    );
    it.likely = likely;
    // Iron-on vinyl is turned over as a whole sheet, so the whole layout
    // flips as one rigid group: every visible object, skipped ones too
    // (they are cut once Skip is off), and the test shape, which is
    // symmetric and follows whatever its own flag. Nothing can land on
    // another object. The state (none, all or mixed) counts the visible
    // design objects only; a mixed layout gets a hint instead of a button.
    let design: Vec<&JobObject> = input
        .objects
        .iter()
        .filter(|o| o.visible && !is_test_shape(&o.name, &o.geometry))
        .collect();
    let shapes: Vec<&JobObject> = input
        .objects
        .iter()
        .filter(|o| o.visible && is_test_shape(&o.name, &o.geometry))
        .collect();
    let design_ids: Vec<u64> = design.iter().map(|o| o.id.0).collect();
    let follow: Vec<u64> = shapes.iter().map(|o| o.id.0).collect();
    let none_mirrored = design.iter().all(|o| !o.transform.mirror);
    let all_mirrored = design.iter().all(|o| o.transform.mirror);
    let label = "Mirror for iron-on";
    let back = "Mirror back";
    let shown = if hot { label } else { back };
    let sheet = design_ids.len() + follow.len();
    // The group mirrors in place (its box stays), so the result is inside
    // the cutting area exactly when the objects are now.
    let off_ids: Vec<u64> = cutting_area(input)
        .map(|area| {
            design
                .iter()
                .chain(&shapes)
                .filter(|o| {
                    report::object_bounds(input, o)
                        .is_some_and(|b| !crate::commands::inside(b, area))
                })
                .map(|o| o.id.0)
                .collect()
        })
        .unwrap_or_default();
    let off_area = !off_ids.is_empty();
    // What the whole layout holds besides the cut design, for the fix's
    // card: only what is there.
    let skipped = design.iter().filter(|o| !o.cut_enabled).count();
    let extra = match (skipped > 0, !follow.is_empty()) {
        (true, true) => ", skipped ones and the test shape too",
        (true, false) => ", skipped ones too",
        (false, true) => ", the test shape too",
        (false, false) => "",
    };
    let mixed = !none_mirrored && !all_mirrored;
    let only_shape = n == 0 && shapes.iter().any(|o| o.cut_enabled);
    if only_shape {
        it.message = ONLY_TEST_SHAPE.into();
    }
    if likely && mixed {
        let hint = if hot {
            MIXED_MIRROR
        } else {
            MIXED_MIRROR_PLAIN
        };
        it.message.push(' ');
        it.message.push_str(hint);
        it.blocked.push(blocked(shown, hint.into()));
    } else if likely && off_area {
        it.blocked.push(blocked(shown, OFF_MAT_MIRROR.into()));
    } else if likely && hot {
        it.fixes.push(Fix {
            label: label.into(),
            description: format!(
                "Mirrors the whole layout ({}{extra}) left and right as one group, in place, so the design reads right once it is ironed on. A second click changes nothing. Undo takes it back.",
                objects(sheet)
            ),
            command: "object.mirror".into(),
            args: json!({ "ids": design_ids, "follow": follow, "axis": "horizontal", "mirrored": true }),
        });
    } else if likely {
        it.fixes.push(Fix {
            label: back.into(),
            description: format!(
                "Mirrors the whole layout ({}{extra}) back as one group, in place, so the design reads right on {}. A second click changes nothing. Undo takes it back.",
                objects(sheet),
                input.material.name
            ),
            command: "object.mirror".into(),
            args: json!({ "ids": design_ids, "follow": follow, "axis": "horizontal", "mirrored": false }),
        });
    } else if only_shape {
        it.blocked.push(blocked(label, ONLY_TEST_SHAPE.into()));
    } else if n == 0 {
        it.blocked
            .push(blocked(label, "Nothing is set to cut yet.".into()));
    } else if hot {
        it.blocked
            .push(blocked(label, "Every object is mirrored already.".into()));
    } else {
        it.blocked.push(blocked(
            label,
            format!(
                "{} is not iron-on vinyl, so it is cut the right way round. If you cut iron-on, pick a material from the Iron-on group.",
                input.material.name
            ),
        ));
    }
    // Off the mat: Show me points at what has to move.
    it.object_ids = if likely && off_area && !mixed {
        off_ids
    } else {
        ids
    };
    it
}

fn scale(input: &JobInput) -> Item {
    let p = &input.plotter;
    let mut message = format!(
        "Check the size of your design in Prepare, on the Layout tab. Some apps save SVG files at 72 or 96 dots per inch, so a design can come in at another size. Then cut the test shape and measure it: the square is {} wide and the hole {}.",
        report::mm(TEST_SQUARE_MM),
        report::mm(TEST_HOLE_MM)
    );
    if (p.scale_x - 1.0).abs() > 1e-9 || (p.scale_y - 1.0).abs() > 1e-9 {
        message.push_str(&format!(
            " {} stretches x by {} and y by {}.",
            p.name, p.scale_x, p.scale_y
        ));
    }
    message.push_str(" For an exact check, cut the test sheet in a new project (Home, New, Test patterns) and measure its two 100 mm rulers: 1 mm off there is 1 % off. If the plotter cuts a little too long or too short, change Scale x or Scale y of the plotter in Settings (save it as your own preset first).");
    item(
        Check::Scale,
        "Size",
        message,
        "Scale x and Scale y stretch every cut by a factor: 1 is true size, 1.01 is 1 % longer. Change them only after measuring a test cut.",
    )
}

fn position(input: &JobInput) -> Item {
    let hint = "FlatCraft measures every position from the 0,0 of the grid printed on the mat, x to the right and y down.";
    match media_name_kind(input) {
        Some((_, MediaKind::NoMat, _)) => item(
            Check::Position,
            "Position",
            "There is no mat: the rollers move the sheet, so a sheet that goes in at a slant cuts at a slant. Feed it in straight.".into(),
            hint,
        ),
        other => {
            let name = other
                .as_ref()
                .map_or("mat".to_string(), |(n, _, _)| n.clone());
            let mut message = format!(
                "FlatCraft places everything from the 0,0 of the grid printed on the {name}, not from the edge of the mat. Line your material up with the grid."
            );
            if other.as_ref().is_some_and(|(_, _, calibrated)| !calibrated) {
                message.push_str(" This mat is not calibrated yet, so FlatCraft keeps 5 mm inside the grid. The calibration wizard arrives in milestone 3.");
            }
            item(Check::Position, "Position", message, hint)
        }
    }
}

fn test_shape(input: &JobInput) -> Item {
    let mut it = item(
        Check::TestShape,
        "Cut a test shape first",
        format!(
            "Put a {} square with a {} hole on a free spot of the mat, away from your design, and cut it first: {}, is it the right size?",
            report::mm(TEST_SQUARE_MM),
            report::mm(TEST_HOLE_MM),
            // On a backing sheet the blade must not go through it.
            if input.material.backing {
                "does it peel off the backing cleanly, does the hole lift out, is the backing only lightly marked and not cut through"
            } else {
                "does it come out, does the hole drop out"
            }
        ),
        "Cut only the test shape sets everything else to Skip cutting for the next send. Afterwards, press Undo, or turn Skip cutting off in the Objects list or the right-click menu. Adding the test shape again moves the one already there, so you never get two.",
    );
    let label = "Add a test shape";
    if test_shape_spot(input).is_some() {
        it.fixes.push(Fix {
            label: label.into(),
            description: format!(
                "Adds a {} square with a {} hole, at least {} away from every object, as the object '{}'. If it is already there, it moves to the free spot. Undo takes it back.",
                report::mm(TEST_SQUARE_MM),
                report::mm(TEST_HOLE_MM),
                report::mm(TEST_SHAPE_GAP_MM),
                crate::patterns::TEST_SHAPE_NAME
            ),
            command: "tools.test_shape".into(),
            args: json!({}),
        });
    } else {
        it.blocked.push(blocked(
            label,
            format!(
                "There is no free spot for a {} square with {} around it. Move your design closer together, or pick a larger mat.",
                report::mm(TEST_SQUARE_MM),
                report::mm(TEST_SHAPE_GAP_MM)
            ),
        ));
    }
    let only = "Cut only the test shape";
    let shape = input
        .objects
        .iter()
        .find(|o| is_test_shape(&o.name, &o.geometry));
    let others: Vec<u64> = input
        .objects
        .iter()
        .filter(|o| o.visible && o.cut_enabled && !is_test_shape(&o.name, &o.geometry))
        .map(|o| o.id.0)
        .collect();
    match shape {
        None => it.blocked.push(blocked(only, "Add the test shape first.".into())),
        Some(o) if !(o.visible && o.cut_enabled) => it.blocked.push(blocked(
            only,
            "The test shape itself is hidden or set to Skip cutting. Show it and turn Skip cutting off first.".into(),
        )),
        Some(_) if others.is_empty() => it.blocked.push(blocked(
            only,
            "Only the test shape is set to cut already.".into(),
        )),
        Some(_) => {
            let n = others.len();
            it.fixes.push(Fix {
                label: only.into(),
                description: format!(
                    "Sets {} to Skip cutting, so the next send cuts only the test shape. To cut your design again, press Undo, or turn Skip cutting off in the Objects list or the right-click menu.",
                    if n == 1 { "the other object".to_string() } else { format!("the other {n} objects") }
                ),
                command: "object.set_cut".into(),
                args: json!({ "ids": others, "cut": false }),
            });
        }
    }
    it
}
