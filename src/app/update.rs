//! What every [`Message`] does. `handle` is the single entry point; the longer cases are
//! separate methods below it.

use std::time::{Duration, Instant};

use chrono::Local;
use cosmic::app::Task;
use cosmic::cctk::sctk::reexports::client::protocol::wl_output::WlOutput;
use cosmic::iced::event::wayland;
use cosmic::iced::platform_specific::shell::commands::layer_surface::destroy_layer_surface;
use cosmic::iced::platform_specific::shell::commands::popup::get_popup;
use cosmic::iced::{window, Limits, Point};

use super::geometry::POPUP_W;
use super::message::{ContextTarget, Drag, DragKind, ItemDrag, Message};
use super::monitors::Monitor;
use super::style::system_alpha;
use super::{spawn_child, App};
use crate::settings::{DisplayMode, Settings, MIN_OPACITY};
use crate::store::{DropTarget, ItemRef, SavedSize, Store};
use crate::{autostart, paths};

/// Messages that happen in the background and must neither confirm a rename in progress nor
/// close the right-click menu.
fn is_background(message: &Message) -> bool {
    matches!(
        message,
        Message::Noop
            | Message::Tick
            | Message::CheckRequests
            | Message::Output(..)
            | Message::OutputsSettled(_)
            | Message::SurfaceLost
            | Message::Pointer(_)
            | Message::WindowState(..)
    )
}

