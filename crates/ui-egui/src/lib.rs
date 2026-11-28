//! Desktop presentation over shared archive browsing and extraction commands.

mod filesystem;
mod jobs;
mod preview_image;
pub mod typography;

#[cfg(feature = "native-capture")]
pub mod native_capture;

#[cfg(test)]
mod tests;

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll, Wake, Waker};

use arcforge_core::archive::{
    ArchiveEntry, ArchiveError, CancellationToken, EntryId, EntryKind, ExtractionLimits,
    ExtractionPhase, ExtractionProgress, ExtractionReport, ExtractionSelection, ExtractionStatus,
    FilenameChoice, FilenameEncoding, FilenamePolicy, NameProvenance, NameState, escape_display,
};
use arcforge_core::{Availability, COMMANDS, VERSION};
use filesystem::FilesystemBrowser;
use jobs::{
    Completed, PreparedDocument, PreparedPreview, ProgressReply, Reply, Request, Task, Ticket,
    ViewMode, ViewRow, Worker,
};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

struct PendingNativePicker {
    generation: u64,
    purpose: NativePickerPurpose,
    future: Pin<Box<dyn Future<Output = Option<PathBuf>> + Send>>,
    waker: Waker,
}

struct PickerRepaint(egui::Context);

impl Wake for PickerRepaint {
    fn wake(self: Arc<Self>) {
        self.0.request_repaint();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.request_repaint();
    }
}

/// Commands shared by a native menu/toolbar and the portable workspace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkspaceCommand {
    OpenArchive,
    OpenByPath,
    OpenSettings,
    CloseArchive,
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
    CancelOperation,
    Cut,
    Copy,
    Paste,
}

