//! The macOS menu bar: muda items built from a [`MenuModel`]. The model
//! says what the items are; this file only mirrors it into NSApp, and
//! updates titles, enabled and checked states when the next model differs.

use std::sync::mpsc;

use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};

use super::Action;
use super::menu_model::{Accel, Cmd, Entry, Leaf, MenuModel, Predef, Role};

/// The installed menu bar and the channel its clicks arrive on.
pub struct AppMenu {
    menu: Menu,
    events: mpsc::Receiver<MenuEvent>,
    /// The model the muda items show now.
    shown: MenuModel,
    items: Vec<Live>,
    subs: Vec<Submenu>,
    /// A click may have flipped a check mark that the model does not
    /// change: the next update writes every state again.
    stale: bool,
}

/// A muda item of a leaf, in model order.
enum Live {
    Plain(MenuItem),
    Check(CheckMenuItem),
}

/// The key of an accelerator, for the keys the menu uses.
pub fn code_of(key: egui::Key) -> Option<Code> {
    use egui::Key as K;
    Some(match key {
        K::A => Code::KeyA,
        K::B => Code::KeyB,
        K::C => Code::KeyC,
        K::D => Code::KeyD,
        K::E => Code::KeyE,
        K::F => Code::KeyF,
        K::G => Code::KeyG,
        K::H => Code::KeyH,
        K::I => Code::KeyI,
        K::J => Code::KeyJ,
        K::K => Code::KeyK,
        K::L => Code::KeyL,
        K::M => Code::KeyM,
        K::N => Code::KeyN,
        K::O => Code::KeyO,
        K::P => Code::KeyP,
        K::Q => Code::KeyQ,
        K::R => Code::KeyR,
        K::S => Code::KeyS,
        K::T => Code::KeyT,
        K::U => Code::KeyU,
        K::V => Code::KeyV,
        K::W => Code::KeyW,
        K::X => Code::KeyX,
        K::Y => Code::KeyY,
        K::Z => Code::KeyZ,
        K::Comma => Code::Comma,
        _ => return None,
    })
}

fn accelerator(a: Accel) -> Option<Accelerator> {
    let mut mods = Modifiers::META;
    if a.shift {
        mods |= Modifiers::SHIFT;
    }
    if a.alt {
        mods |= Modifiers::ALT;
    }
    Some(Accelerator::new(mods, code_of(a.key)?))
}

impl AppMenu {
    /// Builds the menu bar and makes it the app's menu. A click wakes egui.
    pub fn install(ctx: &egui::Context, model: MenuModel) -> muda::Result<Self> {
        let (menu, items, subs) = build(&model)?;
        let (tx, events) = mpsc::channel();
        let ctx = ctx.clone();
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            let _ = tx.send(e);
            ctx.request_repaint();
        }));
        Ok(Self {
            menu,
            events,
            shown: model,
            items,
            subs,
            stale: false,
        })
    }

    /// Shows `model`. Nothing happens when it is the one on screen; the
    /// items are changed in place when only their state differs, and the
    /// whole bar is built again when the items themselves changed (the
    /// recent projects).
    pub fn update(&mut self, model: MenuModel) {
        if model == self.shown && !self.stale {
            return;
        }
        if model.shape() != self.shown.shape() {
            match build(&model) {
                Ok((menu, items, subs)) => {
                    self.menu = menu;
                    self.items = items;
                    self.subs = subs;
                }
                Err(e) => {
                    eprintln!("warning: cannot update the menu bar: {e}");
                    return;
                }
            }
        } else {
            let (old, new) = (self.shown.leaves(), model.leaves());
            for ((live, was), now) in self.items.iter().zip(old).zip(new) {
                let all = self.stale;
                match live {
                    Live::Plain(i) => {
                        if all || was.title != now.title {
                            i.set_text(&now.title);
                        }
                        if all || was.enabled != now.enabled {
                            i.set_enabled(now.enabled);
                        }
                    }
                    Live::Check(i) => {
                        if all || was.title != now.title {
                            i.set_text(&now.title);
                        }
                        if all || was.enabled != now.enabled {
                            i.set_enabled(now.enabled);
                        }
                        if all || was.checked != now.checked {
                            i.set_checked(now.checked.unwrap_or(false));
                        }
                    }
                }
            }
            for ((s, was), now) in self.subs.iter().zip(self.shown.subs()).zip(model.subs()) {
                if self.stale || was.enabled != now.enabled {
                    s.set_enabled(now.enabled);
                }
            }
        }
        self.shown = model;
        self.stale = false;
    }

    /// The actions of the menu clicks since the last call. Save, Close and
    /// the like act on `active`, the document active when the click is
    /// handled.
    pub fn actions(&mut self, active: u64) -> Vec<Action> {
        let recent = &self.shown.recent;
        let clicks: Vec<MenuEvent> = self.events.try_iter().collect();
        if !clicks.is_empty() {
            // A check mark flips by itself on a click.
            self.stale = true;
        }
        clicks
            .iter()
            .filter_map(|e| Cmd::parse(e.id().as_ref()))
            .filter_map(|cmd| cmd.action(active, recent))
            .collect()
    }
}

