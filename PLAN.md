# AK820 Studio — native Linux app for the AJAZZ / Epomaker AK820 Pro screen

Status: plan v1 · target Fedora 45 (GNOME/Wayland) · Rust + Tauri v2

Upstream reference: <https://github.com/fromagge/ak820-screen> @ `9030b81` (MIT).
Web app: <https://fromagge.github.io/ak820-screen/>.

---

## 1. Goal

A native Linux desktop app that is a **UI and feature copy** of the upstream web app, but:

- runs as a real window (no browser, no WebHID, no permission chooser),
- talks to the keyboard directly from Rust over `hidraw`,
- **keeps the screen clock in sync automatically** (the headline feature),
- keeps upload history on disk, restores the last image when the keyboard is replugged,
- ships as an RPM (COPR) with `.desktop`, AppStream metadata, udev rule and systemd user units.

Out of scope for v1: Windows/macOS builds, OTA firmware, key remapping, RGB control, Flatpak.

---

## 2. What upstream actually is (facts, verified on this machine)

| Piece | Size | Notes |
|---|---|---|
| `web/index.html` | 396 lines | full UI + CSS, light/dark theme, device mock |
| `web/app.js` | 740 lines | render pipeline, tabs, effects wiring, history, GIPHY |
| `web/gen.js` | 252 lines | 10 procedural generators + seeded PRNG |
| `web/fx.js` | 193 lines | text fitting, rainbow, ~20 effects |
| `web/ak820.js` | 169 lines | WebHID transport (the only part that must be rewritten) |
| `ak820.py` | 444 lines | CLI: info/upload/time/builtin/stats/history/raw |

All of the pixel work happens in `<canvas>` 2D. None of it depends on browser-only APIs except
`createImageBitmap`, `<video>`, IndexedDB, `localStorage` and `navigator.hid`.

### Device as measured here (fw 1.07, PID `0c45:8009`)

```
config iface  /dev/hidraw5   64 B output report   usage page 0xFF68
bulk   iface  /dev/hidraw6 4104 B output report   usage page 0xFF67
firmware 1.07 · workMode 0 · battery 100% · charging · tftMaxFrames 255 · frameVersion 0
```

Report descriptor of the bulk interface:

```
06 67 ff 09 61 a1 01 09 62 15 00 26 ff 00 95 40 75 08 b1 02   <- feature 64 B
09 63 15 00 26 ff 00 96 08 10 75 08 91 02                     <- output 0x1008 bits = 4104 B
```

Two consequences the port must respect:

1. **Never hard-code report sizes.** The config interface is 64 B here but the upstream JS falls
   back to 32 B, and older/newer firmware differs. Parse
   `/sys/class/hidraw/hidrawN/device/report_descriptor` (readable as the logged-in user) exactly
   like `ak820.py:parse_descriptor()` and fall back to 32/4104.
2. **`tftMaxFrames` is 255 here**, not the 140 the web UI assumes by default. The UI must take the
   limit from `GET_DEVICE_INFO` (`min(maxFrames, 255)`).

Wire format (full detail in `PROTOCOL.md`):

- host→device `AA cmd len addr_lo addr_hi 00 last 00 <payload>`, device replies `55 cmd …`
- animation upload `0x50` on bulk, custom header `AA 50 idx_lo idx_hi total_lo total_hi 50 06`,
  reply `55 41`, ≤2 s timeout, 3 retries
- payload = 256 B header (`[0]=N`, `[1..N-1]=delay/2 ms`, `[N]=0`, rest `0xFF`) + N × 128×128 RGB565 LE
- clock `0x34` on config: `5A 01 5A YY MM DD hh mm ss wd` padded to 24 B
- built-in animation `0x51` on bulk, 1 byte index, no reply
- firmware redraw floor ≈60 ms/frame; persistence of `0x50` uploads is unreliable (fw 1.16 loses
  them on unplug). Needs re-testing on 1.07.

---

## 3. Architecture decision

**Tauri v2**: Rust backend + the upstream web UI vendored as the frontend.

Why:

- The UI is 1 500 lines of canvas code that would take weeks to reproduce faithfully in `iced` or
  GTK4. Reusing it verbatim gives a pixel-accurate copy on day one and keeps upstream fixes
  cherry-pickable.
- The one thing a webview cannot do — WebHID — is exactly what Rust does well.
- Everything users care about being "native" (device access, tray, autostart, daemon, packaging,
  notifications) is Rust-side.