/// Actual controller state for enabling native controls without duplicating logic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolbarState {
    pub title: String,
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

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum RowKey {
    Entry(EntryId),
    Folder(String),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum ExtractionScope {
    #[default]
    Selected,
    All,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NativePickerPurpose {
    OpenArchive,
    ExtractionParent {
        scope: ExtractionScope,
        keep_name: bool,
    },
}

#[derive(Default)]
enum ExtractionState {
    #[default]
    Editing,
    Running {
        progress: Option<ExtractionProgress>,
        cancel_requested: bool,
    },
    Finished(Result<ExtractionReport, ArchiveError>),
    Interrupted(String),
}

#[derive(Default)]
struct ExtractionDialog {
    open: bool,
    scope: ExtractionScope,
    destination: PathBuf,
    name_input: String,
    state: ExtractionState,
}

#[derive(Clone, Copy)]
enum FavoritesRowKind {
    Directory {
        expanded: bool,
        count: Option<usize>,
    },
    Archive,
    Notice,
}

#[derive(Clone)]
struct FavoritesRow {
    path: PathBuf,
    label: String,
    depth: usize,
    kind: FavoritesRowKind,
}

pub struct ArchiveWorkspace {
    initial_path: Option<PathBuf>,
    worker: Option<Worker>,
    document: Option<Arc<PreparedDocument>>,
    rows: Vec<ViewRow>,
    selected: Option<EntryId>,
    selected_folder: Option<String>,
    selected_entries: BTreeSet<EntryId>,
    selected_folders: BTreeSet<String>,
    selection_anchor: Option<RowKey>,
    view_mode: ViewMode,
    expanded_folders: BTreeSet<String>,
    applied_view_mode: ViewMode,
    applied_expanded_folders: BTreeSet<String>,
    navigation: Vec<Option<String>>,
    navigation_index: usize,
    recent_archives: Vec<PathBuf>,
    favorites_query: String,
    filesystem: Option<FilesystemBrowser>,
    filesystem_initialized: bool,
    filesystem_startup_expanded: bool,
    filesystem_error: Option<String>,
    expanded_places: BTreeSet<PathBuf>,
    selected_place: Option<PathBuf>,
    favorites_rows: Vec<FavoritesRow>,
    favorites_content_width: f32,
    show_sidebar: bool,
    sidebar_width: f32,
    show_search: bool,
    show_settings: bool,
    native_toolbar_attached: bool,
    style_initialized: bool,
    extraction: ExtractionDialog,
    preview: Option<PreparedPreview>,
    preview_texture: Option<egui::TextureHandle>,
    preview_show_image: bool,
    preview_hex: bool,
    policy: FilenamePolicy,
    query: String,
    folder: Option<String>,
    applied_query: String,
    applied_folder: Option<String>,
    path_input: String,
    show_open_path: bool,
    native_picker_request: Option<NativePickerPurpose>,
    native_picker: Option<PendingNativePicker>,
    show_inspector: bool,
    show_commands: bool,
    generation: u64,
    request: u64,
    current: Option<Ticket>,
    blocks_selection: bool,
    source_changed: bool,
    status: String,
    error: Option<String>,
}

impl Default for ArchiveWorkspace {
    fn default() -> Self {
        Self::new(None)
    }
}

impl ArchiveWorkspace {
    pub fn new(initial_path: Option<PathBuf>) -> Self {
        Self {
            initial_path,
            worker: None,
            document: None,
            rows: Vec::new(),
            selected: None,
            selected_folder: None,
            selected_entries: BTreeSet::new(),
            selected_folders: BTreeSet::new(),
            selection_anchor: None,
            view_mode: ViewMode::Outline,
            expanded_folders: BTreeSet::new(),
            applied_view_mode: ViewMode::Outline,
            applied_expanded_folders: BTreeSet::new(),
            navigation: vec![None],
            navigation_index: 0,
            recent_archives: Vec::new(),
            favorites_query: String::new(),
            filesystem: None,
            filesystem_initialized: false,
            filesystem_startup_expanded: false,
            filesystem_error: None,
            expanded_places: BTreeSet::new(),
            selected_place: None,
            favorites_rows: Vec::new(),
            favorites_content_width: 0.0,
            show_sidebar: true,
            sidebar_width: 0.0,
            show_search: false,
            show_settings: false,
            native_toolbar_attached: false,
            style_initialized: false,
            extraction: ExtractionDialog::default(),
            preview: None,
            preview_texture: None,
            preview_show_image: false,
            preview_hex: false,
            policy: FilenamePolicy::default(),
            query: String::new(),
            folder: None,
            applied_query: String::new(),
            applied_folder: None,
            path_input: String::new(),
            show_open_path: false,
            native_picker_request: None,
            native_picker: None,
            show_inspector: true,
            show_commands: false,
            generation: 0,
            request: 0,
            current: None,
            blocks_selection: false,
            source_changed: false,
            status: "Open a ZIP or drop one file into the workspace.".into(),
            error: None,
        }
    }

    pub fn toolbar_state(&self) -> ToolbarState {
        let idle = !self.extraction_active() && !self.native_picker_pending();
        let has_archive = self.document.is_some();
        let ready = has_archive && !self.blocks_selection && !self.source_changed && idle;
        ToolbarState {
            title: self.document.as_ref().map_or_else(
                || "Untitled".into(),
                |document| {
                    bounded_display(
                        &document
                            .snapshot
                            .path()
                            .file_name()
                            .unwrap_or(OsStr::new("Archive"))
                            .to_string_lossy(),
                    )
                },
            ),
            can_open: idle,
            can_close: has_archive && idle,
            can_extract_all: ready,
            can_extract_selected: ready && self.selection_count() > 0,
            can_test: false,
            can_preview: ready
                && self
                    .selected
                    .and_then(|id| self.document.as_ref()?.entry(id))
                    .is_some_and(|entry| !entry.is_directory),
            can_select_all: has_archive && !self.blocks_selection && idle,
            can_cancel: self.current.is_some()
                && !self.native_picker_pending()
                && !matches!(
                    self.extraction.state,
                    ExtractionState::Running {
                        cancel_requested: true,
                        ..
                    }
                ),
            can_search: has_archive && !self.source_changed && idle,
            can_change_view: has_archive && !self.source_changed && idle,
            can_go_back: idle && self.navigation_index > 0,
            can_go_forward: idle && self.navigation_index + 1 < self.navigation.len(),
            favorites_visible: self.show_sidebar,
            inspector_visible: self.show_inspector,
            outline_view: self.view_mode == ViewMode::Outline,
            search_query: self.query.clone(),
        }
    }

    /// Opt in from a real desktop host. Rendering alone never scans local folders.
    pub fn enable_filesystem_favorites(&mut self, context: &egui::Context) {
        if self.filesystem_initialized {
            return;
        }
        self.filesystem_initialized = true;
        match FilesystemBrowser::new(context.clone()) {
            Ok(browser) => self.filesystem = Some(browser),
            Err(error) => {
                self.filesystem_error = Some(format!("Could not load Favorites: {error}"))
            }
        }
    }

    /// Capture only after discovery and the one initial place scan have settled.
    pub fn filesystem_favorites_ready(&self) -> bool {
        self.filesystem_startup_expanded
            && self.filesystem.as_ref().is_some_and(|browser| {
                !browser.roots_pending()
                    && self
                        .expanded_places
                        .iter()
                        .all(|path| !browser.pending(path))
            })
    }

    pub fn filesystem_favorites_startup_error(&self) -> Option<&str> {
        self.filesystem
            .is_none()
            .then_some(self.filesystem_error.as_deref())
            .flatten()
    }

    fn poll_filesystem_favorites(&mut self) {
        let Some(browser) = &mut self.filesystem else {
            return;
        };
        let mut changed = browser.poll();
        if !self.filesystem_startup_expanded && !browser.roots_pending() {
            self.filesystem_startup_expanded = true;
            let initial = browser
                .roots()
                .iter()
                .find(|place| place.label == "Downloads")
                .or_else(|| browser.roots().first())
                .map(|place| place.path.clone());
            if let Some(path) = initial {
                self.expanded_places.insert(path.clone());
                self.selected_place = Some(path.clone());
                if let Err(error) = browser.request(path) {
                    self.filesystem_error = Some(error);
                }
            }
            changed = true;
        }
        if changed {
            self.rebuild_favorites_rows();
        }
    }

    fn rebuild_favorites_rows(&mut self) {
        self.favorites_rows = self.filesystem.as_ref().map_or_else(Vec::new, |browser| {
            build_favorites_rows(browser, &self.expanded_places, &self.favorites_query)
        });
        self.favorites_content_width = self
            .favorites_rows
            .iter()
            .map(|row| 180.0 + row.depth as f32 * 15.0)
            .fold(0.0, f32::max);
    }

    fn toggle_place(&mut self, path: PathBuf) {
        let Some(browser) = &mut self.filesystem else {
            return;
        };
        self.selected_place = Some(path.clone());
        if !self.expanded_places.remove(&path) {
            if self.expanded_places.len() == 32 {
                self.filesystem_error = Some("Close a folder before opening another.".into());
                return;
            }
            // Keep expanded cached folders recent without requesting evicted ones.
            // This lets the bounded cache evict a closed folder first.
            for expanded in &self.expanded_places {
                if browser.listing(expanded).is_some() || browser.pending(expanded) {
                    let _ = browser.request(expanded.clone());
                }
            }
            match browser.request(path.clone()) {
                Ok(()) => {
                    self.expanded_places.insert(path);
                    self.filesystem_error = None;
                }
                Err(error) => self.filesystem_error = Some(error),
            }
        }
        self.rebuild_favorites_rows();
    }

    fn refresh_place(&mut self, path: PathBuf) {
        if let Some(browser) = &mut self.filesystem {
            self.filesystem_error = browser.refresh(path).err();
            self.rebuild_favorites_rows();
        }
    }

    /// Hide the portable header only after the host has installed its native one.
    pub fn set_native_toolbar_attached(&mut self, attached: bool) {
        self.native_toolbar_attached = attached;
    }

    pub fn dispatch_command(&mut self, command: WorkspaceCommand, context: &egui::Context) {
        match command {
            WorkspaceCommand::OpenArchive => self.request_open_archive(),
            WorkspaceCommand::OpenByPath if self.toolbar_state().can_open => {
                self.show_open_path = true
            }
            WorkspaceCommand::OpenSettings if self.toolbar_state().can_open => {
                self.show_settings = true
            }
            WorkspaceCommand::CloseArchive if !self.native_picker_pending() => self.close(),
            WorkspaceCommand::ExtractAll => self.open_extraction(ExtractionScope::All),
            WorkspaceCommand::ExtractSelected => self.open_extraction(ExtractionScope::Selected),
            WorkspaceCommand::TestArchive => {
                self.error = Some("Archive testing is not available yet.".into());
            }
            WorkspaceCommand::PreviewSelected if self.toolbar_state().can_preview => {
                self.show_inspector = true;
                self.preview_selected(context);
            }
            WorkspaceCommand::ViewOutline if !self.native_picker_pending() => {
                self.set_view(ViewMode::Outline, context);
            }
            WorkspaceCommand::ViewFlat if !self.native_picker_pending() => {
                self.set_view(ViewMode::Flat, context);
            }
            WorkspaceCommand::GoBack if self.toolbar_state().can_go_back => {
                self.navigate_history(true, context)
            }
            WorkspaceCommand::GoForward if self.toolbar_state().can_go_forward => {
                self.navigate_history(false, context)
            }
            WorkspaceCommand::ToggleFavorites => self.show_sidebar = !self.show_sidebar,
            WorkspaceCommand::ToggleInspector => self.show_inspector = !self.show_inspector,
            WorkspaceCommand::FocusSearch if !self.native_toolbar_attached => {
                self.show_search = true;
                context
                    .memory_mut(|memory| memory.request_focus(egui::Id::unique("archive_search")));
            }
            WorkspaceCommand::SearchQuery(query)
                if !self.extraction_active()
                    && !self.native_picker_pending()
                    && !self.source_changed
                    && query != self.query =>
            {
                self.query = query;
                self.filter(context);
            }
            WorkspaceCommand::SelectAll if context.text_edit_focused() => {
                // The native responder chain already handled AppKit text editors.
                // Forward Select All to an active egui path/other text field.
                context.input_mut(|input| {
                    input.events.push(egui::Event::Key {
                        key: egui::Key::A,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::COMMAND,
                    })
                });
            }
            WorkspaceCommand::SelectAll if !self.native_picker_pending() => {
                self.select_all_visible()
            }
            WorkspaceCommand::CancelOperation if self.current.is_some() => self.cancel(),
            WorkspaceCommand::Cut => context.send_viewport_cmd(egui::ViewportCommand::RequestCut),
            WorkspaceCommand::Copy => context.send_viewport_cmd(egui::ViewportCommand::RequestCopy),
            WorkspaceCommand::Paste => {
                context.send_viewport_cmd(egui::ViewportCommand::RequestPaste)
            }
            _ => {}
        }
    }

    /// Queue the same native Open action used by menus, toolbar and shortcuts.
    /// The desktop host starts the dialog with its real parent window after `show`.
    pub fn request_open_archive(&mut self) {
        if self.extraction_active() || self.native_picker_pending() {
            return;
        }
        self.show_open_path = false;
        self.native_picker_request = Some(NativePickerPurpose::OpenArchive);
    }

    /// Start and poll one native file chooser on the window's UI thread.
    /// macOS requires a running native event loop to present an asynchronous sheet.
    pub fn service_native_picker<W: HasWindowHandle + HasDisplayHandle + ?Sized>(
        &mut self,
        parent: &W,
        context: &egui::Context,
    ) {
        if self.native_picker.is_none()
            && let Some(purpose) = self.native_picker_request.take()
        {
            if self.extraction_active() {
                return;
            }
            if let Err(error) = parent
                .window_handle()
                .map(|_| ())
                .and_then(|()| parent.display_handle().map(|_| ()))
            {
                self.error = Some(format!("Could not open the file chooser: {error}"));
                return;
            }
            let mut dialog =
                rfd::AsyncFileDialog::new()
                    .set_parent(parent)
                    .set_title(match purpose {
                        NativePickerPurpose::OpenArchive => "Open archive",
                        NativePickerPurpose::ExtractionParent { .. } => {
                            "Choose a folder for extraction"
                        }
                    });
            let archive_directory = self
                .document
                .as_ref()
                .map(|document| document.snapshot.path())
                .or_else(|| self.recent_archives.first().map(PathBuf::as_path))
                .and_then(Path::parent);
            let directory = match purpose {
                NativePickerPurpose::ExtractionParent {
                    keep_name: true, ..
                } => self.extraction.destination.parent().or(archive_directory),
                _ => archive_directory,
            };
            if let Some(directory) = directory {
                dialog = dialog.set_directory(directory);
            }
            // Calling pick_file here creates the macOS sheet on the main thread.
            // Windows/Linux native backends perform their blocking work internally.
            let future: Pin<Box<dyn Future<Output = Option<rfd::FileHandle>> + Send>> =
                match purpose {
                    NativePickerPurpose::OpenArchive => Box::pin(dialog.pick_file()),
                    NativePickerPurpose::ExtractionParent { .. } => Box::pin(dialog.pick_folder()),
                };
            self.native_picker = Some(PendingNativePicker {
                generation: self.generation,
                purpose,
                future: Box::pin(async move { future.await.map(PathBuf::from) }),
                waker: Waker::from(Arc::new(PickerRepaint(context.clone()))),
            });
        }
        self.poll_native_picker(context);
    }

    fn native_picker_pending(&self) -> bool {
        self.native_picker_request.is_some() || self.native_picker.is_some()
    }

    fn poll_native_picker(&mut self, context: &egui::Context) {
        let completed = self.native_picker.as_mut().and_then(|pending| {
            let mut task_context = TaskContext::from_waker(&pending.waker);
            match pending.future.as_mut().poll(&mut task_context) {
                Poll::Pending => None,
                Poll::Ready(path) => Some((pending.generation, pending.purpose, path)),
            }
        });
        if let Some((generation, purpose, path)) = completed {
            self.native_picker = None;
            // A later drop/open/close takes precedence over an older chooser.
            // Cancellation (and backend failures exposed by rfd as None) leaves
            // the existing document, selection, preview and worker untouched.
            if generation == self.generation
                && !self.extraction_active()
                && let Some(path) = path
            {
                match purpose {
                    NativePickerPurpose::OpenArchive => self.open(path, context),
                    NativePickerPurpose::ExtractionParent { scope, keep_name } => {
                        self.review_extraction_parent(scope, path, keep_name);
                    }
                }
            }
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui) {
        if !self.style_initialized {
            configure_style(ui.ctx());
            self.style_initialized = true;
        }
        self.poll();
        self.poll_filesystem_favorites();
        if let Some(path) = self.initial_path.take() {
            self.open(path, ui.ctx());
        }
        let dropped = ui.input_mut(|input| std::mem::take(&mut input.raw.dropped_files));
        if dropped.len() == 1 {
            self.open(dropped[0].path().to_path_buf(), ui.ctx());
        } else if !dropped.is_empty() {
            self.error = Some("Drop one archive at a time.".into());
        }
        if self.extraction_active() && ui.input(|input| input.viewport().close_requested()) {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.cancel();
            self.extraction.open = true;
        }
        if !self.extraction_active()
            && ui.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::O))
        {
            self.request_open_archive();
        }
        if ui.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::B)) {
            self.show_sidebar = !self.show_sidebar;
        }
        if !self.native_toolbar_attached
            && !self.extraction_active()
            && ui.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::F))
        {
            self.show_search = true;
            ui.memory_mut(|memory| memory.request_focus(egui::Id::unique("archive_search")));
        }
        if !self.extraction_active()
            && !ui.ctx().text_edit_focused()
            && !self.show_open_path
            && !self.native_picker_pending()
            && !self.extraction.open
        {
            if ui.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::A)) {
                self.select_all_visible();
            }
            for (key, mode) in [
                (egui::Key::Num1, ViewMode::Flat),
                (egui::Key::Num2, ViewMode::Outline),
            ] {
                if ui.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, key)) {
                    self.set_view(mode, ui.ctx());
                }
            }
        }
        if !self.native_picker_pending()
            && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            if self.extraction_active() {
                self.cancel();
            } else if self.extraction.open {
                self.extraction.open = false;
            } else if self.show_open_path {
                self.show_open_path = false;
            } else if self.current.is_some() {
                self.cancel();
            }
        }
        let workspace_width = ui.available_width();
        if !self.native_toolbar_attached {
            self.toolbar(ui);
        }
        self.status_bar(ui);
        if self.show_sidebar {
            self.favorites(ui, workspace_width);
        }
        if self.show_inspector {
            self.inspector(ui, workspace_width);
        }
        self.entries(ui);
        self.open_path_window(ui.ctx());
        self.extraction_window(ui.ctx());
        self.settings_window(ui.ctx());
        self.commands_window(ui.ctx());
    }

    fn poll(&mut self) {
        if let Some(progress) = self.worker.as_ref().and_then(Worker::take_progress) {
            self.apply_progress(progress);
        }
        let replies = match self.worker.as_mut().map(Worker::poll) {
            Some(Ok(replies)) => replies,
            Some(Err(error)) => {
                if self.extraction_active() {
                    self.extraction.state = ExtractionState::Interrupted(error.to_string());
                    self.extraction.open = true;
                    self.status =
                        "Extraction worker stopped; inspect the destination for retained output."
                            .into();
                }
                self.current = None;
                self.blocks_selection = false;
                self.error = Some(error.to_string());
                self.worker = None;
                if let Some(document) = &self.document {
                    self.policy = document.snapshot.policy.clone();
                }
                self.restore_filter();
                return;
            }
            None => return,
        };
        for reply in replies {
            self.apply(reply);
        }
    }

    fn apply(&mut self, reply: Reply) {
        if self.current != Some(reply.ticket) {
            return;
        }
        let extracting = self.extraction_active();
        self.current = None;
        self.blocks_selection = false;
        match reply.result {
            Ok(Completed::Document {
                document,
                rows,
                query,
                mode,
            }) => {
                self.policy = document.snapshot.policy.clone();
                if self.selected.is_some_and(|id| document.entry(id).is_none()) {
                    self.selected = None;
                }
                self.selected_entries
                    .retain(|id| document.entry(*id).is_some());
                self.selected_folders.clear();
                self.selected_folder = None;
                let path = document.snapshot.path().to_path_buf();
                self.recent_archives.retain(|recent| recent != &path);
                self.recent_archives.insert(0, path);
                self.recent_archives.truncate(12);
                self.status = view_status(
                    rows.len(),
                    document.snapshot.entries.len(),
                    !query.is_empty(),
                );
                self.document = Some(document);
                self.rows = rows;
                self.applied_query = query.clone();
                self.query = query;
                self.applied_folder = None;
                self.folder = None;
                self.view_mode = mode;
                self.applied_view_mode = mode;
                self.expanded_folders.clear();
                self.applied_expanded_folders.clear();
                self.navigation = vec![None];
                self.navigation_index = 0;
                self.error = None;
                self.source_changed = false;
            }
            Ok(Completed::Filter {
                rows,
                query,
                folder,
                mode,
                expanded,
            }) => {
                self.status = view_status(
                    rows.len(),
                    self.document
                        .as_ref()
                        .map_or(0, |document| document.snapshot.entries.len()),
                    !query.is_empty(),
                );
                self.rows = rows;
                self.applied_query = query.clone();
                self.query = query;
                self.applied_folder = folder.clone();
                self.folder = folder;
                self.view_mode = mode;
                self.applied_view_mode = mode;
                self.applied_expanded_folders = expanded.clone();
                self.expanded_folders = expanded;
            }
            Ok(Completed::Preview(preview)) => {
                if self.selected == Some(preview.preview.entry_id) {
                    self.preview_hex = preview.text_rows.is_none();
                    self.preview_show_image = preview.image.is_some();
                    self.preview_texture = None;
                    self.status = format!(
                        "Previewed {} bytes · CRC32 verified",
                        preview.preview.bytes.len()
                    );
                    self.preview = Some(preview);
                }
            }
            Ok(Completed::Extraction(report)) => {
                if matches!(report.error, Some(ArchiveError::SourceChanged)) {
                    self.source_changed = true;
                }
                self.status = match &report.status {
                    ExtractionStatus::Completed => format!(
                        "Extracted {} · {} · Source archive unchanged",
                        counted(report.files_written, "file", "files"), format_bytes(report.bytes_written)
                    ),
                    ExtractionStatus::Cancelled => "Extraction cancelled; the new directory retains completed and partial output.".into(),
                    ExtractionStatus::Failed => "Extraction failed; inspect the retained output in the new directory.".into(),
                };
                self.extraction.open = true;
                self.extraction.state = ExtractionState::Finished(Ok(report));
                self.error = None;
            }
            Err(error) if extracting => {
                if matches!(error, ArchiveError::SourceChanged) {
                    self.source_changed = true;
                }
                self.status = "Extraction did not create a destination directory.".into();
                self.extraction.open = true;
                self.extraction.state = ExtractionState::Finished(Err(error));
                self.error = None;
            }
            Err(error) => {
                if matches!(error, ArchiveError::SourceChanged) {
                    self.source_changed = true;
                }
                if let Some(document) = &self.document {
                    self.policy = document.snapshot.policy.clone();
                }
                self.restore_filter();
                self.status = "Operation did not complete.".into();
                self.error = Some(error.to_string());
            }
        }
    }

    fn next_generation(&mut self) -> bool {
        if self.reject_during_extraction() {
            return false;
        }
        self.cancel_worker();
        let Some(next) = self.generation.checked_add(1) else {
            self.error = Some("Document generation limit reached; restart the application.".into());
            return false;
        };
        self.generation = next;
        self.request = 0;
        true
    }

    fn submit(
        &mut self,
        task: Task,
        blocks_selection: bool,
        status: &str,
        context: &egui::Context,
    ) {
        if self.reject_during_extraction() {
            return;
        }
        let Some(request) = self.request.checked_add(1) else {
            self.cancel_worker();
            self.error = Some("Request limit reached; close and reopen the archive.".into());
            return;
        };
        self.request = request;
        let ticket = Ticket {
            generation: self.generation,
            request,
        };
        if self.worker.is_none() {
            match Worker::new(context.clone()) {
                Ok(worker) => self.worker = Some(worker),
                Err(error) => {
                    if let Some(document) = &self.document {
                        self.policy = document.snapshot.policy.clone();
                    }
                    self.restore_filter();
                    self.error = Some(format!("Could not start archive worker: {error}"));
                    return;
                }
            }
        }
        let request = Request {
            ticket,
            cancel: CancellationToken::new(),
            task,
        };
        if let Some(worker) = &mut self.worker {
            match worker.submit(request) {
                Ok(()) => {
                    self.current = Some(ticket);
                    self.blocks_selection = blocks_selection;
                    self.status = status.into();
                    self.error = None;
                }
                Err(error) => {
                    self.current = None;
                    self.blocks_selection = false;
                    self.error = Some(error.to_string());
                    self.worker = None;
                    if let Some(document) = &self.document {
                        self.policy = document.snapshot.policy.clone();
                    }
                    self.restore_filter();
                }
            }
        }
    }

    fn open(&mut self, path: PathBuf, context: &egui::Context) {
        if !self.next_generation() {
            return;
        }
        self.native_picker_request = None;
        self.document = None;
        self.rows.clear();
        self.selected = None;
        self.selected_entries.clear();
        self.selected_folders.clear();
        self.selected_folder = None;
        self.selection_anchor = None;
        self.view_mode = ViewMode::Outline;
        self.applied_view_mode = ViewMode::Outline;
        self.expanded_folders.clear();
        self.applied_expanded_folders.clear();
        self.navigation = vec![None];
        self.navigation_index = 0;
        self.extraction = ExtractionDialog::default();
        self.clear_preview();
        self.policy = FilenamePolicy::default();
        self.query.clear();
        self.folder = None;
        self.applied_query.clear();
        self.applied_folder = None;
        self.source_changed = false;
        self.path_input = path.to_str().unwrap_or_default().to_owned();
        self.submit(
            Task::Open {
                path,
                policy: self.policy.clone(),
            },
            true,
            "Reading ZIP metadata…",
            context,
        );
    }

    fn close(&mut self) {
        if self.reject_during_extraction() {
            return;
        }
        self.next_generation();
        self.native_picker_request = None;
        self.document = None;
        self.rows.clear();
        self.selected = None;
        self.selected_entries.clear();
        self.selected_folders.clear();
        self.selected_folder = None;
        self.selection_anchor = None;
        self.extraction = ExtractionDialog::default();
        self.clear_preview();
        self.query.clear();
        self.folder = None;
        self.applied_query.clear();
        self.applied_folder = None;
        self.navigation = vec![None];
        self.navigation_index = 0;
        self.expanded_folders.clear();
        self.applied_expanded_folders.clear();
        self.policy = FilenamePolicy::default();
        self.source_changed = false;
        self.status = "Archive closed.".into();
    }

    fn cancel_worker(&mut self) {
        if self.extraction_active() {
            self.cancel();
            return;
        }
        if let Some(worker) = &mut self.worker {
            worker.cancel();
        }
        self.current = None;
        self.blocks_selection = false;
    }

    fn cancel(&mut self) {
        if let ExtractionState::Running {
            cancel_requested, ..
        } = &mut self.extraction.state
        {
            if let Some(worker) = &mut self.worker {
                worker.cancel();
            }
            *cancel_requested = true;
            self.status = "Cancellation requested; waiting for the extraction result…".into();
            return;
        }
        self.cancel_worker();
        if let Some(document) = &self.document {
            self.policy = document.snapshot.policy.clone();
        }
        self.restore_filter();
        self.status = "Operation cancelled; the archive was not changed.".into();
    }

    fn filter(&mut self, context: &egui::Context) {
        if self.reject_during_extraction() {
            return;
        }
        let Some(document) = self.document.clone() else {
            return;
        };
        // Expanding or collapsing an outline branch changes its visible rows,
        // not the selected archive entry or its already checked preview. Search,
        // navigation, view changes and reinterpretation still reset the inspector.
        let disclosure_only = self.view_mode == ViewMode::Outline
            && self.applied_view_mode == ViewMode::Outline
            && self.query == self.applied_query
            && self.folder == self.applied_folder
            && self.policy == document.snapshot.policy;
        if !disclosure_only {
            self.selected = None;
            self.selected_folder = None;
            self.clear_preview();
        }
        // A changed search must not replace a pending name reinterpretation with
        // a filter of the previous names while displaying the new encoding policy.
        let task = if self.policy != document.snapshot.policy {
            self.folder = None;
            Task::Reinterpret {
                document,
                policy: self.policy.clone(),
                query: self.query.clone(),
                mode: self.view_mode,
            }
        } else {
            Task::Filter {
                document,
                query: self.query.clone(),
                folder: self.folder.clone(),
                mode: self.view_mode,
                expanded: self.expanded_folders.clone(),
            }
        };
        self.submit(task, true, "Filtering entries…", context);
    }

    fn restore_filter(&mut self) {
        self.query = self.applied_query.clone();
        self.folder = self.applied_folder.clone();
        self.view_mode = self.applied_view_mode;
        self.expanded_folders = self.applied_expanded_folders.clone();
        if let Some(index) = self
            .navigation
            .iter()
            .rposition(|folder| *folder == self.folder)
        {
            self.navigation_index = index;
        }
    }

    fn change_policy(&mut self, policy: FilenamePolicy, context: &egui::Context) {
        let Some(document) = self.document.clone() else {
            return;
        };
        if !self.next_generation() {
            return;
        }
        self.policy = policy.clone();
        self.clear_preview();
        self.folder = None;
        self.submit(
            Task::Reinterpret {
                document,
                policy,
                query: self.query.clone(),
                mode: self.view_mode,
            },
            true,
            "Reinterpreting filenames…",
            context,
        );
    }

    fn clear_preview(&mut self) {
        self.preview = None;
        self.preview_texture = None;
        self.preview_show_image = false;
    }

    fn preview_selected(&mut self, context: &egui::Context) {
        let (Some(document), Some(entry)) = (self.document.clone(), self.selected) else {
            return;
        };
        if self.source_changed || self.blocks_selection || self.extraction_active() {
            return;
        }
        self.clear_preview();
        self.submit(
            Task::Preview { document, entry },
            false,
            "Reading and checking preview…",
            context,
        );
    }

    fn extraction_active(&self) -> bool {
        matches!(self.extraction.state, ExtractionState::Running { .. })
    }

    fn reject_during_extraction(&mut self) -> bool {
        if self.extraction_active() {
            self.error = Some("Wait for the extraction result, or request cancellation.".into());
            true
        } else {
            false
        }
    }

    fn open_extraction(&mut self, scope: ExtractionScope) {
        self.request_extraction_parent(scope, false);
    }

    fn request_extraction_parent(&mut self, scope: ExtractionScope, keep_name: bool) {
        if self.extraction_active()
            || self.blocks_selection
            || self.source_changed
            || self.native_picker_pending()
        {
            return;
        }
        if self.document.is_none() {
            return;
        }
        if self.selected_entries.is_empty()
            && self.selected_folders.is_empty()
            && let Some(entry) = self.selected
        {
            self.selected_entries.insert(entry);
        }
        if scope == ExtractionScope::Selected && self.selection_count() == 0 {
            return;
        }
        self.native_picker_request =
            Some(NativePickerPurpose::ExtractionParent { scope, keep_name });
    }

    fn review_extraction_parent(
        &mut self,
        scope: ExtractionScope,
        parent: PathBuf,
        keep_name: bool,
    ) {
        let Some(document) = &self.document else {
            return;
        };
        let default = default_extraction_destination(document.snapshot.path());
        let name = if keep_name {
            self.extraction
                .destination
                .file_name()
                .or_else(|| default.file_name())
        } else {
            default.file_name()
        }
        .unwrap_or(OsStr::new("Archive-extracted"))
        .to_os_string();
        let destination = parent.join(&name);
        self.extraction = ExtractionDialog {
            open: true,
            scope,
            name_input: name.to_string_lossy().into_owned(),
            destination,
            state: ExtractionState::Editing,
        };
    }

    fn start_extraction(&mut self, context: &egui::Context) {
        if self.extraction_active()
            || self.blocks_selection
            || self.source_changed
            || self.native_picker_pending()
        {
            return;
        }
        let Some(document) = self.document.clone() else {
            return;
        };
        if self.extraction.destination.as_os_str().is_empty()
            || !valid_new_folder_name(&self.extraction.name_input)
        {
            self.error = Some("Enter a name for the new folder.".into());
            return;
        }
        let selection = match self.extraction.scope {
            ExtractionScope::All => ExtractionSelection::All,
            ExtractionScope::Selected if self.selection_count() > 0 => {
                ExtractionSelection::Entries(self.selected_entries.iter().copied().collect())
            }
            ExtractionScope::Selected => {
                self.error = Some("Select one or more archive entries to extract.".into());
                return;
            }
        };
        self.cancel_worker();
        self.submit(
            Task::Extract {
                document,
                selection,
                folders: self.selected_folders.clone(),
                destination: self.extraction.destination.clone(),
            },
            true,
            "Checking extraction names and destination…",
            context,
        );
        if self.current.is_some() {
            self.extraction.state = ExtractionState::Running {
                progress: None,
                cancel_requested: false,
            };
        }
    }

    fn apply_progress(&mut self, reply: ProgressReply) {
        if self.current != Some(reply.ticket) {
            return;
        }
        if let ExtractionState::Running {
            progress,
            cancel_requested,
        } = &mut self.extraction.state
        {
            if !*cancel_requested {
                self.status = match reply.progress.phase {
                    ExtractionPhase::Checking => "Checking extraction names and destination…",
                    ExtractionPhase::Extracting => "Extracting entries…",
                    ExtractionPhase::Finalizing => "Verifying extraction output…",
                }
                .into();
            }
            *progress = Some(reply.progress);
        }
    }

    fn selection_count(&self) -> usize {
        self.selected_entries.len() + self.selected_folders.len()
    }

    fn row_selected(&self, key: &RowKey) -> bool {
        match key {
            RowKey::Entry(id) => self.selected_entries.contains(id),
            RowKey::Folder(path) => self.selected_folders.contains(path),
        }
    }

    fn mark_row(&mut self, key: &RowKey, marked: bool) {
        match key {
            RowKey::Entry(id) => {
                if marked {
                    self.selected_entries.insert(*id);
                } else {
                    self.selected_entries.remove(id);
                }
            }
            RowKey::Folder(path) => {
                if marked {
                    self.selected_folders.insert(path.clone());
                } else {
                    self.selected_folders.remove(path);
                }
            }
        }
    }

    fn select_row(&mut self, index: usize, modifiers: egui::Modifiers) {
        if self.blocks_selection || self.extraction_active() || self.native_picker_pending() {
            return;
        }
        let Some(document) = self.document.clone() else {
            return;
        };
        let Some(row) = self.rows.get(index).copied() else {
            return;
        };
        let key = row_key(&document, row);
        let additive = modifiers.command || modifiers.ctrl;
        let anchor = self.selection_anchor.as_ref().and_then(|anchor| {
            self.rows
                .iter()
                .position(|row| row_key(&document, *row) == *anchor)
        });
        if !additive {
            self.selected_entries.clear();
            self.selected_folders.clear();
        }
        if modifiers.shift {
            let anchor = anchor.unwrap_or(index);
            for position in anchor.min(index)..=anchor.max(index) {
                let key = row_key(&document, self.rows[position]);
                self.mark_row(&key, true);
            }
        } else {
            let marked = !additive || !self.row_selected(&key);
            self.mark_row(&key, marked);
            self.selection_anchor = Some(key.clone());
        }
        self.cancel_worker();
        self.clear_preview();
        self.selected_folder = None;
        self.selected = None;
        if self.row_selected(&key) {
            match key {
                RowKey::Entry(id) => self.selected = Some(id),
                RowKey::Folder(path) => {
                    self.selected_folder = Some(path);
                    self.selected = row.entry.map(|index| document.snapshot.entries[index].id);
                }
            }
        }
    }

    fn select_all_visible(&mut self) {
        if self.blocks_selection || self.extraction_active() || self.native_picker_pending() {
            return;
        }
        let Some(document) = self.document.clone() else {
            return;
        };
        self.selected_entries.clear();
        self.selected_folders.clear();
        for index in 0..self.rows.len() {
            self.mark_row(&row_key(&document, self.rows[index]), true);
        }
    }

    fn set_view(&mut self, mode: ViewMode, context: &egui::Context) {
        if self.view_mode != mode && !self.extraction_active() {
            self.view_mode = mode;
            self.filter(context);
        }
    }

    fn toggle_folder(&mut self, path: String, context: &egui::Context) {
        if self.extraction_active() {
            return;
        }
        if !self.expanded_folders.remove(&path) {
            self.expanded_folders.insert(path);
        }
        self.filter(context);
    }

    fn navigate(&mut self, folder: Option<String>, context: &egui::Context) {
        if self.folder == folder || self.extraction_active() {
            return;
        }
        self.navigation.truncate(self.navigation_index + 1);
        self.navigation.push(folder.clone());
        // Session navigation remains bounded, independent of archive size.
        if self.navigation.len() > 128 {
            self.navigation.remove(0);
        }
        self.navigation_index = self.navigation.len() - 1;
        self.folder = folder;
        self.filter(context);
    }

    fn navigate_history(&mut self, back: bool, context: &egui::Context) {
        if self.extraction_active() {
            return;
        }
        let next = if back {
            self.navigation_index.checked_sub(1)
        } else {
            self.navigation_index
                .checked_add(1)
                .filter(|index| *index < self.navigation.len())
        };
        if let Some(index) = next {
            self.navigation_index = index;
            self.folder = self.navigation[index].clone();
            self.filter(context);
        }
    }

    fn empty_workspace(&mut self, ui: &mut egui::Ui) {
        ui.add_space(((ui.available_height() - 235.0) / 2.0).max(24.0));
        ui.vertical_centered(|ui| {
            if self.current.is_some() {
                ui.spinner();
                ui.add_space(12.0);
                ui.label(egui::RichText::new("Opening archive…").font(typography::semibold(20.0)));
                return;
            }
            let size = ui.available_width().min(132.0);
            let (target, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
            let highlight = ui.input(|input| !input.raw.hovered_files.is_empty());
            let color = if highlight {
                selection_color()
            } else {
                quiet_color(ui)
            };
            let mut outline = Vec::with_capacity(25);
            let radius = 10.0;
            for (center, first_angle) in [
                (
                    target.left_top() + egui::vec2(radius, radius),
                    std::f32::consts::PI,
                ),
                (
                    target.right_top() + egui::vec2(-radius, radius),
                    -std::f32::consts::FRAC_PI_2,
                ),
                (target.right_bottom() - egui::vec2(radius, radius), 0.0),
                (
                    target.left_bottom() + egui::vec2(radius, -radius),
                    std::f32::consts::FRAC_PI_2,
                ),
            ] {
                for step in 0..=5 {
                    let angle = first_angle + step as f32 * std::f32::consts::FRAC_PI_2 / 5.0;
                    outline.push(center + egui::vec2(angle.cos(), angle.sin()) * radius);
                }
            }
            outline.push(outline[0]);
            ui.painter().extend(egui::Shape::dashed_line(
                &outline,
                egui::Stroke::new(1.2, color),
                7.0,
                4.0,
            ));
            ui.add_space(12.0);
            ui.label(egui::RichText::new("Drop an archive here to open").color(quiet_color(ui)));
            ui.add_space(16.0);
            if ui
                .add_enabled(
                    !self.native_picker_pending(),
                    egui::Button::new("Open archive…").min_size(egui::vec2(150.0, 32.0)),
                )
                .clicked()
            {
                self.request_open_archive();
            }
        });
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let idle = !self.extraction_active() && !self.native_picker_pending();
        let ready =
            self.document.is_some() && !self.blocks_selection && !self.source_changed && idle;
        let title = self
            .document
            .as_ref()
            .map(|document| {
                document
                    .snapshot
                    .path()
                    .file_name()
                    .unwrap_or(OsStr::new("Archive"))
                    .to_string_lossy()
                    .into_owned()
            })
            .unwrap_or_else(|| "Untitled".into());
        egui::Panel::top("toolbar")
            .frame(
                egui::Frame::NONE
                    .fill(chrome_color(ui))
                    .inner_margin(egui::Margin::symmetric(12, 10)),
            )
            .show(ui, |ui| {
                let width = ui.available_width();
                let wide = width >= 1100.0;
                let full = width >= 1560.0;
                let native_inset = if cfg!(target_os = "macos") { 80.0 } else { 0.0 };
                let sidebar_span = if self.show_sidebar {
                    if self.sidebar_width > 0.0 {
                        self.sidebar_width - 12.0
                    } else {
                        width * 0.18
                    }
                } else {
                    native_inset + 42.0
                };
                let searching = self.show_search || !self.query.is_empty();
                let controls_width = if full {
                    760.0
                } else if wide {
                    610.0
                } else {
                    450.0
                } + if searching { 170.0 } else { 0.0 };
                let title_width = (width - sidebar_span - controls_width).clamp(110.0, 340.0);
                ui.horizontal(|ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(sidebar_span.max(native_inset + 36.0), 40.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.set_min_width(sidebar_span.max(native_inset + 36.0));
                            ui.add_space(native_inset);
                            if toolbar_icon(
                                ui,
                                LineIcon::Sidebar,
                                "Show Favorites (Ctrl/Cmd+B)",
                                true,
                                self.show_sidebar,
                            )
                            .clicked()
                            {
                                self.show_sidebar = !self.show_sidebar;
                            }
                        },
                    );
                    // Native window dragging starts only in this caption region,
                    // never over a toolbar button or a simulated window control.
                    let (caption, drag) =
                        ui.allocate_exact_size(egui::vec2(title_width, 40.0), egui::Sense::drag());
                    paint_archive_icon(
                        ui.painter(),
                        egui::Rect::from_min_size(
                            caption.left_center() + egui::vec2(0.0, -10.0),
                            egui::vec2(19.0, 20.0),
                        ),
                        false,
                    );
                    ui.painter()
                        .with_clip_rect(caption.intersect(ui.clip_rect()))
                        .text(
                            caption.left_center() + egui::vec2(27.0, 0.0),
                            egui::Align2::LEFT_CENTER,
                            bounded_display(&title),
                            typography::semibold(15.0),
                            ui.visuals().strong_text_color(),
                        );
                    if drag.drag_started() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
                    }
                    drag.on_hover_text(bounded_display(&title));
                    toolbar_group(ui, |ui| {
                        toolbar_menu(ui, LineIcon::Extract, "Extract", ready, |ui| {
                            self.extract_menu(ui)
                        });
                        toolbar_icon(ui, LineIcon::Save, "Save — planned", false, false);
                        toolbar_icon(ui, LineIcon::Check, "Test — planned", false, false);
                        if wide {
                            toolbar_icon(ui, LineIcon::Move, "Move — planned", false, false);
                        }
                    });
                    toolbar_group(ui, |ui| {
                        if wide {
                            toolbar_icon(ui, LineIcon::Add, "Add — planned", false, false);
                        }
                        if toolbar_icon(
                            ui,
                            LineIcon::Preview,
                            "View selected item",
                            self.toolbar_state().can_preview,
                            false,
                        )
                        .clicked()
                        {
                            self.show_inspector = true;
                            self.preview_selected(ui.ctx());
                        }
                        if wide {
                            toolbar_icon(ui, LineIcon::Delete, "Delete — planned", false, false);
                            toolbar_icon(
                                ui,
                                LineIcon::NewFolder,
                                "New Folder — planned",
                                false,
                                false,
                            );
                        }
                    });
                    if full {
                        toolbar_group(ui, |ui| {
                            toolbar_icon(ui, LineIcon::Collect, "Collect — planned", false, false);
                            toolbar_icon(
                                ui,
                                LineIcon::DirectMode,
                                "Direct Mode — planned",
                                false,
                                false,
                            );
                        });
                        toolbar_group(ui, |ui| {
                            toolbar_icon(ui, LineIcon::Share, "Share — planned", false, false);
                        });
                    }
                    toolbar_group(ui, |ui| {
                        toolbar_menu(
                            ui,
                            LineIcon::More,
                            "Archive actions and settings",
                            true,
                            |ui| self.archive_menu(ui),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if toolbar_icon(
                            ui,
                            LineIcon::SidebarRight,
                            "Show Info and Preview",
                            true,
                            self.show_inspector,
                        )
                        .clicked()
                        {
                            self.show_inspector = !self.show_inspector;
                        }
                        if searching {
                            ui.add_enabled_ui(
                                self.document.is_some() && idle && !self.source_changed,
                                |ui| {
                                    if ui
                                        .add(
                                            egui::TextEdit::singleline(&mut self.query)
                                                .id(egui::Id::unique("archive_search"))
                                                .font(typography::regular(14.0))
                                                .hint_text("Search archive")
                                                .desired_width(170.0),
                                        )
                                        .changed()
                                    {
                                        self.filter(ui.ctx());
                                    }
                                },
                            );
                        }
                        toolbar_group(ui, |ui| {
                            if toolbar_icon(
                                ui,
                                LineIcon::Search,
                                "Search archive (Ctrl/Cmd+F)",
                                idle,
                                searching,
                            )
                            .clicked()
                            {
                                self.show_search = !self.show_search;
                                if self.show_search {
                                    ui.memory_mut(|memory| {
                                        memory.request_focus(egui::Id::unique("archive_search"))
                                    });
                                }
                            }
                        });
                    });
                });
            });
    }

    fn view_menu(&mut self, ui: &mut egui::Ui) {
        for (mode, label) in [
            (ViewMode::Outline, "Outline   Ctrl/Cmd+2"),
            (ViewMode::Flat, "Flat   Ctrl/Cmd+1"),
        ] {
            if ui.selectable_label(self.view_mode == mode, label).clicked() {
                self.set_view(mode, ui.ctx());
                ui.close();
            }
        }
        ui.separator();
        ui.checkbox(&mut self.show_sidebar, "Favorites");
        ui.checkbox(&mut self.show_inspector, "Info and Preview");
        ui.separator();
        disabled_action(ui, "Columns");
        disabled_action(ui, "Icons");
    }

    fn extract_menu(&mut self, ui: &mut egui::Ui) {
        if ui
            .add_enabled(
                self.selection_count() > 0,
                egui::Button::new("Extract Selected…"),
            )
            .clicked()
        {
            self.open_extraction(ExtractionScope::Selected);
            ui.close();
        }
        if ui.button("Extract All…").clicked() {
            self.open_extraction(ExtractionScope::All);
            ui.close();
        }
    }

    fn archive_menu(&mut self, ui: &mut egui::Ui) {
        let idle = !self.extraction_active() && !self.native_picker_pending();
        if ui
            .add_enabled(idle, egui::Button::new("Open archive…"))
            .clicked()
        {
            self.request_open_archive();
            ui.close();
        }
        ui.add_enabled_ui(idle && !self.recent_archives.is_empty(), |ui| {
            ui.menu_button("Open Recent", |ui| {
                let mut open = None;
                for path in &self.recent_archives {
                    let name = path
                        .file_name()
                        .unwrap_or(path.as_os_str())
                        .to_string_lossy();
                    if ui
                        .button(bounded_display(&name))
                        .on_hover_text(escape_display(&path.to_string_lossy()))
                        .clicked()
                    {
                        open = Some(path.clone());
                        ui.close();
                    }
                }
                if let Some(path) = open {
                    self.open(path, ui.ctx());
                }
            });
        });
        if ui
            .add_enabled(
                self.document.is_some() && idle,
                egui::Button::new("Reopen source"),
            )
            .clicked()
        {
            if let Some(document) = &self.document {
                self.open(document.snapshot.path().to_path_buf(), ui.ctx());
            }
            ui.close();
        }
        if ui
            .add_enabled(
                self.document.is_some() && idle,
                egui::Button::new("Close archive"),
            )
            .clicked()
        {
            self.close();
            ui.close();
        }
        ui.separator();
        ui.menu_button("View", |ui| self.view_menu(ui));
        ui.separator();
        for label in [
            "Save",
            "Test",
            "Move",
            "Add files…",
            "Delete",
            "New Folder",
            "Collect",
            "Direct Mode",
            "Share",
        ] {
            disabled_action(ui, label);
        }
        ui.separator();
        if ui
            .add_enabled(idle, egui::Button::new("Settings…"))
            .clicked()
        {
            self.dispatch_command(WorkspaceCommand::OpenSettings, ui.ctx());
            ui.close();
        }
        ui.menu_button("Advanced", |ui| {
            if ui
                .add_enabled(idle, egui::Button::new("Open by path…"))
                .clicked()
            {
                self.show_open_path = true;
                ui.close();
            }
        });
        if ui.button("Command availability…").clicked() {
            self.show_commands = true;
            ui.close();
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("status")
            .frame(
                egui::Frame::NONE
                    .fill(chrome_color(ui))
                    .inner_margin(egui::Margin::symmetric(10, 5)),
            )
            .show(ui, |ui| {
                if let Some(error) = &self.error {
                    ui.colored_label(ui.visuals().error_fg_color, escape_display(error));
                }
                if self.source_changed {
                    ui.label("The source archive changed. Reopen it from the Archive menu.");
                }
                ui.horizontal(|ui| {
                    if self.current.is_some() {
                        ui.spinner();
                        if ui.small_button("Stop").clicked() {
                            self.cancel();
                        }
                    }
                    let selection = self.selection_count();
                    if selection > 0 {
                        ui.small(format!("{selection} selected"));
                        ui.separator();
                    }
                    ui.add(egui::Label::new(egui::RichText::new(&self.status).small()).truncate())
                        .on_hover_text(&self.status);
                    if matches!(
                        self.extraction.state,
                        ExtractionState::Finished(_) | ExtractionState::Interrupted(_)
                    ) && ui.small_button("Details…").clicked()
                    {
                        self.extraction.open = true;
                    }
                    if self.extraction_active() && ui.small_button("Progress…").clicked() {
                        self.extraction.open = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.weak(format!("ArcForge {VERSION}"));
                    });
                });
            });
    }

    fn favorites(&mut self, ui: &mut egui::Ui, workspace_width: f32) {
        let current = self
            .document
            .as_ref()
            .map(|document| document.snapshot.path().to_path_buf());
        let can_open = !self.extraction_active() && !self.native_picker_pending();
        let panel = egui::Panel::left("favorites")
            .default_size(workspace_width * 0.18)
            .min_size(160.0)
            .max_size(workspace_width * 0.30)
            .resizable(true)
            .frame(egui::Frame::NONE.fill(sidebar_color(ui)).inner_margin(9))
            .show(ui, |ui| {
                egui::Panel::bottom("favorites_filter")
                    .frame(
                        egui::Frame::NONE
                            .fill(sidebar_color(ui))
                            .inner_margin(egui::Margin::symmetric(0, 6)),
                    )
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let available = (ui.available_width() - 38.0).max(70.0);
                            if ui
                                .add(
                                    egui::TextEdit::singleline(&mut self.favorites_query)
                                        .hint_text("Search Favorites")
                                        .font(typography::regular(13.0))
                                        .desired_width(available),
                                )
                                .changed()
                            {
                                self.rebuild_favorites_rows();
                            }
                            if icon_button(
                                ui,
                                LineIcon::Refresh,
                                "Refresh folder",
                                self.selected_place.is_some(),
                                false,
                            )
                            .clicked()
                                && let Some(path) = self.selected_place.clone()
                            {
                                self.refresh_place(path);
                            }
                        });
                    });
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new("Favorites")
                        .font(typography::semibold(13.0))
                        .color(quiet_color(ui)),
                );
                ui.add_space(8.0);
                if let Some(error) = self.filesystem_error.as_deref().or_else(|| {
                    self.filesystem
                        .as_ref()
                        .and_then(FilesystemBrowser::roots_error)
                }) {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(bounded_display(error))
                                .color(ui.visuals().error_fg_color),
                        )
                        .wrap(),
                    );
                }
                if self
                    .filesystem
                    .as_ref()
                    .is_some_and(FilesystemBrowser::roots_pending)
                {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.weak("Loading Favorites…");
                    });
                }
                let mut toggle = None;
                let mut open = None;
                let mut refresh = None;
                ui.spacing_mut().item_spacing.y = 0.0;
                egui::ScrollArea::both()
                    .id_salt("filesystem_favorites")
                    .auto_shrink([false, false])
                    .show_rows(ui, 29.0, self.favorites_rows.len(), |ui, range| {
                        ui.set_min_width(self.favorites_content_width.max(ui.available_width()));
                        for index in range {
                            let row = &self.favorites_rows[index];
                            let (rect, response) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), 29.0),
                                egui::Sense::click(),
                            );
                            let selected = match row.kind {
                                FavoritesRowKind::Archive => current.as_ref() == Some(&row.path),
                                FavoritesRowKind::Directory { .. } => {
                                    self.selected_place.as_ref() == Some(&row.path)
                                }
                                FavoritesRowKind::Notice => false,
                            };
                            if selected {
                                ui.painter()
                                    .rect_filled(rect, 6, sidebar_selection_color(ui));
                            }
                            let left = rect.left() + row.depth as f32 * 15.0;
                            let mut label_right = rect.right() - 3.0;
                            let enabled = !self.native_picker_pending()
                                && (!matches!(row.kind, FavoritesRowKind::Archive) || can_open);
                            let color = if matches!(row.kind, FavoritesRowKind::Notice) || !enabled
                            {
                                quiet_color(ui)
                            } else {
                                ui.visuals().text_color()
                            };
                            match row.kind {
                                FavoritesRowKind::Directory { expanded, count } => {
                                    paint_disclosure(
                                        ui.painter(),
                                        egui::Rect::from_min_size(
                                            egui::pos2(left, rect.center().y - 9.0),
                                            egui::vec2(18.0, 18.0),
                                        ),
                                        expanded,
                                        color,
                                    );
                                    paint_folder_icon(
                                        ui.painter(),
                                        egui::Rect::from_min_size(
                                            egui::pos2(left + 18.0, rect.center().y - 11.0),
                                            egui::vec2(22.0, 22.0),
                                        ),
                                    );
                                    if let Some(count) = count {
                                        let badge = egui::Rect::from_min_size(
                                            egui::pos2(rect.right() - 36.0, rect.center().y - 9.0),
                                            egui::vec2(33.0, 18.0),
                                        );
                                        ui.painter().rect_filled(
                                            badge,
                                            9,
                                            egui::Color32::from_rgb(166, 184, 210),
                                        );
                                        ui.painter().text(
                                            badge.center(),
                                            egui::Align2::CENTER_CENTER,
                                            count.to_string(),
                                            typography::semibold(12.0),
                                            egui::Color32::WHITE,
                                        );
                                        label_right = badge.left() - 3.0;
                                    }
                                }
                                FavoritesRowKind::Archive => paint_archive_icon(
                                    ui.painter(),
                                    egui::Rect::from_min_size(
                                        egui::pos2(left + 18.0, rect.center().y - 10.0),
                                        egui::vec2(20.0, 20.0),
                                    ),
                                    false,
                                ),
                                FavoritesRowKind::Notice => {}
                            }
                            let label_left = left
                                + if matches!(row.kind, FavoritesRowKind::Notice) {
                                    18.0
                                } else {
                                    44.0
                                };
                            let text_rect = egui::Rect::from_min_max(
                                egui::pos2(label_left, rect.top()),
                                egui::pos2(label_right.max(label_left), rect.bottom()),
                            );
                            paint_cell(ui, text_rect, &bounded_display(&row.label), color, false);
                            response.widget_info(|| {
                                egui::WidgetInfo::selected(
                                    egui::Role::ListBoxOption,
                                    enabled,
                                    selected,
                                    row.label.as_str(),
                                )
                            });
                            let tip = match row.kind {
                                FavoritesRowKind::Directory {
                                    count: Some(count), ..
                                } => {
                                    format!(
                                        "{}\n{count} ZIP archives in this folder",
                                        escape_display(&row.path.to_string_lossy())
                                    )
                                }
                                FavoritesRowKind::Notice => format!(
                                    "{}\nClick to refresh this folder.",
                                    escape_display(&row.label)
                                ),
                                _ => escape_display(&row.path.to_string_lossy()),
                            };
                            if response.on_hover_text(tip).clicked() && enabled {
                                match row.kind {
                                    FavoritesRowKind::Directory { .. } => {
                                        toggle = Some(row.path.clone())
                                    }
                                    FavoritesRowKind::Archive
                                        if current.as_ref() != Some(&row.path) =>
                                    {
                                        open = Some(row.path.clone())
                                    }
                                    FavoritesRowKind::Notice => refresh = Some(row.path.clone()),
                                    _ => {}
                                }
                            }
                        }
                    });
                if let Some(path) = toggle {
                    self.toggle_place(path);
                }
                if let Some(path) = refresh {
                    self.refresh_place(path);
                }
                if let Some(path) = open {
                    self.selected_place = path.parent().map(Path::to_path_buf);
                    self.open(path, ui.ctx());
                }
            });
        self.sidebar_width = panel.response.rect.width();
    }

    fn navigation_bar(&mut self, ui: &mut egui::Ui, document: &PreparedDocument) {
        ui.horizontal(|ui| {
            if icon_button(
                ui,
                LineIcon::Back,
                "Back",
                self.navigation_index > 0 && !self.extraction_active(),
                false,
            )
            .clicked()
            {
                self.navigate_history(true, ui.ctx());
            }
            if icon_button(
                ui,
                LineIcon::Forward,
                "Forward",
                self.navigation_index + 1 < self.navigation.len() && !self.extraction_active(),
                false,
            )
            .clicked()
            {
                self.navigate_history(false, ui.ctx());
            }
            ui.separator();
            let name = document
                .snapshot
                .path()
                .file_name()
                .unwrap_or(OsStr::new("Archive"))
                .to_string_lossy();
            if ui
                .add_enabled(
                    !self.extraction_active(),
                    egui::Button::new(bounded_display(&name)).frame(false),
                )
                .clicked()
            {
                self.navigate(None, ui.ctx());
            }
            if let Some(folder) = self.folder.clone() {
                ui.weak("/");
                ui.add(egui::Label::new(bounded_display(folder.trim_end_matches('/'))).truncate())
                    .on_hover_text(escape_display(&folder));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.weak(match self.view_mode {
                    ViewMode::Outline => "Outline",
                    ViewMode::Flat => "Flat",
                });
            });
        });
    }

    fn entries(&mut self, ui: &mut egui::Ui) {
        let document = self.document.clone();
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(content_color(ui)).inner_margin(6)).show(ui, |ui| {
            let Some(document) = document else {
                self.empty_workspace(ui);
                return;
            };
            self.navigation_bar(ui, &document);
            ui.separator();
            if document.folders_limited {
                ui.small("Outline folder cache reached its limit. Flat view and search retain every entry.");
            }
            let mut clicked = None;
            let mut disclosed = None;
            let mut entered = None;
            egui::ScrollArea::horizontal().id_salt("contents_horizontal").auto_shrink([false, false]).show(ui, |ui| {
                let width = ui.available_width().max(600.0);
                let columns = table_columns(width);
                ui.set_min_width(width);
                ui.spacing_mut().item_spacing.y = 0.0;
                let (header, _) = ui.allocate_exact_size(egui::vec2(width, 27.0), egui::Sense::hover());
                ui.painter().rect_filled(header, 0, chrome_color(ui));
                let mut x = header.left();
                for (label, cell_width) in columns {
                    let rect = egui::Rect::from_min_size(egui::pos2(x, header.top()), egui::vec2(cell_width, header.height()));
                    paint_header_cell(ui, rect, label);
                    x += cell_width;
                    ui.painter().line_segment([egui::pos2(x, header.top() + 5.0), egui::pos2(x, header.bottom() - 5.0)], egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color));
                }
                if self.rows.is_empty() {
                    ui.add_space(12.0);
                    ui.weak("No matching entries");
                }
                ui.add_enabled_ui(!self.blocks_selection && !self.extraction_active(), |ui| {
                    let background = ui.painter().add(egui::Shape::Noop);
                    let mut content_origin_y = ui.cursor().top();
                    let scrolling = egui::ScrollArea::vertical().id_salt("contents_rows").auto_shrink([false, false])
                        .show_rows(ui, 25.0, self.rows.len(), |ui, range| {
                            content_origin_y = ui.max_rect().top() - range.start as f32 * 25.0;
                            for index in range {
                                let row = self.rows[index];
                                let key = row_key(&document, row);
                                let selected = self.row_selected(&key);
                                let name = row_display_name(&document, row, self.view_mode);
                                let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 25.0), egui::Sense::click());
                                response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Row, ui.is_enabled(), selected, &name));
                                let fill = if selected { selection_color() } else if index % 2 == 1 { stripe_color(ui) } else { content_color(ui) };
                                ui.painter().rect_filled(rect, 0, fill);
                                let color = if selected { egui::Color32::WHITE } else { ui.visuals().text_color() };
                                let indent = (row.depth.min(12) as f32) * 15.0;
                                let disclosure_rect = egui::Rect::from_min_size(rect.left_top() + egui::vec2(4.0 + indent, 3.0), egui::vec2(18.0, 19.0));
                                let mut disclosure_clicked = false;
                                if let Some(folder) = row.folder {
                                    let path = &document.folders[folder];
                                    let expanded = !self.query.is_empty() || self.expanded_folders.contains(path);
                                    paint_disclosure(ui.painter(), disclosure_rect, expanded, color);
                                    let disclosure = ui.interact(disclosure_rect, ui.scope_id().with(("disclosure", folder)), egui::Sense::click());
                                    disclosure.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), if expanded { "Collapse folder" } else { "Expand folder" }));
                                    if disclosure.clicked() && self.query.is_empty() {
                                        disclosed = Some(path.clone());
                                        disclosure_clicked = true;
                                    }
                                }
                                let icon_rect = egui::Rect::from_min_size(rect.left_top() + egui::vec2(24.0 + indent, 1.5), egui::vec2(22.0, 22.0));
                                let directory = row.folder.is_some() || row.entry.is_some_and(|entry| document.snapshot.entries[entry].is_directory);
                                if directory {
                                    paint_folder_icon(ui.painter(), icon_rect);
                                } else {
                                    let image_file = row.entry.is_some_and(|entry| is_image_name(&document.snapshot.entries[entry].name.display));
                                    paint_archive_icon(ui.painter(), icon_rect, image_file);
                                }
                                let name_rect = egui::Rect::from_min_max(egui::pos2(rect.left() + 48.0 + indent, rect.top()), egui::pos2(rect.left() + columns[0].1, rect.bottom()));
                                paint_cell(ui, name_rect, &name, color, false);
                                let entry = row.entry.map(|entry| &document.snapshot.entries[entry]);
                                let totals = row.folder.map(|folder| document.folder_totals[folder]);
                                let values = [
                                    entry.and_then(|entry| entry.modified.as_deref()).unwrap_or("—").to_owned(),
                                    totals.map(|totals| format_bytes(totals.size)).or_else(|| entry.map(|entry| format_bytes(entry.size))).unwrap_or_else(|| "—".into()),
                                    entry.map(entry_kind).unwrap_or_else(|| "Folder".into()),
                                    totals.map(|totals| size_ratio(totals.size, totals.compressed_size)).or_else(|| entry.map(compression_ratio)).unwrap_or_else(|| "—".into()),
                                ];
                                let mut x = rect.left() + columns[0].1;
                                for (column, value) in values.iter().enumerate() {
                                    let cell_width = columns[column + 1].1;
                                    let cell = egui::Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(cell_width, rect.height()));
                                    paint_cell(ui, cell, value, color, column == 1 || column == 3);
                                    x += cell_width;
                                }
                                let response = response.on_hover_text(row_full_path(&document, row));
                                if response.clicked() && !disclosure_clicked {
                                    clicked = Some((index, ui.input(|input| input.modifiers)));
                                }
                                if response.double_clicked() && let Some(folder) = row.folder {
                                    entered = Some(document.folders[folder].clone());
                                }
                            }
                        });
                    ui.painter().set(background, table_stripes(ui, scrolling.inner_rect, content_origin_y));
                });
            });
            if let Some(folder) = entered {
                self.navigate(Some(folder), ui.ctx());
            } else if let Some(folder) = disclosed {
                self.toggle_folder(folder, ui.ctx());
            } else if let Some((index, modifiers)) = clicked {
                self.select_row(index, modifiers);
                if self.selected.and_then(|id| document.entry(id)).is_some_and(|entry| !entry.is_directory) {
                    self.preview_selected(ui.ctx());
                }
            }
        });
    }

    fn inspector(&mut self, ui: &mut egui::Ui, workspace_width: f32) {
        let document = self.document.clone();
        egui::Panel::right("inspector")
            .default_size(workspace_width * 0.33)
            .min_size(280.0)
            .max_size(workspace_width * 0.50)
            .resizable(true)
            .frame(egui::Frame::NONE.fill(content_color(ui)))
            .show(ui, |ui| {
                let Some(document) = document else {
                    egui::Panel::top("empty_inspector_properties")
                        .exact_size(190.0)
                        .frame(egui::Frame::NONE.fill(content_color(ui)).inner_margin(egui::Margin::symmetric(18, 20)))
                        .show(ui, |ui| {
                            centered_heading(ui, "No files selected");
                            ui.add_space(18.0);
                            for label in ["Path", "Kind", "Size", "Modified"] {
                                inspector_property(ui, label, "");
                            }
                        });
                    return;
                };
                let height = ui.available_height();
                egui::Panel::top("inspector_properties")
                    .default_size(height * 0.43)
                    .min_size(180.0)
                    .max_size(height * 0.67)
                    .resizable(true)
                    .frame(egui::Frame::NONE.fill(content_color(ui)).inner_margin(egui::Margin::symmetric(18, 16)))
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical().id_salt("information_scroll").auto_shrink([false, false]).show(ui, |ui| {
                            self.inspector_info(ui, &document);
                        });
                    });
                egui::Panel::top("inspector_checksum")
                    .exact_size(68.0)
                    .frame(egui::Frame::NONE.fill(content_color(ui)).inner_margin(egui::Margin::symmetric(10, 5)))
                    .show(ui, |ui| {
                        self.checksum_bar(ui, &document);
                    });
                egui::Frame::NONE.fill(content_color(ui)).inner_margin(egui::Margin::symmetric(10, 9)).show(ui, |ui| {
                    ui.set_min_height(ui.available_height());
                    let entry = self.selected.and_then(|id| document.entry(id));
                    if entry.is_some_and(|entry| entry.is_directory) || self.selected_folder.is_some() {
                        ui.weak("Folder preview");
                    } else if let Some(entry) = entry {
                        if self.preview.as_ref().is_some_and(|preview| preview.preview.entry_id == entry.id) {
                            self.preview_view(ui);
                        } else {
                            ui.label(egui::RichText::new("Preview").font(typography::semibold(14.0)));
                            ui.add_space(8.0);
                            ui.weak("Select a small file to preview its text, image, or bytes.");
                            ui.small("Stored/Deflate, up to 256 KiB; complete payload and CRC32 verification.");
                            if ui.add_enabled(!self.blocks_selection && !self.source_changed && !self.extraction_active(), egui::Button::new("Load preview")).clicked() {
                                self.preview_selected(ui.ctx());
                            }
                        }
                    } else {
                        ui.weak("Select an item to preview it.");
                    }
                });
            });
    }

    fn inspector_info(&mut self, ui: &mut egui::Ui, document: &PreparedDocument) {
        if let Some(entry) = self.selected.and_then(|id| document.entry(id)) {
            let totals = self
                .selected_folder
                .as_ref()
                .and_then(|folder| document.folders.binary_search(folder).ok())
                .map(|folder| document.folder_totals[folder]);
            centered_heading(ui, &display_basename(entry));
            ui.add_space(22.0);
            let path = entry
                .name
                .display
                .trim_end_matches('/')
                .rsplit_once('/')
                .map_or("Archive root", |(path, _)| path);
            inspector_property(ui, "Path", &bounded_display(path));
            inspector_property(ui, "Kind", &entry_kind(entry));
            let size = totals.map_or(entry.size, |totals| totals.size);
            let packed = totals.map_or(entry.compressed_size, |totals| totals.compressed_size);
            inspector_property(ui, "Size", &detailed_size(size));
            inspector_property(ui, "Compressed", &detailed_size(packed));
            inspector_property(
                ui,
                "Modified",
                entry.modified.as_deref().unwrap_or("Unknown"),
            );
            inspector_property(ui, "Attributes", &unix_permissions(entry));
            inspector_property(ui, "Ratio", &size_ratio(size, packed));
            if let Some(totals) = totals {
                inspector_property(ui, "Items", &totals.entries.to_string());
            }
            ui.add_space(10.0);
            let issue = matches!(
                entry.name.state,
                NameState::Invalid | NameState::Conflict | NameState::Provisional
            );
            egui::CollapsingHeader::new("Filename and archive metadata").default_open(issue).show(ui, |ui| {
                ui.label(format!("Entry ID: {}", entry.id));
                ui.label(format!("Method: {}", method_name(entry.compression_method)));
                ui.label(format!("Encrypted: {}", if entry.encrypted { "Yes" } else { "No" }));
                ui.label(format!("Name state: {:?}", entry.name.state));
                ui.label(format!("Evidence: {:?}", entry.name.provenance));
                ui.label(format!("Encoding: {}", entry.name.encoding.map(FilenameEncoding::label).unwrap_or("Unresolved")));
                show_diagnostics(ui, &entry.name.diagnostics);
                let mut policy = self.policy.clone();
                let mut overridden = policy.entries.contains_key(&entry.id);
                let mut choice = policy.entries.get(&entry.id).copied().unwrap_or(policy.archive);
                let mut changed = false;
                ui.add_enabled_ui(!self.source_changed && !self.blocks_selection && !self.extraction_active(), |ui| {
                    changed |= ui.checkbox(&mut overridden, "Override this entry's filename interpretation").changed();
                    if overridden {
                        changed |= encoding_picker(ui, ("entry_encoding", entry.id), &mut choice);
                    }
                });
                if changed {
                    if overridden { policy.entries.insert(entry.id, choice); } else { policy.entries.remove(&entry.id); }
                    self.change_policy(policy, ui.ctx());
                }
                ui.small("Interpretation preserves original archive bytes. Archive fallback is in Settings.");
                ui.collapsing("Original filename bytes (hex)", |ui| show_hex(ui, ("raw_name", entry.id), &entry.raw_name_hex));
                ui.collapsing("Original extra fields (hex)", |ui| show_hex(ui, ("raw_extra", entry.id), &entry.raw_extra_hex));
                ui.collapsing("Original entry comment (hex)", |ui| show_hex(ui, ("raw_comment", entry.id), &entry.raw_comment_hex));
                show_diagnostics(ui, &entry.warnings);
            });
        } else if let Some(folder) = &self.selected_folder {
            centered_heading(
                ui,
                &bounded_display(
                    folder
                        .trim_end_matches('/')
                        .rsplit('/')
                        .next()
                        .unwrap_or(folder),
                ),
            );
            ui.add_space(22.0);
            let parent = folder
                .trim_end_matches('/')
                .rsplit_once('/')
                .map_or("Archive root", |(path, _)| path);
            inspector_property(ui, "Path", &bounded_display(parent));
            inspector_property(ui, "Kind", "Folder");
            if let Ok(index) = document.folders.binary_search(folder) {
                let totals = document.folder_totals[index];
                inspector_property(ui, "Size", &detailed_size(totals.size));
                inspector_property(ui, "Compressed", &detailed_size(totals.compressed_size));
                inspector_property(
                    ui,
                    "Ratio",
                    &size_ratio(totals.size, totals.compressed_size),
                );
                inspector_property(ui, "Items", &totals.entries.to_string());
            }
            ui.add_space(12.0);
            ui.small(
                "This folder is inferred from entry paths. Extraction includes its descendants.",
            );
        } else {
            centered_heading(ui, "Archive info");
            ui.add_space(22.0);
            inspector_property(ui, "Entries", &document.snapshot.entries.len().to_string());
            inspector_property(
                ui,
                "Size",
                &detailed_size(document.snapshot.uncompressed_size),
            );
            inspector_property(
                ui,
                "Compressed",
                &detailed_size(document.snapshot.compressed_size),
            );
            ui.add_space(16.0);
            ui.weak("Select an item to see its properties and preview.");
            ui.collapsing("Archive metadata", |ui| {
                show_diagnostics(ui, &document.snapshot.warnings);
                show_hex(ui, "archive_comment", &document.snapshot.comment_hex);
            });
        }
    }

    fn checksum_bar(&mut self, ui: &mut egui::Ui, document: &PreparedDocument) {
        let entry = self
            .selected
            .and_then(|id| document.entry(id))
            .filter(|entry| !entry.is_directory);
        ui.horizontal(|ui| {
            ui.add_enabled_ui(entry.is_some(), |ui| {
                ui.menu_button(egui::RichText::new("CRC32").font(typography::regular(14.0)), |ui| {
                    ui.selectable_label(true, "CRC32").on_hover_text("Checksum recorded in the ZIP directory; verified when the complete preview is loaded.");
                    disabled_action(ui, "SHA-1");
                    disabled_action(ui, "SHA-256");
                });
            });
            if icon_button(ui, LineIcon::Refresh, "Read the complete preview and verify CRC32", entry.is_some() && !self.blocks_selection && !self.source_changed && !self.extraction_active(), false).clicked() {
                self.preview_selected(ui.ctx());
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icon_button(ui, LineIcon::Copy, "Copy CRC32", entry.is_some(), false).clicked() && let Some(entry) = entry {
                    ui.ctx().copy_text(format!("{:08x}", entry.crc32));
                }
            });
        });
        if let Some(entry) = entry {
            ui.horizontal(|ui| {
                ui.monospace(format!("{:08x}", entry.crc32));
                let verified = self.preview.as_ref().is_some_and(|preview| {
                    preview.preview.entry_id == entry.id && preview.preview.crc32_verified
                });
                ui.label(
                    egui::RichText::new(if verified {
                        "Verified"
                    } else {
                        "Archive value"
                    })
                    .small()
                    .color(quiet_color(ui)),
                );
            });
        } else {
            ui.weak("Select a file to inspect its checksum.");
        }
    }

    fn preview_view(&mut self, ui: &mut egui::Ui) {
        let Some(preview) = &self.preview else {
            return;
        };
        if self.preview_texture.is_none()
            && let Some(image) = &preview.image
        {
            self.preview_texture = Some(ui.ctx().load_texture(
                format!(
                    "archive-image-{}-{}",
                    self.generation, preview.preview.entry_id
                ),
                image.clone(),
                egui::TextureOptions::LINEAR,
            ));
        }
        ui.horizontal(|ui| {
            if preview.image.is_some()
                && ui
                    .selectable_label(self.preview_show_image, "Image")
                    .clicked()
            {
                self.preview_show_image = true;
            }
            if preview.text_rows.is_some()
                && ui
                    .selectable_label(
                        !self.preview_show_image && !self.preview_hex,
                        "Text (UTF-8)",
                    )
                    .clicked()
            {
                self.preview_show_image = false;
                self.preview_hex = false;
            }
            if ui
                .selectable_label(!self.preview_show_image && self.preview_hex, "Hex")
                .clicked()
            {
                self.preview_show_image = false;
                self.preview_hex = true;
            }
        });
        if let Some(error) = &preview.image_error {
            ui.colored_label(ui.visuals().warn_fg_color, error);
        }
        if self.preview_show_image
            && let Some(texture) = &self.preview_texture
        {
            let available = egui::vec2(
                ui.available_width().max(1.0),
                ui.available_height().max(1.0),
            );
            let dimensions = texture.size_vec2();
            let scale = (available.x / dimensions.x).min(available.y / dimensions.y);
            let size = dimensions * scale;
            ui.vertical_centered(|ui| {
                ui.add_space(((available.y - size.y) * 0.5).max(0.0));
                ui.image((texture.id(), size));
            });
            return;
        }
        ui.label(
            egui::RichText::new(format!(
                "{} bytes · CRC32 verified",
                preview.preview.bytes.len()
            ))
            .small()
            .color(quiet_color(ui)),
        );
        if !self.preview_hex && preview.text_limited {
            ui.small("Text view capped; the hex view contains the full checked payload.");
        }
        let rows = if self.preview_hex {
            &preview.hex_rows
        } else {
            preview.text_rows.as_ref().unwrap_or(&preview.hex_rows)
        };
        egui::ScrollArea::both()
            .id_salt(("preview", preview.preview.entry_id, self.preview_hex))
            .max_height(ui.available_height().max(80.0))
            .show_rows(
                ui,
                ui.text_style_height(&egui::TextStyle::Monospace),
                rows.len(),
                |ui, range| {
                    for index in range {
                        ui.add(
                            egui::Label::new(egui::RichText::new(&rows[index]).monospace())
                                .wrap_mode(egui::TextWrapMode::Extend),
                        );
                    }
                },
            );
    }

    fn open_path_window(&mut self, context: &egui::Context) {
        if !self.show_open_path {
            return;
        }
        let mut visible = true;
        let mut open = false;
        egui::Window::new("Open by path")
            .open(&mut visible)
            .collapsible(false)
            .default_width(480.0)
            .auto_sized()
            .show(context, |ui| {
                ui.set_width(480.0);
                ui.label("Archive path:");
                let field = ui.add(
                    egui::TextEdit::singleline(&mut self.path_input).desired_width(f32::INFINITY),
                );
                open = ui
                    .add_enabled(!self.path_input.is_empty(), egui::Button::new("Open"))
                    .clicked()
                    || (field.lost_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter))
                        && !self.path_input.is_empty());
            });
        self.show_open_path = visible && !open;
        if open {
            self.open(PathBuf::from(&self.path_input), context);
        }
    }

    fn extraction_window(&mut self, context: &egui::Context) {
        if !self.extraction.open {
            return;
        }
        let running = self.extraction_active();
        let selected = self.selection_count();
        let total = self
            .document
            .as_ref()
            .map_or(0, |document| document.snapshot.entries.len());
        let document = self.document.clone();
        let mut visible = true;
        let mut start = false;
        let mut cancel = false;
        let mut edit = false;
        let mut dismiss = false;
        let mut change_parent = false;
        let mut window = egui::Window::new("Extract")
            .id(egui::Id::unique("extraction_sheet"))
            .anchor(egui::Align2::CENTER_TOP, [0.0, 100.0])
            .auto_sized()
            .collapsible(false);
        if !running {
            window = window.open(&mut visible);
        }
        window.show(context, |ui| {
            // Fix the width for readable wrapping, while the height follows
            // the current form/progress/result rather than Window's 420px default.
            ui.set_width(500.0);
            if self.native_picker_pending() {
                ui.disable();
            }
            match &self.extraction.state {
                ExtractionState::Editing => {
                    ui.heading(match self.extraction.scope {
                        ExtractionScope::Selected => format!("Extract {}", counted(selected as u64, "selected item", "selected items")),
                        ExtractionScope::All => "Extract archive".into(),
                    });
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.label("Scope");
                        ui.add_enabled_ui(selected > 0, |ui| {
                            ui.radio_value(&mut self.extraction.scope, ExtractionScope::Selected, format!("Selected ({selected})"));
                        });
                        ui.radio_value(&mut self.extraction.scope, ExtractionScope::All, format!("All entries ({total})"));
                    });
                    ui.small("Selected folders include their descendants. Paths inside the archive are preserved.");
                    ui.add_space(12.0);
                    let parent = self.extraction.destination.parent().unwrap_or(Path::new("")).to_path_buf();
                    ui.horizontal(|ui| {
                        ui.label("Where");
                        let width = (ui.available_width() - 95.0).max(60.0);
                        ui.add_sized(egui::vec2(width, 24.0), egui::Label::new(bounded_display(&parent.to_string_lossy())).truncate())
                            .on_hover_text(escape_display(&parent.to_string_lossy()));
                        change_parent = ui.button("Choose…").clicked();
                    });
                    ui.add_space(6.0);
                    ui.label("New folder");
                    let field = ui.add(egui::TextEdit::singleline(&mut self.extraction.name_input)
                        .desired_width(f32::INFINITY)
                        .hint_text("Folder name"));
                    let name_valid = valid_new_folder_name(&self.extraction.name_input);
                    if field.changed() && name_valid {
                        self.extraction.destination = parent.join(&self.extraction.name_input);
                    }
                    if !name_valid {
                        ui.colored_label(ui.visuals().error_fg_color, "Use one folder name without / or \\, other than . or ..");
                    } else if self.extraction.destination.file_name().is_some_and(|name| name.to_str().is_none()) {
                        ui.small("Editing the name replaces characters that cannot be displayed.");
                    }
                    ui.add_space(8.0);
                    ui.label("The destination must not exist. Its parent must already exist.");
                    ui.small("The source archive stays unchanged. If extraction fails or is cancelled, completed files and partial output remain in the new directory.");
                    let limits = ExtractionLimits::default();
                    ui.collapsing("Extraction limits", |ui| {
                        egui::Grid::new("extraction_limits").num_columns(2).show(ui, |ui| {
                            property(ui, "Entries", &limits.stream.max_entries.to_string());
                            property(ui, "Per file", &format_bytes(limits.stream.max_entry_bytes));
                            property(ui, "Total output", &format_bytes(limits.stream.max_total_bytes));
                            property(ui, "Read budget", &format_bytes(limits.stream.max_read_bytes));
                            property(ui, "Time budget", &format!("{} seconds", limits.stream.operation_timeout.as_secs()));
                            property(ui, "Path depth", &limits.max_path_depth.to_string());
                            property(ui, "Path bytes", &limits.max_path_bytes.to_string());
                            property(ui, "Output paths", &limits.max_output_paths.to_string());
                        });
                    });
                    ui.add_space(10.0);
                    let allowed = name_valid && !self.extraction.destination.as_os_str().is_empty()
                        && (self.extraction.scope == ExtractionScope::All || selected > 0)
                        && !self.blocks_selection && !self.source_changed;
                    dialog_actions(ui, |ui| {
                        start = ui.add_enabled(allowed, egui::Button::new("Extract Here")).clicked();
                        dismiss = ui.button("Cancel").clicked();
                    });
                    start |= allowed && field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                }
                ExtractionState::Running { progress, cancel_requested } => {
                    ui.heading(if *cancel_requested { "Cancelling extraction…" } else { "Extracting…" });
                    ui.add_space(8.0);
                    ui.label(bounded_display(&self.extraction.destination.to_string_lossy()));
                    if let Some(progress) = progress {
                        ui.label(match progress.phase {
                            ExtractionPhase::Checking => "Checking filenames and destination",
                            ExtractionPhase::Extracting => "Writing and checking files",
                            ExtractionPhase::Finalizing => "Finalizing output",
                        });
                        if progress.total_bytes > 0 {
                            ui.add(egui::ProgressBar::new((progress.bytes_written as f64 / progress.total_bytes as f64).clamp(0.0, 1.0) as f32).show_percentage());
                        } else {
                            ui.spinner();
                        }
                        ui.label(format!("{} of {} entries · {} of {}", progress.completed_entries, progress.total_entries, format_bytes(progress.bytes_written), format_bytes(progress.total_bytes)));
                        if let Some(entry) = progress.current_entry.and_then(|id| document.as_ref().and_then(|document| document.entry(id))) {
                            ui.add(egui::Label::new(display_name(entry)).truncate());
                        }
                    } else {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Checking filenames and destination…");
                        });
                    }
                    ui.add_space(8.0);
                    if *cancel_requested {
                        ui.small("Waiting for the worker to report what was written. Completed and partial output will be retained.");
                    } else {
                        ui.small("Cancelling retains output already written to this directory.");
                    }
                    dialog_actions(ui, |ui| {
                        cancel = ui.add_enabled(!*cancel_requested, egui::Button::new("Cancel extraction")).clicked();
                    });
                }
                ExtractionState::Finished(Ok(report)) => {
                    ui.heading(match report.status {
                        ExtractionStatus::Completed => "Extraction complete",
                        ExtractionStatus::Cancelled => "Extraction cancelled",
                        ExtractionStatus::Failed => "Extraction failed",
                    });
                    ui.add_space(8.0);
                    ui.label(escape_display(&report.destination));
                    ui.label(format!("{} · {} · {}", counted(report.files_written, "file", "files"), counted(report.directories_created, "directory", "directories"), format_bytes(report.bytes_written)));
                    ui.label(format!("{} of {} planned entries completed", report.completed_entries.len(), report.entries_planned));
                    if report.status == ExtractionStatus::Completed {
                        ui.small("The source archive is unchanged.");
                    } else if report.destination_created {
                        ui.colored_label(ui.visuals().warn_fg_color, "The new directory is retained. It may contain completed files and incomplete output.");
                    }
                    if let Some(incomplete) = &report.incomplete {
                        ui.label(format!("Incomplete file: {}", escape_display(&incomplete.path)));
                        ui.label(format!("{} bytes written to this file", incomplete.bytes_written));
                    }
                    if let Some(error) = &report.error {
                        ui.colored_label(ui.visuals().error_fg_color, escape_display(&error.to_string()));
                    }
                    ui.add_space(10.0);
                    dialog_actions(ui, |ui| {
                        dismiss = ui.button("Done").clicked();
                        if ui.button("Copy destination").clicked() {
                            context.copy_text(report.destination.clone());
                        }
                    });
                }
                ExtractionState::Finished(Err(error)) => {
                    ui.heading(if matches!(error, ArchiveError::Cancelled) { "Extraction cancelled" } else { "Could not extract" });
                    ui.add_space(8.0);
                    ui.colored_label(ui.visuals().error_fg_color, escape_display(&error.to_string()));
                    ui.label("No destination directory was created by this operation.");
                    ui.small("For a filename conflict or unresolved name, return to the archive and adjust Filename interpretation. Existing destinations require a new name.");
                    ui.add_space(10.0);
                    dialog_actions(ui, |ui| {
                        dismiss = ui.button("Return to archive").clicked();
                        edit = ui.button("Edit destination").clicked();
                    });
                }
                ExtractionState::Interrupted(error) => {
                    ui.heading("Extraction worker stopped");
                    ui.colored_label(ui.visuals().error_fg_color, escape_display(error));
                    ui.label("The final output state is unknown. Inspect this directory for retained files:");
                    ui.label(bounded_display(&self.extraction.destination.to_string_lossy()));
                    dialog_actions(ui, |ui| {
                        dismiss = ui.button("Return to archive").clicked();
                    });
                }
            }
        });
        if !running {
            self.extraction.open = visible && !dismiss;
        }
        if edit {
            self.extraction.state = ExtractionState::Editing;
        }
        if change_parent {
            self.request_extraction_parent(self.extraction.scope, true);
        }
        if start {
            self.start_extraction(context);
        }
        if cancel {
            self.cancel();
        }
    }

    fn settings_window(&mut self, context: &egui::Context) {
        if !self.show_settings {
            return;
        }
        let mut visible = true;
        egui::Window::new("Settings")
            .open(&mut visible)
            .default_width(390.0)
            .collapsible(false)
            .show(context, |ui| {
                theme_picker(ui);
                ui.separator();
                ui.strong("Archive filename fallback");
                ui.small("Automatic uses archive filename evidence. Choose a fallback for legacy names; explicit recovery can override conflicting metadata.");
                let mut policy = self.policy.clone();
                let mut changed = false;
                ui.add_enabled_ui(self.document.is_some() && !self.blocks_selection && !self.source_changed && !self.extraction_active(), |ui| {
                    changed = encoding_picker(ui, "archive_encoding", &mut policy.archive);
                });
                if changed {
                    self.change_policy(policy, context);
                }
                ui.small("Interpretation does not rewrite the source archive.");
                ui.separator();
                if ui.button("Command availability…").clicked() {
                    self.show_commands = true;
                }
            });
        self.show_settings = visible;
    }

    fn commands_window(&mut self, context: &egui::Context) {
        egui::Window::new("Command availability")
            .open(&mut self.show_commands)
            .default_width(560.0)
            .show(context, |ui| {
                ui.label("GUI and CLI share this command catalogue.");
                ui.separator();
                egui::Grid::new("command_catalog")
                    .striped(true)
                    .show(ui, |ui| {
                        for command in COMMANDS {
                            ui.monospace(command.name);
                            ui.label(match command.availability {
                                Availability::Implemented => "Available",
                                Availability::Planned => "Planned",
                            });
                            ui.label(command.description);
                            ui.end_row();
                        }
                    });
            });
    }
}

