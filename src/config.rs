//! Persistent settings stored as JSON in `%APPDATA%\downcop\config.json`
//! (or `$XDG_CONFIG_HOME/downcop` / `~/.config/downcop` elsewhere).

use crate::policy::{Action, Policy};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    Ko,
    En,
}

impl Language {
    pub fn index(self) -> i32 {
        match self {
            Language::Ko => 0,
            Language::En => 1,
        }
    }

    pub fn from_index(i: i32) -> Self {
        if i == 1 { Language::En } else { Language::Ko }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Unit {
    #[default]
    #[serde(rename = "KB/s")]
    KBps,
    #[serde(rename = "MB/s")]
    MBps,
}

impl Unit {
    pub fn multiplier(self) -> f64 {
        match self {
            Unit::KBps => 1024.0,
            Unit::MBps => 1024.0 * 1024.0,
        }
    }

    pub fn index(self) -> i32 {
        match self {
            Unit::KBps => 0,
            Unit::MBps => 1,
        }
    }

    pub fn from_index(i: i32) -> Self {
        if i == 1 { Unit::MBps } else { Unit::KBps }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DirectionConfig {
    pub blocked: bool,
    pub limit_enabled: bool,
    pub limit_value: f64,
    pub limit_unit: Unit,
}

impl DirectionConfig {
    fn with_limit(value: f64, unit: Unit) -> Self {
        DirectionConfig { blocked: false, limit_enabled: false, limit_value: value, limit_unit: unit }
    }

    /// Limit in bytes per second, if a usable one is configured.
    pub fn limit_bytes(&self) -> Option<u64> {
        let v = self.limit_value * self.limit_unit.multiplier();
        (v.is_finite() && v >= 1.0).then(|| v.round() as u64)
    }

    pub fn action(&self) -> Action {
        if self.blocked {
            Action::Block
        } else if self.limit_enabled {
            self.limit_bytes().map_or(Action::Pass, Action::Limit)
        } else {
            Action::Pass
        }
    }
}

impl Default for DirectionConfig {
    fn default() -> Self {
        DirectionConfig::with_limit(1024.0, Unit::KBps)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub language: Language,
    /// Start downcop when the user logs on to Windows.
    pub autostart: bool,
    /// Re-apply the last block/limit state as soon as downcop starts.
    /// Independent from `autostart`.
    pub apply_on_launch: bool,
    /// Leave local network traffic alone.
    pub lan_exclude: bool,
    pub download: DirectionConfig,
    pub upload: DirectionConfig,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            language: crate::i18n::system_language(),
            autostart: false,
            apply_on_launch: false,
            lan_exclude: true,
            download: DirectionConfig::with_limit(1024.0, Unit::KBps),
            upload: DirectionConfig::with_limit(512.0, Unit::KBps),
        }
    }
}

impl Config {
    pub fn policy(&self) -> Policy {
        Policy { download: self.download.action(), upload: self.upload.action(), lan_exclude: self.lan_exclude }
    }

    /// Turns off every block/limit switch while keeping the configured values.
    /// Used at start-up when `apply_on_launch` is disabled.
    pub fn clear_active_state(&mut self) {
        for d in [&mut self.download, &mut self.upload] {
            d.blocked = false;
            d.limit_enabled = false;
        }
    }

    pub fn all_blocked(&self) -> bool {
        self.download.blocked && self.upload.blocked
    }

    /// One-click toggle: block everything, or unblock everything if both
    /// directions are already blocked.
    pub fn toggle_all_blocked(&mut self) {
        let block = !self.all_blocked();
        self.download.blocked = block;
        self.upload.blocked = block;
    }

    pub fn from_json(s: &str) -> Config {
        serde_json::from_str(s).unwrap_or_default()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("config serializes")
    }

    pub fn load() -> Config {
        match std::fs::read_to_string(config_file()) {
            Ok(s) => Config::from_json(&s),
            Err(_) => Config::default(),
        }
    }

    pub fn save(&self) {
        let dir = config_dir();
        let path = config_file();
        let tmp = path.with_extension("json.tmp");
        let result = std::fs::create_dir_all(&dir)
            .and_then(|_| std::fs::write(&tmp, self.to_json()))
            .and_then(|_| std::fs::rename(&tmp, &path));
        if let Err(e) = result {
            log(&format!("failed to save config to {}: {e}", path.display()));
        }
    }
}

pub fn config_dir() -> PathBuf {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    };
    base.unwrap_or_else(std::env::temp_dir).join("downcop")
}

fn config_file() -> PathBuf {
    config_dir().join("config.json")
}

/// Appends a line to `downcop.log` next to the config (the GUI has no console).
pub fn log(msg: &str) {
    let dir = config_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("downcop.log");
    // keep the log small
    if std::fs::metadata(&path).map(|m| m.len() > 512 * 1024).unwrap_or(false) {
        let _ = std::fs::remove_file(&path);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let _ = writeln!(f, "[{secs}] {msg}");
    }
    #[cfg(debug_assertions)]
    eprintln!("{msg}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_round_trip() {
        let mut c = Config { language: Language::En, autostart: true, ..Default::default() };
        c.download.blocked = true;
        c.upload.limit_enabled = true;
        c.upload.limit_value = 2.5;
        c.upload.limit_unit = Unit::MBps;
        let back = Config::from_json(&c.to_json());
        assert_eq!(back, c);
    }

    #[test]
    fn json_uses_readable_names() {
        let c = Config { language: Language::Ko, ..Default::default() };
        let j = c.to_json();
        assert!(j.contains("\"language\": \"ko\""));
        assert!(j.contains("\"limit_unit\": \"KB/s\""));
        assert!(j.contains("\"apply_on_launch\""));
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let c = Config::from_json(r#"{"autostart": true, "download": {"blocked": true}}"#);
        assert!(c.autostart);
        assert!(c.download.blocked);
        assert_eq!(c.download.limit_value, 1024.0);
        assert!(c.lan_exclude);
    }

    #[test]
    fn garbage_gives_defaults() {
        assert_eq!(Config::from_json("not json").upload.limit_value, 512.0);
    }

    #[test]
    fn actions_from_config() {
        let mut d = DirectionConfig::default();
        assert_eq!(d.action(), Action::Pass);
        d.limit_enabled = true;
        assert_eq!(d.action(), Action::Limit(1024 * 1024));
        d.limit_unit = Unit::MBps;
        d.limit_value = 1.5;
        assert_eq!(d.action(), Action::Limit(1_572_864));
        d.limit_value = 0.0;
        assert_eq!(d.action(), Action::Pass); // unusable limit → no limit
        d.blocked = true;
        assert_eq!(d.action(), Action::Block); // block wins over limit
    }

    #[test]
    fn toggle_all_and_clear() {
        let mut c = Config::default();
        c.download.blocked = true;
        c.toggle_all_blocked();
        assert!(c.all_blocked());
        c.toggle_all_blocked();
        assert!(!c.download.blocked && !c.upload.blocked);

        c.download.limit_enabled = true;
        c.upload.blocked = true;
        c.download.limit_value = 77.0;
        c.clear_active_state();
        assert!(c.policy().is_passthrough());
        assert_eq!(c.download.limit_value, 77.0);
    }
}
