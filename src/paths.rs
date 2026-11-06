//! Filesystem locations and the small cross-process helpers (single-instance lock and the
//! "open your settings" request) shared by the rest of the program.
//!
//! Everything here is Flatpak-aware: inside a sandbox `$XDG_CONFIG_HOME` / `$XDG_DATA_HOME`
//! already point at the app's private directories, and the runtime directory shared by all
//! instances of the app is `$XDG_RUNTIME_DIR/app/$FLATPAK_ID`.

use std::{fs, path::{Path, PathBuf}, time::Duration};

/// Application id (also the name of the panel applet's `.desktop` file).
pub const APP_ID: &str = "io.github.user.CosmicTodoWidget";

/// Name of the per-app folder inside the config and data directories.
const DIR_NAME: &str = "todo-widget";

/// The Flatpak application id when running inside a Flatpak sandbox.
pub fn flatpak_id() -> Option<String> {
    std::env::var("FLATPAK_ID").ok().filter(|id| !id.is_empty())
}

/// Whether the desktop widget (a wlr-layer-shell surface) can work in this process.
///
/// COSMIC's compositor only offers layer-shell to clients without a Wayland security context,
/// and Flatpak always attaches one, so inside a Flatpak the widget can never appear. There the
/// list is only available as the panel applet (which cosmic-panel connects itself).
pub fn widget_supported() -> bool {
    flatpak_id().is_none()
}

/// Folder holding `settings.json`.
pub fn config_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join(DIR_NAME)
}

/// Folder holding `tasks.json`.
pub fn data_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join(DIR_NAME)
}

/// Directory for lock / request files. Must be the same for every process of this app.
fn runtime_dir() -> PathBuf {
    let base = dirs::runtime_dir().unwrap_or_else(std::env::temp_dir);
    match flatpak_id() {
        Some(id) => base.join("app").join(id),
        None => base,
    }
}

/// Writes `contents` to `path` through a temporary file so a crash never leaves a half-written
/// file behind. Failures are reported on stderr only (saving is best effort).
pub fn write_atomic(path: &Path, contents: &str) {
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("json.tmp");
    let result = fs::write(&tmp, contents).and_then(|()| fs::rename(&tmp, path));
    if let Err(err) = result {
        eprintln!("Could not save {}: {err}", path.display());
    }
}

// ------------------------------------------------------------------ single-instance lock

fn open_lock_file() -> Option<fs::File> {
    fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(runtime_dir().join("todo-widget.lock"))
        .ok()
}

/// Takes the "one desktop widget at a time" lock (held for as long as the returned file lives).
/// Retries for a few seconds so a restart (old instance still shutting down) works.
pub fn acquire_widget_lock() -> Option<fs::File> {
    let file = open_lock_file()?;
    for _ in 0..30 {
        if file.try_lock().is_ok() {
            return Some(file);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    None
}

/// True while another process holds the widget lock, i.e. the desktop widget is on screen.
pub fn widget_running() -> bool {
    // If the lock could be taken it is released again when the file is dropped.
    open_lock_file().is_some_and(|file| file.try_lock().is_err())
}

// ------------------------------------------------------------------ "open settings" request

fn settings_request_path() -> PathBuf {
    runtime_dir().join("todo-widget.open-settings")
}

/// Asks the running desktop widget to open its settings window (the widget polls for this).
pub fn request_settings() {
    if let Err(err) = fs::write(settings_request_path(), b"") {
        eprintln!("Could not ask the widget to open its settings: {err}");
    }
}

/// Consumes a pending settings request. Returns `true` if there was one.
pub fn take_settings_request() -> bool {
    fs::remove_file(settings_request_path()).is_ok()
}