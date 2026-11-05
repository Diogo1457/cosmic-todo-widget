//! Settings and task storage.
//!
//! * `~/.config/todo-widget/settings.json`   – the user's settings
//! * `~/.local/share/todo-widget/tasks.json` – tasks, folders + position/size the widget was left with
//! * `~/.config/autostart/todo-widget.desktop` – exists while "Start at login" is on
//!
//! Old files stay readable: unknown keys are ignored. Tasks are never removed automatically:
//! only the user deletes them (a task, "Clear done", or a folder together with its tasks after
//! a confirmation).

use std::{fs, path::PathBuf, time::Duration};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// Narrowest the widget can be made (logical px).
pub const MIN_WIDTH: f32 = 380.0;

/// Lowest opacity the user can choose for the widget (1.0 = fully opaque).
pub const MIN_OPACITY: f32 = 0.3;

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

        if let Some(Value::String(d)) = map.get("display") {
            match d.as_str() {
                "panel" => s.display = DisplayMode::Panel,
                "widget" => s.display = DisplayMode::Widget,
                _ => {}
            }
        }
        boolean("show_clock", &mut s.show_clock);
        if let Some(a) = map.get("opacity").and_then(Value::as_f64) {
            s.opacity = Some((a as f32).clamp(MIN_OPACITY, 1.0));
        }
        if let Some(Value::String(m)) = map.get("monitor") {
            s.monitor = m.clone();
        }
        boolean("above_windows", &mut s.above_windows);
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
    /// Id of the folder this task lives in (`None` = top level).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Folder {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub collapsed: bool,
}

/// Something in the list that can be dragged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ItemRef {
    Task(String),
    Folder(String),
}