Trade-offs accepted: WebKitGTK runtime dependency (already present on Fedora Workstation),
video import depends on GStreamer codecs, window chrome is GTK not adwaita-styled.

Escape hatch: all protocol/rendering-independent logic lives in `ak820-*` crates, so a later
`iced`/GTK frontend can reuse the core with zero rewrites.

### Workspace layout

```
.
├── Cargo.toml                    # workspace
├── crates/
│   ├── ak820-proto/              # pure: packet framing, payload build/parse, RGB565, device info
│   ├── ak820-device/             # sysfs discovery, hidraw open, transact/retry, single-writer lock
│   ├── ak820-store/              # SQLite (rusqlite) history + settings, payload blobs on disk
│   └── ak820-cli/                # `ak820ctl` — parity with ak820.py (info/upload/time/builtin/...)
├── apps/desktop/
│   ├── src/                      # Tauri commands, tray, autostart, IPC
│   ├── ui/                       # vendored upstream UI (MIT, see NOTICE)
│   └── tauri.conf.json
├── systemd/                      # ak820-clock.service, .timer, sleep hook
├── udev/70-ak820.rules
├── packaging/                    # rpm spec, .desktop, AppStream metainfo, icons
└── docs/                         # UI-PARITY.md, ARCHITECTURE.md, UPSTREAM.md
```

`ak820-proto` has no I/O and no async: it is unit-testable against golden vectors captured from
the upstream JS (`buildPayload`/`parsePayload`) and from real traffic.

---

## 4. Porting the UI (minimal-diff strategy)

Vendor `web/` as `apps/desktop/ui/` untouched, then change exactly two files:

1. **`ak820.js` → `ipc.js`** — same exported surface, same `Keyboard` class shape, backed by
   `invoke()`:

   ```js
   export class Keyboard {
     static async connect()            // -> { firmware, battery, charging, maxFrames, ... }
     async info()                      // same shape as upstream
     async syncClock()                 // tauri: sync_clock
     async showBuiltIn(index)          // tauri: show_builtin
     async upload(frames, {onProgress, signal})  // sends buildPayload(frames) as raw bytes
   }
   export function buildPayload(frames)  // unchanged, byte-identical to upstream
   export function parsePayload(payload) // unchanged
   ```

   `buildPayload` stays in JS: it is 15 lines, byte-exact, and avoids 8 MB of per-frame IPC.
   The frontend sends one packed `Uint8Array` (max 8.4 MB) over a Tauri **raw** request body; Rust
   validates the header, chunks it, and streams progress back on a `Channel`.

2. **`app.js`** — patch only: connection bootstrap (`connect` button, hotplug events), history
   calls (`saveHistory`/`renderHistory`/`loadHistory` → SQLite commands), GIPHY key storage
   (`localStorage` → `settings` command), file open/drag-drop (`<input type=file>` still works in
   WebKitGTK; add Tauri drag-drop for a native feel).

Everything else — generators, effects, text fitting, drawing tab, adjust sliders, trim/speed,
history thumbnails — stays byte-identical. `docs/UPSTREAM.md` records the diff so upstream
commits can be merged mechanically.

### Tauri command surface

```rust
connect() -> Result<DeviceInfo>          // enumerate + open, idempotent
info() -> Result<DeviceInfo>
sync_clock() -> Result<()>
show_builtin(index: u8) -> Result<()>
upload(payload: RawBytes, name: String, progress: Channel<UploadProgress>) -> Result<HistoryId>
cancel_upload()
history_list() -> Vec<HistoryEntry>      // id, name, when, frames, bytes, thumb
history_restore(id) -> Result<()>
history_export(id, path: PathBuf) -> Result<()>   // payload -> GIF via `gif` crate
history_clear()
settings_get() / settings_set()
giphy_search(q, limit) -> Result<Vec<Gif>>        // HTTP from Rust, key in keyring
```

### Feature parity matrix

