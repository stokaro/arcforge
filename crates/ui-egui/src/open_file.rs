use std::path::{Path, PathBuf};

use arcforge_core::archive::{
    ArchiveError, ArchiveSnapshot, CancellationToken, EntryId, EntryKind, ExtractionLimits,
    ExtractionSelection, ExtractionStatus, extract_archive, plan_extraction,
};

pub(super) struct OpenCopy {
    _directory: tempfile::TempDir,
    pub path: PathBuf,
}

impl OpenCopy {
    pub fn prepare(
        snapshot: &ArchiveSnapshot,
        entry: EntryId,
        cancel: &CancellationToken,
    ) -> Result<Self, ArchiveError> {
        let selection = ExtractionSelection::ExactEntries(vec![entry]);
        let limits = ExtractionLimits::default();
        let plan = plan_extraction(snapshot, &selection, &limits, cancel)?;
        let [planned] = plan.entries.as_slice() else {
            return Err(ArchiveError::EntryNotFound(entry));
        };
        if planned.kind != EntryKind::File {
            return Err(ArchiveError::Unsupported(
                "only regular files can be opened in another app".into(),
            ));
        }
        let relative_path = Path::new(&planned.path);
        if opens_as_program(relative_path) {
            return Err(ArchiveError::Unsupported(
                "this file type can run as a program or shortcut. Use the preview or extract the file explicitly".into(),
            ));
        }

        let mut temporary = tempfile::Builder::new();
        temporary.prefix("arcforge-open-");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            temporary.permissions(std::fs::Permissions::from_mode(0o700));
        }
        let directory = temporary.tempdir()?;
        let destination = std::path::absolute(directory.path())?.join("entry");
        let report = extract_archive(
            snapshot,
            &selection,
            &destination,
            &limits,
            cancel,
            &mut |_| {},
        )?;
        match report.status {
            ExtractionStatus::Completed => Ok(Self {
                path: destination.join(relative_path),
                _directory: directory,
            }),
            ExtractionStatus::Cancelled => Err(ArchiveError::Cancelled),
            ExtractionStatus::Failed => Err(report
                .error
                .unwrap_or_else(|| ArchiveError::Io("temporary extraction failed".into()))),
        }
    }
}

fn opens_as_program(path: &Path) -> bool {
    let extension = path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.rsplit_once('.').map(|(_, extension)| extension))
        .unwrap_or_default()
        .to_ascii_lowercase();
    // These formats can start a program, interpreter, installer or shell action
    // through a file association. Extraction still supports their regular bytes.
    matches!(
        extension.as_str(),
        "app"
            | "application"
            | "appref-ms"
            | "bat"
            | "cmd"
            | "com"
            | "command"
            | "cpl"
            | "desktop"
            | "exe"
            | "hta"
            | "inetloc"
            | "jar"
            | "js"
            | "jse"
            | "lnk"
            | "msc"
            | "msi"
            | "pif"
            | "ps1"
            | "py"
            | "pyw"
            | "reg"
            | "scf"
            | "scr"
            | "sh"
            | "terminal"
            | "url"
            | "vbe"
            | "vbs"
            | "webloc"
            | "website"
            | "wsf"
            | "wsh"
    )
}

pub(super) fn handoff(path: &Path) -> Result<(), ArchiveError> {
    dispatch(path).map_err(|error| ArchiveError::Io(format!("default app: {error}")))
}

#[cfg(target_os = "windows")]
fn dispatch(path: &Path) -> std::io::Result<()> {
    open::that_detached(path)
}

#[cfg(not(target_os = "windows"))]
fn dispatch(path: &Path) -> std::io::Result<()> {
    use std::io;
    use std::process::{Command, Stdio};

    let launcher = if cfg!(target_os = "macos") {
        "/usr/bin/open"
    } else {
        "gio"
    };
    let mut command = Command::new(launcher);
    if !cfg!(target_os = "macos") {
        command.arg("open");
    }
    let status = command
        .arg("--")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| io::Error::new(error.kind(), format!("{launcher}: {error}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "{launcher} returned {status}. Check the file association"
        )))
    }
}
