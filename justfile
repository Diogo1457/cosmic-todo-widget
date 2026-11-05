# Task runner for the COSMIC Todo Widget: build, install, update, start/stop,
# uninstall, nuke...  Run `just` (or `just --list`) to see the recipes.
#
# "Start at login" is off by default; turn it on in the widget's Settings window.
# The list can be shown as a desktop widget or in the COSMIC panel (Settings > Display).
# For the panel, add the "Todo Widget" applet in COSMIC Settings > Desktop > Panel.
#
# Options are plain just variables, set before the recipe name:
#   just yes=1 remove          don't ask for confirmation
#   just no_start=1 update     (install/update) don't start the widget afterwards

set shell := ["bash", "-euo", "pipefail", "-c"]

# ----------------------------------------------------------------------------- settings

app      := "cosmic-todo-widget"
# process names are truncated to 15 chars (needed by pkill -x / pgrep -x)
comm     := "cosmic-todo-wid"
min_rust := "1.93"
src      := justfile_directory()
home     := home_directory()

bin_dir           := home + "/.local/bin"
apps_dir          := home + "/.local/share/applications"
data_dir          := home + "/.local/share/todo-widget"   # tasks.json + position/size
config_dir        := home + "/.config/todo-widget"        # settings.json
bin               := bin_dir + "/" + app
autostart_file    := home + "/.config/autostart/todo-widget.desktop"   # created by the Settings toggle
launcher_file     := apps_dir + "/todo-widget.desktop"
# panel applet: a symlink with its own name so it can be stopped separately (comm is 15 chars max)
applet_bin        := bin_dir + "/todo-widget-applet"
applet_comm       := "todo-widget-app"
applet_file       := apps_dir + "/io.github.user.CosmicTodoWidget.desktop"
old_settings_file := apps_dir + "/todo-widget-settings.desktop"   # installed by older versions

# Options (override on the command line, e.g. `just yes=1 remove`)
yes      := "0"
no_start := "0"

sudo := if `id -u` == "0" { "" } else { "sudo" }

# rustup installs into ~/.cargo/bin
export PATH := home + "/.cargo/bin:" + env("PATH")
export CARGO_NET_GIT_FETCH_WITH_CLI := "true"

# ----------------------------------------------------------------------------- help

# list the recipes
default:
    @just --justfile "{{justfile()}}" --list --unsorted

# ----------------------------------------------------------------------------- build & install

# full setup: apt deps + Rust + build + install + start (autostart stays off)
install: deps build _install-files _done (_start no_start)

# rebuild and reinstall (no apt), then restart
update: build _install-files _done (_start no_start)

# build only (cargo build --release)
build: rust
    #!/usr/bin/env bash
    set -euo pipefail
    echo "==> Building (the first build takes a few minutes)"
    lock_flag=""
    if [ -f "{{src}}/Cargo.lock" ]; then
        lock_flag="--locked"
    else
        echo "    (no Cargo.lock found - resolving dependencies; one will be created)"
    fi
    cd "{{src}}"
    cargo build --release $lock_flag

# build and run in the foreground with backtraces (for debugging)
run: build stop
    @echo "==> Running in the foreground (Ctrl+C to quit)"
    RUST_BACKTRACE=1 "{{src}}/target/release/{{app}}"

# install the apt build dependencies (needs sudo)
deps:
    @echo "==> Installing build dependencies with apt (needs sudo)"
    {{sudo}} apt update
    {{sudo}} apt install -y build-essential pkg-config cmake git curl \
        libxkbcommon-dev libwayland-dev libfontconfig-dev libfreetype-dev \
        libinput-dev libudev-dev libseat-dev libgbm-dev libegl1-mesa-dev libssl-dev

# install / update the Rust toolchain (rustup, per-user)
rust:
    #!/usr/bin/env bash
    set -euo pipefail
    min="{{min_rust}}"
    rust_ok() {
        command -v rustc >/dev/null 2>&1 || return 1
        local have; have="$(rustc --version | awk '{print $2}')"
        [ "$(printf '%s\n%s\n' "$min" "$have" | sort -V | head -n1)" = "$min" ]
    }
    if ! rust_ok; then
        if command -v rustup >/dev/null 2>&1; then
            echo "==> Updating Rust with rustup"
            rustup update stable
        else
            echo "==> Installing Rust with rustup (per-user, no sudo)"
            curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
        fi
    fi
    rust_ok || { echo "Error: Rust >= $min is required but could not be set up." >&2; exit 1; }
    echo "    using $(rustc --version)"

# ----------------------------------------------------------------------------- control

# start the widget
start: (_start "0")

# stop the widget (and settings window)
stop:
    @pkill -x {{comm}} 2>/dev/null || true

# stop the panel applet (cosmic-panel starts it again)
[private]
_stop-applet:
    @pkill -x {{applet_comm}} 2>/dev/null || true

# stop + start
restart: stop _pause (_start "0")

# open the settings window
settings: stop _pause
    #!/usr/bin/env bash
    set -euo pipefail
    [ -x "{{bin}}" ] || { echo "Error: Not installed - run: just install" >&2; exit 1; }
    setsid nohup "{{bin}}" --settings >/dev/null 2>&1 </dev/null &