type Built = (Menu, Vec<Live>, Vec<Submenu>);

fn build(model: &MenuModel) -> muda::Result<Built> {
    let menu = Menu::new();
    let mut items = Vec::new();
    let mut subs = Vec::new();
    let mut tops: Vec<(Submenu, Role)> = Vec::new();
    for top in &model.menus {
        let sm = Submenu::with_id(top.id.as_str(), &top.title, top.enabled);
        subs.push(sm.clone());
        fill(&sm, &top.entries, &mut items, &mut subs)?;
        menu.append(&sm)?;
        tops.push((sm, top.role));
    }
    for (sm, role) in &tops {
        match role {
            Role::Plain => {}
            Role::Window => sm.set_as_windows_menu_for_nsapp(),
            Role::Help => sm.set_as_help_menu_for_nsapp(),
        }
    }
    menu.init_for_nsapp();
    Ok((menu, items, subs))
}

fn fill(
    parent: &Submenu,
    entries: &[Entry],
    items: &mut Vec<Live>,
    subs: &mut Vec<Submenu>,
) -> muda::Result<()> {
    for e in entries {
        match e {
            Entry::Separator => parent.append(&PredefinedMenuItem::separator())?,
            Entry::Predef(p) => parent.append(&predef(*p))?,
            Entry::Item(l) => {
                let live = leaf(l);
                match &live {
                    Live::Plain(i) => parent.append(i)?,
                    Live::Check(i) => parent.append(i)?,
                }
                items.push(live);
            }
            Entry::Sub(s) => {
                let sm = Submenu::with_id(s.id.as_str(), &s.title, s.enabled);
                subs.push(sm.clone());
                fill(&sm, &s.entries, items, subs)?;
                parent.append(&sm)?;
            }
        }
    }
    Ok(())
}

fn leaf(l: &Leaf) -> Live {
    let accel = l.accel.and_then(accelerator);
    match l.checked {
        Some(on) => Live::Check(CheckMenuItem::with_id(
            l.cmd.id(),
            &l.title,
            l.enabled,
            on,
            accel,
        )),
        None => Live::Plain(MenuItem::with_id(l.cmd.id(), &l.title, l.enabled, accel)),
    }
}

fn predef(p: Predef) -> PredefinedMenuItem {
    match p {
        Predef::Services => PredefinedMenuItem::services(None),
        Predef::Hide => PredefinedMenuItem::hide(None),
        Predef::HideOthers => PredefinedMenuItem::hide_others(None),
        Predef::ShowAll => PredefinedMenuItem::show_all(None),
        Predef::Minimize => PredefinedMenuItem::minimize(None),
        Predef::Zoom => PredefinedMenuItem::maximize(Some("Zoom")),
        Predef::BringAllToFront => PredefinedMenuItem::bring_all_to_front(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::menu_model::MenuInput;
    use crate::ui::prepare::PrepareState;

    #[test]
    fn every_accelerator_of_the_model_has_a_muda_key() {
        let prepare = PrepareState::default();
        let model = MenuModel::build(&MenuInput {
            workspace: crate::ui::Workspace::Home,
            snap: None,
            prepare: &prepare,
            show_debug: false,
            can_undo: true,
            can_redo: true,
            text_focused: true,
            send_ready: true,
            recent: &[],
        });
        let mut n = 0;
        for l in model.leaves() {
            if let Some(a) = l.accel {
                n += 1;
                assert!(accelerator(a).is_some(), "{:?} has no muda key", l.cmd);
            }
        }
        assert!(n >= 14, "{n} accelerators");
    }
}
