//! Managing the windows: raising, moving and sizing them with the pointer or
//! the keys of desktop mode, closing them with a question while a program runs, tiling, the
//! workspaces, and leaving qdesk.

use std::time::Duration;

use qframe::prelude::*;
use qframe::runtime::Confirm;
use qframe::widgets::{ContextItem, FileView, KeyHints, Panel, Toast};

use super::{Arrow, Ask, Desk, FAR_STEP, FLOOR, Keys, Msg};
use crate::apps::Screen;
use crate::desktop;
use crate::dock;
use crate::settings::DragStyle;
use crate::wm::{self, Grip, SPACES, TooSmall, Window, WindowId, layout};

/// How long the note on resizing stays while the pointer is not on it: two sentences take longer
/// to read than a toast's usual five seconds.
const RESIZE_HINT_FOR: Duration = Duration::from_secs(12);

impl Desk {
    /// Brings the window `id` forward, from the dock or from a press on one of its notifications,
    /// going to its workspace when it is on another one.
    /// "Resize" on a window's menu: the window comes forward and desktop mode opens at its sizing
    /// step, where the arrows size it and the dock's row says the mouse does it too.
    pub(super) fn resize_from_keys(&mut self, id: WindowId) -> Command<Msg> {
        self.inbox_open = false;
        self.more = false;
        if !self.windows.bring(id) {
            return Command::none();
        }
        self.keys = Some(Keys::Resize);
        Command::focus(FLOOR)
    }

    /// The note on how a window is resized, the first time a window opens and never again: the
    /// edges that size a window say nothing of themselves until the pointer is over them.
    pub(super) fn resize_hint(&mut self) -> Command<Msg> {
        if self.desktop.resize_hint_seen {
            return Command::none();
        }
        self.desktop.resize_hint_seen = true;
        let toast = Toast::info(t!("window.resize-hint"))
            .body(t!("window.resize-hint-body"))
            .key("resize-hint")
            .duration(RESIZE_HINT_FOR);
        Command::batch([Command::toast(toast), self.save()])
    }

    pub(super) fn bring(&mut self, id: WindowId) -> Command<Msg> {
        // The list has been acted on, so it steps out of the way of the window it just raised.
        self.inbox_open = false;
        let away = self.windows.get(id).is_some_and(|window| window.space() != self.windows.current());
        if !self.windows.bring(id) {
            return Command::none();
        }
        if away {
            self.more = false;
        }
        self.body_focus()
    }

    /// Goes to the workspace `space` and gives the keys to the window that had them there, or to
    /// the floor: like opening a window from the launcher, going somewhere is a step out of desktop
    /// mode.
    pub(super) fn switch(&mut self, space: usize) -> Command<Msg> {
        if !self.windows.switch(space) {
            return Command::none();
        }
        // The list of the windows that did not fit named the windows of the workspace just left.
        self.more = false;
        self.keys = None;
        self.dragging = None;
        self.body_focus()
    }

    /// Sends the window `id` to the workspace `space`. Desktop mode stays: a person sending windows
    /// away is arranging them, and the next key is likely another such step.
    pub(super) fn send_to(&mut self, id: WindowId, space: usize) -> Command<Msg> {
        if !self.windows.send(id, space) {
            return Command::none();
        }
        if self.dragging.map(wm::Dragging::id) == Some(id) {
            self.dragging = None;
        }
        self.body_focus()
    }

    /// Leaving qdesk while programs are running: the question counts them. Asking
    /// again while it is on screen keeps the one question, instead of stacking a second.
    pub(super) fn ask_quit(&mut self) -> Command<Msg> {
        if self.leaving {
            return Command::none();
        }
        self.leaving = true;
        let running = self.programs.running();
        Command::confirm(
            Confirm::new(t!("window.quit-title"), Msg::Quit)
                .message(t!("window.quit-running", n = running))
                .confirm_label(t!("window.quit"))
                .danger()
                .on_cancel(Msg::KeepRunning),
        )
    }

