//! Drawing the desktop: the dock's row at its end, and the floor with the windows
//! and every surface over it.

use qframe::prelude::*;
use qframe::widgets::{EmptyState, HelpLayer};

use super::{Desk, MIN_HEIGHT, MIN_WIDTH, Msg};
use crate::desktop;
use crate::dock;
use crate::settings::{self, DockPosition};
use crate::wm::Window;

impl Desk {
    pub(super) fn too_small(ui: &mut View<'_, Msg>) {
        let hint = t!("screen.too-small-hint", columns = MIN_WIDTH, rows = MIN_HEIGHT);
        ui.add(EmptyState::new(t!("screen.too-small")).message(hint)).fill();
    }

    /// The open windows of the workspace on screen as the dock shows them, in the order they were
    /// opened.
    pub(super) fn items(&self, ui: &View<'_, Msg>) -> Vec<dock::Item> {
        let language = ui.env().i18n().active();
        let icons = ui.env().icons();
        let mark = icons.glyph("window-minimize").into_owned();
        let called = icons.glyph("dot").into_owned();
        let focus = self.windows.focus();
        let mut windows: Vec<&Window> = self.windows.here().collect();
        windows.sort_by_key(|window| window.id());
        windows
            .into_iter()
            .map(|window| dock::Item {
                id: window.id(),
                focused: focus == Some(window.id()),
                label: dock::Label {
                    glyph: desktop::glyph_of(window.entry(), icons),
                    name: window.entry().name.get(language).to_owned(),
                    mark: window.is_minimized().then(|| mark.clone()),
                    attention: self.attention.contains(&window.id()).then(|| called.clone()),
                },
            })
            .collect()
    }