impl App {
    pub(super) fn handle(&mut self, message: Message) -> Task<Message> {
        // Any action other than typing in / cancelling the rename box confirms the rename
        // (clicking elsewhere, another button, starting a drag, ...).
        let keeps_rename = is_background(&message)
            || matches!(
                message,
                Message::Input(_) | Message::EditInput(_) | Message::EditCancel
            );
        if !keeps_rename {
            self.commit_edit();
        }
        // Anything but background events closes the right-click menu and a pending folder delete
        // confirmation (the menu's target is kept for "New task").
        let transient = is_background(&message) || matches!(message, Message::Context(_));
        let menu_target = if transient { None } else { self.context.take() };
        if !transient {
            self.confirm_folder = None;
        }

        match message {
            Message::Noop => {}
            Message::Tick => {
                self.now = Local::now();
                if self.args.applet {
                    return self.sync_applet();
                }
            }
            Message::CheckRequests => {
                // The panel applet asked this widget to open its settings window.
                if paths::take_settings_request() {
                    return self.open_settings();
                }
            }
            Message::WindowState(id, maximized) => {
                if Some(id) == self.settings_id {
                    self.settings_maximized = maximized;
                }
            }

            // ----- monitors / surface -----
            Message::Output(ev, wl) => return self.on_output(ev, wl),
            Message::OutputsSettled(generation) => return self.on_outputs_settled(generation),
            Message::SurfaceLost => {
                self.surface_id = None;
                self.surface_failures += 1;
                if self.surface_failures <= 5 {
                    return self.settle_after(self.generation, 1000);
                }
                eprintln!("The compositor keeps closing the widget surface; giving up.");
            }

            // ----- tasks -----
            Message::Input(text) => self.entry = text,
            Message::Add => {
                let text = self.entry.trim().to_string();
                if !text.is_empty() {
                    // Goes into the selected folder, or to the top level without one.
                    let folder = self.target_folder();
                    self.store.add(text, folder);
                    self.entry.clear();
                }
            }
            Message::Toggle(id, done) => self.store.toggle(&id, done),
            Message::Delete(id) => self.store.delete(&id),
            Message::ClearDone => self.store.clear_done(),

            // ----- rename -----
            Message::EditStart(item) => return self.start_edit(item),
            Message::EditInput(text) => {
                if let Some((_, buf)) = self.editing.as_mut() {
                    *buf = text;
                }
            }
            // Enter / ✓ / click elsewhere: the rename was already applied above.
            Message::EditCommit | Message::EditClickAway => {}
            Message::EditCancel => self.editing = None,

            // ----- folders -----
            Message::AddFolder | Message::CtxNewFolder => {
                // The new folder is selected, so the next task typed goes straight into it.
                let id = self.store.add_folder("New folder");
                self.selected_folder = Some(id.clone());
                self.editing = Some((ItemRef::Folder(id), "New folder".to_string()));
                return Self::focus_edit();
            }
            Message::ToggleFolder(id) => self.store.toggle_folder(&id),
            Message::SelectFolder(id) => self.selected_folder = Some(id),
            Message::ClearTarget => self.selected_folder = None,
            Message::DeleteFolder(id) => {
                self.editing = None;
                if self.store.folder_progress(&id).1 > 0 {
                    // Not empty: ask before deleting the tasks inside.
                    self.confirm_folder = Some(id);
                } else {
                    self.remove_folder(&id);
                }
            }
            Message::ConfirmDeleteFolder(id) => {
                self.editing = None;
                self.remove_folder(&id);
            }
            Message::CancelDeleteFolder => {}

            // ----- right-click menu -----
            Message::Context(target) => self.context = Some(target),
            Message::CtxNewTask => return self.new_task_from_menu(menu_target),

            // ----- drag and drop inside the list -----
            Message::DragStart(item) => {
                self.editing = None;
                self.item_drag = Some(ItemDrag { item, over: None });
            }
            Message::DragOver(target) => self.drag_over(target),
            Message::DragLeave(target) => {
                if let Some(d) = self.item_drag.as_mut() {
                    if d.over.as_ref() == Some(&target) {
                        d.over = None;
                    }
                }
            }
            Message::ItemDrop => {
                if let Some(ItemDrag {
                    item,
                    over: Some(target),
                }) = self.item_drag.take()
                {
                    match item {
                        ItemRef::Task(id) => self.store.move_task(&id, &target),
                        ItemRef::Folder(id) => self.store.move_folder(&id, &target),
                    }
                }
            }

            // ----- settings window -----
            Message::OpenSettings => {
                self.confirm_restore = false;
                if self.args.applet {
                    // The applet has no room for a normal window: use a separate process.
                    spawn_child(&["--settings"], &mut self.settings_child);
                    return self.close_popup();
                }
                return self.open_settings();
            }
            Message::CloseSettings => {
                self.confirm_restore = false;
                self.settings_maximized = false;
                if self.settings.display == DisplayMode::Panel {
                    // Nothing else is shown by this process while the list lives in the panel.
                    std::process::exit(0);
                }
                if let Some(id) = self.settings_id.take() {
                    return window::close(id);
                }
            }

            // ----- panel applet -----
            Message::AppletPressed => return self.applet_pressed(),
            Message::PopupClosed(id) => {
                if self.popup == Some(id) {
                    self.popup = None;
                    self.reset_list_state();
                }
            }

            // ----- settings -----
            Message::SetDisplay(mode) => return self.set_display(mode),
            Message::SetClock(v) => {
                self.settings.show_clock = v;
                return self.apply_setting_change(false);
            }
            Message::SetOpacitySystem(system) => {
                // Switching to a custom value starts from what the system uses right now.
                self.settings.opacity = if system {
                    None
                } else {
                    Some(system_alpha().clamp(MIN_OPACITY, 1.0))
                };
                return self.apply_setting_change(false);
            }
            Message::SetOpacity(pct) => {
                self.settings.opacity = Some((f32::from(pct) / 100.0).clamp(MIN_OPACITY, 1.0));
                return self.apply_setting_change(false);
            }
            Message::SetMonitor(id) => {
                self.settings.monitor = id;
                return self.apply_setting_change(true);
            }
            Message::SetAbove(v) => {
                self.settings.above_windows = v;
                return self.apply_setting_change(true);
            }
            Message::SetAutostart(v) => {
                autostart::set(v);
                self.settings.autostart = autostart::is_enabled();
            }
            Message::AskRestore => self.confirm_restore = true,
            Message::CancelRestore => self.confirm_restore = false,
            Message::RestoreDefaults => return self.restore_defaults(),

            // ----- move / resize of the desktop widget -----
            Message::MovePress => self.begin_drag(DragKind::Move),
            Message::GripPress => {
                self.begin_drag(DragKind::Resize);
                return self.grow_surface();
            }
            Message::GripRelease => {
                if let Some(d) = self.drag.take() {
                    if let (DragKind::Resize, Some(start)) = (d.kind, d.start) {
                        self.apply_resize(d.w0, d.h0, start, d.last);
                    }
                    self.clamp_pos();
                    self.persist_ui();
                }
            }
            Message::GripAuto => {
                self.drag = None;
                self.manual_list_h = None;
                self.clamp_pos();
                self.persist_ui();
                return self.apply_margin();
            }
            Message::Pointer(p) => return self.pointer_moved(p),
            Message::Quit => {
                self.persist_ui();
                std::process::exit(0);
            }
        }
        Task::none()
    }

