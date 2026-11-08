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
    host_config_dir()
        .join("autostart")
        .join("todo-widget.desktop")
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
    let exe = installed
        .or_else(|| std::env::current_exe().ok())?
        .display()
        .to_string();
    Some(format!("{} --background", quote_exec(&exe)))
}

/// Paths with spaces must be quoted in `Exec=`.
fn quote_exec(exe: &str) -> String {
    if exe.contains(' ') {
        format!("\"{}\"", exe.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        exe.to_string()
    }
}

/// The content of the autostart file: the template with the command line filled in (and, in a
/// Flatpak, the app it belongs to).
fn render_entry(exec: &str, flatpak_id: Option<&str>) -> String {
    let mut entry = TEMPLATE.replace("@EXEC@", exec);
    if let Some(id) = flatpak_id {
        entry.push_str(&format!("X-Flatpak={id}\n"));
    }
    entry
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
    let entry = render_entry(&exec, paths::flatpak_id().as_deref());
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if let Err(err) = fs::write(&path, entry) {
        eprintln!("Could not save {}: {err}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_template_has_exactly_one_placeholder() {
        assert_eq!(TEMPLATE.matches("@EXEC@").count(), 1);
        assert!(TEMPLATE.starts_with("[Desktop Entry]"));
    }

    #[test]
    fn the_command_line_is_filled_in() {
        let entry = render_entry("/home/me/.local/bin/cosmic-todo-widget --background", None);
        assert!(!entry.contains('@'));
        assert!(entry.contains("\nExec=/home/me/.local/bin/cosmic-todo-widget --background\n"));
        assert!(entry.contains("X-GNOME-Autostart-enabled=true"));
        assert!(!entry.contains("X-Flatpak"));
        assert!(entry.ends_with('\n'));
    }

    #[test]
    fn inside_a_flatpak_the_entry_names_the_app() {
        let entry = render_entry("flatpak run my.App --background", Some("my.App"));
        assert!(entry.ends_with("X-Flatpak=my.App\n"));
    }

    #[test]
    fn paths_with_spaces_are_quoted() {
        assert_eq!(quote_exec("/usr/bin/app"), "/usr/bin/app");
        assert_eq!(quote_exec("/my apps/app"), "\"/my apps/app\"");
        assert_eq!(quote_exec("/my \"apps\"/app"), "\"/my \\\"apps\\\"/app\"");
    }
}
