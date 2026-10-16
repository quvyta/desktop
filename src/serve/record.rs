//! What a screen tells the background about a program's window, so the next screen opens the
//! same window: the entry it was opened from, where it is and how it is placed.
//!
//! The background keeps it as the bytes it was given and never reads it; only screens do. It
//! lives only as long as the background, which speaks one [protocol](super::wire::PROTOCOL), so
//! a screen that reads it was built by the same qdesk that wrote it; a first byte still says which
//! form it is in, so a description a screen cannot read opens no window rather than a wrong one.

use std::ffi::OsStr;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

use qframe::geometry::Rect;

use super::wire::{In, Out};
use crate::apps::{Category, Entry, Install, Launch, Localized, Screen, Source, WindowPrefs};
use crate::wm::{Edge, Placed, Placement};

/// The form this qdesk writes.
const FORM: u8 = 1;

/// The bytes that describe `placed`.
#[must_use]
pub fn encode(placed: &Placed) -> Vec<u8> {
    let mut out = Out::default();
    out.byte(FORM);
    entry(&mut out, &placed.entry);
    maybe_text(&mut out, placed.title.as_deref());
    rect(&mut out, placed.rect);
    match placed.placement {
        Placement::Floating => out.byte(0),
        Placement::Maximized { restore } => {
            out.byte(1);
            rect(&mut out, restore);
        }
        Placement::Snapped { edge, restore } => {
            out.byte(match edge {
                Edge::Left => 2,
                Edge::Right => 3,
                Edge::Top => 4,
            });
            rect(&mut out, restore);
        }
    }
    out.flag(placed.minimized);
    out.u32(u32::try_from(placed.space).unwrap_or(0));
    out.0
}

/// The window `bytes` describe; `None` when they describe none this qdesk can open.
#[must_use]
pub fn decode(bytes: &[u8]) -> Option<Placed> {
    let mut from = In(bytes);
    if from.byte().ok()? != FORM {
        return None;
    }
    let placed = read(&mut from).ok()?;
    from.end().ok()?;
    Some(placed)
}

fn read(from: &mut In<'_>) -> io::Result<Placed> {
    let entry = entry_from(from)?;
    let title = maybe_text_from(from)?;
    let rect = rect_from(from)?;
    let placement = match from.byte()? {
        0 => Placement::Floating,
        1 => Placement::Maximized { restore: rect_from(from)? },
        edge @ 2..=4 => {
            let edge = match edge {
                2 => Edge::Left,
                3 => Edge::Right,
                _ => Edge::Top,
            };
            Placement::Snapped { edge, restore: rect_from(from)? }
        }
        _ => return Err(unknown()),
    };
    let minimized = from.flag()?;
    let space = from.u32()? as usize;
    Ok(Placed { entry, title, rect, placement, minimized, space })
}

fn unknown() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "a window this qdesk cannot read")
}

fn rect(out: &mut Out, rect: Rect) {
    out.u32(rect.x.cast_unsigned());
    out.u32(rect.y.cast_unsigned());
    out.u16(rect.width);
    out.u16(rect.height);
}

fn rect_from(from: &mut In<'_>) -> io::Result<Rect> {
    let x = from.u32()?.cast_signed();
    let y = from.u32()?.cast_signed();
    Ok(Rect::new(x, y, from.u16()?, from.u16()?))
}

fn text(out: &mut Out, text: &str) {
    out.bytes(text.as_bytes());
}

fn maybe_text(out: &mut Out, value: Option<&str>) {
    out.maybe_bytes(value.map(str::as_bytes));
}

fn maybe_text_from(from: &mut In<'_>) -> io::Result<Option<String>> {
    from.maybe_bytes()?.map(|bytes| String::from_utf8(bytes).map_err(|_| unknown())).transpose()
}

fn texts(out: &mut Out, values: &[String]) {
    out.count(values.len());
    for value in values {
        text(out, value);
    }
}

