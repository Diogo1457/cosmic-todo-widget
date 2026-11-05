# COSMIC Todo Widget

A daily to-do list that lives on your desktop, like a sticky note. It is written in Rust with [libcosmic](https://github.com/pop-os/libcosmic) for the [COSMIC](https://system76.com/cosmic) desktop (Pop!_OS 24.04) and uses the Wayland layer-shell protocol, so it sits on a monitor as part of the desktop instead of being a normal window.

## Features

- Add, tick off, delete and reorder (▲ / ▼) tasks; pending tasks are listed first and completed ones go to the bottom.
- Two display modes: a **desktop widget** or a **panel applet** (popup with a fixed size and room for 5 tasks without scrolling).
- Progress bar showing how many tasks are done.
- Clock and date in the header (12 or 24-hour, optional seconds).
- Fresh every day: completed tasks from yesterday are removed when the date changes. Unfinished tasks carry over.
- Move it by dragging the header, resize it with the ◢ handle in the bottom-right corner (double-click the handle for automatic height).
- Close it with the ✕ button in the header.
- Choose which monitor it appears on, whether it stays above windows or behind them, and its opacity.
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
- **Clock**: 24-hour or 12-hour, show seconds.
- **Monitor**: Auto (the second monitor if there is one, otherwise the only one) or a specific monitor.
- **Keep above windows**: off means it is part of the desktop and windows cover it.
- **Opacity**: 30–100 %.
- **Start at login**: off by default. Turning it on creates an autostart entry for the current user; turning it off removes it.
- **Reset**: restore the default settings (start at login goes back to off), position and size. Tasks and the display choice are not touched.

## Using the panel

1. Run `just install` (it also installs the applet entry). *Show on Panel* is the default.
2. Add the **Todo Widget** applet: COSMIC Settings → Desktop → Panel (or Dock) → Configure panel applets.
3. Click its icon in the panel to open the list.

While the display is set to *Show as Widget*, the applet's panel icon simply starts the widget. Settings opened from the applet appear in their own window.

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
| `src/main.rs` | The widget and the panel applet: layer-shell surface, popup, UI, settings window, move/resize |
| `src/config.rs` | Settings, task storage and the autostart entry |
| `Cargo.toml` / `Cargo.lock` | Dependencies (libcosmic is pinned to a specific commit) |
| `justfile` | Build, install and management commands (`just --list`) |

## License

GPL-3.0-only