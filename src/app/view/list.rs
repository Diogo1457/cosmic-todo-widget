//! The to-do list itself: entry field, folder and task rows, context menu, tool buttons.

use cosmic::iced::{Alignment, Color, Length, mouse};
use cosmic::widget;
use cosmic::Element;

use crate::app::geometry::ROW_H;
use crate::app::message::{ContextTarget, Message};
use crate::app::style::row_style;
use crate::app::{App, EDIT_ID, ENTRY_ID};
use crate::store::{self, DropTarget, ItemRef};

/// The ≡ handle: press it to start dragging a task or folder.
fn drag_handle(item: ItemRef) -> Element<'static, Message> {
    widget::mouse_area(widget::container(widget::text("≡").size(18)).padding([4, 6]))
        .on_press(Message::DragStart(item))
        .interaction(mouse::Interaction::Grab)
        .into()
}

impl App {
    /// Progress bar and the "x of y done" counter.
    pub(in crate::app) fn progress_block(&self) -> Element<'_, Message> {
        let (done, total) = self.store.progress();
        let fraction = if total > 0 { done as f32 / total as f32 } else { 0.0 };
        let counter = widget::text::caption(if total > 0 {
            format!("{done} of {total} done")
        } else {
            "No tasks yet".to_string()
        });
        widget::Column::new()
            .spacing(6)
            .push(widget::progress_bar::determinate_linear(fraction).width(Length::Fill))
            .push(counter)
            .into()
    }

    /// The "add a task" field. While a folder is selected the placeholder names it and a ✕
    /// button goes back to the top level.
    pub(in crate::app) fn entry_row(&self) -> Element<'_, Message> {
        let target = self.target_folder();
        let placeholder = match target.as_deref().and_then(|id| self.store.folder_name(id)) {
            Some(name) => format!("Add a task to “{name}”…"),
            None => "Add a task…".to_string(),
        };
        let mut row = widget::Row::new().spacing(10).align_y(Alignment::Center).push(
            widget::text_input(placeholder, &self.entry)
                .id(ENTRY_ID.clone())
                .on_input(Message::Input)
                .on_submit(|_| Message::Add)
                .width(Length::Fill),
        );
        if target.is_some() {
            row = row.push(widget::button::text("✕").on_press(Message::ClearTarget));
        }
        row.push(widget::button::suggested("Add").on_press(Message::Add)).into()
    }

    /// "Clear done", "New folder" and "Settings".
    pub(in crate::app) fn tools_row(&self) -> Element<'_, Message> {
        let any_done = self.store.progress().0 > 0;
        widget::Row::new()
            .spacing(8)
            .align_y(Alignment::Center)
            .push(widget::button::text("Clear done").on_press_maybe(any_done.then_some(Message::ClearDone)))
            .push(widget::button::text("New folder").on_press(Message::AddFolder))
            .push(widget::button::text("Settings").on_press(Message::OpenSettings))
            .into()
    }

    /// Tasks of one group: pending first, completed at the bottom (order otherwise kept).
    fn ordered(&self, folder: Option<&str>) -> Vec<&store::Task> {
        let mut v = self.store.tasks_in(folder);
        v.sort_by_key(|t| t.done);
        v
    }

    /// Text field used while renaming a task or folder.
    fn edit_field(&self, placeholder: &'static str) -> Element<'_, Message> {
        let value = self.editing.as_ref().map(|(_, s)| s.as_str()).unwrap_or("");
        widget::text_input(placeholder, value)
            .id(EDIT_ID.clone())
            .on_input(Message::EditInput)
            .on_submit(|_| Message::EditCommit)
            .width(Length::Fill)
            .into()
    }

    /// Wraps a list row so dragged items can be dropped on it (highlighted while hovered or
    /// `selected`). Right-clicking opens the context menu. `on_press` is sent for clicks on the
    /// row that none of its own widgets handled (used to select a folder by clicking its row).
    fn drop_zone<'a>(
        &self,
        content: Element<'a, Message>,
        target: DropTarget,
        selected: bool,
        on_press: Option<Message>,
    ) -> Element<'a, Message> {
        let over = self.item_drag.as_ref().is_some_and(|d| d.over.as_ref() == Some(&target));
        let ctx = match &target {
            DropTarget::Task(id) => Some(ContextTarget::Task(id.clone())),
            DropTarget::Folder(id) => Some(ContextTarget::Folder(id.clone())),
            DropTarget::RootEnd => None,
        };
        let framed = widget::container(content)
            .width(Length::Fill)
            .class(row_style(over || selected));
        let mut area = widget::mouse_area(framed);
        if let Some(c) = ctx {
            area = area.on_right_press(Message::Context(c));
        }
        if let Some(msg) = on_press {
            area = area.on_press(msg);
        }
        if self.item_drag.is_some() {
            area = area
                .on_enter(Message::DragOver(target.clone()))
                .on_exit(Message::DragLeave(target));
        }
        area.into()
    }

    /// The small right-click menu, shown right below the row that was clicked.
    fn context_menu(&self) -> Element<'static, Message> {
        let row = widget::Row::new()
            .spacing(8)
            .align_y(Alignment::Center)
            .push(widget::button::text("+ New task").on_press(Message::CtxNewTask))
            .push(widget::button::text("+ New folder").on_press(Message::CtxNewFolder));
        widget::container(row).padding([4, 10]).class(row_style(true)).into()
    }

    /// A folder header: drag handle, expand arrow, name, counter, rename and delete.
    /// Clicking anywhere on it (outside the buttons) selects the folder.
    fn folder_row(&self, f: &store::Folder) -> Element<'_, Message> {
        let id = f.id.clone();
        let (done, total) = self.store.folder_progress(&f.id);
        let drop_target = DropTarget::Folder(f.id.clone());

        // Waiting for the confirmation to delete a folder together with its tasks.
        if self.confirm_folder.as_deref() == Some(f.id.as_str()) {
            let row = widget::Row::new()
                .spacing(8)
                .align_y(Alignment::Center)
                .padding([7, 6])
                .push(
                    widget::text::body(format!(
                        "Delete “{}” and its {} task{}?",
                        f.name,
                        total,
                        if total == 1 { "" } else { "s" }
                    ))
                    .width(Length::Fill),
                )
                .push(widget::button::destructive("Delete").on_press(Message::ConfirmDeleteFolder(id.clone())))
                .push(widget::button::standard("Cancel").on_press(Message::CancelDeleteFolder));
            return self.drop_zone(row.into(), drop_target, false, None);
        }

        let editing = matches!(&self.editing, Some((ItemRef::Folder(e), _)) if *e == f.id);
        let selected = self.target_folder().as_deref() == Some(f.id.as_str());
        let mut row = widget::Row::new().spacing(6).align_y(Alignment::Center).padding([7, 6]);
        row = row.push(drag_handle(ItemRef::Folder(id.clone())));
        row = row.push(
            widget::button::text(if f.collapsed { "▸" } else { "▾" })
                .on_press(Message::ToggleFolder(id.clone())),
        );
        if editing {
            row = row
                .push(self.edit_field("Folder name"))
                .push(widget::button::text("✓").on_press(Message::EditCommit))
                .push(widget::button::text("✕").on_press(Message::EditCancel));
        } else {
            // Click selects the folder (new tasks go inside it); double-click renames it.
            let label = widget::mouse_area(widget::text::heading(f.name.clone()).width(Length::Fill))
                .on_press(Message::SelectFolder(id.clone()))
                .on_double_click(Message::EditStart(ItemRef::Folder(id.clone())));
            row = row
                .push(label)
                .push(widget::text::caption(format!("{done}/{total}")))
                .push(widget::button::text("✎").on_press(Message::EditStart(ItemRef::Folder(id.clone()))))
                .push(widget::button::text("✕").on_press(Message::DeleteFolder(id.clone())));
        }
        self.drop_zone(row.into(), drop_target, selected, Some(Message::SelectFolder(id)))
    }

    /// A task row: drag handle, checkbox, text (double-click renames), rename and delete.
    fn task_row(&self, t: &store::Task, indented: bool) -> Element<'_, Message> {
        let id = t.id.clone();
        let toggle_id = t.id.clone();
        let editing = matches!(&self.editing, Some((ItemRef::Task(e), _)) if *e == t.id);

        let mut row = widget::Row::new().spacing(6).align_y(Alignment::Center);
        row = if indented { row.padding([7, 6, 7, 28]) } else { row.padding([7, 6, 7, 6]) };
        row = row
            .push(drag_handle(ItemRef::Task(id.clone())))
            .push(widget::checkbox(t.done).on_toggle(move |v| Message::Toggle(toggle_id.clone(), v)));

        if editing {
            row = row
                .push(self.edit_field("Task"))
                .push(widget::button::text("✓").on_press(Message::EditCommit))
                .push(widget::button::text("✕").on_press(Message::EditCancel));
        } else {
            // Completed tasks are dimmed.
            let label = if t.done {
                widget::text::body(t.text.clone()).class(cosmic::theme::Text::Custom(|theme| {
                    let mut c: Color = theme.cosmic().on_bg_color().into();
                    c.a = 0.45;
                    cosmic::iced::widget::text::Style { color: Some(c), ..Default::default() }
                }))
            } else {
                widget::text::body(t.text.clone())
            };
            let label = widget::mouse_area(label.width(Length::Fill))
                .on_double_click(Message::EditStart(ItemRef::Task(id.clone())));
            row = row
                .push(label)
                .push(widget::button::text("✎").on_press(Message::EditStart(ItemRef::Task(id.clone()))))
                .push(widget::button::text("✕").on_press(Message::Delete(id.clone())));
        }
        self.drop_zone(row.into(), DropTarget::Task(t.id.clone()), false, None)
    }

    /// The scrollable list (folders with their tasks first, then top-level tasks) with a fixed
    /// height.
    pub(in crate::app) fn task_list(&self, height: f32) -> Element<'_, Message> {
        // Right padding keeps the text clear of the scrollbar.
        let mut list = widget::Column::new().spacing(4).width(Length::Fill).padding([0, 14, 0, 0]);
        if self.store.tasks.is_empty() && self.store.folders.is_empty() {
            list = list.push(
                widget::container(widget::text::body("Nothing to do - enjoy the day"))
                    .width(Length::Fill)
                    .height(Length::Fixed(ROW_H))
                    .align_x(Alignment::Center)
                    .align_y(Alignment::Center),
            );
        }
        for f in &self.store.folders {
            list = list.push(self.folder_row(f));
            if self.context == Some(ContextTarget::Folder(f.id.clone())) {
                list = list.push(self.context_menu());
            }
            if !f.collapsed {
                let inside = self.ordered(Some(f.id.as_str()));
                if inside.is_empty() {
                    list = list.push(
                        widget::container(widget::text::caption("Empty - select the folder and add a task"))
                            .padding([2, 0, 6, 40]),
                    );
                }
                for t in inside {
                    list = list.push(self.task_row(t, true));
                    if self.context == Some(ContextTarget::Task(t.id.clone())) {
                        list = list.push(self.context_menu());
                    }
                }
            }
        }
        for t in self.ordered(None) {
            list = list.push(self.task_row(t, false));
            if self.context == Some(ContextTarget::Task(t.id.clone())) {
                list = list.push(self.context_menu());
            }
        }
        if self.context == Some(ContextTarget::Background) {
            list = list.push(self.context_menu());
        }
        if self.item_drag.is_some() {
            let zone = widget::container(widget::text::caption("Drop here: outside folders, at the end"))
                .width(Length::Fill)
                .height(Length::Fixed(ROW_H))
                .align_x(Alignment::Center)
                .align_y(Alignment::Center);
            list = list.push(self.drop_zone(zone.into(), DropTarget::RootEnd, false, None));
        }
        // Right-click on empty space (rows handle their own right-click).
        widget::mouse_area(widget::scrollable(list).height(Length::Fixed(height)))
            .on_right_press(Message::Context(ContextTarget::Background))
            .into()
    }
}
