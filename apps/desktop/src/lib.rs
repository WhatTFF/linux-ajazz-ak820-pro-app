//! Tauri backend: device access, uploads, history, settings and tray.
//!
//! All USB work happens here. The frontend is the vendored upstream UI, with its
//! WebHID layer replaced by the commands below (see `ui/ipc.js`).

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, UNIX_EPOCH};

use ak820_device::{Device, DeviceLock, Presence};
use ak820_proto::{CivilTime, DeviceInfo};
use ak820_store::history;
use ak820_store::settings::Settings;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::{Datelike, Timelike};
use serde::Serialize;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{Emitter, Manager, State, WindowEvent};

/// How often the background threads look at the world.
const POLL: Duration = Duration::from_secs(2);
/// Minimum sync interval the settings accept, to keep the USB bus quiet.
const MIN_SYNC_INTERVAL: u64 = 10;

/// Shared, cheap state: just the cancel flag.
#[derive(Default)]
pub struct AppState {
    cancel: Arc<AtomicBool>,
}

/// What the UI shows in the keyboard pill and device panel.
#[derive(Clone, Debug, Default, Serialize)]
pub struct DeviceStatus {
    /// Any AK820 device is attached (cable or dongle).
    pub connected: bool,
    /// `"usb"`, `"dongle"` or `"none"`.
    pub mode: String,
    /// The animation interfaces are there: sending is possible.
    pub can_upload: bool,
    /// The clock interface is there.
    pub can_sync: bool,
    pub name: String,
    pub firmware: String,
    pub battery: u8,
    pub charging: bool,
    pub profile: u8,
    pub max_frames: u16,
    pub has_screen: bool,
    pub has_config: bool,
    pub error: Option<String>,
}

/// One stored upload, with a PNG thumbnail as a data URL.
#[derive(Clone, Debug, Serialize)]
pub struct HistoryEntryDto {
    /// File name, used as the identifier by the UI.
    pub id: String,
    pub name: String,
    /// Milliseconds since the Unix epoch.
    pub date: i64,
    pub frames: u16,
    pub bytes: u64,
    pub thumb: String,
}

/// Settings as the UI sees them.
#[derive(Clone, Debug, Default, Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SettingsDto {
    pub autosync_clock: bool,
    pub restore_on_connect: bool,
    pub sync_on_connect: bool,
    pub giphy_key: String,
    pub start_at_login: bool,
    pub start_minimized: bool,
    pub minimize_to_tray: bool,
    pub sync_on_start: bool,
    pub quit_after_start_sync: bool,
    pub sync_interval_secs: u64,
    pub check_updates: bool,
}

impl From<Settings> for SettingsDto {
    fn from(value: Settings) -> Self {
        SettingsDto {
            autosync_clock: value.autosync_clock,
            restore_on_connect: value.restore_on_connect,
            sync_on_connect: value.sync_on_connect,
            giphy_key: value.giphy_key,
            start_at_login: value.start_at_login,
            start_minimized: value.start_minimized,
            minimize_to_tray: value.minimize_to_tray,
            sync_on_start: value.sync_on_start,
            quit_after_start_sync: value.quit_after_start_sync,
            sync_interval_secs: value.sync_interval_secs,
            check_updates: value.check_updates,
        }
    }
}

impl From<SettingsDto> for Settings {
    fn from(value: SettingsDto) -> Self {
        Settings {
            autosync_clock: value.autosync_clock,
            restore_on_connect: value.restore_on_connect,
            sync_on_connect: value.sync_on_connect,
            giphy_key: value.giphy_key,
            start_at_login: value.start_at_login,
            start_minimized: value.start_minimized,
            minimize_to_tray: value.minimize_to_tray,
            sync_on_start: value.sync_on_start,
            quit_after_start_sync: value.quit_after_start_sync,
            sync_interval_secs: value.sync_interval_secs,
            check_updates: value.check_updates,
        }
    }
}

// --- device -----------------------------------------------------------------------

/// Cheap snapshot: what is plugged in and what it can do.
fn snapshot() -> DeviceStatus {
    let interfaces = ak820_device::find();
    let presence = ak820_device::presence();
    // The dongle bridges the clock channel but has no screen interface, so the
    // presence of the bulk role is what separates "cable" from "dongle".
    let mode = if interfaces.bulk.is_some() {
        "usb"
    } else if presence.dongle {
        "dongle"
    } else if interfaces.config.is_some() || presence.wired {
        "usb"
    } else {
        "none"
    };
    DeviceStatus {
        connected: !presence.is_empty() || !interfaces.nodes.is_empty(),
        mode: mode.into(),
        can_upload: interfaces.bulk.is_some(),
        can_sync: interfaces.config.is_some(),
        has_config: interfaces.config.is_some(),
        has_screen: interfaces.bulk.is_some(),
        name: "AK820 Pro".into(),
        ..Default::default()
    }
}

