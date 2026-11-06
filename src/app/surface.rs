//! The layer-shell surface of the desktop widget and the settings window.

use std::time::Duration;

use cosmic::app::Task;
use cosmic::iced::platform_specific::runtime::wayland::layer_surface::{
    IcedMargin, IcedOutput, SctkLayerSurfaceSettings,
};
use cosmic::iced::platform_specific::shell::commands::layer_surface::{
    Anchor, KeyboardInteractivity, Layer, destroy_layer_surface, get_layer_surface, set_margin,
    set_size,
};
use cosmic::iced::{Limits, Size, window};

use super::message::Message;
use super::App;

impl App {
    /// Creates the desktop widget's surface on the wanted monitor (`first`: also restore the
    /// saved size and position). Does nothing if another widget already holds the lock.
    pub(super) fn create_surface(&mut self, first: bool) -> Task<Message> {
        if !self.ensure_lock() {
            return Task::none();
        }
        let monitor = self.resolve_monitor();
        self.recompute_geometry(monitor.as_ref(), first);
        self.surface_output = monitor.as_ref().map(|m| m.wl.clone());

        // Always anchored top-left; the position is the margin from that corner.
        let margin = IcedMargin {
            top: self.pos.y.round() as i32,
            bottom: 0,
            left: self.pos.x.round() as i32,
            right: 0,
        };

        let id = window::Id::unique();
        self.surface_id = Some(id);

        get_layer_surface::<cosmic::Action<Message>>(SctkLayerSurfaceSettings {
            id,
            layer: if self.settings.above_windows { Layer::Top } else { Layer::Bottom },
            keyboard_interactivity: KeyboardInteractivity::OnDemand,
            input_zone: None,
            anchor: Anchor::TOP | Anchor::LEFT,
            output: monitor.map_or(IcedOutput::Active, |m| IcedOutput::Output(m.wl)),
            namespace: "todo-widget".into(),
            margin,
            size: None,
            exclusive_zone: 0,
            size_limits: Limits::NONE
                .min_width(1.0)
                .min_height(1.0)
                .max_width(self.mon_w)
                .max_height(self.mon_h),
        })
    }

    /// Replaces the surface (monitor or layer changed).
    pub(super) fn rebuild_surface(&mut self) -> Task<Message> {
        let old = self.surface_id.take();
        let create = self.create_surface(false);
        match old {
            Some(old) => Task::batch([destroy_layer_surface::<cosmic::Action<Message>>(old), create]),
            None => create,
        }
    }

    /// Moves the existing surface by updating its margins (no rebuild).
    pub(super) fn apply_margin(&self) -> Task<Message> {
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

    /// Before a resize: makes the (transparent) surface as big as the card can ever get at this
    /// position, so resizing only re-lays out the card inside it and never has to wait for the
    /// compositor to resize the surface.
    pub(super) fn grow_surface(&self) -> Task<Message> {
        match self.surface_id {
            Some(id) => set_size::<cosmic::Action<Message>>(
                id,
                Some(self.max_w_at_x().ceil() as u32),
                Some(self.max_card_h_at_y().ceil() as u32),
            ),
            None => Task::none(),
        }
    }

    /// Sends `OutputsSettled(generation)` after `ms` milliseconds (debounce for monitor events).
    pub(super) fn settle_after(&self, generation: u64, ms: u64) -> Task<Message> {
        cosmic::task::future(async move {
            tokio::time::sleep(Duration::from_millis(ms)).await;
            Message::OutputsSettled(generation)
        })
    }

    /// Opens the settings window in this process (nothing happens if it is already open).
    pub(super) fn open_settings(&mut self) -> Task<Message> {
        if self.settings_id.is_some() {
            return Task::none();
        }
        let (id, open) = window::open(window::Settings {
            size: Size::new(480.0, 680.0),
            min_size: Some(Size::new(400.0, 420.0)),
            ..Default::default()
        });
        self.settings_id = Some(id);
        self.settings_maximized = false;
        open.map(|_| cosmic::Action::App(Message::Noop))
    }

    /// Saves the settings after a change; `rebuild` recreates the widget surface.
    pub(super) fn apply_setting_change(&mut self, rebuild: bool) -> Task<Message> {
        self.settings.save();
        self.confirm_restore = false;
        if rebuild && self.widget_mode() {
            return self.rebuild_surface();
        }
        Task::none()
    }
}
