//! One window: what it holds, where it is, and what state it is in.
//!
//! A window carries the whole entry it was opened from, not only its id. It is the window's own
//! copy: the launcher may reload its folders while the window is open, and a program is restarted
//! with the command it was started with, not with whatever the file says later.

use qframe::geometry::Rect;

use crate::apps::{Entry, Launch, Screen};

use super::snap::Edge;

/// The name of one window, unique while the desktop runs and never given to another window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowId(u64);

impl WindowId {
    /// The number behind the id, for a stable order and for keying the screen's own tables.
    #[must_use]
    pub fn number(self) -> u64 {
        self.0
    }

    /// The id after this one.
    pub(super) fn next(self) -> Self {
        Self(self.0 + 1)
    }

    /// The first id a desktop hands out.
    pub(super) fn first() -> Self {
        Self(1)
    }
}

/// How far along the program of a window is.
///
/// The window manager only keeps this: it starts nothing and waits for nothing. The part of the
/// desktop that owns the terminal sessions sets it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Run {
    /// The window is open but the program has not been started yet.
    Waiting,
    /// The program is running.
    Running,
    /// The program has ended, with the exit code the system gave for it.
    Ended {
        /// The exit code, or `None` when the program was killed by a signal instead.
        code: Option<i32>,
    },
}

/// What a window holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Body {
    /// The program its entry names, and how far along it is.
    Program(Run),
    /// A screen qdesk draws itself — settings, or a file in a viewer. Nothing is started, so
    /// there is no run state; which screen it is is in the entry's [`Launch`].
    Screen,
}

/// How a window is placed on the desktop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// At a rectangle of its own, which the person moved and sized.
    Floating,
    /// Filling the desktop.
    Maximized {
        /// The rectangle it had before, and gets back when it is restored.
        restore: Rect,
    },
    /// Snapped to an edge of the desktop.
    Snapped {
        /// Which edge.
        edge: Edge,
        /// The rectangle it had before, and gets back when it floats again.
        restore: Rect,
    },
}

impl Placement {
    /// The rectangle the window goes back to, for a placement that replaced one.
    #[must_use]
    pub fn restore(self) -> Option<Rect> {
        match self {
            Self::Floating => None,
            Self::Maximized { restore } | Self::Snapped { restore, .. } => Some(restore),
        }
    }
}

/// One window of the desktop.
///
/// Its rectangle and state are only changed through the window list, which keeps them inside the
/// desktop and no smaller than the smallest window size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    pub(super) id: WindowId,
    pub(super) entry: Entry,
    pub(super) body: Body,
    pub(super) title: Option<String>,
    pub(super) rect: Rect,
    pub(super) placement: Placement,
    pub(super) minimized: bool,
    /// The workspace the window belongs to, from 0; see [`SPACES`](super::SPACES).
    pub(super) space: usize,
}

impl Window {
    /// The window's id.
    #[must_use]
    pub fn id(&self) -> WindowId {
        self.id
    }

    /// The entry the window was opened from.
    #[must_use]
    pub fn entry(&self) -> &Entry {
        &self.entry
    }

    /// What the window holds.
    #[must_use]
    pub fn body(&self) -> Body {
        self.body
    }

    /// How far along the program is, for a window that holds one.
    #[must_use]
    pub fn run(&self) -> Option<Run> {
        match self.body {
            Body::Program(run) => Some(run),
            Body::Screen => None,
        }
    }

    /// Whether a program of this window is running now. A window whose program ended, or that
    /// draws a screen of qdesk, closes without asking.
    #[must_use]
    pub fn is_running(&self) -> bool {
        matches!(self.body, Body::Program(Run::Running))
    }

    /// The title the program gave itself, when it gave one.
    #[must_use]
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// What the window's entry starts.
    #[must_use]
    pub fn launch(&self) -> &Launch {
        &self.entry.launch
    }

    /// The screen of qdesk the window draws, for a window that draws one.
    #[must_use]
    pub fn screen(&self) -> Option<Screen> {
        match self.entry.launch {
            Launch::Screen(screen) => Some(screen),
            Launch::Command(_) | Launch::Open(_) => None,
        }
    }

    /// Where the window is now, whether it floats, is maximized or is snapped.
    #[must_use]
    pub fn rect(&self) -> Rect {
        self.rect
    }

    /// How the window is placed.
    #[must_use]
    pub fn placement(&self) -> Placement {
        self.placement
    }

    /// Whether the window fills the desktop.
    #[must_use]
    pub fn is_maximized(&self) -> bool {
        matches!(self.placement, Placement::Maximized { .. })
    }

    /// The edge the window is snapped to, when it is snapped.
    #[must_use]
    pub fn snapped_to(&self) -> Option<Edge> {
        match self.placement {
            Placement::Snapped { edge, .. } => Some(edge),
            Placement::Floating | Placement::Maximized { .. } => None,
        }
    }

    /// Whether the window is minimized: it keeps its place in the order and its program keeps
    /// running, but it is not drawn and cannot hold the focus.
    #[must_use]
    pub fn is_minimized(&self) -> bool {
        self.minimized
    }

    /// The workspace the window belongs to, counted from 0.
    #[must_use]
    pub fn space(&self) -> usize {
        self.space
    }
}