    /// What the pointer did to a window.
    pub(super) fn on_window(&mut self, action: wm::Action) -> Command<Msg> {
        match action {
            // The press that raises a window is not taken away from what it landed on: the click
            // goes on to the body, and only the stacking order changes here.
            wm::Action::Focus(id) => {
                self.windows.raise(id);
                Command::none()
            }
            wm::Action::Move { id, by } => {
                self.drag_move(id, by);
                Command::none()
            }
            wm::Action::Resize { id, grip, by } => {
                self.drag_size(id, grip, by);
                Command::none()
            }
            wm::Action::Dropped(id) => {
                self.drop_window(id);
                Command::none()
            }
            wm::Action::Minimize(id) => self.minimize(id),
            wm::Action::ToggleMaximize(id) => {
                self.windows.toggle_maximized(id);
                Command::none()
            }
            // The mark on the title strip is the way a window is closed, so it asks about a
            // running program exactly as the dock's menu and the desktop mode's key do. It used to
            // close outright: every other way in asked, and this one — the obvious one — did not.
            wm::Action::Close(id) => self.ask_close(id),
        }
    }

    /// A step of a drag of the window `id`, which has gone `by` columns and rows since the button
    /// went down.
    ///
    /// A ghost drag leaves the window where it is and only the ghost follows the pointer; the
    /// window lands in one frame when the button comes up. That is one changed area a frame
    /// instead of every cell of the window, cheap enough to be the default everywhere, not only
    /// over a remote link.
    ///
    /// Either way the window or its ghost is placed from where the drag began by the whole way
    /// the pointer has gone, not a step at a time: held at the screen's edge or above the dock's
    /// row, it stays there until the pointer comes back over it.
    pub(super) fn drag_move(&mut self, id: WindowId, by: (i32, i32)) {
        let area = self.windows.area();
        let running = match self.dragging {
            Some(wm::Dragging::Moving { id: dragged, from } | wm::Dragging::Ghosting { id: dragged, from, .. })
                if dragged == id =>
            {
                Some(from)
            }
            _ => None,
        };
        let Some(from) = running.or_else(|| Some(wm::view::drag_start(self.windows.get(id)?, area))) else {
            return;
        };
        if self.prefs.drag == DragStyle::Live {
            if self.windows.move_from(id, from, by.0, by.1) {
                self.dragging = Some(wm::Dragging::Moving { id, from });
            }
            return;
        }
        self.dragging = Some(wm::Dragging::Ghosting { id, from, rect: layout::moved(from, by.0, by.1, area) });
    }

    /// A step of a drag of the edge or corner `grip` of the window `id`, which has gone `by`
    /// columns and rows since the button went down.
    ///
    /// The window is sized from the rectangle it had when the drag began, so the held edge stays
    /// under the pointer: one that stopped at the smallest size or at the screen does not start
    /// back until the pointer is over it again.
    pub(super) fn drag_size(&mut self, id: WindowId, grip: Grip, by: (i32, i32)) {
        let running = match self.dragging {
            Some(wm::Dragging::Sizing { id: held, grip: holding, from }) if held == id && holding == grip => Some(from),
            _ => None,
        };
        let Some(from) = running.or_else(|| Some(self.windows.get(id)?.rect())) else {
            return;
        };
        if self.windows.resize_from(id, from, grip, by.0, by.1) {
            self.dragging = Some(wm::Dragging::Sizing { id, grip, from });
        }
    }

    /// The end of a drag: a ghost lands, and a window that was moved against an edge snaps to it.
    pub(super) fn drop_window(&mut self, id: WindowId) {
        let dragged = self.dragging.take();
        match dragged {
            Some(wm::Dragging::Ghosting { id: dragged, from, rect }) if dragged == id => {
                // One movement from where the ghost started puts the window exactly where it
                // stood, whether it was floating, maximized or snapped to an edge.
                self.windows.move_by(id, rect.x - from.x, rect.y - from.y);
                self.windows.drop_dragged(id);
            }
            Some(wm::Dragging::Moving { id: dragged, .. }) if dragged == id => {
                self.windows.drop_dragged(id);
            }
            // A resize never snaps: an edge pulled to the screen's edge is being sized.
            _ => {}
        }
    }

    /// Takes the window `id` down to the dock.
    pub(super) fn minimize(&mut self, id: WindowId) -> Command<Msg> {
        if self.windows.minimize(id) { self.body_focus() } else { Command::none() }
    }

