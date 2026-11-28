//! Desktop presentation over the shared read-only archive commands.

mod jobs;

#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::sync::Arc;

use arcforge_core::archive::{
    ArchiveEntry, ArchiveError, CancellationToken, EntryId, FilenameChoice, FilenameEncoding,
    FilenamePolicy, NameProvenance, NameState, escape_display,
};
use arcforge_core::{Availability, COMMANDS, VERSION};
use jobs::{Completed, PreparedDocument, PreparedPreview, Reply, Request, Task, Ticket, Worker};

pub struct ArchiveWorkspace {
    initial_path: Option<PathBuf>,
    worker: Option<Worker>,
    document: Option<Arc<PreparedDocument>>,
    rows: Vec<usize>,
    selected: Option<EntryId>,
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
        if ui.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::O)) {
            self.show_open = true;
        }
        if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            if self.show_open {
                self.show_open = false;
            } else if self.current.is_some() {
                self.cancel();
            }
        }
        self.toolbar(ui);
        self.status_bar(ui);
        self.folders(ui);
        if self.show_inspector {
            self.inspector(ui);
        }
        self.entries(ui);
        self.open_window(ui.ctx());
        self.commands_window(ui.ctx());
    }

    fn poll(&mut self) {
        let replies = match self.worker.as_mut().map(Worker::poll) {
            Some(Ok(replies)) => replies,
            Some(Err(error)) => {
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
        self.current = None;
        self.blocks_selection = false;
        match reply.result {
            Ok(Completed::Document {
                document,
                rows,
                query,
            }) => {
                self.policy = document.snapshot.policy.clone();
                if self.selected.is_some_and(|id| document.entry(id).is_none()) {
                    self.selected = None;
                }
                self.status = format!("{} entries · Read-only ZIP", document.snapshot.entries.len());
                self.document = Some(document);
                self.rows = rows;
                self.applied_query = query.clone();
                self.query = query;
                self.applied_folder = None;
                self.folder = None;
                self.error = None;
                self.source_changed = false;
            }
            Ok(Completed::Filter {
                rows,
                query,
                folder,
            }) => {
                self.status = format!("{} matching entries", rows.len());
                self.rows = rows;
                self.applied_query = query.clone();
                self.query = query;
                self.applied_folder = folder.clone();
                self.folder = folder;
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
        self.next_generation();
        self.document = None;
        self.rows.clear();
        self.selected = None;
        self.preview = None;
        self.query.clear();
        self.folder = None;
        self.applied_query.clear();
        self.applied_folder = None;
        self.policy = FilenamePolicy::default();
        self.source_changed = false;
        self.status = "Archive closed.".into();
    }

    fn cancel_worker(&mut self) {
        if let Some(worker) = &mut self.worker {
            worker.cancel();
        }
        self.current = None;
        self.blocks_selection = false;
    }

    fn cancel(&mut self) {
        self.cancel_worker();
        if let Some(document) = &self.document {
            self.policy = document.snapshot.policy.clone();
        }
        self.restore_filter();
        self.status = "Operation cancelled; the archive was not changed.".into();
    }

    fn filter(&mut self, context: &egui::Context) {
        let Some(document) = self.document.clone() else {
            return;
        };
        self.selected = None;
        self.preview = None;
        // A changed search must not replace a pending name reinterpretation with
        // a filter of the previous names while displaying the new encoding policy.
        let task = if self.policy != document.snapshot.policy {
            self.folder = None;
            Task::Reinterpret {
                document,
                policy: self.policy.clone(),
                query: self.query.clone(),
            }
        } else {
            Task::Filter {
                document,
                query: self.query.clone(),
                folder: self.folder.clone(),
            }
        };
        self.submit(task, true, "Filtering entries…", context);
    }

    fn restore_filter(&mut self) {
        self.query = self.applied_query.clone();
        self.folder = self.applied_folder.clone();
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
        if self.source_changed || self.blocks_selection {
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

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.strong("ArcForge");
                ui.separator();
                if ui
                    .button("Open…")
                    .on_hover_text("Open a ZIP path (Ctrl/Cmd+O), or drop a file here")
                    .clicked()
                {
                    self.show_open = true;
                }
                if ui
                    .add_enabled(
                        self.document.is_some() || self.current.is_some(),
                        egui::Button::new("Close"),
                    )
                    .clicked()
                {
                    self.close();
                }
                if ui
                    .add_enabled(self.document.is_some(), egui::Button::new("Reopen"))
                    .on_hover_text("Read the source again; filename choices reset for the new snapshot")
                    .clicked()
                    && let Some(document) = &self.document
                {
                    self.open(document.snapshot.path().to_path_buf(), ui.ctx());
                }
                ui.separator();
                if ui
                    .add_enabled(
                        self.selected.is_some() && !self.blocks_selection && !self.source_changed,
                        egui::Button::new("Preview"),
                    )
                    .clicked()
                {
                    self.preview_selected(ui.ctx());
                }
                for label in ["Add", "Extract", "Test", "Delete", "Save"] {
                    disabled_action(ui, label);
                }
                if ui
                    .add_enabled(self.current.is_some(), egui::Button::new("Cancel"))
                    .clicked()
                {
                    self.cancel();
                }
            });
            ui.horizontal(|ui| {
                let source = self
                    .document
                    .as_ref()
                    .map(|document| document.snapshot.source.as_str())
                    .unwrap_or("No archive open");
                ui.add(egui::Label::new(escape_display(source)).truncate())
                    .on_hover_text(escape_display(source));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.toggle_value(&mut self.show_inspector, "Info");
                    ui.toggle_value(&mut self.show_commands, "Commands");
                    egui::widgets::global_theme_preference_buttons(ui);
                });
            });
            let enabled = self.document.is_some() && !self.source_changed;
            ui.add_enabled_ui(enabled, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label("Find:");
                    if ui
                        .add(egui::TextEdit::singleline(&mut self.query).hint_text("Search full names"))
                        .changed()
                    {
                        self.filter(ui.ctx());
                    }
                    ui.label("Archive filename fallback:");
                    let mut policy = self.policy.clone();
                    if encoding_picker(ui, "archive_encoding", &mut policy.archive) {
                        self.change_policy(policy, ui.ctx());
                    }
                });
            });
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("status").show(ui, |ui| {
            if let Some(error) = &self.error {
                ui.colored_label(ui.visuals().error_fg_color, escape_display(error));
            }
            if self.source_changed {
                ui.label("The source archive changed. Use Reopen to load a new snapshot.");
            }
            ui.horizontal(|ui| {
                if self.current.is_some() {
                    ui.spinner();
                }
                ui.small(&self.status);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.small(format!("{VERSION} · Read-only"));
                });
            });
        });
    }

    fn folders(&mut self, ui: &mut egui::Ui) {
        let document = self.document.clone();
        egui::Panel::left("archive_tree")
            .default_size(210.0)
            .min_size(150.0)
            .resizable(true)
            .show(ui, |ui| {
                ui.heading("Archive");
                ui.separator();
                let Some(document) = document else {
                    ui.weak("No archive open");
                    ui.small("Open a ZIP to browse its folder hierarchy.");
                    return;
                };
                ui.add_enabled_ui(!self.source_changed, |ui| {
                    if ui.selectable_label(self.folder.is_none(), "All entries").clicked() {
                        self.folder = None;
                        self.filter(ui.ctx());
                    }
                    ui.small("Folders include their descendants.");
                    if document.folders_limited {
                        ui.small("Folder list capped; all entries remain searchable.");
                    }
                    egui::ScrollArea::both()
                        .id_salt("folder_rows")
                        .show_rows(ui, 22.0, document.folders.len(), |ui, range| {
                            for index in range {
                                let folder = &document.folders[index];
                                let label = bounded_display(folder);
                                if ui
                                    .selectable_label(self.folder.as_deref() == Some(folder), label)
                                    .clicked()
                                {
                                    self.folder = Some(folder.clone());
                                    self.filter(ui.ctx());
                                }
                            }
                        });
                });
            });
    }

    fn entries(&mut self, ui: &mut egui::Ui) {
        let document = self.document.clone();
        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                for (label, width) in [
                    ("Name", 280.0),
                    ("Size", 85.0),
                    ("Packed", 85.0),
                    ("Method", 95.0),
                    ("Modified", 150.0),
                ] {
                    ui.add_sized([width, 20.0], egui::Label::new(egui::RichText::new(label).strong()));
                }
            });
            ui.separator();
            let Some(document) = document else {
                ui.add_space(70.0);
                ui.vertical_centered(|ui| {
                    ui.heading("No archive open");
                    ui.label("Open a ZIP path or drop one archive into this window.");
                    ui.small("Browsing and small-entry previews are available. Editing and extraction are planned.");
                    if ui.button("Open ZIP…").clicked() {
                        self.show_open = true;
                    }
                });
                return;
            };
            if self.rows.is_empty() {
                ui.weak("No matching entries");
            }
            let mut clicked = None;
            let mut folder = None;
            ui.add_enabled_ui(!self.blocks_selection, |ui| {
                egui::ScrollArea::both()
                    .id_salt("entry_rows")
                    .show_rows(ui, 24.0, self.rows.len(), |ui, range| {
                        for row in range {
                            let entry = &document.snapshot.entries[self.rows[row]];
                            ui.push_id(entry.id, |ui| {
                                ui.horizontal(|ui| {
                                    let label = entry_label(entry);
                                    let response = ui.add_sized(
                                        [280.0, 22.0],
                                        egui::Button::new(label).selected(self.selected == Some(entry.id)),
                                    );
                                    if response.clicked() {
                                        clicked = Some(entry.id);
                                    }
                                    if response.double_clicked() && entry.is_directory {
                                        folder = Some(entry.name.display.clone());
                                    }
                                    for (value, width) in [
                                        (entry.size.to_string(), 85.0),
                                        (entry.compressed_size.to_string(), 85.0),
                                        (method_name(entry.compression_method), 95.0),
                                        (entry.modified.as_deref().unwrap_or("—").to_owned(), 150.0),
                                    ] {
                                        ui.add_sized([width, 22.0], egui::Label::new(value).truncate());
                                    }
                                });
                            });
                        }
                    });
            });
            if let Some(folder) = folder {
                self.folder = Some(folder);
                self.filter(ui.ctx());
            } else if let Some(id) = clicked {
                self.cancel_worker();
                self.selected = Some(id);
                self.preview = None;
                if document.entry(id).is_some_and(|entry| !entry.is_directory) {
                    self.preview_selected(ui.ctx());
                }
            }
        });
    }

    fn inspector(&mut self, ui: &mut egui::Ui) {
        let document = self.document.clone();
        egui::Panel::right("inspector")
            .default_size(300.0)
            .min_size(210.0)
            .resizable(true)
            .show(ui, |ui| {
                ui.heading("Information");
                ui.separator();
                let Some(document) = document else {
                    ui.weak("No item selected");
                    ui.small("Select an archive entry to inspect its properties.");
                    return;
                };
                egui::ScrollArea::vertical().id_salt("information_scroll").show(ui, |ui| {
                    if let Some(entry) = self.selected.and_then(|id| document.entry(id)) {
                        ui.label(display_name(entry));
                        ui.label(format!("Entry ID: {}", entry.id));
                        ui.label(format!("Size: {} bytes", entry.size));
                        ui.label(format!("Packed: {} bytes", entry.compressed_size));
                        ui.label(format!("Method: {}", method_name(entry.compression_method)));
                        ui.label(format!("CRC32: {:08x}", entry.crc32));
                        ui.label(format!("Encrypted: {}", entry.encrypted));
                        ui.label(format!("Modified: {}", entry.modified.as_deref().unwrap_or("Unknown")));
                        ui.separator();
                        ui.strong("Filename interpretation");
                        ui.label(format!("State: {:?}", entry.name.state));
                        ui.label(format!("Evidence: {:?}", entry.name.provenance));
                        ui.label(format!("Encoding: {}", entry.name.encoding.map(FilenameEncoding::label).unwrap_or("Unresolved")));
                        show_diagnostics(ui, &entry.name.diagnostics);
                        let mut policy = self.policy.clone();
                        let mut overridden = policy.entries.contains_key(&entry.id);
                        let previous_override = overridden;
                        let mut choice = policy.entries.get(&entry.id).copied().unwrap_or(policy.archive);
                        let mut changed = false;
                        ui.add_enabled_ui(!self.source_changed && !self.blocks_selection, |ui| {
                            changed |= ui.checkbox(&mut overridden, "Override this entry's filename interpretation").changed();
                            if overridden {
                                changed |= encoding_picker(ui, ("entry_encoding", entry.id), &mut choice);
                            }
                        });
                        if changed || previous_override != overridden {
                            if overridden {
                                policy.entries.insert(entry.id, choice);
                            } else {
                                policy.entries.remove(&entry.id);
                            }
                            self.change_policy(policy, ui.ctx());
                        }
                        ui.small("These controls change the displayed interpretation; archive bytes remain unchanged.");
                        ui.separator();
                        ui.strong("Preview");
                        if entry.is_directory {
                            ui.weak("Directory entry");
                        } else if self.preview.as_ref().is_some_and(|value| value.preview.entry_id == entry.id) {
                            self.preview_view(ui);
                        } else {
                            ui.small("Stored/Deflate entries up to 256 KiB; the full payload is read and its CRC32 checked.");
                            if ui.add_enabled(!self.blocks_selection && !self.source_changed, egui::Button::new("Load preview")).clicked() {
                                self.preview_selected(ui.ctx());
                            }
                        }
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
                    } else {
                        ui.strong("Archive properties");
                        ui.label(format!("Entries: {}", document.snapshot.entries.len()));
                        ui.label(format!("Size: {} bytes", document.snapshot.uncompressed_size));
                        ui.label(format!("Packed: {} bytes", document.snapshot.compressed_size));
                        ui.small("Select an entry to view name evidence and a bounded preview.");
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
        ui.label(format!("{} bytes · CRC32 verified", preview.preview.bytes.len()));
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
            .show_rows(ui, ui.text_style_height(&egui::TextStyle::Monospace), rows.len(), |ui, range| {
                for index in range {
                    ui.monospace(&rows[index]);
                }
            });
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
                let field = ui.add(egui::TextEdit::singleline(&mut self.path_input).desired_width(f32::INFINITY));
                ui.small("You can also drop one file into the workspace or pass its path at launch.");
                open = ui.add_enabled(!self.path_input.is_empty(), egui::Button::new("Open")).clicked()
                    || (field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) && !self.path_input.is_empty());
            });
        self.show_open = visible && !open;
        if open {
            self.open(PathBuf::from(&self.path_input), context);
        }
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

fn encoding_picker(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    choice: &mut FilenameChoice,
) -> bool {
    let previous = *choice;
    egui::ComboBox::from_id_salt(id)
        .selected_text(choice.encoding.map(FilenameEncoding::label).unwrap_or("Automatic"))
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut choice.encoding, None, "Automatic (use filename evidence)");
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

fn entry_label(entry: &ArchiveEntry) -> String {
    let name = display_name(entry);
    match entry.name.state {
        NameState::Provisional => format!("{name} [assumed]"),
        NameState::Conflict | NameState::Invalid => format!("{name} [name issue]"),
        NameState::Resolved => name,
    }
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
        display.push_str("…");
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
        ui.small(format!("{} more diagnostics; use CLI JSON for the full list.", diagnostics.len() - 20));
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
        .show_rows(ui, ui.text_style_height(&egui::TextStyle::Monospace), count, |ui, range| {
            for index in range {
                let start = index * 64;
                ui.monospace(&text[start..(start + 64).min(text.len())]);
            }
        });
}

fn disabled_action(ui: &mut egui::Ui, label: &str) {
    ui.add_enabled(false, egui::Button::new(label))
        .on_disabled_hover_text("This operation is planned, not implemented.");
}
