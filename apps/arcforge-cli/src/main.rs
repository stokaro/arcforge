use std::ffi::OsString;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use arcforge_core::archive::{
    ArchiveError, ArchiveLimits, ArchiveSnapshot, CancellationToken, FilenameChoice,
    FilenameEncoding, FilenamePolicy, NameState, escape_display, open_archive, preview_entry,
};
use arcforge_core::{
    ArchiveOperation, Availability, Command, CommandError, CommandResult, execute,
};

const HELP: &str = "ArcForge command-line interface

Usage: arcforge-cli <command> [arguments]

  version                    Show application version
  commands                   List commands and implementation status
  doctor                     Report compiled capabilities (no system probes)
  list ZIP                   List every ZIP central-directory entry
  info ZIP                   Inspect ZIP metadata and filename diagnostics
  preview ZIP --entry ID     Read a small Stored/Deflate entry and verify its CRC

Options:
  -h, --help                 Show this help
  -V, --version              Show application version
  --json                     Structured output (archive failures go to stderr)
  --filename-encoding NAME   auto, utf-8, cp437, cp866, windows-1251, windows-1252,
                             koi8-r, koi8-u, shift-jis, gbk, big5, euc-kr
  --filename-encoding-mode MODE  fallback (default) or force (explicit recovery)
  --entry-encoding ID=NAME   Override an entry's fallback; requires --metadata-revision
  --recover-entry ID=NAME    Override an entry's declarations; requires --metadata-revision
  --metadata-revision SHA256  Require the metadata revision reported by list/info
  --max-bytes BYTES          Preview limit: 1..1048576 (default 262144)
  --                         End options; use before an option-like archive path

Entry IDs are central-directory ordinals scoped to a snapshot, never filenames.
Metadata revisions bind retained directory bytes; they are not whole-file hashes.
Unmarked non-ASCII names use a visible provisional CP437 fallback. Filename choices
do not change file contents or the archive. Preview is complete and CRC-checked;
oversized entries, encrypted content and other compression methods are rejected.
Extraction, editing and other planned operations remain unavailable.

Exit codes: 0 success, 1 output error, 2 invalid usage, 3 operation not implemented,
4 archive inspection/preview failed. No interactive prompts or stdin reads.
";

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

