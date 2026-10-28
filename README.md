# AK820 Studio

[![CI](https://github.com/WhatTFF/linux-ajazz-ak820-pro-app/actions/workflows/ci.yml/badge.svg)](https://github.com/WhatTFF/linux-ajazz-ak820-pro-app/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/WhatTFF/linux-ajazz-ak820-pro-app?include_prereleases&label=release&color=ff6a2b)](https://github.com/WhatTFF/linux-ajazz-ak820-pro-app/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/platform-Linux%20x86__64-lightgrey.svg)](#download)

Native Linux app for the **AJAZZ / Epomaker AK820 Pro** keyboard screen — a Rust rewrite of
[fromagge/ak820-screen](https://github.com/fromagge/ak820-screen) with automatic clock sync.

Put GIFs, images, video, text, drawings and procedural animations on the 0.85" 128×128 screen,
then forget about it: the app keeps the keyboard clock in step with your system clock on boot,
on plug-in, after suspend and after timezone changes.

> Unofficial. Not affiliated with AJAZZ or Epomaker. Protocol recovered by the upstream project
> from the vendor's own WebHID driver — see [PROTOCOL.md](PROTOCOL.md).

**Source, issues and releases:** <https://github.com/WhatTFF/linux-ajazz-ak820-pro-app> ·
[releases](https://github.com/WhatTFF/linux-ajazz-ak820-pro-app/releases) ·
[issues](https://github.com/WhatTFF/linux-ajazz-ak820-pro-app/issues)

## Download

[![Download the RPM](https://img.shields.io/badge/download-.rpm-294172?style=for-the-badge&logo=fedora&logoColor=white)](https://github.com/WhatTFF/linux-ajazz-ak820-pro-app/releases)
[![Download the DEB](https://img.shields.io/badge/download-.deb-A81D33?style=for-the-badge&logo=debian&logoColor=white)](https://github.com/WhatTFF/linux-ajazz-ak820-pro-app/releases)
[![Download the AppImage](https://img.shields.io/badge/download-.AppImage-ff6a2b?style=for-the-badge&logo=linux&logoColor=white)](https://github.com/WhatTFF/linux-ajazz-ak820-pro-app/releases)

Every [release](https://github.com/WhatTFF/linux-ajazz-ak820-pro-app/releases) carries the three
x86_64 packages plus a `SHA256SUMS` file; the newest release is listed first. You do not have to
come back here for updates: the app checks for new releases itself and offers to install them.

| Format | Distros | Install |
| --- | --- | --- |
| `.rpm` | Fedora, RHEL, openSUSE | `sudo dnf install ./ak820-studio-*.rpm` |
| `.deb` | Debian, Ubuntu, Mint, Pop!\_OS | `sudo apt install ./ak820-studio_*.deb` |
| `.AppImage` | any glibc distro, no root | `chmod +x ak820-studio_*.AppImage && ./ak820-studio_*.AppImage` |

All three need **WebKitGTK 4.1** and GTK 3 at runtime (already installed on Fedora Workstation
and Ubuntu 22.04+). The RPM also installs the udev rule and the systemd autosync units; for the
DEB and AppImage install the udev rule once by hand:

```bash
sudo install -Dm644 udev/70-ak820.rules /usr/lib/udev/rules.d/70-ak820.rules
sudo udevadm control --reload && sudo udevadm trigger
```

Then unplug and replug the keyboard so the new permissions apply.

## Status

Working on hardware (fw 1.07, PID `0c45:8009`):

- `ak820ctl` — `info`, `time`, `upload`, `builtin`, `history list|export|restore|clear`, `raw`
- `ak820-studio` — Tauri window with the full upstream UI (animations, image/GIF/video, text,
  draw, effects, adjust), native device access, on-disk history
- **Settings window**: start at login, start hidden in the tray, keep running in the tray,
  sync on start (then every 12 h while running), sync on plug-in, restore the last image
  on plug-in
- **Tray icon**: open, sync now, quit
- **In-app updates**: checks GitHub releases quietly every 12 hours (toggleable) and pops up a
  window when something newer exists. Also checks whenever the app opens — cold start or restored
  from the tray — so you never miss a version. The AppImage replaces itself in place; RPM/DEB
  download to `~/Downloads`, then either install through the desktop's polkit password prompt or
  show the exact `sudo dnf`/`sudo apt` command to copy. "Later" silences that version until the next one.
- **Dongle-aware**: on the 2.4 GHz dongle the clock still syncs and the UI explains that pictures
  need the cable; designs can always be saved to history
- clock autosync units (`systemd/`), udev rule, RPM spec, CI, deb/rpm/AppImage releases

Not done yet: video import fallback for missing codecs, GIPHY key in the keyring,
SQLite-backed history, Flatpak, COPR. See [PLAN.md](PLAN.md).

## Install (from source, Fedora)

```bash
sudo dnf install gtk3-devel webkit2gtk4.1-devel libsoup3-devel librsvg2-devel openssl-devel \
                 cargo rust gcc gcc-c++ pkgconf-pkg-config
cargo build --release -p ak820-studio -p ak820-cli

sudo install -Dm755 target/release/ak820-studio /usr/local/bin/
sudo install -Dm755 target/release/ak820ctl /usr/local/bin/
sudo install -Dm644 udev/70-ak820.rules /usr/lib/udev/rules.d/70-ak820.rules
sudo udevadm control --reload && sudo udevadm trigger
```

Unplug and replug the keyboard after installing the rule. Then:

```bash
ak820ctl info                 # interfaces, firmware, frame limit
ak820ctl upload cat.gif       # PNG / JPEG / WebP / GIF
ak820ctl time                 # push the clock once
./target/release/ak820-studio # the app
```

## Clock autosync

Two ways, pick one:

- **In the app** (default): ⚙ Settings → sync on start. The clock is pushed when
  the app opens (cold start or restored from the tray), then every 12 hours while
  it runs.
- **With systemd**: the units below, for machines where the app is not running.

```bash
mkdir -p ~/.config/systemd/user
cp systemd/ak820-*.service systemd/ak820-clock.timer ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now ak820-clock.timer
systemctl --user enable ak820-resume.service
```

The timer pushes the clock every 60 s while the keyboard is connected (it exits silently when it
is not, and when `autosync_clock` is switched off in the app), `ak820-resume.service` covers suspend/resume, and the udev rule starts
`ak820-connect.service` on plug-in, which also re-uploads the newest history entry when
`restore_on_connect` is enabled in `~/.local/share/ak820/settings.json`.

## Layout

```
crates/ak820-proto    packet framing, animation payload, RGB565, device info (pure, unit-tested)
crates/ak820-device   sysfs interface discovery, hidraw I/O, retries, single-writer lock
crates/ak820-store    history on disk (same format as the Python CLI), settings
crates/ak820-cli      ak820ctl
apps/desktop          Tauri v2 app + vendored upstream UI (ui/ipc.js replaces WebHID)
systemd/ udev/        autosync units and device permissions
packaging/            RPM spec, .desktop, AppStream metadata
```

## Tests

```bash
cargo test --workspace                            # pure logic, no hardware needed
AK820_SELFTEST=1 cargo run -p ak820-studio        # UI smoke test: prints what loaded, then exits
AK820_SELFTEST=upload cargo run -p ak820-studio   # also performs a real 3-frame upload
AK820_SELFTEST=save cargo run -p ak820-studio     # saves a design to history, then deletes it
AK820_SELFTEST=settings cargo run -p ak820-studio # settings round-trip + autostart file
AK820_SELFTEST=update cargo run -p ak820-studio   # real GitHub update check (needs network)
```

## Credits

Upstream protocol research, web UI, generators and effects: **fromagge/ak820-screen** (MIT).
This project vendors that UI and reimplements the transport, storage, autosync and packaging in
Rust. See [NOTICE](NOTICE) and [docs/UPSTREAM.md](docs/UPSTREAM.md).
