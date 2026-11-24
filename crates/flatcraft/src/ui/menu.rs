//! The macOS menu bar: About, Preferences, Quit, and Edit with Undo and Redo.

use std::sync::mpsc;

use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{AboutMetadata, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use serde_json::json;

use super::Action;

const PREFERENCES: &str = "flatcraft.preferences";
const QUIT: &str = "flatcraft.quit";
const UNDO: &str = "flatcraft.undo";
const REDO: &str = "flatcraft.redo";

/// The installed menu bar and the channel its clicks arrive on.
pub struct AppMenu {
    _menu: Menu,
    events: mpsc::Receiver<MenuEvent>,
}

impl AppMenu {
    /// Builds the menu bar and makes it the app's menu. A click wakes egui.
    pub fn install(ctx: &egui::Context) -> muda::Result<Self> {
        let menu = Menu::new();
        let app = Submenu::new("FlatCraft", true);
        let about = AboutMetadata {
            name: Some("FlatCraft".into()),
            version: Some(env!("CARGO_PKG_VERSION").into()),
            license: Some("MPL-2.0".into()),
            website: Some(super::HELP_URL.into()),
            ..Default::default()
        };
        app.append_items(&[
            &PredefinedMenuItem::about(Some("About FlatCraft"), Some(about)),
            &PredefinedMenuItem::separator(),
            &MenuItem::with_id(
                PREFERENCES,
                "Preferences...",
                true,
                Some(Accelerator::new(Modifiers::META, Code::Comma)),
            ),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(None),
            &PredefinedMenuItem::separator(),
            &MenuItem::with_id(
                QUIT,
                "Quit FlatCraft",
                true,
                Some(Accelerator::new(Modifiers::META, Code::KeyQ)),
            ),
        ])?;
        let edit = Submenu::new("Edit", true);
        edit.append_items(&[
            &MenuItem::with_id(
                UNDO,
                "Undo",
                true,
                Some(Accelerator::new(Modifiers::META, Code::KeyZ)),
            ),
            &MenuItem::with_id(
                REDO,
                "Redo",
                true,
                Some(Accelerator::new(
                    Modifiers::META | Modifiers::SHIFT,
                    Code::KeyZ,
                )),
            ),
        ])?;
        menu.append_items(&[&app, &edit])?;
        menu.init_for_nsapp();

        let (tx, events) = mpsc::channel();
        let ctx = ctx.clone();
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            let _ = tx.send(e);
            ctx.request_repaint();
        }));
        Ok(Self {
            _menu: menu,
            events,
        })
    }

    /// The actions of the menu clicks since the last call.
    pub fn actions(&self) -> Vec<Action> {
        self.events
            .try_iter()
            .filter_map(|e| match e.id().as_ref() {
                PREFERENCES => Some(Action::ShowProfiles),
                QUIT => Some(Action::Quit),
                UNDO => Some(Action::Execute("edit.undo", json!({}))),
                REDO => Some(Action::Execute("edit.redo", json!({}))),
                _ => None,
            })
            .collect()
    }
}
