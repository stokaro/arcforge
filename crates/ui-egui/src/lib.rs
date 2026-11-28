//! Desktop presentation over shared archive browsing and extraction commands.

mod jobs;

#[cfg(test)]
mod tests;

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arcforge_core::archive::{
    ArchiveEntry, ArchiveError, CancellationToken, EntryId, EntryKind, ExtractionLimits,
    ExtractionPhase, ExtractionProgress, ExtractionReport, ExtractionSelection, ExtractionStatus,
    FilenameChoice, FilenameEncoding, FilenamePolicy, NameProvenance, NameState, escape_display,
};
use arcforge_core::{Availability, COMMANDS, VERSION};
use jobs::{
    Completed, FolderTotals, PreparedDocument, PreparedPreview, ProgressReply, Reply, Request,
    Task, Ticket, ViewMode, ViewRow, Worker,
};

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum RowKey {
    Entry(EntryId),
    Folder(String),
}

#[derive(Clone, Copy, Default, Eq, PartialEq)]
enum ExtractionScope {
    #[default]
    Selected,
    All,
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
    destination_input: String,
    state: ExtractionState,
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
    show_sidebar: bool,
    show_settings: bool,
    style_initialized: bool,
    extraction: ExtractionDialog,
    preview: Option<PreparedPreview>,
    preview_hex: bool,
    policy: FilenamePolicy,
    query: String,
    folder: Option<String>,
    applied_query: String,
    applied_folder: Option<String>,
    path_input: String,
    show_open: bool,
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
            show_sidebar: true,
            show_settings: false,
            style_initialized: false,
            extraction: ExtractionDialog::default(),
            preview: None,
            preview_hex: false,
            policy: FilenamePolicy::default(),
            query: String::new(),
            folder: None,
            applied_query: String::new(),
            applied_folder: None,
            path_input: String::new(),
            show_open: false,
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

