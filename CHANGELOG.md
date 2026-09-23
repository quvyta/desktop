# Changelog

Every release of quvyta-desktop, newest first. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/). While the version starts with 0, a minor release may change the files qdesk writes; the notes say so when it does.

## 0.1.1 - 2026-09-23

### Added

- A screen too narrow for the desktop's icons says in one quiet line, in the middle of the floor, where the applications went: `Applications: ❖, or ctrl alt space then space`. Where that does not fit, only the dock's button is named, so the line is never cut half way.
- Seven more languages: German, Spanish, French, Japanese, Brazilian Portuguese, Russian and Simplified Chinese, beside English and Turkish.

### Fixed

- The last lines a program wrote before it ended stay in sight. The line that says it ended used to take several rows of the window from its last screen, so at 80 by 24 a program that printed sixty lines showed only up to the fifty-fourth, and the last lines are where an error message is. The line now takes one row under the screen.
- The field of the remembered lines shows its whole number: ten thousand read as `000`.
- Keys that arrive together are no longer lost: a name typed at once, keys passed through tmux or a slow connection, or a paste in a terminal without bracketed paste, reach the launcher's search and the settings whole instead of keeping only the last key.
- A button, a switch or any other control pressed by a click acts only when the press began on it, so the release of the click that opens a window can no longer press a control the new window puts in the same place.

### Changed

- Built on quvyta-framework 0.1.18.

## 0.1.0 - 2026-09-20

The first release: windows, a desktop with application icons, a dock and a launcher inside the terminal, meant first of all for a server reached over SSH.

### Added

- Every terminal program is an application: a window runs any program through a real terminal, and a desktop entry is a small TOML file with a name, an icon and a command. Terminal programs of the system's own `.desktop` entries are listed too.
- Windows are moved and sized with the mouse or the keys, snapped to an edge, maximized, minimized to the dock and laid out at once. Desktop mode (`ctrl+alt+space`) takes the keys out of a window to the desktop and back.
- A window whose program is still running asks before it closes, from every way of closing it, and leaving qdesk counts the running programs and asks. The window of a program that has ended keeps its last screen, faded and readable.
- The dock always shows the machine's name, so the server you are on is never in doubt; the clock and the count of unread notifications give way first on a narrow row.
- Settings for the theme, the language, the dock's place, the way a window is dragged, the frame limit and the scrollback. English and Turkish.
