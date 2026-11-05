//! COSMIC Todo Widget – a daily to-do list that lives on a monitor as part of the
//! desktop (wlr-layer-shell), written with libcosmic.
//!
//! Two ways to show the list (setting "Display"):
//!   * Widget – a sticky note on the desktop: drag the header to move it, drag the ◢
//!     handle (bottom-right) to resize it.
//!   * Panel  – an applet in the COSMIC panel; the list opens as a fixed-size popup.
//! Settings open in their own normal window.
//!
//! Options:
//!   --settings     open the settings window on start
//!   --applet       run as the COSMIC panel applet (started by cosmic-panel)
//!   --background   start quietly at login (does nothing when the display is "Panel")

mod config;

use std::process::{Child, Command, Stdio};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use chrono::{DateTime, Local};
use config::{DisplayMode, MIN_WIDTH, SavedSize, Settings, Store, autostart};
use cosmic::app::{Core, Task};
use cosmic::cctk::sctk::reexports::client::protocol::wl_output::WlOutput;
use cosmic::iced::event::{self, PlatformSpecific, wayland};
use cosmic::iced::platform_specific::runtime::wayland::layer_surface::{
    IcedMargin, IcedOutput, SctkLayerSurfaceSettings,
};
use cosmic::iced::platform_specific::shell::commands::layer_surface::{
    Anchor, KeyboardInteractivity, Layer, destroy_layer_surface, get_layer_surface, set_margin, set_size,
};
use cosmic::iced::platform_specific::shell::commands::popup::{destroy_popup, get_popup};
use cosmic::iced::{
    self, Alignment, Background, Border, Color, Event, Length, Limits, Point, Size, Subscription, mouse,
    window,
};
use cosmic::widget::{self, autosize};
use cosmic::{Application, Element, executor};

// ------------------------------------------------------------------ layout constants
/// Height of one single-line task row including the gap to the next one (logical px).
const ROW_H: f32 = 50.0;
/// Extra height of every additional wrapped line of a task.
const LINE_H: f32 = 22.0;
/// Height of everything in the card that is not the list (header, progress, entry,
/// footer, paddings and spacing), with a small safety margin.
const CHROME_H: f32 = 262.0;
const CARD_PADDING: f32 = 18.0;
/// Pixels kept free at the screen edges.
const SAFETY: f32 = 8.0;
/// Widest the card can ever be.
const MAX_WIDTH: f32 = 900.0;
/// Minimum time between two move requests sent to the compositor.
const MOVE_INTERVAL: Duration = Duration::from_millis(30);
/// Fixed width of the panel popup.
const POPUP_W: f32 = 400.0;
/// Fixed list height of the panel popup: room for 5 tasks without scrolling.
const POPUP_LIST_H: f32 = ROW_H * 5.0 + 6.0;

static AUTOSIZE_ID: LazyLock<iced::id::Id> = LazyLock::new(|| iced::id::Id::new("todo-autosize"));
static ENTRY_ID: LazyLock<iced::id::Id> = LazyLock::new(|| iced::id::Id::new("todo-entry"));

#[derive(Clone, Debug, Default)]
struct Args {
    settings: bool,
    applet: bool,
    background: bool,
}

fn main() -> cosmic::iced::Result {
    let mut args = Args::default();
    for a in std::env::args().skip(1) {
        match a.as_str() {
            "--settings" => args.settings = true,
            "--applet" => args.applet = true,
            "--background" => args.background = true,
            "-h" | "--help" => {
                println!(
                    "cosmic-todo-widget\n  --settings     open the settings window on start\n  --applet       run as the COSMIC panel applet\n  --background   start quietly (nothing happens when the display is \"Panel\")"
                );
                return Ok(());
            }
            other => eprintln!("Ignoring unknown option {other}"),
        }
    }

    if args.applet {
        return cosmic::applet::run::<App>(args);
    }

    // Started at login while the list lives in the panel: nothing to do.
    if args.background && Settings::load().display == DisplayMode::Panel {
        return Ok(());
    }
    let settings = cosmic::app::Settings::default().transparent(true).no_main_window(true);
    cosmic::app::run::<App>(settings, args)
}

// --------------------------------------------------------------------------- monitors
#[derive(Clone, Debug)]
struct Monitor {
    wl: WlOutput,
    global_id: u32,
    make: String,
    model: String,
    name: String,
    width: i32,
    height: i32,
}

impl Monitor {
    fn from_info(wl: WlOutput, info: &cosmic::cctk::sctk::output::OutputInfo) -> Self {
        let (width, height) = info
            .logical_size
            .filter(|(w, h)| *w > 0 && *h > 0)
            .unwrap_or((1920, 1080));
        Monitor {
            wl,
            global_id: info.id,
            make: info.make.trim().to_string(),
            model: info.model.trim().to_string(),
            name: info.name.clone().unwrap_or_default(),
            width,
            height,
        }
    }
}

