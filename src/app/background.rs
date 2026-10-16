//! The desktop's side of the background: putting back the windows of the programs a screen
//! attaches to, telling the background where windows are, and leaving.

use qframe::prelude::*;
use qframe::widgets::Toast;

use super::{Desk, Msg};
use crate::apps::{Category, Entry, Install, Launch, Localized, Screen, Source, WindowPrefs};
use crate::inbox::Notice;
use crate::serve::record;
use crate::wm::WindowId;

/// Why a screen runs its programs itself, though it would have run them in the background.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Alone {
    /// The background answering on the socket was started by another qdesk, which speaks another
    /// protocol; it is left as it is, since it may hold programs.
    Mismatch {
        /// The version of the qdesk that runs it.
        version: String,
    },
    /// No background answered and none could be started.
    Unreached {
        /// What the system said.
        reason: String,
    },
}

impl Desk {
    /// Opens again the window of every program the background kept, where it was, and listens to
    /// each: a program still running goes on in it, one that ended while nobody was attached says
    /// so as any program does when it ends. Done once, at the first size the terminal gives,
    /// since windows are placed inside the desktop and before that it has no size.
    pub(super) fn reopen_kept(&mut self) -> Command<Msg> {
        let kept = std::mem::take(&mut self.kept);
        let mut listening = Vec::new();
        for kept in kept {
            let (id, entry) = match record::decode(&kept.record.window) {
                Some(placed) => (self.windows.reopen(&placed), placed.entry),
                // A description this qdesk cannot read still holds a program of the person's: it
                // gets a window of its own rather than none.
                None => {
                    let entry = self.unplaced(kept.record.title.as_deref());
                    (self.windows.open(&entry), entry)
                }
            };
            self.programs.adopt(id, &entry, kept);
            self.windows.mark_running(id);
            listening.push(self.listen(id));
        }
        Command::batch(listening)
    }

    /// Tells the background where every window with a program in it is now. Only what changed
    /// is sent; a screen of its own sends nothing.
    pub(super) fn describe_windows(&mut self) {
        if self.programs.background().is_none() {
            return;
        }
        let described: Vec<(WindowId, Vec<u8>)> = self
            .windows
            .iter()
            .filter_map(|window| Some((window.id(), record::encode(&self.windows.placed(window.id())?))))
            .collect();
        for (id, bytes) in described {
            self.programs.describe(id, bytes);
        }
    }

    /// What the background is told about the window `id` when its program starts.
    pub(super) fn described(&self, id: WindowId) -> Vec<u8> {
        self.windows.placed(id).map(|placed| record::encode(&placed)).unwrap_or_default()
    }

    /// Leaving the desktop: the person asked to, so the background ends its programs. Detaching
    /// is [`Command::quit`] alone.
    pub(super) fn leave(&self) -> Command<Msg> {
        self.programs.finish();
        Command::quit()
    }

    /// The entry of a window whose description could not be read: a terminal window, named by the
    /// title its program last gave.
    fn unplaced(&self, title: Option<&str>) -> Entry {
        let mut entry = self.catalog.get("terminal").cloned().unwrap_or_else(|| Entry {
            id: "terminal".to_owned(),
            name: Localized::plain("Terminal"),
            comment: None,
            icon: None,
            launch: Launch::Screen(Screen::Terminal),
            folder: None,
            env: Vec::new(),
            unset: Vec::new(),
            category: Category::System,
            keywords: Vec::new(),
            single: false,
            close_on_exit: false,
            window: WindowPrefs::default(),
            install: Install::default(),
            try_exec: None,
            source: Source::Builtin,
            file: None,
        });
        if let Some(title) = title.filter(|title| !title.trim().is_empty()) {
            entry.name = Localized::plain(title);
        }
        entry
    }

    /// Says once, in the corner and in the list, that this screen runs its programs itself and
    /// why: they end when the screen does.
    pub(super) fn tell_alone(&mut self) -> Command<Msg> {
        let Some(alone) = self.alone.take() else { return Command::none() };
        let body = match &alone {
            Alone::Mismatch { version } => t!("background.mismatch", version = version.as_str()),
            Alone::Unreached { reason } => t!("background.unreached", reason = reason.as_str()),
        };
        let heading = t!("background.alone-title");
        self.inbox.add(Notice::desktop(heading.clone(), body.clone()));
        Command::toast(Toast::warning(heading).body(body))
    }
}
