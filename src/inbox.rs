//! The notifications the desktop has said, kept until qdesk closes.
//!
//! Everything the corner says is also written down here, so a person who was looking somewhere
//! else can read it afterwards: the dock's right side counts what has not been read and opens the
//! list (design 3.4 and 3.6). A notice that came from a window carries it, and choosing that
//! notice brings the window forward — the same thing a press on the toast does.
//!
//! **Nothing here is written to disk.** The design says the list is forgotten when qdesk closes
//! (3.6), and it is meant literally: these are the words of one sitting, and a list that came back
//! the next morning with yesterday's bells in it would be a thing to clear, not a thing to read.
//! So the inbox lives in memory only and no path is ever given to it.

use std::collections::VecDeque;

use crate::wm::WindowId;

/// How many notices are kept. Older ones fall off the end.
///
/// A number is needed at all because a program in a loop can ring the bell as fast as the pty
/// carries it, and a list without a bound would grow for as long as qdesk runs. Fifty is far more
/// than a person reads in one sitting and a few kilobytes at most.
pub const KEPT: usize = 50;

/// One thing the desktop said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    /// What stands before the notice when it comes from no window: the heading of a warning of
    /// qdesk's own. A notice that has a window is led by that window's name instead, read from the
    /// window while the list is drawn, so it is in the language on screen.
    pub lead: Option<String>,
    /// The notice itself.
    pub body: String,
    /// The window it came from, when it came from one. Choosing such a notice brings it forward.
    pub window: Option<WindowId>,
}

impl Notice {
    /// A notice of the desktop's own, belonging to no window.
    #[must_use]
    pub fn desktop(lead: String, body: String) -> Self {
        Self { lead: Some(lead), body, window: None }
    }

    /// A notice that came from the window `id`. Its own name leads it, so it carries none.
    #[must_use]
    pub fn from(id: WindowId, body: String) -> Self {
        Self { lead: None, body, window: Some(id) }
    }
}

/// The recent notices, newest first, and how many of them have not been read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inbox {
    notices: VecDeque<Notice>,
    unread: usize,
}

impl Inbox {
    /// Writes `notice` down as the newest one, unread.
    pub fn add(&mut self, notice: Notice) {
        self.notices.push_front(notice);
        self.notices.truncate(KEPT);
        // A notice that has already fallen off the end cannot be read, so the count never claims
        // more than the list holds.
        self.unread = self.unread.saturating_add(1).min(self.notices.len());
    }

    /// The notices, newest first.
    pub fn notices(&self) -> impl ExactSizeIterator<Item = &Notice> {
        self.notices.iter()
    }

    /// How many have not been read: what the dock's right side counts.
    #[must_use]
    pub fn unread(&self) -> usize {
        self.unread
    }

    /// Whether nothing has been said yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.notices.is_empty()
    }

    /// The list has been opened: nothing in it is unread any more.
    pub fn read(&mut self) {
        self.unread = 0;
    }

    /// The window `id` has closed: its notices stay and keep their words, but they no longer lead
    /// anywhere.
    ///
    /// What a program said is worth reading after its window is gone — that is the same reason a
    /// window whose program ended stays open at all (design 3.3). What cannot stay is the way
    /// forward: a row that offers to bring back a window that is not there would be a lie. The
    /// name of the window goes with it for the same reason: it is read from the window itself, and
    /// there is no window left to read.
    pub fn closed(&mut self, id: WindowId) {
        for notice in &mut self.notices {
            if notice.window == Some(id) {
                notice.window = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use qframe::geometry::Size;

    use super::*;
    use crate::apps::{Folders, load};
    use crate::wm::Windows;

    /// Two window ids, handed out by the window manager itself: an id is its to give.
    fn two_windows() -> (WindowId, WindowId) {
        let folders = Folders { user: None, system: Vec::new(), desktop_files: Vec::new() };
        let entry = load(&folders, None).entries.into_iter().next().expect("qdesk carries entries of its own");
        let mut windows = Windows::new(Size::new(80, 24));
        (windows.open(&entry), windows.open(&entry))
    }

    fn said(body: &str) -> Notice {
        Notice::desktop("qdesk".to_owned(), body.to_owned())
    }

    #[test]
    fn the_newest_notice_is_first_and_every_one_of_them_is_unread() {
        let mut inbox = Inbox::default();
        assert!(inbox.is_empty() && inbox.unread() == 0);
        inbox.add(said("bir"));
        inbox.add(said("iki"));
        let bodies: Vec<&str> = inbox.notices().map(|notice| notice.body.as_str()).collect();
        assert_eq!(bodies, ["iki", "bir"]);
        assert_eq!(inbox.unread(), 2);
    }

    #[test]
    fn reading_the_list_takes_the_count_to_zero_and_keeps_the_notices() {
        let mut inbox = Inbox::default();
        inbox.add(said("bir"));
        inbox.read();
        assert_eq!(inbox.unread(), 0);
        assert_eq!(inbox.notices().count(), 1, "what was read is still there to read again");
        inbox.add(said("iki"));
        assert_eq!(inbox.unread(), 1, "only what came after counts");
    }

    #[test]
    fn a_program_that_never_stops_talking_does_not_grow_the_list_without_end() {
        let mut inbox = Inbox::default();
        for index in 0..KEPT * 3 {
            inbox.add(said(&format!("{index}")));
        }
        assert_eq!(inbox.notices().count(), KEPT);
        assert_eq!(inbox.unread(), KEPT, "the count never claims more than the list holds");
        assert_eq!(
            inbox.notices().next().map(|notice| notice.body.as_str()),
            Some((KEPT * 3 - 1).to_string().as_str())
        );
    }

    #[test]
    fn a_closed_window_leaves_its_words_and_takes_away_the_way_back() {
        let (first, second) = two_windows();
        let mut inbox = Inbox::default();
        inbox.add(Notice::from(first, "bir".to_owned()));
        inbox.add(Notice::from(second, "iki".to_owned()));
        inbox.closed(first);
        let kept: Vec<(&str, Option<WindowId>)> =
            inbox.notices().map(|notice| (notice.body.as_str(), notice.window)).collect();
        assert_eq!(kept, [("iki", Some(second)), ("bir", None)]);
        assert_eq!(inbox.unread(), 2, "nothing was read and nothing was lost");
    }
}