/// A monitor as shown in settings: stable id + readable label.
#[derive(Clone, Debug)]
struct MonitorChoice {
    id: String,
    label: String,
    index: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DragKind {
    Move,
    Resize,
}

#[derive(Clone, Debug)]
struct Drag {
    kind: DragKind,
    start: Option<Point>,
    w0: f32,
    h0: f32,
    last_apply: Instant,
    /// Latest pointer position seen (used to apply the final size / trailing update).
    last: Point,
}

// --------------------------------------------------------------------------- messages
#[derive(Clone, Debug)]
enum Message {
    Noop,
    Tick,
    Output(wayland::OutputEvent, WlOutput),
    OutputsSettled(u64),
    SurfaceLost,

    // tasks
    Input(String),
    Add,
    Toggle(String, bool),
    Delete(String),
    MoveUp(String),
    MoveDown(String),
    ClearDone,

    // settings window
    OpenSettings,
    CloseSettings,

    // settings
    SetDisplay(DisplayMode),
    SetTime24(bool),
    SetSeconds(bool),
    SetMonitor(String),
    SetAbove(bool),
    SetOpacity(f32),
    SetAutostart(bool),
    AskRestore,
    CancelRestore,
    RestoreDefaults,

    // move / resize
    MovePress,
    GripPress,
    GripRelease,
    GripAuto,
    Pointer(Point),

    // panel applet
    AppletPressed,
    PopupClosed(window::Id),

    // close the widget
    Quit,
}

// --------------------------------------------------------------------------- app
struct App {
    core: Core,
    args: Args,
    settings: Settings,
    store: Store,
    now: DateTime<Local>,

    monitors: Vec<Monitor>,
    generation: u64,
    surface_id: Option<window::Id>,
    surface_output: Option<WlOutput>,
    surface_failures: u32,
    settings_id: Option<window::Id>,
    /// Panel applet popup.
    popup: Option<window::Id>,
    /// Held while the desktop widget exists (only one at a time).
    lock: Option<std::fs::File>,
    settings_child: Option<Child>,
    widget_child: Option<Child>,

    // geometry derived from the target monitor
    mon_w: f32,
    mon_h: f32,
    /// Top-left corner of the card relative to the monitor (logical px).
    pos: Point,
    cur_w: f32,
    /// `None` = automatic (follows the number of tasks).
    manual_list_h: Option<f32>,

    entry: String,
    confirm_restore: bool,
    drag: Option<Drag>,
}

/// Start another copy of this program unless the previous one is still running.
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

    fn ensure_lock(&mut self) -> bool {
        if self.lock.is_none() {
            self.lock = config::acquire_widget_lock();
            if self.lock.is_none() {
                eprintln!("Another Todo Widget is already running.");
            }
        }
        self.lock.is_some()
    }

    fn close_popup(&mut self) -> Task<Message> {
        match self.popup.take() {
            Some(id) => destroy_popup::<cosmic::Action<Message>>(id),
            None => Task::none(),
        }
    }

    /// Applet only: pick up settings changed in the settings window (a separate process).
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

    // ---------- geometry ----------
    fn default_w(&self) -> f32 {
        (self.mon_w * 0.20).clamp(MIN_WIDTH + 20.0, 520.0)
    }

    fn default_pos(&self) -> Point {
        let m = (self.mon_w.min(self.mon_h) * 0.06).clamp(24.0, 100.0);
        Point::new(m, m)
    }

    /// Widest the card may be at its current x so it stays on the monitor.
    fn max_w_at_x(&self) -> f32 {
        (self.mon_w - self.pos.x - SAFETY).min(MAX_WIDTH).max(MIN_WIDTH)
    }

    /// Tallest the whole card may be at its current y.
    fn max_card_h_at_y(&self) -> f32 {
        (self.mon_h - self.pos.y - SAFETY).max(CHROME_H + ROW_H * 2.0)
    }

    /// Tallest the list may ever be (whole card still fits on the monitor).
    fn full_cap(&self) -> f32 {
        (self.mon_h - 2.0 * SAFETY - CHROME_H).max(ROW_H * 2.0)
    }

    /// Tallest the list can be at the current y.
    fn avail_list_h(&self) -> f32 {
        (self.max_card_h_at_y() - CHROME_H).max(ROW_H * 2.0)
    }

    /// How many tasks the automatic size shows before scrolling: derived from the monitor height.
    fn auto_cap(&self) -> f32 {
        let rows = (self.mon_h * 0.45 / ROW_H).floor().clamp(4.0, 12.0);
        (rows * ROW_H).min(self.full_cap())
    }

    /// Estimated natural height of the list (wrapped lines are estimated).
    fn natural_list_h(&self) -> f32 {
        let text_w = (self.cur_w - 2.0 * CARD_PADDING - 120.0).max(100.0);
        let chars_per_line = (text_w / 8.0).floor().max(8.0) as usize;
        self.store
            .tasks
            .iter()
            .map(|t| {
                let lines = t.text.chars().count().div_ceil(chars_per_line).max(1);
                ROW_H + (lines as f32 - 1.0) * LINE_H
            })
            .sum()
    }

