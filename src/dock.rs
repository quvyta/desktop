//! The dock: one row of the screen, at the bottom or, when the settings say so, at the top.
//!
//! Its left end holds the launcher button, then the marks of the workspaces, then the open windows
//! of the workspace on screen in the order they were opened, and its right side the status strip
//! (design 3.10), the name of the machine and the clock. The launcher button works like a
//! Start button: it opens the launcher and closes it again, and it stays pressed while the launcher
//! is open. Which of them fit in a given width is
//! decided by [`plan`], a pure function, and drawn by [`view`] into whichever row it is given.
//! Nothing here knows which edge that is: the row is the same row at either end.

use qframe::event::{Event, MouseButton, MouseKind};
use qframe::geometry::{Rect, Size};
use qframe::prelude::*;
use qframe::style::CellStyle;
use qframe::text;
use qframe::widget::{EventCx, MeasureCx, PaintCx, Widget};
use qframe::widgets::{ContextItem, ContextMenu, Tooltip};

use crate::desktop::quvyta_icon;
use crate::status;
use crate::wm::{SPACES, WindowId};

/// Rows the dock takes at its edge of the screen. It is always there, so the desktop has this
/// row less whichever edge it sits on.
pub const HEIGHT: u16 = 1;

/// Cells kept free at each end of the dock, so nothing touches the edge of the terminal.
pub const EDGE: u16 = 1;

/// Cells between two parts of the dock.
pub const GAP: u16 = 3;

/// Cells the Quvyta glyph on the launcher button takes, in every glyph mode.
pub const LAUNCHER_GLYPH: u16 = 1;

/// The id of the launcher button.
pub const LAUNCHER: &str = "dock-launcher";

/// Cells between two window items.
pub const ITEM_GAP: u16 = 1;

/// The id of the button that counts the unread notifications and opens their list.
pub const NOTICES: &str = "dock-notices";

/// Cells on each side of the workspace marks, between them and the launcher button and between
/// them and the first window.
pub const MARKS_GAP: u16 = 2;

/// Cells between two items of the status strip. Narrower than the gap between the dock's parts, so
/// the strip reads as one part.
pub const STRIP_GAP: u16 = 2;

/// How the workspaces are shown beside the launcher button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Marks {
    /// One mark for every workspace, a cell apart.
    All,
    /// Only the number of the workspace on screen.
    Current,
    /// Nothing: the row is left to the machine name.
    Hidden,
}

impl Marks {
    /// Cells the marks themselves take.
    #[must_use]
    pub fn width(self) -> u16 {
        match self {
            // SPACES is four; the cast cannot lose anything.
            Self::All => u16::try_from(2 * SPACES - 1).unwrap_or(u16::MAX),
            Self::Current => 1,
            Self::Hidden => 0,
        }
    }

    /// Cells the marks take with the gaps on both sides of them.
    #[must_use]
    pub fn lead(self) -> u16 {
        if self == Self::Hidden { 0 } else { self.width().saturating_add(2 * MARKS_GAP) }
    }
}

/// The workspaces as the marks show them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Workspaces {
    /// The workspace on screen, from 0.
    pub current: usize,
    /// Which workspaces hold a window.
    pub occupied: [bool; SPACES],
}

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

/// One item of the status strip, as the dock draws it: what it shows and how urgently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chip {
    /// The part of the machine it shows.
    pub kind: status::Kind,
    /// Everything it writes: the glyph, the mark of a warning or an alarm, and the short text.
    pub text: String,
    /// How urgently it asks to be looked at, which picks its colour. The mark in `text` says the
    /// same without colour.
    pub tone: status::Tone,
}

