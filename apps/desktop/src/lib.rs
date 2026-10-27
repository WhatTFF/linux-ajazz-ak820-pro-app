//! Tauri backend: device access, uploads, history.
//!
//! All USB work happens here. The frontend is the vendored upstream UI, with its
//! WebHID layer replaced by the commands below (see `ui/ipc.js`).

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, UNIX_EPOCH};

use ak820_device::{Device, DeviceLock};
use ak820_proto::{CivilTime, DeviceInfo};
use ak820_store::history;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::{Datelike, Timelike};
use serde::Serialize;
use tauri::{Emitter, Manager, State};

/// Shared, cheap state: just the cancel flag.
#[derive(Default)]
pub struct AppState {
    cancel: Arc<AtomicBool>,
}

/// What the UI shows in the keyboard pill and device panel.
#[derive(Clone, Debug, Default, Serialize)]
pub struct DeviceStatus {
    pub connected: bool,
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
#[derive(Clone, Debug, Serialize, serde::Deserialize)]
pub struct SettingsDto {
    pub autosync_clock: bool,
    pub restore_on_connect: bool,
    pub giphy_key: String,
}

impl From<ak820_store::settings::Settings> for SettingsDto {
    fn from(value: ak820_store::settings::Settings) -> Self {
        SettingsDto {
            autosync_clock: value.autosync_clock,
            restore_on_connect: value.restore_on_connect,
            giphy_key: value.giphy_key,
        }
    }
}

impl From<SettingsDto> for ak820_store::settings::Settings {
    fn from(value: SettingsDto) -> Self {
        ak820_store::settings::Settings {
            autosync_clock: value.autosync_clock,
            restore_on_connect: value.restore_on_connect,
            giphy_key: value.giphy_key,
        }
    }
}

// --- commands ---------------------------------------------------------------------

/// Open the keyboard and read its info; errors are shown to the user.
#[tauri::command]
fn connect() -> Result<DeviceStatus, String> {
    let interfaces = ak820_device::find();
    if interfaces.nodes.is_empty() {
        return Err(ak820_device::Error::NotFound.to_string());
    }
    let _lock = DeviceLock::acquire().map_err(|e| e.to_string())?;
    let mut device = Device::open().map_err(|e| e.to_string())?;
    let has_config = device.has_config();
    let info = device.info().map_err(|e| e.to_string())?;
    Ok(status_from(&info, has_config, None))
}

/// Cheap poll used by the UI and the hotplug watcher; never fails.
#[tauri::command]
fn device_status() -> DeviceStatus {
    let interfaces = ak820_device::find();
    if interfaces.nodes.is_empty() {
        return DeviceStatus::default();
    }
    let has_config = interfaces.config.is_some();
    let mut status = DeviceStatus {
        connected: true,
        name: "AK820 Pro".into(),
        has_config,
        ..Default::default()
    };

    let Ok(_lock) = DeviceLock::acquire() else {
        status.error = Some("busy: another AK820 tool is talking to the keyboard".into());
        return status;
    };
    if has_config
        && let Ok(mut device) = Device::open()
        && let Ok(info) = device.info()
    {
        status = status_from(&info, has_config, None);
    }
    status
}

/// Full device info (firmware, battery, frame limit).
#[tauri::command]
fn device_info() -> Result<DeviceStatus, String> {
    let _lock = DeviceLock::acquire().map_err(|e| e.to_string())?;
    let mut device = Device::open().map_err(|e| e.to_string())?;
    let has_config = device.has_config();
    let info = device.info().map_err(|e| e.to_string())?;
    Ok(status_from(&info, has_config, None))
}

/// Push this computer's clock to the screen.
#[tauri::command]
fn sync_clock() -> Result<String, String> {
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
    let _lock = DeviceLock::acquire().map_err(|e| e.to_string())?;
    let mut device = Device::open().map_err(|e| e.to_string())?;
    device.sync_clock(time).map_err(|e| e.to_string())?;
    Ok(now.format("%H:%M:%S").to_string())
}

/// Select a firmware built-in animation.
#[tauri::command]
fn show_builtin(index: u8) -> Result<(), String> {
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

/// Smoke-test hook: the UI reports its own state, then exits when `AK820_SELFTEST` is set.
#[tauri::command]
fn selftest_report(report: String) {
    eprintln!("[selftest] {report}");
    if std::env::var_os("AK820_SELFTEST").is_some() {
        std::process::exit(0);
    }
}

/// Read persisted settings.
#[tauri::command]
fn settings_get() -> SettingsDto {
    ak820_store::settings::Settings::load().into()
}

/// Write persisted settings.
#[tauri::command]
fn settings_set(settings: SettingsDto) -> Result<(), String> {
    ak820_store::settings::Settings::from(settings).save()
}

// --- helpers ----------------------------------------------------------------------

fn status_from(info: &DeviceInfo, has_config: bool, error: Option<String>) -> DeviceStatus {
    DeviceStatus {
        connected: true,
        name: "AK820 Pro".into(),
        firmware: format!(
            "{}.{:02}",
            info.firmware_hundredths / 100,
            info.firmware_hundredths % 100
        ),
        battery: info.battery,
        charging: info.charging,
        profile: info.profile,
        max_frames: info.max_frames,
        has_screen: info.has_screen(),
        has_config,
        error,
    }
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

// --- app --------------------------------------------------------------------------

/// Watch USB hotplug and tell the UI when the keyboard appears or disappears.
fn watch_devices(app: tauri::AppHandle) {
    let mut last: Option<bool> = None;
    loop {
        let connected = !ak820_device::find().nodes.is_empty();
        if last != Some(connected) {
            last = Some(connected);
            let _ = app.emit("device-changed", connected);
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

/// Injected by `AK820_SELFTEST=1` to prove the UI loaded and can reach Rust.
const SELFTEST_JS: &str = r#"
(() => {
  const text = (id) => document.getElementById(id)?.textContent?.trim() ?? null;
  const info = {
    title: document.title,
    tauri: !!window.__TAURI__,
    tabs: document.querySelectorAll('[role=tab]').length,
    generators: document.querySelectorAll('#gens button').length,
    hasPreview: !!document.getElementById('preview'),
    hasHistoryPanel: !!document.getElementById('hist'),
    historyItems: document.querySelectorAll('#hist .hitem').length,
    pill: text('pilltext'),
    devicePanelHidden: document.getElementById('devpanel')?.hidden ?? null,
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

/// Start the Tauri application.
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::default())
        .setup(|app| {
            let handle = app.handle().clone();
            std::thread::spawn(move || watch_devices(handle));

            if std::env::var_os("AK820_SELFTEST").is_some()
                && let Some(window) = app.get_webview_window("main")
            {
                let script = match std::env::var("AK820_SELFTEST").as_deref() {
                    Ok("upload") => SELFTEST_UPLOAD_JS,
                    _ => SELFTEST_JS,
                };
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(4));
                    let _ = window.eval(script);
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            connect,
            device_status,
            device_info,
            sync_clock,
            show_builtin,
            upload,
            cancel_upload,
            history_list,
            history_load,
            history_delete,
            history_clear,
            settings_get,
            settings_set,
            selftest_report,
        ])
        .run(tauri::generate_context!())
        .expect("error while running AK820 Studio");
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