    /// The list height the user (or the automatic mode) wants, independent of position.
    fn desired_list_h(&self) -> f32 {
        match self.manual_list_h {
            Some(h) => h.clamp(ROW_H, self.full_cap()),
            None => self.natural_list_h().clamp(ROW_H, self.auto_cap()),
        }
    }

    /// The list height actually used: wanted height, but never past the screen bottom.
    fn list_h(&self) -> f32 {
        self.desired_list_h().min(self.avail_list_h()).max(ROW_H)
    }

    /// Keep the card fully on the monitor.
    fn clamp_pos(&mut self) {
        let max_x = (self.mon_w - self.cur_w - SAFETY).max(0.0);
        let max_y = (self.mon_h - SAFETY - CHROME_H - self.desired_list_h()).max(0.0);
        self.pos.x = self.pos.x.clamp(0.0, max_x);
        self.pos.y = self.pos.y.clamp(0.0, max_y);
    }

    fn fit_to_monitor(&mut self) {
        let max_w = (self.mon_w - 2.0 * SAFETY).min(MAX_WIDTH).max(MIN_WIDTH);
        self.cur_w = self.cur_w.clamp(MIN_WIDTH, max_w);
        if let Some(h) = self.manual_list_h {
            self.manual_list_h = Some(h.clamp(ROW_H, self.full_cap()));
        }
        self.clamp_pos();
    }

    /// Set width / list height from the pointer offset since the drag started.
    fn apply_resize(&mut self, w0: f32, h0: f32, start: Point, p: Point) {
        let max_w = self.max_w_at_x();
        let cap = self.full_cap().min(self.avail_list_h());
        // Whole pixels only: sub-pixel changes would still trigger a surface resize.
        self.cur_w = (w0 + (p.x - start.x)).clamp(MIN_WIDTH, max_w).round();
        self.manual_list_h = Some((h0 + (p.y - start.y)).clamp(ROW_H, cap).round());
    }

    fn persist_ui(&mut self) {
        self.store.set_ui(SavedSize {
            x: Some(self.pos.x),
            y: Some(self.pos.y),
            width: Some(self.cur_w),
            list_h: self.manual_list_h,
        });
    }

    /// Recompute everything that depends on the monitor and clamp size and position.
    fn recompute_geometry(&mut self, monitor: Option<&Monitor>, first: bool) {
        let (w, h) = monitor
            .map(|m| (m.width as f32, m.height as f32))
            .unwrap_or((1920.0, 1080.0));
        self.mon_w = w;
        self.mon_h = h;

        if first {
            self.cur_w = self.store.ui.width.unwrap_or_else(|| self.default_w());
            self.manual_list_h = self.store.ui.list_h;
            self.pos = match (self.store.ui.x, self.store.ui.y) {
                (Some(x), Some(y)) => Point::new(x, y),
                _ => self.default_pos(),
            };
        }
        self.fit_to_monitor();
    }

    // ---------- monitors ----------
    fn sorted_monitors(&self) -> Vec<&Monitor> {
        let mut v: Vec<&Monitor> = self.monitors.iter().collect();
        v.sort_by_key(|m| m.global_id);
        v
    }

    fn monitor_choices(&self) -> Vec<MonitorChoice> {
        let mut seen: Vec<(String, usize)> = Vec::new();
        self.sorted_monitors()
            .into_iter()
            .enumerate()
            .map(|(index, m)| {
                let key = format!("{}|{}", m.make, m.model);
                let n = match seen.iter_mut().find(|(k, _)| *k == key) {
                    Some((_, n)) => {
                        *n += 1;
                        *n
                    }
                    None => {
                        seen.push((key.clone(), 0));
                        0
                    }
                };
                let known = |v: &str| !v.is_empty() && !v.eq_ignore_ascii_case("unknown");
                let name = [m.make.as_str(), m.model.as_str()]
                    .into_iter()
                    .filter(|v| known(v))
                    .collect::<Vec<_>>()
                    .join(" ");
                let name = match (name.is_empty(), m.name.is_empty()) {
                    (false, false) => format!("{name} ({})", m.name),
                    (false, true) => name,
                    (true, false) => m.name.clone(),
                    (true, true) => format!("Monitor {}", index + 1),
                };
                MonitorChoice {
                    id: if n == 0 { key } else { format!("{key}#{n}") },
                    label: format!("{name} - {}×{}", m.width, m.height),
                    index,
                }
            })
            .collect()
    }

    /// Wanted monitor, or "auto": the only monitor, else the second one.
    fn resolve_monitor(&self) -> Option<Monitor> {
        let sorted = self.sorted_monitors();
        if sorted.is_empty() {
            return None;
        }
        if self.settings.monitor != "auto" {
            if let Some(c) = self.monitor_choices().into_iter().find(|c| c.id == self.settings.monitor) {
                return sorted.get(c.index).map(|m| (*m).clone());
            }
        }
        sorted.get(1).or(sorted.first()).map(|m| (*m).clone())
    }

