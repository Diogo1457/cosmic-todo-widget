//! Tasks and folders, stored in `<data dir>/todo-widget/tasks.json` together with the position
//! and size the widget was left with.
//!
//! Old files stay readable: unknown keys are ignored. Tasks are never removed automatically:
//! only the user deletes them (a task, "Clear done", or a folder together with its tasks after
//! a confirmation).

use std::{fs, path::PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

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
        Self::load_from(Self::path())
    }

    /// [`Store::load`] for an explicit file (so tests do not touch the real one).
    fn load_from(path: PathBuf) -> Self {
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
        self.tasks
            .iter()
            .filter(|t| t.folder.as_deref() == folder)
            .collect()
    }

    pub fn folder_exists(&self, id: &str) -> bool {
        self.folders.iter().any(|f| f.id == id)
    }

    pub fn folder_name(&self, id: &str) -> Option<&str> {
        self.folders
            .iter()
            .find(|f| f.id == id)
            .map(|f| f.name.as_str())
    }

    /// `(done, total)` over all tasks.
    pub fn progress(&self) -> (usize, usize) {
        (
            self.tasks.iter().filter(|t| t.done).count(),
            self.tasks.len(),
        )
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
        self.tasks.push(Task {
            id: Self::new_id(),
            text,
            done: false,
            folder,
        });
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
        self.folders.push(Folder {
            id: id.clone(),
            name: name.to_string(),
            collapsed: false,
        });
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::TestDir;

    fn empty() -> (Store, TestDir) {
        let dir = TestDir::new();
        let store = Store::load_from(dir.file("tasks.json"));
        (store, dir)
    }

    fn texts(store: &Store, folder: Option<&str>) -> Vec<String> {
        store
            .tasks_in(folder)
            .iter()
            .map(|t| t.text.clone())
            .collect()
    }

    fn id_of(store: &Store, text: &str) -> String {
        store
            .tasks
            .iter()
            .find(|t| t.text == text)
            .expect("no such task")
            .id
            .clone()
    }

    /// A store with the top-level tasks a, b, c.
    fn abc() -> (Store, TestDir) {
        let (mut store, dir) = empty();
        for t in ["a", "b", "c"] {
            store.add(t.into(), None);
        }
        (store, dir)
    }

    // ---- tasks ----

    #[test]
    fn new_tasks_are_added_in_order() {
        let (store, _dir) = abc();
        assert_eq!(texts(&store, None), ["a", "b", "c"]);
        assert!(store.tasks.iter().all(|t| !t.done && t.folder.is_none()));
    }

    #[test]
    fn task_ids_are_unique() {
        let (store, _dir) = abc();
        let mut ids: Vec<_> = store.tasks.iter().map(|t| t.id.clone()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 3);
    }

    #[test]
    fn rename_toggle_and_delete() {
        let (mut store, _dir) = abc();
        let b = id_of(&store, "b");
        store.rename(&b, "bee".into());
        store.toggle(&b, true);
        assert_eq!(store.tasks[1].text, "bee");
        assert!(store.tasks[1].done);
        store.toggle(&b, false);
        assert!(!store.tasks[1].done);
        store.delete(&b);
        assert_eq!(texts(&store, None), ["a", "c"]);
    }

    #[test]
    fn clear_done_only_removes_finished_tasks() {
        let (mut store, _dir) = abc();
        let (a, c) = (id_of(&store, "a"), id_of(&store, "c"));
        store.toggle(&a, true);
        store.toggle(&c, true);
        store.clear_done();
        assert_eq!(texts(&store, None), ["b"]);
    }

    #[test]
    fn progress_counts_done_and_total() {
        let (mut store, _dir) = abc();
        assert_eq!(store.progress(), (0, 3));
        let a = id_of(&store, "a");
        store.toggle(&a, true);
        assert_eq!(store.progress(), (1, 3));
    }

    #[test]
    fn operations_on_unknown_ids_change_nothing() {
        let (mut store, _dir) = abc();
        store.rename("nope", "x".into());
        store.toggle("nope", true);
        store.delete("nope");
        store.rename_folder("nope", "x".into());
        store.delete_folder("nope");
        assert_eq!(texts(&store, None), ["a", "b", "c"]);
        assert!(store.folders.is_empty());
    }

    // ---- folders ----

    #[test]
    fn adding_into_a_folder_expands_it() {
        let (mut store, _dir) = empty();
        let f = store.add_folder("Work");
        store.toggle_folder(&f);
        assert!(store.folders[0].collapsed);
        store.add("report".into(), Some(f.clone()));
        assert!(!store.folders[0].collapsed);
        assert_eq!(texts(&store, Some(&f)), ["report"]);
        assert!(texts(&store, None).is_empty());
    }

    #[test]
    fn adding_into_an_unknown_folder_goes_to_the_top_level() {
        let (mut store, _dir) = empty();
        store.add("x".into(), Some("missing".into()));
        assert_eq!(texts(&store, None), ["x"]);
    }

    #[test]
    fn folder_helpers() {
        let (mut store, _dir) = empty();
        let f = store.add_folder("Home");
        assert!(store.folder_exists(&f));
        assert!(!store.folder_exists("other"));
        assert_eq!(store.folder_name(&f), Some("Home"));
        store.rename_folder(&f, "House".into());
        assert_eq!(store.folder_name(&f), Some("House"));
        store.toggle_folder(&f);
        assert!(store.folders[0].collapsed);
        store.expand_folder(&f);
        assert!(!store.folders[0].collapsed);
    }

    #[test]
    fn folder_progress_only_counts_its_own_tasks() {
        let (mut store, _dir) = empty();
        let f = store.add_folder("F");
        store.add("in1".into(), Some(f.clone()));
        store.add("in2".into(), Some(f.clone()));
        store.add("out".into(), None);
        let in1 = id_of(&store, "in1");
        store.toggle(&in1, true);
        assert_eq!(store.folder_progress(&f), (1, 2));
        assert_eq!(store.progress(), (1, 3));
    }

    #[test]
    fn deleting_a_folder_deletes_its_tasks() {
        let (mut store, _dir) = empty();
        let f = store.add_folder("F");
        store.add("inside".into(), Some(f.clone()));
        store.add("outside".into(), None);
        store.delete_folder(&f);
        assert!(store.folders.is_empty());
        assert_eq!(texts(&store, None), ["outside"]);
        assert_eq!(store.tasks.len(), 1);
    }

    // ---- drag and drop ----

    #[test]
    fn move_task_down_lands_after_the_target() {
        let (mut store, _dir) = abc();
        let (a, c) = (id_of(&store, "a"), id_of(&store, "c"));
        store.move_task(&a, &DropTarget::Task(c));
        assert_eq!(texts(&store, None), ["b", "c", "a"]);
    }

    #[test]
    fn move_task_up_lands_before_the_target() {
        let (mut store, _dir) = abc();
        let (a, c) = (id_of(&store, "a"), id_of(&store, "c"));
        store.move_task(&c, &DropTarget::Task(a));
        assert_eq!(texts(&store, None), ["c", "a", "b"]);
    }

    #[test]
    fn dropping_a_task_on_itself_changes_nothing() {
        let (mut store, _dir) = abc();
        let b = id_of(&store, "b");
        store.move_task(&b, &DropTarget::Task(b.clone()));
        assert_eq!(texts(&store, None), ["a", "b", "c"]);
    }

    #[test]
    fn dropping_a_task_on_a_folder_puts_it_inside_at_the_end() {
        let (mut store, _dir) = empty();
        let f = store.add_folder("F");
        store.add("first".into(), Some(f.clone()));
        store.add("loose".into(), None);
        store.toggle_folder(&f);
        let loose = id_of(&store, "loose");
        store.move_task(&loose, &DropTarget::Folder(f.clone()));
        assert_eq!(texts(&store, Some(&f)), ["first", "loose"]);
        assert!(texts(&store, None).is_empty());
        assert!(
            !store.folders[0].collapsed,
            "the folder opens so the task is visible"
        );
    }

    #[test]
    fn dropping_a_task_on_a_task_in_a_folder_joins_that_folder() {
        let (mut store, _dir) = empty();
        let f = store.add_folder("F");
        store.add("inside".into(), Some(f.clone()));
        store.add("loose".into(), None);
        let (inside, loose) = (id_of(&store, "inside"), id_of(&store, "loose"));
        store.move_task(&loose, &DropTarget::Task(inside));
        assert_eq!(texts(&store, Some(&f)).len(), 2);
        assert!(texts(&store, None).is_empty());
    }

    #[test]
    fn dropping_a_task_on_the_bottom_zone_takes_it_out_of_its_folder() {
        let (mut store, _dir) = empty();
        let f = store.add_folder("F");
        store.add("inside".into(), Some(f));
        store.add("other".into(), None);
        let inside = id_of(&store, "inside");
        store.move_task(&inside, &DropTarget::RootEnd);
        assert_eq!(texts(&store, None), ["other", "inside"]);
    }

    #[test]
    fn moving_to_an_unknown_target_changes_nothing() {
        let (mut store, _dir) = abc();
        let a = id_of(&store, "a");
        store.move_task(&a, &DropTarget::Folder("missing".into()));
        store.move_task(&a, &DropTarget::Task("missing".into()));
        store.move_task("missing", &DropTarget::RootEnd);
        assert_eq!(texts(&store, None), ["a", "b", "c"]);
    }

    fn folder_names(store: &Store) -> Vec<&str> {
        store.folders.iter().map(|f| f.name.as_str()).collect()
    }

    fn three_folders() -> (Store, TestDir, [String; 3]) {
        let (mut store, dir) = empty();
        let ids = [
            store.add_folder("one"),
            store.add_folder("two"),
            store.add_folder("three"),
        ];
        (store, dir, ids)
    }

    #[test]
    fn folders_can_be_reordered() {
        let (mut store, _dir, [one, _two, three]) = three_folders();
        store.move_folder(&one, &DropTarget::Folder(three.clone()));
        assert_eq!(folder_names(&store), ["two", "three", "one"]);
        store.move_folder(&three, &DropTarget::Folder(one.clone()));
        assert_eq!(folder_names(&store), ["two", "one", "three"]);
    }

    #[test]
    fn a_folder_dropped_on_the_bottom_zone_goes_last() {
        let (mut store, _dir, [one, ..]) = three_folders();
        store.move_folder(&one, &DropTarget::RootEnd);
        assert_eq!(folder_names(&store), ["two", "three", "one"]);
    }

    #[test]
    fn a_folder_dropped_on_a_task_takes_the_place_of_that_tasks_folder() {
        let (mut store, _dir, [one, _two, three]) = three_folders();
        store.add("t".into(), Some(three));
        let t = id_of(&store, "t");
        store.move_folder(&one, &DropTarget::Task(t));
        assert_eq!(folder_names(&store), ["two", "three", "one"]);
    }

    #[test]
    fn a_folder_dropped_on_a_top_level_task_stays_where_it_is() {
        let (mut store, _dir, [one, ..]) = three_folders();
        store.add("loose".into(), None);
        let loose = id_of(&store, "loose");
        store.move_folder(&one, &DropTarget::Task(loose));
        assert_eq!(folder_names(&store), ["one", "two", "three"]);
    }

    // ---- file ----

    #[test]
    fn everything_survives_a_save_and_load() {
        let dir = TestDir::new();
        let file = dir.file("tasks.json");
        let mut store = Store::load_from(file.clone());
        let f = store.add_folder("Work");
        store.add("top".into(), None);
        store.add("inside".into(), Some(f.clone()));
        let top = id_of(&store, "top");
        store.toggle(&top, true);
        store.toggle_folder(&f);
        store.set_ui(SavedSize {
            x: Some(40.0),
            y: Some(60.0),
            width: Some(500.0),
            list_h: None,
        });

        let loaded = Store::load_from(file);
        assert_eq!(loaded.tasks.len(), 2);
        assert!(loaded.tasks.iter().find(|t| t.text == "top").unwrap().done);
        assert_eq!(texts(&loaded, Some(&f)), ["inside"]);
        assert_eq!(loaded.folders.len(), 1);
        assert_eq!(loaded.folders[0].name, "Work");
        assert!(loaded.folders[0].collapsed);
        assert_eq!(loaded.ui, store.ui);
    }

    #[test]
    fn position_and_size_are_stored_in_whole_pixels() {
        let dir = TestDir::new();
        let file = dir.file("tasks.json");
        let mut store = Store::load_from(file.clone());
        store.set_ui(SavedSize {
            x: Some(10.4),
            y: Some(20.6),
            width: Some(400.0),
            list_h: Some(250.0),
        });
        let ui = Store::load_from(file).ui;
        assert_eq!((ui.x, ui.y), (Some(10.0), Some(21.0)));
        assert_eq!((ui.width, ui.list_h), (Some(400.0), Some(250.0)));
    }

    #[test]
    fn top_level_tasks_have_no_folder_key_in_the_file() {
        let (mut store, dir) = empty();
        store.add("x".into(), None);
        let text = fs::read_to_string(dir.file("tasks.json")).unwrap();
        assert!(!text.contains("\"folder\""));
    }

    #[test]
    fn a_missing_file_gives_an_empty_store() {
        let (store, _dir) = empty();
        assert!(store.tasks.is_empty() && store.folders.is_empty());
        assert_eq!(store.ui, SavedSize::default());
    }

    #[test]
    fn a_task_in_a_folder_that_no_longer_exists_goes_to_the_top_level() {
        let dir = TestDir::new();
        let file = dir.file("tasks.json");
        fs::write(
            &file,
            r#"{"tasks":[{"id":"1","text":"a","folder":"gone"}],"folders":[]}"#,
        )
        .unwrap();
        let store = Store::load_from(file);
        assert_eq!(store.tasks[0].folder, None);
    }

    #[test]
    fn old_files_with_unknown_keys_and_missing_fields_still_load() {
        let dir = TestDir::new();
        let file = dir.file("tasks.json");
        fs::write(
            &file,
            r#"{"future":true,
                "tasks":[{"id":"1","text":"a","extra":5},{"id":"2"}],
                "folders":[{"id":"f","name":"F"}]}"#,
        )
        .unwrap();
        let store = Store::load_from(file);
        assert_eq!(
            store.tasks.len(),
            1,
            "the broken entry is skipped, the good one kept"
        );
        assert!(!store.tasks[0].done);
        assert!(!store.folders[0].collapsed);
    }

    #[test]
    fn a_corrupt_file_is_backed_up_and_never_overwritten() {
        let dir = TestDir::new();
        let file = dir.file("tasks.json");
        fs::write(&file, "this is not json {").unwrap();
        let mut store = Store::load_from(file.clone());
        assert!(store.tasks.is_empty());
        let backup = dir.file("tasks.json.corrupt");
        assert_eq!(fs::read_to_string(&backup).unwrap(), "this is not json {");
        // Loading alone does not replace the unreadable file ...
        assert_eq!(fs::read_to_string(&file).unwrap(), "this is not json {");
        // ... only the user's next change does, and the copy stays available.
        store.add("new".into(), None);
        assert!(fs::read_to_string(&file).unwrap().contains("new"));
        assert_eq!(fs::read_to_string(&backup).unwrap(), "this is not json {");
    }
}
