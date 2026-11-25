//! The macOS menu bar: About (FlatCraft's own dialog), Preferences, Quit,
//! Edit with Undo and Redo, and Help.

use std::sync::mpsc;

use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};

use super::Action;
use super::help::HelpItem;

const ABOUT: &str = "flatcraft.about";
const PREFERENCES: &str = "flatcraft.preferences";
const QUIT: &str = "flatcraft.quit";
const UNDO: &str = "flatcraft.undo";
const REDO: &str = "flatcraft.redo";

/// The menu id of a Help item.
fn help_id(item: HelpItem) -> String {
    format!("flatcraft.help.{}", item.label())
}

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
        app.append_items(&[
            &MenuItem::with_id(ABOUT, "About FlatCraft", true, None),
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
        let help = Submenu::new("Help", true);
        for item in HelpItem::ALL {
            help.append(&MenuItem::with_id(help_id(item), item.label(), true, None))?;
        }
        menu.append_items(&[&app, &edit, &help])?;
        help.set_as_help_menu_for_nsapp();
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

    /// The actions of the menu clicks since the last call. Undo and Redo
    /// act on `active`, the document active when the click is handled.
    pub fn actions(&self, active: u64) -> Vec<Action> {
        self.events
            .try_iter()
            .filter_map(|e| match e.id().as_ref() {
                ABOUT => Some(Action::ShowAbout),
                PREFERENCES => Some(Action::ShowProfiles),
                QUIT => Some(Action::Quit),
                UNDO => Some(Action::Undo(active)),
                REDO => Some(Action::Redo(active)),
                id => HelpItem::ALL
                    .into_iter()
                    .find(|&item| help_id(item) == id)
                    .map(HelpItem::action),
            })
            .collect()
    }
}
