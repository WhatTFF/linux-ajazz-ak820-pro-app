# AK820 Studio

Native Linux app for the **AJAZZ / Epomaker AK820 Pro** keyboard screen — a Rust rewrite of
[fromagge/ak820-screen](https://github.com/fromagge/ak820-screen) with automatic clock sync.

Put GIFs, images, video, text, drawings and procedural animations on the 0.85" 128×128 screen,
then forget about it: the app keeps the keyboard clock in step with your system clock on boot,
on plug-in, after suspend and after timezone changes.

> Unofficial. Not affiliated with AJAZZ or Epomaker. Protocol recovered by the upstream project
> from the vendor's own WebHID driver — see [PROTOCOL.md](PROTOCOL.md).

## Status

**Planning stage.** The architecture, porting strategy, autosync design, packaging and milestones
live in [PLAN.md](PLAN.md). Nothing is implemented yet.

## Target

- Fedora 45 Workstation (GNOME / Wayland), RPM via COPR
- Rust + Tauri v2, WebKitGTK frontend reusing the upstream UI under MIT
- Direct `hidraw` access, no browser, no WebHID permission chooser
- USB-C cable only — the 2.4 GHz dongle and Bluetooth cannot reach the screen

## Planned layout

```
crates/ak820-proto    packet framing, animation payload, RGB565, device info (pure, unit-tested)
crates/ak820-device   sysfs interface discovery, hidraw I/O, retries, single-writer lock
crates/ak820-store    SQLite history + settings, same on-disk format as the Python CLI
crates/ak820-cli      ak820ctl: info / upload / time / builtin / history
apps/desktop          Tauri v2 app + vendored upstream UI
systemd/ udev/        autosync service + timer, device permissions
packaging/            RPM spec, .desktop, AppStream metadata, icons
```

## Permissions (once)

```bash
sudo cp udev/70-ak820.rules /usr/lib/udev/rules.d/
sudo udevadm control --reload && sudo udevadm trigger
```

Then unplug and replug the keyboard.

## Credits

Upstream protocol research, web UI, generators and effects: **fromagge/ak820-screen** (MIT).
This project vendors that UI and reimplements the transport, storage, autosync and packaging in
Rust. See [NOTICE](NOTICE).
