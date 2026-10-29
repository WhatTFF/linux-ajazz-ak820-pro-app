# Contributing to AK820 Studio

Thanks for looking. AK820 Studio is a one-person project, so the most useful
things you can do are, in order: **report a bug well**, **tell me what broke on
your keyboard**, and **send a small, focused pull request**.

This file is short on ceremony and long on the one thing that actually bites
contributors: the hardware. Read that section before touching the transport.

The project is on hold while a native GTK version is built at
[WhatTFF/linux-ajazz-ak820-pro-gtk](https://github.com/WhatTFF/linux-ajazz-ak820-pro-gtk).
Bug fixes, packaging fixes and security fixes are still welcome. Large new
features will probably be redirected there.

---

## Before you write code

Open an issue first, or comment on an existing one. Not to gatekeep — to avoid
you spending an evening on something that is already planned, already tried, or
deliberately not going to happen (see *What will not be merged* below).

Small fixes — a typo, a crash, an obviously wrong string — can go straight to a
pull request without an issue.

## What will not be merged

The README makes promises, and code that breaks them gets closed regardless of
quality:

- Accounts, sign-up, or any kind of login.
- Cloud sync, backup services, or a server of any kind.
- Analytics, telemetry, crash reporting SDKs, or "anonymous" usage stats.
- Advertising, tracking pixels, or anything from a data broker.
- Anything that uploads designs, history, settings, or device identifiers off
  the machine.
- Anything that runs the app, the package scriptlets, or the udev rule as root,
  or that widens the hidraw rule beyond `uaccess` for the logged-in user.
- Bundling the Windows driver, closed vendor binaries, or firmware images.
- A third network request. There are exactly two today: the GitHub release
  check, and the optional GIPHY search that only runs with a key you paste in
  yourself. A change that adds another one is a different app.

## Setting up

```bash
git clone https://github.com/WhatTFF/linux-ajazz-ak820-pro-app.git
cd linux-ajazz-ak820-pro-app
cargo build --release -p ak820-studio -p ak820-cli
```

Fedora build dependencies:

```bash
sudo dnf install gtk3-devel webkit2gtk4.1-devel libsoup3-devel librsvg2-devel \
                 openssl-devel libappindicator-gtk3-devel \
                 cargo rust gcc gcc-c++ pkgconf-pkg-config
```

Debian/Ubuntu equivalents: `libgtk-3-dev`, `libwebkit2gtk-4.1-dev`,
`libsoup-3.0-dev`, `librsvg2-dev`, `libssl-dev`, `libayatana-appindicator3-dev`.

The workspace targets **Rust 1.85+**, edition 2024. CI builds and tests in a
`fedora:45` container.

The version number lives in `[workspace.package] version` in the root
`Cargo.toml` and is mirrored into `apps/desktop/tauri.conf.json` and
`packaging/ak820-studio.spec`. Never bump any of them by hand in a pull
request; see below.

## Checks to run before opening a pull request

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

All three must be clean. CI additionally builds the release binaries and runs
the webview under `xvfb` with `AK820_SELFTEST=1`; that self-test drives the UI
against a fake backend and needs no keyboard attached.

## The rule that matters: the wire format needs a real keyboard

`crates/ak820-device` has no test harness that can stand in for the device.
`cargo test` does not exercise report IDs, chunk sizes, timing, or the
device's own quirks, and a diff that looks entirely reasonable can produce a
payload the keyboard ignores, a black screen, or a keyboard that has to be
unplugged and plugged back in.

So, if your change touches the transport, report sizing, or payload packing:

1. Try it on a real AK820 Pro. Not "it compiles", not the protocol simulator.
2. Put the keyboard in **wired** mode on the USB-C cable for anything that
   reaches the screen. The 2.4 GHz dongle can do `GET_DEVICE_INFO` and clock
   sync, but has no screen interface.
3. Record what the device reports, so the report can be reproduced:

   ```bash
   cargo run -p ak820-cli -- info
   cargo run -p ak820-cli -- raw ...   # if that is what you are debugging
   ```

   `raw` is a debugging tool with no guard rails. Say which command bytes and
   payload size you sent.
4. For anything automatic — autosync, plug-in detection, resume — watch the
   logs, not just the end state:

   ```bash
   journalctl --user -u ak820-clock -b --no-pager
   systemctl --user status ak820-clock.timer ak820-connect.service ak820-resume.service
   ```

5. Say in the pull request what you did and what you saw. "AK820 Pro, wired,
   firmware 1.07, `info` reports workMode 1, screen updated and clock matched
   after replug" is enough.

Byte layouts were validated against the upstream JavaScript
([fromagge/ak820-screen](https://github.com/fromagge/ak820-screen)), so if you
change packing, include the before/after bytes in the pull request. For UI
changes, `AK820_SELFTEST=1` plus a screenshot or short recording is expected.

## Code style

- Follow `rustfmt` and `clippy`. They are the style guide; CI enforces both
  with `-D warnings`.
- Match the surrounding code. Comments explain *why*, not *what*.
- No new dependencies without discussing it in an issue first. Every crate is
  something the app has to keep building on someone else's distro, and a
  WebKit/GTK bump breaks packagers more often than it breaks us.
- Do not reformat files you are not changing. A pull request that is 90%
  whitespace will be sent back.

## Commits and pull requests

- Keep a pull request to one thing. Two unrelated fixes in one branch will be
  asked to split.
- Commit subjects are short and prefixed, matching the existing history:
  `fix:`, `chore:`, `docs:`, `release:`. The body explains why.
- **Do not bump the version.** The version, the tag, the spec changelog, the
  README badges, and the release notes are one decision made at release time
  by the maintainer. A pull request that bumps `Cargo.toml` will be asked to
  drop that commit.
- **Do not update the README for a release.** Same reason. Do update it if your
  change makes something the README says untrue.
- For anything visible in the UI, include a screenshot or a short recording.
- Expect review to take a while. It is one person, and anything touching the
  transport gets read slowly on purpose.

## Security

Do not open a public issue for a security problem. Use the private advisory
form linked in [SECURITY.md](SECURITY.md).

## Licence of your contribution

AK820 Studio is licensed under the **GNU General Public License, version 3.0
only** (see [LICENSE](LICENSE)). There is no CLA to sign and no copyright
assignment.

By opening a pull request you confirm that you wrote the change (or have the
right to submit it) and that you license it to this project under the same
GPL-3.0-only terms. If your employer owns your work, make sure they are fine
with that before you submit.

Some of the tree came from MIT-licensed upstream code (see [NOTICE](NOTICE)).
Do not paste code from a source whose licence is incompatible with the GPL; if
you copy something, say where it came from and under what licence.

## Code of conduct

Participation is covered by [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md). The short
version: be decent, no contempt for people who want their hardware to work on
Linux, no reckless hardware advice.
