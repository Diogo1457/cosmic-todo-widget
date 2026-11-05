//! Settings and task storage.
//!
//! * `~/.config/todo-widget/settings.json`   – the user's settings
//! * `~/.local/share/todo-widget/tasks.json` – tasks + position/size the widget was left with
//! * `~/.config/autostart/todo-widget.desktop` – exists while "Start at login" is on
//!
//! Old files stay readable: unknown keys are ignored.

use std::{fs, path::PathBuf, time::Duration};

use chrono::Local;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// Narrowest the widget can be made (logical px).
pub const MIN_WIDTH: f32 = 320.0;

fn today() -> String {
    Local::now().date_naive().to_string() // YYYY-MM-DD
}

/// Takes the "one desktop widget at a time" lock (held for as long as the returned file lives).
/// Retries for a few seconds so a restart (old instance still shutting down) works.
pub fn acquire_widget_lock() -> Option<fs::File> {
    let dir = dirs::runtime_dir().unwrap_or_else(std::env::temp_dir);
    let file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(dir.join("todo-widget.lock"))
        .ok()?;
    for _ in 0..30 {
        if file.try_lock().is_ok() {
            return Some(file);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    None
}

fn write_atomic(path: &PathBuf, contents: &str) {
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("json.tmp");
    let result = fs::write(&tmp, contents).and_then(|()| fs::rename(&tmp, path));
    if let Err(err) = result {
        eprintln!("Could not save {}: {err}", path.display());
    }
}

// --------------------------------------------------------------------------- autostart

/// "Start at login" is on exactly when the autostart `.desktop` file exists, so the file is the
/// single source of truth (nothing about it is stored in `settings.json`). Off by default.
pub mod autostart {
    use std::{fs, path::PathBuf};

    fn path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("autostart")
            .join("todo-widget.desktop")
    }

    pub fn is_enabled() -> bool {
        path().exists()
    }

    /// The installed binary if there is one (so autostart survives rebuilds of a dev copy),
    /// otherwise the running executable.
    fn exec_path() -> Option<PathBuf> {
        let installed = dirs::executable_dir()
            .map(|d| d.join(env!("CARGO_PKG_NAME")))
            .filter(|p| p.is_file());
        installed.or_else(|| std::env::current_exe().ok())
    }

    /// Create or remove the autostart entry.
    pub fn set(enabled: bool) {
        let path = path();
        if !enabled {
            if let Err(err) = fs::remove_file(&path) {
                if err.kind() != std::io::ErrorKind::NotFound {
                    eprintln!("Could not remove {}: {err}", path.display());
                }
            }
            return;
        }
        let Some(exe) = exec_path() else {
            eprintln!("Could not find the executable path; autostart not enabled");
            return;
        };
        let exe = exe.display().to_string();
        let exec = if exe.contains(' ') {
            format!("\"{}\"", exe.replace('\\', "\\\\").replace('"', "\\\""))
        } else {
            exe
        };
        let entry = format!(
            "[Desktop Entry]\nType=Application\nName=Todo Widget\n\
             Comment=Daily to-do list widget\nExec={exec} --background\nTerminal=false\n\
             X-GNOME-Autostart-enabled=true\n"
        );
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if let Err(err) = fs::write(&path, entry) {
            eprintln!("Could not save {}: {err}", path.display());
        }
    }
}

// --------------------------------------------------------------------------- settings

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
    /// 24-hour clock (`true`) or 12-hour AM/PM (`false`).
    pub time_24h: bool,
    pub show_seconds: bool,
    /// `"auto"` or a monitor id.
    pub monitor: String,
    /// `false` = part of the desktop (windows cover it).
    pub above_windows: bool,
    /// Card opacity in percent.
    pub opacity: u32,
    /// Start the widget at login (mirrors the autostart file; off by default).
    pub autostart: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            display: DisplayMode::Panel,
            time_24h: true,
            show_seconds: false,
            monitor: "auto".into(),
            above_windows: false,
            opacity: 88,
            autostart: false,
        }
    }
}