    /// What the close mark and `x` mean: a window whose program is still running is asked about
    /// first, with the framework's question and qdesk's own sentence; one whose program has
    /// ended, or that holds none, closes at once.
    pub(super) fn ask_close(&mut self, id: WindowId) -> Command<Msg> {
        if !self.programs.is_running(id) {
            return self.close_window(id);
        }
        let name = self.program_word(id);
        Command::confirm(
            Confirm::new(t!("window.close-title", name = name.as_str()), Msg::CloseAnyway(id))
                .message(t!("window.close-running", name = name.as_str()))
                .confirm_label(t!("window.close"))
                .danger(),
        )
    }

    /// Closes the window `id` and ends its program politely: it is sent the hangup a closing
    /// terminal window sends and killed if it has not gone when its grace is over.
    pub(super) fn close_window(&mut self, id: WindowId) -> Command<Msg> {
        if self.dragging.map(wm::Dragging::id) == Some(id) {
            self.dragging = None;
        }
        if !self.windows.close(id) {
            return Command::none();
        }
        self.programs.close(id);
        self.forget(id);
        if self.windows.is_empty() {
            self.more = false;
            self.keys = None;
        }
        self.body_focus()
    }

    /// What a press on a window item of the dock means: a minimized window comes back, the
    /// focused one goes down to the dock, and any other one comes forward.
    pub(super) fn on_dock(&mut self, id: WindowId) -> Command<Msg> {
        let Some(window) = self.windows.get(id) else {
            return Command::none();
        };
        if window.is_minimized() {
            self.windows.restore(id);
            return self.body_focus();
        }
        if self.windows.focus() == Some(id) {
            return self.minimize(id);
        }
        self.windows.raise(id);
        self.body_focus()
    }

    /// Lays every window out at once, or says why it cannot be done.
    pub(super) fn tile(&mut self) -> Command<Msg> {
        match self.windows.tile() {
            Ok(()) => Command::none(),
            Err(TooSmall::Narrow) => Command::toast(Toast::info(t!("window.tile-narrow"))),
            Err(TooSmall::Short { fits }) => Command::toast(Toast::info(t!("window.tile-short", fits = fits))),
        }
    }

    /// An arrow key while the desktop has the keys: it picks a window, or moves or sizes the
    /// picked one by one cell, or by [`FAR_STEP`] with shift.
    pub(super) fn on_arrow(&mut self, arrow: Arrow, far: bool) -> Command<Msg> {
        let Some(keys) = self.keys else {
            return Command::none();
        };
        let step = if far { i32::from(FAR_STEP) } else { 1 };
        let (dx, dy) = match arrow {
            Arrow::Left => (-step, 0),
            Arrow::Right => (step, 0),
            Arrow::Up => (0, -step),
            Arrow::Down => (0, step),
        };
        match keys {
            Keys::Pick => {
                let forward = matches!(arrow, Arrow::Right | Arrow::Down);
                if let Some(id) = wm::view::pick(&self.windows, self.windows.focus(), forward) {
                    self.windows.raise(id);
                }
            }
            Keys::Move => {
                if let Some(id) = self.windows.focus() {
                    self.windows.move_by(id, dx, dy);
                }
            }
            Keys::Resize => {
                // The keys move the right and the bottom edge, so a window sized from the keys
                // keeps its top left corner; the mouse can hold any edge or corner instead.
                if let Some(id) = self.windows.focus() {
                    let grip = if dx == 0 { Grip::Bottom } else { Grip::Right };
                    self.windows.resize_by(id, grip, dx, dy);
                }
            }
        }
        Command::none()
    }

    /// What Esc and the close action mean: they put away the topmost thing that is open, one step
    /// at a time, and the last step leaves desktop mode.
    pub(super) fn on_close(&mut self) -> Command<Msg> {
        if self.help {
            self.help = false;
            return self.body_focus();
        }
        if self.launcher.take().is_some() {
            return Command::focus(FLOOR);
        }
        if self.more {
            self.more = false;
            return Command::none();
        }
        if self.inbox_open {
            self.inbox_open = false;
            return self.body_focus();
        }
        match self.keys {
            // Moving or sizing a window ends where Enter ends it: back to picking windows.
            Some(Keys::Move | Keys::Resize) => {
                self.keys = Some(Keys::Pick);
                Command::none()
            }
            Some(Keys::Pick) => {
                self.keys = None;
                self.body_focus()
            }
            None => Command::none(),
        }
    }