/// The id of the status strip's item of `kind`.
#[must_use]
pub fn chip_id(kind: status::Kind) -> &'static str {
    match kind {
        status::Kind::Tmux => "dock-status-tmux",
        status::Kind::Network => "dock-status-network",
        status::Kind::Cpu => "dock-status-cpu",
        status::Kind::Memory => "dock-status-memory",
        status::Kind::Battery => "dock-status-battery",
    }
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
    /// How the workspaces are shown.
    pub marks: Marks,
    /// How many items of the status strip are drawn, counted from its right end: the ones nearest
    /// the machine name stay longest.
    pub strip: usize,
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
///
/// The workspaces stand beside the launcher button (design 3.10). Their four marks give way before
/// the windows do: when the windows would go behind `+n` beside them, only the number of the
/// workspace on screen is left. That number outlives the clock and the count, because it is the
/// one answer to "where am I", and goes only when the whole machine name would not fit beside it.
///
/// The status strip, whose items are `strip`, gives way before everything else (design 3.10): it
/// takes only what the windows, the marks, the clock and the count leave, and its items leave from
/// the left, the tmux sessions first and the battery last. A strip item is something to glance at;
/// a window's name is how a window is found, and the machine name how the machine is.
#[must_use]
pub fn plan(
    width: u16,
    name: &str,
    clock: &str,
    count: &str,
    labels: &[Label],
    strip: &[String],
    padding: u16,
) -> Plan {
    let room = width.saturating_sub(2 * EDGE + launcher_width(padding));
    // The room beside the number; none at all when the number itself does not fit.
    let beside = room.checked_sub(Marks::Current.lead());
    let fits_beside = |used: u16| beside.is_some_and(|beside| used <= beside);
    let name_width = text::width(name);
    let count_width = if count.is_empty() { 0 } else { item_width(count, padding).saturating_add(GAP) };
    let with_clock = count_width.saturating_add(name_width).saturating_add(GAP).saturating_add(text::width(clock));
    let with_count = count_width.saturating_add(name_width);
    let (shown_name, count_shown, clock_shown, right, marks) = if fits_beside(with_clock) {
        (name.to_owned(), count_width > 0, true, with_clock, Marks::Current)
    } else if fits_beside(with_count) {
        (name.to_owned(), count_width > 0, false, with_count, Marks::Current)
    } else if fits_beside(name_width) {
        (name.to_owned(), false, false, name_width, Marks::Current)
    } else if name_width <= room {
        (name.to_owned(), false, false, name_width, Marks::Hidden)
    } else {
        let shortened = text::truncate_middle(name, room).into_owned();
        let used = text::width(&shortened);
        (shortened, false, false, used, Marks::Hidden)
    };
    // The items keep a gap away from whatever stands at the right end.
    let left = room.saturating_sub(right).saturating_sub(if right == 0 { 0 } else { GAP }).saturating_sub(marks.lead());
    // Every mark is drawn when the windows still all stand on the row beside them.
    let more = Marks::All.lead() - Marks::Current.lead();
    let marks = if marks == Marks::Current && left >= more && windows(left - more, labels, padding).0 == labels.len() {
        Marks::All
    } else {
        marks
    };
    let left = left.saturating_sub(if marks == Marks::All { more } else { 0 });
    let (shown, named, hidden) = windows(left, labels, padding);
    // The strip stands between the windows and the right end, a gap from each; the gap on the
    // windows' side is the one `left` already kept. Beside no name at all it has nothing to stand
    // beside, and the row is too narrow for it anyway.
    let spare = left.saturating_sub(windows_width(labels, shown, named, hidden, padding));
    let strip = if right == 0 { 0 } else { strip_items(spare, strip) };
    Plan { name: shown_name, count: count_shown, clock: clock_shown, shown, named, hidden, marks, strip }
}

/// Cells the window items and the `+n` control take as [`windows`] chose them.
fn windows_width(labels: &[Label], shown: usize, named: bool, hidden: usize, padding: u16) -> u16 {
    let widths: Vec<u16> = labels.iter().take(shown).map(|label| item_width(&label.text(named), padding)).collect();
    let mut used = row_width(&widths);
    if hidden > 0 {
        let gap = if shown > 0 { ITEM_GAP } else { 0 };
        used = used.saturating_add(gap).saturating_add(item_width(&more_label(hidden), padding));
    }
    used
}

/// Cells the last `count` items of the strip take, the gaps between them and the gap after them
/// included.
fn strip_width(strip: &[String], count: usize) -> u16 {
    let items = &strip[strip.len() - count.min(strip.len())..];
    let gaps = u16::try_from(items.len().saturating_sub(1)).unwrap_or(u16::MAX).saturating_mul(STRIP_GAP);
    items.iter().fold(gaps.saturating_add(GAP), |sum, item| sum.saturating_add(text::width(item)))
}

