//! COSMIC Todo Widget – a daily to-do list that lives on a monitor as part of the
//! desktop (wlr-layer-shell), written with libcosmic.
//!
//! The widget behaves like a sticky note: drag the header to move it, drag the ◢
//! handle (bottom-right) to resize it. Settings open in their own normal window.
//!
//! Options:
//!   --settings   open the settings window on start
//!   --windowed   normal window instead of a desktop widget (no layer-shell)

mod config;

use std::sync::LazyLock;
use std::time::{Duration, Instant};

use chrono::{DateTime, Local};
use config::{MIN_WIDTH, SavedSize, Settings, Store, autostart};
use cosmic::app::{Core, Task};
use cosmic::cctk::sctk::reexports::client::protocol::wl_output::WlOutput;
use cosmic::iced::event::{self, PlatformSpecific, wayland};
use cosmic::iced::platform_specific::runtime::wayland::layer_surface::{
    IcedMargin, IcedOutput, SctkLayerSurfaceSettings,
};
use cosmic::iced::platform_specific::shell::commands::layer_surface::{
    Anchor, KeyboardInteractivity, Layer, destroy_layer_surface, get_layer_surface, set_margin, set_size,
};
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

static AUTOSIZE_ID: LazyLock<iced::id::Id> = LazyLock::new(|| iced::id::Id::new("todo-autosize"));
static ENTRY_ID: LazyLock<iced::id::Id> = LazyLock::new(|| iced::id::Id::new("todo-entry"));

#[derive(Clone, Debug, Default)]
struct Args {
    settings: bool,
    windowed: bool,
}