    // ---------- monitors ----------

    fn on_output(&mut self, ev: wayland::OutputEvent, wl: WlOutput) -> Task<Message> {
        use wayland::OutputEvent as E;
        match ev {
            E::Created(Some(info)) | E::InfoUpdate(info) => {
                let mon = Monitor::from_info(wl.clone(), &info);
                match self.monitors.iter_mut().find(|m| m.wl == wl) {
                    Some(slot) => *slot = mon,
                    None => self.monitors.push(mon),
                }
            }
            E::Created(None) => return Task::none(),
            E::Removed => self.monitors.retain(|m| m.wl != wl),
        }
        // Debounce: several events arrive together when (un)plugging.
        self.generation += 1;
        self.settle_after(self.generation, 600)
    }

    /// The monitors stopped changing: create the surface, or rebuild it if the wanted monitor
    /// or its size changed.
    fn on_outputs_settled(&mut self, generation: u64) -> Task<Message> {
        if generation != self.generation || !self.widget_mode() {
            return Task::none();
        }
        let target = self.resolve_monitor();
        let moved = self.surface_id.is_none()
            || target.as_ref().map(|m| &m.wl) != self.surface_output.as_ref()
            || target
                .as_ref()
                .is_some_and(|m| m.width as f32 != self.mon_w || m.height as f32 != self.mon_h);
        if !moved {
            return Task::none();
        }
        if self.surface_id.is_none() {
            if !self.ensure_lock() {
                // Another widget is already on the desktop.
                if self.settings_id.is_none() {
                    std::process::exit(0);
                }
                return Task::none();
            }
            return self.create_surface(true);
        }
        self.rebuild_surface()
    }

    // ---------- list ----------

    fn start_edit(&mut self, item: ItemRef) -> Task<Message> {
        let text = match &item {
            ItemRef::Task(id) => self
                .store
                .tasks
                .iter()
                .find(|t| t.id == *id)
                .map(|t| t.text.clone()),
            ItemRef::Folder(id) => self
                .store
                .folders
                .iter()
                .find(|f| f.id == *id)
                .map(|f| f.name.clone()),
        };
        let Some(text) = text else {
            return Task::none();
        };
        self.editing = Some((item, text));
        Self::focus_edit()
    }

    /// "+ New task" in the right-click menu: the task goes where the click was, i.e. inside the
    /// folder that was clicked (or that contains the clicked task), otherwise at the top level.
    fn new_task_from_menu(&mut self, menu_target: Option<ContextTarget>) -> Task<Message> {
        let folder = match menu_target {
            Some(ContextTarget::Folder(id)) => Some(id),
            Some(ContextTarget::Task(id)) => self
                .store
                .tasks
                .iter()
                .find(|t| t.id == id)
                .and_then(|t| t.folder.clone()),
            _ => None,
        };
        if let Some(fid) = &folder {
            self.store.expand_folder(fid);
        }
        self.selected_folder = folder;
        Self::focus_entry()
    }

    /// Tracks what the dragged item is over, ignoring targets that make no sense.
    fn drag_over(&mut self, target: DropTarget) {
        let Some(d) = self.item_drag.as_mut() else {
            return;
        };
        let skip = match (&d.item, &target) {
            // Not onto itself.
            (ItemRef::Task(a), DropTarget::Task(b))
            | (ItemRef::Folder(a), DropTarget::Folder(b)) => a == b,
            // Folders are reordered on other folders (or the bottom zone) only.
            (ItemRef::Folder(_), DropTarget::Task(_)) => true,
            _ => false,
        };
        if !skip {
            d.over = Some(target);
        }
    }

    // ---------- panel applet ----------

