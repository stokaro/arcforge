//! Maintainer-only driving of real workspace commands for native CI captures.
//!
//! This module never inserts a document, preview, or completed operation. The
//! ordinary archive worker opens the fixture and produces every displayed byte.

use std::path::Path;

use super::edits::DISCARD_MODAL_ID;
use super::{ArchiveWorkspace, EntryId, ViewMode, WorkspaceCommand};

const REMOVED_ENTRY: &str = "Photos/Color-study.png";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Scene {
    Text,
    Image,
    StagedDelete,
    DiscardPrompt,
    #[cfg(target_os = "macos")]
    SavePanel,
    SaveCollision,
    SavedCopy,
    Empty,
    #[cfg(target_os = "macos")]
    OpenPanel,
}

impl Scene {
    pub const ALL: &'static [Self] = &[
        Self::Text,
        Self::Image,
        Self::StagedDelete,
        Self::DiscardPrompt,
        #[cfg(target_os = "macos")]
        Self::SavePanel,
        Self::SaveCollision,
        Self::SavedCopy,
        Self::Empty,
        #[cfg(target_os = "macos")]
        Self::OpenPanel,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Text => "01-text-preview",
            Self::Image => "02-image-preview",
            Self::StagedDelete => "05-staged-delete",
            Self::DiscardPrompt => "06-discard-prompt",
            #[cfg(target_os = "macos")]
            Self::SavePanel => "07-save-panel",
            Self::SaveCollision => "08-save-collision",
            Self::SavedCopy => "09-saved-copy",
            Self::Empty => "03-empty-window",
            #[cfg(target_os = "macos")]
            Self::OpenPanel => "04-open-panel",
        }
    }

    pub fn entry_name(self) -> &'static str {
        match self {
            Self::Text => "Documents/Guide.txt",
            Self::Image => REMOVED_ENTRY,
            _ => "",
        }
    }

    #[cfg(target_os = "macos")]
    pub fn is_panel(self) -> bool {
        matches!(self, Self::OpenPanel | Self::SavePanel)
    }
}

