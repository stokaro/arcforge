use std::process::ExitCode;

use arcforge_core::application_info;
use arcforge_ui_egui::ArchiveWorkspace;

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() == 1 && (args[0] == "--version" || args[0] == "-V") {
        let info = application_info();
        println!("{} {} ({})", info.name, info.version, info.stage);
        return ExitCode::SUCCESS;
    }
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        println!(
            "Usage: arcforge [--help | --version]\n\nLaunch the ArcForge desktop bootstrap.\nArchive operations are not implemented yet. Use arcforge-cli for command discovery."
        );
        return ExitCode::SUCCESS;
    }
    if !args.is_empty() {
        eprintln!(
            "arcforge: unexpected arguments; archive opening is not implemented. See --help."
        );
        return ExitCode::from(2);
    }

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 680.0])
            .with_min_inner_size([900.0, 440.0])
            .with_app_id("org.stokaro.arcforge")
            .with_icon(eframe::egui::IconData::default()),
        ..Default::default()
    };
    match eframe::run_native(
        "ArcForge",
        options,
        Box::new(|_| Ok(Box::<DesktopApp>::default())),
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("arcforge: could not start the desktop application: {error}");
            ExitCode::FAILURE
        }
    }
}

#[derive(Default)]
struct DesktopApp {
    workspace: ArchiveWorkspace,
}

impl eframe::App for DesktopApp {
    fn ui(&mut self, ui: &mut eframe::egui::Ui, _frame: &mut eframe::Frame) {
        self.workspace.show(ui);
    }
}