/// Why a sync attempt failed.
enum SyncFail {
    /// Another AK820 tool (usually the UI reading device info) holds the lock.
    Busy,
    Other(String),
}

/// Push this computer's clock. Works on the cable, and on the 2.4 GHz dongle,
/// which bridges the config channel.
fn try_sync() -> Result<String, SyncFail> {
    let now = chrono::Local::now();
    let time = CivilTime {
        year: now.year() as u16,
        month: now.month() as u8,
        day: now.day() as u8,
        hour: now.hour() as u8,
        minute: now.minute() as u8,
        second: now.second() as u8,
        weekday: now.weekday().num_days_from_sunday() as u8,
    };
    let _lock = match DeviceLock::acquire() {
        Ok(lock) => lock,
        Err(ak820_device::Error::Locked(_)) => return Err(SyncFail::Busy),
        Err(e) => return Err(SyncFail::Other(e.to_string())),
    };
    let mut device = Device::open().map_err(|e| SyncFail::Other(e.to_string()))?;
    if !device.has_config() {
        return Err(SyncFail::Other(
            "no clock interface: replug the dongle, or use the USB cable.".into(),
        ));
    }
    device
        .sync_clock(time)
        .map_err(|e| SyncFail::Other(e.to_string()))?;
    Ok(now.format("%H:%M:%S").to_string())
}

/// One sync attempt, for the UI button and the tray menu.
fn sync_once() -> Result<String, String> {
    try_sync().map_err(|e| match e {
        SyncFail::Busy => "the keyboard is busy: another AK820 tool is talking to it".into(),
        SyncFail::Other(e) => e,
    })
}

/// Background sync: the UI polls device info, so losing the lock race is normal.
fn sync_background() -> Result<String, String> {
    let mut last = String::from("the keyboard stayed busy");
    for _ in 0..5 {
        match try_sync() {
            Ok(done) => return Ok(done),
            Err(SyncFail::Busy) => std::thread::sleep(Duration::from_millis(300)),
            Err(SyncFail::Other(e)) => return Err(e),
        }
        last = "the keyboard stayed busy".into();
    }
    Err(last)
}

/// Put the newest history entry back on the screen, best effort.
fn restore_latest() {
    if ak820_device::find().bulk.is_none() {
        return;
    }
    let Ok(payload) = history::load(1) else {
        return;
    };
    let Ok(_lock) = DeviceLock::acquire() else {
        return;
    };
    let Ok(mut device) = Device::open() else {
        return;
    };
    let _ = device.upload(&payload, &mut |_, _| {});
}

// --- commands ---------------------------------------------------------------------

/// Open the keyboard and read its info; errors are shown to the user.
#[tauri::command]
fn connect() -> Result<DeviceStatus, String> {
    let mut status = snapshot();
    if !status.connected {
        return Err(ak820_device::Error::NotFound.to_string());
    }
    if !status.can_sync {
        // Nothing but a bare dongle node: the UI still gets a useful status.
        status.error = Some(
            "The clock channel is not reachable. Try replugging the dongle or using the USB cable."
                .into(),
        );
        return Ok(status);
    }
    let _lock = DeviceLock::acquire().map_err(|e| e.to_string())?;
    let mut device = Device::open().map_err(|e| e.to_string())?;
    let has_config = device.has_config();
    let info = device.info().map_err(|e| e.to_string())?;
    Ok(status_from(&info, has_config, status))
}

/// Cheap poll used by the UI and the hotplug watcher; never fails.
#[tauri::command]
fn device_status() -> DeviceStatus {
    let mut status = snapshot();
    if !status.can_sync {
        return status;
    }
    let Ok(_lock) = DeviceLock::acquire() else {
        status.error = Some("busy: another AK820 tool is talking to the keyboard".into());
        return status;
    };
    if let Ok(mut device) = Device::open()
        && let Ok(info) = device.info()
    {
        status = status_from(&info, status.has_config, status);
    }
    status
}

/// Full device info (firmware, battery, frame limit).
#[tauri::command]
fn device_info() -> Result<DeviceStatus, String> {
    let mut status = snapshot();
    if !status.can_sync {
        status.error = Some("no clock interface: connect the USB cable".into());
        return Ok(status);
    }
    let _lock = DeviceLock::acquire().map_err(|e| e.to_string())?;
    let mut device = Device::open().map_err(|e| e.to_string())?;
    let has_config = device.has_config();
    let info = device.info().map_err(|e| e.to_string())?;
    Ok(status_from(&info, has_config, status))
}

/// Push this computer's clock to the screen.
#[tauri::command]
fn sync_clock() -> Result<String, String> {
    sync_once()
}