| Upstream feature | Native implementation |
|---|---|
| Animations tab, 10 generators, Shuffle, Surprise me | unchanged JS (`gen.js`) |
| Image / GIF / APNG / WebP import | unchanged JS (`createImageBitmap` in WebKitGTK) |
| Video import (MP4/WebM), frame-rate sampling | `<video>` + GStreamer; fallback: Rust `gstreamer-rs` decode if codec missing |
| Text tab (static/scroll/wave/typewriter/blink, fonts, outline) | unchanged JS |
| Draw tab (brush/eraser/rainbow, undo, trace last frame) | unchanged JS |
| Effects (20 chips + caption + hue/pixelate/loop) | unchanged JS |
| Adjust (fit/rotate/zoom/pan/mirror/bg/bg-anim/bri/con/sat/speed/trim/skip) | unchanged JS |
| Send + progress + cancel | Rust chunked upload over hidraw |
| Sync clock button | Rust `0x34` |
| Built-in animation picker | Rust `0x51` |
| History (IndexedDB, 30 entries, restore/export) | SQLite + blob files, 50 entries, PNG thumbs |
| GIPHY search | Rust HTTP + keyring; no browser CORS issues |
| Keyboard pill (firmware/battery/frames) | same, from `GET_DEVICE_INFO` |
| Help / udev instructions | rewritten for the packaged app |

### Storage

```
~/.local/share/ak820/
├── history.db          # SQLite WAL: id, ts, name, frames, bytes, thumb, payload_path
├── history/<ts>_<slug>.bin   # raw 0x50 payload, identical to ak820.py format
└── settings.json       # or the same DB, table `settings`
```

Same on-disk format as the Python CLI, so `ak820.py history` and `ak820ctl` interoperate.

---

## 5. Automatic clock sync (the reason for the app)

The screen has no usable RTC; the vendor driver pushes the clock. Design:

```
ak820-clock.service      oneshot, ExecStart=ak820ctl time --quiet
ak820-clock.timer        OnBootSec=10s, OnUnitActiveSec=60s, AccuracySec=1s, Persistent=false
```

Triggers, in order of importance:

1. **Boot** — timer fires 10 s after login.
2. **Device plugged in / switched to wired** — udev rule:
   `TAG+="systemd", ENV{SYSTEMD_USER_WANTS}+="ak820-clock.service"`.
3. **Resume from suspend** — `WantedBy=sleep.target` + `After=suspend.target` oneshot.
4. **Every 60 s while connected** — daemon compares local minute with the last value it pushed and
   only writes when it changed (one 24-byte report, ~1 ms; negligible).
5. **Timezone / DST / manual clock change** — listen to `org.freedesktop.timedate1`
   `PropertiesChanged` (and `SetTime`), push immediately.

Guards:

- no device → exit 0 silently (wireless/dongle mode must not spam logs);
- `flock` on `$XDG_RUNTIME_DIR/ak820.lock` so the GUI and the daemon never interleave writes;
- never send `04 72` (destroys the saved animation on 1.16);
- opt-out toggle in Settings; "sync on connect" and "restore last image on connect" separate.

Open question to answer in M1 (needs the physical keyboard): **does the clock survive unplug on
fw 1.07?** If yes, sync only on change; if no, resync on every connect. Also measure 24 h drift to
justify the 60 s timer vs 15 min.

---

## 6. Packaging & distro integration

- **RPM** via COPR (`packaging/ak820-studio.spec`), `BuildRequires: webkit2gtk4.1-devel,
  libsoup3-devel, librsvg2-devel, openssl-devel`, `Requires: webkit2gtk4.1`.
- `.desktop` with `StartupWMClass=ak820-studio`, AppStream `io.github.whattff.Ak820Studio.metainfo.xml`,
  hicolor icons, GPL-compatible MIT license field.
- `udev/70-ak820.rules` shipped but **not auto-installed** by RPM: `%post` prints the one-liner
  (`cp` + `udevadm control --reload`) instead of silently touching `/etc/udev`. Alternative:
  package as `/usr/lib/udev/rules.d/70-ak820.rules` — allowed and cleaner; do that.
- systemd **user** units go to `%{_userunitdir}`, enabled by `systemctl --user enable --now`
  (documented, not forced).
- Tray icon (optional, `libayatana-appindicator-gtk3`) with: show window, sync now, quit.
- Autostart: XDG `.desktop` in `/etc/xdg/autostart` for the tray-less case, or `tauri-plugin-autostart`.
- Flatpak deferred: hidraw inside the sandbox needs `--device=all` and udev inside the runtime;
  do it only after the RPM works.

CI (GitHub Actions): `cargo fmt --check`, `clippy -D warnings`, `cargo test` (proto/store),
`cargo tauri build` inside a `fedora:45` container, artifacts uploaded; release job attaches the
`.rpm`, `.tar.gz` and a source tarball.

