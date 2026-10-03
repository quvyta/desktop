//! The notifications: what the corner says, and the list of the recent ones that
//! opens against the dock.

use qframe::prelude::*;
use qframe::widgets::{EmptyState, Panel};

use super::{Desk, FLOOR, Msg};
use crate::apps::Diagnostic;
use crate::dock;
use crate::inbox::Notice;
use crate::notice;
use crate::wm::WindowId;

/// The id of the row the notification `index` is drawn on, so the keys can reach it.
fn notice_id(index: usize) -> String {
    format!("notice-{index}")
}

impl Desk {
    /// The notification one problem of the loaders becomes: the corner says it and the list keeps
    /// it, because a person who was reading something else still has to be able to find out why an
    /// entry of theirs is missing.
    pub(super) fn told(&mut self, diagnostic: &Diagnostic) -> Command<Msg> {
        let heading =
            if diagnostic.is_warning() { t!("diagnostic.title-warning") } else { t!("diagnostic.title-error") };
        self.inbox.add(Notice::desktop(heading, notice::sentence(diagnostic)));
        notice::toast(diagnostic)
    }

    /// Opening the list of recent notifications, or putting it away. Opening it reads them: the
    /// count is what has not been read, and the list is the reading.
    pub(super) fn on_notices(&mut self) -> Command<Msg> {
        self.inbox_open = !self.inbox_open;
        if !self.inbox_open {
            return self.body_focus();
        }
        self.more = false;
        self.inbox.read();
        // The keys land on the first notice that leads somewhere, so the list is walked and
        // answered without a mouse. A list of notices that lead nowhere takes no focus of its own.
        match self.leading() {
            Some(index) => Command::focus(notice_id(index)),
            None => Command::focus(FLOOR),
        }
    }

    /// The place in the list of the newest notice that has a window to bring forward.
    pub(super) fn leading(&self) -> Option<usize> {
        self.inbox.notices().position(|notice| notice.window.is_some())
    }

    /// The recent notifications, listed against the dock's row at the end the count that opens
    /// them stands on. The newest is first; one that came from a window is a way back to it, and
    /// one of qdesk's own is there to be read.
    pub(super) fn notices_view(&self, ui: &mut View<'_, Msg>) {
        let language = ui.env().i18n().active().to_owned();
        let rows: Vec<(String, Option<WindowId>)> = self
            .inbox
            .notices()
            .map(|notice| {
                // A notice that came from a window is led by that window's name, in the language on
                // screen; one of qdesk's own by the heading it was said with.
                let named = notice
                    .window
                    .and_then(|id| self.windows.get(id))
                    .map(|window| window.entry().name.get(&language).to_owned());
                let line = match named.or_else(|| notice.lead.clone()) {
                    Some(lead) => t!("notice.list-line", heading = lead.as_str(), body = notice.body.as_str()),
                    None => notice.body.clone(),
                };
                (line, notice.window)
            })
            .collect();
        // The surface is as wide as its longest line and no wider, up to the screen: a row cut in
        // the middle would lose the end of a sentence, which is where an exit code stands. The
        // paddings are read from the theme, so the width is the one that is really painted.
        // The fallbacks are the widgets' own, so a theme that sets no padding is measured the way
        // it is drawn: three cells each side for a surface, two for a button.
        let padding = |key: &str, bare: u16| {
            ui.env().theme().style(key, None, &[]).pair("padding").map_or(bare, |(_, sides)| sides)
        };
        let chrome = 2 * padding("panel", 3) + 2 * padding("button", 2);
        let empty = t!("notice.list-empty-hint");
        // A list with nothing in it is as wide as the sentence that says so.
        let longest =
            rows.iter().map(|(line, _)| qframe::text::width(line)).max().unwrap_or_else(|| qframe::text::width(&empty));
        let width = longest.saturating_add(chrome).min(ui.size().width.saturating_sub(2 * dock::EDGE));
        self.against_dock(ui, |ui| {
            ui.row(|ui| {
                ui.add_with(Panel::new().title(t!("notice.list")), |ui| {
                    if rows.is_empty() {
                        ui.add(EmptyState::new(t!("notice.list-empty")).message(empty));
                        return;
                    }
                    for (index, (line, window)) in rows.into_iter().enumerate() {
                        match window {
                            // A press does what a press on its toast did: brings the window forward.
                            Some(id) => {
                                ui.add(Button::new(line).on_press(Msg::Bring(id))).id(notice_id(index));
                            }
                            // Nothing to go back to, so nothing offers to go there.
                            None => {
                                ui.add(Text::new(line));
                            }
                        }
                    }
                })
                .width(Length::Cells(width));
                // The corner the dock's count stands in is where the toasts live; a list opened
                // under them would be read through them. It opens at the end the window list opens
                // at instead, so the two surfaces of the dock behave the same way.
                ui.spacer();
            })
            .fill_width();
        });
    }
}
