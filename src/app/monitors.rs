//! The connected monitors and which one the desktop widget is shown on.

use cosmic::cctk::sctk::output::OutputInfo;
use cosmic::cctk::sctk::reexports::client::protocol::wl_output::WlOutput;

use super::App;

/// A monitor as reported by the compositor.
#[derive(Clone, Debug)]
pub(crate) struct Monitor {
    pub(super) wl: WlOutput,
    /// Wayland global id; gives the monitors a stable order.
    global_id: u32,
    make: String,
    model: String,
    name: String,
    /// Logical size in px.
    pub(super) width: i32,
    pub(super) height: i32,
}

impl Monitor {
    pub(super) fn from_info(wl: WlOutput, info: &OutputInfo) -> Self {
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

/// A monitor as shown in the settings: stable id + readable label.
#[derive(Clone, Debug)]
pub(super) struct MonitorChoice {
    pub(super) id: String,
    pub(super) label: String,
    index: usize,
}

impl App {
    fn sorted_monitors(&self) -> Vec<&Monitor> {
        let mut v: Vec<&Monitor> = self.monitors.iter().collect();
        v.sort_by_key(|m| m.global_id);
        v
    }

    /// One entry per monitor. Two identical monitors get ids `make|model` and `make|model#1`.
    pub(super) fn monitor_choices(&self) -> Vec<MonitorChoice> {
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

    /// The wanted monitor, or for "auto": the second one if there is one, else the only one.
    pub(super) fn resolve_monitor(&self) -> Option<Monitor> {
        let sorted = self.sorted_monitors();
        if sorted.is_empty() {
            return None;
        }
        if self.settings.monitor != "auto" {
            if let Some(c) = self
                .monitor_choices()
                .into_iter()
                .find(|c| c.id == self.settings.monitor)
            {
                return sorted.get(c.index).map(|m| (*m).clone());
            }
        }
        sorted.get(1).or(sorted.first()).map(|m| (*m).clone())
    }
}
