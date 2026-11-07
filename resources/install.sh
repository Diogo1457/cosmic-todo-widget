#!/usr/bin/env bash
# Installs the Todo Widget for the current user (no sudo, no build).
#   ./install.sh            install
#   ./install.sh uninstall  remove (keeps your tasks and settings)
#
# Used both by the release tarball (binary + resources/ next to this script) and by
# `just install` (which points RESOURCES_DIR and BIN_SRC at the source tree).
set -euo pipefail

app=cosmic-todo-widget
comm=cosmic-todo-wid            # process names are cut to 15 characters (pkill -x)
applet_comm=todo-widget-app     # "todo-widget-applet" cut to 15 characters
app_id=io.github.Diogo1457.CosmicTodoWidget

here="$(cd "$(dirname "$0")" && pwd)"
res="${RESOURCES_DIR:-$here/resources}"
src_bin="${BIN_SRC:-$here/$app}"

bin_dir="$HOME/.local/bin"
apps_dir="$HOME/.local/share/applications"
icons_dir="$HOME/.local/share/icons/hicolor/scalable/apps"
bin="$bin_dir/$app"
applet_bin="$bin_dir/todo-widget-applet"
applet_file="$apps_dir/$app_id.desktop"
launcher_file="$apps_dir/$app_id.Widget.desktop"
icon_file="$icons_dir/$app_id.svg"
autostart_file="$HOME/.config/autostart/todo-widget.desktop"

# Files left behind by older versions (before the app id and the resources folder).
legacy_files=(
    "$apps_dir/todo-widget.desktop"
    "$apps_dir/io.github.user.CosmicTodoWidget.desktop"
    "$icons_dir/io.github.user.CosmicTodoWidget.svg"
)

stop_running() {
    pkill -x "$comm" 2>/dev/null || true
    pkill -x "$applet_comm" 2>/dev/null || true
}

if [ "${1:-}" = "uninstall" ]; then
    stop_running
    rm -f "$bin" "$applet_bin" "$applet_file" "$launcher_file" "$icon_file" \
          "$autostart_file" "${legacy_files[@]}"
    [ -n "${QUIET:-}" ] || echo "Removed. Your tasks and settings were kept."
    exit 0
fi

[ -f "$src_bin" ] || { echo "Error: $src_bin not found." >&2; exit 1; }
for f in "$app_id.desktop" "$app_id.Widget.desktop" "$app_id.svg"; do
    [ -f "$res/$f" ] || { echo "Error: $res/$f not found." >&2; exit 1; }
done

stop_running
mkdir -p "$bin_dir" "$apps_dir" "$icons_dir"
install -m 755 "$src_bin" "$bin"
# The applet gets its own name so it can be stopped separately.
ln -sf "$bin" "$applet_bin"
install -m 644 "$res/$app_id.svg" "$icon_file"

# The desktop files in resources/ use the plain command name (that is what the Flatpak needs);
# here the Exec lines are pointed at the installed binary. The launcher restarts the widget
# instead of starting a second copy.
sed -E "s|^Exec=$app |Exec=$applet_bin |" "$res/$app_id.desktop" > "$applet_file"
sed -E "s|^Exec=$app( --settings)?\$|Exec=sh -c 'pkill -x $comm; exec $bin\\1'|" \
    "$res/$app_id.Widget.desktop" > "$launcher_file"
rm -f "${legacy_files[@]}"

echo "Installed to $bin_dir."
echo "Add the applet: COSMIC Settings > Desktop > Panel (or Dock) > Configure panel applets > Todo Widget."
echo "If it is not listed, log out and in again."