/// Select a firmware built-in animation.
#[tauri::command]
fn show_builtin(index: u8) -> Result<(), String> {
    if ak820_device::find().bulk.is_none() {
        return Err(
            "Built-in animations need the USB cable: switch the keyboard to wired mode.".into(),
        );
    }
    let _lock = DeviceLock::acquire().map_err(|e| e.to_string())?;
    let mut device = Device::open().map_err(|e| e.to_string())?;
    device.show_builtin(index).map_err(|e| e.to_string())
}

/// Upload a packed `0x50` payload and store it in history.
#[tauri::command]
async fn upload(
    payload: Vec<u8>,
    name: String,
    progress: tauri::ipc::Channel<u8>,
    state: State<'_, AppState>,
) -> Result<HistoryEntryDto, String> {
    if ak820_device::find().bulk.is_none() {
        return Err(
            "The screen is only reachable over the USB cable. Switch the keyboard to wired mode, \
             or save this design to history instead."
                .into(),
        );
    }
    validate_payload(&payload)?;
    let cancel = state.cancel.clone();
    cancel.store(false, Ordering::SeqCst);
    tauri::async_runtime::spawn_blocking(move || upload_blocking(payload, name, progress, cancel))
        .await
        .map_err(|e| e.to_string())?
}

/// Ask a running upload to stop after the current chunk.
#[tauri::command]
fn cancel_upload(state: State<'_, AppState>) {
    state.cancel.store(true, Ordering::SeqCst);
}

/// Store a payload without sending it. This is how designs get saved while the
/// keyboard is on the dongle, or not plugged in at all.
#[tauri::command]
fn history_save(payload: Vec<u8>, name: String) -> Result<HistoryEntryDto, String> {
    validate_payload(&payload)?;
    let path = history::save(&payload, &name)?;
    Ok(entry_dto(&path))
}

/// Every stored upload, newest first.
#[tauri::command]
fn history_list() -> Vec<HistoryEntryDto> {
    history::list()
        .iter()
        .map(|entry| entry_dto(&entry.path))
        .collect()
}

/// Raw payload of one stored upload (for "show me what was on the screen").
#[tauri::command]
fn history_load(id: String) -> Result<Vec<u8>, String> {
    let path = history_path(&id)?;
    std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Delete one stored upload.
#[tauri::command]
fn history_delete(id: String) -> Result<(), String> {
    let path = history_path(&id)?;
    std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Delete every stored upload.
#[tauri::command]
fn history_clear() -> Result<usize, String> {
    history::clear()
}

/// Read persisted settings.
#[tauri::command]
fn settings_get() -> SettingsDto {
    Settings::load().into()
}

/// Write persisted settings and apply the ones that touch the system.
#[tauri::command]
fn settings_set(settings: SettingsDto) -> Result<(), String> {
    let settings: Settings = settings.into();
    settings.save()?;
    set_autostart(settings.start_at_login)?;
    Ok(())
}

// --- links -------------------------------------------------------------------------

/// Only web pages may be handed to the desktop, and only well-formed ones: the
/// argument goes straight to a process spawn, so no control characters, spaces
/// or other schemes.
fn is_openable_url(url: &str) -> bool {
    let rest = match url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    {
        Some(rest) => rest,
        None => return false,
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    !host.is_empty()
        && rest.len() <= 2048
        && !url
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '"' || c == '\\')
}

/// Open a link in the user's browser instead of navigating this window.
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    if !is_openable_url(&url) {
        return Err(format!("refusing to open {url}"));
    }
    std::process::Command::new("xdg-open")
        .arg(&url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("xdg-open: {e}"))
}

// --- updates ----------------------------------------------------------------------

/// What the UI needs to know about this build.
#[derive(Clone, Debug, Serialize)]
pub struct AppInfo {
    pub version: String,
    /// `"appimage"`, `"rpm"`, `"deb"` or `"source"`.
    pub package: String,
    pub data_dir: String,
}

/// How this copy was installed, guessed from the running binary.
fn package_kind() -> String {
    if std::env::var_os("APPIMAGE").is_some() {
        return "appimage".into();
    }
    let Ok(exe) = std::env::current_exe() else {
        return "source".into();
    };
    let exe = exe.to_string_lossy().to_string();
    let owned = |cmd: &str, args: &[&str]| {
        std::process::Command::new(cmd)
            .args(args)
            .arg(&exe)
            .output()
            .is_ok_and(|out| out.status.success())
    };
    if owned("rpm", &["-qf"]) {
        "rpm".into()
    } else if owned("dpkg", &["-S"]) {
        "deb".into()
    } else {
        "source".into()
    }
}

/// Version, install kind and data directory.
#[tauri::command]
fn app_info() -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        package: package_kind(),
        data_dir: ak820_store::data_dir().display().to_string(),
    }
}