fn build_favorites_rows(
    browser: &FilesystemBrowser,
    expanded: &BTreeSet<PathBuf>,
    query: &str,
) -> Vec<FavoritesRow> {
    // At most 32 expanded, bounded 512-entry listings plus their status rows.
    const MAX_ROWS: usize = 32 * 514 + 8;
    fn append_directory(
        rows: &mut Vec<FavoritesRow>,
        browser: &FilesystemBrowser,
        expanded: &BTreeSet<PathBuf>,
        path: &Path,
        label: &str,
        depth: usize,
    ) {
        if rows.len() >= MAX_ROWS || depth > 32 {
            return;
        }
        let listing = browser.listing(path);
        let is_expanded = expanded.contains(path);
        rows.push(FavoritesRow {
            path: path.to_path_buf(),
            label: label.to_owned(),
            depth,
            kind: FavoritesRowKind::Directory {
                expanded: is_expanded,
                count: listing.and_then(|listing| listing.supported_archive_count()),
            },
        });
        if !is_expanded {
            return;
        }
        let notice = |label: String| FavoritesRow {
            path: path.to_path_buf(),
            label,
            depth: depth + 1,
            kind: FavoritesRowKind::Notice,
        };
        if browser.pending(path) {
            rows.push(notice("Loading…".into()));
            return;
        }
        let Some(listing) = listing else {
            rows.push(notice("Refresh to load this folder".into()));
            return;
        };
        let mut visible = false;
        for entry in &listing.entries {
            if rows.len() >= MAX_ROWS {
                break;
            }
            if entry.is_directory {
                visible = true;
                append_directory(
                    rows,
                    browser,
                    expanded,
                    &entry.path,
                    &entry.label,
                    depth + 1,
                );
            } else if entry.is_supported_archive {
                visible = true;
                rows.push(FavoritesRow {
                    path: entry.path.clone(),
                    label: entry.label.clone(),
                    depth: depth + 1,
                    kind: FavoritesRowKind::Archive,
                });
            }
        }
        if let Some(error) = &listing.error {
            rows.push(notice(error.clone()));
        } else if !visible && !listing.truncated {
            rows.push(notice("No ZIP archives".into()));
        }
        if listing.truncated {
            rows.push(notice("More items are not shown".into()));
        }
    }

    let mut rows = Vec::new();
    for place in browser.roots() {
        append_directory(&mut rows, browser, expanded, &place.path, &place.label, 0);
    }
    if !query.is_empty() {
        let query = query.to_lowercase();
        let mut matching_paths = BTreeSet::new();
        for row in &rows {
            if row.label.to_lowercase().contains(&query) {
                for path in row.path.ancestors().take(34) {
                    matching_paths.insert(path.to_path_buf());
                }
            }
        }
        rows.retain(|row| {
            row.depth == 0
                || matches!(row.kind, FavoritesRowKind::Notice)
                || matching_paths.contains(&row.path)
        });
    }
    rows
}