/// Where a dragged item is dropped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DropTarget {
    /// On a task: take its place (and its folder).
    Task(String),
    /// On a folder header: go inside it (for tasks) or take its place (for folders).
    Folder(String),
    /// Below everything, outside of any folder.
    RootEnd,
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
    pub folders: Vec<Folder>,
    pub ui: SavedSize,
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
            folders: Vec::new(),
            ui: SavedSize::default(),
        };

        let mut readable = true;
        let data = match fs::read_to_string(&store.path) {
            Ok(text) => match serde_json::from_str::<Value>(&text) {
                Ok(v) => v,
                Err(err) => {
                    // Never silently lose tasks: keep a copy of a file we cannot parse.
                    readable = false;
                    let backup = store.path.with_extension("json.corrupt");
                    eprintln!(
                        "Could not parse {} ({err}); keeping a copy at {}",
                        store.path.display(),
                        backup.display()
                    );
                    let _ = fs::copy(&store.path, &backup);
                    Value::Null
                }
            },
            Err(_) => Value::Null,
        };

        if let Some(Value::Array(items)) = data.get("folders") {
            store.folders = items
                .iter()
                .filter_map(|v| serde_json::from_value::<Folder>(v.clone()).ok())
                .collect();
        }
        if let Some(Value::Array(items)) = data.get("tasks") {
            store.tasks = items
                .iter()
                .filter_map(|v| serde_json::from_value::<Task>(v.clone()).ok())
                .collect();
        }
        // A task pointing to a folder that no longer exists goes back to the top level.
        let ids: Vec<String> = store.folders.iter().map(|f| f.id.clone()).collect();
        for t in &mut store.tasks {
            if t.folder.as_ref().is_some_and(|f| !ids.contains(f)) {
                t.folder = None;
            }
        }
        if let Some(ui) = data.get("ui") {
            let get = |k: &str| ui.get(k).and_then(Value::as_f64).map(|v| v as f32);
            store.ui.x = get("x");
            store.ui.y = get("y");
            store.ui.width = get("width");
            store.ui.list_h = get("list_h");
        }
        if readable {
            store.save();
        }
        store
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
        let value = json!({ "tasks": self.tasks, "folders": self.folders, "ui": ui });
        if let Ok(text) = serde_json::to_string_pretty(&value) {
            write_atomic(&self.path, &text);
        }
    }

    fn new_id() -> String {
        uuid::Uuid::new_v4().simple().to_string()
    }

    /// Tasks directly inside `folder` (`None` = top level), in stored order.
    pub fn tasks_in(&self, folder: Option<&str>) -> Vec<&Task> {
        self.tasks.iter().filter(|t| t.folder.as_deref() == folder).collect()
    }

    /// Adds a task at the end of `folder` (`None` = top level). The folder is expanded so the
    /// new task is visible.
    pub fn add(&mut self, text: String, folder: Option<String>) {
        let folder = folder.filter(|id| self.folders.iter().any(|f| f.id == *id));
        if let Some(fid) = &folder {
            for f in self.folders.iter_mut().filter(|f| f.id == *fid) {
                f.collapsed = false;
            }
        }
        self.tasks.push(Task {
            id: Self::new_id(),
            text,
            done: false,
            folder,
        });
        self.save();
    }

    pub fn folder_exists(&self, id: &str) -> bool {
        self.folders.iter().any(|f| f.id == id)
    }

    pub fn folder_name(&self, id: &str) -> Option<&str> {
        self.folders.iter().find(|f| f.id == id).map(|f| f.name.as_str())
    }

    /// Number of tasks directly inside the folder.
    pub fn folder_task_count(&self, id: &str) -> usize {
        self.tasks.iter().filter(|t| t.folder.as_deref() == Some(id)).count()
    }

    pub fn expand_folder(&mut self, id: &str) {
        for f in self.folders.iter_mut().filter(|f| f.id == id) {
            f.collapsed = false;
        }
        self.save();
    }

    pub fn rename(&mut self, id: &str, text: String) {
        for t in self.tasks.iter_mut().filter(|t| t.id == id) {
            t.text = text.clone();
        }
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

    pub fn clear_done(&mut self) {
        self.tasks.retain(|t| !t.done);
        self.save();
    }

    // ---- folders ----

    /// Creates a folder (expanded, at the end of the folders) and returns its id.
    pub fn add_folder(&mut self, name: &str) -> String {
        let id = Self::new_id();
        self.folders.push(Folder {
            id: id.clone(),
            name: name.to_string(),
            collapsed: false,
        });
        self.save();
        id
    }

    pub fn rename_folder(&mut self, id: &str, name: String) {
        for f in self.folders.iter_mut().filter(|f| f.id == id) {
            f.name = name.clone();
        }
        self.save();
    }

    pub fn toggle_folder(&mut self, id: &str) {
        for f in self.folders.iter_mut().filter(|f| f.id == id) {
            f.collapsed = !f.collapsed;
        }
        self.save();
    }

    /// Deletes the folder together with the tasks inside it (the UI asks for confirmation first
    /// when the folder is not empty).
    pub fn delete_folder(&mut self, id: &str) {
        self.tasks.retain(|t| t.folder.as_deref() != Some(id));
        self.folders.retain(|f| f.id != id);
        self.save();
    }

    // ---- drag and drop ----

    /// Move a task: onto another task (takes its place and folder), onto a folder (goes
    /// inside, at the end) or to the end of the top level.
    pub fn move_task(&mut self, id: &str, target: &DropTarget) {
        let Some(from) = self.tasks.iter().position(|t| t.id == id) else {
            return;
        };
        match target {
            DropTarget::Task(tid) => {
                if tid == id {
                    return;
                }
                let Some(to) = self.tasks.iter().position(|t| t.id == *tid) else {
                    return;
                };
                let folder = self.tasks[to].folder.clone();
                let mut task = self.tasks.remove(from);
                task.folder = folder;
                // Moving down: the target shifted up by one, and we land after it (= `to`).
                // Moving up: we land before it (= `to`).
                self.tasks.insert(to, task);
            }
            DropTarget::Folder(fid) => {
                if !self.folders.iter().any(|f| f.id == *fid) {
                    return;
                }
                let mut task = self.tasks.remove(from);
                task.folder = Some(fid.clone());
                self.tasks.push(task);
                for f in self.folders.iter_mut().filter(|f| f.id == *fid) {
                    f.collapsed = false;
                }
            }
            DropTarget::RootEnd => {
                let mut task = self.tasks.remove(from);
                task.folder = None;
                self.tasks.push(task);
            }
        }
        self.save();
    }

    /// Reorder a folder: onto another folder (takes its place) or to the end.
    pub fn move_folder(&mut self, id: &str, target: &DropTarget) {
        let Some(from) = self.folders.iter().position(|f| f.id == id) else {
            return;
        };
        let to = match target {
            DropTarget::Folder(tid) => self.folders.iter().position(|f| f.id == *tid),
            DropTarget::Task(tid) => self
                .tasks
                .iter()
                .find(|t| t.id == *tid)
                .and_then(|t| t.folder.clone())
                .and_then(|fid| self.folders.iter().position(|f| f.id == fid)),
            DropTarget::RootEnd => Some(self.folders.len() - 1),
        };
        let Some(to) = to else { return };
        if to == from {
            return;
        }
        let folder = self.folders.remove(from);
        self.folders.insert(to, folder);
        self.save();
    }

    pub fn set_ui(&mut self, ui: SavedSize) {
        self.ui = ui;
        self.save();
    }
}