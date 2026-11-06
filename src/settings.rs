//! User settings, stored in `<config dir>/todo-widget/settings.json`.
//!
//! Old files stay readable: unknown keys and invalid values are ignored. "Start at login" is
//! not stored here; see [`crate::autostart`].

use std::{fs, path::PathBuf};

use serde_json::{Map, Value, json};

use crate::{autostart, paths};

/// Lowest opacity the user can choose for the widget (1.0 = fully opaque).
pub const MIN_OPACITY: f32 = 0.3;

/// Where the to-do list is shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DisplayMode {
    /// A free-standing desktop widget (layer-shell).
    Widget,
    /// A popup of the COSMIC panel applet (the default).
    #[default]
    Panel,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// Show as a desktop widget or as a panel applet popup.
    pub display: DisplayMode,
    /// Widget background opacity chosen by the user (`MIN_OPACITY..=1.0`).
    /// `None` (the default) = follow the system's frosted-glass transparency.
    pub opacity: Option<f32>,
    /// Show the clock and the date in the header (off by default).
    pub show_clock: bool,
    /// `"auto"` or a monitor id.
    pub monitor: String,
    /// `false` = part of the desktop (windows cover it).
    pub above_windows: bool,
    /// Start the widget at login (mirrors the autostart file; off by default).
    pub autostart: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            display: DisplayMode::Panel,
            opacity: None,
            show_clock: false,
            monitor: "auto".into(),
            above_windows: false,
            autostart: false,
        }
    }
}

impl Settings {
    fn path() -> PathBuf {
        paths::config_dir().join("settings.json")
    }

    /// Loads the settings; anything missing or invalid falls back to the default.
    pub fn load() -> Self {
        let mut s = Settings { autostart: autostart::is_enabled(), ..Settings::default() };
        let Ok(text) = fs::read_to_string(Self::path()) else {
            return s;
        };
        let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&text) else {
            return s;
        };

        match map.get("display").and_then(Value::as_str) {
            Some("panel") => s.display = DisplayMode::Panel,
            Some("widget") => s.display = DisplayMode::Widget,
            _ => {}
        }
        // The widget cannot be shown inside a Flatpak (see `paths::widget_supported`).
        if !paths::widget_supported() {
            s.display = DisplayMode::Panel;
        }
        if let Some(b) = map.get("show_clock").and_then(Value::as_bool) {
            s.show_clock = b;
        }
        if let Some(a) = map.get("opacity").and_then(Value::as_f64) {
            s.opacity = Some((a as f32).clamp(MIN_OPACITY, 1.0));
        }
        if let Some(m) = map.get("monitor").and_then(Value::as_str) {
            s.monitor = m.to_string();
        }
        if let Some(b) = map.get("above_windows").and_then(Value::as_bool) {
            s.above_windows = b;
        }
        s
    }

    pub fn save(&self) {
        let mut map = Map::new();
        map.insert(
            "display".into(),
            json!(match self.display {
                DisplayMode::Widget => "widget",
                DisplayMode::Panel => "panel",
            }),
        );
        map.insert("show_clock".into(), json!(self.show_clock));
        if let Some(a) = self.opacity {
            map.insert("opacity".into(), json!((a * 100.0).round() / 100.0));
        }
        map.insert("monitor".into(), json!(self.monitor));
        map.insert("above_windows".into(), json!(self.above_windows));
        if let Ok(text) = serde_json::to_string_pretty(&Value::Object(map)) {
            paths::write_atomic(&Self::path(), &text);
        }
    }
}