impl Settings {
    fn path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("todo-widget")
            .join("settings.json")
    }

    /// Load settings; unknown keys and invalid values are ignored (defaults are used).
    pub fn load() -> Self {
        let mut s = Self::default();
        s.autostart = autostart::is_enabled();
        let Ok(text) = fs::read_to_string(Self::path()) else {
            return s;
        };
        let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&text) else {
            return s;
        };

        let boolean = |k: &str, into: &mut bool| {
            if let Some(Value::Bool(b)) = map.get(k) {
                *into = *b;
            }
        };

        if let Some(Value::String(t)) = map.get("time_format") {
            match t.as_str() {
                "12" => s.time_24h = false,
                "24" => s.time_24h = true,
                _ => {}
            }
        }
        if let Some(Value::String(d)) = map.get("display") {
            match d.as_str() {
                "panel" => s.display = DisplayMode::Panel,
                "widget" => s.display = DisplayMode::Widget,
                _ => {}
            }
        }
        boolean("show_seconds", &mut s.show_seconds);
        if let Some(Value::String(m)) = map.get("monitor") {
            s.monitor = m.clone();
        }
        boolean("above_windows", &mut s.above_windows);
        if let Some(v) = map.get("opacity").and_then(Value::as_f64) {
            s.opacity = (v.round().max(0.0) as u32).clamp(30, 100);
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
        map.insert("time_format".into(), json!(if self.time_24h { "24" } else { "12" }));
        map.insert("show_seconds".into(), json!(self.show_seconds));
        map.insert("monitor".into(), json!(self.monitor));
        map.insert("above_windows".into(), json!(self.above_windows));
        map.insert("opacity".into(), json!(self.opacity));
        if let Ok(text) = serde_json::to_string_pretty(&Value::Object(map)) {
            write_atomic(&Self::path(), &text);
        }
    }
}

// --------------------------------------------------------------------------- tasks

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub text: String,
    #[serde(default)]
    pub done: bool,
}

/// Position and size the widget was left with (logical px).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SavedSize {
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub width: Option<f32>,
    pub list_h: Option<f32>,
}

pub struct Store {
    path: PathBuf,
    pub tasks: Vec<Task>,
    pub ui: SavedSize,
    day: String,
}

impl Store {
    fn path() -> PathBuf {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("todo-widget")
            .join("tasks.json")
    }

    pub fn load() -> Self {
        let path = Self::path();
        let mut store = Store {
            path,
            tasks: Vec::new(),
            ui: SavedSize::default(),
            day: today(),
        };

        let data = fs::read_to_string(&store.path)
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .unwrap_or(Value::Null);

        if let Some(Value::Array(items)) = data.get("tasks") {
            store.tasks = items
                .iter()
                .filter_map(|v| serde_json::from_value::<Task>(v.clone()).ok())
                .collect();
        }
        if let Some(ui) = data.get("ui") {
            let get = |k: &str| ui.get(k).and_then(Value::as_f64).map(|v| v as f32);
            store.ui.x = get("x");
            store.ui.y = get("y");
            store.ui.width = get("width");
            store.ui.list_h = get("list_h");
        }
        // New day: completed tasks from yesterday are dropped.
        if let Some(saved_day) = data.get("date").and_then(Value::as_str) {
            if saved_day != store.day {
                store.tasks.retain(|t| !t.done);
            }
        }
        store.save();
        store
    }

    /// Drops completed tasks when the date changes. Returns `true` if it did.
    pub fn rollover_if_needed(&mut self) -> bool {
        let now = today();
        if self.day == now {
            return false;
        }
        self.tasks.retain(|t| !t.done);
        self.day = now;
        self.save();
        true
    }

    pub fn save(&self) {
        let mut ui = Map::new();
        for (key, val) in [
            ("x", self.ui.x),
            ("y", self.ui.y),
            ("width", self.ui.width),
            ("list_h", self.ui.list_h),
        ] {
            if let Some(v) = val {
                ui.insert(key.into(), json!(v.round() as i64));
            }
        }
        let value = json!({ "date": self.day, "tasks": self.tasks, "ui": ui });
        if let Ok(text) = serde_json::to_string_pretty(&value) {
            write_atomic(&self.path, &text);
        }
    }

    pub fn add(&mut self, text: String) {
        self.tasks.push(Task {
            id: uuid::Uuid::new_v4().simple().to_string(),
            text,
            done: false,
        });
        self.save();
    }

    pub fn toggle(&mut self, id: &str, done: bool) {
        for t in self.tasks.iter_mut().filter(|t| t.id == id) {
            t.done = done;
        }
        self.save();
    }

    pub fn delete(&mut self, id: &str) {
        self.tasks.retain(|t| t.id != id);
        self.save();
    }

    /// Move a task one step up or down among the tasks with the same state
    /// (pending tasks are listed before completed ones, so they never mix).
    pub fn move_task(&mut self, id: &str, up: bool) {
        let Some(i) = self.tasks.iter().position(|t| t.id == id) else {
            return;
        };
        let done = self.tasks[i].done;
        let mut j = i;
        loop {
            if up {
                if j == 0 {
                    return;
                }
                j -= 1;
            } else {
                j += 1;
                if j >= self.tasks.len() {
                    return;
                }
            }
            if self.tasks[j].done == done {
                break;
            }
        }
        self.tasks.swap(i, j);
        self.save();
    }

    pub fn clear_done(&mut self) {
        self.tasks.retain(|t| !t.done);
        self.save();
    }

    pub fn set_ui(&mut self, ui: SavedSize) {
        self.ui = ui;
        self.save();
    }
}