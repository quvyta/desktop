//! The text viewer's windows: opening a text, code or Markdown file in one, drawing it
//! with the framework's code and Markdown views, and handing it to the person's editor in the same
//! window.
//!
//! The file is read off the drawing thread, never more than [`texts::LIMIT`] of it, so a log of
//! gigabytes opens as quickly as a note. While it is read the window shows a spinner, and only
//! after the framework's delay, so a quick file never blinks one.

use std::path::Path;

use qframe::event::{Event, KeyEvent};
use qframe::geometry::{Rect, Size};
use qframe::keymap::Key;
use qframe::prelude::*;
use qframe::widget::{Container, EventCx, MeasureCx, Node, PaintCx, Widget};
use qframe::widgets::{CodeView, EmptyState, Language, Markdown, ScrollView, Spinner, Toast};

use super::{Desk, Msg};
use crate::apps::{Category, Launch};
use crate::desktop;
use crate::files;
use crate::texts::{self, ReadText, Reading, TextWindow, Unread};
use crate::wm::{self, WindowId};

/// The id the entry of a text viewer's window carries.
pub const VIEWER: &str = "texts";

/// How many bytes a mebibyte is, for the notice of a file shown in part.
const MIB: f64 = 1024.0 * 1024.0;

impl Desk {
    /// Opens `file` in a text viewer window of its own, and starts reading it.
    pub(super) fn open_text(&mut self, file: &Path) -> Command<Msg> {
        self.keys = None;
        let window = TextWindow::new(file);
        let name = window.name();
        let icon = desktop::kind_icon(&name, false, false);
        let mut entry = Self::made_entry(VIEWER, &name, icon, Category::Files, Launch::Open(file.to_path_buf()));
        entry.folder = file.parent().map(Path::to_path_buf);
        let id = self.windows.open(&entry);
        let path = file.to_path_buf();
        let reading = Command::perform(move || Msg::TextRead(id, texts::read(&path, texts::LIMIT)));
        self.texts.insert(id, window);
        Command::batch([reading, self.resize_hint(), self.body_focus()])
    }

    /// The file of the viewer `id` was read; one for a window since closed, or since given to the
    /// editor, is dropped.
    pub(super) fn on_text_read(&mut self, id: WindowId, read: Result<ReadText, Unread>) -> Command<Msg> {
        let Some(window) = self.texts.get_mut(&id) else { return Command::none() };
        window.read_as(read);
        Command::none()
    }

    /// Edit in the viewer `id`: the window keeps its place, its size and its name, and runs the
    /// person's editor on the file in the file's folder ([`files::editor_command`]) in place of the
    /// viewer. When the editor ends the window stays, as every program's window does.
    pub(super) fn edit_text(&mut self, id: WindowId) -> Command<Msg> {
        let Some(window) = self.texts.get(&id) else { return Command::none() };
        let file = window.file().to_path_buf();
        let Some(words) = files::editor_command(self.apps.editor.as_deref(), self.apps.path.as_deref(), &file) else {
            let name = window.name();
            return Command::toast(Toast::info(t!("files.not-text", name = name.as_str())));
        };
        if !self.windows.hold_program(id, Launch::Command(words)) {
            return Command::none();
        }
        self.texts.remove(&id);
        let Some(entry) = self.windows.get(id).map(|window| window.entry().clone()) else { return Command::none() };
        let started = self.start(id, &entry);
        Command::batch([started, self.body_focus()])
    }

