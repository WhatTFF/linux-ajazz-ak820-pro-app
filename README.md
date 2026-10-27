# AK820 Studio

Native Linux app for the **AJAZZ / Epomaker AK820 Pro** keyboard screen — a Rust rewrite of
[fromagge/ak820-screen](https://github.com/fromagge/ak820-screen) with automatic clock sync.

Put GIFs, images, video, text, drawings and procedural animations on the 0.85" 128×128 screen,
then forget about it: the app keeps the keyboard clock in step with your system clock on boot,
on plug-in, after suspend and after timezone changes.

> Unofficial. Not affiliated with AJAZZ or Epomaker. Protocol recovered by the upstream project
> from the vendor's own WebHID driver — see [PROTOCOL.md](PROTOCOL.md).

## Status

Working on hardware (fw 1.07, PID `0c45:8009`):

- `ak820ctl` — `info`, `time`, `upload`, `builtin`, `history list|export|restore|clear`, `raw`
- `ak820-studio` — Tauri window with the full upstream UI (animations, image/GIF/video, text,
  draw, effects, adjust), native device access, on-disk history
- clock autosync units (`systemd/`), udev rule, RPM spec, CI

Not done yet: video import fallback for missing codecs, GIPHY key in the keyring, tray icon,
SQLite-backed history, Flatpak, release automation. See [PLAN.md](PLAN.md).

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

```bash
mkdir -p ~/.config/systemd/user
cp systemd/ak820-*.service systemd/ak820-clock.timer ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now ak820-clock.timer
systemctl --user enable ak820-resume.service
```

The timer pushes the clock every 60 s while the keyboard is connected (it exits silently when it
is not), `ak820-resume.service` covers suspend/resume, and the udev rule starts
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
cargo test --workspace                       # pure logic, no hardware needed
AK820_SELFTEST=1 cargo run -p ak820-studio   # UI smoke test: prints what loaded, then exits
AK820_SELFTEST=upload cargo run -p ak820-studio  # also performs a real 3-frame upload
```

## Credits

Upstream protocol research, web UI, generators and effects: **fromagge/ak820-screen** (MIT).
This project vendors that UI and reimplements the transport, storage, autosync and packaging in
Rust. See [NOTICE](NOTICE) and [docs/UPSTREAM.md](docs/UPSTREAM.md).
