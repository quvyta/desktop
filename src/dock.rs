//! The dock: one row of the screen, at the bottom or, when the settings say so, at the top.
//!
//! Its left end holds the launcher button, then the open windows in the order they were opened,
//! and its right side the name of the machine and the clock. Which of them fit in a given width is
//! decided by [`plan`], a pure function, and drawn by [`view`] into whichever row it is given.
//! Nothing here knows which edge that is: the row is the same row at either end.

use qframe::prelude::*;
use qframe::text;
use qframe::widgets::{ContextItem, ContextMenu, IconButton};

use crate::desktop::family_icon;
use crate::wm::WindowId;

/// Rows the dock takes at its edge of the screen. It is always there, so the desktop has this
/// row less whichever edge it sits on.
pub const HEIGHT: u16 = 1;

/// Cells kept free at each end of the dock, so nothing touches the edge of the terminal.
pub const EDGE: u16 = 1;

/// Cells between two parts of the dock.
pub const GAP: u16 = 3;

/// Cells the launcher button takes: a space, its glyph and a space, in every glyph mode.
pub const LAUNCHER_WIDTH: u16 = 3;

/// Cells between two window items.
pub const ITEM_GAP: u16 = 1;

/// The id of the button that counts the unread notifications and opens their list.
pub const NOTICES: &str = "dock-notices";

/// What one window item says: the parts [`plan`] measures and [`view`] writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Label {
    /// The glyph of the application.
    pub glyph: String,
    /// The name of the application, in the language on screen.
    pub name: String,
    /// The mark a minimized window carries; `None` while the window is on the desktop. It stands
    /// before the attention mark, so a minimized window that also calls for attention reads the
    /// same way every time.
    ///
    /// A minimized window is told apart by a mark and not by its tone alone: meaning is never
    /// given by colour only (VISION §4.5), and a faint item on the dock's own tone is hard to see
    /// in 16 colours.
    pub mark: Option<String>,
    /// The mark of a window whose program rang the bell or sent a notification while the window
    /// did not have the keys (design 3.4); `None` once the window is focused again.
    pub attention: Option<String>,
}

impl Label {
    /// What the dock writes: the glyph, the name while the row has room for names, and the mark of
    /// a minimized window.
    #[must_use]
    pub fn text(&self, named: bool) -> String {
        let mut text = self.glyph.clone();
        if named {
            text.push(' ');
            text.push_str(&self.name);
        }
        for mark in [self.mark.as_deref(), self.attention.as_deref()].into_iter().flatten() {
            text.push(' ');
            text.push_str(mark);
        }
        text
    }
}

/// One open window, as the dock shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// The window, so a press knows which one it means.
    pub id: WindowId,
    /// Whether it is the window the keys go to.
    pub focused: bool,
    /// What the item says.
    pub label: Label,
}

/// What the dock shows in the width it has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The machine name as shown: whole, shortened in the middle, or empty when not even one
    /// cell is left for it.
    pub name: String,
    /// Whether the count of unread notifications is shown. It is never shown when there is
    /// nothing to count.
    pub count: bool,
    /// Whether the clock is shown.
    pub clock: bool,
    /// How many window items are drawn, from the first one opened.
    pub shown: usize,
    /// Whether the items show their names, or only their glyphs.
    pub named: bool,
    /// How many windows are left over; they are reached through the `+n` control.
    pub hidden: usize,
}