/// `~/Downloads`, or the data directory when there is none.
fn download_dir() -> PathBuf {
    std::env::var_os("XDG_DOWNLOAD_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Downloads")))
        .filter(|p| p.is_dir())
        .unwrap_or_else(ak820_store::data_dir)
}

fn sanitize_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '\0' => '_',
            c => c,
        })
        .collect();
    if cleaned.is_empty() {
        "ak820-studio".into()
    } else {
        cleaned
    }
}

fn curl(url: &str, out: &Path) -> Result<(), String> {
    if !url.starts_with("https://github.com/")
        && !url.starts_with("https://objects.githubusercontent.com/")
    {
        return Err(format!("refusing to download from {url}"));
    }
    let status = std::process::Command::new("curl")
        .args(["-fsSL", "--retry", "3", "--connect-timeout", "20", "-o"])
        .arg(out)
        .arg(url)
        .status()
        .map_err(|e| format!("curl: {e}"))?;
    if !status.success() {
        return Err(format!("download failed ({status})"));
    }
    Ok(())
}

/// The command a user can paste into a terminal instead of using the prompt.
fn install_command(kind: &str, path: &Path) -> String {
    let path = path.display();
    match kind {
        "deb" => format!("sudo apt install -y \"{path}\""),
        _ => format!("sudo dnf install -y \"{path}\""),
    }
}

/// A downloaded package, plus how to install it by hand.
#[derive(Clone, Debug, Serialize)]
pub struct DownloadedPackage {
    pub path: String,
    pub command: String,
}

/// Download a release package next to the user's other downloads.
#[tauri::command]
fn update_download(url: String, name: String) -> Result<DownloadedPackage, String> {
    let dir = download_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let target = dir.join(sanitize_file_name(&name));
    let tmp = target.with_extension("part");
    curl(&url, &tmp)?;
    std::fs::rename(&tmp, &target).map_err(|e| format!("{}: {e}", target.display()))?;
    Ok(DownloadedPackage {
        command: install_command(&package_kind(), &target),
        path: target.display().to_string(),
    })
}

/// Is this a package this build is allowed to hand to the system installer?
fn is_installable(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();
    if ext != "rpm" && ext != "deb" {
        return false;
    }
    // Only files we downloaded ourselves, so a stray command cannot install
    // something from anywhere else on disk.
    let (Ok(file), Ok(dir)) = (path.canonicalize(), download_dir().canonicalize()) else {
        return false;
    };
    file.starts_with(dir) && file.is_file()
}

/// Install a downloaded package through polkit, which shows the desktop's own
/// password prompt. The terminal command stays available in the UI either way.
#[tauri::command]
async fn update_install(path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || install_blocking(Path::new(&path)))
        .await
        .map_err(|e| e.to_string())?
}

fn install_blocking(path: &Path) -> Result<String, String> {
    if !is_installable(path) {
        return Err(format!("not a downloaded .rpm or .deb: {}", path.display()));
    }
    let kind = package_kind();
    let mut cmd = std::process::Command::new("pkexec");
    match kind.as_str() {
        "deb" => cmd.args(["apt-get", "install", "-y"]),
        _ => cmd.args(["dnf", "install", "-y"]),
    };
    let out = cmd.arg(path).output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            format!(
                "pkexec is not installed. Run: {}",
                install_command(&kind, path)
            )
        } else {
            format!("pkexec: {e}")
        }
    })?;
    if out.status.success() {
        Ok("Installed. Quit and start AK820 Studio again to use the new version.".into())
    } else if out.status.code() == Some(126) || out.status.code() == Some(127) {
        Err(format!(
            "The password prompt was dismissed, so nothing was installed. Run: {}",
            install_command(&kind, path)
        ))
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        let tail: String = err.lines().rev().take(4).collect::<Vec<_>>().join("\n");
        Err(format!(
            "The installer failed ({}). {}",
            out.status,
            tail.trim()
        ))
    }
}

/// Replace this AppImage in place. Linux lets us rename over a running binary;
/// the new file is used on the next start.
#[tauri::command]
fn update_replace_appimage(url: String) -> Result<(), String> {
    let current = std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .ok_or("this is not an AppImage build")?;
    let tmp = current.with_extension("AppImage.new");
    curl(&url, &tmp)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("{}: {e}", tmp.display()))?;
    }
    std::fs::rename(&tmp, &current).map_err(|e| format!("{}: {e}", current.display()))
}

/// Where the autostart entry lives.
fn autostart_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"));
    base.join("autostart").join("ak820-studio.desktop")
}

