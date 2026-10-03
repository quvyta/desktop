//! The program in a window: starting it, listening to what it says, how it ended or
//! why it never started, and the body of the window that shows it.

use std::io;
use std::path::Path;

use qframe::keymap::Scope;
use qframe::prelude::*;
use qframe::widgets::Toast;

use super::{Ask, Desk, Msg};
use crate::apps::{Entry, Launch, Screen};
use crate::inbox::Notice;
use crate::notice;
use crate::session::{self, Change, Start, Subtitle};
use crate::wm::{self, Exit, Window, WindowId};

/// Why a window holds no program: the program that was to be started and what the system said.
///
/// The words are kept apart from the sentence they end up in, so a person who changes the language
/// while the window is open reads the reason in the new one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Failure {
    program: String,
    reason: String,
}

impl Desk {
    /// Starts the program of `entry` for the window `id`, on a screen the size of that window's
    /// body, and listens to it.
    pub(super) fn start(&mut self, id: WindowId, entry: &Entry) -> Command<Msg> {
        let Some(body) = self.body_size(id) else { return Command::none() };
        let started = self.programs.start(id, entry, body);
        self.answer(id, started)
    }

    /// Starts the program of the window `id` again, from the entry it was opened with: the Restart
    /// of a window whose program has ended.
    pub(super) fn restart(&mut self, id: WindowId) -> Command<Msg> {
        let Some(body) = self.body_size(id) else { return Command::none() };
        let Some(started) = self.programs.restart(id, body) else { return Command::none() };
        let answered = self.answer(id, started);
        Command::batch([answered, self.body_focus()])
    }

    /// What came of starting a program: a window that runs one is listened to, one that could not
    /// start says why, and a screen of qdesk has nothing to listen to.
    pub(super) fn answer(&mut self, id: WindowId, started: Start) -> Command<Msg> {
        match started {
            Start::Running => {
                self.failures.remove(&id);
                self.windows.mark_running(id);
                self.listen(id)
            }
            Start::Screen => Command::none(),
            Start::Failed(error) => self.failed(id, &error),
        }
    }

    /// The size the body of the window `id` is drawn at; `None` when there is no such window.
    pub(super) fn body_size(&self, id: WindowId) -> Option<Size> {
        self.windows.get(id).map(|window| wm::view::body_size(window.rect()))
    }

    /// A program that could not be started: the window stays open and says why, and the corner
    /// says it once. The system's own words are given a place to belong to, never alone.
    pub(super) fn failed(&mut self, id: WindowId, error: &io::Error) -> Command<Msg> {
        let program = self.program_word(id);
        let heading = notice::start_failed_title(&program);
        // The list keeps the whole sentence: it names the program, and a row read days later must
        // say what could not be started without the heading above it.
        self.inbox.add(Notice::from(id, notice::start_failed(&program, &error.to_string())));
        let toast = Toast::warning(heading).body(error.to_string());
        self.failures.insert(id, Failure { program, reason: error.to_string() });
        Command::toast(toast)
    }

    /// Waits for the next thing the program of the window `id` says. One wait hears one change, so
    /// another is started after every change but the last.
    pub(super) fn listen(&self, id: WindowId) -> Command<Msg> {
        let Some(watch) = self.programs.watch(id) else { return Command::none() };
        match self.patience {
            // What the desktop does: one thread, one wait, for as long as it takes.
            None => Command::perform(move || Msg::Program(watch.next())),
            // What a screen test does, so a live program with nothing to say does not hold it.
            Some(bound) => Command::perform(move || match watch.next_within(bound) {
                Some(report) => Msg::Program(report),
                None => Msg::Quiet(id),
            }),
        }
    }

    /// What a window's program said, or what became of it.
    pub(super) fn on_program(&mut self, report: session::Report) -> Command<Msg> {
        let id = report.window();
        // A report of a program that has been replaced, or of a window that is gone, changes
        // nothing and is not waited on again.
        let Some(change) = self.programs.accept(report) else { return Command::none() };
        let ended = matches!(change, Change::Ended { .. });
        let told = match change {
            // The screen changed; the frame after this update draws it.
            Change::Output | Change::Folder(_) => Command::none(),
            Change::Title(title) => {
                self.windows.set_title(id, title);
                Command::none()
            }
            Change::Bell => {
                self.called(id);
                // A bell carries no words, so the list says what happened and which window it was.
                // It stays out of the corner: a shell rings for every completion it cannot finish,
                // and a toast for each of those would be noise, while the mark on the dock and a
                // line in the list are exactly what a bell is worth.
                self.inbox.add(Notice::from(id, t!("notice.bell")));
                Command::none()
            }
            Change::Notify { title, body } => self.notify(id, title, body),
            // A copy is the program's own doing and needs no word: the text goes to the
            // clipboard and nothing is said about it, in the corner or in the list. A program that
            // copies is doing what it was asked to do, and a toast for every one of those would be
            // the noise a bell already is.
            Change::Copied(text) => Command::copy(text),
            Change::Ended { code } => self.ended(id, code),
        };
        if ended { told } else { Command::batch([told, self.listen(id)]) }
    }

