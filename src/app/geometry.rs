//! Size and position maths of the desktop widget: keeping the card on the monitor, how tall the
//! list may get, and the resize logic.

use cosmic::iced::Point;

use super::message::DragKind;
use super::monitors::Monitor;
use super::App;
use crate::store::SavedSize;

/// Narrowest the widget can be made (logical px).
pub(super) const MIN_WIDTH: f32 = 380.0;
/// Widest the card can ever be.
const MAX_WIDTH: f32 = 900.0;
/// Height of one single-line task row including the gap to the next one (logical px).
pub(super) const ROW_H: f32 = 50.0;
/// Extra height of every additional wrapped line of a task.
const LINE_H: f32 = 22.0;
/// Height of everything in the card that is not the list (header, progress, entry,
/// footer, paddings and spacing), with a small safety margin.
const CHROME_H: f32 = 262.0;
/// Padding between the card's border and its content.
pub(super) const CARD_PADDING: f32 = 18.0;
/// Pixels kept free at the screen edges.
const SAFETY: f32 = 8.0;
/// Fixed width of the panel popup.
pub(super) const POPUP_W: f32 = 400.0;
/// Fixed list height of the panel popup: room for 5 tasks without scrolling.
pub(super) const POPUP_LIST_H: f32 = ROW_H * 5.0 + 6.0;

impl App {
    pub(super) fn default_w(&self) -> f32 {
        (self.mon_w * 0.20).clamp(MIN_WIDTH + 20.0, 520.0)
    }

    pub(super) fn default_pos(&self) -> Point {
        let m = (self.mon_w.min(self.mon_h) * 0.06).clamp(24.0, 100.0);
        Point::new(m, m)
    }

    /// Widest the card may be at its current x so it stays on the monitor.
    pub(super) fn max_w_at_x(&self) -> f32 {
        (self.mon_w - self.pos.x - SAFETY).min(MAX_WIDTH).max(MIN_WIDTH)
    }

    /// Tallest the whole card may be at its current y.
    pub(super) fn max_card_h_at_y(&self) -> f32 {
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

    /// How tall the automatic size lets the list grow before it scrolls (from the monitor height).
    fn auto_cap(&self) -> f32 {
        let rows = (self.mon_h * 0.45 / ROW_H).floor().clamp(4.0, 12.0);
        (rows * ROW_H).min(self.full_cap())
    }

    /// Estimated natural height of the list (wrapped lines are estimated).
    fn natural_list_h(&self) -> f32 {
        let text_w = (self.cur_w - 2.0 * CARD_PADDING - 190.0).max(100.0);
        let chars_per_line = (text_w / 8.0).floor().max(8.0) as usize;
        let mut h = ROW_H * self.store.folders.len() as f32;
        for t in &self.store.tasks {
            let hidden = t
                .folder
                .as_ref()
                .is_some_and(|fid| self.store.folders.iter().any(|f| f.collapsed && f.id == *fid));
            if hidden {
                continue;
            }
            let lines = t.text.chars().count().div_ceil(chars_per_line).max(1);
            h += ROW_H + (lines as f32 - 1.0) * LINE_H;
        }
        h
    }

    /// The list height the user (or the automatic mode) wants, independent of position.
    fn desired_list_h(&self) -> f32 {
        match self.manual_list_h {
            Some(h) => h.clamp(ROW_H, self.full_cap()),
            None => self.natural_list_h().clamp(ROW_H, self.auto_cap()),
        }
    }

    /// The list height actually used: the wanted height, but never past the screen bottom.
    pub(super) fn list_h(&self) -> f32 {
        self.desired_list_h().min(self.avail_list_h()).max(ROW_H)
    }

    /// Keeps the card fully on the monitor.
    pub(super) fn clamp_pos(&mut self) {
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

    /// Sets width / list height from the pointer offset since the drag started.
    pub(super) fn apply_resize(&mut self, w0: f32, h0: f32, start: Point, p: Point) {
        let max_w = self.max_w_at_x();
        let cap = self.full_cap().min(self.avail_list_h());
        // Whole pixels only: sub-pixel changes would still trigger a surface resize.
        self.cur_w = (w0 + (p.x - start.x)).clamp(MIN_WIDTH, max_w).round();
        self.manual_list_h = Some((h0 + (p.y - start.y)).clamp(ROW_H, cap).round());
    }

    /// Remembers position and size for the next start.
    pub(super) fn persist_ui(&mut self) {
        self.store.set_ui(SavedSize {
            x: Some(self.pos.x),
            y: Some(self.pos.y),
            width: Some(self.cur_w),
            list_h: self.manual_list_h,
        });
    }

    /// Recomputes everything that depends on the monitor and clamps size and position.
    /// With `first`, size and position are also restored from the saved values.
    pub(super) fn recompute_geometry(&mut self, monitor: Option<&Monitor>, first: bool) {
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

    /// True while the resize handle is being dragged.
    pub(super) fn resizing(&self) -> bool {
        self.drag.as_ref().is_some_and(|d| d.kind == DragKind::Resize)
    }
}
