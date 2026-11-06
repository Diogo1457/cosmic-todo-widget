//! COSMIC Todo Widget – a to-do list that lives on a monitor as part of the desktop
//! (wlr-layer-shell), or in the COSMIC panel, written with libcosmic.
//!
//! Two ways to show the list (setting "Display"):
//!   * Widget – a sticky note on the desktop: drag the header to move it, drag the ◢
//!     handle (bottom-right) to resize it.
//!   * Panel  – an applet in the COSMIC panel; the list opens as a fixed-size popup.
//!
//! Settings open in their own normal window. While the display is "Widget", clicking the panel
//! applet opens that window (in the running widget, or in a freshly started one).
//!
//! In the list: drag the ≡ handle of a task onto another task to put it there, onto a folder to
//! put it inside, or onto the bottom zone to take it out of its folder. Double-click a task or
//! folder name (or press ✎) to rename it. Click a folder to select it: new tasks typed in the
//! entry field go straight into the selected folder (✕ next to the entry field goes back to the
//! top level). Right-click a row or the empty space for "New task" / "New folder".
//!
//! Source layout:
//!   * `main.rs`      – command line and start-up
//!   * `app/`         – the libcosmic application (state, events, views, geometry, surfaces)
//!   * `settings.rs`  – settings file
//!   * `store.rs`     – tasks / folders file
//!   * `autostart.rs` – "Start at login" entry
//!   * `paths.rs`     – directories, single-instance lock, settings request

mod app;
mod autostart;
mod paths;
mod settings;
mod store;

use app::App;
use settings::{DisplayMode, Settings};

/// Command-line options.
#[derive(Clone, Debug, Default)]
pub struct Args {
    /// Open the settings window on start.
    pub settings: bool,
    /// Run as the COSMIC panel applet (started by cosmic-panel).
    pub applet: bool,
    /// Started quietly at login.
    pub background: bool,
}

const HELP: &str = "cosmic-todo-widget
  --settings     open the settings window on start
  --applet       run as the COSMIC panel applet
  --background   start quietly (nothing happens when the display is \"Panel\")";

fn main() -> cosmic::iced::Result {
    let mut args = Args::default();
    for a in std::env::args().skip(1) {
        match a.as_str() {
            "--settings" => args.settings = true,
            "--applet" => args.applet = true,
            "--background" => args.background = true,
            "-h" | "--help" => {
                println!("{HELP}");
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
