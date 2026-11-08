## Install

**Flatpak** (panel applet only)

```
flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
flatpak install --user todo-widget.flatpak
```

Then add *Todo Widget* in COSMIC Settings → Desktop → Panel → Configure panel applets.

**Binary** (x86_64 Linux, desktop widget + panel applet)

```
tar -xzf cosmic-todo-widget-@TAG@-x86_64-linux.tar.gz
cd cosmic-todo-widget-@TAG@-x86_64-linux
./install.sh
```

Then add *Todo Widget* in the same panel settings. `./install.sh uninstall` removes it and keeps your tasks and settings.

The desktop widget only works with the binary, because COSMIC does not allow sandboxed apps to use layer-shell.

---

