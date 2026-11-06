//! The application: its state, its lifecycle (`Application` impl) and the modules its behaviour
//! is split over.
//!
//! * `message`  – every event the UI or the system can send
//! * `update`   – what each event does
//! * `view`     – the widgets (list, card, panel popup, settings window)
//! * `style`    – looks of the containers
//! * `geometry` – size / position maths of the desktop widget
//! * `monitors` – the connected monitors and which one is used
//! * `surface`  – the layer-shell surface and the settings window

mod geometry;
mod message;
mod monitors;
mod style;
mod surface;
mod update;
mod view;

use std::process::{Child, Command, Stdio};
use std::sync::LazyLock;
use std::time::Duration;

use chrono::{DateTime, Local};
use cosmic::app::{Core, Task};
use cosmic::cctk::sctk::reexports::client::protocol::wl_output::WlOutput;
use cosmic::cctk::sctk::reexports::csd_frame::WindowState;
use cosmic::iced::event::{self, PlatformSpecific, wayland};
use cosmic::iced::platform_specific::shell::commands::popup::destroy_popup;
use cosmic::iced::{self, Color, Event, Point, Subscription, mouse, window};
use cosmic::widget;
use cosmic::{Application, Element, executor};

use crate::paths;
use crate::settings::{DisplayMode, Settings};
use crate::store::{ItemRef, Store};
use crate::Args;
use message::{ContextTarget, Drag, ItemDrag, Message};
use monitors::Monitor;

static AUTOSIZE_ID: LazyLock<iced::id::Id> = LazyLock::new(|| iced::id::Id::new("todo-autosize"));
static ENTRY_ID: LazyLock<iced::id::Id> = LazyLock::new(|| iced::id::Id::new("todo-entry"));
static EDIT_ID: LazyLock<iced::id::Id> = LazyLock::new(|| iced::id::Id::new("todo-edit"));

/// Application state. One process shows either the desktop widget (plus its settings window) or
/// the panel applet; `args.applet` tells which.
pub(crate) struct App {
    core: Core,
    args: Args,
    settings: Settings,
    store: Store,
    /// Current time (updated by `Tick`, only used for the optional clock).
    now: DateTime<Local>,

    // ---- monitors and windows ----
    monitors: Vec<Monitor>,
    /// Bumped on every monitor event; lets the debounce timer ignore stale firings.
    generation: u64,
    /// The desktop widget's layer surface.
    surface_id: Option<window::Id>,
    surface_output: Option<WlOutput>,
    /// How often the compositor closed the surface in a row.
    surface_failures: u32,
    settings_id: Option<window::Id>,
    /// Panel applet popup.
    popup: Option<window::Id>,
    /// Held while the desktop widget exists (only one at a time).
    lock: Option<std::fs::File>,
    /// Settings window started by the applet (a separate process).
    settings_child: Option<Child>,
    /// Widget (with settings) started by the applet (a separate process).
    widget_child: Option<Child>,

    // ---- geometry derived from the target monitor ----
    mon_w: f32,
    mon_h: f32,
    /// Top-left corner of the card relative to the monitor (logical px).
    pos: Point,
    cur_w: f32,
    /// `None` = automatic (follows the number of tasks).
    manual_list_h: Option<f32>,
    /// Move / resize in progress.
    drag: Option<Drag>,

    // ---- list state ----
    /// Text typed into the "add a task" field.
    entry: String,
    /// The task or folder being renamed, with the text typed so far.
    editing: Option<(ItemRef, String)>,
    /// A task or folder being dragged inside the list.
    item_drag: Option<ItemDrag>,
    /// Folder new tasks are added to (`None` = top level).
    selected_folder: Option<String>,
    /// Folder waiting for the "delete it and its tasks?" confirmation.
    confirm_folder: Option<String>,
    /// Open right-click menu.
    context: Option<ContextTarget>,

    // ---- settings window state ----
    /// The settings window is maximized / fullscreen.
    settings_maximized: bool,
    confirm_restore: bool,
}

