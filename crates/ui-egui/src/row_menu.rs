use arcforge_core::archive::{EntryKind, NameState};

use super::{ArchiveWorkspace, RowKey, WorkspaceCommand, jobs::PreparedDocument};

impl ArchiveWorkspace {
    pub(super) fn entry_context_menu(
        &mut self,
        response: &egui::Response,
        document: &PreparedDocument,
        row: &RowKey,
        index: usize,
    ) -> bool {
        if !response.enabled() {
            return false;
        }
        let context_click = response.container_secondary_clicked()
            || (cfg!(target_os = "macos")
                && response.container_contains_pointer()
                && response
                    .ctx
                    .input(|input| input.modifiers.ctrl && input.pointer.primary_clicked()));
        if context_click && !self.row_selected(row) {
            self.select_row(index, egui::Modifiers::NONE);
        }
        // Bind the menu to the entry, even when virtual rows are reused while
        // scrolling or a new archive is installed.
        let id = egui::Id::unique(("archive-entry-menu", self.generation, row));
        let open = if context_click {
            Some(egui::SetOpenCommand::Bool(true))
        } else if response.clicked() {
            Some(egui::SetOpenCommand::Bool(false))
        } else {
            None
        };
        egui::Popup::context_menu(response)
            .id(id)
            .open_memory(open)
            .show(|ui| self.show_entry_menu(ui, document, row));
        #[cfg(feature = "native-capture")]
        if matches!(row, RowKey::Entry(entry) if self.selected == Some(*entry))
            && response.interact_rect.is_positive()
        {
            self.capture_entry_menu = Some((response.interact_rect.center(), id));
        }
        context_click
    }

    fn show_entry_menu(&mut self, ui: &mut egui::Ui, document: &PreparedDocument, row: &RowKey) {
        ui.set_min_width(172.0);
        let state = self.toolbar_state();
        if self.selection_count() == 1 && self.row_selected(row) {
            let file = match row {
                RowKey::Entry(id) => document.entry(*id),
                RowKey::Folder(_) => None,
            };
            let folder = match row {
                RowKey::Folder(path) => Some(path.as_str()),
                RowKey::Entry(_) => file.filter(|entry| entry.is_directory).and_then(|entry| {
                    let path = format!("{}/", entry.name.display.trim_end_matches('/'));
                    let index = document.folders.binary_search(&path).ok()?;
                    Some(document.folders[index].as_str())
                }),
            };
            if let Some(path) = folder {
                if menu_action(ui, self.workspace_idle(), "Open Folder") {
                    self.navigate(Some(path.to_owned()), ui.ctx());
                }
                ui.separator();
            } else if let Some(entry) = file.filter(|entry| !entry.is_directory) {
                let can_open = state.can_preview
                    && entry.kind == EntryKind::File
                    && entry.name.state == NameState::Resolved
                    && !entry.encrypted
                    && matches!(entry.compression_method, 0 | 8);
                if menu_action(ui, can_open, "Open in Default App") {
                    self.open_file(entry.id, ui.ctx());
                }
                if menu_action(ui, state.can_preview, "Preview") {
                    self.dispatch_command(WorkspaceCommand::PreviewSelected, ui.ctx());
                }
                ui.separator();
            }
        }
        if menu_action(ui, state.can_extract_selected, "Extract Selected…") {
            self.dispatch_command(WorkspaceCommand::ExtractSelected, ui.ctx());
        }
        if menu_action(ui, state.can_delete, "Delete Selected") {
            self.dispatch_command(WorkspaceCommand::DeleteSelected, ui.ctx());
        }
    }
}

fn menu_action(ui: &mut egui::Ui, enabled: bool, text: &str) -> bool {
    let clicked = ui.add_enabled(enabled, egui::Button::new(text)).clicked();
    if clicked {
        ui.close();
    }
    clicked
}