    /// A program called for attention: its window's item on the dock carries a mark until the
    /// window has the keys again. A window that already has them needs none.
    pub(super) fn called(&mut self, id: WindowId) {
        if self.windows.focus() != Some(id) {
            self.attention.insert(id);
        }
    }

    /// A notification a program asked for: the mark on its item, a word in the corner, and a press
    /// on it brings the window forward.
    pub(super) fn notify(&mut self, id: WindowId, title: Option<String>, body: String) -> Command<Msg> {
        self.called(id);
        // The heading is the program's own: the title it sent, else what its window's strip says.
        let heading = title.filter(|title| !title.trim().is_empty()).or_else(|| self.said(id));
        // In the list the window's own name leads the row, so the title the program sent goes
        // with the words it belongs to.
        let written = match &heading {
            Some(heading) => t!("notice.list-said", title = heading.as_str(), body = body.as_str()),
            None => body.clone(),
        };
        self.inbox.add(Notice::from(id, written));
        // Over the lock screen the corner would show what the program said to anyone passing; the
        // list keeps it for the person who unlocks.
        if self.lock.is_some() {
            return Command::none();
        }
        let toast = match heading {
            Some(heading) => Toast::info(heading).body(body),
            None => Toast::info(body),
        };
        Command::toast(toast.on_press(Msg::Bring(id)))
    }

    /// The program of a window ended: the window keeps its last screen and says how it ended, or
    /// closes when its entry asked to. A window the person was not looking at says it in the
    /// corner as well.
    pub(super) fn ended(&mut self, id: WindowId, code: Option<i32>) -> Command<Msg> {
        let elsewhere = self.windows.focus() != Some(id);
        let said = self.said(id);
        match self.windows.mark_ended(id, code) {
            // The entry asked for the window to go with its program, so nothing is said about a
            // window that is no longer there.
            Exit::Closed => {
                self.programs.close(id);
                self.forget(id);
                if self.windows.is_empty() {
                    self.more = false;
                    self.keys = None;
                }
                self.body_focus()
            }
            Exit::Kept if elsewhere => {
                let line = match code {
                    Some(code) => t!("window.ended", code = code),
                    None => t!("window.ended-signal"),
                };
                self.inbox.add(Notice::from(id, line.clone()));
                if self.lock.is_some() {
                    return Command::none();
                }
                let toast = match said {
                    Some(said) => Toast::info(said).body(line),
                    None => Toast::info(line),
                };
                Command::toast(toast.on_press(Msg::Bring(id)))
            }
            // The keys were in the program's own screen, and that screen only shows now; they are
            // put where a window without a program keeps them, so the next key is the desktop's and
            // the two ways on are a Tab away.
            Exit::Kept => self.body_focus(),
            Exit::Unknown => Command::none(),
        }
    }

    /// What the strip of the window `id` shows after the name: what its program says about itself
    /// (its own title, else the folder it is in), else the title of a window without a program.
    pub(super) fn said(&self, id: WindowId) -> Option<String> {
        match self.programs.subtitle(id) {
            Some(Subtitle::Title(title)) => Some(title.to_owned()),
            Some(Subtitle::Folder(folder)) => Some(wm::view::folder_text(folder, self.apps.home.as_deref())),
            // A Files window says which folder it shows, as a Terminal window says which folder
            // its shell is in.
            None if self.files.contains_key(&id) => self.files.get(&id).map(|window| {
                let shown = window.manager.path(window.manager.folder());
                wm::view::folder_text(&shown, self.apps.home.as_deref())
            }),
            // A picture viewer says which folder its pictures are in.
            None if self.pictures.contains_key(&id) => self.pictures.get(&id).and_then(|window| {
                let folder = window.file().parent()?;
                Some(wm::view::folder_text(folder, self.apps.home.as_deref()))
            }),
            // A text viewer says which folder its file is in.
            None if self.texts.contains_key(&id) => self.texts.get(&id).and_then(|window| {
                let folder = window.file().parent()?;
                Some(wm::view::folder_text(folder, self.apps.home.as_deref()))
            }),
            None => self.windows.get(id).and_then(Window::title).map(str::to_owned),
        }
    }

