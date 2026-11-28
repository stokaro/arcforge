use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use arcforge_core::application_info;
use arcforge_ui_egui::ArchiveWorkspace;

#[cfg(feature = "native-capture")]
mod capture;
mod platform;

#[derive(Debug, Eq, PartialEq)]
enum Startup {
    Help,
    Version,
    Desktop(Option<PathBuf>),
    #[cfg(feature = "native-capture")]
    Capture(capture::Options),
}

fn parse_args(args: Vec<OsString>) -> Result<Startup, &'static str> {
    match args.as_slice() {
        #[cfg(feature = "native-capture")]
        [flag, directory, separator, archive]
            if flag == "--capture-native"
                && separator == "--"
                && !directory.is_empty()
                && !archive.is_empty() =>
        {
            Ok(Startup::Capture(capture::Options {
                directory: directory.into(),
                archive: archive.into(),
            }))
        }
        [] => Ok(Startup::Desktop(None)),
        [arg] if arg == "--help" || arg == "-h" => Ok(Startup::Help),
        [arg] if arg == "--version" || arg == "-V" => Ok(Startup::Version),
        [arg] if arg == "--" => Ok(Startup::Desktop(None)),
        [separator, path] if separator == "--" && !path.is_empty() => {
            Ok(Startup::Desktop(Some(PathBuf::from(path))))
        }
        [path] if path.is_empty() => Err("archive path must not be empty"),
        [path] if path.as_encoded_bytes().starts_with(b"-") => {
            Err("unknown option; use -- before a path beginning with '-'")
        }
        [path] => Ok(Startup::Desktop(Some(PathBuf::from(path)))),
        _ => Err("expected at most one archive path"),
    }
}

fn main() -> ExitCode {
    let startup = match parse_args(std::env::args_os().skip(1).collect()) {
        Ok(startup) => startup,
        Err(error) => {
            eprintln!("arcforge: {error}. See --help.");
            return ExitCode::from(2);
        }
    };
    let path = match startup {
        #[cfg(feature = "native-capture")]
        Startup::Capture(options) => return capture::run(options),
        Startup::Version => {
            let info = application_info();
            println!("{} {} ({})", info.name, info.version, info.stage);
            return ExitCode::SUCCESS;
        }
        Startup::Help => {
            println!(
                "Usage: arcforge [ARCHIVE]\n       arcforge -- PATH\n       arcforge --help | --version\n\nLaunch the ArcForge desktop with an optional ZIP path.\nZIP browsing, small-entry previews, and extraction into a new directory are available.\nEditing and saving are planned. Use arcforge-cli for command discovery.\nPaths may also be entered in the window or dropped into the workspace."
            );
            return ExitCode::SUCCESS;
        }
        Startup::Desktop(path) => path,
    };

    // Read a bounded set of system font files before the UI event loop starts.
    arcforge_ui_egui::typography::prepare();
    let options = platform::native_options();
    match eframe::run_native(
        "ArcForge",
        options,
        Box::new(move |_| {
            Ok(Box::new(DesktopApp {
                workspace: ArchiveWorkspace::new(path),
                ..Default::default()
            }))
        }),
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
    native_shell: platform::NativeShell,
    #[cfg(feature = "native-capture")]
    capture: Option<capture::Session>,
}

impl eframe::App for DesktopApp {
    fn ui(&mut self, ui: &mut eframe::egui::Ui, frame: &mut eframe::Frame) {
        #[cfg(feature = "native-capture")]
        if self
            .capture
            .as_ref()
            .is_some_and(capture::Session::finished)
        {
            return;
        }
        self.workspace.enable_filesystem_favorites(ui.ctx());
        self.native_shell
            .before_frame(&mut self.workspace, ui.ctx(), frame);
        self.workspace.show(ui);
        self.workspace.service_native_picker(frame, ui.ctx());
        self.native_shell.after_frame(&self.workspace);
        #[cfg(feature = "native-capture")]
        if let Some(capture) = &mut self.capture {
            let panel = self.native_shell.active_file_panel();
            capture.after_frame(
                &mut self.workspace,
                ui.ctx(),
                self.native_shell.window_id(),
                panel,
            );
            if let Some(panel_id) = capture.take_panel_to_cancel() {
                self.native_shell.cancel_file_panel(panel_id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn startup_arguments_are_strict_and_allow_one_native_path() {
        assert_eq!(parse_args(vec![]), Ok(Startup::Desktop(None)));
        assert_eq!(parse_args(arguments(&["--help"])), Ok(Startup::Help));
        assert_eq!(parse_args(arguments(&["-h"])), Ok(Startup::Help));
        assert_eq!(parse_args(arguments(&["--version"])), Ok(Startup::Version));
        assert_eq!(parse_args(arguments(&["-V"])), Ok(Startup::Version));
        assert_eq!(
            parse_args(arguments(&["test archive.zip"])),
            Ok(Startup::Desktop(Some("test archive.zip".into())))
        );
        assert_eq!(
            parse_args(arguments(&["--", "-archive.zip"])),
            Ok(Startup::Desktop(Some("-archive.zip".into())))
        );
        assert_eq!(parse_args(arguments(&["--"])), Ok(Startup::Desktop(None)));
        for invalid in [
            vec![""],
            vec!["--", ""],
            vec!["--unknown"],
            vec!["first.zip", "second.zip"],
            vec!["--help", "archive.zip"],
            vec!["--", "first.zip", "second.zip"],
        ] {
            assert!(parse_args(arguments(&invalid)).is_err(), "{invalid:?}");
        }
    }

    #[cfg(feature = "native-capture")]
    #[test]
    fn developer_capture_requires_explicit_output_and_one_native_archive_path() {
        assert_eq!(
            parse_args(arguments(&[
                "--capture-native",
                "new captures",
                "--",
                "архив.zip"
            ])),
            Ok(Startup::Capture(capture::Options {
                directory: "new captures".into(),
                archive: "архив.zip".into(),
            }))
        );
        for args in [
            vec!["--capture-native"],
            vec!["--capture-native", "", "--", "archive.zip"],
            vec!["--capture-native", "output", "--", ""],
            vec!["--capture-native", "output", "archive.zip"],
            vec!["--capture-native", "output", "--", "one.zip", "two.zip"],
        ] {
            assert!(parse_args(arguments(&args)).is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn startup_path_keeps_non_utf8_native_bytes() {
        use std::os::unix::ffi::OsStringExt;

        let path = OsString::from_vec(b"archive-\xff.zip".to_vec());
        assert_eq!(
            parse_args(vec![path.clone()]),
            Ok(Startup::Desktop(Some(PathBuf::from(path))))
        );
    }
}