fn configure_style(context: &egui::Context) {
    typography::install(context);
    context.set_theme(egui::ThemePreference::Light);
    context.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(7.0, 5.0);
        style.spacing.button_padding = egui::vec2(8.0, 4.0);
        style
            .text_styles
            .insert(egui::TextStyle::Body, typography::regular(14.0));
        style
            .text_styles
            .insert(egui::TextStyle::Button, typography::regular(14.0));
        style
            .text_styles
            .insert(egui::TextStyle::Small, typography::regular(12.0));
        style
            .text_styles
            .insert(egui::TextStyle::Heading, typography::semibold(18.0));
        style.visuals.selection.bg_fill = selection_color();
        style.visuals.selection.stroke = egui::Stroke::new(1.0, egui::Color32::WHITE);
        style.visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(5);
        style.visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(5);
        style.visuals.widgets.active.corner_radius = egui::CornerRadius::same(5);
        style.visuals.window_corner_radius = egui::CornerRadius::same(9);
        if !style.visuals.dark_mode {
            style.visuals.widgets.noninteractive.fg_stroke.color =
                egui::Color32::from_rgb(31, 34, 38);
            style.visuals.widgets.inactive.fg_stroke.color = egui::Color32::from_rgb(40, 43, 47);
            style.visuals.weak_text_alpha = 0.74;
            style.visuals.panel_fill = egui::Color32::from_rgb(249, 249, 250);
            style.visuals.window_fill = egui::Color32::from_rgb(250, 250, 251);
            style.visuals.widgets.inactive.weak_bg_fill = egui::Color32::from_rgb(241, 242, 244);
            style.visuals.widgets.inactive.bg_stroke =
                egui::Stroke::new(1.0, egui::Color32::from_rgb(210, 213, 217));
        }
    });
}

