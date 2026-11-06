//! Looks of the containers: list highlights, the widget card and the settings window.

use cosmic::iced::{Background, Border, Color};

use super::App;

/// The system's frosted-glass opacity for the widget's layer surface (1.0 when the user has
/// frosted glass turned off in COSMIC).
pub(super) fn system_alpha() -> f32 {
    let theme = cosmic::theme::active();
    let c = theme.cosmic();
    if c.frosted_system_interface {
        c.alpha_map.blurred_alpha(c.frosted)
    } else {
        1.0
    }
}

/// Highlight behind a list row (while a dragged item is over it, or when it is selected).
pub(super) fn row_style(active: bool) -> cosmic::theme::Container<'static> {
    cosmic::theme::Container::custom(move |theme| {
        let on: Color = theme.cosmic().on_bg_color().into();
        if active {
            cosmic::iced::widget::container::Style {
                background: Some(Background::Color(Color { a: 0.14, ..on })),
                border: Border {
                    radius: 10.0.into(),
                    width: 1.0,
                    color: Color { a: 0.45, ..on },
                },
                ..Default::default()
            }
        } else {
            Default::default()
        }
    })
}

impl App {
    /// The widget card follows the system theme, including its frosted-glass transparency.
    /// Only the opacity can be overridden (Settings → Opacity).
    pub(super) fn card_style(&self) -> cosmic::theme::Container<'static> {
        let custom = self.settings.opacity;
        cosmic::theme::Container::custom(move |theme| {
            let c = theme.cosmic();
            // Layer-shell surfaces count as "system interface" for the frosted glass setting.
            let bg_set = c.background(c.frosted_system_interface);
            let mut bg: Color = bg_set.base.into();
            if let Some(a) = custom {
                bg.a = a;
            }
            let on: Color = bg_set.on.into();
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

    /// Background of the settings window: always the system's values (never the widget's custom
    /// opacity). Maximized / fullscreen windows are opaque unless COSMIC is set to keep them
    /// frosted, exactly like COSMIC's own apps, so nothing shows through.
    pub(super) fn settings_bg(&self) -> cosmic::theme::Container<'static> {
        let maximized = self.settings_maximized;
        cosmic::theme::Container::custom(move |theme| {
            let c = theme.cosmic();
            let frosted = c.frosted_windows && (!maximized || c.frosted_maximized_apps);
            let set = c.background(frosted);
            cosmic::iced::widget::container::Style {
                icon_color: Some(set.on.into()),
                text_color: Some(set.on.into()),
                background: Some(Background::Color(set.base.into())),
                ..Default::default()
            }
        })
    }
}