    /// Every window of the desktop, from the back forwards, and the ghost over them.
    pub(super) fn windows_view(&self, ui: &mut View<'_, Msg>) {
        if self.windows.is_empty() {
            return;
        }
        let language = ui.env().i18n().active();
        let icons = ui.env().icons();
        // The names and the glyphs are read before the windows are drawn: drawing borrows the
        // view, and the language and the icon set live in it.
        let written: Vec<(WindowId, String, String)> = self
            .windows
            .visible()
            .map(|window| {
                (window.id(), window.entry().name.get(language).to_owned(), desktop::glyph_of(window.entry(), icons))
            })
            .collect();
        let find = |wanted: WindowId| written.iter().find(|(id, ..)| *id == wanted);
        let look = wm::Look { shadow: true, preview: wm::view::preview(&self.windows, self.dragging) };
        let strip = wm::view::Strip {
            name: &|window| find(window.id()).map_or_else(String::new, |(_, name, _)| name.clone()),
            glyph: &|window| find(window.id()).map_or_else(String::new, |(.., glyph)| glyph.clone()),
            subtitle: &|window| self.said(window.id()),
        };
        wm::view::view(&self.windows, &look, Msg::Window, &strip, &mut |window, ui| self.window_body(window, ui), ui);
    }

    /// What a window holds.
    pub(super) fn window_body(&self, window: &Window, ui: &mut View<'_, Msg>) {
        match window.body() {
            wm::Body::Screen => {
                if window.screen() == Some(Screen::Settings) {
                    self.settings_body(ui);
                } else if let Some(files) = self.files.get(&window.id()) {
                    self.files_body(window.id(), files, ui);
                } else if let Some(picture) = self.pictures.get(&window.id()) {
                    Self::picture_body(window.id(), picture, ui);
                } else if let Some(text) = self.texts.get(&window.id()) {
                    Self::text_body(window.id(), text, ui);
                }
            }
            wm::Body::Program(run) => self.program_body(window.id(), run, ui),
        }
    }

    /// The rows of a window's menu, on the dock and in desktop mode.
    ///
    /// A Files window adds the shapes its folder can be drawn in, the one in use marked with a
    /// sign rather than a colour alone.
    pub(super) fn window_menu(&self, id: WindowId) -> Vec<ContextItem<Msg>> {
        let mut items = vec![
            ContextItem::new(t!("window.minimize"), Msg::Ask(Ask::Minimize, id)),
            ContextItem::new(t!("window.maximize"), Msg::Ask(Ask::Maximize, id)),
            ContextItem::new(t!("window.resize"), Msg::Ask(Ask::Resize, id)),
            ContextItem::new(t!("window.tile"), Msg::Tile),
        ];
        // The other workspaces, each a row of its own: four is few enough to name them all, and a
        // submenu would hide the one thing the row is for.
        let here = self.windows.get(id).map_or(self.windows.current(), Window::space);
        items.push(ContextItem::gap());
        for space in (0..SPACES).filter(|space| *space != here) {
            items.push(ContextItem::new(t!("window.send-to", n = space + 1), Msg::SendTo(id, space)));
        }
        if let Some(files) = self.files.get(&id) {
            items.push(ContextItem::gap());
            for (view, label) in [
                (FileView::List, t!("files.view-list")),
                (FileView::Tree, t!("files.view-tree")),
                (FileView::Icons, t!("files.view-icons")),
            ] {
                let item = ContextItem::new(label, Msg::FilesView(id, view));
                items.push(if files.view == view { item.icon("check") } else { item });
            }
        }
        items.push(ContextItem::gap());
        items.push(ContextItem::new(t!("window.close"), Msg::Ask(Ask::Close, id)));
        items
    }