/// Starts another copy of this program unless the previous one is still running.
fn spawn_child(args: &[&str], slot: &mut Option<Child>) {
    if let Some(child) = slot.as_mut() {
        if matches!(child.try_wait(), Ok(None)) {
            return;
        }
    }
    let Ok(exe) = std::env::current_exe() else { return };
    match Command::new(exe)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => *slot = Some(child),
        Err(err) => eprintln!("Could not start {}: {err}", env!("CARGO_PKG_NAME")),
    }
}

impl App {
    /// True when this process shows the desktop widget.
    fn widget_mode(&self) -> bool {
        !self.args.applet && self.settings.display == DisplayMode::Widget
    }

    /// Takes the "one desktop widget at a time" lock if this process does not have it yet.
    fn ensure_lock(&mut self) -> bool {
        if self.lock.is_none() {
            self.lock = paths::acquire_widget_lock();
            if self.lock.is_none() {
                eprintln!("Another Todo Widget is already running.");
            }
        }
        self.lock.is_some()
    }

    /// Forgets everything transient in the list (rename, drags, menu, pending confirmation).
    fn reset_list_state(&mut self) {
        self.editing = None;
        self.item_drag = None;
        self.context = None;
        self.confirm_folder = None;
    }

    fn close_popup(&mut self) -> Task<Message> {
        self.reset_list_state();
        match self.popup.take() {
            Some(id) => destroy_popup::<cosmic::Action<Message>>(id),
            None => Task::none(),
        }
    }

    /// Applet only: picks up settings changed in the settings window (a separate process).
    fn sync_applet(&mut self) -> Task<Message> {
        let latest = Settings::load();
        if latest == self.settings {
            return Task::none();
        }
        self.settings = latest;
        if self.settings.display != DisplayMode::Panel {
            return self.close_popup();
        }
        Task::none()
    }

    /// Applies the rename in progress (an empty name keeps the old one).
    fn commit_edit(&mut self) {
        if let Some((item, text)) = self.editing.take() {
            let text = text.trim().to_string();
            if !text.is_empty() {
                match item {
                    ItemRef::Task(id) => self.store.rename(&id, text),
                    ItemRef::Folder(id) => self.store.rename_folder(&id, text),
                }
            }
        }
    }

    fn focus_edit() -> Task<Message> {
        widget::text_input::focus::<cosmic::Action<Message>>(EDIT_ID.clone())
    }

    fn focus_entry() -> Task<Message> {
        widget::text_input::focus::<cosmic::Action<Message>>(ENTRY_ID.clone())
    }

    /// The selected folder, if it still exists.
    fn target_folder(&self) -> Option<String> {
        self.selected_folder.clone().filter(|id| self.store.folder_exists(id))
    }

    /// Deletes a folder with its tasks and forgets it as the target.
    fn remove_folder(&mut self, id: &str) {
        if self.selected_folder.as_deref() == Some(id) {
            self.selected_folder = None;
        }
        self.store.delete_folder(id);
    }
}