    /// The body of the viewer `id`: the text over a strip that names the file and offers Edit.
    pub(super) fn text_body(id: WindowId, window: &TextWindow, ui: &mut View<'_, Msg>) {
        let name = window.name();
        let icon = desktop::kind_icon(&name, false, false);
        let (content, notices, lines) = match window.reading() {
            Reading::Ready(shown) => (Some(shown), notices(shown), Some(shown.lines)),
            Reading::Reading | Reading::Failed(_) => (None, Vec::new(), None),
        };
        let mut strip = vec![name.clone()];
        if let Some(lines) = lines {
            strip.push(t!("texts.lines", n = lines));
        }
        let strip = strip.join("   ");
        let holds_focus = content.is_none();
        let viewer = ui.add_with(TextViewer { id, holds_focus, children: Vec::new() }, |ui| {
            match window.reading() {
                Reading::Reading => {
                    ui.add(Spinner::new().label(t!("texts.reading")).delayed(true))
                        .id(format!("text-spinner-{}", id.number()));
                }
                Reading::Failed(why) => {
                    let reason = match why {
                        Unread::Missing => t!("texts.missing"),
                        Unread::Denied => t!("texts.denied"),
                        Unread::Other(words) => words.clone(),
                    };
                    ui.add(EmptyState::new(t!("texts.unreadable", name = name.as_str())).icon(icon).message(reason));
                }
                Reading::Ready(shown) => {
                    let markdown = texts::is_markdown(window.file());
                    ui.add_with(ScrollView::new(), |ui| {
                        if markdown {
                            ui.add(Markdown::new(&shown.text)).fill_width();
                        } else {
                            let language = Language::from_file_name(&name);
                            ui.add(CodeView::new(shown.text.as_str(), language).line_numbers(true)).fill_width();
                        }
                    })
                    .id(wm::view::body_id(id))
                    .fill();
                }
            }
            for notice in notices {
                ui.add(Text::new(notice).role("warning").no_wrap());
            }
            ui.row(|ui| {
                ui.add(Text::new(strip).role("secondary").no_wrap());
                ui.add(Button::new(t!("texts.edit")).shortcut("e").on_press(Msg::EditText(id)));
            })
            .gap(3)
            .fill_width();
        });
        if holds_focus {
            viewer.id(wm::view::body_id(id)).fill();
        } else {
            viewer.fill();
        }
    }
}

/// The lines under a text shown in part: how much of a long file it is, and that some of it was
/// not UTF-8.
fn notices(shown: &ReadText) -> Vec<String> {
    let mut said = Vec::new();
    if let Some(size) = shown.cut {
        #[allow(clippy::cast_precision_loss, reason = "a size said to one decimal")]
        let size = qframe::i18n::number(size as f64 / MIB, 1);
        said.push(t!("texts.cut", size = size.as_str()));
    }
    if shown.lossy {
        said.push(t!("texts.not-utf8"));
    }
    said
}

/// The body of a text viewer: the text in all but the last rows, which are the notices and the
/// strip, one row each; `e` hands the file to the editor, whichever part of the body has the keys.
///
/// While the file is being read, or could not be, the body itself takes the keys; once it is shown
/// the scrolling text does, and the keys it leaves reach the body.
struct TextViewer {
    id: WindowId,
    holds_focus: bool,
    children: Vec<Node<Msg>>,
}

impl TextViewer {
    fn on_key(&self, cx: &mut EventCx<'_, Msg>, key: &KeyEvent) -> bool {
        if !key.is_plain(Key::Char('e')) {
            return false;
        }
        cx.emit(Msg::EditText(self.id));
        true
    }
}

impl Widget<Msg> for TextViewer {
    fn measure(&self, _cx: &mut MeasureCx<'_>, available: Size) -> Size {
        available
    }

    fn paint(&self, cx: &mut PaintCx<'_>, area: Rect) {
        if area.is_empty() {
            return;
        }
        cx.register_hit(area);
        if self.holds_focus {
            cx.register_focusable();
        }
        let below = u16::try_from(self.children.len().saturating_sub(1)).unwrap_or(u16::MAX).min(area.height);
        let text = Rect::new(area.x, area.y, area.width, area.height - below);
        if let Some(content) = self.children.first() {
            if self.holds_focus {
                // A spinner or an empty state stands in the middle, as the picture viewer's do.
                let size = cx.measure_child(content, text.size()).min(text.size());
                let x = text.x + i32::from((text.width - size.width) / 2);
                let y = text.y + i32::from((text.height - size.height) / 2);
                cx.paint_child(content, Rect::new(x, y, size.width, size.height));
            } else {
                cx.paint_child(content, text);
            }
        }
        let first = i32::from(area.height - below);
        for (row, line) in self.children.iter().skip(1).take(usize::from(below)).enumerate() {
            let y = area.y + first + i32::try_from(row).unwrap_or(0);
            cx.paint_child(line, Rect::new(area.x, y, area.width, 1));
        }
    }

    fn event(&self, cx: &mut EventCx<'_, Msg>, event: &Event) -> bool {
        match event {
            Event::Key(key) => self.on_key(cx, key),
            _ => false,
        }
    }

    fn focusable(&self) -> bool {
        self.holds_focus
    }

    fn children(&self) -> &[Node<Msg>] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Node<Msg>] {
        &mut self.children
    }
}

impl Container<Msg> for TextViewer {
    fn set_children(&mut self, children: Vec<Node<Msg>>) {
        self.children = children;
    }
}