/// How many items of the strip fit in `room` columns, counted from its right end.
fn strip_items(room: u16, strip: &[String]) -> usize {
    (1..=strip.len()).rev().find(|count| strip_width(strip, *count) <= room).unwrap_or(0)
}

/// Cells the launcher button takes with a button padding of `padding` columns on each side.
///
/// It is a button like the window items, not an icon button: a button has a chosen state, which
/// is how it stays pressed while the launcher is open, and an icon button has none.
#[must_use]
pub fn launcher_width(padding: u16) -> u16 {
    LAUNCHER_GLYPH.saturating_add(padding.saturating_mul(2))
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
    /// Going to a workspace, by its number from 0.
    pub space: &'a dyn Fn(usize) -> Msg,
    /// The workspaces the marks show.
    pub workspaces: Workspaces,
    /// Opening the launcher, or closing it while it is open.
    pub launcher: Msg,
    /// Whether the launcher is open, which the button shows by staying pressed.
    pub launcher_open: bool,
    /// Showing the windows that do not fit on the row.
    pub more: Msg,
    /// Opening the list of recent notifications.
    pub notices: Msg,
    /// Pressing the item of a window.
    pub press: &'a dyn Fn(&Item) -> Msg,
    /// The rows of the menu a right click on a window's item opens.
    pub menu: &'a dyn Fn(&Item) -> Vec<ContextItem<Msg>>,
    /// Pressing an item of the status strip; `None` for an item a press does nothing on.
    pub chip: &'a dyn Fn(&Chip) -> Option<Msg>,
    /// The rows of the menu a right click on an item of the strip opens; none for most.
    pub chip_menu: &'a dyn Fn(&Chip) -> Vec<ContextItem<Msg>>,
}

/// What the dock draws: the clock, the unread count, the windows and the status strip.
pub struct Parts<'a> {
    /// The clock's text.
    pub clock: &'a str,
    /// The unread count, empty when nothing is unread.
    pub count: &'a str,
    /// The windows of the workspace on screen.
    pub items: &'a [Item],
    /// The status strip, left to right; empty while it is off.
    pub strip: &'a [Chip],
}