/// Create or remove `~/.config/autostart/ak820-studio.desktop`.
fn set_autostart(enabled: bool) -> Result<(), String> {
    set_autostart_in(&autostart_path(), enabled)
}

fn set_autostart_in(path: &Path, enabled: bool) -> Result<(), String> {
    let path = path.to_path_buf();
    if !enabled {
        match std::fs::remove_file(&path) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let entry = "\
[Desktop Entry]
Type=Application
Name=AK820 Studio
Comment=Keyboard screen studio and clock sync
Exec=ak820-studio --tray
Icon=io.github.whattff.Ak820Studio
Terminal=false
X-GNOME-Autostart-enabled=true
";
    std::fs::write(&path, entry).map_err(|e| format!("{}: {e}", path.display()))
}

// --- helpers ----------------------------------------------------------------------

fn status_from(info: &DeviceInfo, has_config: bool, mut status: DeviceStatus) -> DeviceStatus {
    // `mode`, `can_upload` and `can_sync` come from the snapshot: the dongle
    // answers `GET_DEVICE_INFO` too, so device info must not imply "cable".
    status.connected = true;
    status.name = "AK820 Pro".into();
    status.firmware = format!(
        "{}.{:02}",
        info.firmware_hundredths / 100,
        info.firmware_hundredths % 100
    );
    status.battery = info.battery;
    status.charging = info.charging;
    status.profile = info.profile;
    status.max_frames = info.max_frames;
    status.has_screen = info.has_screen();
    status.has_config = has_config;
    status
}

fn validate_payload(payload: &[u8]) -> Result<(), String> {
    let frames = *payload.first().ok_or("the payload is empty")? as usize;
    if frames == 0 {
        return Err("the payload has no frames".into());
    }
    let expected = ak820_proto::payload_len(frames);
    if payload.len() != expected {
        return Err(format!(
            "the payload is {} bytes, but {frames} frames need {expected}",
            payload.len()
        ));
    }
    Ok(())
}

fn upload_blocking(
    payload: Vec<u8>,
    name: String,
    progress: tauri::ipc::Channel<u8>,
    cancel: Arc<AtomicBool>,
) -> Result<HistoryEntryDto, String> {
    let _lock = DeviceLock::acquire().map_err(|e| e.to_string())?;
    let mut device = Device::open().map_err(|e| e.to_string())?;

    let mut last = u8::MAX;
    device
        .upload_with(&payload, &mut |done, total| {
            if cancel.load(Ordering::SeqCst) {
                return false;
            }
            let percent = ((done * 100) / total) as u8;
            if percent != last {
                last = percent;
                let _ = progress.send(percent);
            }
            true
        })
        .map_err(|e| e.to_string())?;
    let _ = progress.send(100);

    let path = history::save(&payload, &name)?;
    Ok(entry_dto(&path))
}

fn history_path(id: &str) -> Result<PathBuf, String> {
    if id.is_empty() || id.contains('/') || id.contains('\\') || id.contains("..") {
        return Err(format!("invalid history id {id:?}"));
    }
    let path = history::dir().join(id);
    if !path.is_file() {
        return Err(format!("no history entry {id}"));
    }
    Ok(path)
}

fn entry_dto(path: &Path) -> HistoryEntryDto {
    let payload = std::fs::read(path).unwrap_or_default();
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string();
    let stem = path
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let name = stem
        .split_once('_')
        .map(|(_, n)| n)
        .unwrap_or(stem)
        .to_string();
    let date = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    HistoryEntryDto {
        id: file_name,
        name,
        date,
        frames: payload.first().copied().unwrap_or(0) as u16,
        bytes: payload.len() as u64,
        thumb: thumb_data_url(&payload).unwrap_or_default(),
    }
}

/// First frame of a payload as a PNG data URL, for the history grid.
fn thumb_data_url(payload: &[u8]) -> Option<String> {
    let frames = ak820_proto::parse_payload(payload, 1).ok()?;
    let mut rgba = Vec::with_capacity(128 * 128 * 4);
    ak820_proto::to_rgba(&frames.first()?.pixels, &mut rgba);
    let image = image::RgbaImage::from_raw(128, 128, rgba)?;
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
        .ok()?;
    Some(format!("data:image/png;base64,{}", BASE64.encode(png)))
}

// --- background work ---------------------------------------------------------------

/// Watch USB hotplug: tell the UI, sync the clock, restore the last image.
fn watch_devices(app: tauri::AppHandle) {
    let mut last: Option<Presence> = None;
    loop {
        let now = ak820_device::presence();
        if last != Some(now) {
            let appeared = last.is_some_and(|p| p.is_empty()) && !now.is_empty();
            last = Some(now);
            let _ = app.emit("device-changed", !now.is_empty());

            if appeared {
                let settings = Settings::load();
                if settings.sync_on_connect {
                    match sync_background() {
                        Ok(_) => {
                            let _ = app.emit("clock-synced", ());
                        }
                        Err(e) => {
                            let _ = app.emit("clock-error", e);
                        }
                    }
                }
                if settings.restore_on_connect {
                    restore_latest();
                }
            }
        }
        std::thread::sleep(POLL);
    }
}

/// Re-sync every `sync_interval_secs` while the app runs (window or tray).
fn watch_interval(app: tauri::AppHandle) {
    let mut elapsed = 0u64;
    loop {
        std::thread::sleep(Duration::from_secs(1));
        let interval = Settings::load().sync_interval_secs;
        if interval < MIN_SYNC_INTERVAL {
            elapsed = 0;
            continue;
        }
        elapsed += 1;
        if elapsed < interval {
            continue;
        }
        elapsed = 0;
        if ak820_device::presence().is_empty() {
            continue;
        }
        match sync_background() {
            Ok(_) => {
                let _ = app.emit("clock-synced", ());
            }
            Err(e) => {
                let _ = app.emit("clock-error", e);
            }
        }
    }
}

/// Build the tray icon and its menu.
fn build_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open AK820 Studio", true, None::<&str>)?;
    let sync = MenuItem::with_id(app, "sync", "Sync clock now", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &sync, &quit])?;

    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip("AK820 Studio")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_window(app),
            "sync" => {
                let _ = sync_once();
                let _ = app.emit("clock-synced", ());
            }
            "quit" => app.exit(0),
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

