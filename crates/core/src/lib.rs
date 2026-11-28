//! Shared application commands and truthful capabilities, independent of the UI.

pub mod archive;

use std::fmt;

use serde::Serialize;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const PROJECT_URL: &str = "https://github.com/stokaro/arcforge";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    Implemented,
    Planned,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct CommandDescriptor {
    pub name: &'static str,
    pub description: &'static str,
    pub availability: Availability,
}

pub const COMMANDS: &[CommandDescriptor] = &[
    CommandDescriptor {
        name: "version",
        description: "Show application version and development stage",
        availability: Availability::Implemented,
    },
    CommandDescriptor {
        name: "commands",
        description: "List commands and their implementation status",
        availability: Availability::Implemented,
    },
    CommandDescriptor {
        name: "doctor",
        description: "Report compiled application capabilities (no system probes)",
        availability: Availability::Implemented,
    },
    CommandDescriptor {
        name: "list",
        description: "List ZIP central-directory entries",
        availability: Availability::Implemented,
    },
    CommandDescriptor {
        name: "info",
        description: "Inspect ZIP metadata and filename diagnostics",
        availability: Availability::Implemented,
    },
    CommandDescriptor {
        name: "preview",
        description: "Preview a complete small Stored/Deflate ZIP entry",
        availability: Availability::Implemented,
    },
    CommandDescriptor {
        name: "create",
        description: "Create an archive",
        availability: Availability::Planned,
    },
    CommandDescriptor {
        name: "extract",
        description: "Extract selected Stored/Deflate ZIP entries into a new directory",
        availability: Availability::Implemented,
    },
    CommandDescriptor {
        name: "delete",
        description: "Delete selected entries by saving a verified classic ZIP copy",
        availability: Availability::Implemented,
    },
    CommandDescriptor {
        name: "test",
        description: "Verify archive integrity",
        availability: Availability::Planned,
    },
    CommandDescriptor {
        name: "update",
        description: "Apply changes to an archive",
        availability: Availability::Planned,
    },
    CommandDescriptor {
        name: "convert",
        description: "Convert an archive to another format",
        availability: Availability::Planned,
    },
    CommandDescriptor {
        name: "hash",
        description: "Compute or verify file hashes",
        availability: Availability::Planned,
    },
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArchiveOperation {
    List,
    Info,
    Preview,
    Create,
    Extract,
    Delete,
    Test,
    Update,
    Convert,
    Hash,
}

impl ArchiveOperation {
    pub const fn name(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Info => "info",
            Self::Preview => "preview",
            Self::Create => "create",
            Self::Extract => "extract",
            Self::Delete => "delete",
            Self::Test => "test",
            Self::Update => "update",
            Self::Convert => "convert",
            Self::Hash => "hash",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Command {
    Version,
    Commands,
    Doctor,
    Archive(ArchiveOperation),
}

impl Command {
    pub fn parse(name: &str) -> Result<Self, CommandError> {
        match name {
            "version" => Ok(Self::Version),
            "commands" => Ok(Self::Commands),
            "doctor" => Ok(Self::Doctor),
            "list" => Ok(Self::Archive(ArchiveOperation::List)),
            "info" => Ok(Self::Archive(ArchiveOperation::Info)),
            "preview" => Ok(Self::Archive(ArchiveOperation::Preview)),
            "create" => Ok(Self::Archive(ArchiveOperation::Create)),
            "extract" => Ok(Self::Archive(ArchiveOperation::Extract)),
            "delete" => Ok(Self::Archive(ArchiveOperation::Delete)),
            "test" => Ok(Self::Archive(ArchiveOperation::Test)),
            "update" => Ok(Self::Archive(ArchiveOperation::Update)),
            "convert" => Ok(Self::Archive(ArchiveOperation::Convert)),
            "hash" => Ok(Self::Archive(ArchiveOperation::Hash)),
            other => Err(CommandError::UnknownCommand(other.to_owned())),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct ApplicationInfo {
    pub name: &'static str,
    pub version: &'static str,
    pub stage: &'static str,
    pub license: &'static str,
}

pub const fn application_info() -> ApplicationInfo {
    ApplicationInfo {
        name: "ArcForge",
        version: VERSION,
        stage: "zip-staged-delete",
        license: "MIT",
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct ProviderCapabilities {
    pub name: &'static str,
    pub version: &'static str,
    pub formats: &'static [&'static str],
    pub operations: &'static [&'static str],
    pub preview_compression: &'static [&'static str],
    pub extraction_compression: &'static [&'static str],
    pub encrypted_content: bool,
    pub multi_disk: bool,
    /// Whether the source archive can be created or modified.
    pub write: bool,
}

pub const PROVIDERS: &[ProviderCapabilities] = &[ProviderCapabilities {
    name: "zip-rs",
    version: "9.8.2",
    formats: &["zip", "zip64"],
    operations: &["list", "info", "preview", "extract", "delete"],
    preview_compression: &["stored", "deflate"],
    extraction_compression: &["stored", "deflate"],
    encrypted_content: false,
    multi_disk: false,
    write: false,
}];

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct DiagnosticReport {
    pub application: ApplicationInfo,
    pub ready_for_archive_operations: bool,
    pub archive_provider_count: usize,
    pub supported_archive_formats: &'static [&'static str],
    pub providers: &'static [ProviderCapabilities],
    pub os: &'static str,
    pub architecture: &'static str,
    pub scope: &'static str,
}

pub const fn diagnostics() -> DiagnosticReport {
    DiagnosticReport {
        application: application_info(),
        ready_for_archive_operations: true,
        archive_provider_count: PROVIDERS.len(),
        supported_archive_formats: &["zip", "zip64"],
        providers: PROVIDERS,
        os: std::env::consts::OS,
        architecture: std::env::consts::ARCH,
        scope: "ZIP listing, metadata, bounded previews and streamed extraction into a new directory. Delete saves a verified classic ZIP copy to a new filename. Source archives remain unchanged. Extraction requires resolved safe paths and rejects links, special files, collisions and encrypted content. Compiled capabilities only; the system was not probed.",
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum CommandResult {
    Version(ApplicationInfo),
    Commands(&'static [CommandDescriptor]),
    Doctor(DiagnosticReport),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommandError {
    UnknownCommand(String),
    ArgumentsRequired(&'static str),
    NotImplemented(&'static str),
}

impl fmt::Display for CommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownCommand(name) => write!(formatter, "unknown command '{name}'"),
            Self::ArgumentsRequired(name) => {
                write!(formatter, "'{name}' requires archive arguments")
            }
            Self::NotImplemented(name) => write!(
                formatter,
                "'{name}' is planned but not implemented; no archive operation was performed"
            ),
        }
    }
}

impl std::error::Error for CommandError {}

/// Dispatch a command through the same contract for every frontend.
pub fn execute(command: Command) -> Result<CommandResult, CommandError> {
    match command {
        Command::Version => Ok(CommandResult::Version(application_info())),
        Command::Commands => Ok(CommandResult::Commands(COMMANDS)),
        Command::Doctor => Ok(CommandResult::Doctor(diagnostics())),
        Command::Archive(
            operation @ (ArchiveOperation::List
            | ArchiveOperation::Info
            | ArchiveOperation::Preview
            | ArchiveOperation::Extract
            | ArchiveOperation::Delete),
        ) => Err(CommandError::ArgumentsRequired(operation.name())),
        Command::Archive(operation) => Err(CommandError::NotImplemented(operation.name())),
    }
}

#[cfg(test)]
mod tests {
    use super::{Availability, COMMANDS, Command, CommandError, diagnostics, execute};

    #[test]
    fn discovery_matches_real_dispatch() {
        let mut names = std::collections::BTreeSet::new();
        for descriptor in COMMANDS {
            assert!(names.insert(descriptor.name), "duplicate command");
            let result = execute(Command::parse(descriptor.name).expect("known command"));
            match descriptor.availability {
                Availability::Implemented => assert!(
                    result.is_ok()
                        || result == Err(CommandError::ArgumentsRequired(descriptor.name))
                ),
                Availability::Planned => {
                    assert_eq!(result, Err(CommandError::NotImplemented(descriptor.name)))
                }
            }
        }
    }

    #[test]
    fn unknown_commands_are_not_treated_as_success() {
        assert_eq!(
            Command::parse("invented-operation"),
            Err(CommandError::UnknownCommand(
                "invented-operation".to_owned()
            ))
        );
    }

    #[test]
    fn discovery_advertises_extraction_without_archive_modification() {
        let report = diagnostics();
        assert!(report.ready_for_archive_operations);
        assert_eq!(report.archive_provider_count, 1);
        assert_eq!(report.supported_archive_formats, ["zip", "zip64"]);
        assert_eq!(
            report.providers[0].operations,
            ["list", "info", "preview", "extract", "delete"]
        );
        assert_eq!(
            report.providers[0].extraction_compression,
            ["stored", "deflate"]
        );
        assert!(!report.providers[0].write);
        assert!(!report.providers[0].encrypted_content);
    }
}