fn texts_from(from: &mut In<'_>) -> io::Result<Vec<String>> {
    (0..from.count(4)?).map(|_| from.text()).collect()
}

fn pairs(out: &mut Out, values: &[(String, String)]) {
    out.count(values.len());
    for (a, b) in values {
        text(out, a);
        text(out, b);
    }
}

fn pairs_from(from: &mut In<'_>) -> io::Result<Vec<(String, String)>> {
    (0..from.count(8)?).map(|_| Ok((from.text()?, from.text()?))).collect()
}

fn localized(out: &mut Out, value: &Localized) {
    text(out, &value.default);
    pairs(out, &value.translations);
}

fn localized_from(from: &mut In<'_>) -> io::Result<Localized> {
    Ok(Localized { default: from.text()?, translations: pairs_from(from)? })
}

fn maybe_path(out: &mut Out, path: Option<&std::path::Path>) {
    out.maybe_bytes(path.map(|path| path.as_os_str().as_bytes()));
}

fn maybe_path_from(from: &mut In<'_>) -> io::Result<Option<PathBuf>> {
    Ok(from.maybe_bytes()?.map(|bytes| PathBuf::from(OsStr::from_bytes(&bytes))))
}

const SOURCES: [Source; 4] = [Source::User, Source::System, Source::Builtin, Source::DesktopFile];
const SCREENS: [Screen; 3] = [Screen::Terminal, Screen::Settings, Screen::Files];

fn index_of<T: PartialEq>(all: &[T], value: &T) -> u8 {
    all.iter().position(|item| item == value).and_then(|index| u8::try_from(index).ok()).unwrap_or(0)
}

fn pick<T: Copy>(all: &[T], index: u8) -> io::Result<T> {
    all.get(usize::from(index)).copied().ok_or_else(unknown)
}

fn entry(out: &mut Out, entry: &Entry) {
    text(out, &entry.id);
    localized(out, &entry.name);
    out.flag(entry.comment.is_some());
    if let Some(comment) = &entry.comment {
        localized(out, comment);
    }
    maybe_text(out, entry.icon.as_deref());
    match &entry.launch {
        Launch::Command(words) => {
            out.byte(0);
            texts(out, words);
        }
        Launch::Open(file) => {
            out.byte(1);
            out.os(file.as_os_str());
        }
        Launch::Screen(screen) => {
            out.byte(2);
            out.byte(index_of(&SCREENS, screen));
        }
    }
    maybe_path(out, entry.folder.as_deref());
    pairs(out, &entry.env);
    texts(out, &entry.unset);
    out.byte(index_of(&Category::ALL, &entry.category));
    texts(out, &entry.keywords);
    out.flag(entry.single);
    out.flag(entry.close_on_exit);
    out.flag(entry.window.size.is_some());
    if let Some((columns, rows)) = entry.window.size {
        out.u16(columns);
        out.u16(rows);
    }
    out.flag(entry.window.maximized);
    maybe_text(out, entry.install.qpac.as_deref());
    maybe_text(out, entry.install.quvyta.as_deref());
    maybe_text(out, entry.try_exec.as_deref());
    out.byte(index_of(&SOURCES, &entry.source));
    maybe_path(out, entry.file.as_deref());
}

