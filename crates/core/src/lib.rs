//! Shared application commands, independent of a UI, filesystem, or archive provider.
//!
//! This bootstrap deliberately reports no archive capabilities. Frontends must not
//! turn a planned command into a successful operation or invent archive metadata.

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
        description: "List archive entries",
        availability: Availability::Planned,
    },
    CommandDescriptor {
        name: "info",
        description: "Inspect archive metadata",
        availability: Availability::Planned,
    },
    CommandDescriptor {
        name: "create",
        description: "Create an archive",
        availability: Availability::Planned,
    },
    CommandDescriptor {
        name: "extract",
        description: "Extract archive entries",
        availability: Availability::Planned,
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
    Create,
    Extract,
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
            Self::Create => "create",
            Self::Extract => "extract",
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
            "create" => Ok(Self::Archive(ArchiveOperation::Create)),
            "extract" => Ok(Self::Archive(ArchiveOperation::Extract)),
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
        stage: "bootstrap",
        license: "MIT",
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct DiagnosticReport {
    pub application: ApplicationInfo,
    pub ready_for_archive_operations: bool,
    pub archive_provider_count: usize,
    pub supported_archive_formats: &'static [&'static str],
    pub scope: &'static str,
}

pub const fn diagnostics() -> DiagnosticReport {
    DiagnosticReport {
        application: application_info(),
        ready_for_archive_operations: false,
        archive_provider_count: 0,
        supported_archive_formats: &[],
        scope: "Compiled capabilities only; the filesystem, graphics, and external tools were not probed.",
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
    NotImplemented(&'static str),
}

impl fmt::Display for CommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownCommand(name) => write!(formatter, "unknown command '{name}'"),
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
                Availability::Implemented => assert!(result.is_ok()),
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
    fn bootstrap_does_not_advertise_archive_support() {
        let report = diagnostics();
        assert!(!report.ready_for_archive_operations);
        assert_eq!(report.archive_provider_count, 0);
        assert!(report.supported_archive_formats.is_empty());
    }
}