/// Draws the dock for `plan` in the row it is given: the launcher button at its left end, the open
/// windows after it, the status strip, the machine and the clock at its right.
pub fn view<Msg: Clone + 'static>(plan: &Plan, parts: &Parts<'_>, presses: &Presses<'_, Msg>, ui: &mut View<'_, Msg>) {
    let Parts { clock, count, items, strip } = *parts;
    let icon = quvyta_icon(ui.env().icons());
    let glyph = ui.env().icons().glyph(icon).into_owned();
    ui.row(|ui| {
        let button = Button::new(glyph).selected(presses.launcher_open).on_press(presses.launcher.clone());
        ui.add_with(Tooltip::new(t!("dock.launcher")), |ui| {
            ui.add(button).id(LAUNCHER);
        });
        if plan.marks != Marks::Hidden {
            ui.spacer().width(Length::Cells(MARKS_GAP));
            marks_view(plan.marks, presses.workspaces, presses.space, ui);
            ui.spacer().width(Length::Cells(MARKS_GAP));
        }
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
        let first = strip.len() - plan.strip.min(strip.len());
        for (index, chip) in strip.iter().skip(first).enumerate() {
            if index > 0 {
                ui.spacer().width(Length::Cells(STRIP_GAP));
            }
            chip_view(chip, presses, ui);
        }
        if first < strip.len() {
            ui.spacer().width(Length::Cells(GAP));
        }
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

/// Draws one item of the status strip: its text in the secondary tone, or in the theme's warning or
/// danger tone with the mark that says so in `text`. It answers the pointer only when a press on it
/// does something; its name comes up under the pointer either way.
fn chip_view<Msg: Clone + 'static>(chip: &Chip, presses: &Presses<'_, Msg>, ui: &mut View<'_, Msg>) {
    let token = match chip.tone {
        // The colour of the theme's secondary text, the clock's.
        status::Tone::Calm => "dim",
        status::Tone::Warn => "warning",
        status::Tone::Alert => "danger",
    };
    let mark = Mark { glyph: chip.text.clone(), token, bold: false, on_press: (presses.chip)(chip) };
    let menu = (presses.chip_menu)(chip);
    ui.add_with(Tooltip::new(chip.kind.label()), |ui| {
        if menu.is_empty() {
            ui.add(mark).id(chip_id(chip.kind));
        } else {
            ui.add_with(ContextMenu::new(menu), |ui| {
                ui.add(mark).id(chip_id(chip.kind));
            });
        }
    });
}

/// The id of the mark of the workspace `space`, counted from 0.
#[must_use]
pub fn mark_id(space: usize) -> String {
    format!("dock-space-{}", space + 1)
}

/// The id of the number that stands for the marks on a narrow row.
pub const MARK_CURRENT: &str = "dock-space";

/// Draws the marks of the workspaces into the row `ui` is building: one for each workspace, or only
/// the number of the one on screen. A press on a mark goes to its workspace; a press on the number
/// goes on to the next one, round from the last to the first.
pub fn marks_view<Msg: Clone + 'static>(
    marks: Marks,
    workspaces: Workspaces,
    space: &dyn Fn(usize) -> Msg,
    ui: &mut View<'_, Msg>,
) {
    match marks {
        Marks::Hidden => {}
        Marks::Current => {
            let number = workspaces.current + 1;
            let mark = Mark::space(number.to_string(), Tone::Current, space(number % SPACES));
            ui.add_with(Tooltip::new(t!("dock.workspace-next", n = number)), |ui| {
                ui.add(mark).id(MARK_CURRENT);
            });
        }
        Marks::All => {
            let icons = ui.env().icons();
            let (held, empty) = (icons.glyph("bullet").into_owned(), icons.glyph("dot-outline").into_owned());
            for index in 0..SPACES {
                if index > 0 {
                    ui.spacer().width(Length::Cells(1));
                }
                let tone = if index == workspaces.current {
                    Tone::Current
                } else if workspaces.occupied[index] {
                    Tone::Occupied
                } else {
                    Tone::Empty
                };
                let glyph = match tone {
                    Tone::Current => (index + 1).to_string(),
                    Tone::Occupied => held.clone(),
                    Tone::Empty => empty.clone(),
                };
                let mark = Mark::space(glyph, tone, space(index));
                ui.add_with(Tooltip::new(t!("dock.workspace", n = index + 1)), |ui| {
                    ui.add(mark).id(mark_id(index));
                });
            }
        }
    }
}

/// How a mark of a workspace looks. Each look has a shape of its own as well as a tone, so the
/// three read apart in sixteen colours and without colour (VISION 4.5).
///
/// The workspace on screen is its number and not a filled dot: the dock already says "something
/// here wants you" with the filled dot, on a window that called and before the unread count, and a
/// second meaning for the same mark on the same row would be read as the first. The number is also
/// the key that reaches it, and what the row keeps when it is too narrow for the marks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tone {
    /// The workspace on screen: its number in the accent, bold.
    Current,
    /// A workspace that holds windows: a small dot in the text colour.
    Occupied,
    /// An empty workspace: a ring, faint.
    Empty,
}

/// One mark of a workspace, or one item of the status strip: a few cells of text that do something
/// when clicked.
///
/// It is text and not a button because a button's padding would make four of them wider than
/// the windows they stand beside; it takes the icon button's hover tone, so it answers the pointer
/// the way every small control of the framework does. A mark without `on_press` does not answer
/// it at all.
struct Mark<Msg> {
    glyph: String,
    /// The theme's colour token it is written in.
    token: &'static str,
    bold: bool,
    on_press: Option<Msg>,
}

impl<Msg> Mark<Msg> {
    /// The mark of a workspace in `tone`, going there when pressed.
    fn space(glyph: String, tone: Tone, on_press: Msg) -> Self {
        let token = match tone {
            Tone::Current => "accent",
            Tone::Occupied => "text",
            Tone::Empty => "dim",
        };
        Self { glyph, token, bold: tone == Tone::Current, on_press: Some(on_press) }
    }
}