    /// The windows that do not fit on the dock, listed against its row.
    pub(super) fn more_view(&self, items: &[dock::Item], plan: &dock::Plan, ui: &mut View<'_, Msg>) {
        self.against_dock(ui, |ui| {
            ui.row(|ui| {
                ui.add_with(Panel::new().title(t!("dock.more")), |ui| {
                    for item in items.iter().skip(plan.shown) {
                        ui.add(Button::new(item.label.text(true)).selected(item.focused).on_press(Msg::Dock(item.id)))
                            .id(format!("more-{}", item.id.number()));
                    }
                });
                ui.spacer();
            });
        });
    }

    /// The workspaces as the dock's marks show them.
    pub(super) fn workspaces(&self) -> dock::Workspaces {
        let mut occupied = [false; SPACES];
        for (space, held) in occupied.iter_mut().enumerate() {
            *held = self.windows.occupied(space);
        }
        dock::Workspaces { current: self.windows.current(), occupied }
    }

    /// What the keys do while the desktop has them, in the dock's row.
    pub(super) fn hints_view(keys: Keys, ui: &mut View<'_, Msg>) {
        let icons = ui.env().icons();
        let arrow = |key: &str| icons.glyph(key).into_owned();
        let arrows =
            format!("{}{}{}{}", arrow("arrow-left"), arrow("arrow-up"), arrow("arrow-right"), arrow("arrow-down"));
        let hints = match keys {
            // A narrow row drops hints from its end (the framework's rule), so the order is what
            // matters least last: picking, moving, sizing, closing and the way back are the ones a
            // person cannot guess, and the rest is in the help layer.
            Keys::Pick => KeyHints::new()
                .hint(arrows, t!("mode.pick"))
                .hint("m", t!("mode.move"))
                .hint("r", t!("mode.resize"))
                .hint("x", t!("mode.close"))
                .hint("esc", t!("mode.leave"))
                // Going to a workspace leaves desktop mode, and the dock that comes back shows the
                // marks: the row needs no marks of its own, only the key.
                .hint(format!("1-{SPACES}"), t!("mode.workspace"))
                .hint("z", t!("mode.maximize"))
                .hint("n", t!("mode.minimize"))
                .hint("t", t!("mode.tile"))
                .hint("space", t!("mode.launcher"))
                .hint("b", t!("mode.notices")),
            Keys::Move => KeyHints::new()
                .hint(arrows, t!("mode.move"))
                .hint("shift", t!("mode.far", cells = FAR_STEP))
                .hint("enter", t!("mode.release"))
                .hint("esc", t!("mode.back")),
            // Sizing also says the mouse does it: this row is where a person who came from the
            // window's menu looks, and the edges say nothing until the pointer is over them. It
            // comes before shift, which a narrow row drops first and the help layer still lists.
            Keys::Resize => KeyHints::new()
                .hint(arrows, t!("mode.resize"))
                .hint(t!("mode.drag"), t!("mode.drag-label"))
                .hint("enter", t!("mode.release"))
                .hint("esc", t!("mode.back"))
                .hint("shift", t!("mode.far", cells = FAR_STEP)),
        };
        ui.add(hints).fill_width().height(Length::Cells(dock::HEIGHT));
    }

    /// Maximizes the window `id`, or gives it back the size it had.
    pub(super) fn maximize(&mut self, id: WindowId) -> Command<Msg> {
        self.windows.toggle_maximized(id);
        Command::none()
    }

    /// The answer "keep running" to the question about leaving qdesk.
    pub(super) fn keep_running(&mut self) -> Command<Msg> {
        self.leaving = false;
        Command::none()
    }

    /// Opens the list of the windows that do not fit on the dock, or puts it away.
    pub(super) fn more_windows(&mut self) -> Command<Msg> {
        self.more = !self.more;
        self.inbox_open = false;
        Command::none()
    }

    /// The key of desktop mode: the desktop takes the keys to pick a window, or gives them back.
    pub(super) fn desktop_mode(&mut self) -> Command<Msg> {
        if self.keys.is_some() {
            self.keys = None;
            return self.body_focus();
        }
        self.keys = Some(Keys::Pick);
        Command::focus(FLOOR)
    }

    /// A step of desktop mode: picking, moving or sizing, only while the desktop has the keys.
    pub(super) fn keys_step(&mut self, keys: Keys) -> Command<Msg> {
        if self.keys.is_some() {
            self.keys = Some(keys);
        }
        Command::none()
    }
}
