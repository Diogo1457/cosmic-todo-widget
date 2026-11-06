//! Tasks and folders, stored in `<data dir>/todo-widget/tasks.json` together with the position
//! and size the widget was left with.
//!
//! Old files stay readable: unknown keys are ignored. Tasks are never removed automatically:
//! only the user deletes them (a task, "Clear done", or a folder together with its tasks after
//! a confirmation).

use std::{fs, path::PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::paths;

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

/// All tasks and folders. Every mutating method saves to disk immediately.
pub struct Store {
    path: PathBuf,
    pub tasks: Vec<Task>,
    pub folders: Vec<Folder>,
    pub ui: SavedSize,
}

impl Store {
    fn path() -> PathBuf {
        paths::data_dir().join("tasks.json")
    }

    /// Loads the store. A file that cannot be parsed is copied to `tasks.json.corrupt` and never
    /// overwritten, so tasks are not lost silently.
    pub fn load() -> Self {
        let mut store = Store {
            path: Self::path(),
            tasks: Vec::new(),
            folders: Vec::new(),
            ui: SavedSize::default(),
        };

        let mut readable = true;
        let data = match fs::read_to_string(&store.path) {
            Ok(text) => match serde_json::from_str::<Value>(&text) {
                Ok(v) => v,
                Err(err) => {
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
            store.ui = SavedSize {
                x: get("x"),
                y: get("y"),
                width: get("width"),
                list_h: get("list_h"),
            };
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
            paths::write_atomic(&self.path, &text);
        }
    }

    fn new_id() -> String {
        uuid::Uuid::new_v4().simple().to_string()
    }

    // ---- queries ----

    /// Tasks directly inside `folder` (`None` = top level), in stored order.
    pub fn tasks_in(&self, folder: Option<&str>) -> Vec<&Task> {
        self.tasks.iter().filter(|t| t.folder.as_deref() == folder).collect()
    }

    pub fn folder_exists(&self, id: &str) -> bool {
        self.folders.iter().any(|f| f.id == id)
    }

    pub fn folder_name(&self, id: &str) -> Option<&str> {
        self.folders.iter().find(|f| f.id == id).map(|f| f.name.as_str())
    }

    /// `(done, total)` over all tasks.
    pub fn progress(&self) -> (usize, usize) {
        (self.tasks.iter().filter(|t| t.done).count(), self.tasks.len())
    }

    /// `(done, total)` over the tasks directly inside a folder.
    pub fn folder_progress(&self, id: &str) -> (usize, usize) {
        let inside = self.tasks_in(Some(id));
        (inside.iter().filter(|t| t.done).count(), inside.len())
    }

    fn folder_mut(&mut self, id: &str) -> Option<&mut Folder> {
        self.folders.iter_mut().find(|f| f.id == id)
    }

    // ---- tasks ----

    /// Adds a task at the end of `folder` (`None` = top level). An unknown folder id means top
    /// level; the folder is expanded so the new task is visible.
    pub fn add(&mut self, text: String, folder: Option<String>) {
        let folder = folder.filter(|id| self.folder_exists(id));
        if let Some(f) = folder.as_deref().and_then(|id| self.folder_mut(id)) {
            f.collapsed = false;
        }
        self.tasks.push(Task { id: Self::new_id(), text, done: false, folder });
        self.save();
    }

    pub fn rename(&mut self, id: &str, text: String) {
        if let Some(t) = self.tasks.iter_mut().find(|t| t.id == id) {
            t.text = text;
        }
        self.save();
    }

    pub fn toggle(&mut self, id: &str, done: bool) {
        if let Some(t) = self.tasks.iter_mut().find(|t| t.id == id) {
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

    /// Creates a folder (expanded, after the existing ones) and returns its id.
    pub fn add_folder(&mut self, name: &str) -> String {
        let id = Self::new_id();
        self.folders.push(Folder { id: id.clone(), name: name.to_string(), collapsed: false });
        self.save();
        id
    }

    pub fn rename_folder(&mut self, id: &str, name: String) {
        if let Some(f) = self.folder_mut(id) {
            f.name = name;
        }
        self.save();
    }

    pub fn toggle_folder(&mut self, id: &str) {
        if let Some(f) = self.folder_mut(id) {
            f.collapsed = !f.collapsed;
        }
        self.save();
    }

    pub fn expand_folder(&mut self, id: &str) {
        if let Some(f) = self.folder_mut(id) {
            f.collapsed = false;
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

    /// Moves a task: onto another task (takes its place and folder), onto a folder (goes inside,
    /// at the end) or to the end of the top level.
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
                if !self.folder_exists(fid) {
                    return;
                }
                let mut task = self.tasks.remove(from);
                task.folder = Some(fid.clone());
                self.tasks.push(task);
                if let Some(f) = self.folder_mut(fid) {
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

    /// Reorders a folder: onto another folder (takes its place) or to the end.
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

    // ---- widget position / size ----

    pub fn set_ui(&mut self, ui: SavedSize) {
        self.ui = ui;
        self.save();
    }
}
