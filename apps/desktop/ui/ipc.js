// Native transport for the AK820 Pro screen.
//
// This replaces the WebHID layer of the upstream `ak820.js` with Tauri commands.
// The public shape is unchanged, so `app.js` only needs its import line patched.
// `buildPayload` and `parsePayload` are kept byte-identical to upstream (MIT,
// fromagge/ak820-screen) because they define the wire format.

const core = window.__TAURI__.core;
export const invoke = core.invoke;

export const listen = window.__TAURI__.event.listen;
export const W = 128, H = 128;

export const supported = () => true;

export class Keyboard {
  constructor(status) {
    this.status = status;
    // `cfg` gates clock sync, `bulk` gates sending pictures.
    this.cfg = status.can_sync ? true : null;
    this.bulk = status.can_upload ? true : null;
    this.mode = status.mode || (status.connected ? 'usb' : 'none');
    this.name = status.name || 'AK820 Pro';
  }

  static async connect() {
    const status = await invoke('connect');
    return new Keyboard(status);
  }

  static async reconnect() {
    try {
      const status = await invoke('device_status');
      return status.connected ? new Keyboard(status) : null;
    } catch {
      return null;
    }
  }

  async info() {
    const s = await invoke('device_info');
    this.status = s;
    this.cfg = s.can_sync ? true : null;
    this.bulk = s.can_upload ? true : null;
    this.mode = s.mode || this.mode;
    return {
      firmware: s.firmware,
      battery: s.battery,
      charging: s.charging,
      profile: s.profile,
      maxFrames: s.max_frames,
      hasScreen: s.has_screen,
    };
  }

  async syncClock() {
    await invoke('sync_clock');
  }

  async showBuiltIn(index) {
    await invoke('show_builtin', { index });
  }

  // frames: [{ data: ImageData(128x128), delay: ms }]
  async upload(frames, { onProgress, signal, name } = {}) {
    if (!this.bulk) {
      throw new Error('the screen only answers on the USB cable — switch the keyboard to wired mode, or use "Save to history"');
    }
    const payload = buildPayload(frames);
    const channel = new core.Channel();
    channel.onmessage = (percent) => onProgress?.(percent / 100);
    signal?.addEventListener('abort', () => { invoke('cancel_upload').catch(() => {}); });
    await invoke('upload', { payload: Array.from(payload), name: name || 'Image', progress: channel });
    return payload;
  }
}

export function buildPayload(frames) {
  const n = frames.length, hdr = new Uint8Array(256).fill(0xff);
  hdr[0] = n;
  for (let k = 0; k < n - 1; k++) hdr[k + 1] = Math.max(1, Math.min(255, Math.round(frames[k].delay / 2)));
  hdr[n] = 0;
  const out = new Uint8Array(256 + n * W * H * 2);
  out.set(hdr);
  let j = 256;
  for (const { data } of frames) {
    const d = data.data;
    for (let p = 0; p < d.length; p += 4) {
      const v = ((d[p] & 0xf8) << 8) | ((d[p + 1] & 0xfc) << 3) | (d[p + 2] >> 3);
      out[j++] = v & 255;
      out[j++] = v >> 8;
    }
  }
  return out;
}

// Inverse of buildPayload: what was sent → [{ data: ImageData, delay }].
export function parsePayload(payload, limit = Infinity) {
  const n = payload[0], frames = [];
  for (let k = 0; k < Math.min(n, limit); k++) {
    const img = new ImageData(W, H), d = img.data;
    let j = 256 + k * W * H * 2;
    for (let p = 0; p < d.length; p += 4, j += 2) {
      const v = payload[j] | (payload[j + 1] << 8);
      d[p] = ((v >> 11) & 31) * 255 / 31;
      d[p + 1] = ((v >> 5) & 63) * 255 / 63;
      d[p + 2] = (v & 31) * 255 / 31;
      d[p + 3] = 255;
    }
    const raw = k < n - 1 ? payload[k + 1] : payload[n - 1] || 50;
    frames.push({ data: img, delay: n === 1 ? 500 : raw * 2 });
  }
  return frames;
}

export const payloadBytes = (n) => 256 + n * W * H * 2;

// Store a design without sending it (works with no keyboard attached).
export async function saveHistory(payload, name) {
  return invoke('history_save', { payload: Array.from(payload), name });
}

// A webview is not a browser: following an <a href> would replace this whole UI
// with the target page and leave no way back. Hand web links to the desktop.
document.addEventListener('click', (e) => {
  const a = e.target instanceof Element ? e.target.closest('a[href]') : null;
  if (!a) return;
  const href = a.href;
  if (!/^https?:\/\//i.test(href)) return;
  e.preventDefault();
  invoke('open_url', { url: href }).catch((err) => console.warn(`could not open ${href}`, err));
});
document.documentElement.dataset.linkGuard = '1';

// Exposed for the smoke test and for debugging from the web inspector.
window.__ak820 = { Keyboard, buildPayload, parsePayload, saveHistory };