# show what is installed and whether it is running
status:
    #!/usr/bin/env bash
    yes_no() { if [ -e "$1" ]; then echo yes; else echo no; fi; }
    echo "Binary:        {{bin}} ($(yes_no "{{bin}}"))"
    echo "Launcher:      {{launcher_file}} ($(yes_no "{{launcher_file}}"))"
    echo "Panel applet:  {{applet_file}} ($(yes_no "{{applet_file}}"))"
    echo "Autostart:     {{autostart_file}} ($(yes_no "{{autostart_file}}"))"
    echo "Tasks/UI:      {{data_dir}} ($(yes_no "{{data_dir}}"))"
    echo "Settings:      {{config_dir}} ($(yes_no "{{config_dir}}"))"
    echo "Build folder:  {{src}}/target ($(yes_no "{{src}}/target"))"
    if pgrep -x "{{comm}}" >/dev/null 2>&1; then
        echo "Running:       yes (pid $(pgrep -x "{{comm}}" | head -n1))"
    else
        echo "Running:       no"
    fi
    if pgrep -x "{{applet_comm}}" >/dev/null 2>&1; then
        echo "Panel applet:  running"
    else
        echo "Panel applet:  not running (add it in COSMIC Settings > Desktop > Panel)"
    fi

# ----------------------------------------------------------------------------- removal

# remove the app (binary, launcher, autostart entry); keeps tasks and settings
uninstall: _remove-app
    @echo "    Your tasks ({{data_dir}}) and settings ({{config_dir}}) were kept."

# uninstall + delete tasks and settings
remove: (_confirm "Remove the app AND delete all tasks and settings?") _remove-app _delete-data

# delete the build folder (target/)
clean:
    @echo "==> Deleting the build folder"
    @rm -rf "{{src}}/target"

# remove + clean + delete the cached libcosmic sources (asks first)
nuke: _nuke-info (_confirm "Really nuke everything?") _remove-app _delete-data clean _delete-cache

# ----------------------------------------------------------------------------- internals

[private]
_done:
    @echo "==> Done."

[private]
_pause:
    @sleep 0.3

[private]
_start skip:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ "{{skip}}" = "1" ]; then exit 0; fi
    [ -x "{{bin}}" ] || { echo "Error: Not installed - run: just install" >&2; exit 1; }
    if pgrep -x "{{comm}}" >/dev/null 2>&1; then
        echo "    Already running."
        exit 0
    fi
    if [ -z "${WAYLAND_DISPLAY:-}" ]; then
        echo "    No Wayland session detected here; it will start at your next login."
        exit 0
    fi
    echo "==> Starting the widget"
    setsid nohup "{{bin}}" --background >/dev/null 2>&1 </dev/null &

[private]
_check-built:
    @[ -x "{{src}}/target/release/{{app}}" ] || { echo "Error: Nothing built yet - run: just build" >&2; exit 1; }

[private]
_install-files: _check-built stop _stop-applet _write-launcher
    #!/usr/bin/env bash
    set -euo pipefail
    echo "==> Installing"
    pkill -f '^python3? .*todo_widget\.py' 2>/dev/null || true     # old Python version, if running
    rm -f "{{data_dir}}/todo_widget.py"
    mkdir -p "{{bin_dir}}"
    install -m 755 "{{src}}/target/release/{{app}}" "{{bin}}"
    ln -sf "{{bin}}" "{{applet_bin}}"

[private]
_write-launcher:
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p "{{apps_dir}}"
    rm -f "{{old_settings_file}}"
    cat > "{{applet_file}}" <<EOF
    [Desktop Entry]
    Type=Application
    Name=Todo Widget
    Comment=Daily to-do list in the panel
    Exec={{applet_bin}} --applet
    Icon=view-list-symbolic
    Terminal=false
    Categories=COSMIC;
    NoDisplay=true
    StartupNotify=false
    X-CosmicApplet=true
    X-CosmicHoverPopup=Auto
    X-OverflowPriority=10
    EOF
    cat > "{{launcher_file}}" <<EOF
    [Desktop Entry]
    Type=Application
    Name=Todo Widget
    Comment=Daily to-do list widget
    Exec=sh -c 'pkill -x {{comm}}; exec {{bin}}'
    Icon=view-list-symbolic
    Terminal=false
    Categories=Utility;
    Actions=Settings;

    [Desktop Action Settings]
    Name=Settings
    Exec=sh -c 'pkill -x {{comm}}; exec {{bin}} --settings'
    EOF

[private]
_remove-app: stop _stop-applet
    @echo "==> Removing {{app}}"
    @rm -f "{{bin}}" "{{applet_bin}}" "{{applet_file}}" "{{autostart_file}}" "{{launcher_file}}" "{{old_settings_file}}"

[private]
_delete-data:
    @echo "==> Deleting tasks and settings"
    @rm -rf "{{data_dir}}" "{{config_dir}}"

[private]
_delete-cache:
    @echo "==> Deleting cached libcosmic sources"
    @rm -rf "{{home}}"/.cargo/git/checkouts/libcosmic-* "{{home}}"/.cargo/git/db/libcosmic-*
    @echo "    Left alone: apt packages and Rust. Remove Rust with: rustup self uninstall"

[private]
_nuke-info:
    @echo "This will:"
    @echo "  - stop and remove the app, launcher and autostart"
    @echo "  - delete ALL tasks and settings"
    @echo "  - delete the build folder ({{src}}/target)"
    @echo "  - delete the cached libcosmic sources in ~/.cargo/git"

# Ask for confirmation (skipped with yes=1). Cancelling stops the whole run quietly.
[private]
[no-exit-message]
_confirm prompt:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ "{{yes}}" = "1" ]; then exit 0; fi
    read -r -p "{{prompt}} [y/N] " reply || { echo; echo "Cancelled."; exit 1; }
    case "$reply" in
        [yY]|[yY][eE][sS]) ;;
        *) echo "Cancelled."; exit 1 ;;
    esac