    pub fn show(&mut self, ui: &mut egui::Ui) {
        if !self.style_initialized {
            configure_style(ui.ctx());
            self.style_initialized = true;
        }
        self.poll();
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
            self.show_open = true;
        }
        if ui.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::B)) {
            self.show_sidebar = !self.show_sidebar;
        }
        if !self.extraction_active()
            && ui.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::F))
        {
            ui.memory_mut(|memory| memory.request_focus(egui::Id::unique("archive_search")));
        }
        if !self.extraction_active()
            && !ui.ctx().text_edit_focused()
            && !self.show_open
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
        if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            if self.extraction_active() {
                self.cancel();
            } else if self.extraction.open {
                self.extraction.open = false;
            } else if self.show_open {
                self.show_open = false;
            } else if self.current.is_some() {
                self.cancel();
            }
        }
        self.toolbar(ui);
        self.status_bar(ui);
        if self.show_sidebar {
            self.favorites(ui);
        }
        if self.show_inspector {
            self.inspector(ui);
        }
        self.entries(ui);
        self.open_window(ui.ctx());
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
        self.preview = None;
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
        self.document = None;
        self.rows.clear();
        self.selected = None;
        self.selected_entries.clear();
        self.selected_folders.clear();
        self.selected_folder = None;
        self.selection_anchor = None;
        self.extraction = ExtractionDialog::default();
        self.preview = None;
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
        self.selected = None;
        self.selected_folder = None;
        self.preview = None;
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
        self.preview = None;
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

    fn preview_selected(&mut self, context: &egui::Context) {
        let (Some(document), Some(entry)) = (self.document.clone(), self.selected) else {
            return;
        };
        if self.source_changed || self.blocks_selection || self.extraction_active() {
            return;
        }
        self.preview = None;
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
        if self.extraction_active() || self.blocks_selection || self.source_changed {
            return;
        }
        let Some(document) = &self.document else {
            return;
        };
        if self.selected_entries.is_empty()
            && self.selected_folders.is_empty()
            && let Some(entry) = self.selected
        {
            self.selected_entries.insert(entry);
        }
        let destination = default_extraction_destination(document.snapshot.path());
        self.extraction = ExtractionDialog {
            open: true,
            scope,
            destination_input: destination.to_string_lossy().into_owned(),
            destination,
            state: ExtractionState::Editing,
        };
    }

    fn start_extraction(&mut self, context: &egui::Context) {
        if self.extraction_active() || self.blocks_selection || self.source_changed {
            return;
        }
        let Some(document) = self.document.clone() else {
            return;
        };
        if self.extraction.destination.as_os_str().is_empty() {
            self.error = Some("Enter the full path of a new destination directory.".into());
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
        if self.blocks_selection || self.extraction_active() {
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
        self.preview = None;
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
        if self.blocks_selection || self.extraction_active() {
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

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let idle = !self.extraction_active();
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
            .unwrap_or_else(|| "ArcForge".into());
        egui::Panel::top("toolbar")
            .frame(
                egui::Frame::NONE
                    .fill(chrome_color(ui))
                    .inner_margin(egui::Margin::symmetric(12, 8)),
            )
            .show(ui, |ui| {
                let wide = ui.available_width() >= 1000.0;
                ui.horizontal(|ui| {
                    if icon_button(
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
                    ui.add_sized(
                        [if wide { 210.0 } else { 145.0 }, 31.0],
                        egui::Label::new(
                            egui::RichText::new(bounded_display(&title))
                                .strong()
                                .size(15.0),
                        )
                        .truncate(),
                    )
                    .on_hover_text(bounded_display(&title));
                    toolbar_group(ui, |ui| {
                        if icon_button(
                            ui,
                            LineIcon::Open,
                            "Open archive… (Ctrl/Cmd+O)",
                            idle,
                            false,
                        )
                        .clicked()
                        {
                            self.show_open = true;
                        }
                        if wide {
                            icon_button(ui, LineIcon::Save, "Save — planned", false, false);
                        }
                    });
                    toolbar_group(ui, |ui| {
                        ui.add_enabled_ui(idle, |ui| {
                            ui.menu_button("View", |ui| {
                                for (mode, label) in [
                                    (ViewMode::Outline, "Outline   Ctrl/Cmd+2"),
                                    (ViewMode::Flat, "Flat   Ctrl/Cmd+1"),
                                ] {
                                    if ui.selectable_label(self.view_mode == mode, label).clicked()
                                    {
                                        self.set_view(mode, ui.ctx());
                                        ui.close();
                                    }
                                }
                                ui.separator();
                                disabled_action(ui, "Columns");
                                disabled_action(ui, "Icons");
                            });
                        });
                    });
                    toolbar_group(ui, |ui| {
                        ui.add_enabled_ui(ready, |ui| {
                            ui.menu_button("Extract", |ui| {
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
                            });
                        });
                        if wide {
                            icon_button(ui, LineIcon::Delete, "Delete — planned", false, false);
                        }
                    });
                    toolbar_group(ui, |ui| {
                        ui.menu_button("Archive", |ui| {
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
                            disabled_action(ui, "Save");
                            disabled_action(ui, "Delete");
                            disabled_action(ui, "Add files…");
                            disabled_action(ui, "Test archive");
                            ui.separator();
                            if ui.button("Settings…").clicked() {
                                self.show_settings = true;
                                ui.close();
                            }
                            if ui.button("Command availability…").clicked() {
                                self.show_commands = true;
                                ui.close();
                            }
                        });
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icon_button(
                            ui,
                            LineIcon::Info,
                            "Show Info and Preview",
                            true,
                            self.show_inspector,
                        )
                        .clicked()
                        {
                            self.show_inspector = !self.show_inspector;
                        }
                        if wide
                            && icon_button(
                                ui,
                                LineIcon::Settings,
                                "Settings and filename encoding",
                                true,
                                false,
                            )
                            .clicked()
                        {
                            self.show_settings = true;
                        }
                        ui.add_enabled_ui(
                            self.document.is_some() && idle && !self.source_changed,
                            |ui| {
                                if ui
                                    .add(
                                        egui::TextEdit::singleline(&mut self.query)
                                            .id(egui::Id::unique("archive_search"))
                                            .hint_text("Search archive")
                                            .desired_width(if wide { 200.0 } else { 145.0 }),
                                    )
                                    .changed()
                                {
                                    self.filter(ui.ctx());
                                }
                            },
                        );
                    });
                });
            });
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

    fn favorites(&mut self, ui: &mut egui::Ui) {
        let current = self
            .document
            .as_ref()
            .map(|document| document.snapshot.path().to_path_buf());
        egui::Panel::left("favorites")
            .default_size(185.0)
            .min_size(145.0)
            .max_size(340.0)
            .resizable(true)
            .frame(egui::Frame::NONE.fill(sidebar_color(ui)).inner_margin(10))
            .show(ui, |ui| {
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new("Favorites")
                        .strong()
                        .color(ui.visuals().weak_text_color()),
                );
                ui.add_space(9.0);
                ui.small("Session archives");
                if self.recent_archives.is_empty() {
                    ui.add_space(8.0);
                    ui.weak("Opened archives appear here.");
                }
                let mut open = None;
                egui::ScrollArea::vertical()
                    .id_salt("favorite_archives")
                    .show(ui, |ui| {
                        ui.add_enabled_ui(!self.extraction_active(), |ui| {
                            for path in &self.recent_archives {
                                let title = path
                                    .file_name()
                                    .unwrap_or(path.as_os_str())
                                    .to_string_lossy();
                                let response = ui
                                    .add_sized(
                                        [ui.available_width(), 27.0],
                                        egui::Button::new(bounded_display(&title))
                                            .selected(current.as_ref() == Some(path))
                                            .truncate(),
                                    )
                                    .on_hover_text(escape_display(&path.to_string_lossy()));
                                if response.clicked() && current.as_ref() != Some(path) {
                                    open = Some(path.clone());
                                }
                            }
                        });
                    });
                if let Some(path) = open {
                    self.open(path, ui.ctx());
                }
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.small("Current session")
                        .on_hover_text("This list contains only archives opened in this session.");
                });
            });
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
                ui.add_space(90.0);
                ui.vertical_centered(|ui| {
                    ui.heading("No archive open");
                    ui.add_space(10.0);
                    ui.label("Open a ZIP archive or drop one into this window.");
                    ui.weak("Browse, preview, and extract into a new directory.");
                    ui.add_space(12.0);
                    if ui.button("Open archive…").clicked() {
                        self.show_open = true;
                    }
                });
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
                let width = ui.available_width().max(680.0);
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
                }
                if self.rows.is_empty() {
                    ui.add_space(12.0);
                    ui.weak("No matching entries");
                }
                ui.add_enabled_ui(!self.blocks_selection && !self.extraction_active(), |ui| {
                    egui::ScrollArea::vertical().id_salt("contents_rows").auto_shrink([false, false])
                        .show_rows(ui, 25.0, self.rows.len(), |ui, range| {
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
                                let icon_rect = egui::Rect::from_min_size(rect.left_top() + egui::vec2(25.0 + indent, 4.0), egui::vec2(17.0, 17.0));
                                let directory = row.folder.is_some() || row.entry.is_some_and(|entry| document.snapshot.entries[entry].is_directory);
                                paint_icon(ui.painter(), icon_rect, if directory { LineIcon::Folder } else { LineIcon::File }, if selected { color } else { egui::Color32::from_rgb(93, 128, 153) });
                                let name_rect = egui::Rect::from_min_max(egui::pos2(rect.left() + 45.0 + indent, rect.top()), egui::pos2(rect.left() + columns[0].1, rect.bottom()));
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

    fn inspector(&mut self, ui: &mut egui::Ui) {
        let document = self.document.clone();
        egui::Panel::right("inspector")
            .default_size(275.0)
            .min_size(220.0)
            .max_size(600.0)
            .resizable(true)
            .frame(egui::Frame::NONE.fill(content_color(ui)).inner_margin(12))
            .show(ui, |ui| {
                let Some(document) = document else {
                    ui.weak("No item selected");
                    return;
                };
                egui::ScrollArea::vertical().id_salt("information_scroll").show(ui, |ui| {
                    if let Some(entry) = self.selected.and_then(|id| document.entry(id)) {
                        let totals = self.selected_folder.as_ref()
                            .and_then(|folder| document.folders.binary_search(folder).ok())
                            .map(|folder| document.folder_totals[folder]);
                        ui.add_space(5.0);
                        ui.add(egui::Label::new(egui::RichText::new(display_name(entry)).strong().size(15.0)));
                        ui.add_space(8.0);
                        ui.strong("Info");
                        egui::Grid::new("entry_properties").num_columns(2).spacing([12.0, 5.0]).show(ui, |ui| {
                            property(ui, "Kind", &entry_kind(entry));
                            property(ui, "Size", &format!("{} bytes", totals.map_or(entry.size, |totals| totals.size)));
                            property(ui, "Packed", &format!("{} bytes", totals.map_or(entry.compressed_size, |totals| totals.compressed_size)));
                            property(ui, "Modified", entry.modified.as_deref().unwrap_or("Unknown"));
                            if let Some(totals) = totals {
                                property(ui, "Items", &totals.entries.to_string());
                                property(ui, "Ratio", &size_ratio(totals.size, totals.compressed_size));
                            } else {
                                property(ui, "Method", &method_name(entry.compression_method));
                                property(ui, "Encrypted", if entry.encrypted { "Yes" } else { "No" });
                                property(ui, "Ratio", &compression_ratio(entry));
                            }
                        });
                        ui.add_space(6.0);
                        ui.separator();
                        if totals.is_some() {
                            ui.small("Folder totals include descendant items.");
                        } else {
                            ui.horizontal(|ui| {
                                ui.weak("CRC32");
                                ui.monospace(format!("{:08x}", entry.crc32));
                            }).response.on_hover_text("Checksum declared by the archive. A loaded preview verifies its complete payload.");
                        }
                        ui.separator();
                        ui.strong("Preview");
                        if entry.is_directory {
                            ui.weak("Folder");
                        } else if self.preview.as_ref().is_some_and(|value| value.preview.entry_id == entry.id) {
                            self.preview_view(ui);
                        } else {
                            ui.small("Stored/Deflate, up to 256 KiB. The complete payload is checked before display.");
                            if ui.add_enabled(!self.blocks_selection && !self.source_changed && !self.extraction_active(), egui::Button::new("Load preview")).clicked() {
                                self.preview_selected(ui.ctx());
                            }
                        }
                        ui.add_space(8.0);
                        ui.separator();
                        let issue = matches!(entry.name.state, NameState::Invalid | NameState::Conflict | NameState::Provisional);
                        egui::CollapsingHeader::new("Filename interpretation").default_open(issue).show(ui, |ui| {
                            ui.label(format!("Entry ID: {}", entry.id));
                            ui.label(format!("State: {:?}", entry.name.state));
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
                                if overridden {
                                    policy.entries.insert(entry.id, choice);
                                } else {
                                    policy.entries.remove(&entry.id);
                                }
                                self.change_policy(policy, ui.ctx());
                            }
                            ui.small("Interpretation changes preserve original archive bytes. Archive fallback is in Settings.");
                        });
                        ui.collapsing("Original filename bytes (hex)", |ui| {
                            show_hex(ui, ("raw_name", entry.id), &entry.raw_name_hex);
                        });
                        ui.collapsing("Original extra fields (hex)", |ui| {
                            show_hex(ui, ("raw_extra", entry.id), &entry.raw_extra_hex);
                        });
                        ui.collapsing("Original entry comment (hex)", |ui| {
                            show_hex(ui, ("raw_comment", entry.id), &entry.raw_comment_hex);
                        });
                        show_diagnostics(ui, &entry.warnings);
                    } else if let Some(folder) = &self.selected_folder {
                        ui.heading(bounded_display(folder.trim_end_matches('/')));
                        ui.add_space(8.0);
                        ui.strong("Info");
                        ui.label("Folder inferred from entry paths");
                        if let Ok(index) = document.folders.binary_search(folder) {
                            folder_properties(ui, document.folder_totals[index]);
                        }
                        ui.small("Selecting this folder includes its descendants when extracting.");
                        ui.separator();
                        ui.strong("Preview");
                        ui.weak("Folder");
                    } else {
                        ui.add_space(5.0);
                        ui.strong("Archive info");
                        ui.add_space(8.0);
                        egui::Grid::new("archive_properties").num_columns(2).spacing([12.0, 5.0]).show(ui, |ui| {
                            property(ui, "Entries", &document.snapshot.entries.len().to_string());
                            property(ui, "Size", &format_bytes(document.snapshot.uncompressed_size));
                            property(ui, "Packed", &format_bytes(document.snapshot.compressed_size));
                        });
                        ui.add_space(10.0);
                        ui.small("Select an item for its properties and preview.");
                        ui.separator();
                        show_diagnostics(ui, &document.snapshot.warnings);
                        ui.collapsing("Archive comment (hex)", |ui| {
                            show_hex(ui, "archive_comment", &document.snapshot.comment_hex);
                        });
                    }
                });
            });
    }

    fn preview_view(&mut self, ui: &mut egui::Ui) {
        let Some(preview) = &self.preview else {
            return;
        };
        ui.label(format!(
            "{} bytes · CRC32 verified",
            preview.preview.bytes.len()
        ));
        ui.horizontal(|ui| {
            ui.add_enabled_ui(preview.text_rows.is_some(), |ui| {
                ui.selectable_value(&mut self.preview_hex, false, "Text (UTF-8)");
            });
            ui.selectable_value(&mut self.preview_hex, true, "Hex");
        });
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
            .max_height(240.0)
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

    fn open_window(&mut self, context: &egui::Context) {
        if !self.show_open {
            return;
        }
        let mut visible = true;
        let mut open = false;
        egui::Window::new("Open ZIP")
            .open(&mut visible)
            .collapsible(false)
            .default_width(480.0)
            .show(context, |ui| {
                ui.label("Archive path:");
                let field = ui.add(
                    egui::TextEdit::singleline(&mut self.path_input).desired_width(f32::INFINITY),
                );
                ui.small(
                    "You can also drop one file into the workspace or pass its path at launch.",
                );
                open = ui
                    .add_enabled(!self.path_input.is_empty(), egui::Button::new("Open"))
                    .clicked()
                    || (field.lost_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter))
                        && !self.path_input.is_empty());
            });
        self.show_open = visible && !open;
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
                    ui.label("To — new directory");
                    let field = ui.add(egui::TextEdit::singleline(&mut self.extraction.destination_input)
                        .desired_width(f32::INFINITY)
                        .hint_text("Full path of a new directory"));
                    if field.changed() {
                        self.extraction.destination = PathBuf::from(&self.extraction.destination_input);
                    }
                    if self.extraction.destination.to_str().is_none() {
                        ui.small("The native path is preserved. Editing this display replaces it with the entered Unicode path.");
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
                    let allowed = !self.extraction.destination.as_os_str().is_empty()
                        && (self.extraction.scope == ExtractionScope::All || selected > 0)
                        && !self.blocks_selection && !self.source_changed;
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        dismiss = ui.button("Return to archive").clicked();
                        edit = ui.button("Edit destination").clicked();
                    });
                }
                ExtractionState::Interrupted(error) => {
                    ui.heading("Extraction worker stopped");
                    ui.colored_label(ui.visuals().error_fg_color, escape_display(error));
                    ui.label("The final output state is unknown. Inspect this directory for retained files:");
                    ui.label(bounded_display(&self.extraction.destination.to_string_lossy()));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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

fn configure_style(context: &egui::Context) {
    context.set_theme(egui::ThemePreference::Light);
    context.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(7.0, 5.0);
        style.spacing.button_padding = egui::vec2(8.0, 4.0);
        style
            .text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
        style
            .text_styles
            .insert(egui::TextStyle::Button, egui::FontId::proportional(14.0));
        style
            .text_styles
            .insert(egui::TextStyle::Small, egui::FontId::proportional(12.0));
        style
            .text_styles
            .insert(egui::TextStyle::Heading, egui::FontId::proportional(18.0));
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
        egui::Color32::from_rgb(242, 243, 245)
    }
}

fn sidebar_color(ui: &egui::Ui) -> egui::Color32 {
    if ui.visuals().dark_mode {
        egui::Color32::from_rgb(36, 38, 41)
    } else {
        egui::Color32::from_rgb(239, 241, 244)
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
        egui::Color32::from_rgb(247, 248, 250)
    }
}

fn selection_color() -> egui::Color32 {
    egui::Color32::from_rgb(52, 119, 218)
}

#[derive(Clone, Copy)]
enum LineIcon {
    Sidebar,
    Open,
    Save,
    Delete,
    Info,
    Settings,
    Back,
    Forward,
    Folder,
    File,
}

fn toolbar_group(ui: &mut egui::Ui, contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::NONE
        .fill(content_color(ui))
        .stroke(egui::Stroke::new(
            1.0,
            ui.visuals().widgets.noninteractive.bg_stroke.color,
        ))
        .corner_radius(6)
        .inner_margin(2)
        .show(ui, |ui| {
            ui.horizontal(contents);
        });
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
        LineIcon::Open | LineIcon::Folder => {
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
            if matches!(icon, LineIcon::Open) {
                line(&[(10.0, 15.0), (10.0, 11.0)]);
                line(&[(8.0, 13.0), (10.0, 11.0), (12.0, 13.0)]);
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
            line(&[(6.0, 2.0), (6.0, 8.0), (14.0, 8.0), (14.0, 2.0)]);
            line(&[(6.0, 18.0), (6.0, 12.0), (15.0, 12.0), (15.0, 18.0)]);
        }
        LineIcon::Delete => {
            line(&[(3.0, 5.0), (17.0, 5.0)]);
            line(&[(7.0, 5.0), (7.0, 2.0), (13.0, 2.0), (13.0, 5.0)]);
            line(&[(5.0, 5.0), (6.0, 18.0), (14.0, 18.0), (15.0, 5.0)]);
            line(&[(8.0, 8.0), (8.0, 15.0)]);
            line(&[(12.0, 8.0), (12.0, 15.0)]);
        }
        LineIcon::Info => {
            painter.circle_stroke(rect.center(), rect.width() * 0.43, stroke);
            painter.circle_filled(point(10.0, 5.5), 1.0, color);
            line(&[(9.0, 9.0), (10.0, 9.0), (10.0, 15.0)]);
        }
        LineIcon::Settings => {
            for (y, knob) in [(4.0, 7.0), (10.0, 13.0), (16.0, 9.0)] {
                line(&[(2.0, y), (18.0, y)]);
                painter.circle_filled(point(knob, y), 2.0, color);
            }
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
        ("Name", width - 394.0),
        ("Modified", 150.0),
        ("Size", 82.0),
        ("Kind", 102.0),
        ("Ratio", 60.0),
    ]
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
            egui::FontId::proportional(14.0),
            color,
        );
}

fn property(ui: &mut egui::Ui, name: &str, value: &str) {
    ui.weak(name);
    ui.label(value);
    ui.end_row();
}

fn paint_header_cell(ui: &egui::Ui, rect: egui::Rect, text: &str) {
    paint_cell(ui, rect, text, ui.visuals().strong_text_color(), false);
}

fn folder_properties(ui: &mut egui::Ui, totals: FolderTotals) {
    egui::Grid::new("folder_properties")
        .num_columns(2)
        .spacing([12.0, 5.0])
        .show(ui, |ui| {
            property(ui, "Size", &format_bytes(totals.size));
            property(ui, "Packed", &format_bytes(totals.compressed_size));
            property(ui, "Items", &totals.entries.to_string());
            property(
                ui,
                "Ratio",
                &size_ratio(totals.size, totals.compressed_size),
            );
        });
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
            match name.rsplit_once('.') {
                Some((_, extension)) if !extension.is_empty() && extension.len() <= 8 => {
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