fn chrome_color(ui: &egui::Ui) -> egui::Color32 {
    if ui.visuals().dark_mode {
        egui::Color32::from_rgb(42, 43, 46)
    } else {
        egui::Color32::from_rgb(250, 250, 251)
    }
}

fn sidebar_color(ui: &egui::Ui) -> egui::Color32 {
    if ui.visuals().dark_mode {
        egui::Color32::from_rgb(36, 38, 41)
    } else {
        egui::Color32::from_rgb(247, 247, 248)
    }
}

fn content_color(ui: &egui::Ui) -> egui::Color32 {
    if ui.visuals().dark_mode {
        egui::Color32::from_rgb(29, 30, 33)
    } else {
        egui::Color32::from_rgb(255, 255, 255)
    }
}

fn stripe_color(ui: &egui::Ui) -> egui::Color32 {
    if ui.visuals().dark_mode {
        egui::Color32::from_rgb(34, 35, 38)
    } else {
        egui::Color32::from_rgb(244, 244, 245)
    }
}

fn selection_color() -> egui::Color32 {
    egui::Color32::from_rgb(24, 112, 232)
}

fn quiet_color(ui: &egui::Ui) -> egui::Color32 {
    if ui.visuals().dark_mode {
        egui::Color32::from_gray(170)
    } else {
        egui::Color32::from_rgb(127, 128, 133)
    }
}

