//! Every event the UI or the system can send to the application, plus the small types they carry.

use std::time::Instant;

use cosmic::cctk::sctk::reexports::client::protocol::wl_output::WlOutput;
use cosmic::iced::event::wayland;
use cosmic::iced::{window, Point};

use crate::settings::DisplayMode;
use crate::store::{DropTarget, ItemRef};

/// What was right-clicked (decides where "New task" puts the task).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ContextTarget {
    Task(String),
    Folder(String),
    /// Empty space in the list.
    Background,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DragKind {
    Move,
    Resize,
}

/// A move or resize of the desktop widget in progress.
#[derive(Clone, Debug)]
pub(crate) struct Drag {
    pub(super) kind: DragKind,
    /// Pointer position when the drag really began (the first pointer event after the press).
    pub(super) start: Option<Point>,
    /// Width and list height when the drag began.
    pub(super) w0: f32,
    pub(super) h0: f32,
    /// When the last move request was sent to the compositor.
    pub(super) last_apply: Instant,
    /// Latest pointer position seen (used to apply the final size).
    pub(super) last: Point,
}

/// A task or folder being dragged inside the list, and what it is currently over.
#[derive(Clone, Debug)]
pub(crate) struct ItemDrag {
    pub(super) item: ItemRef,
    pub(super) over: Option<DropTarget>,
}

#[derive(Clone, Debug)]
#[allow(clippy::large_enum_variant)] // the Wayland output event is big but rare
pub(crate) enum Message {
    /// Does nothing (results of tasks we do not care about).
    Noop,
    /// Clock / applet refresh.
    Tick,
    /// The desktop widget checks whether the panel applet asked for the settings window.
    CheckRequests,
    Output(wayland::OutputEvent, WlOutput),
    OutputsSettled(u64),
    SurfaceLost,

    // tasks
    Input(String),
    Add,
    Toggle(String, bool),
    Delete(String),
    ClearDone,

    // rename
    EditStart(ItemRef),
    EditInput(String),
    EditCommit,
    EditCancel,
    /// A click on empty space while renaming or with the right-click menu open.
    EditClickAway,

    // folders
    AddFolder,
    ToggleFolder(String),
    /// Select a folder: new tasks go inside it.
    SelectFolder(String),
    /// Back to adding tasks at the top level.
    ClearTarget,
    DeleteFolder(String),
    ConfirmDeleteFolder(String),
    CancelDeleteFolder,

    // right-click menu
    Context(ContextTarget),
    CtxNewTask,
    CtxNewFolder,

    // drag and drop inside the list
    DragStart(ItemRef),
    DragOver(DropTarget),
    DragLeave(DropTarget),
    ItemDrop,

    // settings window
    OpenSettings,
    CloseSettings,

    // settings
    SetDisplay(DisplayMode),
    SetClock(bool),
    /// `true` = follow the system's transparency.
    SetOpacitySystem(bool),
    /// Custom widget opacity in percent.
    SetOpacity(u8),
    SetMonitor(String),
    SetAbove(bool),
    SetAutostart(bool),
    AskRestore,
    CancelRestore,
    RestoreDefaults,

    // move / resize of the desktop widget
    MovePress,
    GripPress,
    GripRelease,
    GripAuto,
    Pointer(Point),

    // panel applet
    AppletPressed,
    PopupClosed(window::Id),

    /// A window became (or stopped being) maximized / fullscreen.
    WindowState(window::Id, bool),

    /// Close the desktop widget.
    Quit,
}
