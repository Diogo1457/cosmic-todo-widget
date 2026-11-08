//! The settings window, built from COSMIC's settings widgets so it follows the system theme.

use cosmic::iced::{Alignment, Length};
use cosmic::widget;
use cosmic::Element;

use crate::app::message::Message;
use crate::app::App;
use crate::paths;
use crate::settings::{DisplayMode, MIN_OPACITY};

/// A full-width button that looks pressed while `selected` (a radio-button replacement).
fn choice(label: &str, selected: bool, msg: Message) -> Element<'static, Message> {
    let button = if selected {
        widget::button::suggested(label.to_string())
    } else {
        widget::button::standard(label.to_string())
    };
    button.on_press(msg).width(Length::Fill).into()
}

/// Gives a row inside a settings section the same padding as the section's own items.
fn padded(e: Element<'static, Message>) -> Element<'static, Message> {
    widget::container(e)
        .padding([8, 16])
        .width(Length::Fill)
        .into()
}

impl App {
    /// Content of the separate settings window. Monitor, placement and opacity only exist while
    /// the display is "Widget".
    pub(in crate::app) fn settings_window(&self) -> Element<'_, Message> {
        let s = &self.settings;
        let widget_mode = s.display == DisplayMode::Widget;

        let can_widget = paths::widget_supported();
        let mut display = widget::settings::section().title("Display");
        if can_widget {
            display = display.add(padded(
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
                    ))
                    .into(),
            ));
        } else {
            display = display.add(padded(
                widget::text::caption(
                    "The desktop widget is not available in the Flatpak version: COSMIC does not allow sandboxed apps to draw on the desktop. The list is shown in the panel. For the desktop widget install the native version (see the README).",
                )
                .into(),
            ));
        }
        let display = display
            .add(padded(
                widget::text::caption(if widget_mode {
                    "A sticky note on the desktop: drag its header to move it, resize it with the ◢ handle in the bottom-right corner (double-click it for automatic height). The panel applet opens this window."
                } else {
                    "A popup of the COSMIC panel with room for 5 tasks. Add the \"Todo Widget\" applet to your panel in COSMIC Settings → Desktop → Panel (or Dock) → Configure panel applets."
                })
                .into(),
            ));

        let mut page = widget::Column::new()
            .spacing(24)
            .width(Length::Fill)
            .padding([0, 16, 0, 0])
            .push(display)
            .push(
                widget::settings::section().title("Header").add(
                    widget::settings::item::builder("Show clock and date")
                        .toggler(s.show_clock, Message::SetClock),
                ),
            );

        if widget_mode {
            let mut placement = widget::settings::section()
                .title("Monitor")
                .add(padded(choice(
                    "Auto (second monitor if present)",
                    s.monitor == "auto",
                    Message::SetMonitor("auto".into()),
                )));
            for c in self.monitor_choices() {
                placement = placement.add(padded(choice(
                    &c.label,
                    s.monitor == c.id,
                    Message::SetMonitor(c.id.clone()),
                )));
            }
            let layer = widget::settings::section().title("Placement").add(
                widget::settings::item::builder("Keep above windows")
                    .description("Off: part of the desktop, hidden when windows cover it")
                    .toggler(s.above_windows, Message::SetAbove),
            );
            let mut opacity = widget::settings::section().title("Opacity").add(
                widget::settings::item::builder("Use system transparency")
                    .description("Follows the frosted glass setting of COSMIC (Settings → Desktop → Appearance)")
                    .toggler(s.opacity.is_none(), Message::SetOpacitySystem),
            );
            if let Some(a) = s.opacity {
                let lo = (MIN_OPACITY * 100.0).round() as u8;
                let pct = ((a * 100.0).round() as u8).clamp(lo, 100);
                opacity = opacity.add(
                    widget::settings::item::builder(format!("Widget opacity: {pct} %")).control(
                        widget::slider(lo..=100, pct, Message::SetOpacity)
                            .width(Length::Fixed(200.0)),
                    ),
                );
            }
            page = page.push(placement).push(layer).push(opacity);
        }

        // "Start at login" only starts the desktop widget, so it is pointless in a Flatpak.
        if can_widget {
            page = page.push(
                widget::settings::section().title("Startup").add(
                    widget::settings::item::builder("Start at login")
                        .description("Off by default (the panel applet starts with the panel)")
                        .toggler(s.autostart, Message::SetAutostart),
                ),
            );
        }
        let reset = widget::settings::section().title("Reset").add(
            widget::container(self.restore_view())
                .padding([8, 16])
                .width(Length::Fill),
        );
        page = page.push(reset);

        let header = widget::Row::new()
            .align_y(Alignment::Center)
            .push(widget::text::title3("Settings").width(Length::Fill))
            .push(widget::button::suggested("Done").on_press(Message::CloseSettings));

        widget::container(
            widget::Column::new()
                .spacing(16)
                .push(header)
                .push(widget::scrollable(page).height(Length::Fill)),
        )
        .padding(20)
        .width(Length::Fill)
        .height(Length::Fill)
        .class(self.settings_bg())
        .into()
    }

    /// The "Restore defaults" button and its confirmation. Both states have the same height so
    /// the confirmation never scrolls out of view.
    fn restore_view(&self) -> Element<'_, Message> {
        if self.confirm_restore {
            widget::Column::new()
                .spacing(8)
                .push(
                    widget::Row::new()
                        .spacing(10)
                        .push(widget::button::destructive("Restore defaults").on_press(Message::RestoreDefaults))
                        .push(widget::button::standard("Cancel").on_press(Message::CancelRestore)),
                )
                .push(widget::text::caption(
                    "Really reset settings, position and size? Tasks, folders and the display choice are kept.",
                ))
                .into()
        } else {
            widget::Column::new()
                .spacing(8)
                .push(widget::button::destructive("Restore defaults…").on_press(Message::AskRestore))
                .push(widget::text::caption(
                    "Resets settings, position and size. Tasks, folders and the display choice are not touched.",
                ))
                .into()
        }
    }
}