fn show_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

// --- app --------------------------------------------------------------------------

/// Injected by `AK820_SELFTEST=1` to prove the UI loaded and can reach Rust.
const SELFTEST_JS: &str = r#"
(async () => {
  const text = (id) => document.getElementById(id)?.textContent?.trim() ?? null;
  const info = {
    title: document.title,
    tauri: !!window.__TAURI__,
    tabs: document.querySelectorAll('[role=tab]').length,
    generators: document.querySelectorAll('#gens button').length,
    hasPreview: !!document.getElementById('preview'),
    hasHistoryPanel: !!document.getElementById('hist'),
    historyItems: document.querySelectorAll('#hist .hitem').length,
    hasSettings: !!document.getElementById('settings'),
    bodyMode: document.body.dataset.mode ?? null,
    saveHandler: !!document.getElementById('savehist')?.onclick,
    linkGuard: document.documentElement.dataset.linkGuard === '1',
    // The updater must not offer a prerelease as newer than its own release.
    vercmp: (() => {
      try {
        return [['v0.1.2-beta', 'v0.1.2', -1], ['v0.1.2', 'v0.1.2-beta', 1],
                ['v0.1.2-beta', 'v0.1.1-beta', 1], ['v0.1.10-beta', 'v0.1.9', 1],
                ['v0.1.0-beta', 'v0.1.0-beta', 0]]
          .every(([a, b, want]) => Math.sign(window.__ak820.newer(a, b)) === want);
      } catch { return 'missing'; }
    })(),
    externalLinks: document.querySelectorAll('a[href^="https://"]').length,
    repoLinks: [...document.querySelectorAll('a[href^="https://github.com/"]')]
      .filter((a) => a.href.includes('WhatTFF/linux-ajazz-ak820-pro-app')).length,
    errors: window.__errs ?? [],
    updDialog: !!document.getElementById('upd'),
    updAuto: (await window.__TAURI__.core.invoke('settings_get')).check_updates === true,
    updButtons: ['updact', 'updcopy', 'updlater', 'updpage'].every((id) => !!document.getElementById(id)),
    pill: text('pilltext'),
    mode: document.getElementById('modebanner')?.hidden === false ? 'banner' : 'no-banner',
    msg: text('msg'),
  };
  window.__TAURI__.core.invoke('selftest_report', { report: JSON.stringify(info) })
    .catch((e) => window.__TAURI__.core.invoke('selftest_report', { report: 'invoke failed: ' + e }));
})();
"#;

/// Upload smoke test: renders three frames, sends them, then reports.
const SELFTEST_UPLOAD_JS: &str = r#"
(async () => {
  const report = (data) => window.__TAURI__.core.invoke('selftest_report', { report: JSON.stringify(data) });
  try {
    const { Keyboard, buildPayload } = window.__ak820;
    const kb = await Keyboard.reconnect();
    if (!kb) return report({ error: 'not connected' });
    const frames = [];
    for (let i = 0; i < 3; i++) {
      const d = new ImageData(128, 128);
      for (let p = 0; p < d.data.length; p += 4) {
        d.data[p] = i * 80; d.data[p + 1] = 255 - i * 80; d.data[p + 2] = 128; d.data[p + 3] = 255;
      }
      frames.push({ data: d, delay: 120 });
    }
    const expected = buildPayload(frames).length;
    let progress = 0;
    const payload = await kb.upload(frames, { name: 'selftest', onProgress: (p) => { progress = Math.round(p * 100); } });
    const hist = await window.__TAURI__.core.invoke('history_list');
    report({ ok: true, bytes: payload.length, expected, progress, newest: hist[0]?.name, count: hist.length });
  } catch (e) {
    report({ error: String(e) });
  }
})();
"#;

