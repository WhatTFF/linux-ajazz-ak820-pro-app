---
name: Bug Report
about: Report a bug to help improve AK820 Studio
title: ''
labels: bug
assignees: ''
---

**Describe the bug**
A clear and concise description of what the bug is.

**To Reproduce**
Steps to reproduce the behavior:
1. Open AK820 Studio
2. Click on '...'
3. See error

**Expected behavior**
A clear and concise description of what you expected to happen.

**Screenshots / Recordings**
If applicable, add media to help explain your problem.

**Environment (please complete the following):**
- Distribution and version: [e.g. Fedora 43, Ubuntu 24.04]
- Desktop / compositor: [e.g. GNOME 49 Wayland, KDE Plasma 6 X11]
- Install format: [RPM / DEB / AppImage / built from source]
- App version: [e.g. 1.0.1, from Settings or `ak820ctl version`]
- Rust version, if built from source: [e.g. 1.90.0]

**Keyboard (please complete the following):**
- Connection: [wired USB-C / 2.4 GHz dongle]
- Firmware version: [output of `ak820ctl info`]
- udev rule installed: [yes / no / not sure]

**Logs**
Paste anything relevant from:

```bash
journalctl --user -u ak820-clock -b --no-pager
```

Or run the app from a terminal so Rust-side errors show up, and include that
output. If the problem is the screen, say what it shows (black, previous
design, garbage) and whether replugging the keyboard fixes it.

**Additional context**
Add any other context about the problem here.