/// Observed state after the worker has returned a complete CRC-checked preview.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReadyScene {
    Preview(ReadyPreview),
    Editing(ReadyEdit),
    Empty,
    #[cfg(target_os = "macos")]
    OpenPanelRequested,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadyEdit {
    pub snapshot_id: String,
    pub archive_name: String,
    pub visible_entry_count: u64,
    pub pending_deletions: usize,
    pub discard_prompt: bool,
    pub picker_pending: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadyPreview {
    pub snapshot_id: String,
    pub metadata_revision: String,
    pub entry_id: EntryId,
    pub entry_name: String,
    pub preview_bytes: usize,
    pub image_size: Option<[usize; 2]>,
}

impl ArchiveWorkspace {
    /// Drive selection through the same commands used by the visible workspace.
    /// Call after `show`; `Some` means the requested state was actually drawn in
    /// that UI pass. The native harness separately waits for renderer callbacks.
    pub fn prepare_native_capture(
        &mut self,
        scene: Scene,
        context: &egui::Context,
        saved_copy: &Path,
    ) -> Result<Option<ReadyScene>, String> {
        if let Some(error) = self.filesystem_favorites_startup_error() {
            return Err(error.to_owned());
        }
        if !self.filesystem_favorites_ready() {
            return Ok(None);
        }
        match scene {
            Scene::StagedDelete
            | Scene::DiscardPrompt
            | Scene::SaveCollision
            | Scene::SavedCopy => {
                return self.prepare_edit_capture(scene, context, saved_copy);
            }
            #[cfg(target_os = "macos")]
            Scene::SavePanel => return self.prepare_edit_capture(scene, context, saved_copy),
            _ => {}
        }
        if scene.entry_name().is_empty() {
            if self.document.is_some() || self.initial_path.is_some() {
                self.close();
                return Ok(None);
            }
            if self.current.is_some() {
                return Ok(None);
            }
            #[cfg(target_os = "macos")]
            if scene == Scene::OpenPanel {
                if !self.native_picker_pending() {
                    self.request_open_archive();
                    return Ok(None);
                }
                return Ok(Some(ReadyScene::OpenPanelRequested));
            }
            if self.selected.is_some() || self.preview.is_some() || self.native_picker_pending() {
                return Err("empty capture retained a selection, preview, or active picker".into());
            }
            return Ok(Some(ReadyScene::Empty));
        }
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        if self.source_changed {
            return Err("the fixture changed while preparing the capture".into());
        }
        if self.current.is_some() || self.initial_path.is_some() {
            return Ok(None);
        }
        let Some(document) = self.document.clone() else {
            return Ok(None);
        };
        let mut matches = document
            .snapshot
            .entries
            .iter()
            .filter(|entry| entry.name.display == scene.entry_name());
        let entry = matches
            .next()
            .ok_or_else(|| format!("capture fixture lacks {}", scene.entry_name()))?;
        if matches.next().is_some() {
            return Err(format!(
                "capture fixture has ambiguous {}",
                scene.entry_name()
            ));
        }
        if entry.is_directory {
            return Err("capture preview requires a file".into());
        }

        if self.view_mode != ViewMode::Outline || !self.query.is_empty() || self.folder.is_some() {
            return Err("capture requires the ordinary unfiltered Outline workspace".into());
        }
        let mut changed = false;
        for (offset, character) in scene.entry_name().char_indices() {
            if character == '/' {
                changed |= self
                    .expanded_folders
                    .insert(scene.entry_name()[..=offset].to_owned());
            }
        }
        if changed {
            self.filter(context);
            return Ok(None);
        }

        if self.selected != Some(entry.id) {
            let row = self
                .rows
                .iter()
                .position(|row| {
                    row.entry
                        .is_some_and(|index| document.snapshot.entries[index].id == entry.id)
                })
                .ok_or("capture entry is not visible in the prepared Outline rows")?;
            self.select_row(row, egui::Modifiers::NONE);
            self.preview_selected(context);
            return Ok(None);
        }
        let Some(preview) = &self.preview else {
            self.preview_selected(context);
            return Ok(None);
        };
        if preview.preview.entry_id != entry.id || !preview.preview.crc32_verified {
            return Err("capture preview does not match the CRC-checked selection".into());
        }
        let image_size = match scene {
            Scene::Text if preview.text_rows.is_some() && !self.preview_hex => None,
            Scene::Text => return Err("text capture did not produce a text preview".into()),
            Scene::Image => {
                let image = preview.image.as_ref().ok_or_else(|| {
                    preview
                        .image_error
                        .clone()
                        .unwrap_or_else(|| "image capture did not decode an image".into())
                })?;
                if !self.preview_show_image || self.preview_texture.is_none() {
                    return Ok(None);
                }
                Some(image.size)
            }
            _ => unreachable!("non-preview scenes return before archive inspection"),
        };
        Ok(Some(ReadyScene::Preview(ReadyPreview {
            snapshot_id: document.snapshot.id.clone(),
            metadata_revision: document.snapshot.metadata_revision.clone(),
            entry_id: entry.id,
            entry_name: entry.name.display.clone(),
            preview_bytes: preview.preview.bytes.len(),
            image_size,
        })))
    }

    fn prepare_edit_capture(
        &mut self,
        scene: Scene,
        context: &egui::Context,
        saved_copy: &Path,
    ) -> Result<Option<ReadyScene>, String> {
        if self.source_changed {
            return Err("the archive changed during the edit capture".into());
        }
        if self.current.is_some() {
            return Ok(None);
        }
        let document = self
            .document
            .clone()
            .ok_or("edit capture lost its document")?;
        let copy_open = document.snapshot.path() == saved_copy;
        let dirty = document.edits.is_dirty();
        match scene {
            Scene::StagedDelete if !dirty => {
                let selected = self.selected.and_then(|id| document.entry(id));
                if selected.is_none_or(|entry| entry.name.display != REMOVED_ENTRY)
                    || !self.toolbar_state().can_delete
                {
                    return Err("edit capture could not delete the selected fixture image".into());
                }
                self.dispatch_command(WorkspaceCommand::DeleteSelected, context);
                return Ok(None);
            }
            Scene::DiscardPrompt if self.discard.is_none() => {
                self.dispatch_command(WorkspaceCommand::CloseArchive, context);
                return Ok(None);
            }
            Scene::DiscardPrompt => {
                // The ordinary modal fades in. Wait for its actual visibility
                // timestamp plus the configured duration before collecting pixels.
                let visible_since =
                    egui::AreaState::load(context, egui::Id::unique(DISCARD_MODAL_ID))
                        .and_then(|area| area.last_became_visible_at);
                let now = context.input(|input| input.time);
                if visible_since.is_none_or(|time| {
                    now - time < f64::from(context.global_style().animation_time)
                }) {
                    return Ok(None);
                }
            }
            #[cfg(target_os = "macos")]
            Scene::SavePanel if self.discard.take().is_some() => return Ok(None),
            #[cfg(target_os = "macos")]
            Scene::SavePanel if !self.native_picker_pending() => {
                self.dispatch_command(WorkspaceCommand::SaveAs, context);
                return Ok(None);
            }
            Scene::SaveCollision
                if self.discard.take().is_some() || self.native_picker_pending() =>
            {
                return Ok(None);
            }
            Scene::SaveCollision if self.error.is_none() => {
                self.start_save_as(document.snapshot.path().to_path_buf(), context);
                return Ok(None);
            }
            Scene::SavedCopy if !copy_open => {
                if self
                    .error
                    .as_deref()
                    .is_none_or(|error| !error.contains("already exists"))
                {
                    return Err(
                        "native edit capture did not observe the expected destination conflict"
                            .into(),
                    );
                }
                self.start_save_as(saved_copy.to_owned(), context);
                return Ok(None);
            }
            _ => {}
        }
        let image_still_visible =
            document.snapshot.entries.iter().any(|entry| {
                entry.name.display == REMOVED_ENTRY && document.entry(entry.id).is_some()
            });
        if image_still_visible
            || (copy_open == dirty)
            || (!copy_open && document.edits.removed.len() != 1)
        {
            return Err("edit capture does not show the intended staged or saved deletion".into());
        }
        if (scene == Scene::SaveCollision)
            != self
                .error
                .as_deref()
                .is_some_and(|error| error.contains("already exists"))
        {
            return Err("edit capture did not reach the expected save outcome".into());
        }
        Ok(Some(ReadyScene::Editing(ReadyEdit {
            snapshot_id: document.snapshot.id.clone(),
            archive_name: super::archive_label(document.snapshot.path()),
            visible_entry_count: document.totals.entries,
            pending_deletions: document.edits.removed.len(),
            discard_prompt: self.discard.is_some(),
            picker_pending: self.native_picker_pending(),
            error: self.error.clone(),
        })))
    }
}