fn run(args: &[OsString]) -> Output {
    let Some(first) = args.first() else {
        return Output {
            code: 0,
            text: HELP.into(),
        };
    };
    let Some(name) = first.to_str() else {
        return usage_error("the command name must be valid UTF-8");
    };
    if matches!(name, "help" | "--help" | "-h") {
        return if args.len() == 1 {
            Output {
                code: 0,
                text: HELP.into(),
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
    if let Command::Archive(
        operation @ (ArchiveOperation::List | ArchiveOperation::Info | ArchiveOperation::Preview),
    ) = command
    {
        return match archive_options(operation, &args[1..]) {
            Ok(options) => archive_operation(operation, options),
            Err(error) => usage_error(&error),
        };
    }
    // Planned commands reject before interpreting or opening any input paths.
    let result = match execute(command) {
        Ok(result) => result,
        Err(error) => return command_error(error),
    };
    let json = match &args[1..] {
        [] => false,
        [option] if option == "--json" => true,
        _ => return usage_error("this command accepts only the optional --json flag"),
    };
    if json {
        json_output(&result)
    } else {
        Output {
            code: 0,
            text: format!("{}\n", render_text(result)),
        }
    }
}

struct ArchiveOptions {
    path: PathBuf,
    json: bool,
    policy: FilenamePolicy,
    entry: Option<u64>,
    metadata_revision: Option<String>,
    limits: ArchiveLimits,
}

fn archive_options(
    operation: ArchiveOperation,
    args: &[OsString],
) -> Result<ArchiveOptions, String> {
    let mut path = None;
    let mut json = false;
    let mut policy = FilenamePolicy::default();
    let mut entry = None;
    let mut metadata_revision = None;
    let mut limits = ArchiveLimits::default();
    let mut seen = std::collections::BTreeSet::new();
    let mut positional = false;
    let mut args = args.iter();
    while let Some(argument) = args.next() {
        if !positional && argument == "--" {
            positional = true;
            continue;
        }
        let option = argument.to_str();
        if !positional && option.is_some_and(|value| value.starts_with('-')) {
            let option = option.expect("checked UTF-8 option");
            if !matches!(option, "--entry-encoding" | "--recover-entry") && !seen.insert(option) {
                return Err(format!("{option} may be supplied only once"));
            }
            if option == "--json" {
                json = true;
                continue;
            }
            if !matches!(
                option,
                "--filename-encoding"
                    | "--filename-encoding-mode"
                    | "--entry-encoding"
                    | "--recover-entry"
                    | "--entry"
                    | "--metadata-revision"
                    | "--max-bytes"
            ) {
                return Err(format!(
                    "unrecognized argument {option:?}; use -- before an option-like archive path"
                ));
            }
            let value = args
                .next()
                .and_then(|value| value.to_str())
                .ok_or_else(|| format!("{option} requires a UTF-8 value"))?;
            match option {
                "--filename-encoding" => policy.archive.encoding = encoding(value, true)?,
                "--filename-encoding-mode" => {
                    policy.archive.recovery = match value {
                        "fallback" => false,
                        "force" => true,
                        _ => return Err("filename encoding mode must be fallback or force".into()),
                    }
                }
                "--entry-encoding" | "--recover-entry" => {
                    let (id, label) = value
                        .split_once('=')
                        .ok_or("entry encoding must be ID=NAME")?;
                    let id = id
                        .parse::<u64>()
                        .map_err(|_| "entry ID must be an unsigned integer")?;
                    let recovery = option == "--recover-entry";
                    let choice = FilenameChoice {
                        encoding: encoding(label, !recovery)?,
                        recovery,
                    };
                    if policy.entries.insert(id, choice).is_some() {
                        return Err(format!("entry {id} has more than one filename override"));
                    }
                }
                "--entry" => {
                    entry = Some(
                        value
                            .parse::<u64>()
                            .map_err(|_| "entry ID must be an unsigned integer")?,
                    )
                }
                "--metadata-revision" => {
                    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                        return Err("metadata revision must be the 64-character hexadecimal token reported by list/info".into());
                    }
                    metadata_revision = Some(value.to_ascii_lowercase());
                }
                "--max-bytes" => {
                    let maximum = value
                        .parse::<u64>()
                        .map_err(|_| "preview limit must be an unsigned integer")?;
                    if !(1..=1024 * 1024).contains(&maximum) {
                        return Err("preview limit must be between 1 and 1048576 bytes".into());
                    }
                    limits.max_preview_bytes = maximum;
                }
                _ => unreachable!("validated option"),
            }
        } else if path.replace(PathBuf::from(argument)).is_some() {
            return Err("expected exactly one archive path".into());
        }
    }
    let path = path
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or("an archive path is required")?;
    if matches!(operation, ArchiveOperation::Preview) {
        if entry.is_none() {
            return Err("preview requires --entry ID".into());
        }
    } else if entry.is_some() || seen.contains("--max-bytes") {
        return Err("--entry and --max-bytes are preview options".into());
    }
    policy.validate()?;
    if !policy.entries.is_empty() && metadata_revision.is_none() {
        return Err(
            "per-entry filename overrides require --metadata-revision from list/info".into(),
        );
    }
    Ok(ArchiveOptions {
        path,
        json,
        policy,
        entry,
        metadata_revision,
        limits,
    })
}

fn encoding(label: &str, allow_auto: bool) -> Result<Option<FilenameEncoding>, String> {
    if allow_auto && label.eq_ignore_ascii_case("auto") {
        return Ok(None);
    }
    FilenameEncoding::parse(label).map(Some).ok_or_else(|| {
        format!("unsupported filename encoding {label:?}; see --help for exact supported mappings")
    })
}

fn archive_operation(operation: ArchiveOperation, options: ArchiveOptions) -> Output {
    let cancel = CancellationToken::new();
    let snapshot = match open_archive(&options.path, &options.policy, &options.limits, &cancel) {
        Ok(snapshot) => snapshot,
        Err(error) => return archive_error(error, options.json),
    };
    if options
        .metadata_revision
        .as_ref()
        .is_some_and(|expected| expected != &snapshot.metadata_revision)
    {
        return archive_error(ArchiveError::SourceChanged, options.json);
    }
    match operation {
        ArchiveOperation::List if options.json => {
            json_output(&serde_json::json!({"kind":"list","data":snapshot}))
        }
        ArchiveOperation::List => {
            let mut text = format!(
                "Snapshot: {}\nMetadata revision: {}\nID\tSize\tPacked\tMethod\tName status\tName\n",
                snapshot.id, snapshot.metadata_revision
            );
            for entry in &snapshot.entries {
                use std::fmt::Write;
                let _ = writeln!(
                    text,
                    "{}\t{}\t{}\t{}\t{:?}\t{}",
                    entry.id,
                    entry.size,
                    entry.compressed_size,
                    entry.compression_method,
                    entry.name.state,
                    escape_display(&entry.name.display)
                );
            }
            Output { code: 0, text }
        }
        ArchiveOperation::Info => {
            let provisional = snapshot
                .entries
                .iter()
                .filter(|entry| entry.name.state == NameState::Provisional)
                .count();
            let unresolved = snapshot
                .entries
                .iter()
                .filter(|entry| {
                    matches!(entry.name.state, NameState::Conflict | NameState::Invalid)
                })
                .count();
            let encrypted = snapshot
                .entries
                .iter()
                .filter(|entry| entry.encrypted)
                .count();
            if options.json {
                json_output(&serde_json::json!({"kind":"info","data":{
                    "snapshot_id":snapshot.id,"metadata_revision":snapshot.metadata_revision,"source":snapshot.source,"format":"zip",
                    "entry_count":snapshot.entries.len(),"compressed_size":snapshot.compressed_size,"uncompressed_size":snapshot.uncompressed_size,
                    "provisional_names":provisional,"conflicting_or_invalid_names":unresolved,"encrypted_entries":encrypted,
                    "comment_hex":snapshot.comment_hex,"warnings":snapshot.warnings,"filename_policy":snapshot.policy,"providers":arcforge_core::PROVIDERS,
                }}))
            } else {
                Output {
                    code: 0,
                    text: format!(
                        "Archive: {}\nSnapshot: {}\nMetadata revision: {}\nFormat: ZIP\nEntries: {}\nUncompressed bytes: {}\nCompressed entry bytes: {}\nProvisional names: {provisional}\nConflicting or invalid names: {unresolved}\nEncrypted entries: {encrypted}\nRead-only: yes\n",
                        escape_display(&snapshot.source),
                        snapshot.id,
                        snapshot.metadata_revision,
                        snapshot.entries.len(),
                        snapshot.uncompressed_size,
                        snapshot.compressed_size
                    ),
                }
            }
        }
        ArchiveOperation::Preview => preview_output(
            &snapshot,
            options.entry.expect("validated preview entry"),
            &options.limits,
            &cancel,
            options.json,
        ),
        _ => command_error(CommandError::NotImplemented(operation.name())),
    }
}

fn preview_output(
    snapshot: &ArchiveSnapshot,
    entry: u64,
    limits: &ArchiveLimits,
    cancel: &CancellationToken,
    json: bool,
) -> Output {
    match preview_entry(snapshot, entry, limits, cancel) {
        Ok(preview) if json => json_output(
            &serde_json::json!({"kind":"preview","data":{"snapshot_id":snapshot.id,"metadata_revision":snapshot.metadata_revision,"preview":preview}}),
        ),
        Ok(preview) => {
            let rendered = if let Some(text) = preview.text {
                text.split('\n')
                    .map(escape_display)
                    .collect::<Vec<_>>()
                    .join("\n")
            } else {
                format!("Hex: {}", preview.data_hex)
            };
            Output {
                code: 0,
                text: format!(
                    "Snapshot: {}\nMetadata revision: {}\nEntry: {entry}\nBytes: {}\nCRC32 verified: {}\n{rendered}\n",
                    snapshot.id,
                    snapshot.metadata_revision,
                    preview.bytes.len(),
                    preview.crc32_verified
                ),
            }
        }
        Err(error) => archive_error(error, json),
    }
}

fn json_output(value: &impl serde::Serialize) -> Output {
    match serde_json::to_string_pretty(value) {
        Ok(text) => Output {
            code: 0,
            text: format!("{text}\n"),
        },
        Err(error) => Output {
            code: 1,
            text: format!("arcforge-cli: serialization failed: {error}\n"),
        },
    }
}

fn archive_error(error: ArchiveError, json: bool) -> Output {
    let text = if json {
        format!(
            "{}\n",
            serde_json::json!({"kind":"error","data":{"exit_code":4,"error":error}})
        )
    } else {
        format!("arcforge-cli: {}\n", escape_display(&error.to_string()))
    };
    Output { code: 4, text }
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
            "ArcForge {} ({})\nReady for archive operations: {}\nArchive providers: {}\nSupported archive formats: {}\n{}",
            report.application.version,
            report.application.stage,
            report.ready_for_archive_operations,
            report.archive_provider_count,
            report.supported_archive_formats.join(", "),
            report.scope
        ),
    }
}

fn usage_error(message: &str) -> Output {
    Output {
        code: 2,
        text: format!("arcforge-cli: {}. See --help.\n", escape_display(message)),
    }
}

fn command_error(error: CommandError) -> Output {
    let code = match error {
        CommandError::UnknownCommand(_) | CommandError::ArgumentsRequired(_) => 2,
        CommandError::NotImplemented(_) => 3,
    };
    Output {
        code,
        text: format!("arcforge-cli: {}\n", escape_display(&error.to_string())),
    }
}