fn entry_from(from: &mut In<'_>) -> io::Result<Entry> {
    let id = from.text()?;
    let name = localized_from(from)?;
    let comment = if from.flag()? { Some(localized_from(from)?) } else { None };
    let icon = maybe_text_from(from)?;
    let launch = match from.byte()? {
        0 => {
            let words = texts_from(from)?;
            // A command names a program: one with no words could start nothing.
            if words.is_empty() {
                return Err(unknown());
            }
            Launch::Command(words)
        }
        1 => Launch::Open(PathBuf::from(from.os()?)),
        2 => Launch::Screen(pick(&SCREENS, from.byte()?)?),
        _ => return Err(unknown()),
    };
    let folder = maybe_path_from(from)?;
    let env = pairs_from(from)?;
    let unset = texts_from(from)?;
    let category = pick(&Category::ALL, from.byte()?)?;
    let keywords = texts_from(from)?;
    let single = from.flag()?;
    let close_on_exit = from.flag()?;
    let size = if from.flag()? { Some((from.u16()?, from.u16()?)) } else { None };
    let maximized = from.flag()?;
    let install = Install { qpac: maybe_text_from(from)?, quvyta: maybe_text_from(from)? };
    let try_exec = maybe_text_from(from)?;
    let source = pick(&SOURCES, from.byte()?)?;
    let file = maybe_path_from(from)?;
    Ok(Entry {
        id,
        name,
        comment,
        icon,
        launch,
        folder,
        env,
        unset,
        category,
        keywords,
        single,
        close_on_exit,
        window: WindowPrefs { size, maximized },
        install,
        try_exec,
        source,
        file,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full() -> Entry {
        Entry {
            id: "logs".to_owned(),
            name: Localized {
                default: "Logs".to_owned(),
                translations: vec![("tr".to_owned(), "Günlükler".to_owned())],
            },
            comment: Some(Localized::plain("What the server said")),
            icon: Some("file-text".to_owned()),
            launch: Launch::Command(vec!["/bin/sh".to_owned(), "-c".to_owned(), "tail -f x".to_owned()]),
            folder: Some(PathBuf::from("/srv/app")),
            env: vec![("PAGER".to_owned(), "cat".to_owned())],
            unset: vec!["DISPLAY".to_owned()],
            category: Category::Development,
            keywords: vec!["journal".to_owned()],
            single: true,
            close_on_exit: true,
            window: WindowPrefs { size: Some((100, 30)), maximized: true },
            install: Install { qpac: Some("logs".to_owned()), quvyta: None },
            try_exec: Some("tail".to_owned()),
            source: Source::DesktopFile,
            file: Some(PathBuf::from("/usr/share/applications/logs.desktop")),
        }
    }

    #[test]
    fn every_part_of_a_window_reads_back_as_it_was_written() {
        let mut terminal = full();
        terminal.launch = Launch::Screen(Screen::Terminal);
        let mut viewer = full();
        viewer.launch = Launch::Open(PathBuf::from("/srv/notes.txt"));
        viewer.comment = None;
        viewer.window.size = None;
        let restore = Rect::new(2, 3, 50, 12);
        for (entry, placement) in [
            (full(), Placement::Floating),
            (terminal.clone(), Placement::Maximized { restore }),
            (viewer, Placement::Snapped { edge: Edge::Left, restore }),
            (terminal.clone(), Placement::Snapped { edge: Edge::Right, restore }),
            (terminal, Placement::Snapped { edge: Edge::Top, restore }),
        ] {
            let placed = Placed {
                entry,
                title: Some("tail".to_owned()),
                rect: Rect::new(-1, 4, 60, 20),
                placement,
                minimized: true,
                space: 3,
            };
            assert_eq!(decode(&encode(&placed)), Some(placed));
        }
    }

    #[test]
    fn what_cannot_be_read_as_a_window_is_none() {
        let placed = Placed {
            entry: full(),
            title: None,
            rect: Rect::new(0, 0, 10, 5),
            placement: Placement::Floating,
            minimized: false,
            space: 0,
        };
        let bytes = encode(&placed);
        assert_eq!(decode(&[]), None, "nothing");
        let mut later = bytes.clone();
        later[0] = FORM + 1;
        assert_eq!(decode(&later), None, "a form a later qdesk writes");
        for cut in 1..bytes.len() {
            assert_eq!(decode(&bytes[..cut]), None, "cut at {cut}");
        }
        let mut longer = bytes;
        longer.push(0);
        assert_eq!(decode(&longer), None, "with a byte more");

        let mut empty = placed;
        empty.entry.launch = Launch::Command(Vec::new());
        assert_eq!(decode(&encode(&empty)), None, "a command with no program");
    }
}