fn sidebar_selection_color(ui: &egui::Ui) -> egui::Color32 {
    if ui.visuals().dark_mode {
        egui::Color32::from_rgb(66, 72, 83)
    } else {
        egui::Color32::from_rgb(222, 225, 230)
    }
}

#[derive(Clone, Copy)]
enum LineIcon {
    Sidebar,
    SidebarRight,
    Move,
    Save,
    Delete,
    Check,
    Extract,
    Preview,
    Add,
    NewFolder,
    Collect,
    DirectMode,
    Share,
    More,
    Search,
    Copy,
    Refresh,
    Back,
    Forward,
    Folder,
    File,
}

fn toolbar_group(ui: &mut egui::Ui, contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::NONE
        .fill(if ui.visuals().dark_mode {
            egui::Color32::from_rgb(51, 52, 55)
        } else {
            egui::Color32::from_rgb(249, 249, 250)
        })
        .stroke(egui::Stroke::new(
            1.0,
            if ui.visuals().dark_mode {
                egui::Color32::from_gray(72)
            } else {
                egui::Color32::WHITE
            },
        ))
        .shadow(egui::epaint::Shadow {
            offset: [0, 3],
            blur: 12,
            spread: 0,
            color: egui::Color32::from_black_alpha(10),
        })
        .corner_radius(24)
        .inner_margin(egui::Margin::symmetric(7, 3))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            ui.horizontal(contents);
        });
}

