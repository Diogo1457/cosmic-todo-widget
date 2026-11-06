# COSMIC Todo Widget

A daily to-do list that lives on your desktop, like a sticky note. It is written in Rust with [libcosmic](https://github.com/pop-os/libcosmic) for the [COSMIC](https://system76.com/cosmic) desktop (Pop!_OS 24.04) and uses the Wayland layer-shell protocol, so it sits on a monitor as part of the desktop instead of being a normal window.

## Features

- Add, tick off, rename, delete and reorder tasks (drag the ≡ handle); pending tasks are listed first and completed ones go to the bottom.
- Folders: click a folder (or its row) to select it and new tasks are added straight into it; a new folder is selected automatically (✕ next to the entry field goes back to the top level). Drag tasks onto a folder to move them in. Deleting a folder that is not empty asks for confirmation, because its tasks are deleted with it.
- Right-click a task, a folder or the empty space for **New task** / **New folder**.
- Two display modes: a **desktop widget** (native install only) or a **panel applet** (popup with a fixed size and room for 5 tasks without scrolling). While the display is *Show as Widget*, clicking the applet in the panel opens the settings.
- Progress bar showing how many tasks are done.
- Move it by dragging the header, resize it with the ◢ handle in the bottom-right corner (double-click the handle for automatic height).
- Close it with the ✕ button in the header.
- Choose which monitor it appears on, whether it stays above windows or behind them, and its opacity.
- Transparency follows your COSMIC frosted-glass setting by default.
- Optionally start at login (off by default).
- Position, size, tasks and settings are remembered between runs.

## Requirements

- Pop!_OS 24.04 with COSMIC (or another Wayland compositor that supports layer-shell; Ubuntu 24.04 is also supported for building).
- Rust 1.93 or newer. The script installs it per-user with `rustup` if needed.
- `sudo` once, for the apt build dependencies.
- [`just`](https://github.com/casey/just), the command runner that drives the build and install (e.g. `sudo apt install just`, or see the [just manual](https://just.systems/man/en/packages.html) for other ways to install it).

## Install

Put the `justfile` next to `Cargo.toml`, then run this from that folder:

```bash
just install
```

This installs the build dependencies, Rust (if needed), builds the widget (the first build takes a few minutes), installs it to `~/.local/bin`, adds a launcher entry, and starts it.

The widget does not start at login by default. To change that, turn on **Start at login** in its Settings. If you close it with ✕, start it again from the app launcher ("Todo Widget") or with `just start`.

## Commands

Run `just` (or `just --list`) for the full list.

| Command | What it does |
|---|---|
| `install` | Full setup: dependencies, Rust, build, install, start |
| `update` | Rebuild, reinstall and restart (use this after changing the code) |
| `build` | Build only |
| `run` | Build and run in the foreground with backtraces (for debugging) |
| `deps` / `rust` | Install only the apt packages / only the Rust toolchain |
| `start` / `stop` / `restart` | Control the widget |
| `status` | Show what is installed and whether it is running |
| `settings` | Open the settings window |
| `flatpak-build` / `flatpak-run` | Build + install the Flatpak for your user / run it (see [Flatpak](#flatpak)) |
| `flatpak-bundle` / `flatpak-uninstall` | Make `target/todo-widget.flatpak` / remove the Flatpak |
| `uninstall` | Remove the app and its autostart entry; keeps your tasks and settings |
| `remove` | Uninstall and delete your tasks and settings |
| `clean` | Delete the `target/` build folder |
| `nuke` | Remove everything, including the build folder and cached libcosmic sources |

Options are `just` variables, written before the command name:

```bash
just yes=1 remove        # skip the confirmation prompt (remove, nuke)
just no_start=1 update   # don't start the widget afterwards (install, update)
```

## Running without installing

```bash
cargo run --release                  # as a desktop widget (display mode from the settings)
cargo run --release -- --settings    # open the settings window on start
```

## Settings

Click **Settings** on the widget (or run `cosmic-todo-widget --settings`):

- **Display**: *Show on Panel* (panel applet popup, the default) or *Show as Widget* (desktop widget). Everything works in both; moving and resizing only exist for the widget, and so do Monitor, Keep above windows and Opacity.
- **Monitor**: Auto (the second monitor if there is one, otherwise the only one) or a specific monitor.
- **Keep above windows**: off means it is part of the desktop and windows cover it.
- **Opacity**: *Use system transparency* (default) follows the frosted-glass setting of COSMIC; turn it off to set your own value, 30–100 %. Applies to the desktop widget only. The settings window and the panel popup always use the system values (the settings window is opaque when maximized, like other COSMIC apps).
- **Start at login**: off by default. Turning it on creates an autostart entry for the current user; turning it off removes it.
- **Reset**: restore the default settings (start at login goes back to off), position and size. Tasks and the display choice are not touched.

## Using the panel

1. Run `just install` (it also installs the applet entry). *Show on Panel* is the default.
2. Add the **Todo Widget** applet: COSMIC Settings → Desktop → Panel (or Dock) → Configure panel applets.
3. Click its icon in the panel to open the list.

While the display is set to *Show as Widget*, clicking the applet's panel icon opens the settings window (in the running widget; if the widget is not running it is started together with the settings). Settings opened from the applet in panel mode appear in their own window.

## Flatpak

Needs `flatpak` and `flatpak-builder` (`sudo apt install flatpak flatpak-builder`, plus the Flathub remote). Then:

```bash
just flatpak-build     # generates flatpak/cargo-sources.json, builds, installs for your user
just flatpak-run       # start it
```

The first build downloads the runtime, the Rust SDK extension and all crates, and takes a while. Afterwards log out and in (or restart the panel) so COSMIC finds the exported applet, then add **Todo Widget** in COSMIC Settings → Desktop → Panel → Configure panel applets. Inside the Flatpak your data lives in `~/.var/app/io.github.user.CosmicTodoWidget/` (copy `config/todo-widget` and `data/todo-widget` there to keep an existing list). 

**Limitation:** the desktop widget does not work in the Flatpak. COSMIC's compositor only offers `wlr-layer-shell` to clients without a Wayland security context, and Flatpak always attaches one, so a sandboxed app cannot draw on the desktop (no permission can change that). The Flatpak therefore always uses *Show on Panel*; the Display choice and "Start at login" are hidden. For the desktop widget use the native install (`just install`).

## Where your data is stored

| File | Contents |
|---|---|
| `~/.config/todo-widget/settings.json` | Settings |
| `~/.local/share/todo-widget/tasks.json` | Tasks, position and size |
| `~/.config/autostart/todo-widget.desktop` | Exists only while **Start at login** is on |

The first two are plain JSON. `uninstall` leaves them alone; `remove` and `nuke` delete them. `uninstall` always removes the autostart entry.

## Project layout

| File | Purpose |
|---|---|
| `src/main.rs` | Command line and start-up |
| `src/app/` | The libcosmic application: `mod.rs` (state, lifecycle), `message.rs`, `update.rs` (event handling), `view/` (list, card + popup, settings window), `style.rs`, `geometry.rs`, `monitors.rs`, `surface.rs` (layer-shell surface) |
| `src/settings.rs` | Settings file |
| `src/store.rs` | Tasks, folders, saved position and size |
| `src/autostart.rs` | The "Start at login" entry |
| `src/paths.rs` | Directories, single-instance lock, settings request between applet and widget |
| `flatpak/` | Flatpak manifest, desktop files, metainfo, icon |
| `Cargo.toml` / `Cargo.lock` | Dependencies (libcosmic is pinned to a specific commit) |
| `justfile` | Build, install and management commands (`just --list`) |

## License

GPL-3.0-only