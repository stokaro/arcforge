//! Desktop presentation over the UI-independent command contract.

use arcforge_core::{Availability, COMMANDS, VERSION, diagnostics};

pub struct ArchiveWorkspace {
    show_inspector: bool,
    show_commands: bool,
}

impl Default for ArchiveWorkspace {
    fn default() -> Self {
        Self {
            show_inspector: true,
            show_commands: false,
        }
    }
}

impl ArchiveWorkspace {
    pub fn show(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.strong("ArcForge");
                ui.separator();
                disabled_action(ui, "Back");
                disabled_action(ui, "Forward");
                ui.separator();
                disabled_action(ui, "Open");
                disabled_action(ui, "Add");
                disabled_action(ui, "New folder");
                ui.separator();
                disabled_action(ui, "Extract");
                disabled_action(ui, "Test");
                disabled_action(ui, "Preview");
                disabled_action(ui, "Delete");
            });
            ui.horizontal(|ui| {
                ui.small("No archive open");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.toggle_value(&mut self.show_inspector, "Info");
                    ui.toggle_value(&mut self.show_commands, "Commands");
                    egui::widgets::global_theme_preference_buttons(ui);
                });
            });
        });

        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.small("Archive / No selection");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.small(format!("{VERSION} · Bootstrap"));
                });
            });
        });

        egui::Panel::left("archive_tree")
            .default_size(220.0)
            .min_size(150.0)
            .resizable(true)
            .show(ui, |ui| {
                ui.heading("Archive");
                ui.separator();
                ui.weak("No archive open");
                ui.add_space(18.0);
                ui.small("The folder hierarchy will appear here.");
            });

        if self.show_inspector {
            egui::Panel::right("inspector")
                .default_size(260.0)
                .min_size(190.0)
                .resizable(true)
                .show(ui, |ui| {
                    ui.heading("Information");
                    ui.separator();
                    ui.weak("No item selected");
                    ui.add_space(18.0);
                    ui.small("Select an archive entry to inspect its properties.");
                    ui.add_space(24.0);
                    ui.collapsing("Application status", |ui| {
                        let report = diagnostics();
                        ui.label(format!("Version: {}", report.application.version));
                        ui.label(format!(
                            "Archive providers: {}",
                            report.archive_provider_count
                        ));
                        ui.label("Archive operations are not implemented yet.");
                    });
                });
        }

        egui::CentralPanel::default().show(ui, |ui| {
            egui::Grid::new("entry_columns")
                .num_columns(5)
                .min_col_width(60.0)
                .show(ui, |ui| {
                    for name in ["Name", "Size", "Packed", "Kind", "Modified"] {
                        ui.strong(name);
                    }
                    ui.end_row();
                });
            ui.separator();
            ui.add_space(70.0);
            ui.vertical_centered(|ui| {
                ui.heading("No archive open");
                ui.add_space(8.0);
                ui.label("This development preview cannot open or edit archives yet.");
                ui.add_space(16.0);
                if ui.button("Show available commands").clicked() {
                    self.show_commands = true;
                }
            });
        });

        egui::Window::new("Command availability")
            .open(&mut self.show_commands)
            .default_width(560.0)
            .show(ui.ctx(), |ui| {
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

fn disabled_action(ui: &mut egui::Ui, label: &str) {
    ui.add_enabled(false, egui::Button::new(label))
        .on_disabled_hover_text("Archive operations are planned, not implemented.");
}
