//! User settings, stored as a small JSON file.

use std::path::PathBuf;

/// Everything the app remembers between runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    /// Keep the keyboard clock in sync automatically.
    pub autosync_clock: bool,
    /// Re-upload the last animation when the keyboard is plugged in.
    pub restore_on_connect: bool,
    /// GIPHY API key (empty = search disabled).
    pub giphy_key: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            autosync_clock: true,
            restore_on_connect: false,
            giphy_key: String::new(),
        }
    }
}

/// Path of `settings.json`.
pub fn path() -> PathBuf {
    crate::data_dir().join("settings.json")
}

impl Settings {
    /// Load from disk, falling back to defaults.
    pub fn load() -> Self {
        std::fs::read_to_string(path())
            .ok()
            .and_then(|raw| parse(&raw))
            .unwrap_or_default()
    }

    /// Write to disk, creating the data directory if needed.
    pub fn save(&self) -> Result<(), String> {
        let path = path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&path, self.to_json()).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Serialise without a JSON dependency (the schema is three scalars).
    pub fn to_json(&self) -> String {
        format!(
            "{{\n  \"autosync_clock\": {},\n  \"restore_on_connect\": {},\n  \"giphy_key\": {}\n}}\n",
            self.autosync_clock,
            self.restore_on_connect,
            escape(&self.giphy_key)
        )
    }
}

fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Parse the tiny JSON object we write ourselves.
pub fn parse(raw: &str) -> Option<Settings> {
    let mut settings = Settings::default();
    for line in raw.lines() {
        let line = line
            .trim()
            .trim_end_matches(',')
            .trim_matches(|c| c == '{' || c == '}')
            .trim();
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim().trim_matches('"');
        let value = value.trim();
        match key {
            "autosync_clock" => settings.autosync_clock = value == "true",
            "restore_on_connect" => settings.restore_on_connect = value == "true",
            "giphy_key" => settings.giphy_key = unquote(value),
            _ => {}
        }
    }
    Some(settings)
}

fn unquote(value: &str) -> String {
    let inner = value.trim_matches('"');
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('u') => {
                let hex: String = chars.by_ref().take(4).collect();
                if let Ok(code) = u32::from_str_radix(&hex, 16)
                    && let Some(c) = char::from_u32(code)
                {
                    out.push(c);
                }
            }
            Some(other) => out.push(other),
            None => break,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let settings = Settings {
            autosync_clock: false,
            restore_on_connect: true,
            giphy_key: "abc\"def\\ghi\n".into(),
        };
        let parsed = parse(&settings.to_json()).unwrap();
        assert_eq!(parsed, settings);
    }

    #[test]
    fn defaults_when_missing_keys() {
        let parsed = parse("{}").unwrap();
        assert!(parsed.autosync_clock);
        assert!(!parsed.restore_on_connect);
        assert!(parsed.giphy_key.is_empty());
    }

    #[test]
    fn tolerates_junk() {
        assert!(parse("not json at all").is_some());
        assert_eq!(parse("{\"giphy_key\": \"x\"}").unwrap().giphy_key, "x");
    }
}