fn toolbar_icon(
    ui: &mut egui::Ui,
    icon: LineIcon,
    label: &str,
    enabled: bool,
    selected: bool,
) -> egui::Response {
    let response = ui.add_enabled(
        enabled,
        egui::Button::new("")
            .min_size(egui::vec2(35.0, 34.0))
            .frame(false),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::Button, enabled && ui.is_enabled(), label)
    });
    if selected {
        ui.painter().rect_filled(
            response.rect.shrink(2.0),
            9,
            selection_color().gamma_multiply(0.10),
        );
    }
    let color = if !enabled || !ui.is_enabled() {
        quiet_color(ui).gamma_multiply(0.5)
    } else if selected {
        selection_color()
    } else {
        ui.visuals().strong_text_color()
    };
    paint_icon(
        ui.painter(),
        egui::Rect::from_center_size(response.rect.center(), egui::vec2(23.0, 23.0)),
        icon,
        color,
    );
    response.on_hover_text(label).on_disabled_hover_text(label)
}

fn toolbar_menu(
    ui: &mut egui::Ui,
    icon: LineIcon,
    label: &str,
    enabled: bool,
    contents: impl FnOnce(&mut egui::Ui),
) {
    let response = ui
        .add_enabled_ui(enabled, |ui| {
            let (response, _) = egui::containers::menu::MenuButton::from_button(
                egui::Button::new("")
                    .min_size(egui::vec2(45.0, 34.0))
                    .frame(false),
            )
            .ui(ui, contents);
            let color = if ui.is_enabled() {
                ui.visuals().strong_text_color()
            } else {
                quiet_color(ui).gamma_multiply(0.5)
            };
            paint_icon(
                ui.painter(),
                egui::Rect::from_center_size(
                    response.rect.center() - egui::vec2(5.0, 0.0),
                    egui::vec2(23.0, 23.0),
                ),
                icon,
                color,
            );
            let center = response.rect.right_center() - egui::vec2(6.0, 0.0);
            ui.painter().add(egui::Shape::line(
                vec![
                    center + egui::vec2(-2.5, -1.0),
                    center + egui::vec2(0.0, 1.5),
                    center + egui::vec2(2.5, -1.0),
                ],
                egui::Stroke::new(1.4, color),
            ));
            response
        })
        .inner;
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::Button, enabled && ui.is_enabled(), label)
    });
    response.on_hover_text(label).on_disabled_hover_text(label);
}

fn icon_button(
    ui: &mut egui::Ui,
    icon: LineIcon,
    label: &str,
    enabled: bool,
    selected: bool,
) -> egui::Response {
    let response = ui.add_enabled(
        enabled,
        egui::Button::new("")
            .min_size(egui::vec2(29.0, 27.0))
            .frame(false),
    );
    if selected {
        ui.painter()
            .rect_filled(response.rect, 5, selection_color().gamma_multiply(0.12));
    }
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::Button, enabled && ui.is_enabled(), label)
    });
    let color = if !enabled || !ui.is_enabled() {
        ui.visuals().weak_text_color().gamma_multiply(0.45)
    } else if selected {
        selection_color()
    } else {
        ui.visuals().text_color()
    };
    paint_icon(
        ui.painter(),
        egui::Rect::from_center_size(response.rect.center(), egui::vec2(19.0, 19.0)),
        icon,
        color,
    );
    response.on_hover_text(label).on_disabled_hover_text(label)
}

// Original geometric line icons. No platform symbol artwork or reference assets.
fn paint_icon(painter: &egui::Painter, rect: egui::Rect, icon: LineIcon, color: egui::Color32) {
    let point =
        |x: f32, y: f32| rect.min + egui::vec2(rect.width() * x / 20.0, rect.height() * y / 20.0);
    let stroke = egui::Stroke::new(1.35, color);
    let line = |points: &[(f32, f32)]| {
        painter.add(egui::Shape::line(
            points.iter().map(|(x, y)| point(*x, *y)).collect(),
            stroke,
        ));
    };
    match icon {
        LineIcon::Sidebar => {
            painter.rect_stroke(rect.shrink(1.5), 2, stroke, egui::StrokeKind::Inside);
            line(&[(7.0, 2.0), (7.0, 18.0)]);
            line(&[(3.5, 6.0), (5.0, 6.0)]);
            line(&[(3.5, 9.0), (5.0, 9.0)]);
        }
        LineIcon::SidebarRight => {
            painter.rect_stroke(rect.shrink(1.5), 2, stroke, egui::StrokeKind::Inside);
            line(&[(13.0, 2.0), (13.0, 18.0)]);
            line(&[(15.0, 6.0), (17.0, 6.0)]);
            line(&[(15.0, 9.0), (17.0, 9.0)]);
        }
        LineIcon::Move | LineIcon::Folder => {
            line(&[
                (2.0, 17.0),
                (2.0, 5.0),
                (8.0, 5.0),
                (10.0, 7.0),
                (18.0, 7.0),
                (18.0, 17.0),
                (2.0, 17.0),
            ]);
            line(&[(2.0, 9.0), (18.0, 9.0)]);
            if matches!(icon, LineIcon::Move) {
                line(&[(6.0, 13.0), (14.0, 13.0)]);
                line(&[(11.0, 10.0), (14.0, 13.0), (11.0, 16.0)]);
            }
        }
        LineIcon::Save => {
            line(&[
                (3.0, 2.0),
                (15.0, 2.0),
                (18.0, 5.0),
                (18.0, 18.0),
                (3.0, 18.0),
                (3.0, 2.0),
            ]);
            line(&[(10.0, 3.0), (10.0, 13.0)]);
            for y in [5.0, 8.0, 11.0] {
                line(&[(8.0, y), (12.0, y)]);
            }
            line(&[
                (8.0, 14.0),
                (12.0, 14.0),
                (12.0, 17.0),
                (8.0, 17.0),
                (8.0, 14.0),
            ]);
        }
        LineIcon::Delete => {
            line(&[(3.0, 5.0), (17.0, 5.0)]);
            line(&[(7.0, 5.0), (7.0, 2.0), (13.0, 2.0), (13.0, 5.0)]);
            line(&[(5.0, 5.0), (6.0, 18.0), (14.0, 18.0), (15.0, 5.0)]);
            line(&[(8.0, 8.0), (8.0, 15.0)]);
            line(&[(12.0, 8.0), (12.0, 15.0)]);
        }
        LineIcon::Check => {
            line(&[
                (16.0, 10.0),
                (16.0, 17.0),
                (3.0, 17.0),
                (3.0, 4.0),
                (11.0, 4.0),
            ]);
            line(&[(7.0, 9.0), (10.0, 12.0), (18.0, 2.0)]);
        }
        LineIcon::Extract | LineIcon::Add => {
            line(&[
                (2.0, 8.0),
                (18.0, 8.0),
                (18.0, 18.0),
                (2.0, 18.0),
                (2.0, 8.0),
            ]);
            line(&[(10.0, 1.0), (10.0, 10.0)]);
            if matches!(icon, LineIcon::Extract) {
                line(&[(6.0, 5.0), (10.0, 1.0), (14.0, 5.0)]);
            } else {
                line(&[(6.0, 6.0), (10.0, 10.0), (14.0, 6.0)]);
            }
        }
        LineIcon::Preview => {
            line(&[
                (13.0, 18.0),
                (3.0, 18.0),
                (3.0, 2.0),
                (12.0, 2.0),
                (16.0, 6.0),
            ]);
            painter.circle_stroke(point(11.0, 10.0), rect.width() * 0.22, stroke);
            line(&[(14.0, 13.0), (19.0, 18.0)]);
        }
        LineIcon::NewFolder => {
            line(&[
                (2.0, 17.0),
                (2.0, 5.0),
                (8.0, 5.0),
                (10.0, 7.0),
                (18.0, 7.0),
                (18.0, 17.0),
                (2.0, 17.0),
            ]);
            line(&[(7.0, 12.0), (13.0, 12.0)]);
            line(&[(10.0, 9.0), (10.0, 15.0)]);
        }
        LineIcon::Collect => {
            line(&[
                (3.0, 10.0),
                (1.0, 14.0),
                (1.0, 18.0),
                (19.0, 18.0),
                (19.0, 14.0),
                (17.0, 10.0),
            ]);
            line(&[
                (1.0, 14.0),
                (6.0, 14.0),
                (8.0, 16.0),
                (12.0, 16.0),
                (14.0, 14.0),
                (19.0, 14.0),
            ]);
            line(&[(10.0, 1.0), (10.0, 11.0)]);
            line(&[(6.0, 7.0), (10.0, 11.0), (14.0, 7.0)]);
        }
        LineIcon::DirectMode => {
            line(&[
                (5.0, 6.0),
                (15.0, 6.0),
                (15.0, 18.0),
                (5.0, 18.0),
                (5.0, 6.0),
            ]);
            line(&[(10.0, 7.0), (10.0, 15.0)]);
            line(&[(8.0, 9.0), (12.0, 9.0)]);
            line(&[(8.0, 12.0), (12.0, 12.0)]);
            line(&[(3.0, 1.0), (3.0, 5.0), (1.0, 3.0)]);
            line(&[(17.0, 5.0), (17.0, 1.0), (19.0, 3.0)]);
        }
        LineIcon::Share => {
            line(&[
                (6.0, 7.0),
                (3.0, 7.0),
                (3.0, 18.0),
                (17.0, 18.0),
                (17.0, 7.0),
                (14.0, 7.0),
            ]);
            line(&[(10.0, 12.0), (10.0, 1.0)]);
            line(&[(6.0, 5.0), (10.0, 1.0), (14.0, 5.0)]);
        }
        LineIcon::More => {
            for x in [4.0, 10.0, 16.0] {
                painter.circle_filled(point(x, 10.0), 1.5, color);
            }
        }
        LineIcon::Search => {
            painter.circle_stroke(point(8.5, 8.5), rect.width() * 0.32, stroke);
            line(&[(13.0, 13.0), (19.0, 19.0)]);
        }
        LineIcon::Copy => {
            painter.rect_stroke(
                egui::Rect::from_min_max(point(3.0, 6.0), point(14.0, 18.0)),
                1,
                stroke,
                egui::StrokeKind::Inside,
            );
            line(&[
                (7.0, 6.0),
                (7.0, 2.0),
                (17.0, 2.0),
                (17.0, 14.0),
                (14.0, 14.0),
            ]);
        }
        LineIcon::Refresh => {
            line(&[
                (16.0, 6.0),
                (13.0, 3.0),
                (7.0, 3.0),
                (3.0, 7.0),
                (3.0, 13.0),
                (7.0, 17.0),
                (13.0, 17.0),
                (17.0, 13.0),
            ]);
            line(&[(16.0, 1.0), (16.0, 6.0), (11.0, 6.0)]);
        }
        LineIcon::Back => line(&[(12.0, 4.0), (6.0, 10.0), (12.0, 16.0)]),
        LineIcon::Forward => line(&[(8.0, 4.0), (14.0, 10.0), (8.0, 16.0)]),
        LineIcon::File => {
            line(&[
                (4.0, 2.0),
                (12.0, 2.0),
                (17.0, 7.0),
                (17.0, 18.0),
                (4.0, 18.0),
                (4.0, 2.0),
            ]);
            line(&[(12.0, 2.0), (12.0, 7.0), (17.0, 7.0)]);
            line(&[(7.0, 11.0), (14.0, 11.0)]);
            line(&[(7.0, 14.0), (13.0, 14.0)]);
        }
    }
}