---

## 7. Milestones

| # | Milestone | Deliverable | Acceptance | Est. |
|---|---|---|---|---|
| M0 | Repo, license, docs, udev, plan | this repo | docs reviewed, udev rule verified on fw 1.07 | done |
| M1 | `ak820-proto` + `ak820-device` + `ak820ctl info/upload/time` | CLI works against the real keyboard | `ak820ctl info` matches `ak820.py info`; upload of a GIF renders; golden-vector tests for `buildPayload` | 1–2 d |
| M2 | Tauri shell + UI vendored + `ipc.js` | window opens, connects, syncs clock, sends an animation | pixel-identical UI at 1040 px; upload progress works; hotplug detected | 2–3 d |
| M3 | Full render parity | all tabs, effects, adjust, trim/speed, video | side-by-side screenshot diff vs the website < 1 % pixels | 4–6 d |
| M4 | History + settings | SQLite history, restore/export, GIPHY, keyring | restore after replug; export yields a playable GIF | 1–2 d |
| M5 | Autosync daemon + packaging | systemd units, udev, RPM, AppStream, COPR | clock correct after boot/plug/resume/DST; `dnf install` on clean Fedora 45 | 3–4 d |
| M6 | Polish | tray, autostart, single-instance, error states, a11y, i18n-ready strings | no crash with keyboard absent; app usable with keyboard unplugged | 2 d |
| M7 | CI + release v0.1.0 | GH Actions, signed tag, release artifacts | green CI, downloadable RPM | 1 d |

Total ≈ 3 weeks part-time. M1 is the risk spike: it proves the protocol on fw 1.07 before any UI
work starts.

---

## 8. Risks

| Risk | Impact | Mitigation |
|---|---|---|
| WebKitGTK video codecs missing for MP4/H.264 | video tab broken for some files | probe with `canPlayType`, fall back to `gstreamer-rs` decode in Rust; document codec RPMs |
| Tauri raw-body IPC of 8.4 MB payloads | slow sends / memory spikes | send as raw `ArrayBuffer` (not JSON), stream chunks, reuse buffer; benchmark early |
| fw 1.07 differs from 1.16 (frames, persistence, clock) | wrong assumptions | M1 measures on hardware; keep limits from `GET_DEVICE_INFO` |
| Keyboard holds upload in RAM only | image lost on unplug | optional auto-restore of last history entry on connect |
| Concurrent writers (GUI + daemon + vendor driver) | corrupted uploads | `flock` + 2 s timeout + "device busy" message |
| WebKitGTK canvas perf on 4K/HiDPI | sluggish preview | fixed 128×128 pipeline + `image-rendering: pixelated`; profile in M3 |
| Upstream moves (UI drift) | merge pain | vendored copy + `docs/UPSTREAM.md` diff notes; sync by hand each release |
| Udev permission errors for non-admin users | "can't open keyboard" | ship rule in `/usr/lib/udev/rules.d`, detect `EACCES` and show exact fix command |
| Trademark / affiliation | legal | README: unofficial, not affiliated; keep upstream MIT notice |

---

## 9. Testing strategy

- **Unit**: framing (chunk boundaries, `last` flag, addr math), payload header, RGB565 round-trip,
  clock encoding across month/year edges and DST, `parsePayload` fuzz (never panics).
- **Golden vectors**: capture `buildPayload` output for a fixed 3-frame input in Node, commit as
  `tests/fixtures/*.bin`, assert byte equality from Rust.
- **Hardware-in-the-loop**: `AK820_DEVICE=1 cargo test --test hardware` — info, clock, 1-frame
  upload, builtin. Skipped in CI.
- **UI parity**: Playwright screenshot of the website vs Tauri window at the same size, pixel diff
  in CI (threshold 1 %).
- **Daemon**: fake clock + fake device trait, assert push-once-per-minute and no-op when absent.

---

## 10. Open decisions (need a call before M3/M5)

1. Video fallback: bundle `gstreamer-rs` decode, or document required codec RPMs only?
2. Tray icon in v1, or GNOME has no tray by default anyway (extension) → skip?
3. GIPHY key: `keyring` (libsecret) vs plain config file (simpler, less secure)?
4. History limit: match web (30) or CLI (50)?
5. Repo stays a standalone app, or fork upstream and add `native/`? (recommended: standalone with
   vendored UI + clear attribution)
