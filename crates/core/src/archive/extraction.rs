//! Validated, capability-scoped extraction into a new directory.
//!
//! Files are created exclusively and written in place. A failed or cancelled job
//! retains its new directory and reports the incomplete entry; it is not a
//! transaction and never recursively removes a path that another process may own.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

use cap_fs_ext::DirExt;
use cap_std::ambient_authority;
use cap_std::fs::{Dir, DirBuilder, OpenOptions};
use caseless::Caseless;
use same_file::Handle;
use serde::Serialize;
use unicode_normalization::UnicodeNormalization;

use super::{
    ArchiveEntry, ArchiveError, ArchiveSnapshot, CancellationToken, EntryId, EntryKind, NameState,
    StreamLimits, StreamProgress, begin_stream,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExtractionSelection {
    All,
    /// Explicit directory records include their currently interpreted descendants.
    Entries(Vec<EntryId>),
}

#[derive(Clone, Debug)]
pub struct ExtractionLimits {
    pub stream: StreamLimits,
    pub max_path_depth: usize,
    pub max_path_bytes: usize,
    /// Aggregate stored names and collision keys, excluding bounded object overhead.
    pub max_plan_bytes: usize,
    /// Includes implicit parent directories as well as explicit archive entries.
    pub max_output_paths: usize,
}

impl Default for ExtractionLimits {
    fn default() -> Self {
        Self {
            stream: StreamLimits::default(),
            max_path_depth: 64,
            max_path_bytes: 4096,
            max_plan_bytes: 64 * 1024 * 1024,
            max_output_paths: 100_000,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct PlannedExtractionEntry {
    pub entry_id: EntryId,
    /// Original resolved Unicode spelling; collision keys never rename this path.
    pub path: String,
    pub kind: EntryKind,
    pub size: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct ExtractionPlan {
    pub snapshot_id: String,
    pub metadata_revision: String,
    pub entries: Vec<PlannedExtractionEntry>,
    pub total_bytes: u64,
    pub total_files: u64,
    pub output_paths: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionPhase {
    Checking,
    Extracting,
    Finalizing,
}

#[derive(Clone, Debug, Serialize)]
pub struct ExtractionProgress {
    pub phase: ExtractionPhase,
    pub completed_entries: u64,
    pub total_entries: u64,
    pub bytes_written: u64,
    pub total_bytes: u64,
    pub current_entry: Option<EntryId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionStatus {
    Completed,
    Cancelled,
    Failed,
}

#[derive(Clone, Debug, Serialize)]
pub struct IncompleteExtractionEntry {
    pub entry_id: EntryId,
    pub path: String,
    pub bytes_written: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct ExtractionReport {
    /// Display path only; every filesystem operation retains native path objects.
    pub destination: String,
    pub destination_created: bool,
    pub status: ExtractionStatus,
    pub entries_planned: u64,
    pub completed_entries: Vec<EntryId>,
    pub files_written: u64,
    /// Directories below the destination root, including implicit parents.
    pub directories_created: u64,
    pub bytes_written: u64,
    pub incomplete: Option<IncompleteExtractionEntry>,
    pub error: Option<ArchiveError>,
}

/// Validate the complete selection without touching the destination filesystem.
/// A plan is descriptive; `extract_archive` always prepares its own fresh plan.
pub fn plan_extraction(
    snapshot: &ArchiveSnapshot,
    selection: &ExtractionSelection,
    limits: &ExtractionLimits,
    cancel: &CancellationToken,
) -> Result<ExtractionPlan, ArchiveError> {
    prepare_plan(snapshot, selection, limits, cancel, Instant::now())
}

fn check_job(
    started: Instant,
    limits: &ExtractionLimits,
    cancel: &CancellationToken,
) -> Result<(), ArchiveError> {
    cancel.check()?;
    if started.elapsed() >= limits.stream.operation_timeout {
        return Err(ArchiveError::LimitExceeded(
            "extraction operation time".into(),
        ));
    }
    Ok(())
}

fn selected_ids(
    snapshot: &ArchiveSnapshot,
    selection: &ExtractionSelection,
    limits: &ExtractionLimits,
    cancel: &CancellationToken,
    started: Instant,
) -> Result<BTreeSet<EntryId>, ArchiveError> {
    let mut ids = BTreeSet::new();
    match selection {
        ExtractionSelection::All => {
            for entry in &snapshot.entries {
                check_job(started, limits, cancel)?;
                ids.insert(entry.id);
            }
        }
        ExtractionSelection::Entries(requested) => {
            if requested.is_empty() {
                return Err(ArchiveError::Unsupported(
                    "no entries selected for extraction".into(),
                ));
            }
            if requested.len() as u64 > limits.stream.max_entries {
                return Err(ArchiveError::LimitExceeded("selected entry count".into()));
            }
            let mut prefixes = BTreeSet::new();
            for &id in requested {
                check_job(started, limits, cancel)?;
                let entry = entry_by_id(snapshot, id)?;
                ids.insert(id);
                if entry.kind == EntryKind::Directory {
                    prefixes.insert(format!("{}/", entry.name.display.trim_end_matches('/')));
                }
            }
            // Disjoint lexical prefix intervals avoid a selected-folders × entries scan.
            let mut disjoint = BTreeSet::<String>::new();
            for prefix in prefixes {
                check_job(started, limits, cancel)?;
                if disjoint
                    .last()
                    .is_none_or(|parent| !prefix.starts_with(parent))
                {
                    disjoint.insert(prefix);
                }
            }
            if !disjoint.is_empty() {
                for entry in &snapshot.entries {
                    check_job(started, limits, cancel)?;
                    if disjoint
                        .range(..=entry.name.display.clone())
                        .next_back()
                        .is_some_and(|prefix| entry.name.display.starts_with(prefix))
                    {
                        ids.insert(entry.id);
                    }
                }
            }
        }
    }
    if ids.len() as u64 > limits.stream.max_entries {
        return Err(ArchiveError::LimitExceeded("selected entry count".into()));
    }
    Ok(ids)
}

fn entry_by_id(snapshot: &ArchiveSnapshot, id: EntryId) -> Result<&ArchiveEntry, ArchiveError> {
    snapshot
        .entries
        .get(usize::try_from(id).map_err(|_| ArchiveError::EntryNotFound(id))?)
        .filter(|entry| entry.id == id)
        .ok_or(ArchiveError::EntryNotFound(id))
}

struct PathNode {
    spelling: String,
    directory: bool,
    owner: EntryId,
    explicit: bool,
}

fn prepare_plan(
    snapshot: &ArchiveSnapshot,
    selection: &ExtractionSelection,
    limits: &ExtractionLimits,
    cancel: &CancellationToken,
    started: Instant,
) -> Result<ExtractionPlan, ArchiveError> {
    check_job(started, limits, cancel)?;
    let ids = selected_ids(snapshot, selection, limits, cancel, started)?;
    let mut unresolved = Vec::new();
    for &id in &ids {
        check_job(started, limits, cancel)?;
        if entry_by_id(snapshot, id)?.name.state != NameState::Resolved {
            unresolved.push(id);
        }
    }
    if !unresolved.is_empty() {
        return Err(ArchiveError::FilenameEncodingRequired(unresolved));
    }
    let mut plan = ExtractionPlan {
        snapshot_id: snapshot.id.clone(),
        metadata_revision: snapshot.metadata_revision.clone(),
        entries: Vec::new(),
        total_bytes: 0,
        total_files: 0,
        output_paths: 0,
    };
    let mut nodes = BTreeMap::<String, PathNode>::new();
    let mut plan_bytes = 0usize;
    for id in ids {
        check_job(started, limits, cancel)?;
        let entry = entry_by_id(snapshot, id)?;
        if !matches!(entry.kind, EntryKind::File | EntryKind::Directory) {
            return Err(ArchiveError::Unsupported(format!(
                "entry {id}: links, special files and inconsistent file types cannot be extracted"
            )));
        }
        if entry.encrypted || !matches!(entry.compression_method, 0 | 8) {
            return Err(ArchiveError::Unsupported(format!(
                "entry {id}: extraction supports unencrypted Stored and Deflate entries"
            )));
        }
        if entry.kind == EntryKind::Directory && entry.size != 0 {
            return Err(ArchiveError::Unsupported(format!(
                "directory entry {id} has file data"
            )));
        }
        if entry.size > limits.stream.max_entry_bytes {
            return Err(ArchiveError::LimitExceeded(format!(
                "entry {id} output bytes"
            )));
        }
        plan.total_bytes = plan
            .total_bytes
            .checked_add(entry.size)
            .ok_or_else(|| ArchiveError::LimitExceeded("total extraction size overflow".into()))?;
        if plan.total_bytes > limits.stream.max_total_bytes {
            return Err(ArchiveError::LimitExceeded(
                "total extraction output bytes".into(),
            ));
        }
        let path = validated_path(entry, limits)?;
        charge_plan(&mut plan_bytes, path.len(), limits)?;
        let components: Vec<_> = path.split('/').collect();
        let mut prefix = String::new();
        for (index, component) in components.iter().enumerate() {
            check_job(started, limits, cancel)?;
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(component);
            let key: String = prefix.nfd().default_case_fold().nfd().collect();
            let last = index + 1 == components.len();
            let directory = !last || entry.kind == EntryKind::Directory;
            if let Some(node) = nodes.get_mut(&key) {
                if node.spelling != prefix
                    || !node.directory
                    || !directory
                    || (last && node.explicit)
                {
                    return Err(ArchiveError::PathCollision {
                        path: prefix,
                        entries: vec![node.owner, id],
                    });
                }
                node.explicit |= last;
            } else {
                if nodes.len() >= limits.max_output_paths {
                    return Err(ArchiveError::LimitExceeded("output path count".into()));
                }
                charge_plan(&mut plan_bytes, prefix.len() + key.len(), limits)?;
                nodes.insert(
                    key,
                    PathNode {
                        spelling: prefix.clone(),
                        directory,
                        owner: id,
                        explicit: last,
                    },
                );
            }
        }
        if entry.kind == EntryKind::File {
            plan.total_files += 1;
        }
        plan.entries.push(PlannedExtractionEntry {
            entry_id: id,
            path,
            kind: entry.kind,
            size: entry.size,
        });
    }
    plan.output_paths = nodes.len() as u64;
    Ok(plan)
}

fn charge_plan(
    bytes: &mut usize,
    added: usize,
    limits: &ExtractionLimits,
) -> Result<(), ArchiveError> {
    *bytes = bytes
        .checked_add(added)
        .ok_or_else(|| ArchiveError::LimitExceeded("extraction plan size".into()))?;
    if *bytes > limits.max_plan_bytes {
        return Err(ArchiveError::LimitExceeded("extraction plan bytes".into()));
    }
    Ok(())
}

fn validated_path(entry: &ArchiveEntry, limits: &ExtractionLimits) -> Result<String, ArchiveError> {
    let fail = |reason: &str| ArchiveError::UnsafeEntryPath {
        entry_id: entry.id,
        reason: reason.into(),
    };
    let name = &entry.name.display;
    let path = if entry.kind == EntryKind::Directory {
        name.strip_suffix('/').unwrap_or(name)
    } else {
        name
    };
    if path.is_empty() || path.len() > limits.max_path_bytes {
        return Err(fail(
            "empty path or path exceeds the extraction length limit",
        ));
    }
    if path.starts_with('/') || path.contains('\\') {
        return Err(fail(
            "absolute paths and backslashes are not portable relative archive paths",
        ));
    }
    if path
        .chars()
        .any(|c| c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
    {
        return Err(fail(
            "control characters, drive/stream syntax or reserved filename characters",
        ));
    }
    for (depth, component) in path.split('/').enumerate() {
        if depth >= limits.max_path_depth {
            return Err(fail("path nesting exceeds the extraction limit"));
        }
        if component.is_empty() || matches!(component, "." | "..") {
            return Err(fail(
                "empty, current-directory or parent-directory path component",
            ));
        }
        if component.len() > 255 || component.ends_with(['.', ' ']) {
            return Err(fail("overlong component or trailing dot/space alias"));
        }
        let base = component
            .split('.')
            .next()
            .unwrap_or(component)
            .trim_end()
            .to_uppercase();
        let numbered_device = ["COM", "LPT"].iter().any(|prefix| {
            base.strip_prefix(prefix).is_some_and(|suffix| {
                matches!(
                    suffix,
                    "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        });
        if numbered_device
            || matches!(
                base.as_str(),
                "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$" | "CONIN$" | "CONOUT$"
            )
        {
            return Err(fail("reserved device name"));
        }
    }
    Ok(path.to_owned())
}

/// Extract into one new directory. `Err` means no destination was created by
/// this invocation. Once creation succeeds, every normal outcome is a report,
/// including failure/cancellation with retained completed and partial files.
pub fn extract_archive(
    snapshot: &ArchiveSnapshot,
    selection: &ExtractionSelection,
    destination: &Path,
    limits: &ExtractionLimits,
    cancel: &CancellationToken,
    progress: &mut dyn FnMut(ExtractionProgress),
) -> Result<ExtractionReport, ArchiveError> {
    #[cfg(test)]
    {
        extract_impl(
            snapshot,
            selection,
            destination,
            limits,
            cancel,
            progress,
            None,
        )
    }
    #[cfg(not(test))]
    {
        extract_impl(snapshot, selection, destination, limits, cancel, progress)
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TestStage {
    ParentOpened,
    DestinationOpened,
    BeforeEntry(EntryId),
}
#[cfg(test)]
type TestHook<'a> = &'a dyn Fn(TestStage);

fn extract_impl(
    snapshot: &ArchiveSnapshot,
    selection: &ExtractionSelection,
    destination: &Path,
    limits: &ExtractionLimits,
    cancel: &CancellationToken,
    progress: &mut dyn FnMut(ExtractionProgress),
    #[cfg(test)] hook: Option<TestHook<'_>>,
) -> Result<ExtractionReport, ArchiveError> {
    let started = Instant::now();
    progress(ExtractionProgress {
        phase: ExtractionPhase::Checking,
        completed_entries: 0,
        total_entries: 0,
        bytes_written: 0,
        total_bytes: 0,
        current_entry: None,
    });
    let plan = prepare_plan(snapshot, selection, limits, cancel, started)?;
    let target = DestinationParent::open(destination)?;
    // A changed archive fails before destination creation, even for an empty ZIP.
    // Planning and destination validation already consumed part of this job's
    // deadline. Do not give a single long entry a second full timeout window.
    let mut stream_limits = limits.stream.clone();
    stream_limits.operation_timeout = stream_limits
        .operation_timeout
        .saturating_sub(started.elapsed());
    let mut stream = begin_stream(snapshot, &stream_limits, cancel)?;
    check_job(started, limits, cancel)?;
    #[cfg(test)]
    if let Some(hook) = hook {
        hook(TestStage::ParentOpened);
    }
    target.verify_parent()?;
    target.create_new()?;
    let mut report = ExtractionReport {
        destination: target.path.to_string_lossy().into_owned(),
        destination_created: true,
        status: ExtractionStatus::Completed,
        entries_planned: plan.entries.len() as u64,
        completed_entries: Vec::new(),
        files_written: 0,
        directories_created: 0,
        bytes_written: 0,
        incomplete: None,
        error: None,
    };
    let operation = (|| -> Result<(), ArchiveError> {
        let root = target.parent.open_dir_nofollow(&target.leaf)?;
        let root_identity = directory_identity(&root)?;
        #[cfg(test)]
        if let Some(hook) = hook {
            hook(TestStage::DestinationOpened);
        }
        for entry in &plan.entries {
            check_job(started, limits, cancel)?;
            target.verify_root(&root_identity)?;
            #[cfg(test)]
            if let Some(hook) = hook {
                hook(TestStage::BeforeEntry(entry.entry_id));
            }
            publish_progress(
                progress,
                &plan,
                &report,
                ExtractionPhase::Extracting,
                Some(entry.entry_id),
            );
            check_job(started, limits, cancel)?;
            if entry.kind == EntryKind::Directory {
                {
                    let mut update = |value: StreamProgress| {
                        update_progress(progress, &plan, &mut report, value)
                    };
                    stream.copy_entry(entry.entry_id, &mut io::sink(), &mut update)?;
                }
                open_directories(
                    &root,
                    entry.path.split('/'),
                    &mut report.directories_created,
                )?;
            } else {
                let (parent, name) = entry
                    .path
                    .rsplit_once('/')
                    .unwrap_or(("", entry.path.as_str()));
                let directory = open_directories(
                    &root,
                    parent.split('/').filter(|p| !p.is_empty()),
                    &mut report.directories_created,
                )?;
                let mut options = OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use cap_std::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                let mut file = directory.open_with(name, &options)?;
                report.incomplete = Some(IncompleteExtractionEntry {
                    entry_id: entry.entry_id,
                    path: entry.path.clone(),
                    bytes_written: 0,
                });
                {
                    let mut update = |value: StreamProgress| {
                        update_progress(progress, &plan, &mut report, value)
                    };
                    stream.copy_entry(entry.entry_id, &mut file, &mut update)?;
                }
                file.sync_all()?;
                report.files_written += 1;
            }
            report.completed_entries.push(entry.entry_id);
            report.incomplete = None;
            publish_progress(progress, &plan, &report, ExtractionPhase::Extracting, None);
        }
        check_job(started, limits, cancel)?;
        publish_progress(progress, &plan, &report, ExtractionPhase::Finalizing, None);
        stream.finish()?;
        target.verify_root(&root_identity)?;
        Ok(())
    })();
    if let Err(error) = operation {
        report.status = if error == ArchiveError::Cancelled {
            ExtractionStatus::Cancelled
        } else {
            ExtractionStatus::Failed
        };
        report.error = Some(error);
    }
    Ok(report)
}

fn update_progress(
    progress: &mut dyn FnMut(ExtractionProgress),
    plan: &ExtractionPlan,
    report: &mut ExtractionReport,
    value: StreamProgress,
) {
    report.bytes_written = value.total_bytes_written;
    if let Some(incomplete) = &mut report.incomplete {
        incomplete.bytes_written = value.entry_bytes_written;
    }
    publish_progress(
        progress,
        plan,
        report,
        ExtractionPhase::Extracting,
        Some(value.entry_id),
    );
}

fn publish_progress(
    progress: &mut dyn FnMut(ExtractionProgress),
    plan: &ExtractionPlan,
    report: &ExtractionReport,
    phase: ExtractionPhase,
    current_entry: Option<EntryId>,
) {
    progress(ExtractionProgress {
        phase,
        completed_entries: report.completed_entries.len() as u64,
        total_entries: plan.entries.len() as u64,
        bytes_written: report.bytes_written,
        total_bytes: plan.total_bytes,
        current_entry,
    });
}

struct DestinationParent {
    path: PathBuf,
    parent_path: PathBuf,
    leaf: OsString,
    parent: Dir,
    parent_identity: Handle,
}

impl DestinationParent {
    fn open(destination: &Path) -> Result<Self, ArchiveError> {
        let invalid = || {
            ArchiveError::InvalidDestination(
                "choose a new directory under an existing parent".into(),
            )
        };
        if destination.as_os_str().is_empty() {
            return Err(invalid());
        }
        let path = if destination.is_absolute() {
            destination.to_owned()
        } else {
            std::env::current_dir()?.join(destination)
        };
        let leaf = path.file_name().ok_or_else(invalid)?.to_owned();
        let parent_path = path.parent().ok_or_else(invalid)?.to_owned();
        let parent = Dir::open_ambient_dir(&parent_path, ambient_authority())?;
        let parent_identity = directory_identity(&parent)?;
        Ok(Self {
            path,
            parent_path,
            leaf,
            parent,
            parent_identity,
        })
    }

    fn create_new(&self) -> Result<(), ArchiveError> {
        let builder = private_directory_builder();
        self.parent
            .create_dir_with(&self.leaf, &builder)
            .map_err(|error| {
                if error.kind() == io::ErrorKind::AlreadyExists {
                    ArchiveError::DestinationExists(self.path.to_string_lossy().into_owned())
                } else {
                    ArchiveError::from(error)
                }
            })
    }

    fn verify_parent(&self) -> Result<(), ArchiveError> {
        let current = Dir::open_ambient_dir(&self.parent_path, ambient_authority())
            .map_err(|_| ArchiveError::DestinationChanged)?;
        if directory_identity(&current)? != self.parent_identity {
            return Err(ArchiveError::DestinationChanged);
        }
        Ok(())
    }

    fn verify_root(&self, identity: &Handle) -> Result<(), ArchiveError> {
        self.verify_parent()?;
        let current = self
            .parent
            .open_dir_nofollow(&self.leaf)
            .map_err(|_| ArchiveError::DestinationChanged)?;
        if directory_identity(&current)? != *identity {
            return Err(ArchiveError::DestinationChanged);
        }
        Ok(())
    }
}

fn directory_identity(directory: &Dir) -> Result<Handle, ArchiveError> {
    Ok(Handle::from_file(directory.try_clone()?.into_std_file())?)
}

fn private_directory_builder() -> DirBuilder {
    let mut builder = DirBuilder::new();
    #[cfg(unix)]
    {
        use cap_std::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    // `recursive(false)` is intentional: every component gets a no-follow open.
    builder.recursive(false);
    builder
}

fn open_directories<'a>(
    root: &Dir,
    components: impl Iterator<Item = &'a str>,
    created: &mut u64,
) -> Result<Dir, ArchiveError> {
    let mut directory = root.try_clone()?;
    for component in components {
        match directory.open_dir_nofollow(component) {
            Ok(child) => directory = child,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                directory.create_dir_with(component, &private_directory_builder())?;
                *created += 1;
                directory = directory.open_dir_nofollow(component)?;
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(directory)
}

#[cfg(test)]
#[path = "extraction_tests.rs"]
mod tests;
