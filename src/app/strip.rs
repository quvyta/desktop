//! The desktop's side of the status strip (design 3.10): the items the dock draws from the last
//! reading, what a press on each does, the list of tmux sessions and the windows it opens.
//!
//! The readings themselves are the system gadget's: one probe and one task read the machine for
//! both, and the strip and the gadget draw the same reading.

use std::path::PathBuf;

use qframe::icons::Icons;
use qframe::prelude::*;
use qframe::widgets::{ContextItem, Panel};

use super::{Desk, Msg};
use crate::apps::{Category, Environment, Launch, find_program, is_executable};
use crate::dock::{self, Chip};
use crate::status::{self, Kind, Tone};

/// The programs a press on the processor or the memory opens, the first one on the `PATH` winning.
pub const MONITORS: [&str; 2] = ["btop", "htop"];

/// The id of the row of the `index`th session in the list of tmux sessions.
#[must_use]
pub fn session_id(index: usize) -> String {
    format!("tmux-{index}")
}

/// The program a press on the processor or the memory opens on the `PATH` of `apps`: `btop`, else
/// `htop`, else none, and those items do not answer a press.
#[must_use]
pub fn monitor(apps: &Environment) -> Option<PathBuf> {
    MONITORS.into_iter().find_map(|program| find_program(program, apps.path.as_deref(), is_executable))
}

impl Desk {
    /// The strip's items as the dock draws them, left to right; none while Settings has it off.
    pub(super) fn chips(&self, icons: &Icons) -> Vec<Chip> {
        if !self.prefs.status_strip {
            return Vec::new();
        }
        status::items(&self.status)
            .into_iter()
            .map(|item| {
                let mut text = icons.glyph(item.glyph_key).into_owned();
                text.push(' ');
                // A warning or an alarm is said by a mark as well as by its colour, the way the
                // system gadget's meters say it.
                let mark = match item.tone {
                    Tone::Calm => None,
                    Tone::Warn => Some("warning"),
                    Tone::Alert => Some("error"),
                };
                if let Some(mark) = mark {
                    text.push_str(&icons.glyph(mark));
                    text.push(' ');
                }
                // As wide as the widest number it can show, the number to the right: a changing
                // number never moves its neighbours.
                let widest = status::widest(item.kind).map_or(0, |widest| qframe::text::width(&widest));
                let pad = widest.saturating_sub(qframe::text::width(&item.text));
                text.extend(std::iter::repeat_n(' ', usize::from(pad)));
                text.push_str(&item.text);
                Chip { kind: item.kind, text, tone: item.tone }
            })
            .collect()
    }

    /// What a press on the strip's item of `kind` does: the tmux item lists the sessions, the
    /// processor and the memory open the machine's process monitor when it has one.
    pub(super) fn chip_press(&self, kind: Kind) -> Option<Msg> {
        match kind {
            Kind::Tmux => Some(Msg::TmuxSessions),
            Kind::Cpu | Kind::Memory => self.monitor.as_ref().map(|_| Msg::Monitor),
            Kind::Network | Kind::Battery => None,
        }
    }

    /// The menu a right click on the strip's item of `kind` opens: the tmux sessions, each one
    /// attaching a new window to it. The other items have none.
    pub(super) fn chip_menu(&self, kind: Kind) -> Vec<ContextItem<Msg>> {
        if kind != Kind::Tmux {
            return Vec::new();
        }
        self.status.tmux.iter().map(|name| ContextItem::new(name.clone(), Msg::Attach(name.clone()))).collect()
    }

    /// Whether the list of tmux sessions is on screen: asked for, with the strip on and sessions
    /// to list.
    pub(super) fn sessions_shown(&self) -> bool {
        self.tmux_open && self.prefs.status_strip && !self.status.tmux.is_empty()
    }

    /// What the strip's messages mean.
    pub(super) fn on_strip(&mut self, msg: Msg) -> Command<Msg> {
        match msg {
            Msg::TmuxSessions => {
                self.tmux_open = !self.tmux_open;
                if !self.tmux_open {
                    return self.body_focus();
                }
                self.more = false;
                self.inbox_open = false;
                // The keys land on the first session, so the list is answered without a mouse.
                Command::focus(session_id(0))
            }
            Msg::Attach(name) => self.attach(&name),
            Msg::Monitor => self.open_monitor(),
            _ => Command::none(),
        }
    }

    /// Opens a new terminal window attached to the tmux session `name`, with the tmux found on the
    /// environment's `PATH`, the same one the strip asked for the sessions.
    ///
    /// `TMUX` is emptied in the window: a qdesk started inside tmux passes it on, and tmux then
    /// refuses to attach, saying sessions should be nested with care. tmux takes an empty `TMUX`
    /// for none. The framework's program builder can set a variable but not take one away, so
    /// empty is as far as it goes.
    fn attach(&mut self, name: &str) -> Command<Msg> {
        self.tmux_open = false;
        let Some(tmux) = find_program("tmux", self.apps.path.as_deref(), is_executable) else {
            return self.body_focus();
        };
        let Some(program) = tmux.to_str() else { return self.body_focus() };
        let mut words = status::attach_command(name);
        program.clone_into(&mut words[0]);
        let mut entry = Self::made_entry("tmux", name, "prompt", Category::System, Launch::Command(words));
        entry.env.push(("TMUX".to_owned(), String::new()));
        let opened = self.open_entry(&entry, true);
        Command::batch([opened, self.body_focus()])
    }

    /// Opens the machine's process monitor in a new window, when it has one.
    fn open_monitor(&mut self) -> Command<Msg> {
        let Some(program) = self.monitor.clone() else { return Command::none() };
        let Some(words) = program.to_str().map(|path| vec![path.to_owned()]) else { return Command::none() };
        let name = program.file_name().map_or_else(|| "btop".to_owned(), |name| name.to_string_lossy().into_owned());
        let entry = Self::made_entry(&name, &name, "category-system", Category::System, Launch::Command(words));
        let opened = self.open_entry(&entry, true);
        Command::batch([opened, self.body_focus()])
    }

    /// The tmux sessions, listed against the dock's row at its right end, under the item that
    /// opened them. A press on one attaches a new window to it.
    pub(super) fn sessions_view(&self, ui: &mut View<'_, Msg>) {
        let names = self.status.tmux.clone();
        self.against_dock(ui, |ui| {
            ui.row(|ui| {
                ui.spacer();
                ui.add_with(Panel::new().title(Kind::Tmux.label()), |ui| {
                    for (index, name) in names.into_iter().enumerate() {
                        ui.add(Button::new(name.clone()).on_press(Msg::Attach(name))).id(session_id(index));
                    }
                });
                ui.spacer().width(Length::Cells(dock::EDGE));
            });
        });
    }
}