/// Which parts of the dock fit in `width` columns, for the machine name `name`, the clock text
/// `clock`, the unread count `count` (empty when nothing is unread), the open windows `labels` and
/// a button padding of `padding` columns on each side.
///
/// The machine name goes last. Over SSH it is what tells one server from another, and a wrong
/// guess there can mean a command run on the wrong machine; the time is on the person's own
/// computer anyway. So the clock goes first, and only when the name alone does not fit is it
/// shortened. It is cut in the middle, because host names usually differ at their end
/// (`web-prod-3`, `web-prod-4`) and that end must stay readable.
///
/// The count of unread notifications stands between the two. It gives way before the name and
/// after the clock, because it is the only way to the list — a number nobody can see is a message
/// nobody reads — while the time is on the person's own screen anyway. It is drawn as a button, so
/// it is measured as one.
///
/// The windows take what is left of the row after that. First every item shows its name; when they
/// do not all fit they fall back to their glyphs alone, and when even those do not fit the last
/// ones go behind a `+n` control that opens a list of them (design 3.4).
#[must_use]
pub fn plan(width: u16, name: &str, clock: &str, count: &str, labels: &[Label], padding: u16) -> Plan {
    let room = width.saturating_sub(2 * EDGE + LAUNCHER_WIDTH);
    let name_width = text::width(name);
    let count_width = if count.is_empty() { 0 } else { item_width(count, padding).saturating_add(GAP) };
    let with_clock = count_width.saturating_add(name_width).saturating_add(GAP).saturating_add(text::width(clock));
    let with_count = count_width.saturating_add(name_width);
    let (shown_name, count_shown, clock_shown, right) = if with_clock <= room {
        (name.to_owned(), count_width > 0, true, with_clock)
    } else if with_count <= room {
        (name.to_owned(), count_width > 0, false, with_count)
    } else if name_width <= room {
        (name.to_owned(), false, false, name_width)
    } else {
        let shortened = text::truncate_middle(name, room).into_owned();
        let used = text::width(&shortened);
        (shortened, false, false, used)
    };
    // The items keep a gap away from whatever stands at the right end.
    let left = room.saturating_sub(right).saturating_sub(if right == 0 { 0 } else { GAP });
    let (shown, named, hidden) = windows(left, labels, padding);
    Plan { name: shown_name, count: count_shown, clock: clock_shown, shown, named, hidden }
}

/// Cells an item of this label takes as a button: the label between two paddings.
fn item_width(label: &str, padding: u16) -> u16 {
    text::width(label).saturating_add(padding.saturating_mul(2))
}

/// Cells the row of `count` items of `widths` takes, gaps included.
fn row_width(widths: &[u16]) -> u16 {
    let gaps = u16::try_from(widths.len().saturating_sub(1)).unwrap_or(u16::MAX).saturating_mul(ITEM_GAP);
    widths.iter().fold(gaps, |sum, width| sum.saturating_add(*width))
}

/// How many windows the dock draws in `room` columns, whether with their names, and how many are
/// left behind the `+n` control.
fn windows(room: u16, labels: &[Label], padding: u16) -> (usize, bool, usize) {
    if labels.is_empty() {
        return (0, true, 0);
    }
    for named in [true, false] {
        let widths: Vec<u16> = labels.iter().map(|label| item_width(&label.text(named), padding)).collect();
        if row_width(&widths) <= room {
            return (labels.len(), named, 0);
        }
    }
    // Glyphs alone still do not fit: the last items go behind a control that says how many.
    let widths: Vec<u16> = labels.iter().map(|label| item_width(&label.text(false), padding)).collect();
    for shown in (1..labels.len()).rev() {
        let hidden = labels.len() - shown;
        let control = item_width(&more_label(hidden), padding);
        if row_width(&widths[..shown]).saturating_add(ITEM_GAP).saturating_add(control) <= room {
            return (shown, false, hidden);
        }
    }
    // Not even one glyph beside the control: the control alone still says how many windows are
    // open, and when there is no room for that either the dock keeps only its ends (design 3.7).
    let control = item_width(&more_label(labels.len()), padding);
    if control <= room { (0, false, labels.len()) } else { (0, false, 0) }
}

/// What the control at the end of the items says: how many windows it holds.
#[must_use]
pub fn more_label(hidden: usize) -> String {
    format!("+{hidden}")
}

/// What the right side of the dock says about the notifications: how many have not been read, in
/// the mark of the icon set, as the design writes it (`●2`, 3.4). Empty when nothing is unread.
#[must_use]
pub fn count_label(mark: &str, unread: usize) -> String {
    if unread == 0 { String::new() } else { format!("{mark}{unread}") }
}

