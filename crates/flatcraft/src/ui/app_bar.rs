//! The app bar: name, Home, File, Save, Undo, Redo and the document tabs on
//! the left; the agent pill, Settings, Help and the version on the right.

use std::time::Instant;

use egui::{RichText, Ui};

use super::icons::Icon;
use super::settings::set_name;
use super::{Action, FlatCraftApp, ICON_SIZE, Workspace, display_name, fonts, icon_button, lock};

/// One open document as the tabs show it.
struct Tab {
    id: u64,
    name: String,
    dirty: bool,
}

pub fn show(ui: &mut Ui, app: &FlatCraftApp, actions: &mut Vec<Action>) {
    let t = app.tokens;
    let (tabs, active, can_undo, can_redo) = {
        let s = lock(&app.session);
        let tabs: Vec<Tab> = s
            .docs
            .iter()
            .map(|d| Tab {
                id: d.id(),
                name: d.path.as_deref().map(display_name).unwrap_or_else(|| {
                    let n = app.ui.untitled.get(&d.id()).copied().unwrap_or(1);
                    format!("Untitled {n}")
                }),
                dirty: d.dirty,
            })
            .collect();
        let doc = s.doc();
        (tabs, doc.id(), doc.undo.can_undo(), doc.undo.can_redo())
    };
    let agent = app.agent_status(Instant::now());
    // The two sides are separate closures; the right one collects here.
    let mut right_actions = Vec::new();

    egui::Panel::top("flatcraft-app-bar")
        .frame(
            egui::Frame::new()
                .fill(t.panel)
                .inner_margin(egui::Margin::symmetric(
                    t.spacing as i8,
                    (t.spacing / 2.0) as i8,
                )),
        )
        .show(ui, |ui| {
            egui::Sides::new().shrink_left().show(
                ui,
                |ui| {
                    ui.label(RichText::new("FlatCraft").font(fonts::semibold(16.0)));
                    ui.add_space(t.spacing);
                    if icon_button(ui, Icon::Home, "Home", true).clicked() {
                        actions.push(Action::SetWorkspace(Workspace::Home));
                    }
                    ui.menu_button("File", |ui| {
                        if ui.button("New").clicked() {
                            actions.push(Action::NewProject);
                        }
                        if ui.button("Open").clicked() {
                            actions.push(Action::OpenDialog);
                        }
                        if ui.button("Save").clicked() {
                            actions.push(Action::Save { doc_id: active });
                        }
                        if ui.button("Save As").clicked() {
                            actions.push(Action::SaveAs { doc_id: active });
                        }
                    });
                    if icon_button(ui, Icon::Save, "Save", true).clicked() {
                        actions.push(Action::Save { doc_id: active });
                    }
                    if icon_button(ui, Icon::Undo, "Undo", can_undo).clicked() {
                        actions.push(Action::Undo(active));
                    }
                    if icon_button(ui, Icon::Redo, "Redo", can_redo).clicked() {
                        actions.push(Action::Redo(active));
                    }
                    ui.separator();
                    doc_tabs(ui, &tabs, active, actions);
                },
                |ui| {
                    let actions = &mut right_actions;
                    let version = egui::Label::new(
                        RichText::new(concat!("v", env!("CARGO_PKG_VERSION"))).color(t.text_dim),
                    )
                    .sense(egui::Sense::click());
                    let version = ui
                        .add(version)
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text("About FlatCraft");
                    if version.clicked() {
                        actions.push(Action::ShowAbout);
                    }
                    let help = icon_button(ui, Icon::Help, "Help", true);
                    egui::Popup::menu(&help).show(|ui| super::help::menu(ui, actions));
                    if icon_button(ui, Icon::Settings, "Settings", true).clicked() {
                        actions.push(Action::ShowProfiles);
                    }
                    if let Some(status) = agent {
                        let color = if status == "Agent connected" {
                            t.accent_prepare
                        } else {
                            t.text_dim
                        };
                        egui::Frame::new()
                            .stroke(egui::Stroke::new(1.0, color))
                            .corner_radius(t.radius * 2)
                            .inner_margin(egui::Margin::symmetric(t.spacing as i8, 2))
                            .show(ui, |ui| {
                                ui.label(RichText::new(status).color(color));
                            });
                    }
                },
            );
        });
    actions.extend(right_actions);
}