    /// The program of the window `id` as the questions and the notices name it: the first word of
    /// its command, or the shell a Terminal window runs, without the folders it lives in.
    pub(super) fn program_word(&self, id: WindowId) -> String {
        let shell = self.apps.terminal_shell();
        let Some(window) = self.windows.get(id) else { return String::new() };
        let program: &Path = match window.launch() {
            Launch::Command(words) => match words.first() {
                Some(word) => Path::new(word),
                None => return String::new(),
            },
            Launch::Screen(Screen::Terminal) => shell.as_path(),
            Launch::Screen(Screen::Settings | Screen::Files) | Launch::Open(_) => return String::new(),
        };
        program.file_name().map_or_else(|| program.display().to_string(), |name| name.to_string_lossy().into_owned())
    }

    /// Forgets what the desktop kept about a window that is gone.
    pub(super) fn forget(&mut self, id: WindowId) {
        self.attention.remove(&id);
        self.failures.remove(&id);
        // A closed Files window's manager goes with it, and its folder watch with the manager.
        self.files.remove(&id);
        // A closed viewer's picture goes with it; a decoding still under way is dropped when it ends.
        self.pictures.remove(&id);
        // A closed text viewer's file goes with it; a reading still under way is dropped when it ends.
        self.texts.remove(&id);
        // What its program said is still worth reading; what is gone is the window to go back to.
        self.inbox.closed(id);
    }

    /// The body of a window that holds a program: the program's own screen, and under it the line
    /// of a program that has ended or the reason one never started.
    ///
    /// A window whose program has ended keeps that screen, drawn as something that only shows: the
    /// whole reason such a window stays open is that the last thing the program said is read after
    /// it is gone, and a window that forgot it would be worse than one that closed itself. Faded,
    /// out of the focus order and written to by nobody — not even with the size it is drawn at, so
    /// the window changing shape cannot reflow a screen somebody is reading. What is left of the
    /// terminal is what reading needs: the program's own colours, its wide characters, the cursor
    /// it left behind, selecting its output and scrolling back through it.
    pub(super) fn program_body(&self, id: WindowId, run: wm::Run, ui: &mut View<'_, Msg>) {
        if let Some(terminal) = self.programs.terminal(id) {
            let ended = matches!(run, wm::Run::Ended { .. });
            let terminal = if ended {
                terminal.read_only()
            } else {
                // Every key is the program's, as the framework's terminal has it, except the one
                // that takes the keys out to the desktop: without it a window of `vim` would be a
                // room with no door.
                terminal.pass_through(Scope::App, "desktop-mode")
            };
            ui.add(terminal).id(wm::view::body_id(id)).fill();
        }
        match run {
            wm::Run::Ended { code } => Self::ended_body(code, id, ui),
            // A window whose program never started says why, and the only way on is to close it:
            // there is nothing to start again.
            wm::Run::Waiting => {
                if let Some(failure) = self.failures.get(&id) {
                    Self::failed_body(failure, id, ui);
                }
            }
            wm::Run::Running => {}
        }
    }

    /// The line a window shows when its program has ended: what it ended with, and the two ways on.
    /// It does not close by itself, so an error message is read before it goes.
    pub(super) fn ended_body(code: Option<i32>, id: WindowId, ui: &mut View<'_, Msg>) {
        let said = match code {
            Some(code) => t!("window.ended", code = code),
            None => t!("window.ended-signal"),
        };
        // The line stands at the foot of the body, under the last screen,
        // so the window keeps its shape and the screen can come back above it unchanged. The
        // screen above keeps every other row: its last lines are where an error message is.
        ui.row(|ui| {
            // The sentence is what yields on a narrow window: the two ways on must not be the
            // things a row drops from its end. It is cut rather than wrapped, so the line stays one
            // row and takes no more of the last screen than that.
            ui.add(Text::new(said).role("secondary").no_wrap()).width(Length::Fill(1));
            ui.add(Button::new(t!("window.restart")).on_press(Msg::Restart(id)));
            ui.add(Button::new(t!("window.close")).on_press(Msg::Ask(Ask::Close, id)));
        })
        .gap(1)
        .fill_width();
    }

    /// The line a window shows when its program never started: which program it was and what the
    /// system said about it, and the way to put the window away.
    pub(super) fn failed_body(failure: &Failure, id: WindowId, ui: &mut View<'_, Msg>) {
        let said = notice::start_failed(&failure.program, &failure.reason);
        ui.row(|ui| {
            ui.add(Text::new(said).role("secondary")).width(Length::Fill(1));
            ui.add(Button::new(t!("window.close")).on_press(Msg::Ask(Ask::Close, id)));
        })
        .gap(1)
        .fill_width();
        ui.spacer().fill();
    }
}
