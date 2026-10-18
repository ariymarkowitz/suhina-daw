// Copyright (C) 2026 The Suhina contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The macOS menu bar (#650): the system's own bar carries Suhina's menus, File, Edit, View and
//! the rest, built from the window's actions (`suhina_ui::settings::keymap::actions`), so every
//! item does what its key does. The window's own menu bar stays as it is for now; the system bar
//! is where a Mac user looks first.

use muda::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};

/// The installed menu bar, and which item runs which action.
pub struct MacMenu {
    /// Kept alive: the bar is AppKit's only while this is.
    _menu: Menu,
    items: Vec<(MenuId, String)>,
}

/// The action groups as menus, in the order the bar shows them; groups not named here (such as
/// the commands with no key of their own) are left out.
const MENUS: [&str; 7] = ["File", "Edit", "View", "Tracks", "Transport", "Session", "Help"];

/// Builds the bar and hands it to the application. Called on the main thread once the window is
/// up; `None` when AppKit would not take it.
pub fn install(actions: &[suhina_ui::settings::keymap::Action]) -> Option<MacMenu> {
    let menu = Menu::new();
    let app = Submenu::new("Suhina", true);
    app.append_items(&[
        &PredefinedMenuItem::about(None, None),
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::services(None),
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::hide(None),
        &PredefinedMenuItem::hide_others(None),
        &PredefinedMenuItem::show_all(None),
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::quit(None),
    ])
    .ok()?;
    menu.append(&app).ok()?;
    let mut items = Vec::new();
    for title in MENUS {
        let group: Vec<&suhina_ui::settings::keymap::Action> = actions.iter().filter(|a| a.group == title).collect();
        if group.is_empty() {
            continue;
        }
        let submenu = Submenu::new(title, true);
        for action in group {
            let item = MenuItem::new(&action.label, true, None);
            if submenu.append(&item).is_ok() {
                items.push((item.id().clone(), action.id.clone()));
            }
        }
        menu.append(&submenu).ok()?;
    }
    menu.init_for_nsapp();
    Some(MacMenu { _menu: menu, items })
}

impl MacMenu {
    /// The actions picked from the bar since the last call, in order.
    pub fn take(&self) -> Vec<String> {
        MenuEvent::receiver()
            .try_iter()
            .filter_map(|event| self.items.iter().find(|(id, _)| *id == event.id).map(|(_, action)| action.clone()))
            .collect()
    }
}
