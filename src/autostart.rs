//! "Start at login" is on exactly when the autostart `.desktop` file exists, so the file is the
//! single source of truth (nothing about it is stored in `settings.json`). Off by default.
//!
//! The file's content comes from `resources/autostart.desktop.in`; only `@EXEC@` is filled in.

use std::{fs, path::PathBuf};

use crate::paths;

/// Template of the autostart entry (`@EXEC@` is replaced by the command line).
const TEMPLATE: &str = include_str!("../resources/autostart.desktop.in");

/// Real `~/.config`. Inside a Flatpak `$XDG_CONFIG_HOME` is the app's private directory, so the
/// host's one is used instead (it needs the `xdg-config/autostart` permission).
fn host_config_dir() -> PathBuf {
    if paths::flatpak_id().is_some() {
        if let Some(dir) = std::env::var_os("HOST_XDG_CONFIG_HOME").filter(|d| !d.is_empty()) {
            return PathBuf::from(dir);
        }
        if let Some(home) = dirs::home_dir() {
            return home.join(".config");
        }
    }
    dirs::config_dir().unwrap_or_else(|| PathBuf::from("."))
}

fn path() -> PathBuf {
    host_config_dir().join("autostart").join("todo-widget.desktop")
}

pub fn is_enabled() -> bool {
    path().exists()
}

/// The `Exec=` line: `flatpak run` inside a Flatpak; otherwise the installed binary if there is
/// one (so autostart survives rebuilds of a dev copy), else the running executable.
fn exec_line() -> Option<String> {
    if let Some(id) = paths::flatpak_id() {
        return Some(format!("flatpak run {id} --background"));
    }
    let installed = dirs::executable_dir()
        .map(|d| d.join(env!("CARGO_PKG_NAME")))
        .filter(|p| p.is_file());
    let exe = installed.or_else(|| std::env::current_exe().ok())?.display().to_string();
    // Paths with spaces must be quoted in `Exec=`.
    let exe = if exe.contains(' ') {
        format!("\"{}\"", exe.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        exe
    };
    Some(format!("{exe} --background"))
}

/// Creates or removes the autostart entry.
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
    let Some(exec) = exec_line() else {
        eprintln!("Could not find the executable path; autostart not enabled");
        return;
    };
    let mut entry = TEMPLATE.replace("@EXEC@", &exec);
    if let Some(id) = paths::flatpak_id() {
        entry.push_str(&format!("X-Flatpak={id}\n"));
    }
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if let Err(err) = fs::write(&path, entry) {
        eprintln!("Could not save {}: {err}", path.display());
    }
}
