//! Home: start a new project, open one, or pick a recent one.

use super::{Action, FlatCraftApp, display_name, fonts, lock};
use egui::{RichText, Ui};

pub fn show(ui: &mut Ui, app: &FlatCraftApp, actions: &mut Vec<Action>) {
    let t = app.tokens;
    let recent = lock(&app.session).prefs.recent_projects.clone();
    ui.add_space(t.spacing * 6.0);
    ui.vertical_centered(|ui| {
        ui.set_max_width(520.0);
        ui.label(RichText::new("Welcome to FlatCraft").font(fonts::semibold(24.0)));
        ui.label(
            RichText::new("Measure twice, cut once. The plotter does the cutting.")
                .color(t.text_dim),
        );
        ui.add_space(t.spacing * 2.0);
        ui.horizontal(|ui| {
            if ui
                .add(egui::Button::new(
                    RichText::new("New").font(fonts::medium(15.0)),
                ))
                .on_hover_text("Start an empty project on the 12x12 inch mat")
                .clicked()
            {
                actions.push(Action::NewProject);
            }
            if ui
                .add(egui::Button::new(
                    RichText::new("Open").font(fonts::medium(15.0)),
                ))
                .on_hover_text("Open a FlatCraft project file")
                .clicked()
            {
                actions.push(Action::OpenDialog);
            }
        });
        ui.add_space(t.spacing * 3.0);
        ui.label(RichText::new("Recent projects").font(fonts::medium(15.0)));
        ui.add_space(t.spacing / 2.0);
        if recent.is_empty() {
            ui.label(RichText::new("No recent projects yet.").color(t.text_dim));
        }
        for path in recent {
            if ui
                .add(egui::Button::new(display_name(&path)).frame(false))
                .on_hover_text(path.display().to_string())
                .clicked()
            {
                actions.push(Action::Open(path));
            }
        }
    });
}