    pub(super) fn desktop_view(&self, ui: &mut View<'_, Msg>) {
        let clock = self.clock_text();
        let items = self.items(ui);
        let labels: Vec<dock::Label> = items.iter().map(|item| item.label.clone()).collect();
        let count = dock::count_label(&ui.env().icons().glyph("dot"), self.inbox.unread());
        let strip = self.chips(ui.env().icons());
        let texts: Vec<String> = strip.iter().map(|chip| chip.text.clone()).collect();
        // The plan measures the items as the buttons that draw them, so the row it plans is the
        // row that is painted whatever the theme's padding is.
        let padding = ui.env().theme().style("button", None, &[]).pair("padding").map_or(2, |(_, sides)| sides);
        let plan = dock::plan(ui.size().width, &self.machine_text(), &clock, &count, &labels, &texts, padding);
        let desktop_keys = self.keys;
        let workspaces = self.workspaces();
        let row = |ui: &mut View<'_, Msg>| match desktop_keys {
            // In desktop mode the dock's row says what the keys do.
            // It replaces the dock wherever the dock now is, so the row the person reads never
            // moves under them.
            Some(keys) => Self::hints_view(keys, ui),
            None => {
                let presses = dock::Presses {
                    space: &Msg::Workspace,
                    workspaces,
                    launcher: Msg::ToggleLauncher,
                    launcher_open: self.launcher.is_some(),
                    more: Msg::MoreWindows,
                    notices: Msg::Notices,
                    press: &|item: &dock::Item| Msg::Dock(item.id),
                    menu: &|item: &dock::Item| self.window_menu(item.id),
                    chip: &|chip: &dock::Chip| self.chip_press(chip.kind),
                    chip_menu: &|chip: &dock::Chip| self.chip_menu(chip.kind),
                };
                let parts = dock::Parts { clock: &clock, count: &count, items: &items, strip: &strip };
                dock::view(&plan, &parts, &presses, ui);
            }
        };
        // The shell's own header and footer are what keep the row out of the body: whichever end
        // the dock takes, the windows get every other row and nothing is drawn behind it.
        let shell = AppShell::new().body(|ui| self.body(&items, &plan, ui));
        match self.prefs.dock {
            DockPosition::Top => shell.header(row),
            DockPosition::Bottom => shell.footer(row),
        }
        .show(ui);
    }

    /// Puts what hangs off the dock — the launcher, the notification list, the list of the windows
    /// that do not fit — against the dock's own row, filling the rest of the desktop with space.
    ///
    /// The launcher rises from above the dock; a dock on the top row is above
    /// it instead, so the surface comes down from it. Either way it touches the row it belongs to
    /// and grows into the desktop.
    pub(super) fn against_dock(&self, ui: &mut View<'_, Msg>, build: impl FnOnce(&mut View<'_, Msg>)) {
        let top = self.prefs.dock == DockPosition::Top;
        ui.column(|ui| {
            if !top {
                ui.spacer();
            }
            build(ui);
            if top {
                ui.spacer();
            }
        })
        .fill();
    }

    /// The floor, with the windows over it and the launcher and the welcome line above them.
    pub(super) fn body(&self, items: &[dock::Item], plan: &dock::Plan, ui: &mut View<'_, Msg>) {
        ui.stack(|ui| {
            // The floor's colour and pattern from the settings, under the icons; the windows keep
            // the theme. A picture covers the whole floor, so nothing is laid under it, and its
            // colour and pattern are what shows wherever the picture cannot be.
            if self.wallpaper_drawn(ui.env()) {
                self.wallpaper_view(ui);
            } else {
                settings::ground(self.prefs.floor, self.prefs.floor_style, ui);
            }
            self.floor(ui);
            self.windows_view(ui);
            if self.more && plan.hidden > 0 {
                self.more_view(items, plan, ui);
            }
            if self.inbox_open {
                self.notices_view(ui);
            }
            if let Some(launcher) = &self.launcher {
                self.launcher_view(launcher, ui);
            }
            if self.first_start_shown() {
                self.first_start_view(ui);
            }
            if let Some(folder) = &self.folder {
                Self::naming_view(folder, ui);
            }
            if self.help {
                self.help_view(ui);
            }
            self.wallpaper_picker_view(ui);
        })
        .fill();
    }

    /// What the desktop can be told from the keyboard: the framework's help layer, which reads the
    /// keymap itself, so a rebound key is listed by the key it now has.
    ///
    /// The keys of the floor are not in any keymap — the icons are walked and opened by the grid
    /// itself — so they are given to the layer as the keys of this screen.
    pub(super) fn help_view(&self, ui: &mut View<'_, Msg>) {
        let icons = ui.env().icons();
        let arrow = |key: &str| icons.glyph(key).into_owned();
        let arrows =
            format!("{}{}{}{}", arrow("arrow-left"), arrow("arrow-up"), arrow("arrow-right"), arrow("arrow-down"));
        let layer = HelpLayer::new(Msg::Help)
            .hint(arrows.clone(), t!("help.icons-move"))
            .hint(format!("shift {arrows}"), t!("floor.help-move"))
            .hint("enter", t!("help.icons-open"))
            .hint("esc", t!("help.icons-clear"))
            .hint(t!("help.icons-jump"), t!("help.icons-jump-label"));
        // The picture viewer's keys are read by its body, not by any keymap: they are listed while
        // a viewer is the window in front.
        let viewer = self.windows.focused().is_some_and(|window| self.pictures.contains_key(&window.id()));
        let reader = self.windows.focused().is_some_and(|window| self.texts.contains_key(&window.id()));
        let layer = if reader { layer.hint("e", t!("texts.help-edit")) } else { layer };
        let layer = if viewer {
            layer
                .hint(format!("{} n space", arrow("arrow-right")), t!("pictures.help-next"))
                .hint(format!("{} p backspace", arrow("arrow-left")), t!("pictures.help-previous"))
                .hint("home end", t!("pictures.help-ends"))
                .hint("f", t!("pictures.help-fit"))
        } else {
            layer
        };
        ui.add(layer);
    }

    /// Opens the help layer, or puts it away and gives the keys back.
    pub(super) fn toggle_help(&mut self) -> Command<Msg> {
        self.help = !self.help;
        if self.help { Command::none() } else { self.body_focus() }
    }
}
