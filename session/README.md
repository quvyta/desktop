# qdesk-session: boot straight into qdesk

These files make a machine with no desktop start qdesk on its screen at boot, with no compositor and no window system in between. [kmscon](https://github.com/kmscon/kmscon) draws a terminal directly onto the screen through the kernel's DRM/KMS; qdesk is the only program it runs. Colours are full 24-bit, fonts are real TrueType fonts (a Nerd Font works), and the mouse reaches qdesk: icons are clicked and dragged, windows moved.

It is optional and separate from qdesk: qdesk itself knows nothing about it. Nothing here is installed by `cargo install`. Installing it changes how the machine boots, so do it only on a machine you mean to use this way.

Tested on a Raspberry Pi 5 (Raspberry Pi OS, Debian 13) and in a virtual machine with a virtio GPU.

## What you need

- qdesk, as `/usr/local/bin/qdesk` or anywhere on the user's `PATH`.
- kmscon 10 or newer, at `/usr/local/bin/kmscon`. Arch Linux has it as a package (`pacman -S kmscon`; then change the path in the unit to `/usr/bin/kmscon`). On Debian and Raspberry Pi OS it is built from source:

```sh
sudo apt install meson ninja-build pkg-config git libdrm-dev libxkbcommon-dev libudev-dev \
  libfreetype-dev libfontconfig-dev libseat-dev libpango1.0-dev libegl-dev libgbm-dev libgles-dev check fonts-hack
git clone https://github.com/kmscon/libtsm && (cd libtsm && meson setup build --prefix=/usr/local && ninja -C build && sudo ninja -C build install)
git clone https://github.com/kmscon/kmscon && (cd kmscon && git checkout "$(git describe --tags --abbrev=0)" && meson setup build --prefix=/usr/local && ninja -C build && sudo ninja -C build install)
sudo ldconfig
```

- A font: `fonts-hack` above, or any monospace font; a Nerd Font gives qdesk its richest icons.

## Install

```sh
sudo install -m644 qdesk-session.service /etc/systemd/system/
sudo install -m644 qdesk-session.conf /etc/
sudoedit /etc/qdesk-session.conf          # set QDESK_USER to your user name
sudo systemctl daemon-reload
sudo systemctl disable getty@tty1.service
sudo systemctl enable --now qdesk-session.service
```

qdesk now fills the first console and comes back at every boot. The other consoles still have their ordinary login (ctrl+alt+F2), and SSH is untouched.

## Undo

```sh
sudo systemctl disable --now qdesk-session.service
sudo systemctl enable --now getty@tty1.service
sudo rm /etc/systemd/system/qdesk-session.service /etc/qdesk-session.conf
sudo systemctl daemon-reload
```

## Notes

- kmscon waits while no screen is plugged in, and qdesk starts the moment one is.
- The font and its size are in `/etc/qdesk-session.conf`; a larger size gives fewer, bigger cells.
- qdesk runs as the user you name, with that user's settings in `~/.config/quvyta`.
