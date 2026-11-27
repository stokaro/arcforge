use std::io::{self, Write};
use std::process::ExitCode;

use arcforge_core::{Availability, Command, CommandError, CommandResult, execute};

const HELP: &str = "ArcForge command-line interface\n\
Usage: arcforge-cli <command> [--json]\n\n\
Implemented commands:\n\
  version       Show application version\n\
  commands      List commands and implementation status\n\
  doctor        Report compiled capabilities (does not probe the system)\n\n\
Options:\n\
  -h, --help    Show this help\n\
  -V, --version Show application version\n\
  --json        Structured output for version, commands, or doctor\n\n\
Archive operations are planned but not implemented. See `commands`.\n\
Exit codes: 0 introspection succeeded, 1 output error, 2 invalid usage,\n\
3 operation not implemented. Doctor's exit code does not imply archive readiness.\n";

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let output = run(&args);
    let result = if output.code == 0 {
        io::stdout().lock().write_all(output.text.as_bytes())
    } else {
        io::stderr().lock().write_all(output.text.as_bytes())
    };
    if let Err(error) = result {
        if error.kind() == io::ErrorKind::BrokenPipe {
            // Closing an output consumer must not turn a rejected command into
            // success. Successful stdout commands still exit successfully.
            return ExitCode::from(output.code);
        }
        eprintln!("arcforge-cli: output failed: {error}");
        return ExitCode::FAILURE;
    }
    ExitCode::from(output.code)
}

struct Output {
    code: u8,
    text: String,
}

fn run(args: &[std::ffi::OsString]) -> Output {
    let Some(first) = args.first() else {
        return Output {
            code: 0,
            text: HELP.to_owned(),
        };
    };
    let Some(name) = first.to_str() else {
        return usage_error("the command name must be valid UTF-8");
    };
    if matches!(name, "help" | "--help" | "-h") {
        return if args.len() == 1 {
            Output {
                code: 0,
                text: HELP.to_owned(),
            }
        } else {
            usage_error("help does not accept arguments")
        };
    }
    let name = if matches!(name, "--version" | "-V") {
        "version"
    } else {
        name
    };
    let command = match Command::parse(name) {
        Ok(command) => command,
        Err(error) => return usage_error(&error.to_string()),
    };
    // The bootstrap dispatch is read-only. Planned commands reject the request
    // before any filename arguments are interpreted or input files are touched.
    let result = match execute(command) {
        Ok(result) => result,
        Err(error) => return command_error(error),
    };
    let json = args.len() == 2 && args[1] == "--json";
    if args.len() > 1 && !json {
        return usage_error("expected no arguments or exactly --json");
    }
    let text = if json {
        match serde_json::to_string_pretty(&result) {
            Ok(text) => text,
            Err(error) => {
                return Output {
                    code: 1,
                    text: format!("arcforge-cli: serialization failed: {error}\n"),
                };
            }
        }
    } else {
        render_text(result)
    };
    Output {
        code: 0,
        text: format!("{text}\n"),
    }
}

fn render_text(result: CommandResult) -> String {
    match result {
        CommandResult::Version(info) => format!("{} {} ({})", info.name, info.version, info.stage),
        CommandResult::Commands(commands) => commands
            .iter()
            .map(|command| {
                let status = match command.availability {
                    Availability::Implemented => "implemented",
                    Availability::Planned => "planned",
                };
                format!(
                    "{:<10} {:<12} {}",
                    command.name, status, command.description
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
        CommandResult::Doctor(report) => format!(
            "ArcForge {} ({})\nReady for archive operations: {}\nArchive providers: {}\nSupported archive formats: none\n{}",
            report.application.version,
            report.application.stage,
            report.ready_for_archive_operations,
            report.archive_provider_count,
            report.scope
        ),
    }
}

fn usage_error(message: &str) -> Output {
    Output {
        code: 2,
        text: format!("arcforge-cli: {message}. See --help.\n"),
    }
}

fn command_error(error: CommandError) -> Output {
    let code = match error {
        CommandError::UnknownCommand(_) => 2,
        CommandError::NotImplemented(_) => 3,
    };
    Output {
        code,
        text: format!("arcforge-cli: {error}\n"),
    }
}
