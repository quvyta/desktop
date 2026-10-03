//! The picture viewer's windows: opening a picture in one, walking the pictures of its
//! folder, and drawing the picture with the framework's [`Image`].
//!
//! A picture is decoded off the drawing thread, never larger than the screen asks for, so a large
//! photograph is never held whole. While it decodes the window shows a spinner, and only after
//! the framework's delay, so a quick picture never blinks one. A picture that cannot be read and a
//! terminal that cannot show pictures each say so in the framework's empty state; pixels are never
//! drawn with letters.

use std::path::{Path, PathBuf};

use qframe::event::{Event, KeyEvent, MouseButton, MouseEvent, MouseKind};
use qframe::geometry::{Rect, Size};
use qframe::keymap::Key;
use qframe::prelude::*;
use qframe::widget::{Container, EventCx, MeasureCx, Node, PaintCx, Widget};
use qframe::widgets::{EmptyState, Image, ImageData, ImageError, Spinner};

use super::{Desk, Msg};
use crate::apps::{Category, Launch};
use crate::desktop;
use crate::pictures::{Decoded, PictureWindow, Step};
use crate::wm::{self, WindowId};

/// The id the entry of a picture viewer's window carries.
pub const VIEWER: &str = "pictures";

impl Desk {
    /// Opens `file` in a picture viewer window of its own, and starts decoding it.
    pub(super) fn open_picture(&mut self, file: &Path) -> Command<Msg> {
        self.keys = None;
        let mut window = PictureWindow::open(file);
        let name = window.name();
        let icon = desktop::kind_icon(&name, false, false);
        let mut entry = Self::made_entry(VIEWER, &name, icon, Category::Files, Launch::Open(file.to_path_buf()));
        entry.folder = file.parent().map(Path::to_path_buf);
        let id = self.windows.open(&entry);
        let run = window.start();
        let decoding = self.decode_picture(id, window.file().to_path_buf(), run);
        self.pictures.insert(id, window);
        Command::batch([decoding, self.resize_hint(), self.body_focus()])
    }

    /// Decodes `file` for the viewer `id`, on the decoding `run`, at the size the screen shows.
    fn decode_picture(&self, id: WindowId, file: PathBuf, run: u64) -> Command<Msg> {
        let size = self.wallpaper_size();
        Command::perform(move || Msg::PictureDecoded(id, run, ImageData::decode_file(&file, size)))
    }

    /// A key or a click in the viewer `id`: another picture of the folder, named on the window's
    /// strip and decoded, or another fit.
    pub(super) fn on_picture(&mut self, id: WindowId, step: Step) -> Command<Msg> {
        let Some(window) = self.pictures.get_mut(&id) else { return Command::none() };
        if !window.step(step) {
            return Command::none();
        }
        let name = window.name();
        let (file, run) = (window.file().to_path_buf(), window.run());
        self.windows.rename(id, &name, desktop::kind_icon(&name, false, false));
        self.decode_picture(id, file, run)
    }

    /// The decoding `run` of the viewer `id` ended; one for a picture since left, or a window since
    /// closed, is dropped.
    pub(super) fn on_picture_decoded(
        &mut self,
        id: WindowId,
        run: u64,
        decoded: Result<ImageData, ImageError>,
    ) -> Command<Msg> {
        if let Some(window) = self.pictures.get_mut(&id) {
            window.decoded_as(run, decoded);
        }
        Command::none()
    }

    /// The body of the viewer `id`: the picture over a strip that names it.
    pub(super) fn picture_body(id: WindowId, window: &PictureWindow, ui: &mut View<'_, Msg>) {
        let name = window.name();
        let drawable = ui.env().graphics().can_draw();
        let icons = ui.env().icons();
        let arrow = |key: &str| icons.glyph(key).into_owned();
        let keys = t!("pictures.keys", left = arrow("arrow-left").as_str(), right = arrow("arrow-right").as_str());
        let (n, total) = window.position();
        let mut strip = vec![name.clone()];
        if let Some((width, height)) = window.pixels() {
            strip.push(format!("{width}\u{d7}{height}"));
        }
        strip.push(t!("pictures.position", n = n, total = total));
        strip.push(keys);
        let strip = strip.join("   ");
        let icon = desktop::kind_icon(&name, false, false);
        ui.add_with(Viewer { id, children: Vec::new() }, |ui| {
            match window.decoded() {
                _ if !drawable => {
                    let mut said = t!("pictures.cannot-draw");
                    if let Some((width, height)) = window.pixels() {
                        said = format!("{width}\u{d7}{height}. {said}");
                    }
                    let message = format!("{said}. {}", t!("pictures.cannot-draw-hint"));
                    ui.add(EmptyState::new(name.clone()).icon(icon).message(message));
                }
                Decoded::Decoding => {
                    ui.add(Spinner::new().label(t!("pictures.decoding")).delayed(true))
                        .id(format!("picture-spinner-{}", id.number()));
                }
                Decoded::Failed(problem) => {
                    ui.add(
                        EmptyState::new(t!("pictures.unreadable", name = name.as_str()))
                            .icon(icon)
                            .message(problem.to_string()),
                    );
                }
                Decoded::Ready(data) => {
                    ui.add(Image::new(data).fit(window.fit())).id(format!("picture-{}", id.number()));
                }
            }
            ui.add(Text::new(strip).role("secondary").no_wrap());
        })
        .id(wm::view::body_id(id))
        .fill();
    }
}