/// The narrowest a tab's name gets before tabs move into the "»" list.
pub const MIN_TAB_LABEL: f32 = 72.0;

/// The accessible name of the button that lists every open document.
pub const ALL_DOCUMENTS: &str = "All documents";

/// How the document tabs fit their room.
#[derive(Debug, Clone, PartialEq)]
pub struct TabFit {
    /// The widest a tab's name button may be; `None`: as wide as it likes.
    pub cap: Option<f32>,
    /// The tabs shown, in order.
    pub shown: Vec<usize>,
    /// Whether the "»" list is needed.
    pub more: bool,
}

/// Fits tabs whose name buttons are `natural` wide into `room`. Each tab
/// also takes `extra` (its close button and the spacing). First the names
/// get narrower (down to [`MIN_TAB_LABEL`]), then only the tabs that fit
/// at that width are shown, with the "»" button (`more` wide). The active
/// tab is always among them.
pub fn fit_tabs(natural: &[f32], extra: f32, room: f32, more: f32, active: usize) -> TabFit {
    if natural.is_empty() {
        return TabFit {
            cap: None,
            shown: Vec::new(),
            more: false,
        };
    }
    let total =
        |idx: &[usize], cap: f32| -> f32 { idx.iter().map(|&i| natural[i].min(cap) + extra).sum() };
    // The largest cap that fits `room`, by bisection.
    let cap_for = |idx: &[usize], room: f32| -> Option<f32> {
        let widest = idx.iter().map(|&i| natural[i]).fold(0.0, f32::max);
        if total(idx, widest) <= room {
            return None;
        }
        let (mut lo, mut hi) = (0.0f32, widest);
        for _ in 0..30 {
            let mid = (lo + hi) / 2.0;
            if total(idx, mid) <= room {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        Some(lo.floor())
    };
    let all: Vec<usize> = (0..natural.len()).collect();
    match cap_for(&all, room) {
        None => {
            return TabFit {
                cap: None,
                shown: all,
                more: false,
            };
        }
        Some(c) if c >= MIN_TAB_LABEL => {
            return TabFit {
                cap: Some(c),
                shown: all,
                more: false,
            };
        }
        Some(_) => {}
    }
    let room = room - more;
    // At least the active tab, even when not one tab fits at the narrowest
    // name: its name then gets narrower still, down to nothing, so the tab
    // never runs into the right side.
    let fits = ((room / (MIN_TAB_LABEL + extra)).floor().max(1.0) as usize).min(natural.len());
    let mut shown: Vec<usize> = (0..fits).collect();
    if active >= fits {
        shown[fits - 1] = active.min(natural.len() - 1);
    }
    let cap = cap_for(&shown, room).map(|c| c.max(0.0));
    TabFit {
        cap,
        shown,
        more: true,
    }
}

/// The document tabs, the "»" list when they do not all fit, and the New
/// document button.
fn doc_tabs(ui: &mut Ui, tabs: &[Tab], active: u64, actions: &mut Vec<Action>) {
    let pad = ui.spacing().button_padding.x;
    let gap = ui.spacing().item_spacing.x;
    let icon_w = ICON_SIZE + 2.0 * pad;
    let font = egui::TextStyle::Button.resolve(ui.style());
    let labels: Vec<String> = tabs
        .iter()
        .map(|t| {
            if t.dirty {
                format!("{} *", t.name)
            } else {
                t.name.clone()
            }
        })
        .collect();
    let natural: Vec<f32> = labels
        .iter()
        .map(|l| {
            let g =
                ui.painter()
                    .layout_no_wrap(l.clone(), font.clone(), egui::Color32::PLACEHOLDER);
            g.size().x + 2.0 * pad
        })
        .collect();
    let more_w = {
        let g = ui
            .painter()
            .layout_no_wrap("»".into(), font, egui::Color32::PLACEHOLDER);
        g.size().x + 2.0 * pad + gap
    };
    // Room for the tabs: all that is left but the New document button.
    let room = ui.available_width() - icon_w - gap;
    let active_ix = tabs.iter().position(|t| t.id == active).unwrap_or(0);
    let fit = fit_tabs(&natural, icon_w + 2.0 * gap, room, more_w, active_ix);
    for &i in &fit.shown {
        let tab = &tabs[i];
        let label = &labels[i];
        let mut button = egui::Button::selectable(tab.id == active, label.as_str());
        let r = match fit.cap {
            Some(cap) if natural[i] > cap => {
                button = button.truncate();
                ui.scope(|ui| {
                    ui.set_max_width(cap);
                    ui.add_sized(egui::vec2(cap, ui.spacing().interact_size.y), button)
                })
                .inner
            }
            _ => ui.add(button),
        };
        set_name(ui, &r, label);
        let r = r.on_hover_text(label.as_str());
        if r.clicked() && tab.id != active {
            actions.push(Action::Switch(tab.id));
        }
        let close = format!("Close {}", tab.name);
        if icon_button(ui, Icon::Close, &close, true).clicked() {
            actions.push(Action::Close(tab.id));
        }
    }
    if fit.more {
        let resp = ui.button("»").on_hover_text("Every open document");
        set_name(ui, &resp, ALL_DOCUMENTS);
        egui::Popup::menu(&resp).show(|ui| {
            for (tab, label) in tabs.iter().zip(&labels) {
                let r = ui.add(egui::Button::selectable(tab.id == active, label.as_str()));
                set_name(ui, &r, &format!("{label} (documents)"));
                if r.clicked() {
                    if tab.id != active {
                        actions.push(Action::Switch(tab.id));
                    }
                    ui.close();
                }
            }
        });
    }
    if icon_button(ui, Icon::Plus, "New document", true).clicked() {
        actions.push(Action::NewDocument);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_narrow_first_then_move_into_the_list() {
        let natural = [100.0; 4];
        // 4 x (100 + 20) = 480 fits.
        assert_eq!(fit_tabs(&natural, 20.0, 480.0, 20.0, 0).cap, None);
        // Narrower names, all shown.
        let f = fit_tabs(&natural, 20.0, 400.0, 20.0, 0);
        assert!(!f.more && f.shown.len() == 4);
        assert!(f.cap.unwrap() >= MIN_TAB_LABEL && f.cap.unwrap() <= 80.0);
        // Too narrow even at the minimum: the list, and the active tab shown.
        let f = fit_tabs(&natural, 20.0, 250.0, 20.0, 3);
        assert!(f.more);
        assert!(f.shown.contains(&3));
        let used: f32 = f
            .shown
            .iter()
            .map(|&i| natural[i].min(f.cap.unwrap_or(100.0)) + 20.0)
            .sum();
        assert!(used + 20.0 <= 250.0, "{f:?}");
    }

    #[test]
    fn a_room_too_small_for_one_tab_still_shows_the_active_one_narrower() {
        let natural = [100.0; 12];
        // Not even one tab fits at the narrowest name (72 + 20 + 20 > 100).
        let f = fit_tabs(&natural, 20.0, 100.0, 20.0, 7);
        assert_eq!(f.shown, [7]);
        assert!(f.more);
        let cap = f.cap.expect("narrower than its name");
        assert!(cap < MIN_TAB_LABEL && cap + 20.0 + 20.0 <= 100.0, "{f:?}");
        // No tabs at all, and no room: nothing to show, no panic.
        assert_eq!(
            fit_tabs(&[], 20.0, -5.0, 20.0, 0).shown,
            Vec::<usize>::new()
        );
        // No room at all: the active tab with an empty name.
        let f = fit_tabs(&natural, 20.0, 10.0, 20.0, 3);
        assert_eq!(
            (f.shown.as_slice(), f.cap),
            ([3usize].as_slice(), Some(0.0))
        );
    }
}
