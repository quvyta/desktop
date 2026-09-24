# qdesk

**A desktop inside the terminal: windows, icons, a dock and a launcher, for the servers you reach over SSH and for small machines that need a very light desktop.**

![A qdesk desktop: a shell window with the keys, a note open beside it and the Settings screen behind](https://raw.githubusercontent.com/quvyta/desktop/main/docs/screenshots/desktop.svg)

**quvyta-desktop**, or **qdesk** for short, turns one terminal into a desktop. Every terminal program is an application: htop, vim, a shell or any other program opens in its own window, which you move, resize, minimise and close with the mouse or the keyboard. It is part of the Quvyta ecosystem of terminal applications, is built on [quvyta-framework](https://github.com/quvyta/framework) and is open source under the MIT licence.

## Where it stands

This is **0.1.9**, the tenth release. It is a young program: what is on this page is what it does on screen today, and the end of the page says plainly what it does not do yet — there is no file or picture viewer yet, and the programs in its windows do not survive a dropped connection.

## Install

```sh
cargo install quvyta-desktop
qdesk
```

If the shell cannot find `qdesk`, add `~/.cargo/bin` to your `PATH` (fish: `fish_add_path ~/.cargo/bin`).

The program is installed as `qdesk` and also as `quvyta-desktop`. It needs nothing else: no compositor, no display server, no configuration. Start it in any terminal — including the one you are sitting in over SSH.

## What it does today

- **Windows that run any terminal program.** A window starts a program in a pseudo-terminal and draws what it says, including a program that takes the whole screen and paints it itself. Drag the title strip to move a window and any edge or corner to resize it; with alt held, the left button moves it and the right button resizes it from anywhere inside it. A window pushed against the edge of the screen or the dock stays there until the pointer comes back to where it holds it. The title strip's controls minimise, maximise or close it.
- **Snapping and tiling.** A window dragged against the left or right edge takes that half of the desktop; against the top edge it takes the whole of it, and the shape is shown before you let go. One key lays every window out at once: one large window on the left, the rest stacked on the right.
- **A window whose program ended stays.** It keeps the last screen the program drew and adds a line saying it ended and with what exit code. Restart starts the program again in the same window.
- **A desktop with icons.** Icons stand on the floor wherever you drop them and keep that place; shift and an arrow move the one under the cursor. They open with a double click or with Enter. Arrow keys walk between them and a letter jumps to the next icon starting with it. A right click on an icon opens it, opens it in a new window, takes it off the desktop or shows its properties; a right click on the floor opens a new terminal, adds an application, arranges the icons or opens Settings. A folder opens in qexp, the ecosystem's file explorer, when it is installed, and in Files otherwise.
- **Four workspaces.** In desktop mode `1`–`4` go to a workspace and `alt+1`–`alt+4` send the window there; marks beside the launcher button show them, and a click on one goes there. The dock lists only the windows of the workspace on screen.
- **A status strip.** The dock shows the machine's tmux sessions, network rate (the busier direction, with its arrow), processor, memory and battery (its icon empties as it runs down) before its name; a click attaches to a session or opens btop or htop. Settings takes it away.
- **Widgets on the floor.** A clock, a calendar, the machine's readings and a note, added from the floor's menu, dragged where you want them and set from their own menus: 12 or 24 hours, seconds, the clock's colour, the first day of the week, which readings show. The note is a plain file.
- **A floor of your own.** The floor takes a colour (the theme's, Deep, Mist or Accent) and a quiet pattern: a gradient warming towards the accent, faint dots on the corners of the icon grid, or both. Every choice is made from the theme's colours and keeps the icon names readable. It can be set from a shell or another program too: `qdesk wallpaper --color deep --pattern both` writes only those two settings, `qdesk wallpaper` alone prints what is set, and an open desktop redraws its floor at once.
- **A picture on the floor.** Right click a PNG, JPEG, GIF or WebP picture in Files, or one on the desktop, and choose Set as wallpaper; or choose one in Settings, where three pictures made for qdesk (Ember, Dusk and Tide, CC0) are offered too; they are built into the program, so choosing one writes no file, and `qdesk wallpaper builtin:tide` chooses one from a shell. The picture fills the floor, drawn with half blocks in any terminal with 256 colours or more, over SSH as well; icons stand on small tiles of the theme's card tone so their names read on any picture. Where a terminal has only 16 colours the floor keeps its colour and pattern. `qdesk wallpaper ~/Pictures/harbour.jpg` sets it from a shell, `qdesk wallpaper --no-picture` takes it away. A picture costs a connection more than anything else on the desktop: about 36 bytes a cell the first time it is drawn, 364 KB on a 200x50 terminal, and again only when it or the terminal's size changes. On a terminal that speaks the kitty graphics protocol the picture is sent once as pixels instead, at about a cell's worth of pixels for every cell (up to 4K) on your own machine, and stays pixels between the icons, widgets and windows standing on it. On a sixel terminal (foot, WezTerm, xterm, mlterm, Windows Terminal) it is drawn in pixels too, but only while nothing stands on it: with icons or a window on the floor it is half blocks. Over SSH both are sent at only twice the half blocks' density (200x116 pixels on a 100x30 terminal), because a grainy photo sent sharp would cost many times what half blocks cost; that way the first screen of a photo on an empty 100x30 floor costs about 51 KB with kitty and 35 KB with sixel, against 95 KB in half blocks. Over a slow connection, `QUVYTA_GRAPHICS=halfblock` keeps it to half blocks.
- **A dock.** One row at the bottom of the screen, or at the top when Settings says so: the launcher button, the open windows in the order they were opened (a minimised one carries a mark), a control holding the windows that do not fit, the count of unread notifications, the name of the machine and the clock.
- **A launcher with search and shelves.** A compact menu in the corner above its dock button, which opens and closes it like a Start button and stays pressed while it is open. The search looks at names, descriptions and commands. The shelves are Recent, All, one for each category, and Installable — the entries whose program is not on this machine, each saying how qpac or quvyta would install it. The applications are small cards: one click or Enter opens one, a right click opens its menu, ctrl+enter puts it on the desktop. The menu keeps one size whatever it shows.
- **Lock, log out, restart, power off.** At the foot of the launcher. Restarting and powering off ask first, counting the programs still running, and ask `systemctl`, which the system allows a person's own local session. Lock covers the whole screen until the person's password is typed, checked by the system's own `unix_chkpwd`; where that helper is missing there is no Lock. Over SSH only Log out is offered: the others would reach everyone on the server.
- **Applications as small files.** A desktop entry is a short TOML file naming the application and its command. Entries come from four places, highest first: your own folder (`~/.local/share/quvyta/desktop/apps`), the system's folders, the entries built into qdesk, and the system's `.desktop` files that say `Terminal=true` — so htop, vim and btop are found without anyone writing an entry for them. A broken file never stops the others: it becomes a warning naming the file, line and column, and the Settings screen lists them.
- **Three built-in screens.** Terminal opens your shell in a window; Settings is a screen of qdesk itself; Files shows your home folder in a window, through Quvyta's shared file manager, as a file explorer does (a click chooses, a double click or Enter opens): new files and folders, renaming, cut, copy and paste, several at once, and deleting into the trash under `~/.local/share/Trash`. It starts as a list with each entry's size, date and permissions, and the window's menu on the dock draws the folder as a tree or as icons instead. Open as many Files windows as you like; each keeps its own folder, and a folder changed by another program is shown as it is now. A click on a file opens it in a terminal window of its own with your editor (`$VISUAL`, else `$EDITOR`, else `less`); a folder's menu opens it in a new Files window or opens a terminal there, and a folder a window's program stands in shows that window's icon on its row. An entry that says `open = "/var/log"` opens that folder in Files.
- **A Settings screen.** The language, theme and glyph mode every Quvyta application shares, and the Quvyta-wide update notice; which edge the dock sits on and the colour, pattern and picture of the floor; whether a folder opens in qexp, the ecosystem's file explorer, when it is installed, or in Files; how a window is dragged and how many frames a second are drawn; how many lines a terminal window remembers; and the folders entries are read from, with anything that could not be read. Settings are kept in `~/.config/quvyta/desktop.conf`, beside the other Quvyta applications, and only what you chose is ever written. An open desktop follows the file: a change saved by hand or by `qdesk wallpaper` is applied without a restart, and a line it cannot use is pointed out.
- **Notifications.** What the desktop says in the corner is also written down. The dock counts what has not been read and opens the list; choosing a notice brings the window it came from forward. The list is this sitting's only and is never written to disk.
- **The whole desktop from the keyboard.** `ctrl+alt+space` takes the keys out of the window and gives them to the desktop; the same keys give them back. While the desktop has them, the arrows pick a window, `m` and the arrows move it, `r` and the arrows size it, `z` fills the desktop, `n` takes the window to the dock, `x` closes it, `t` tiles them all, `b` opens the notifications, `space` opens the launcher and Esc steps back out. `f1` (and `?`) lists every one of these keys, and `ctrl+q` leaves qdesk — counting the programs still running and asking first.
- **Your programs are your data.** A window whose program is still running does not close without asking, and quitting qdesk counts them and asks.
- **Nine languages**: English, Turkish, German, Spanish, French, Japanese, Brazilian Portuguese, Russian and Simplified Chinese; and three glyph modes: Nerd Font, Unicode and plain ASCII.
- **Made for SSH.** What qdesk sends over the connection is counted. A dragged window is a light shaded shape that the window jumps to when you let go — one changed area a frame instead of every cell of the window, and the same everywhere, because a desktop should not behave differently according to where it runs; choose Live in Settings if you want the window itself to follow the mouse. The screen is drawn less often over a remote link than on this machine, a dragged window included, and that number can be set by hand too. Nothing has to be installed on the computer you are sitting at.

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

## No telemetry, and what goes over the network

qdesk collects no statistics and sends nothing about you, your machine or your work anywhere.

It asks one question of its own accord: whether a newer qdesk is out. When qdesk starts, at most once a day, it reads the list of published versions of `quvyta-desktop` from crates.io, the same file `cargo install` reads: one HTTPS `GET` of `https://index.crates.io/qu/vy/quvyta-desktop`. The request carries no cookie and no identifier; its headers are `Host: index.crates.io`, `User-Agent: quvyta-desktop/<the version you run>` and `Accept: */*`, and nothing else. crates.io sees, as with any connection, the address it comes from. When a newer version is out, a notice in the corner says which one and how to update. When there is no network, or crates.io does not answer within ten seconds, nothing is said and the next day asks again. The time of the last question is kept in `~/.local/state/quvyta/desktop/update-check` on Linux.

To turn it off, switch off **Say when an update is out** in **Settings**. The switch belongs to the whole Quvyta ecosystem: it is `update-notice = false` in `~/.config/quvyta/quvyta.conf`, and turning it off stops the question in every Quvyta application. While it is off, qdesk asks nothing at all.

Apart from that question, qdesk itself connects to nothing. The programs you run in its windows are your programs and do whatever they do: a shell, `ssh` or a browser in a window reaches the network as it would in any other terminal.

## What 0.1 does not have

- **No file or picture viewer of its own.** Files shows each file with the icon of its kind and opens it with the terminal program your system sets for that kind, or in your editor or `less`; a file's menu offers **Open with**; an entry that says `open = "some/file"` opens nothing yet, and qdesk says so rather than pretending. The viewers are for a later release.
- **A sixel picture is sharp only on an empty floor.** On a terminal that speaks the kitty graphics protocol (kitty, WezTerm, Ghostty, Konsole) the wallpaper stays real pixels around icons, widgets and windows. A sixel terminal paints the picture into the cells, so it is drawn in pixels only while nothing at all stands on it; with icons, a widget or a window it is half blocks. Icons are glyphs.
- **No surviving a dropped connection**, as above.
- **Booting straight into qdesk is not packaged.** The `session/` folder has the files and the steps to start qdesk at boot on the bare screen with kmscon, with no compositor; tested on a Raspberry Pi 5. Nothing installs them for you yet.
- **No packages yet** beyond crates.io: nothing in the AUR or in a distribution's own repositories.

## Licence

MIT, see [LICENSE](LICENSE).
