//! User settings, stored in `<config dir>/todo-widget/settings.json`.
//!
//! Old files stay readable: unknown keys and invalid values are ignored. "Start at login" is
//! not stored here; see [`crate::autostart`].

use std::{fs, path::PathBuf};

use serde_json::{json, Map, Value};

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
        let text = fs::read_to_string(Self::path()).unwrap_or_default();
        let mut s = Self::parse(&text);
        s.autostart = autostart::is_enabled();
        s
    }

    /// Reads settings from JSON text. Unknown keys and invalid values are ignored.
    fn parse(text: &str) -> Self {
        let mut s = Settings::default();
        let Ok(Value::Object(map)) = serde_json::from_str::<Value>(text) else {
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
        if let Some(text) = self.to_json() {
            paths::write_atomic(&Self::path(), &text);
        }
    }

    /// The text written to `settings.json` ("Start at login" is not part of it).
    fn to_json(&self) -> Option<String> {
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
        serde_json::to_string_pretty(&Value::Object(map)).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_panel_system_transparency_and_no_clock() {
        let s = Settings::default();
        assert_eq!(s.display, DisplayMode::Panel);
        assert_eq!(s.opacity, None);
        assert!(!s.show_clock);
        assert_eq!(s.monitor, "auto");
        assert!(!s.above_windows);
        assert!(!s.autostart);
    }

    #[test]
    fn empty_or_broken_text_gives_the_defaults() {
        for text in ["", "   ", "not json", "[1, 2]", "42", "null"] {
            assert_eq!(
                Settings::parse(text),
                Settings::default(),
                "input: {text:?}"
            );
        }
    }

    #[test]
    fn display_mode_is_read() {
        assert_eq!(
            Settings::parse(r#"{"display":"panel"}"#).display,
            DisplayMode::Panel
        );
        // Inside a Flatpak the widget is not possible and the panel is forced.
        let widget = if paths::widget_supported() {
            DisplayMode::Widget
        } else {
            DisplayMode::Panel
        };
        assert_eq!(Settings::parse(r#"{"display":"widget"}"#).display, widget);
        assert_eq!(
            Settings::parse(r#"{"display":"sideways"}"#).display,
            DisplayMode::Panel
        );
    }

    #[test]
    fn opacity_is_kept_within_the_allowed_range() {
        let opacity = |text: &str| Settings::parse(text).opacity;
        assert_eq!(opacity(r#"{"opacity":0.1}"#), Some(MIN_OPACITY));
        assert_eq!(opacity(r#"{"opacity":7}"#), Some(1.0));
        assert_eq!(opacity(r#"{"opacity":0.5}"#), Some(0.5));
        assert_eq!(opacity(r#"{"opacity":"half"}"#), None);
        assert_eq!(opacity("{}"), None);
    }

    #[test]
    fn other_values_are_read() {
        let s = Settings::parse(r#"{"show_clock":true,"monitor":"ACME|X1","above_windows":true}"#);
        assert!(s.show_clock && s.above_windows);
        assert_eq!(s.monitor, "ACME|X1");
    }

    #[test]
    fn wrong_types_and_unknown_keys_are_ignored() {
        let s =
            Settings::parse(r#"{"show_clock":"yes","monitor":5,"above_windows":1,"colour":"red"}"#);
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn settings_survive_being_written_and_read_again() {
        let s = Settings {
            display: DisplayMode::Panel,
            opacity: Some(0.55),
            show_clock: true,
            monitor: "ACME|X1#1".into(),
            above_windows: true,
            autostart: false,
        };
        let back = Settings::parse(&s.to_json().unwrap());
        assert_eq!(back.monitor, s.monitor);
        assert!(back.show_clock && back.above_windows);
        assert!((back.opacity.unwrap() - 0.55).abs() < 1e-6);
    }

    #[test]
    fn the_file_names_the_display_mode() {
        let s = Settings {
            display: DisplayMode::Widget,
            ..Settings::default()
        };
        assert!(s.to_json().unwrap().contains(r#""display": "widget""#));
        assert!(Settings::default()
            .to_json()
            .unwrap()
            .contains(r#""display": "panel""#));
    }

    #[test]
    fn system_transparency_writes_no_opacity_and_autostart_is_never_stored() {
        let text = Settings::default().to_json().unwrap();
        assert!(!text.contains("opacity"));
        assert!(!text.contains("autostart"));
    }
}
