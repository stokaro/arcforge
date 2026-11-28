//! Native macOS presentation, independent of the archive model and renderer.
//!
//! Commands cross the boundary as owned Rust values. AppKit objects and callbacks
//! stay on the main thread in the private implementation. See the crate README
//! for the audited Objective-C boundary and ownership requirements.

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod macos;

#[cfg(target_os = "macos")]
pub use macos::NativeShell;

/// Intent from an actual AppKit menu, toolbar control, or search field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Command {
    OpenArchive,
    OpenByPath,
    OpenSettings,
    CloseArchive,
    CloseWindow,
    DeleteSelected,
    Undo,
    Redo,
    SaveAs,
    ExtractAll,
    ExtractSelected,
    TestArchive,
    PreviewSelected,
    ViewOutline,
    ViewFlat,
    GoBack,
    GoForward,
    ToggleFavorites,
    ToggleInspector,
    FocusSearch,
    SearchQuery(String),
    SelectAll,
    Cut,
    Copy,
    Paste,
    CancelOperation,
}

/// Presentation state obtained from the workspace, never inferred by AppKit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct State {
    pub title: String,
    pub dirty: bool,
    pub can_delete: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    pub can_save_as: bool,
    pub can_open: bool,
    pub can_close: bool,
    pub can_extract_all: bool,
    pub can_extract_selected: bool,
    pub can_test: bool,
    pub can_preview: bool,
    pub can_select_all: bool,
    pub can_cancel: bool,
    pub can_search: bool,
    pub can_change_view: bool,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub favorites_visible: bool,
    pub inspector_visible: bool,
    pub outline_view: bool,
    pub search_query: String,
}

impl Default for State {
    fn default() -> Self {
        Self {
            title: "ArcForge".into(),
            dirty: false,
            can_delete: false,
            can_undo: false,
            can_redo: false,
            can_save_as: false,
            can_open: true,
            can_close: false,
            can_extract_all: false,
            can_extract_selected: false,
            can_test: false,
            can_preview: false,
            can_select_all: false,
            can_cancel: false,
            can_search: false,
            can_change_view: false,
            can_go_back: false,
            can_go_forward: false,
            favorites_visible: true,
            inspector_visible: true,
            outline_view: true,
            search_query: String::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttachError {
    NotMainThread,
    UnsupportedSystem,
    UnavailableWindowHandle,
    NotAppKitWindow,
    WindowNotFound,
}

impl std::fmt::Display for AttachError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::NotMainThread => "native shell must attach on the macOS main thread",
            Self::UnsupportedSystem => "the unified native shell requires macOS 11 or newer",
            Self::UnavailableWindowHandle => "the desktop window handle is not available",
            Self::NotAppKitWindow => "the desktop handle is not an AppKit view",
            Self::WindowNotFound => "the AppKit window containing the desktop view was not found",
        })
    }
}

impl std::error::Error for AttachError {}

/// Identity of a filesystem sheet attached to this shell's exact window.
#[cfg(feature = "native-capture")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FilePanelInfo {
    pub window_id: u32,
    pub parent_window_id: u32,
    pub is_open: bool,
}
