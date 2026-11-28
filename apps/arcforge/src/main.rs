use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use arcforge_core::application_info;
use arcforge_ui_egui::ArchiveWorkspace;

#[derive(Debug, Eq, PartialEq)]
enum Startup {
    Help,
    Version,
    Desktop(Option<PathBuf>),
}

fn parse_args(args: Vec<OsString>) -> Result<Startup, &'static str> {
    match args.as_slice() {
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
        Startup::Version => {
            let info = application_info();
            println!("{} {} ({})", info.name, info.version, info.stage);
            return ExitCode::SUCCESS;
        }
        Startup::Help => {
            println!(
                "Usage: arcforge [ARCHIVE]\n       arcforge -- PATH\n       arcforge --help | --version\n\nLaunch the ArcForge desktop with an optional ZIP path.\nRead-only browsing and small-entry previews are available.\nEditing and extraction are planned. Use arcforge-cli for command discovery.\nPaths may also be entered in the window or dropped into the workspace."
            );
            return ExitCode::SUCCESS;
        }
        Startup::Desktop(path) => path,
    };

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
        Box::new(move |_| {
            Ok(Box::new(DesktopApp {
                workspace: ArchiveWorkspace::new(path),
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
}

impl eframe::App for DesktopApp {
    fn ui(&mut self, ui: &mut eframe::egui::Ui, _frame: &mut eframe::Frame) {
        self.workspace.show(ui);
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
