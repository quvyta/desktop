//! The panel of the first start: the welcome line, and under it the recommended
//! applications this machine does not have yet, each with a button that installs it.
//!
//! qdesk itself stays small and installs nothing on its own. The panel only offers: a row's
//! Install starts the very install a card of the launcher's Installable shelf starts
//! ([`Desk::install`]), with its question when the installer itself is missing, and nothing
//! happens without that press. The panel is shown once; Settings opens it again.

use qframe::prelude::*;
use qframe::widgets::{Panel, SettingRow, SettingsList};

use super::{Desk, Msg, Target};
use crate::apps::Entry;
use crate::desktop;

/// The launcher entries the first start recommends, in the order they are offered: the file
/// explorer, a web browser that runs anywhere, and the Quvyta applications a person reaches for
/// first. An id the catalog has no entry for is not offered.
pub const RECOMMENDED: [&str; 5] = ["qexp", "w3m", "qcode", "qfocus", "qtools"];

/// The widget id of the list of recommended applications, which takes the keyboard when the panel
/// is opened from Settings.
pub const LIST: &str = "recommended-list";

/// The widest the panel is drawn, in cells: room for a name, a description and the button, and
/// no wider, so it reads as a note rather than a page and, on an 80-column screen, leaves the
/// names of the icons in the floor's first column in view.
const WIDTH: u16 = 60;

/// The screen height at or below which the welcome line leaves the panel to the rows: the dock's
/// launcher button is on screen all the same.
const SHORT: u16 = 12;

/// Cells kept free on either side of the panel on a screen narrower than [`WIDTH`].
const MARGIN: u16 = 2;

impl Desk {
    /// The recommended applications this machine does not have, in the order they are offered.
    fn recommended_missing(&self) -> Vec<&Entry> {
        RECOMMENDED
            .iter()
            .filter_map(|id| self.catalog.get(id))
            .filter(|entry| !self.catalog.is_installed(&entry.id))
            .collect()
    }

    /// Whether the recommended applications are on screen: once at the first start while any of
    /// them is missing, and whenever Settings asks for them.
    #[must_use]
    pub fn recommended(&self) -> bool {
        self.recommended_open || (!self.desktop.recommended_seen && !self.recommended_missing().is_empty())
    }

    /// Whether the panel of the first start is on screen at all.
    pub(super) fn first_start_shown(&self) -> bool {
        self.welcome() || self.recommended()
    }

    /// The panel is closed: neither the welcome line nor the recommended applications come back
    /// by themselves.
    pub(super) fn close_first_start(&mut self) -> Command<Msg> {
        self.desktop.welcome_seen = true;
        self.desktop.recommended_seen = true;
        self.recommended_open = false;
        self.save()
    }

    /// Install on a row of the panel: the panel makes way, so the window or the question the
    /// install opens stands in front of the person, and the install is the launcher's own.
    pub(super) fn install_recommended(&mut self, target: &Target) -> Command<Msg> {
        let saved = self.close_first_start();
        let installing = self.install(target);
        Command::batch([saved, installing])
    }

    /// The panel, in the middle of the floor over everything but the dialogs.
    pub(super) fn first_start_view(&self, ui: &mut View<'_, Msg>) {
        let size = ui.size();
        let narrow = Self::narrow(size);
        let width = WIDTH.min(size.width.saturating_sub(2 * MARGIN));
        let language = ui.env().i18n().active().to_owned();
        let icons = ui.env().icons();
        let launcher = icons.glyph(desktop::quvyta_icon(icons)).into_owned();
        let rows: Vec<(String, String, Target)> = self
            .recommended_missing()
            .into_iter()
            .map(|entry| {
                let label = format!("{} {}", desktop::glyph_of(entry, icons), entry.name.get(&language));
                let about = entry.comment.as_ref().map(|comment| comment.get(&language).to_owned());
                (label, about.unwrap_or_default(), Target::of(entry, &language))
            })
            .collect();
        let welcome = self.welcome();
        if !self.recommended() {
            // The welcome line alone, as narrow as its sentence.
            Self::centred(
                ui,
                Length::Auto,
                |ui| {
                    ui.add(Text::new(t!("welcome.line", icon = launcher.as_str())));
                    ui.add(Button::new(t!("welcome.close")).on_press(Msg::WelcomeSeen));
                },
                1,
            );
            return;
        }
        // A narrow screen keeps each row to its name and its button and closes the gaps, so every
        // row and the way out still fit above the dock; a floor too short even for that leaves the
        // welcome line to the dock's own button.
        let tight = narrow;
        let welcome = welcome && size.height > SHORT;
        Self::centred(
            ui,
            Length::Cells(width),
            |ui| {
                if welcome {
                    ui.add(Text::new(t!("welcome.line", icon = launcher.as_str()))).fill_width();
                }
                ui.column(|ui| {
                    ui.add(Text::new(t!("welcome.recommended")).bold()).fill_width();
                    if rows.is_empty() {
                        ui.add(Text::new(t!("welcome.recommended-none")).role("secondary")).fill_width();
                        return;
                    }
                    if !tight {
                        ui.add(Text::new(t!("welcome.recommended-text")).role("secondary")).fill_width();
                    }
                    SettingsList::show(ui, |list| {
                        for (label, about, target) in rows {
                            let row = if tight || about.is_empty() {
                                SettingRow::new(label)
                            } else {
                                SettingRow::new(label).description(about)
                            };
                            list.row(row, |ui| {
                                ui.add(Button::new(t!("welcome.install")).on_press(Msg::InstallRecommended(target)));
                            });
                        }
                    })
                    .id(LIST)
                    .fill_width();
                })
                .gap(0)
                .fill_width();
                ui.row(|ui| {
                    ui.add(Text::new(t!("welcome.again")).role("faint")).fill_width();
                    let close = if self.welcome() { t!("welcome.close") } else { t!("welcome.recommended-close") };
                    ui.add(Button::new(close).on_press(Msg::WelcomeSeen));
                })
                .gap(1)
                .fill_width();
            },
            if tight { 0 } else { 1 },
        );
    }

    /// A panel `width` wide in the middle of the floor, holding what `content` adds.
    fn centred(ui: &mut View<'_, Msg>, width: Length, content: impl FnOnce(&mut View<'_, Msg>), gap: u16) {
        ui.column(|ui| {
            ui.spacer();
            ui.row(|ui| {
                ui.spacer();
                ui.add_with(Panel::new().gap(gap), content).width(width);
                ui.spacer();
            })
            .fill_width();
            ui.spacer();
        })
        .fill();
    }
}
