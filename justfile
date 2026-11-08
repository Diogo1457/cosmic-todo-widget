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
# panel applet: a symlink with its own name so it can be stopped separately (comm is 15 chars max)
applet_bin        := bin_dir + "/todo-widget-applet"
applet_comm       := "todo-widget-app"

# Files shipped with the app live in resources/ (desktop entries, icon, metainfo, installer).
# The desktop entries are installed under the app id, so the launcher and panel find them.
app_id            := "io.github.Diogo1457.CosmicTodoWidget"
res_dir           := src + "/resources"
installer         := res_dir + "/install.sh"
applet_file       := apps_dir + "/" + app_id + ".desktop"
launcher_file     := apps_dir + "/" + app_id + ".Widget.desktop"
icon_file         := home + "/.local/share/icons/hicolor/scalable/apps/" + app_id + ".svg"
version           := `sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1`

# Flatpak
flatpak_id       := app_id
flatpak_dir      := src + "/flatpak"   # manifest + cargo-sources.json
flatpak_manifest := flatpak_dir + "/" + flatpak_id + ".yml"

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
    echo "Icon:          {{icon_file}} ($(yes_no "{{icon_file}}"))"
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
_install-files: _check-built stop _stop-applet
    @echo "==> Installing"
    @RESOURCES_DIR="{{res_dir}}" BIN_SRC="{{src}}/target/release/{{app}}" bash "{{installer}}"

[private]
_remove-app: stop _stop-applet
    @echo "==> Removing {{app}}"
    @QUIET=1 bash "{{installer}}" uninstall

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

# ----------------------------------------------------------------------------- development

# format the code
fmt:
    cargo fmt --all

# run the tests
test: rust
    cd "{{src}}" && cargo test --locked

# everything CI checks: formatting, lints, tests and the install script
check: rust
    #!/usr/bin/env bash
    set -euo pipefail
    cd "{{src}}"
    if command -v rustup >/dev/null 2>&1; then rustup component add rustfmt clippy; fi
    cargo fmt --all --check
    cargo clippy --locked --all-targets -- -D warnings
    cargo test --locked
    bash -n "{{installer}}"
    if command -v shellcheck >/dev/null 2>&1; then shellcheck "{{installer}}"; else echo "    (shellcheck not installed - skipped)"; fi
    echo "==> All checks passed"

# set a new version in Cargo.toml, Cargo.lock and the metainfo, e.g. `just bump 0.2.0`
bump new:
    #!/usr/bin/env bash
    set -euo pipefail
    [[ "{{new}}" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "Error: use a version like 0.2.0" >&2; exit 1; }
    cd "{{src}}"
    meta="{{res_dir}}/{{app_id}}.metainfo.xml"
    grep -q "<release version=\"{{new}}\"" "$meta" && { echo "Error: {{new}} is already in the metainfo" >&2; exit 1; }
    sed -i '0,/^version = ".*"/s//version = "{{new}}"/' Cargo.toml
    sed -i "s|<releases>|<releases>\n    <release version=\"{{new}}\" date=\"$(date +%F)\"/>|" "$meta"
    cargo update --workspace
    echo "==> Version is now {{new}} (Cargo.toml, Cargo.lock, metainfo). Commit it, then run: just release"

# check everything, then create and push the tag v<version>: GitHub builds and publishes the release
release: check
    #!/usr/bin/env bash
    set -euo pipefail
    cd "{{src}}"
    tag="v{{version}}"
    [ -z "$(git status --porcelain)" ] || { echo "Error: commit your changes first" >&2; exit 1; }
    if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
        echo "Error: the tag $tag already exists - run 'just bump <new version>' first" >&2
        exit 1
    fi
    git push
    git tag -a "$tag" -m "$tag"
    git push origin "$tag"
    echo "==> Pushed $tag. Follow the build with: gh run watch"

# ----------------------------------------------------------------------------- release

# build the release tarball target/dist/<name>.tar.gz (GitHub runs this for releases; use it to try it locally)
dist: build
    #!/usr/bin/env bash
    set -euo pipefail
    name="{{app}}-v{{version}}-{{arch()}}-{{os()}}"
    out="{{src}}/target/dist"
    rm -rf "$out"
    mkdir -p "$out/$name/resources"
    install -m 755 "{{src}}/target/release/{{app}}" "$out/$name/{{app}}"
    install -m 755 "{{installer}}" "$out/$name/install.sh"
    install -m 644 "{{res_dir}}/{{app_id}}".desktop "{{res_dir}}/{{app_id}}".Widget.desktop \
        "{{res_dir}}/{{app_id}}".svg "$out/$name/resources/"
    install -m 644 "{{src}}/LICENSE" "{{src}}/README.md" "$out/$name/"
    tar -C "$out" -czf "$out/$name.tar.gz" "$name"
    echo "==> $out/$name.tar.gz"

# ----------------------------------------------------------------------------- flatpak

# generate flatpak/cargo-sources.json from Cargo.lock (re-run after changing dependencies)
flatpak-sources:
    #!/usr/bin/env bash
    set -euo pipefail
    tools="{{src}}/target/flatpak-builder-tools"
    venv="{{src}}/target/flatpak-venv"
    mkdir -p "{{src}}/target"
    [ -d "$tools" ] || git clone --depth 1 https://github.com/flatpak/flatpak-builder-tools "$tools"
    [ -d "$venv" ] || python3 -m venv "$venv"
    "$venv/bin/pip" install --quiet aiohttp tomlkit
    "$venv/bin/python" "$tools/cargo/flatpak-cargo-generator.py" "{{src}}/Cargo.lock" -o "{{flatpak_dir}}/cargo-sources.json"

# build and install the Flatpak for the current user (needs flatpak and flatpak-builder)
flatpak-build: flatpak-sources
    cd "{{src}}" && flatpak-builder --user --install --install-deps-from=flathub --force-clean \
        --state-dir="{{src}}/target/flatpak-state" "{{src}}/target/flatpak-build" "{{flatpak_manifest}}"

# run the installed Flatpak (the desktop widget / settings, depending on your Display setting)
flatpak-run:
    flatpak run {{flatpak_id}}

# build a single-file bundle: target/todo-widget.flatpak
flatpak-bundle: flatpak-sources
    cd "{{src}}" && flatpak-builder --force-clean --repo="{{src}}/target/flatpak-repo" \
        --state-dir="{{src}}/target/flatpak-state" "{{src}}/target/flatpak-build" "{{flatpak_manifest}}"
    flatpak build-bundle --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo "{{src}}/target/flatpak-repo" "{{src}}/target/todo-widget.flatpak" {{flatpak_id}}
    @echo "==> {{src}}/target/todo-widget.flatpak"

# remove the Flatpak (add --delete-data via flatpak itself if you also want its tasks gone)
flatpak-uninstall:
    flatpak uninstall --user -y {{flatpak_id}}
