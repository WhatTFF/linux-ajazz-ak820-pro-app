# Security Policy

## Reporting a vulnerability

**Do not open a public issue for a security problem.**

Use GitHub's private advisory form:

> <https://github.com/WhatTFF/linux-ajazz-ak820-pro-app/security/advisories/new>

That creates a private thread only you and the maintainer can see, and it lets
a fix be prepared and released before the details are public.

If you cannot use GitHub, email **emin.by@outlook.com** with
`AK820 Studio security` in the subject.

Please include what you can: the affected version (Settings shows it, or run
`ak820ctl version`), your distribution and desktop, how the app was installed
(RPM, DEB, AppImage, or from source), the keyboard connection (wired USB-C or
the 2.4 GHz dongle), what you did, and what happened. A proof of concept helps
but is not required.

## What to expect

This is a one-person project, so these are intentions rather than a service
level:

- Acknowledgement within about 7 days.
- An assessment and a rough timeline within about 30 days.
- Credit in the release notes and the advisory, unless you ask not to be named.
- Please give 90 days before publishing details, or less if a fix ships sooner.

## In scope

The interesting attack surface is the part that talks to hardware, installs
packages, or sits behind the webview:

- **The update path** — `update_download`, `update_install`,
  `update_replace_appimage`, and `uninstall_app` in
  `apps/desktop/src/lib.rs`. A compromised update is the worst case here: RPM
  and DEB installs are handed to `pkexec`, so they run as root. Anything that
  lets a URL, release asset name, or downloaded file be swapped for something
  else is a finding.
- **The HID transport** — `crates/ak820-device` and `ak820ctl raw`. Report
  sizing, chunking, retries, lock handling, and anything that lets another
  local process make the app send attacker-chosen bytes to the keyboard.
- **The udev rule** — `udev/70-ak820.rules`. It must stay scoped to the
  logged-in user's `uaccess` session and to the two wired PIDs plus the dongle.
  A change that makes hidraw world-writable, or that runs arbitrary commands
  on plug-in, is a finding.
- **The Tauri bridge** — `withGlobalTauri`, the `core:default` capability set,
  the disabled CSP, and the vendored UI under `apps/desktop/ui/`. The webview
  can invoke `upload`, `history_*`, `settings_*`, and the update and uninstall
  commands. Anything that lets page content, a dragged-in file, or a design
  name reach those commands with unexpected arguments is a finding.
- **Local storage** — `~/.local/share/ak820/settings.json` and the `history`
  directory. Anything that lets another local process tamper with settings or
  designs in a way the app then acts on.
- **Import handling** — paths passed to the image, GIF, and video importers,
  and anything that writes outside the download or data directory.
- **Packaging** — `packaging/linux/postinst.sh`, the spec file, and the
  desktop entry. Anything the package does as root at install time.

## Out of scope

- Anything that requires a rooted machine, an unlocked bootloader, or `sudo`
  that the attacker already has.
- Physical access to the keyboard, or a malicious USB device plugged into the
  machine.
- Firmware flashing or raw commands the user deliberately runs through
  `ak820ctl raw`. That tool is an escape hatch with no guard rails.
- The keyboard's own firmware, the official Windows software, or the Sonix
  protocol itself.
- Content returned by GIPHY. It is a third-party service, only queried when you
  supply your own API key, and its results are treated as untrusted images.
- Missing hardening that has no demonstrated impact.
- Reports produced only by an automated scanner, with no working path to
  abuse.

## Supported versions

Only the newest published release gets fixes. There are no backports to older
versions — the update path exists so people can move forward.
