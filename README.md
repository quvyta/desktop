# qdesk

**A desktop inside the terminal: windows, icons, a dock and a launcher, for the servers you reach over SSH and for small machines that need a very light desktop.**

![A qdesk desktop: a shell window with the keys, a note open beside it and the Settings screen behind](https://raw.githubusercontent.com/quvyta/desktop/main/docs/screenshots/desktop.svg)

**quvyta-desktop**, or **qdesk** for short, turns one terminal into a desktop. Every terminal program is an application: htop, vim, a shell or any other program opens in its own window, which you move, resize, minimise and close with the mouse or the keyboard. It is part of the Quvyta family of terminal applications, is built on [quvyta-framework](https://github.com/quvyta/framework) and is open source under the MIT licence.

## Where it stands

This is **0.1.0**, the first release. It is a young program: what is on this page is what it does on screen today, and the end of the page says plainly what it does not do yet — there is no file manager and no picture viewer, and the programs in its windows do not survive a dropped connection.

## Install

```sh
cargo install quvyta-desktop
qdesk
```

If the shell cannot find `qdesk`, add `~/.cargo/bin` to your `PATH` (fish: `fish_add_path ~/.cargo/bin`).

The program is installed as `qdesk` and also as `quvyta-desktop`. It needs nothing else: no compositor, no display server, no configuration. Start it in any terminal — including the one you are sitting in over SSH.

## What it does today

- **Windows that run any terminal program.** A window starts a program in a pseudo-terminal and draws what it says, including a program that takes the whole screen and paints it itself. Drag the title strip to move a window, an edge or a corner to resize it, and use the title strip's controls to minimise, maximise or close it.
- **Snapping and tiling.** A window dragged against the left or right edge takes that half of the desktop; against the top edge it takes the whole of it, and the shape is shown before you let go. One key lays every window out at once: one large window on the left, the rest stacked on the right.
- **A window whose program ended stays.** It keeps the last screen the program drew and adds a line saying it ended and with what exit code. Restart starts the program again in the same window.
- **A desktop with icons.** Icons stand on a bare floor, moved into the order you want, opened with a double click or with Enter. Arrow keys walk between them and a letter jumps to the next icon starting with it. A right click on an icon opens it, opens it in a new window, takes it off the desktop or shows its properties; a right click on the floor opens a new terminal, adds an application, arranges the icons or opens Settings.
- **A dock.** One row at the bottom of the screen, or at the top when Settings says so: the launcher button, the open windows in the order they were opened (a minimised one carries a mark), a control holding the windows that do not fit, the count of unread notifications, the name of the machine and the clock.
- **A launcher with search and shelves.** It rises above the dock without dimming what is behind it. The search looks at names, descriptions and commands. The shelves are Recent, All, one for each category, and Installable — the entries whose program is not on this machine, each saying how qpac or quvyta would install it. Enter opens an application, ctrl+enter puts it on the desktop.
- **Applications as small files.** A desktop entry is a short TOML file naming the application and its command. Entries come from four places, highest first: your own folder (`~/.local/share/quvyta/desktop/apps`), the system's folders, the entries built into qdesk, and the system's `.desktop` files that say `Terminal=true` — so htop, vim and btop are found without anyone writing an entry for them. A broken file never stops the others: it becomes a warning naming the file, line and column, and the Settings screen lists them.
- **Two built-in screens.** Terminal opens your shell in a window; Settings is a screen of qdesk itself.
- **A Settings screen.** The language, theme and glyph mode every Quvyta application shares; how a window is dragged and how many frames a second are drawn; how many lines a terminal window remembers; and the folders entries are read from, with anything that could not be read. Settings are kept in `~/.config/quvyta/desktop.conf`, beside the other applications of the family, and only what you chose is ever written.
- **Notifications.** What the desktop says in the corner is also written down. The dock counts what has not been read and opens the list; choosing a notice brings the window it came from forward. The list is this sitting's only and is never written to disk.
- **The whole desktop from the keyboard.** `ctrl+alt+space` takes the keys out of the window and gives them to the desktop; the same keys give them back. While the desktop has them, the arrows pick a window, `m` and the arrows move it, `r` and the arrows size it, `z` fills the desktop, `n` takes the window to the dock, `x` closes it, `t` tiles them all, `b` opens the notifications, `space` opens the launcher and Esc steps back out. `f1` (and `?`) lists every one of these keys, and `ctrl+q` leaves qdesk — counting the programs still running and asking first.
- **Your programs are your data.** A window whose program is still running does not close without asking, and quitting qdesk counts them and asks.
- **English and Turkish**, and three glyph modes: Nerd Font, Unicode and plain ASCII.
- **Made for SSH.** What qdesk sends over the connection is counted. A dragged window is a light shaded shape that the window jumps to when you let go — one changed area a frame instead of every cell of the window, and the same everywhere, because a desktop should not behave differently according to where it runs; choose Live in Settings if you want the window itself to follow the mouse. The screen is drawn less often over a remote link than on this machine, and that number can be set by hand too. Nothing has to be installed on the computer you are sitting at.

## What it looks like

A window runs a program and shows what it says, nothing else:

![A window running a shell, its body the output of ls, cat and wc](https://raw.githubusercontent.com/quvyta/desktop/main/docs/screenshots/window.svg)

A full-screen program draws its own screen inside the window, and keeps running there:

![A window holding a full-screen program that has drawn the project folder](https://raw.githubusercontent.com/quvyta/desktop/main/docs/screenshots/program.svg)

The launcher searches every application, with its shelves and a shelf for what is not installed yet:

![The launcher open over the desktop, with its shelves and its applications](https://raw.githubusercontent.com/quvyta/desktop/main/docs/screenshots/launcher.svg)

And on the 80x24 terminal of a small SSH window, the same desktop with two windows side by side and Settings waiting on the dock:

![The same desktop on an 80x24 terminal, two windows side by side](https://raw.githubusercontent.com/quvyta/desktop/main/docs/screenshots/desktop-80x24.svg)

## An application, written out

An entry is a file such as `~/.local/share/quvyta/desktop/apps/htop.toml`:

```toml
name = "htop"
comment = { en = "What the machine is doing", tr = "Makine ne yapıyor" }
icon = "prompt"
command = ["htop"]
category = "system"
keywords = ["process", "monitor", "süreç"]

[window]
size = [90, 30]
```

The file may also say the folder the program starts in (`folder`), variables to set (`env`), whether only one window of it opens at a time (`single`), whether the window closes when the program ends (`close_on_exit`), whether the window opens maximized, and, under `[install]`, the package qpac or quvyta installs when the program is not on this machine. `hidden = true` hides an id that a lower place declared.

## Keeping programs alive when the connection drops

**qdesk does not survive a dropped connection.** When the terminal it draws on goes away, qdesk goes with it and so do the programs in its windows. The programs live in qdesk's own process; nothing keeps them running on the machine by itself. Until that changes, open it inside tmux:

```sh
tmux new -A -s desktop qdesk
```

Reconnect and run the same command: the desktop and every program in it are where you left them.

## What 0.1 does not have

- **No file manager and no file or picture viewer.** An entry may say `open = "some/path"` instead of a command, but nothing opens it yet: qdesk says so rather than pretending. The Files application and the viewers are for a later release.
- **No pictures in the terminal.** There is no kitty graphics, no sixel and no half-block drawing. Icons are glyphs.
- **No surviving a dropped connection**, as above.
- **No local session files.** There is no `qdesk-session`, no kiosk compositor and no `.desktop` session entry in this repository. On a machine with no graphical desktop you start qdesk in whatever terminal that machine already has.
- **No packages yet** beyond crates.io: nothing in the AUR or in a distribution's own repositories.

## Licence

MIT, see [LICENSE](LICENSE).
