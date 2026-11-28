use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use arcforge_core::archive::EntryId;

use super::jobs::{PreparedDocument, Task, ViewMode, ViewRow};
use super::{ArchiveWorkspace, NativePickerPurpose, archive_label};

pub(super) const DISCARD_MODAL_ID: &str = "discard-staged-edits";

pub(super) enum EditAction {
    Delete {
        ids: BTreeSet<EntryId>,
        folders: BTreeSet<String>,
    },
    Undo,
    Redo,
}

#[derive(Clone, Default)]
pub(super) struct RemovalHistory {
    pub removed: BTreeSet<EntryId>,
    undo: Vec<Arc<[EntryId]>>,
    redo: Vec<Arc<[EntryId]>>,
}

impl RemovalHistory {
    pub fn is_dirty(&self) -> bool {
        !self.removed.is_empty()
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn delete(&mut self, ids: BTreeSet<EntryId>) {
        let added: Arc<[EntryId]> = ids
            .difference(&self.removed)
            .copied()
            .collect::<Vec<_>>()
            .into();
        if !added.is_empty() {
            self.removed.extend(added.iter().copied());
            self.undo.push(added);
            self.redo.clear();
        }
    }

    pub fn undo(&mut self) {
        if let Some(batch) = self.undo.pop() {
            for id in batch.iter() {
                self.removed.remove(id);
            }
            self.redo.push(batch);
        }
    }

    pub fn redo(&mut self) {
        if let Some(batch) = self.redo.pop() {
            self.removed.extend(batch.iter().copied());
            self.undo.push(batch);
        }
    }
}

pub(super) enum DiscardAction {
    Open(PathBuf),
    CloseArchive,
    CloseWindow,
}

impl ArchiveWorkspace {
    pub(super) fn has_staged_edits(&self) -> bool {
        self.document
            .as_ref()
            .is_some_and(|doc| doc.edits.is_dirty())
    }

    pub(super) fn delete_selection(&mut self, context: &egui::Context) {
        if self.toolbar_state().can_delete {
            self.edit(
                EditAction::Delete {
                    ids: self.selected_entries.clone(),
                    folders: self.selected_folders.clone(),
                },
                context,
            );
        }
    }

    pub(super) fn edit(&mut self, action: EditAction, context: &egui::Context) {
        let Some(document) = self.document.clone() else {
            return;
        };
        if !self.next_generation() {
            return;
        }
        self.submit(
            Task::Edit {
                document,
                action,
                query: self.applied_query.clone(),
                folder: self.applied_folder.clone(),
                mode: self.applied_view_mode,
                expanded: self.applied_expanded_folders.clone(),
            },
            true,
            "Updating staged changes…",
            context,
        );
        self.edit_pending = self.current.is_some();
    }

    pub(super) fn apply_edit(
        &mut self,
        document: Arc<PreparedDocument>,
        rows: Vec<ViewRow>,
        folder: Option<String>,
    ) {
        if self.selected.is_some_and(|id| document.entry(id).is_none()) {
            self.selected = None;
            self.clear_preview();
        }
        self.selected_entries
            .retain(|id| document.entry(*id).is_some());
        let valid_folder = |path: &String| document.folders.binary_search(path).is_ok();
        self.selected_folders.retain(valid_folder);
        if self
            .selected_folder
            .as_ref()
            .is_some_and(|path| !valid_folder(path))
        {
            self.selected_folder = None;
        }
        self.selection_anchor = None;
        self.navigation
            .retain(|item| item.as_ref().is_none_or(valid_folder));
        if let Some(index) = self.navigation.iter().rposition(|item| *item == folder) {
            self.navigation_index = index;
        } else {
            self.navigation.push(folder.clone());
            self.navigation_index = self.navigation.len() - 1;
        }
        self.folder = folder.clone();
        self.applied_folder = folder;
        self.status = super::view_status(
            rows.len(),
            document.totals.entries as usize,
            !self.query.is_empty(),
        );
        self.rows = rows;
        self.document = Some(document);
        self.error = None;
    }

    pub(super) fn request_save_as(&mut self) {
        if self.toolbar_state().can_save_as {
            self.native_picker_request = Some(NativePickerPurpose::SaveAs);
        }
    }

    pub(super) fn start_save_as(&mut self, destination: PathBuf, context: &egui::Context) {
        if !self.toolbar_state().can_save_as {
            return;
        }
        let Some(document) = self.document.clone() else {
            return;
        };
        if !self.next_generation() {
            return;
        }
        self.submit(
            Task::Save {
                document,
                destination,
            },
            true,
            "Writing and verifying ZIP copy…",
            context,
        );
        if self.current.is_some() {
            self.saving = Some(false);
        }
    }

    /// True if the viewport must remain open for a result or a user decision.
    pub(super) fn hold_window_close(&mut self) -> bool {
        if self.file_job_active() {
            self.cancel();
            self.extraction.open |= self.extraction_active();
            return true;
        }
        if self.native_picker_pending() {
            return true;
        }
        if self.edit_pending {
            self.cancel();
        }
        if self.has_staged_edits() {
            self.discard = Some(DiscardAction::CloseWindow);
            return true;
        }
        false
    }

    pub(super) fn discard_window(&mut self, context: &egui::Context) {
        if self.discard.is_none() {
            return;
        }
        let name = self.document.as_ref().map_or_else(
            || "the archive".into(),
            |document| archive_label(document.snapshot.path()),
        );
        let decision = egui::Modal::new(egui::Id::unique(DISCARD_MODAL_ID)).show(context, |ui| {
            ui.set_width(330.0);
            ui.heading("Discard unsaved changes?");
            ui.label(format!("Staged deletions in {name} will be lost."));
            ui.add_space(8.0);
            let mut discard = false;
            let mut cancel = false;
            super::dialog_actions(ui, |ui| {
                discard = ui.button("Discard Changes").clicked();
                cancel = ui.button("Cancel").clicked();
            });
            (discard, cancel)
        });
        if decision.inner.0 {
            match self.discard.take() {
                Some(DiscardAction::Open(path)) => self.open_document(path, context),
                Some(DiscardAction::CloseArchive) => self.close_document(),
                Some(DiscardAction::CloseWindow) => {
                    self.close_document();
                    context.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                None => {}
            }
        } else if decision.inner.1 || decision.should_close() {
            self.discard = None;
        }
    }

    pub(super) fn apply_save(
        &mut self,
        report: arcforge_core::archive::ZipEditReport,
        view: Result<(Arc<PreparedDocument>, Vec<ViewRow>), arcforge_core::archive::ArchiveError>,
    ) {
        match view {
            Ok((document, rows)) => {
                let name = archive_label(document.snapshot.path());
                self.selected = None;
                self.selected_entries.clear();
                self.selection_anchor = None;
                self.clear_preview();
                self.extraction = super::ExtractionDialog::default();
                self.install_document(document, rows, String::new(), ViewMode::Outline);
                self.status = format!("Saved {name} · {} removed", report.entries_removed);
            }
            Err(error) => {
                self.status = format!(
                    "ZIP copy saved to {}",
                    super::bounded_display(&report.destination)
                );
                self.error = Some(format!("Could not display the saved copy: {error}"));
            }
        }
    }
}