fn paint_folder_icon(painter: &egui::Painter, rect: egui::Rect) {
    let body = egui::Rect::from_min_max(
        rect.min + egui::vec2(1.0, 6.0),
        rect.max - egui::vec2(1.0, 2.0),
    );
    painter.rect_filled(
        egui::Rect::from_min_size(
            rect.min + egui::vec2(1.0, 3.0),
            egui::vec2(rect.width() * 0.48, 7.0),
        ),
        2,
        egui::Color32::from_rgb(113, 201, 237),
    );
    painter.rect_filled(body, 2, egui::Color32::from_rgb(79, 179, 223));
    painter.line_segment(
        [
            body.left_top() + egui::vec2(2.0, 2.0),
            body.right_top() + egui::vec2(-2.0, 2.0),
        ],
        egui::Stroke::new(1.0, egui::Color32::from_rgb(157, 222, 245)),
    );
    paint_icon(
        painter,
        rect,
        LineIcon::Folder,
        egui::Color32::from_rgb(55, 145, 183),
    );
}

fn paint_archive_icon(painter: &egui::Painter, rect: egui::Rect, image_file: bool) {
    painter.rect_filled(rect.shrink2(egui::vec2(4.0, 1.5)), 1, egui::Color32::WHITE);
    paint_icon(
        painter,
        rect,
        LineIcon::File,
        egui::Color32::from_rgb(170, 176, 183),
    );
    let badge = egui::Rect::from_center_size(
        rect.center() + egui::vec2(0.0, 4.0),
        egui::vec2(rect.width() * 0.58, 6.0),
    );
    painter.rect_filled(
        badge,
        1,
        if image_file {
            egui::Color32::from_rgb(101, 175, 114)
        } else {
            egui::Color32::from_rgb(97, 151, 195)
        },
    );
}

fn paint_disclosure(
    painter: &egui::Painter,
    rect: egui::Rect,
    expanded: bool,
    color: egui::Color32,
) {
    let center = rect.center();
    let points = if expanded {
        vec![
            center + egui::vec2(-3.5, -2.0),
            center + egui::vec2(3.5, -2.0),
            center + egui::vec2(0.0, 2.5),
        ]
    } else {
        vec![
            center + egui::vec2(-2.0, -3.5),
            center + egui::vec2(-2.0, 3.5),
            center + egui::vec2(2.5, 0.0),
        ]
    };
    painter.add(egui::Shape::convex_polygon(
        points,
        color,
        egui::Stroke::NONE,
    ));
}

fn table_columns(width: f32) -> [(&'static str, f32); 5] {
    [
        ("Name", width - 376.0),
        ("Modified", 146.0),
        ("Size", 74.0),
        ("Kind", 100.0),
        ("Ratio", 56.0),
    ]
}

fn table_stripes(ui: &egui::Ui, viewport: egui::Rect, origin_y: f32) -> egui::Shape {
    let viewport = viewport.intersect(ui.clip_rect());
    if !viewport.is_positive() {
        return egui::Shape::Noop;
    }
    let first = ((viewport.top() - origin_y) / 25.0).floor().max(0.0) as usize;
    let count = (viewport.height() / 25.0).ceil().max(0.0) as usize + 1;
    egui::Shape::Vec(
        (first..first + count)
            .map(|row| {
                let band = egui::Rect::from_min_size(
                    egui::pos2(viewport.left(), origin_y + row as f32 * 25.0),
                    egui::vec2(viewport.width(), 25.0),
                )
                .intersect(viewport);
                egui::Shape::rect_filled(
                    band,
                    0,
                    if row % 2 == 1 {
                        stripe_color(ui)
                    } else {
                        content_color(ui)
                    },
                )
            })
            .collect(),
    )
}

fn paint_cell(ui: &egui::Ui, rect: egui::Rect, text: &str, color: egui::Color32, right: bool) {
    let rect = rect.shrink2(egui::vec2(6.0, 0.0));
    ui.painter()
        .with_clip_rect(rect.intersect(ui.clip_rect()))
        .text(
            if right {
                rect.right_center()
            } else {
                rect.left_center()
            },
            if right {
                egui::Align2::RIGHT_CENTER
            } else {
                egui::Align2::LEFT_CENTER
            },
            text,
            typography::regular(14.0),
            color,
        );
}

fn property(ui: &mut egui::Ui, name: &str, value: &str) {
    ui.weak(name);
    ui.label(value);
    ui.end_row();
}

fn paint_header_cell(ui: &egui::Ui, rect: egui::Rect, text: &str) {
    let rect = rect.shrink2(egui::vec2(6.0, 0.0));
    let right = matches!(text, "Size" | "Ratio");
    let font = if text == "Name" {
        typography::semibold(14.0)
    } else {
        typography::regular(14.0)
    };
    ui.painter()
        .with_clip_rect(rect.intersect(ui.clip_rect()))
        .text(
            if right {
                rect.right_center()
            } else {
                rect.left_center()
            },
            if right {
                egui::Align2::RIGHT_CENTER
            } else {
                egui::Align2::LEFT_CENTER
            },
            text,
            font,
            ui.visuals().strong_text_color(),
        );
}

fn centered_heading(ui: &mut egui::Ui, text: &str) {
    ui.vertical_centered(|ui| {
        ui.add(egui::Label::new(egui::RichText::new(text).font(typography::semibold(18.0))).wrap());
    });
}

fn inspector_property(ui: &mut egui::Ui, name: &str, value: &str) {
    let width = ui.available_width();
    let label_width = (width * 0.28).clamp(65.0, 115.0);
    let height = ui.text_style_height(&egui::TextStyle::Body);
    ui.allocate_ui_with_layout(
        egui::vec2(width, height),
        egui::Layout::left_to_right(egui::Align::Min),
        |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let (label, _) =
                ui.allocate_exact_size(egui::vec2(label_width, height), egui::Sense::hover());
            ui.painter().text(
                label.right_top(),
                egui::Align2::RIGHT_TOP,
                name,
                typography::regular(14.0),
                quiet_color(ui),
            );
            ui.add(
                egui::Label::new(egui::RichText::new(value).font(typography::regular(14.0))).wrap(),
            );
        },
    );
}

fn display_basename(entry: &ArchiveEntry) -> String {
    let name = entry
        .name
        .display
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(&entry.name.display);
    if entry.name.provenance == NameProvenance::EscapedBytes {
        name.chars().take(1024).collect()
    } else {
        bounded_display(name)
    }
}

fn detailed_size(bytes: u64) -> String {
    if bytes < 1024 {
        return counted(bytes, "byte", "bytes");
    }
    let digits = bytes.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index != 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    format!("{} ({grouped} bytes)", format_bytes(bytes))
}

fn unix_permissions(entry: &ArchiveEntry) -> String {
    let Some(mode) = entry.unix_mode else {
        return "Unknown".into();
    };
    let mut text = String::from(match entry.kind {
        EntryKind::Directory => "d",
        EntryKind::Symlink => "l",
        EntryKind::File => "-",
        EntryKind::Special => "?",
    });
    for (read, write, execute, special, lower, upper) in [
        (0o400, 0o200, 0o100, 0o4000, 's', 'S'),
        (0o040, 0o020, 0o010, 0o2000, 's', 'S'),
        (0o004, 0o002, 0o001, 0o1000, 't', 'T'),
    ] {
        text.push(if mode & read != 0 { 'r' } else { '-' });
        text.push(if mode & write != 0 { 'w' } else { '-' });
        text.push(if mode & special != 0 {
            if mode & execute != 0 { lower } else { upper }
        } else if mode & execute != 0 {
            'x'
        } else {
            '-'
        });
    }
    text
}

fn is_image_name(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(_, extension)| {
        matches!(
            extension.to_ascii_lowercase().as_str(),
            "png" | "jpg" | "jpeg"
        )
    })
}

fn counted(count: u64, singular: &str, plural: &str) -> String {
    format!("{count} {}", if count == 1 { singular } else { plural })
}

fn view_status(rows: usize, entries: usize, searching: bool) -> String {
    format!(
        "{} · {}{}",
        counted(rows as u64, "visible row", "visible rows"),
        counted(entries as u64, "archive entry", "archive entries"),
        if searching { " · Search active" } else { "" }
    )
}

fn row_key(document: &PreparedDocument, row: ViewRow) -> RowKey {
    match row.folder {
        Some(folder) => RowKey::Folder(document.folders[folder].clone()),
        None => RowKey::Entry(document.snapshot.entries[row.entry.expect("prepared entry row")].id),
    }
}

fn row_full_path(document: &PreparedDocument, row: ViewRow) -> String {
    match row.folder {
        Some(folder) => bounded_display(&document.folders[folder]),
        None => display_name(&document.snapshot.entries[row.entry.expect("prepared entry row")]),
    }
}

fn row_display_name(document: &PreparedDocument, row: ViewRow, mode: ViewMode) -> String {
    let path = match row.folder {
        Some(folder) => document.folders[folder].as_str(),
        None => document.snapshot.entries[row.entry.expect("prepared entry row")]
            .name
            .display
            .as_str(),
    };
    let name = if mode == ViewMode::Outline {
        path.trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or(path)
    } else {
        path
    };
    if row.entry.is_some_and(|index| {
        document.snapshot.entries[index].name.provenance == NameProvenance::EscapedBytes
    }) {
        name.chars().take(1024).collect()
    } else {
        bounded_display(name)
    }
}

fn entry_kind(entry: &ArchiveEntry) -> String {
    match entry.kind {
        EntryKind::Directory => "Folder".into(),
        EntryKind::Symlink => "Symbolic link".into(),
        EntryKind::Special => "Special file".into(),
        EntryKind::File => {
            let name = entry
                .name
                .display
                .rsplit('/')
                .next()
                .unwrap_or(&entry.name.display);
            match name
                .rsplit_once('.')
                .map(|(_, extension)| extension.to_ascii_lowercase())
                .as_deref()
            {
                Some("png") => "PNG image".into(),
                Some("jpg" | "jpeg") => "JPEG image".into(),
                Some("txt") => "Plain text".into(),
                Some("md") => "Markdown".into(),
                Some("pdf") => "PDF document".into(),
                Some("json") => "JSON document".into(),
                Some("rs") => "Rust source".into(),
                Some(extension) if !extension.is_empty() && extension.len() <= 8 => {
                    format!("{} file", bounded_display(&extension.to_uppercase()))
                }
                _ => "File".into(),
            }
        }
    }
}

fn compression_ratio(entry: &ArchiveEntry) -> String {
    if entry.is_directory {
        "—".into()
    } else {
        size_ratio(entry.size, entry.compressed_size)
    }
}

fn size_ratio(size: u64, compressed_size: u64) -> String {
    if size == 0 {
        "—".into()
    } else {
        // BetterZip's Ratio column is packed/original, truncated to an integer.
        format!("{}%", u128::from(compressed_size) * 100 / u128::from(size))
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.2} GiB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

fn dialog_actions(ui: &mut egui::Ui, actions: impl FnOnce(&mut egui::Ui)) {
    // A bare right-to-left scope inherits all remaining vertical space. A
    // horizontal row gives its centered buttons a finite interactive height.
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), actions);
    });
}

fn theme_picker(ui: &mut egui::Ui) {
    let current = ui.ctx().options(|options| options.theme_preference);
    let mut preference = current;
    let label = match current {
        egui::ThemePreference::Light => "Light",
        egui::ThemePreference::Dark => "Dark",
        egui::ThemePreference::System => "System",
    };
    egui::ComboBox::from_id_salt("theme_preference")
        .selected_text(format!("Theme: {label}"))
        .width(100.0)
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut preference, egui::ThemePreference::Light, "Light");
            ui.selectable_value(&mut preference, egui::ThemePreference::Dark, "Dark");
            ui.selectable_value(&mut preference, egui::ThemePreference::System, "System");
        });
    if preference != current {
        ui.ctx().set_theme(preference);
    }
}

fn valid_new_folder_name(name: &str) -> bool {
    let mut components = Path::new(name).components();
    !name.is_empty()
        && !name.contains(['/', '\\', '\0'])
        && matches!(
            (components.next(), components.next()),
            (Some(std::path::Component::Normal(_)), None)
        )
}

fn default_extraction_destination(source: &Path) -> PathBuf {
    let parent = source
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut leaf = source
        .file_stem()
        .unwrap_or(OsStr::new("archive"))
        .to_os_string();
    leaf.push("-extracted");
    parent.join(leaf)
}

fn encoding_picker(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    choice: &mut FilenameChoice,
) -> bool {
    let previous = *choice;
    egui::ComboBox::from_id_salt(id)
        .selected_text(
            choice
                .encoding
                .map(FilenameEncoding::label)
                .unwrap_or("Automatic"),
        )
        .show_ui(ui, |ui| {
            ui.selectable_value(
                &mut choice.encoding,
                None,
                "Automatic (use filename evidence)",
            );
            for encoding in FilenameEncoding::ALL {
                ui.selectable_value(&mut choice.encoding, Some(*encoding), encoding.label());
            }
        });
    if choice.encoding.is_none() {
        choice.recovery = false;
    }
    ui.add_enabled(choice.encoding.is_some(), egui::Checkbox::new(&mut choice.recovery, "Recovery: ignore filename metadata"))
        .on_hover_text("Explicit recovery overrides conflicting or invalid filename metadata; original bytes are preserved.");
    *choice != previous
}

fn display_name(entry: &ArchiveEntry) -> String {
    let display: String = entry.name.display.chars().take(1024).collect();
    let mut display = if entry.name.provenance == NameProvenance::EscapedBytes {
        display
    } else {
        escape_display(&display)
    };
    if entry.name.display.chars().nth(1024).is_some() {
        display.push_str("… [see original bytes]");
    }
    display
}

fn bounded_display(value: &str) -> String {
    let mut display = escape_display(&value.chars().take(1024).collect::<String>());
    if value.chars().nth(1024).is_some() {
        display.push('…');
    }
    display
}

fn method_name(method: u16) -> String {
    match method {
        0 => "Stored".into(),
        8 => "Deflate".into(),
        value => format!("Method {value}"),
    }
}

fn show_diagnostics(ui: &mut egui::Ui, diagnostics: &[String]) {
    for diagnostic in diagnostics.iter().take(20) {
        ui.small(escape_display(diagnostic));
    }
    if diagnostics.len() > 20 {
        ui.small(format!(
            "{} more diagnostics; use CLI JSON for the full list.",
            diagnostics.len() - 20
        ));
    }
}

fn show_hex(ui: &mut egui::Ui, id: impl std::hash::Hash + std::fmt::Debug, text: &str) {
    let count = text.len().div_ceil(64);
    if count == 0 {
        ui.weak("Empty");
        return;
    }
    egui::ScrollArea::both()
        .id_salt(id)
        .max_height(110.0)
        .show_rows(
            ui,
            ui.text_style_height(&egui::TextStyle::Monospace),
            count,
            |ui, range| {
                for index in range {
                    let start = index * 64;
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(&text[start..(start + 64).min(text.len())])
                                .monospace(),
                        )
                        .wrap_mode(egui::TextWrapMode::Extend),
                    );
                }
            },
        );
}

fn disabled_action(ui: &mut egui::Ui, label: &str) {
    ui.add_enabled(false, egui::Button::new(label))
        .on_disabled_hover_text("This operation is planned, not implemented.");
}