    /// The panel icon was clicked.
    /// * display "Widget": open the settings window (in the running widget, or in a freshly
    ///   started one, which also shows the widget);
    /// * display "Panel": toggle the popup with the list.
    fn applet_pressed(&mut self) -> Task<Message> {
        // Always start from what is on disk (the widget or settings window may have changed it).
        self.settings = Settings::load();

        if self.settings.display == DisplayMode::Widget {
            if paths::widget_running() {
                paths::request_settings();
            } else {
                spawn_child(&["--settings"], &mut self.widget_child);
            }
            return self.close_popup();
        }

        if self.popup.is_some() {
            return self.close_popup();
        }
        self.store = Store::load();
        self.reset_list_state();
        let Some(main_id) = self.core.main_window_id() else {
            return Task::none();
        };
        let new_id = window::Id::unique();
        self.popup = Some(new_id);
        let mut popup = self
            .core
            .applet
            .get_popup_settings(main_id, new_id, None, None, None);
        popup.positioner.size_limits = Limits::NONE
            .min_width(POPUP_W)
            .max_width(POPUP_W)
            .min_height(120.0)
            .max_height(1000.0);
        get_popup::<cosmic::Action<Message>>(popup)
    }

    // ---------- settings ----------

    fn set_display(&mut self, mode: DisplayMode) -> Task<Message> {
        if self.settings.display == mode
            || (mode == DisplayMode::Widget && !paths::widget_supported())
        {
            return Task::none();
        }
        self.settings.display = mode;
        self.settings.save();
        self.confirm_restore = false;
        match mode {
            DisplayMode::Panel => {
                // Remove the desktop widget; the panel applet takes over.
                self.drag = None;
                self.lock = None;
                if let Some(id) = self.surface_id.take() {
                    self.surface_output = None;
                    return destroy_layer_surface::<cosmic::Action<Message>>(id);
                }
                Task::none()
            }
            DisplayMode::Widget => {
                self.generation += 1;
                self.settle_after(self.generation, 0)
            }
        }
    }

    /// Resets settings, position and size (tasks and the display choice are kept).
    fn restore_defaults(&mut self) -> Task<Message> {
        let display = self.settings.display;
        self.settings = Settings {
            display,
            ..Settings::default()
        };
        self.settings.save();
        autostart::set(self.settings.autostart);
        self.confirm_restore = false;
        self.drag = None;
        self.manual_list_h = None;
        self.cur_w = self.default_w();
        self.pos = self.default_pos();
        self.store.set_ui(SavedSize::default());
        if self.widget_mode() {
            self.rebuild_surface()
        } else {
            Task::none()
        }
    }

    // ---------- move / resize ----------

    fn begin_drag(&mut self, kind: DragKind) {
        self.drag = Some(Drag {
            kind,
            start: None,
            w0: self.cur_w,
            h0: self.list_h(),
            last_apply: Instant::now() - Duration::from_secs(1),
            last: Point::ORIGIN,
        });
    }

    /// Minimum time between two move requests sent to the compositor.
    const MOVE_INTERVAL: Duration = Duration::from_millis(30);

    fn pointer_moved(&mut self, p: Point) -> Task<Message> {
        let Some(d) = self.drag.as_mut() else {
            return Task::none();
        };
        d.last = p;
        let Some(start) = d.start else {
            d.start = Some(p);
            return Task::none();
        };
        match d.kind {
            // The card is anchored top-left, so the pointer position relative to the surface does
            // not shift while resizing, and the surface is already full size (see
            // `grow_surface`), so every event is applied directly.
            DragKind::Resize => {
                let (w0, h0) = (d.w0, d.h0);
                self.apply_resize(w0, h0, start, p);
                Task::none()
            }
            // Moving shifts the surface under the pointer, so send at most one margin update per
            // interval and move by the offset from the grab point.
            DragKind::Move => {
                if d.last_apply.elapsed() < Self::MOVE_INTERVAL {
                    return Task::none();
                }
                let (dx, dy) = (p.x - start.x, p.y - start.y);
                if dx.abs() < 1.0 && dy.abs() < 1.0 {
                    return Task::none();
                }
                d.last_apply = Instant::now();
                self.pos = Point::new(self.pos.x + dx, self.pos.y + dy);
                self.clamp_pos();
                self.apply_margin()
            }
        }
    }
}