/// Save smoke test: store a design with no keyboard involved, then clean up.
const SELFTEST_SAVE_JS: &str = r#"
(async () => {
  const report = (data) => window.__TAURI__.core.invoke('selftest_report', { report: JSON.stringify(data) });
  try {
    const { buildPayload, saveHistory } = window.__ak820;
    const frames = [0, 1].map((i) => {
      const d = new ImageData(128, 128);
      for (let p = 0; p < d.data.length; p += 4) {
        d.data[p] = 40 + i * 60; d.data[p + 1] = 90; d.data[p + 2] = 200 - i * 60; d.data[p + 3] = 255;
      }
      return { data: d, delay: 100 };
    });
    const entry = await saveHistory(buildPayload(frames), 'selftest-save');
    const list = await window.__TAURI__.core.invoke('history_list');
    const loaded = await window.__TAURI__.core.invoke('history_load', { id: entry.id });
    await window.__TAURI__.core.invoke('history_delete', { id: entry.id });
    report({ ok: true, frames: entry.frames, bytes: loaded.length, newest: list[0]?.name, count: list.length });
  } catch (e) {
    report({ error: String(e) });
  }
})();
"#;

/// Settings smoke test: round-trip the whole settings object, then restore it.
const SELFTEST_SETTINGS_JS: &str = r#"
(async () => {
  const inv = window.__TAURI__.core.invoke;
  const report = (data) => inv('selftest_report', { report: JSON.stringify(data) });
  try {
    const before = await inv('settings_get');
    await inv('settings_set', { settings: { ...before, start_at_login: true, minimize_to_tray: true, sync_interval_secs: 30 } });
    const after = await inv('settings_get');
    const info = await inv('app_info');
    // Drive the update popup with a fake release: no network involved.
    window.__ak820.showUpdate(
      { tag_name: 'v9.9.9', name: 'AK820 Studio v9.9.9', body: 'notes', prerelease: true,
        assets: [{ name: 'ak820-studio-9.9.9.rpm', browser_download_url: 'https://github.com/x' }] },
      { ...info, package: 'rpm' },
    );
    const dlg = document.getElementById('upd');
    const popup = {
      open: dlg.open,
      action: document.getElementById('updact').textContent,
      version: document.getElementById('updversion').textContent,
      notes: !document.getElementById('updnotes').hidden,
      command: document.getElementById('updcmdwrap').hidden === true,
    };
    dlg.close();
    // The link opener must never hand a non-web URL to the desktop.
    const refused = await inv('open_url', { url: 'javascript:alert(1)' }).then(() => false, () => true);
    await inv('settings_set', { settings: before });
    report({
      ok: after.start_at_login === true && after.minimize_to_tray === true && after.sync_interval_secs === 30,
      version: info.version,
      package: info.package,
      data_dir: info.data_dir,
      restored: (await inv('settings_get')).start_at_login === before.start_at_login,
      refusedBadUrl: refused,
      popup,
    });
  } catch (e) { report({ error: String(e) }); }
})();
"#;

/// Smoke-test hook: the UI reports its own state, then exits when `AK820_SELFTEST` is set.
#[tauri::command]
fn selftest_report(report: String) {
    eprintln!("[selftest] {report}");
    if std::env::var_os("AK820_SELFTEST").is_some() {
        std::process::exit(0);
    }
}