    // ---------- layer surface ----------
    fn create_surface(&mut self, first: bool) -> Task<Message> {
        if !self.ensure_lock() {
            return Task::none();
        }
        let monitor = self.resolve_monitor();
        self.recompute_geometry(monitor.as_ref(), first);
        self.surface_output = monitor.as_ref().map(|m| m.wl.clone());

        // Always anchored top-left; the position is the margin from that corner.
        let m = IcedMargin {
            top: self.pos.y.round() as i32,
            bottom: 0,
            left: self.pos.x.round() as i32,
            right: 0,
        };

        let id = window::Id::unique();
        self.surface_id = Some(id);
        eprintln!(
            "layer-shell: {} layer, monitor {}x{}, at {},{}",
            if self.settings.above_windows { "top" } else { "bottom" },
            self.mon_w,
            self.mon_h,
            m.left,
            m.top
        );

        get_layer_surface::<cosmic::Action<Message>>(SctkLayerSurfaceSettings {
            id,
            layer: if self.settings.above_windows { Layer::Top } else { Layer::Bottom },
            keyboard_interactivity: KeyboardInteractivity::OnDemand,
            input_zone: None,
            anchor: Anchor::TOP | Anchor::LEFT,
            output: monitor.map_or(IcedOutput::Active, |m| IcedOutput::Output(m.wl)),
            namespace: "todo-widget".into(),
            margin: m,
            size: None,
            exclusive_zone: 0,
            size_limits: Limits::NONE
                .min_width(1.0)
                .min_height(1.0)
                .max_width(self.mon_w)
                .max_height(self.mon_h),
        })
    }

    fn rebuild_surface(&mut self) -> Task<Message> {
        let old = self.surface_id.take();
        let create = self.create_surface(false);
        match old {
            Some(old) => Task::batch([destroy_layer_surface::<cosmic::Action<Message>>(old), create]),
            None => create,
        }
    }

    /// Move the existing surface by updating its margins (no rebuild).
    fn apply_margin(&self) -> Task<Message> {
        match self.surface_id {
            Some(id) => set_margin::<cosmic::Action<Message>>(
                id,
                self.pos.y.round() as i32,
                0,
                0,
                self.pos.x.round() as i32,
            ),
            None => Task::none(),
        }
    }

    /// True while the resize handle is being dragged.
    fn resizing(&self) -> bool {
        self.drag.as_ref().is_some_and(|d| d.kind == DragKind::Resize)
    }

    /// Before a resize: make the (transparent) surface as big as the card can ever get at
    /// this position, so resizing only re-lays out the card inside it and never has to
    /// wait for the compositor to resize the surface.
    fn grow_surface(&self) -> Task<Message> {
        match self.surface_id {
            Some(id) => set_size::<cosmic::Action<Message>>(
                id,
                Some(self.max_w_at_x().ceil() as u32),
                Some(self.max_card_h_at_y().ceil() as u32),
            ),
            None => Task::none(),
        }
    }

    fn settle_after(&self, generation: u64, ms: u64) -> Task<Message> {
        cosmic::task::future(async move {
            tokio::time::sleep(Duration::from_millis(ms)).await;
            Message::OutputsSettled(generation)
        })
    }

    // ---------- settings window ----------
    fn open_settings(&mut self) -> Task<Message> {
        if self.settings_id.is_some() {
            return Task::none();
        }
        let (id, open) = window::open(window::Settings {
            size: Size::new(480.0, 680.0),
            min_size: Some(Size::new(400.0, 420.0)),
            ..Default::default()
        });
        self.settings_id = Some(id);
        open.map(|_| cosmic::Action::App(Message::Noop))
    }

    fn apply_setting_change(&mut self, rebuild: bool) -> Task<Message> {
        self.settings.save();
        self.confirm_restore = false;
        if rebuild && self.widget_mode() {
            return self.rebuild_surface();
        }
        Task::none()
    }