fn main() -> cosmic::iced::Result {
    let mut args = Args::default();
    for a in std::env::args().skip(1) {
        match a.as_str() {
            "--settings" => args.settings = true,
            "--windowed" => args.windowed = true,
            "-h" | "--help" => {
                println!(
                    "cosmic-todo-widget\n  --settings   open the settings window on start\n  --windowed   normal window instead of a desktop widget"
                );
                return Ok(());
            }
            other => eprintln!("Ignoring unknown option {other}"),
        }
    }

    let mut settings = cosmic::app::Settings::default().transparent(true);
    if args.windowed {
        settings = settings.size(iced::Size::new(420.0, 640.0));
    } else {
        settings = settings.no_main_window(true);
    }
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
    ClearDone,

    // settings window
    OpenSettings,
    CloseSettings,

    // settings
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

impl App {
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
        if rebuild && !self.args.windowed {
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

    fn tasks_view(&self) -> Element<'_, Message> {
        let total = self.store.tasks.len();
        let done = self.store.tasks.iter().filter(|t| t.done).count();

        let header = widget::Row::new()
            .align_y(Alignment::Start)
            .spacing(12)
            .push(
                widget::Column::new()
                    .spacing(6)
                    .push(widget::text::title2("Today"))
                    .push(widget::text::caption(self.now.format("%A, %d %B %Y").to_string()))
                    .width(Length::Fill),
            )
            .push(self.clock_view())
            .push(widget::button::text("✕").on_press(Message::Quit));
        // The header is the "sticky note" drag handle.
        let header: Element<'_, Message> = if self.args.windowed {
            header.into()
        } else {
            widget::mouse_area(header)
                .on_press(Message::MovePress)
                .on_release(Message::GripRelease)
                .interaction(mouse::Interaction::Grab)
                .into()
        };

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
        let progress_block = widget::Column::new().spacing(6).push(progress).push(counter);

        let entry = widget::Row::new()
            .spacing(10)
            .align_y(Alignment::Center)
            .push(
                widget::text_input("Add a task…", &self.entry)
                    .id(ENTRY_ID.clone())
                    .on_input(Message::Input)
                    .on_submit(|_| Message::Add)
                    .width(Length::Fill),
            )
            .push(widget::button::suggested("Add").on_press(Message::Add));

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
        for t in ordered {
            let id = t.id.clone();
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
                    .spacing(12)
                    .align_y(Alignment::Center)
                    .padding([7, 6])
                    .push(widget::checkbox(t.done).on_toggle(move |v| Message::Toggle(id.clone(), v)))
                    .push(label.width(Length::Fill))
                    .push(widget::button::text("✕").on_press(Message::Delete(del_id))),
            );
        }
        let scroller = widget::scrollable(list).height(Length::Fixed(self.list_h()));

        let tools = widget::Row::new()
            .spacing(8)
            .align_y(Alignment::Center)
            .push(
                widget::button::text("Clear completed")
                    .on_press_maybe((done > 0).then_some(Message::ClearDone)),
            )
            .push(widget::button::text("Settings").on_press(Message::OpenSettings));

        let mut footer = widget::Row::new()
            .align_y(Alignment::Center)
            .push(tools)
            .push(widget::space().width(Length::Fill));
        if !self.args.windowed {
            footer = footer.push(self.grip());
        }

        widget::Column::new()
            .spacing(14)
            .push(header)
            .push(progress_block)
            .push(entry)
            .push(scroller)
            .push(footer)
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

        let mut monitors = widget::Column::new().spacing(8).width(Length::Fill);
        monitors = monitors.push(choice(
            "Auto (second monitor if present)",
            s.monitor == "auto",
            Message::SetMonitor("auto".into()),
        ));
        for c in self.monitor_choices() {
            monitors = monitors.push(choice(&c.label, s.monitor == c.id, Message::SetMonitor(c.id.clone())));
        }

        let body = widget::Column::new()
            .spacing(16)
            .width(Length::Fill)
            .padding([0, 16, 0, 0])
            .push(section("Clock"))
            .push(row("24-hour clock", None, toggle(s.time_24h, Message::SetTime24)))
            .push(row("Show seconds", None, toggle(s.show_seconds, Message::SetSeconds)))
            .push(section("Placement"))
            .push(widget::text::caption(
                "Move the widget by dragging its header. Resize it with the ◢ handle in the bottom-right corner (double-click it for automatic height).",
            ))
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
            ))
            .push(section("Startup"))
            .push(row(
                "Start at login",
                Some("Off by default"),
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
                .push(widget::text::caption("Really reset settings, position and size? Tasks are kept."))
                .into()
        } else {
            widget::Column::new()
                .spacing(8)
                .push(widget::button::destructive("Restore defaults…").on_press(Message::AskRestore))
                .push(widget::text::caption("Resets settings, position and size. Tasks are not touched."))
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
        if !app.args.windowed {
            // If the compositor reports no outputs, fall back to the active one.
            tasks.push(app.settle_after(0, 1500));
        }
        if app.args.settings {
            tasks.push(app.open_settings());
        }
        (app, Task::batch(tasks))
    }

    fn on_close_requested(&self, id: window::Id) -> Option<Message> {
        if Some(id) == self.surface_id {
            Some(Message::SurfaceLost)
        } else if Some(id) == self.settings_id {
            Some(Message::CloseSettings)
        } else {
            None
        }
    }

    fn style(&self) -> Option<iced::theme::Style> {
        let theme = cosmic::theme::active();
        let on: Color = theme.cosmic().on_bg_color().into();
        Some(iced::theme::Style {
            background_color: Color::TRANSPARENT,
            text_color: on,
            icon_color: on,
        })
    }

    fn subscription(&self) -> Subscription<Message> {
        let every = if self.settings.show_seconds { 1 } else { 10 };
        let mut subs = vec![iced::time::every(Duration::from_secs(every)).map(|_| Message::Tick)];

        if !self.args.windowed {
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
                if generation != self.generation || self.args.windowed {
                    return Task::none();
                }
                let target = self.resolve_monitor();
                let moved = self.surface_id.is_none()
                    || target.as_ref().map(|m| &m.wl) != self.surface_output.as_ref()
                    || target
                        .as_ref()
                        .is_some_and(|m| m.width as f32 != self.mon_w || m.height as f32 != self.mon_h);
                if moved {
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
            Message::ClearDone => self.store.clear_done(),

            // ----- settings window -----
            Message::OpenSettings => {
                self.confirm_restore = false;
                return self.open_settings();
            }
            Message::CloseSettings => {
                self.confirm_restore = false;
                if let Some(id) = self.settings_id.take() {
                    return window::close(id);
                }
            }

            // ----- settings -----
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
                self.settings = Settings::default();
                self.settings.save();
                autostart::set(self.settings.autostart);
                self.confirm_restore = false;
                self.drag = None;
                self.manual_list_h = None;
                self.cur_w = self.default_w();
                self.pos = self.default_pos();
                self.store.set_ui(SavedSize::default());
                return if self.args.windowed { Task::none() } else { self.rebuild_surface() };
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
        // Only used with --windowed (the layer surface uses `view_window`).
        widget::container(widget::scrollable(self.card())).padding(8).into()
    }

    fn view_window(&self, id: window::Id) -> Element<'_, Message> {
        if Some(id) == self.settings_id {
            return self.settings_window();
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