/// Start the Tauri application.
pub fn run() {
    let settings = Settings::load();
    let start_hidden = settings.start_minimized || std::env::args().any(|a| a == "--tray");

    tauri::Builder::default()
        .manage(AppState::default())
        .setup(move |app| {
            let handle = app.handle().clone();

            // Keep the autostart entry in step with the settings file.
            let _ = set_autostart(settings.start_at_login);
            // Without a tray icon a hidden window would be unreachable (GNOME
            // needs an extension), so only hide when the tray really exists.
            let tray = build_tray(&handle).is_ok();
            if start_hidden
                && tray
                && let Some(window) = app.get_webview_window("main")
            {
                window.hide()?;
            }

            let watcher = handle.clone();
            std::thread::spawn(move || watch_devices(watcher));
            let interval = handle.clone();
            std::thread::spawn(move || watch_interval(interval));

            if settings.sync_on_start {
                let quit_after = settings.quit_after_start_sync;
                let handle = handle.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(600));
                    if ak820_device::presence().is_empty() {
                        // Nothing to do now; the watcher covers a later plug-in.
                        if quit_after {
                            handle.exit(0);
                        }
                        return;
                    }
                    match sync_background() {
                        Ok(_) => {
                            let _ = handle.emit("clock-synced", ());
                        }
                        Err(e) => {
                            let _ = handle.emit("clock-error", e);
                        }
                    }
                    if quit_after {
                        handle.exit(0);
                    }
                });
            }

            if std::env::var_os("AK820_SELFTEST").is_some() {
                eprintln!(
                    "[selftest] status {}",
                    serde_json::to_string(&snapshot()).unwrap_or_default()
                );
            }

            if std::env::var_os("AK820_SELFTEST").is_some()
                && let Some(window) = app.get_webview_window("main")
            {
                let script = match std::env::var("AK820_SELFTEST").as_deref() {
                    Ok("upload") => SELFTEST_UPLOAD_JS,
                    Ok("save") => SELFTEST_SAVE_JS,
                    Ok("settings") => SELFTEST_SETTINGS_JS,
                    _ => SELFTEST_JS,
                };
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(4));
                    let _ = window.eval(script);
                });
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event
                && Settings::load().minimize_to_tray
            {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            connect,
            device_status,
            device_info,
            sync_clock,
            show_builtin,
            upload,
            cancel_upload,
            history_save,
            history_list,
            history_load,
            history_delete,
            history_clear,
            settings_get,
            settings_set,
            app_info,
            update_download,
            update_install,
            update_replace_appimage,
            open_url,
            selftest_report,
        ])
        .run(tauri::generate_context!())
        .expect("error while running AK820 Studio");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_command_matches_the_package() {
        let rpm = install_command("rpm", Path::new("/home/u/Downloads/a.rpm"));
        assert_eq!(rpm, "sudo dnf install -y \"/home/u/Downloads/a.rpm\"");
        let deb = install_command("deb", Path::new("/home/u/Downloads/a.deb"));
        assert_eq!(deb, "sudo apt install -y \"/home/u/Downloads/a.deb\"");
    }

    #[test]
    fn only_downloaded_packages_are_installable() {
        assert!(!is_installable(Path::new("/etc/passwd")));
        assert!(!is_installable(Path::new("/tmp/evil.rpm")));
        assert!(!is_installable(Path::new("/home/u/Downloads/nope.deb")));
    }

    #[test]
    fn only_web_pages_can_be_opened() {
        assert!(is_openable_url(
            "https://github.com/WhatTFF/linux-ajazz-ak820-pro-app"
        ));
        assert!(is_openable_url("http://localhost:1234/x"));
        for bad in [
            "",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "-rf",
            "https://",
            "https://exa mple.com",
            "https://example.com/\u{1b}[31m",
            "ftp://example.com",
        ] {
            assert!(!is_openable_url(bad), "{bad:?} must be refused");
        }
    }

    #[test]
    fn history_ids_cannot_escape_the_directory() {
        assert!(history_path("../etc/passwd").is_err());
        assert!(history_path("a/b.bin").is_err());
        assert!(history_path("").is_err());
    }

    #[test]
    fn missing_thumb_is_not_fatal() {
        assert!(thumb_data_url(&[]).is_none());
        assert!(thumb_data_url(&[0u8; 10]).is_none());
    }

    #[test]
    fn payload_size_is_checked_before_saving() {
        assert!(validate_payload(&[]).is_err());
        assert!(validate_payload(&[0u8]).is_err());
        assert!(
            validate_payload(&[2u8; 256]).is_err(),
            "too short for 2 frames"
        );
        assert!(validate_payload(&vec![2u8; ak820_proto::payload_len(2)]).is_ok());
    }

    #[test]
    fn autostart_entry_lives_under_xdg_config_home() {
        let path = autostart_path();
        assert!(path.ends_with("autostart/ak820-studio.desktop"), "{path:?}");
    }

    #[test]
    fn autostart_entry_is_written_and_removed() {
        let dir = std::env::temp_dir().join(format!("ak820-autostart-{}", std::process::id()));
        let path = dir.join("autostart/ak820-studio.desktop");
        let _ = std::fs::remove_dir_all(&dir);

        set_autostart_in(&path, true).unwrap();
        let entry = std::fs::read_to_string(&path).unwrap();
        assert!(entry.contains("Exec=ak820-studio --tray"), "{entry}");
        assert!(entry.contains("[Desktop Entry]"));

        set_autostart_in(&path, false).unwrap();
        assert!(!path.exists());
        // Removing it twice is not an error.
        set_autostart_in(&path, false).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