    // ---------- styling ----------
    fn card_style(&self) -> cosmic::theme::Container<'static> {
        let alpha = self.settings.opacity.clamp(30, 100) as f32 / 100.0;
        cosmic::theme::Container::custom(move |theme| {
            let c = theme.cosmic();
            let mut bg: Color = c.bg_color().into();
            bg.a = alpha;
            let on: Color = c.on_bg_color().into();
            cosmic::iced::widget::container::Style {
                icon_color: Some(on),
                text_color: Some(on),
                background: Some(Background::Color(bg)),
                border: Border {
                    radius: 20.0.into(),
                    width: 1.0,
                    color: Color { a: 0.12, ..on },
                },
                ..Default::default()
            }
        })
    }

    // ---------- views ----------
    /// Resize handle: always the bottom-right corner (the card is anchored top-left).
    fn grip(&self) -> Element<'_, Message> {
        let glyph = widget::text("◢").size(18);
        widget::mouse_area(widget::container(glyph).padding([4, 8]))
            .on_press(Message::GripPress)
            .on_release(Message::GripRelease)
            .on_double_click(Message::GripAuto)
            .interaction(mouse::Interaction::ResizingDiagonallyDown)
            .into()
    }

    fn clock_view(&self) -> Element<'_, Message> {
        let s = &self.settings;
        let fmt = match (s.time_24h, s.show_seconds) {
            (true, true) => "%H:%M:%S",
            (true, false) => "%H:%M",
            (false, true) => "%I:%M:%S",
            (false, false) => "%I:%M",
        };
        let mut time = self.now.format(fmt).to_string();
        let mut row = widget::Row::new().align_y(Alignment::End).spacing(6);
        if !s.time_24h {
            time = time.trim_start_matches('0').to_string();
            row = row.push(widget::text(time).size(30));
            row = row.push(widget::text(self.now.format("%p").to_string()).size(12));
        } else {
            row = row.push(widget::text(time).size(30));
        }
        row.into()
    }

    // ---- pieces shared by the widget card and the panel popup ----
    fn title_block(&self) -> Element<'_, Message> {
        widget::Column::new()
            .spacing(6)
            .push(widget::text::title2("Today"))
            .push(widget::text::caption(self.now.format("%A, %d %B %Y").to_string()))
            .width(Length::Fill)
            .into()
    }

    fn progress_block(&self) -> Element<'_, Message> {
        let total = self.store.tasks.len();
        let done = self.store.tasks.iter().filter(|t| t.done).count();
        let progress = widget::progress_bar::determinate_linear(if total > 0 {
            done as f32 / total as f32
        } else {
            0.0
        })
        .width(Length::Fill);
        let counter = widget::text::caption(if total > 0 {
            format!("{done} of {total} done")
        } else {
            "No tasks yet".to_string()
        });
        widget::Column::new().spacing(6).push(progress).push(counter).into()
    }

    fn entry_row(&self) -> Element<'_, Message> {
        widget::Row::new()
            .spacing(10)
            .align_y(Alignment::Center)
            .push(
                widget::text_input("Add a task…", &self.entry)
                    .id(ENTRY_ID.clone())
                    .on_input(Message::Input)
                    .on_submit(|_| Message::Add)
                    .width(Length::Fill),
            )
            .push(widget::button::suggested("Add").on_press(Message::Add))
            .into()
    }

    /// The scrollable task list with a fixed height.
    fn task_list(&self, height: f32) -> Element<'_, Message> {
        // Pending tasks first, completed at the bottom (order otherwise kept).
        let mut ordered: Vec<&config::Task> = self.store.tasks.iter().collect();
        ordered.sort_by_key(|t| t.done);

        // Right padding keeps the text clear of the scrollbar.
        let mut list = widget::Column::new().spacing(4).width(Length::Fill).padding([0, 14, 0, 0]);
        if ordered.is_empty() {
            list = list.push(
                widget::container(widget::text::body("Nothing to do - enjoy the day"))
                    .width(Length::Fill)
                    .height(Length::Fixed(ROW_H))
                    .align_x(Alignment::Center)
                    .align_y(Alignment::Center),
            );
        }
        for (i, t) in ordered.iter().enumerate() {
            let can_up = i > 0 && ordered[i - 1].done == t.done;
            let can_down = i + 1 < ordered.len() && ordered[i + 1].done == t.done;
            let id = t.id.clone();
            let up_id = t.id.clone();
            let down_id = t.id.clone();
            let del_id = t.id.clone();
            let label = if t.done {
                widget::text::body(t.text.clone()).class(cosmic::theme::Text::Custom(|theme| {
                    let mut c: Color = theme.cosmic().on_bg_color().into();
                    c.a = 0.45;
                    cosmic::iced::widget::text::Style { color: Some(c), ..Default::default() }
                }))
            } else {
                widget::text::body(t.text.clone())
            };
            list = list.push(
                widget::Row::new()
                    .spacing(6)
                    .align_y(Alignment::Center)
                    .padding([7, 6])
                    .push(widget::checkbox(t.done).on_toggle(move |v| Message::Toggle(id.clone(), v)))
                    .push(label.width(Length::Fill))
                    .push(
                        widget::button::text("▲")
                            .on_press_maybe(can_up.then(|| Message::MoveUp(up_id))),
                    )
                    .push(
                        widget::button::text("▼")
                            .on_press_maybe(can_down.then(|| Message::MoveDown(down_id))),
                    )
                    .push(widget::button::text("✕").on_press(Message::Delete(del_id))),
            );
        }
        widget::scrollable(list).height(Length::Fixed(height)).into()
    }

    fn tools_row(&self) -> Element<'_, Message> {
        let done = self.store.tasks.iter().filter(|t| t.done).count();
        widget::Row::new()
            .spacing(8)
            .align_y(Alignment::Center)
            .push(
                widget::button::text("Clear completed")
                    .on_press_maybe((done > 0).then_some(Message::ClearDone)),
            )
            .push(widget::button::text("Settings").on_press(Message::OpenSettings))
            .into()
    }

    // ---- desktop widget ----
    fn tasks_view(&self) -> Element<'_, Message> {
        let header = widget::Row::new()
            .align_y(Alignment::Start)
            .spacing(12)
            .push(self.title_block())
            .push(self.clock_view())
            .push(widget::button::text("✕").on_press(Message::Quit));
        // The header is the "sticky note" drag handle.
        let header: Element<'_, Message> = widget::mouse_area(header)
            .on_press(Message::MovePress)
            .on_release(Message::GripRelease)
            .interaction(mouse::Interaction::Grab)
            .into();

        let footer = widget::Row::new()
            .align_y(Alignment::Center)
            .push(self.tools_row())
            .push(widget::space().width(Length::Fill))
            .push(self.grip());

        widget::Column::new()
            .spacing(14)
            .push(header)
            .push(self.progress_block())
            .push(self.entry_row())
            .push(self.task_list(self.list_h()))
            .push(footer)
            .into()
    }

    // ---- panel popup (fixed size, room for 5 tasks) ----
    fn popup_view(&self) -> Element<'_, Message> {
        let header = widget::Row::new()
            .align_y(Alignment::Start)
            .spacing(12)
            .push(self.title_block())
            .push(self.clock_view());
        let content = widget::Column::new()
            .spacing(14)
            .width(Length::Fill)
            .push(header)
            .push(self.progress_block())
            .push(self.entry_row())
            .push(self.task_list(POPUP_LIST_H))
            .push(self.tools_row());
        self.core
            .applet
            .popup_container(
                widget::container(content).padding(16).width(Length::Fixed(POPUP_W)),
            )
            .into()
    }

    /// Content of the separate settings window.
    fn settings_window(&self) -> Element<'_, Message> {
        let s = &self.settings;

        let row = |title: &'static str, hint: Option<&'static str>, control: Element<'static, Message>| {
            let mut left = widget::Column::new().spacing(6).push(widget::text::body(title));
            if let Some(h) = hint {
                left = left.push(widget::text::caption(h));
            }
            widget::Row::new()
                .spacing(16)
                .align_y(Alignment::Center)
                .push(left.width(Length::Fill))
                .push(control)
        };
        let toggle = |v: bool, msg: fn(bool) -> Message| -> Element<'static, Message> {
            widget::toggler(v).on_toggle(msg).into()
        };
        let section = |t: &'static str| widget::container(widget::text::heading(t)).padding([10, 0, 0, 0]);

        let widget_mode = s.display == DisplayMode::Widget;

        let mut body = widget::Column::new()
            .spacing(16)
            .width(Length::Fill)
            .padding([0, 16, 0, 0])
            .push(section("Display"))
            .push(
                widget::Row::new()
                    .spacing(8)
                    .push(choice(
                        "Show as Widget",
                        widget_mode,
                        Message::SetDisplay(DisplayMode::Widget),
                    ))
                    .push(choice(
                        "Show on Panel",
                        !widget_mode,
                        Message::SetDisplay(DisplayMode::Panel),
                    )),
            );
        body = body.push(widget::text::caption(if widget_mode {
            "A sticky note on the desktop: drag its header to move it, resize it with the ◢ handle in the bottom-right corner (double-click it for automatic height)."
        } else {
            "A popup of the COSMIC panel with room for 5 tasks. Add the \"Todo Widget\" applet to your panel in COSMIC Settings → Desktop → Panel (or Dock) → Configure panel applets."
        }));

        body = body
            .push(section("Clock"))
            .push(row("24-hour clock", None, toggle(s.time_24h, Message::SetTime24)))
            .push(row("Show seconds", None, toggle(s.show_seconds, Message::SetSeconds)));

        if widget_mode {
            let mut monitors = widget::Column::new().spacing(8).width(Length::Fill);
            monitors = monitors.push(choice(
                "Auto (second monitor if present)",
                s.monitor == "auto",
                Message::SetMonitor("auto".into()),
            ));
            for c in self.monitor_choices() {
                monitors =
                    monitors.push(choice(&c.label, s.monitor == c.id, Message::SetMonitor(c.id.clone())));
            }
            body = body
                .push(section("Placement"))
                .push(widget::text::body("Monitor"))
                .push(monitors)
                .push(row(
                    "Keep above windows",
                    Some("Off: part of the desktop, hidden when windows cover it"),
                    toggle(s.above_windows, Message::SetAbove),
                ))
                .push(section("Look"))
                .push(row(
                    "Opacity (%)",
                    None,
                    widget::slider(30..=100, s.opacity as i32, |v| Message::SetOpacity(v as f32))
                        .width(Length::Fixed(160.0))
                        .into(),
                ));
        }

        body = body
            .push(section("Startup"))
            .push(row(
                "Start at login",
                Some("Off by default (the panel applet starts with the panel)"),
                toggle(s.autostart, Message::SetAutostart),
            ))
            .push(section("Reset"))
            .push(self.restore_view());

        let header = widget::Row::new()
            .align_y(Alignment::Center)
            .push(widget::text::title3("Settings").width(Length::Fill))
            .push(widget::button::suggested("Done").on_press(Message::CloseSettings));

        widget::container(
            widget::Column::new()
                .spacing(16)
                .push(header)
                .push(widget::scrollable(body).height(Length::Fill)),
        )
        .padding(20)
        .width(Length::Fill)
        .height(Length::Fill)
        .class(cosmic::theme::Container::Background)
        .into()
    }

    fn restore_view(&self) -> Element<'_, Message> {
        // Both states have the same height so the confirmation never scrolls out of view.
        if self.confirm_restore {
            widget::Column::new()
                .spacing(8)
                .push(
                    widget::Row::new()
                        .spacing(10)
                        .push(widget::button::destructive("Restore defaults").on_press(Message::RestoreDefaults))
                        .push(widget::button::standard("Cancel").on_press(Message::CancelRestore)),
                )
                .push(widget::text::caption("Really reset settings, position and size? Tasks and the display choice are kept."))
                .into()
        } else {
            widget::Column::new()
                .spacing(8)
                .push(widget::button::destructive("Restore defaults…").on_press(Message::AskRestore))
                .push(widget::text::caption("Resets settings, position and size. Tasks and the display choice are not touched."))
                .into()
        }
    }

    fn card(&self) -> Element<'_, Message> {
        widget::container(self.tasks_view())
            .padding(CARD_PADDING)
            .width(Length::Fixed(self.cur_w))
            .class(self.card_style())
            .into()
    }
}