/// The body of a viewer: the picture in the middle of all but the last row, which is the strip
/// naming it; the keys of the viewer while the window has them, and a click on the left or the
/// right third of the picture for the one before or after.
struct Viewer {
    id: WindowId,
    children: Vec<Node<Msg>>,
}

impl Viewer {
    /// The rows the picture takes, and the row of the strip under it.
    fn split(area: Rect) -> (Rect, Option<Rect>) {
        if area.height < 2 {
            return (area, None);
        }
        let picture = Rect::new(area.x, area.y, area.width, area.height - 1);
        let strip = Rect::new(area.x, area.y + i32::from(area.height) - 1, area.width, 1);
        (picture, Some(strip))
    }

    fn on_key(&self, cx: &mut EventCx<'_, Msg>, key: &KeyEvent) -> bool {
        let steps = [
            (Key::Right, Step::Next),
            (Key::Char('n'), Step::Next),
            (Key::Space, Step::Next),
            (Key::Left, Step::Previous),
            (Key::Char('p'), Step::Previous),
            (Key::Backspace, Step::Previous),
            (Key::Home, Step::First),
            (Key::End, Step::Last),
            (Key::Char('f'), Step::ToggleFit),
        ];
        let Some((_, step)) = steps.into_iter().find(|(chord, _)| key.is_plain(*chord)) else { return false };
        cx.emit(Msg::Picture(self.id, step));
        true
    }

    fn on_mouse(&self, cx: &mut EventCx<'_, Msg>, mouse: &MouseEvent) -> bool {
        let (picture, _) = Self::split(cx.area());
        match mouse.kind {
            MouseKind::Down(MouseButton::Left) if picture.contains(mouse.x, mouse.y) => {
                cx.request_focus();
                cx.capture_pointer();
                cx.memory::<Pressed>().0 = true;
                true
            }
            // Only a release whose press began here is a click: the release of the double click
            // that opened the viewer lands on it, and must not move on at once.
            MouseKind::Up(MouseButton::Left) if std::mem::take(&mut cx.memory::<Pressed>().0) => {
                if !picture.contains(mouse.x, mouse.y) {
                    return true;
                }
                let third = i32::from(picture.width) / 3;
                let at = mouse.x - picture.x;
                if at < third {
                    cx.emit(Msg::Picture(self.id, Step::Previous));
                } else if at >= i32::from(picture.width) - third {
                    cx.emit(Msg::Picture(self.id, Step::Next));
                }
                true
            }
            _ => false,
        }
    }
}

/// Whether a left press began on the picture and has not been released yet.
#[derive(Debug, Default)]
struct Pressed(bool);

impl Widget<Msg> for Viewer {
    fn measure(&self, _cx: &mut MeasureCx<'_>, available: Size) -> Size {
        available
    }

    fn paint(&self, cx: &mut PaintCx<'_>, area: Rect) {
        if area.is_empty() {
            return;
        }
        cx.register_hit(area);
        cx.register_focusable();
        let (picture, strip) = Self::split(area);
        if let Some(content) = self.children.first() {
            let size = cx.measure_child(content, picture.size()).min(picture.size());
            let x = picture.x + i32::from((picture.width - size.width) / 2);
            let y = picture.y + i32::from((picture.height - size.height) / 2);
            cx.paint_child(content, Rect::new(x, y, size.width, size.height));
        }
        if let (Some(strip), Some(line)) = (strip, self.children.get(1)) {
            cx.paint_child(line, strip);
        }
    }

    fn event(&self, cx: &mut EventCx<'_, Msg>, event: &Event) -> bool {
        match event {
            Event::Key(key) => self.on_key(cx, key),
            Event::Mouse(mouse) => self.on_mouse(cx, mouse),
            _ => false,
        }
    }

    fn focusable(&self) -> bool {
        true
    }

    fn children(&self) -> &[Node<Msg>] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Node<Msg>] {
        &mut self.children
    }
}

impl Container<Msg> for Viewer {
    fn set_children(&mut self, children: Vec<Node<Msg>>) {
        self.children = children;
    }
}