/// What a press on the dock means.
pub struct Presses<'a, Msg> {
    /// Opening the launcher.
    pub launcher: Msg,
    /// Showing the windows that do not fit on the row.
    pub more: Msg,
    /// Opening the list of recent notifications.
    pub notices: Msg,
    /// Pressing the item of a window.
    pub press: &'a dyn Fn(&Item) -> Msg,
    /// The rows of the menu a right click on a window's item opens.
    pub menu: &'a dyn Fn(&Item) -> Vec<ContextItem<Msg>>,
}

/// Draws the dock for `plan` in the row it is given: the launcher button at its left end, the open
/// windows after it, the machine and the clock at its right.
pub fn view<Msg: Clone + 'static>(
    plan: &Plan,
    clock: &str,
    count: &str,
    items: &[Item],
    presses: &Presses<'_, Msg>,
    ui: &mut View<'_, Msg>,
) {
    let icon = family_icon(ui.env().icons());
    ui.row(|ui| {
        ui.add(IconButton::new(icon).on_press(presses.launcher.clone()).tooltip(t!("dock.launcher")));
        // The gaps are spacers of their own and not a gap of the row: a row gap would also stand
        // between the spacer and the machine name and take cells the plan gave the name.
        for (index, item) in items.iter().take(plan.shown).enumerate() {
            if index > 0 {
                ui.spacer().width(Length::Cells(ITEM_GAP));
            }
            let button =
                Button::new(item.label.text(plan.named)).selected(item.focused).on_press((presses.press)(item));
            ui.add_with(ContextMenu::new((presses.menu)(item)), |ui| {
                ui.add(button).id(crate::wm::view::id_of(item.id));
            });
        }
        if plan.hidden > 0 {
            if plan.shown > 0 {
                ui.spacer().width(Length::Cells(ITEM_GAP));
            }
            ui.add(Button::new(more_label(plan.hidden)).on_press(presses.more.clone())).id("dock-more");
        }
        ui.spacer();
        if plan.count && !count.is_empty() {
            ui.add(Button::new(count).on_press(presses.notices.clone())).id(NOTICES);
            ui.spacer().width(Length::Cells(GAP));
        }
        if !plan.name.is_empty() {
            ui.add(Text::new(plan.name.as_str()).no_wrap());
        }
        if plan.clock {
            // The gap is a fixed space between the parts, not a row gap: that would also stand
            // between the spacer and the name and take cells the plan gave the name.
            ui.spacer().width(Length::Cells(GAP));
            ui.add(Text::new(clock).role("secondary").no_wrap());
        }
    })
    .padding(Padding { top: 0, right: EDGE, bottom: 0, left: EDGE })
    .fill_width()
    .height(Length::Cells(HEIGHT));
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLOCK: &str = "14:32";

    /// The button padding of the family's themes.
    const PAD: u16 = 2;

    /// The narrowest row the launcher button and both ends fit in.
    const ROOM: u16 = 2 * EDGE + LAUNCHER_WIDTH;

    fn plan(width: u16, name: &str, clock: &str) -> Plan {
        super::plan(width, name, clock, "", &[], PAD)
    }

    /// The same row with two notifications unread.
    fn with_count(width: u16, name: &str) -> Plan {
        super::plan(width, name, CLOCK, &count_label("●", 2), &[], PAD)
    }

    /// An item of a window that is on the desktop.
    fn label(glyph: &str, name: &str) -> Label {
        Label { glyph: glyph.to_owned(), name: name.to_owned(), mark: None, attention: None }
    }

    fn fits(plan: &Plan, width: u16) {
        let mut used = text::width(&plan.name);
        if plan.clock {
            used += GAP + text::width(CLOCK);
        }
        assert!(used + ROOM <= width.max(ROOM), "{plan:?} is wider than {width}");
    }

    #[test]
    fn a_wide_dock_shows_the_whole_name_and_the_clock() {
        for width in [80, 120, 200, 400] {
            let plan = plan(width, "sunucu-1", CLOCK);
            assert_eq!(plan.name, "sunucu-1", "width {width}");
            assert!(plan.clock, "width {width}");
        }
    }

    #[test]
    fn the_clock_goes_first_when_both_do_not_fit() {
        let name = "a".repeat(30);
        // 30 + 3 + 5 + 2 + 3 = 43 fits exactly; one column less and the clock goes.
        assert!(plan(43, &name, CLOCK).clock);
        let narrower = plan(42, &name, CLOCK);
        assert!(!narrower.clock);
        assert_eq!(narrower.name, name, "the whole name still fits alone");
    }

    #[test]
    fn a_name_too_long_for_the_row_is_cut_in_the_middle_keeping_its_end() {
        let plan = plan(20, "build-server-europe-west-17", CLOCK);
        assert!(!plan.clock);
        assert_eq!(text::width(&plan.name), 15);
        assert!(plan.name.ends_with("west-17"), "{}", plan.name);
        assert!(plan.name.contains(text::ELLIPSIS));
    }

    #[test]
    fn every_width_keeps_the_parts_inside_the_row() {
        let names = ["", "pi", "sunucu-1", "dizüstü", "build-server-europe-west-17", "伺服器-東京-3"];
        for name in names {
            for width in 0..=120 {
                let plan = plan(width, name, CLOCK);
                fits(&plan, width);
                if plan.clock {
                    assert_eq!(plan.name, name, "the clock is only shown beside the whole name");
                }
            }
        }
    }

    #[test]
    fn the_name_stays_as_long_as_one_cell_is_left_for_it() {
        for width in ROOM + 1..=60 {
            assert!(!plan(width, "sunucu-1", CLOCK).name.is_empty(), "width {width}");
        }
        assert!(plan(ROOM, "sunucu-1", CLOCK).name.is_empty());
        assert!(plan(0, "sunucu-1", CLOCK).name.is_empty());
    }

    #[test]
    fn nothing_unread_puts_nothing_on_the_row() {
        let plan = super::plan(80, "sunucu-1", CLOCK, &count_label("●", 0), &[], PAD);
        assert!(!plan.count, "an empty count is no count");
        assert_eq!(count_label("●", 0), "");
        assert_eq!(count_label("●", 2), "●2", "the design writes it as one word (3.4)");
    }

    #[test]
    fn a_wide_row_counts_the_unread_notifications_beside_the_name_and_the_clock() {
        let plan = with_count(80, "sunucu-1");
        assert_eq!((plan.count, plan.clock), (true, true));
        assert_eq!(plan.name, "sunucu-1");
    }

    #[test]
    fn a_tight_row_gives_up_the_clock_then_the_count_and_the_name_last() {
        // The count is a button: `●2` between two paddings, and a gap before the name.
        let count = item_width(&count_label("●", 2), PAD) + GAP;
        let name = "sunucu-1";
        let exact = ROOM + count + text::width(name) + GAP + text::width(CLOCK);
        assert_eq!((with_count(exact, name).count, with_count(exact, name).clock), (true, true));
        let without_clock = with_count(exact - 1, name);
        assert!(without_clock.count && !without_clock.clock, "the clock goes first: {without_clock:?}");
        let narrower = with_count(ROOM + count + text::width(name) - 1, name);
        assert!(!narrower.count, "then the count: {narrower:?}");
        assert_eq!(narrower.name, name, "and the name is whole until nothing else is left");
    }

    #[test]
    fn every_width_keeps_the_count_the_name_and_the_clock_inside_the_row() {
        let count = count_label("●", 12);
        for name in ["", "pi", "sunucu-1", "build-server-europe-west-17"] {
            for width in 0..=120 {
                let plan = super::plan(width, name, CLOCK, &count, &[], PAD);
                let mut used = text::width(&plan.name);
                if plan.count {
                    used += item_width(&count, PAD) + GAP;
                }
                if plan.clock {
                    used += GAP + text::width(CLOCK);
                }
                assert!(used + ROOM <= width.max(ROOM), "{plan:?} is wider than {width}");
                if plan.clock {
                    assert!(plan.count || count.is_empty(), "the clock never outlives the count: {plan:?}");
                }
            }
        }
    }

    #[test]
    fn an_empty_desktop_has_no_window_items() {
        let plan = plan(80, "sunucu-1", CLOCK);
        assert_eq!((plan.shown, plan.hidden), (0, 0));
    }

    #[test]
    fn a_wide_dock_writes_every_window_with_its_name() {
        let labels = [label("▦", "htop"), label("❯", "Terminal")];
        let plan = super::plan(80, "sunucu-1", CLOCK, "", &labels, PAD);
        assert_eq!((plan.shown, plan.named, plan.hidden), (2, true, 0));
        assert_eq!(labels[0].text(true), "▦ htop");
    }

    #[test]
    fn a_minimized_window_carries_its_mark_in_both_widths() {
        let minimized = Label { mark: Some("−".to_owned()), ..label("▦", "htop") };
        assert_eq!(minimized.text(true), "▦ htop −");
        assert_eq!(minimized.text(false), "▦ −");
    }

    #[test]
    fn a_window_that_calls_for_attention_carries_its_mark_after_the_minimized_one() {
        let called = Label { attention: Some("●".to_owned()), ..label("▦", "htop") };
        assert_eq!(called.text(true), "▦ htop ●");
        assert_eq!(called.text(false), "▦ ●");
        let both = Label { mark: Some("−".to_owned()), ..called };
        assert_eq!(both.text(true), "▦ htop − ●");
    }

    #[test]
    fn items_that_do_not_fit_with_their_names_fall_back_to_their_glyphs() {
        let labels: Vec<Label> =
            ["Midnight Commander", "Terminal", "Settings", "htop"].into_iter().map(|name| label("▦", name)).collect();
        // 80 columns: 2 edges, 3 for the launcher, "sunucu-1", a gap and the clock leave 56, and
        // four named items want 4 × (1 + 1 + name + 4) plus three gaps: more than that.
        let plan = super::plan(80, "sunucu-1", CLOCK, "", &labels, PAD);
        assert_eq!((plan.shown, plan.named, plan.hidden), (4, false, 0));
    }

    #[test]
    fn what_does_not_fit_at_all_goes_behind_a_control_that_says_how_many() {
        let labels: Vec<Label> = (0..12).map(|index| label("▦", &format!("window {index}"))).collect();
        let plan = super::plan(60, "pi", CLOCK, "", &labels, PAD);
        assert!(!plan.named);
        assert_eq!(plan.shown + plan.hidden, labels.len());
        assert!(plan.hidden > 0, "{plan:?}");
        assert_eq!(more_label(plan.hidden), format!("+{}", plan.hidden));
    }

    #[test]
    fn the_items_never_reach_past_the_row() {
        let labels: Vec<Label> = (0..8).map(|index| label("▦", &format!("app {index}"))).collect();
        for width in 0..=160 {
            for count in 0..=labels.len() {
                let plan = super::plan(width, "sunucu-1", CLOCK, "", &labels[..count], PAD);
                let widths: Vec<u16> =
                    labels[..plan.shown].iter().map(|label| item_width(&label.text(plan.named), PAD)).collect();
                let mut used = row_width(&widths);
                if plan.hidden > 0 {
                    let control = item_width(&more_label(plan.hidden), PAD);
                    let gap = if plan.shown > 0 { ITEM_GAP } else { 0 };
                    used = used.saturating_add(gap).saturating_add(control);
                }
                // What the right end of the row leaves the items.
                let room = width.saturating_sub(2 * EDGE + LAUNCHER_WIDTH);
                let mut right = text::width(&plan.name);
                if plan.clock {
                    right = right.saturating_add(GAP).saturating_add(text::width(CLOCK));
                }
                let left = room.saturating_sub(right).saturating_sub(if right == 0 { 0 } else { GAP });
                assert!(used <= left, "{plan:?} takes {used} of {left} columns at width {width}");
                let counted = plan.shown + plan.hidden;
                assert!(counted == count || counted == 0, "{plan:?} loses a window of {count} at width {width}");
            }
        }
    }

    #[test]
    fn the_smallest_desktop_shows_a_usual_name_and_the_clock() {
        // 40 columns is the narrowest desktop; ordinary host names keep the clock there even
        // beside the launcher button.
        let plan = plan(40, "raspberrypi", CLOCK);
        assert!(plan.clock);
        assert_eq!(plan.name, "raspberrypi");
    }
}