fn choice(label: &str, selected: bool, msg: Message) -> Element<'static, Message> {
    let b = if selected {
        widget::button::suggested(label.to_string())
    } else {
        widget::button::standard(label.to_string())
    };
    b.on_press(msg).width(Length::Fill).into()
}

// --------------------------------------------------------------------------- Application
impl Application for App {
    type Executor = executor::Default;
    type Flags = Args;
    type Message = Message;
    const APP_ID: &'static str = "io.github.user.CosmicTodoWidget";

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, args: Args) -> (Self, Task<Message>) {
        let settings = Settings::load();
        let store = Store::load();
        let mut app = App {
            core,
            args,
            settings,
            store,
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
            cur_w: 360.0,
            manual_list_h: None,
            entry: String::new(),
            confirm_restore: false,
            drag: None,
        };
        app.recompute_geometry(None, true);

        let mut tasks = Vec::new();
        if app.widget_mode() {
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
        let every = match (self.args.applet, self.settings.show_seconds) {
            (_, true) => 1,
            (true, false) => 2, // the applet also watches for settings changes
            (false, false) => 10,
        };
        let mut subs = vec![iced::time::every(Duration::from_secs(every)).map(|_| Message::Tick)];

        if !self.args.applet {
            subs.push(event::listen_with(|e, _status, _id| match e {
                Event::PlatformSpecific(PlatformSpecific::Wayland(wayland::Event::Output(ev, wl))) => {
                    Some(Message::Output(ev, wl))
                }
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
        Subscription::batch(subs)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Noop => {}
            Message::Tick => {
                self.now = Local::now();
                if self.args.applet {
                    // The applet's task list is only fresh while its popup is open
                    // (it is reloaded on open), so only then may it write tasks.
                    if self.popup.is_some() {
                        self.store.rollover_if_needed();
                    }
                    return self.sync_applet();
                }
                self.store.rollover_if_needed();
            }

            // ----- monitors / surface -----
            Message::Output(ev, wl) => {
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
                return self.settle_after(self.generation, 600);
            }
            Message::OutputsSettled(generation) => {
                if generation != self.generation || !self.widget_mode() {
                    return Task::none();
                }
                let target = self.resolve_monitor();
                let moved = self.surface_id.is_none()
                    || target.as_ref().map(|m| &m.wl) != self.surface_output.as_ref()
                    || target
                        .as_ref()
                        .is_some_and(|m| m.width as f32 != self.mon_w || m.height as f32 != self.mon_h);
                if moved {
                    if self.surface_id.is_none() && !self.ensure_lock() {
                        // Another widget is already on the desktop.
                        if self.settings_id.is_none() {
                            std::process::exit(0);
                        }
                        return Task::none();
                    }
                    return if self.surface_id.is_none() {
                        self.create_surface(true)
                    } else {
                        self.rebuild_surface()
                    };
                }
            }
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
                    self.store.add(text);
                    self.entry.clear();
                }
            }
            Message::Toggle(id, done) => self.store.toggle(&id, done),
            Message::Delete(id) => self.store.delete(&id),
            Message::MoveUp(id) => self.store.move_task(&id, true),
            Message::MoveDown(id) => self.store.move_task(&id, false),
            Message::ClearDone => self.store.clear_done(),

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
                if self.settings.display == DisplayMode::Panel {
                    // Nothing else is shown by this process while the list lives in the panel.
                    std::process::exit(0);
                }
                if let Some(id) = self.settings_id.take() {
                    return window::close(id);
                }
            }

            // ----- panel applet -----
            Message::AppletPressed => {
                if self.settings.display == DisplayMode::Widget {
                    // The list is shown as a desktop widget: the panel icon just starts it.
                    spawn_child(&[], &mut self.widget_child);
                    return Task::none();
                }
                if self.popup.is_some() {
                    return self.close_popup();
                }
                // Always start from what is on disk (the widget may have changed it).
                self.settings = Settings::load();
                self.store = Store::load();
                let Some(main_id) = self.core.main_window_id() else {
                    return Task::none();
                };
                let new_id = window::Id::unique();
                self.popup = Some(new_id);
                let mut popup = self.core.applet.get_popup_settings(main_id, new_id, None, None, None);
                popup.positioner.size_limits = Limits::NONE
                    .min_width(POPUP_W)
                    .max_width(POPUP_W)
                    .min_height(120.0)
                    .max_height(1000.0);
                return get_popup::<cosmic::Action<Message>>(popup);
            }
            Message::PopupClosed(id) => {
                if self.popup == Some(id) {
                    self.popup = None;
                }
            }

            // ----- settings -----
            Message::SetDisplay(mode) => {
                if self.settings.display == mode {
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
                    }
                    DisplayMode::Widget => {
                        self.generation += 1;
                        return self.settle_after(self.generation, 0);
                    }
                }
            }
            Message::SetTime24(v) => {
                self.settings.time_24h = v;
                return self.apply_setting_change(false);
            }
            Message::SetSeconds(v) => {
                self.settings.show_seconds = v;
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
            Message::SetOpacity(v) => {
                self.settings.opacity = (v.round() as u32).clamp(30, 100);
                self.settings.save();
            }
            Message::SetAutostart(v) => {
                autostart::set(v);
                self.settings.autostart = autostart::is_enabled();
            }

            Message::AskRestore => self.confirm_restore = true,
            Message::CancelRestore => self.confirm_restore = false,
            Message::RestoreDefaults => {
                let display = self.settings.display;
                self.settings = Settings::default();
                self.settings.display = display;
                self.settings.save();
                autostart::set(self.settings.autostart);
                self.confirm_restore = false;
                self.drag = None;
                self.manual_list_h = None;
                self.cur_w = self.default_w();
                self.pos = self.default_pos();
                self.store.set_ui(SavedSize::default());
                return if self.widget_mode() { self.rebuild_surface() } else { Task::none() };
            }

            // ----- move / resize -----
            Message::MovePress => {
                self.drag = Some(Drag {
                    kind: DragKind::Move,
                    start: None,
                    w0: self.cur_w,
                    h0: self.list_h(),
                    last_apply: Instant::now() - Duration::from_secs(1),
                    last: Point::ORIGIN,
                });
            }
            Message::GripPress => {
                self.drag = Some(Drag {
                    kind: DragKind::Resize,
                    start: None,
                    w0: self.cur_w,
                    h0: self.list_h(),
                    last_apply: Instant::now() - Duration::from_secs(1),
                    last: Point::ORIGIN,
                });
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
            Message::Pointer(p) => {
                let Some(d) = self.drag.as_mut() else { return Task::none() };
                d.last = p;
                let Some(start) = d.start else {
                    d.start = Some(p);
                    return Task::none();
                };
                match d.kind {
                    // The card is anchored top-left, so the pointer position relative to
                    // the surface does not shift while resizing, and the surface is already
                    // full size (see `grow_surface`), so every event is applied directly.
                    DragKind::Resize => {
                        d.last_apply = Instant::now();
                        let (w0, h0) = (d.w0, d.h0);
                        self.apply_resize(w0, h0, start, p);
                    }
                    // Moving shifts the surface under the pointer, so send at most one
                    // margin update per interval and move by the offset from the grab point.
                    DragKind::Move => {
                        if d.last_apply.elapsed() < MOVE_INTERVAL {
                            return Task::none();
                        }
                        let (dx, dy) = (p.x - start.x, p.y - start.y);
                        if dx.abs() < 1.0 && dy.abs() < 1.0 {
                            return Task::none();
                        }
                        d.last_apply = Instant::now();
                        self.pos = Point::new(self.pos.x + dx, self.pos.y + dy);
                        self.clamp_pos();
                        return self.apply_margin();
                    }
                }
            }
            Message::Quit => {
                self.persist_ui();
                std::process::exit(0);
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        // The widget uses `view_window`; only the applet has a main window (the panel button).
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
        // Always the same widget structure (a different one would break the widget tree);
        // while resizing, the content is a fixed-size transparent area so the surface
        // itself never has to change size.
        let (w, h) = if self.resizing() {
            (Length::Fixed(self.max_w_at_x()), Length::Fixed(self.max_card_h_at_y()))
        } else {
            (Length::Shrink, Length::Shrink)
        };
        let content = widget::container(self.card())
            .width(w)
            .height(h)
            .align_x(Alignment::Start)
            .align_y(Alignment::Start);
        autosize::autosize(content, AUTOSIZE_ID.clone())
            .limits(
                Limits::NONE
                    .min_width(1.0)
                    .min_height(1.0)
                    .max_width(self.max_w_at_x())
                    .max_height(self.max_card_h_at_y()),
            )
            .into()
    }
}