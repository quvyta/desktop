# Changelog

Every release of quvyta-desktop, newest first. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/). While the version starts with 0, a minor release may change the files qdesk writes; the notes say so when it does.

## 0.1.6 - 2026-09-24

### Added

- The Start menu opens an application with one click, keeps one size whatever it shows, and draws the applications as cards again. At its foot: **Lock**, **Log out**, **Restart** and **Power off**. Restarting and powering off ask first and count the programs still running; over SSH only Log out is offered, so a server is never turned off from a remote desktop. Lock covers everything and asks for your password, checked by the system's own password helper, without root.
- Four workspaces: `1`–`4` in desktop mode, `alt+1`–`alt+4` to send a window, marks beside the launcher button, and **Move to workspace** in a window's menu.
- Widgets on the floor: a clock, a calendar, the machine's readings (processor, memory, network, battery) and a note. Add them from the floor's menu, drag them anywhere, change or remove them from their own menus. A clock without seconds costs about 150 bytes a minute over SSH.
- The floor takes a pattern beside its colour: a gradient, dots, or both (**Settings → Desktop → Floor pattern**).
- A status strip on the dock, before the machine's name: the tmux sessions, the network rate, processor, memory and battery (only where there is one). A click on tmux opens a session in a new window; a click on processor or memory opens btop or htop. Warnings show in the theme's warning colour with a mark. On a narrow dock the strip gives way first. **Settings → Desktop → Status on the dock** takes it away, and then the machine is not read at all. A quiet machine costs about 120 bytes a minute over SSH.
- The Desktop folder stands on the floor, by the name your language gives it (`~/Masaüstü`, `~/Schreibtisch`). **New folder** and **Rename** are in the menus, and a folder opens in qexp or Files. A machine without a Desktop folder, as most servers are, shows the applications alone; nothing is created.
- Files draws each file with the icon of its kind, opens it with the terminal program your system sets for that kind, and offers **Open with** in the file's menu. A graphical program is never started.

### Changed

- Files behaves like a file explorer: a click chooses a row, a double click or Enter opens it.
- Several selected icons move together when one of them is dragged; shift with a click selects a range.
- How to resize a window is shown: a **Resize** row in a window's menu, a hint on the sizing row, and a note the first time a window opens.
- Built on quvyta-framework 0.1.23.

## 0.1.5 - 2026-09-24

### Added

- Icons stay where you put them. Drag an icon to any free cell of the floor and it stays there, across restarts; dropped on another icon, the two swap places. While you drag, the cell it will land in is lit. Shift and an arrow move the icon under the cursor. **Arrange icons** puts them back in order. A desktop file from an earlier version still reads; its icons keep their order until you move one.
- The launcher is a compact Start menu: at most 64 columns wide, in the corner above its dock button, with its applications in one list beside the shelves. The button opens and closes it and stays pressed while it is open; space typed into an empty search closes it too.
- The floor can take another colour: **Settings → Desktop → Floor** offers Deep, Mist and Accent besides the theme's own. Each is made from the theme's colours, so it suits every theme; windows and the dock keep the theme.
- A folder opens in qexp, the Quvyta ecosystem's file explorer, when it is installed, and in a Files window otherwise: a folder entry, a folder on the floor, and **Open in a new window** in Files. **Settings → Desktop → Open folders with qexp** turns it off.
- `session/`: the files and the steps to boot a machine straight into qdesk, drawn on the bare screen by kmscon with no compositor. Tested on a Raspberry Pi 5. Nothing installs them for you.

### Changed

- Built on quvyta-framework 0.1.21.

## 0.1.4 - 2026-09-23

### Changed

- qdesk speaks of the Quvyta ecosystem instead of a family. The launcher's shelf of Quvyta's own applications is called **Quvyta** in every language, and the settings, the README and the entries of Quvyta's applications say "Quvyta applications" or "the Quvyta ecosystem". An entry file that still says `category = "family"` lands on the Quvyta shelf as before, without a warning; new entries write `category = "quvyta"`.

## 0.1.3 - 2026-09-23

### Added

- A Files window: the Quvyta family's shared file manager, in a window of the desktop. It opens on your home folder as a list with sizes, dates and permissions; the window's menu on the dock turns it into a tree or icons. Each Files window keeps its own folder and view, and deleting moves things to the trash. Files is on the desktop from the first start.
- A click on a file opens it in your editor, in a terminal window of its own: `$VISUAL`, else `$EDITOR`, else `less`. A folder's menu opens it in a new Files window or starts a terminal there.
- A folder where a window's program is working carries that window's icon in the accent colour, and loses it when the window closes.
- A desktop entry that opens a folder (`open = "/var/log"`) now opens a Files window there. An entry that opens a file still says there is no viewer yet.

### Fixed

- The README lists the headers the update question really sends: `Host`, `User-Agent` and `Accept`. It named an `Accept-Encoding` header that is never sent.

### Changed

- Built on quvyta-framework 0.1.19.

## 0.1.2 - 2026-09-23

### Added

- qdesk says when a newer version of itself is out. When it starts, at most once a day, it reads the list of published versions of `quvyta-desktop` from crates.io, one HTTPS request with no cookie and no identifier, and a notice in the corner names the new version. No network means no notice. This is on by default and belongs to the whole family: **Say when an update is out** on the Settings screen turns it off in every Quvyta application. The README's "No telemetry, and what goes over the network" section says exactly what is sent.

### Changed

- Sizing a window with alt and the right button is tested from each of its four edges and four corners, and over a program that reads the mouse. qdesk no longer keeps its own copy of the nearest-edge rule; the framework's window decides it, the same everywhere.

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
