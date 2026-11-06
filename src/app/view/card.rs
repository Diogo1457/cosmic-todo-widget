//! The two containers the list is shown in: the desktop widget card and the panel popup.

use cosmic::iced::{Alignment, Length, Limits, mouse};
use cosmic::widget::{self, autosize};
use cosmic::Element;

use crate::app::geometry::{CARD_PADDING, POPUP_LIST_H, POPUP_W};
use crate::app::message::Message;
use crate::app::{App, AUTOSIZE_ID};

impl App {
    /// Title (with the date if the clock is on).
    fn title_block(&self) -> Element<'_, Message> {
        let mut col = widget::Column::new()
            .spacing(6)
            .push(widget::text::title2("Today"))
            .width(Length::Fill);
        if self.settings.show_clock {
            col = col.push(widget::text::caption(self.now.format("%A, %d %B %Y").to_string()));
        }
        col.into()
    }

    /// Header: title, the clock if enabled, and the close button on the widget.
    fn header_row(&self, with_quit: bool) -> Element<'_, Message> {
        let mut row = widget::Row::new()
            .align_y(Alignment::Start)
            .spacing(12)
            .push(self.title_block());
        if self.settings.show_clock {
            row = row.push(widget::text(self.now.format("%H:%M").to_string()).size(30));
        }
        if with_quit {
            row = row.push(widget::button::text("✕").on_press(Message::Quit));
        }
        row.into()
    }

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

    /// Content of the desktop widget card.
    fn tasks_view(&self) -> Element<'_, Message> {
        // The header is the "sticky note" drag handle.
        let header: Element<'_, Message> = widget::mouse_area(self.header_row(true))
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

    /// The card: content on a rounded, theme-coloured background of the current width.
    fn card(&self) -> Element<'_, Message> {
        widget::container(self.tasks_view())
            .padding(CARD_PADDING)
            .width(Length::Fixed(self.cur_w))
            .class(self.card_style())
            .into()
    }

    /// Content of the desktop widget's layer surface.
    pub(in crate::app) fn widget_window(&self) -> Element<'_, Message> {
        // Always the same widget structure (a different one would break the widget tree);
        // while resizing, the content is a fixed-size transparent area so the surface itself
        // never has to change size.
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

    /// Content of the panel popup (fixed size, room for 5 tasks).
    pub(in crate::app) fn popup_view(&self) -> Element<'_, Message> {
        let content = widget::Column::new()
            .spacing(14)
            .width(Length::Fill)
            .push(self.header_row(false))
            .push(self.progress_block())
            .push(self.entry_row())
            .push(self.task_list(POPUP_LIST_H))
            .push(self.tools_row());
        self.core
            .applet
            .popup_container(widget::container(content).padding(16).width(Length::Fixed(POPUP_W)))
            .into()
    }
}