/// Whether the left button went down on the mark, so only a click that began on it presses it.
#[derive(Debug, Default)]
struct MarkMemory {
    held: bool,
}

impl<Msg: Clone + 'static> Widget<Msg> for Mark<Msg> {
    fn measure(&self, _cx: &mut MeasureCx<'_>, available: Size) -> Size {
        Size::new(text::width(&self.glyph).max(1), 1).min(available)
    }

    fn paint(&self, cx: &mut PaintCx<'_>, area: Rect) {
        if area.is_empty() {
            return;
        }
        if self.on_press.is_some() {
            let states = cx.pressable_states();
            let style = cx.style("icon-button", None, &states).text();
            if let Some(bg) = style.bg {
                cx.fill(area, bg);
            }
        }
        cx.register_hit(area);
        let look = CellStyle::fg(cx.color(self.token)).with_bold(self.bold);
        cx.text(area.x, area.y, &self.glyph, look, area.width);
    }

    fn event(&self, cx: &mut EventCx<'_, Msg>, event: &Event) -> bool {
        let Event::Mouse(mouse) = event else { return false };
        let Some(on_press) = &self.on_press else { return false };
        match mouse.kind {
            MouseKind::Down(MouseButton::Left) => {
                cx.capture_pointer();
                cx.memory::<MarkMemory>().held = true;
                true
            }
            MouseKind::Up(MouseButton::Left) => {
                let held = std::mem::take(&mut cx.memory::<MarkMemory>().held);
                if held && cx.area().contains(mouse.x, mouse.y) {
                    cx.emit(on_press.clone());
                }
                held
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLOCK: &str = "14:32";

    /// The button padding of the Quvyta themes.
    const PAD: u16 = 2;

    /// The narrowest row the launcher button and both ends fit in.
    const ROOM: u16 = 2 * EDGE + LAUNCHER_GLYPH + 2 * PAD;

    /// What the number of the workspace on screen takes with its gaps.
    const NUMBER: u16 = 1 + 2 * MARKS_GAP;

    fn plan(width: u16, name: &str, clock: &str) -> Plan {
        super::plan(width, name, clock, "", &[], &[], PAD)
    }

    /// The same row with two notifications unread.
    fn with_count(width: u16, name: &str) -> Plan {
        super::plan(width, name, CLOCK, &count_label("●", 2), &[], &[], PAD)
    }

    /// An item of a window that is on the desktop.
    fn label(glyph: &str, name: &str) -> Label {
        Label { glyph: glyph.to_owned(), name: name.to_owned(), mark: None, attention: None }
    }

    fn fits(plan: &Plan, width: u16) {
        let mut used = text::width(&plan.name) + plan.marks.lead();
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
        // 30 + 3 + 5 + 2 + 5 and the number of the workspace fit exactly; one column less and the
        // clock goes.
        assert!(plan(45 + NUMBER, &name, CLOCK).clock);
        let narrower = plan(44 + NUMBER, &name, CLOCK);
        assert!(!narrower.clock);
        assert_eq!(narrower.name, name, "the whole name still fits alone");
    }

    #[test]
    fn a_name_too_long_for_the_row_is_cut_in_the_middle_keeping_its_end() {
        let plan = plan(22, "build-server-europe-west-17", CLOCK);
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
        let plan = super::plan(80, "sunucu-1", CLOCK, &count_label("●", 0), &[], &[], PAD);
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
        let exact = ROOM + NUMBER + count + text::width(name) + GAP + text::width(CLOCK);
        assert_eq!((with_count(exact, name).count, with_count(exact, name).clock), (true, true));
        let without_clock = with_count(exact - 1, name);
        assert!(without_clock.count && !without_clock.clock, "the clock goes first: {without_clock:?}");
        let narrower = with_count(ROOM + NUMBER + count + text::width(name) - 1, name);
        assert!(!narrower.count, "then the count: {narrower:?}");
        assert_eq!(narrower.name, name, "and the name is whole until nothing else is left");
    }

    #[test]
    fn every_width_keeps_the_count_the_name_and_the_clock_inside_the_row() {
        let count = count_label("●", 12);
        for name in ["", "pi", "sunucu-1", "build-server-europe-west-17"] {
            for width in 0..=120 {
                let plan = super::plan(width, name, CLOCK, &count, &[], &[], PAD);
                let mut used = text::width(&plan.name) + plan.marks.lead();
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
        let plan = super::plan(80, "sunucu-1", CLOCK, "", &labels, &[], PAD);
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
        let plan = super::plan(80, "sunucu-1", CLOCK, "", &labels, &[], PAD);
        assert_eq!((plan.shown, plan.named, plan.hidden), (4, false, 0));
    }

    #[test]
    fn what_does_not_fit_at_all_goes_behind_a_control_that_says_how_many() {
        let labels: Vec<Label> = (0..12).map(|index| label("▦", &format!("window {index}"))).collect();
        let plan = super::plan(60, "pi", CLOCK, "", &labels, &[], PAD);
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
                let plan = super::plan(width, "sunucu-1", CLOCK, "", &labels[..count], &[], PAD);
                let widths: Vec<u16> =
                    labels[..plan.shown].iter().map(|label| item_width(&label.text(plan.named), PAD)).collect();
                let mut used = row_width(&widths);
                if plan.hidden > 0 {
                    let control = item_width(&more_label(plan.hidden), PAD);
                    let gap = if plan.shown > 0 { ITEM_GAP } else { 0 };
                    used = used.saturating_add(gap).saturating_add(control);
                }
                // What the right end of the row leaves the items.
                let room = width.saturating_sub(2 * EDGE + launcher_width(PAD));
                let mut right = text::width(&plan.name);
                if plan.clock {
                    right = right.saturating_add(GAP).saturating_add(text::width(CLOCK));
                }
                let left = room
                    .saturating_sub(right)
                    .saturating_sub(if right == 0 { 0 } else { GAP })
                    .saturating_sub(plan.marks.lead());
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

    #[test]
    fn a_wide_row_shows_a_mark_for_every_workspace() {
        let labels = [label("▦", "htop"), label("❯", "Terminal")];
        let plan = super::plan(80, "sunucu-1", CLOCK, "", &labels, &[], PAD);
        assert_eq!(plan.marks, Marks::All);
        assert_eq!((plan.shown, plan.named, plan.hidden), (2, true, 0));
        assert_eq!(Marks::All.width(), 7, "four marks a cell apart");
    }

    #[test]
    fn the_marks_shrink_to_the_number_before_a_window_goes_behind_the_control() {
        // Six glyph items want 6 * 5 + 5 = 35 columns. Beside every mark they would not all fit on a
        // row of 70; beside the number they do.
        let labels: Vec<Label> = (0..6).map(|index| label("▦", &format!("window {index}"))).collect();
        for width in 0..=160 {
            let plan = super::plan(width, "sunucu-1", CLOCK, "", &labels, &[], PAD);
            if plan.marks == Marks::All {
                assert_eq!(plan.hidden, 0, "every mark cost a window at width {width}: {plan:?}");
                assert_eq!(plan.shown, labels.len(), "width {width}");
            }
        }
        let plan = super::plan(70, "sunucu-1", CLOCK, "", &labels, &[], PAD);
        assert_eq!((plan.marks, plan.shown, plan.hidden), (Marks::Current, 6, 0), "{plan:?}");
        assert!(plan.clock, "the marks shrink before the clock goes: {plan:?}");
    }

    #[test]
    fn the_number_outlives_the_clock_and_the_count_and_goes_before_the_name() {
        let name = "sunucu-1";
        let count = item_width(&count_label("●", 2), PAD) + GAP;
        let clockless = with_count(ROOM + NUMBER + count + text::width(name) + GAP + text::width(CLOCK) - 1, name);
        assert_eq!((clockless.clock, clockless.count, clockless.marks), (false, true, Marks::Current));
        let countless = with_count(ROOM + NUMBER + text::width(name), name);
        assert_eq!((countless.count, countless.marks), (false, Marks::Current), "{countless:?}");
        let bare = with_count(ROOM + NUMBER + text::width(name) - 1, name);
        assert_eq!((bare.marks, bare.name.as_str()), (Marks::Hidden, name), "the name is whole: {bare:?}");
    }

    #[test]
    fn the_narrowest_desktop_keeps_every_mark_while_there_is_no_window() {
        let plan = plan(40, "raspberrypi", CLOCK);
        assert_eq!((plan.marks, plan.clock), (Marks::All, true), "{plan:?}");
        let one = super::plan(40, "raspberrypi", CLOCK, "", &[label("▦", "htop")], &[], PAD);
        assert_eq!((one.shown, one.hidden), (1, 0), "{one:?}");
    }

    /// The status strip of a laptop with two tmux sessions, left to right.
    fn strip() -> Vec<String> {
        ["❯ 2", "◎ 1.2M/s", "▣ 12%", "◰ 41%", "○ 87%+"].map(str::to_owned).to_vec()
    }

    /// Two windows on the desktop.
    fn two() -> [Label; 2] {
        [label("▦", "htop"), label("❯", "Terminal")]
    }

    #[test]
    fn the_strip_takes_only_what_every_other_part_leaves() {
        let strip = strip();
        let labels = two();
        let count = count_label("●", 2);
        for windows in [&labels[..0], &labels[..]] {
            for unread in ["", count.as_str()] {
                for width in 0..=200 {
                    let bare = super::plan(width, "sunucu-1", CLOCK, unread, windows, &[], PAD);
                    let plan = super::plan(width, "sunucu-1", CLOCK, unread, windows, &strip, PAD);
                    assert_eq!(Plan { strip: 0, ..plan.clone() }, bare, "the strip cost another part at width {width}");
                    // What the row holds, the strip's items and gaps included, is inside it.
                    let mut used = windows_width(windows, plan.shown, plan.named, plan.hidden, PAD);
                    used += text::width(&plan.name) + plan.marks.lead();
                    if plan.count {
                        used += item_width(unread, PAD) + GAP;
                    }
                    if plan.clock {
                        used += GAP + text::width(CLOCK);
                    }
                    if plan.strip > 0 {
                        used += strip_width(&strip, plan.strip);
                    }
                    if plan.strip > 0 || plan.shown + plan.hidden > 0 {
                        used += GAP;
                    }
                    assert!(used + ROOM <= width.max(ROOM), "{plan:?} is wider than {width}");
                    // A wider row shows no fewer items, unless the windows took the room for their
                    // names: the strip gives way to them.
                    let wider = super::plan(width + 1, "sunucu-1", CLOCK, unread, windows, &strip, PAD);
                    let (rest, wider_rest) = (Plan { strip: 0, ..plan.clone() }, Plan { strip: 0, ..wider.clone() });
                    if rest == wider_rest {
                        assert!(wider.strip >= plan.strip, "width {width}: {plan:?} then {wider:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn the_strips_items_leave_from_the_left_tmux_first_and_the_battery_last() {
        let strip = strip();
        let labels = two();
        let shown = |width: u16, windows: &[Label]| super::plan(width, "sunucu-1", CLOCK, "", windows, &strip, PAD);
        let at_120 = shown(120, &labels);
        assert_eq!((at_120.strip, at_120.shown, at_120.named, at_120.clock), (5, 2, true, true), "{at_120:?}");
        // 80 columns: the two named windows and the marks leave room for the last two items.
        let at_80 = shown(80, &labels);
        assert_eq!((at_80.strip, at_80.shown, at_80.named, at_80.marks), (2, 2, true, Marks::All), "{at_80:?}");
        assert_eq!(shown(80, &[]).strip, 5, "an empty desktop shows the whole strip at 80");
        // 60: the windows are down to their glyphs beside every mark, which is the marks' own
        // rule; what they leave holds the battery.
        let at_60 = shown(60, &labels);
        assert_eq!((at_60.strip, at_60.shown, at_60.clock), (1, 2, true), "{at_60:?}");
        let at_40 = shown(40, &[]);
        assert_eq!((at_40.strip, at_40.clock, at_40.name.as_str()), (0, true, "sunucu-1"), "{at_40:?}");
        // Somewhere between, one item at a time goes, from the left.
        let counts: Vec<usize> = (40..=120).map(|width| shown(width, &labels).strip).collect();
        for count in 1..=5 {
            assert!(counts.contains(&count), "some width shows exactly the last {count}: {counts:?}");
        }
    }
}