impl Application for App {
    type Executor = executor::Default;
    type Flags = Args;
    type Message = Message;
    const APP_ID: &'static str = paths::APP_ID;

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, args: Args) -> (Self, Task<Message>) {
        let mut app = App {
            core,
            args,
            settings: Settings::load(),
            store: Store::load(),
            now: Local::now(),
            monitors: Vec::new(),
            generation: 0,
            surface_id: None,
            surface_output: None,
            surface_failures: 0,
            settings_id: None,
            popup: None,
            lock: None,
            settings_child: None,
            widget_child: None,
            mon_w: 1920.0,
            mon_h: 1080.0,
            pos: Point::new(48.0, 48.0),
            cur_w: 400.0,
            manual_list_h: None,
            drag: None,
            entry: String::new(),
            editing: None,
            item_drag: None,
            selected_folder: None,
            confirm_folder: None,
            context: None,
            settings_maximized: false,
            confirm_restore: false,
        };
        app.recompute_geometry(None, true);

        let mut tasks = Vec::new();
        if app.widget_mode() {
            // A request left over from before this start must not pop up a settings window.
            let _ = paths::take_settings_request();
            // If the compositor reports no outputs, fall back to the active one.
            tasks.push(app.settle_after(0, 1500));
        }
        // Started by hand while the list lives in the panel: show the settings window so the
        // display can be changed back (otherwise the process would have nothing to show).
        let panel_standalone = !app.args.applet && app.settings.display == DisplayMode::Panel;
        if !app.args.applet && (app.args.settings || panel_standalone) {
            tasks.push(app.open_settings());
        }
        (app, Task::batch(tasks))
    }

    fn on_close_requested(&self, id: window::Id) -> Option<Message> {
        if self.popup == Some(id) {
            Some(Message::PopupClosed(id))
        } else if Some(id) == self.surface_id {
            Some(Message::SurfaceLost)
        } else if Some(id) == self.settings_id {
            Some(Message::CloseSettings)
        } else {
            None
        }
    }

    fn style(&self) -> Option<iced::theme::Style> {
        if self.args.applet {
            return Some(cosmic::applet::style());
        }
        let theme = cosmic::theme::active();
        let on: Color = theme.cosmic().on_bg_color().into();
        Some(iced::theme::Style {
            background_color: Color::TRANSPARENT,
            text_color: on,
            icon_color: on,
        })
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subs: Vec<Subscription<Message>> = Vec::new();
        if self.args.applet {
            // The applet watches for settings changed in the settings window (another process).
            subs.push(iced::time::every(Duration::from_secs(2)).map(|_| Message::Tick));
        } else {
            if self.settings.show_clock {
                subs.push(iced::time::every(Duration::from_secs(10)).map(|_| Message::Tick));
            }
            if self.widget_mode() {
                // The panel applet asks the running widget to open its settings this way.
                subs.push(iced::time::every(Duration::from_secs(1)).map(|_| Message::CheckRequests));
            }
            subs.push(event::listen_with(|e, _status, id| match e {
                Event::PlatformSpecific(PlatformSpecific::Wayland(wayland::Event::Output(ev, wl))) => {
                    Some(Message::Output(ev, wl))
                }
                Event::PlatformSpecific(PlatformSpecific::Wayland(wayland::Event::Window(
                    wayland::WindowEvent::WindowState(state),
                ))) => Some(Message::WindowState(
                    id,
                    state.intersects(WindowState::MAXIMIZED | WindowState::FULLSCREEN),
                )),
                _ => None,
            }));
        }
        if self.drag.is_some() {
            subs.push(event::listen_with(|e, _status, _id| match e {
                Event::Mouse(mouse::Event::CursorMoved { position }) => Some(Message::Pointer(position)),
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => Some(Message::GripRelease),
                _ => None,
            }));
        }
        if self.editing.is_some() || self.context.is_some() {
            // A press nobody handled (empty space) confirms the rename / closes the menu.
            subs.push(event::listen_with(|e, status, _id| match (e, status) {
                (Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)), event::Status::Ignored) => {
                    Some(Message::EditClickAway)
                }
                _ => None,
            }));
        }
        if self.item_drag.is_some() {
            // Dropping happens wherever the button is released.
            subs.push(event::listen_with(|e, _status, _id| match e {
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => Some(Message::ItemDrop),
                _ => None,
            }));
        }
        Subscription::batch(subs)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        self.handle(message)
    }

    /// The main window only exists for the panel applet (the panel button); the desktop widget
    /// and the settings window are separate windows (see `view_window`).
    fn view(&self) -> Element<'_, Message> {
        if self.args.applet {
            let button = self
                .core
                .applet
                .icon_button("view-list-symbolic")
                .on_press(Message::AppletPressed);
            return self.core.applet.autosize_window(button).into();
        }
        widget::space().into()
    }

    fn view_window(&self, id: window::Id) -> Element<'_, Message> {
        if Some(id) == self.settings_id {
            return self.settings_window();
        }
        if self.args.applet {
            return if self.popup == Some(id) { self.popup_view() } else { widget::space().into() };
        }
        self.widget_window()
    }
}
