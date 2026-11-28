//! One bounded worker for archive I/O and prepared views.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex};

use arcforge_core::archive::{
    ArchiveEntry, ArchiveError, ArchiveLimits, ArchiveSnapshot, CancellationToken, EntryId,
    EntryPreview, ExtractionLimits, ExtractionProgress, ExtractionReport, ExtractionSelection,
    FilenamePolicy, StreamLimits, ZipEditReport, extract_archive, open_archive, preview_entry,
    reinterpret, save_zip_without_entries, supports_zip_removal,
};

use super::edits::{EditAction, RemovalHistory};

const MAX_VIEW_BYTES: usize = 64 * 1024 * 1024;
const MAX_FOLDER_BYTES: usize = 8 * 1024 * 1024;
const MAX_FOLDERS: usize = 10_000;
const MAX_TEXT_ROWS: usize = 16_384;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Ticket {
    pub generation: u64,
    pub request: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum ViewMode {
    Flat,
    #[default]
    Outline,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ViewRow {
    pub entry: Option<usize>,
    pub folder: Option<usize>,
    pub depth: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct FolderTotals {
    pub size: u64,
    pub compressed_size: u64,
    pub entries: u64,
}

impl FolderTotals {
    fn add(&mut self, other: Self) -> Result<(), ArchiveError> {
        let overflow = || ArchiveError::LimitExceeded("prepared folder totals overflow".into());
        self.size = self.size.checked_add(other.size).ok_or_else(overflow)?;
        self.compressed_size = self
            .compressed_size
            .checked_add(other.compressed_size)
            .ok_or_else(overflow)?;
        self.entries = self
            .entries
            .checked_add(other.entries)
            .ok_or_else(overflow)?;
        Ok(())
    }
}

pub(super) struct PreparedDocument {
    pub snapshot: Arc<ArchiveSnapshot>,
    pub edits: RemovalHistory,
    pub editable: bool,
    pub totals: FolderTotals,
    pub folders: Vec<String>,
    pub folder_totals: Vec<FolderTotals>,
    pub folders_limited: bool,
    search_keys: Vec<String>,
    outline: Vec<ViewRow>,
}

impl PreparedDocument {
    pub fn new(
        snapshot: ArchiveSnapshot,
        cancel: &CancellationToken,
    ) -> Result<Self, ArchiveError> {
        Self::with_edits(Arc::new(snapshot), RemovalHistory::default(), cancel)
    }

    fn with_edits(
        snapshot: Arc<ArchiveSnapshot>,
        edits: RemovalHistory,
        cancel: &CancellationToken,
    ) -> Result<Self, ArchiveError> {
        let mut folders = BTreeSet::new();
        let mut folder_bytes = 0;
        let mut key_bytes = 0usize;
        let mut folders_limited = false;
        let mut search_keys = Vec::with_capacity(snapshot.entries.len());
        let mut totals = FolderTotals::default();
        for entry in &snapshot.entries {
            check_cancel(cancel)?;
            if edits.removed.contains(&entry.id) {
                search_keys.push(String::new());
                continue;
            }
            totals.add(FolderTotals {
                entries: 1,
                size: entry.size,
                compressed_size: entry.compressed_size,
            })?;
            let name = &entry.name.display;
            let key = name.to_lowercase();
            key_bytes = key_bytes.checked_add(key.len()).ok_or_else(view_limit)?;
            if key_bytes > MAX_VIEW_BYTES {
                return Err(view_limit());
            }
            search_keys.push(key);
            for (offset, character) in name.char_indices() {
                if character != '/' || offset == 0 {
                    continue;
                }
                let folder = &name[..=offset];
                if folders.contains(folder) {
                    continue;
                }
                if folders.len() == MAX_FOLDERS || folder.len() > MAX_FOLDER_BYTES - folder_bytes {
                    folders_limited = true;
                    break;
                }
                folder_bytes += folder.len();
                folders.insert(folder.to_owned());
            }
        }
        let folders: Vec<_> = folders.into_iter().collect();
        // One accumulator per cached folder (at most MAX_FOLDERS), rather
        // than storing a descendant list or walking every ancestor per file.
        let mut folder_totals = vec![FolderTotals::default(); folders.len()];
        for entry in &snapshot.entries {
            check_cancel(cancel)?;
            if !edits.removed.contains(&entry.id)
                && !entry.is_directory
                && let Some(folder) = nearest_folder(&folders, &entry.name.display, cancel)?
            {
                folder_totals[folder].add(FolderTotals {
                    size: entry.size,
                    compressed_size: entry.compressed_size,
                    entries: 1,
                })?;
            }
        }
        // Lexically sorted parents precede their children. Propagate each
        // accumulated subtree once, including implicitly represented folders.
        for index in (0..folders.len()).rev() {
            check_cancel(cancel)?;
            if let Some(parent) = nearest_folder(
                &folders,
                &folders[index][..folders[index].len() - 1],
                cancel,
            )? {
                let subtree = folder_totals[index];
                folder_totals[parent].add(subtree)?;
            }
        }
        let mut outline: Vec<_> = folders
            .iter()
            .enumerate()
            .map(|(index, folder)| ViewRow {
                entry: None,
                folder: Some(index),
                depth: folder
                    .trim_end_matches('/')
                    .bytes()
                    .filter(|byte| *byte == b'/')
                    .count(),
            })
            .collect();
        for (index, entry) in snapshot.entries.iter().enumerate() {
            check_cancel(cancel)?;
            if edits.removed.contains(&entry.id) {
                continue;
            }
            if entry.is_directory
                && let Ok(folder) = folders.binary_search(&entry.name.display)
                && outline[folder].entry.is_none()
            {
                outline[folder].entry = Some(index);
                continue;
            }
            outline.push(ViewRow {
                entry: Some(index),
                folder: None,
                depth: entry
                    .name
                    .display
                    .trim_end_matches('/')
                    .bytes()
                    .filter(|byte| *byte == b'/')
                    .count(),
            });
        }
        outline.sort_by(|left, right| {
            let path = |row: &ViewRow| match row.folder {
                Some(folder) => folders[folder].as_str(),
                None => snapshot.entries[row.entry.expect("entry row")]
                    .name
                    .display
                    .as_str(),
            };
            path(left)
                .cmp(path(right))
                .then_with(|| right.folder.is_some().cmp(&left.folder.is_some()))
        });
        Ok(Self {
            editable: supports_zip_removal(&snapshot),
            snapshot,
            edits,
            totals,
            folders,
            folder_totals,
            folders_limited,
            search_keys,
            outline,
        })
    }

    pub fn entry(&self, id: EntryId) -> Option<&ArchiveEntry> {
        // Provider identities are central-directory ordinals; still verify the match.
        if self.edits.removed.contains(&id) {
            return None;
        }
        self.snapshot
            .entries
            .get(usize::try_from(id).ok()?)
            .filter(|entry| entry.id == id)
    }

    pub fn filter(
        &self,
        query: &str,
        folder: Option<&str>,
        cancel: &CancellationToken,
    ) -> Result<Vec<usize>, ArchiveError> {
        let query = query.to_lowercase();
        let mut rows = Vec::new();
        for (index, entry) in self.snapshot.entries.iter().enumerate() {
            check_cancel(cancel)?;
            if !self.edits.removed.contains(&entry.id)
                && folder.is_none_or(|prefix| entry.name.display.starts_with(prefix))
                && self.search_keys[index].contains(&query)
            {
                rows.push(index);
            }
        }
        Ok(rows)
    }

    fn expand_selection(
        &self,
        mut ids: BTreeSet<EntryId>,
        mut folders: BTreeSet<String>,
        cancel: &CancellationToken,
    ) -> Result<BTreeSet<EntryId>, ArchiveError> {
        for &id in &ids {
            check_cancel(cancel)?;
            let entry = self.entry(id).ok_or(ArchiveError::EntryNotFound(id))?;
            if entry.is_directory {
                folders.insert(format!("{}/", entry.name.display.trim_end_matches('/')));
            }
        }
        if !folders.is_empty() {
            for entry in &self.snapshot.entries {
                check_cancel(cancel)?;
                if !self.edits.removed.contains(&entry.id)
                    && entry
                        .name
                        .display
                        .match_indices('/')
                        .any(|(offset, _)| folders.contains(&entry.name.display[..=offset]))
                {
                    ids.insert(entry.id);
                }
            }
        }
        Ok(ids)
    }

    pub fn view_rows(
        &self,
        query: &str,
        folder: Option<&str>,
        mode: ViewMode,
        expanded: &BTreeSet<String>,
        cancel: &CancellationToken,
    ) -> Result<Vec<ViewRow>, ArchiveError> {
        let matches = self.filter(query, folder, cancel)?;
        if mode == ViewMode::Flat {
            return Ok(matches
                .into_iter()
                .map(|entry| ViewRow {
                    entry: Some(entry),
                    folder: None,
                    depth: 0,
                })
                .collect());
        }
        let mut entries = vec![false; self.snapshot.entries.len()];
        let mut folders = vec![false; self.folders.len()];
        for index in matches {
            entries[index] = true;
            let name = &self.snapshot.entries[index].name.display;
            for (offset, character) in name.char_indices() {
                if character == '/'
                    && let Ok(folder) = self
                        .folders
                        .binary_search_by(|candidate| candidate.as_str().cmp(&name[..=offset]))
                {
                    folders[folder] = true;
                }
            }
        }
        let base_depth = folder.map_or(0, |path| path.bytes().filter(|byte| *byte == b'/').count());
        let mut visible = Vec::new();
        for row in &self.outline {
            check_cancel(cancel)?;
            let path = match row.folder {
                Some(index) if folders[index] => self.folders[index].as_str(),
                Some(_) => continue,
                None if row.entry.is_some_and(|entry| entries[entry]) => self.snapshot.entries
                    [row.entry.expect("entry row")]
                .name
                .display
                .as_str(),
                None => continue,
            };
            if folder.is_some_and(|prefix| !path.starts_with(prefix) || path == prefix) {
                continue;
            }
            let collapsed = query.is_empty()
                && path.char_indices().any(|(offset, character)| {
                    if character != '/' || offset + 1 == path.len() {
                        return false;
                    }
                    let prefix = &path[..=offset];
                    if folder.is_some_and(|base| prefix.len() <= base.len()) {
                        return false;
                    }
                    self.folders
                        .binary_search_by(|candidate| candidate.as_str().cmp(prefix))
                        .is_ok()
                        && !expanded.contains(prefix)
                });
            if !collapsed {
                visible.push(ViewRow {
                    depth: row.depth.saturating_sub(base_depth),
                    ..*row
                });
            }
        }
        Ok(visible)
    }
}

fn nearest_folder(
    folders: &[String],
    name: &str,
    cancel: &CancellationToken,
) -> Result<Option<usize>, ArchiveError> {
    for (offset, _) in name.rmatch_indices('/') {
        check_cancel(cancel)?;
        if let Ok(index) = folders.binary_search_by(|folder| folder.as_str().cmp(&name[..=offset]))
        {
            return Ok(Some(index));
        }
    }
    Ok(None)
}

fn view_limit() -> ArchiveError {
    ArchiveError::LimitExceeded("prepared filename search data exceeds 64 MiB".into())
}

fn check_cancel(cancel: &CancellationToken) -> Result<(), ArchiveError> {
    if cancel.is_cancelled() {
        Err(ArchiveError::Cancelled)
    } else {
        Ok(())
    }
}

pub(super) struct PreparedPreview {
    pub preview: EntryPreview,
    pub text_rows: Option<Vec<String>>,
    pub text_limited: bool,
    pub hex_rows: Vec<String>,
    pub image: Option<egui::ColorImage>,
    pub image_error: Option<String>,
}

impl PreparedPreview {
    pub(super) fn new(
        preview: EntryPreview,
        cancel: &CancellationToken,
    ) -> Result<Self, ArchiveError> {
        check_cancel(cancel)?;
        let (image, image_error) = match super::preview_image::decode(&preview.bytes) {
            Some(Ok(image)) => (Some(image), None),
            Some(Err(error)) => (None, Some(error)),
            None => (None, None),
        };
        // Pixel decoding is bounded but not preemptible. Discard a cancelled
        // result before preparing or delivering any UI presentation.
        check_cancel(cancel)?;
        let mut text_limited = false;
        let text_rows = if let Some(text) = &preview.text {
            let mut rows = Vec::new();
            let mut row = String::new();
            let mut columns = 0;
            for character in text.chars() {
                check_cancel(cancel)?;
                if character == '\n' || columns == 200 {
                    rows.push(std::mem::take(&mut row));
                    columns = 0;
                    if rows.len() == MAX_TEXT_ROWS {
                        text_limited = true;
                        break;
                    }
                }
                if character != '\n' {
                    row.push(character);
                    columns += 1;
                }
            }
            if !text_limited {
                rows.push(row);
            }
            Some(rows)
        } else {
            None
        };
        let mut hex_rows = Vec::new();
        for (index, bytes) in preview.bytes.chunks(16).enumerate() {
            check_cancel(cancel)?;
            let hex = bytes
                .iter()
                .map(|byte| format!("{byte:02x} "))
                .collect::<String>();
            let ascii = bytes
                .iter()
                .map(|byte| {
                    if (0x20..=0x7e).contains(byte) {
                        char::from(*byte)
                    } else {
                        '.'
                    }
                })
                .collect::<String>();
            hex_rows.push(format!("{:08x}  {hex:48} {ascii}", index * 16));
        }
        Ok(Self {
            preview,
            text_rows,
            text_limited,
            hex_rows,
            image,
            image_error,
        })
    }
}

pub(super) enum Task {
    Open {
        path: PathBuf,
        policy: FilenamePolicy,
    },
    Reinterpret {
        document: Arc<PreparedDocument>,
        policy: FilenamePolicy,
        query: String,
        mode: ViewMode,
    },
    Filter {
        document: Arc<PreparedDocument>,
        query: String,
        folder: Option<String>,
        mode: ViewMode,
        expanded: BTreeSet<String>,
    },
    Preview {
        document: Arc<PreparedDocument>,
        entry: EntryId,
    },
    OpenFile {
        document: Arc<PreparedDocument>,
        entry: EntryId,
    },
    Edit {
        document: Arc<PreparedDocument>,
        action: EditAction,
        query: String,
        folder: Option<String>,
        mode: ViewMode,
        expanded: BTreeSet<String>,
    },
    Save {
        document: Arc<PreparedDocument>,
        destination: PathBuf,
    },
    Extract {
        document: Arc<PreparedDocument>,
        selection: ExtractionSelection,
        folders: BTreeSet<String>,
        destination: PathBuf,
    },
}

pub(super) struct Request {
    pub ticket: Ticket,
    pub cancel: CancellationToken,
    pub task: Task,
}

pub(super) enum Completed {
    Document {
        document: Arc<PreparedDocument>,
        rows: Vec<ViewRow>,
        query: String,
        mode: ViewMode,
    },
    Filter {
        rows: Vec<ViewRow>,
        query: String,
        folder: Option<String>,
        mode: ViewMode,
        expanded: BTreeSet<String>,
    },
    Preview(PreparedPreview),
    OpenedFile(super::open_file::OpenCopy),
    Extraction(ExtractionReport),
    Edited {
        document: Arc<PreparedDocument>,
        rows: Vec<ViewRow>,
        folder: Option<String>,
    },
    Saved {
        report: ZipEditReport,
        view: Result<(Arc<PreparedDocument>, Vec<ViewRow>), ArchiveError>,
    },
}

pub(super) struct Reply {
    pub ticket: Ticket,
    pub result: Result<Completed, ArchiveError>,
}

pub(super) struct ProgressReply {
    pub ticket: Ticket,
    pub progress: ExtractionProgress,
}

pub(super) struct Worker {
    requests: SyncSender<Request>,
    replies: Receiver<Reply>,
    pending: Option<Request>,
    cancellation: Option<CancellationToken>,
    // Filesystem outcomes must be delivered even when cancellation is requested.
    exclusive: Option<Ticket>,
    progress: Arc<Mutex<Option<ProgressReply>>>,
}

impl Worker {
    #[cfg(test)]
    pub(super) fn test_channels() -> (Self, Receiver<Request>, SyncSender<Reply>) {
        let (requests, incoming) = mpsc::sync_channel(1);
        let (outgoing, replies) = mpsc::sync_channel(1);
        (
            Self {
                requests,
                replies,
                pending: None,
                cancellation: None,
                exclusive: None,
                progress: Arc::default(),
            },
            incoming,
            outgoing,
        )
    }

    pub fn new(context: egui::Context) -> std::io::Result<Self> {
        let (requests, incoming) = mpsc::sync_channel::<Request>(1);
        let (outgoing, replies) = mpsc::sync_channel(1);
        let progress = Arc::<Mutex<Option<ProgressReply>>>::default();
        let worker_progress = progress.clone();
        let thread = std::thread::Builder::new()
            .name("arcforge-archive".into())
            .spawn(move || {
                while let Ok(request) = incoming.recv() {
                    let result = execute(&request, &mut |progress| {
                        if let Ok(mut latest) = worker_progress.lock() {
                            *latest = Some(ProgressReply {
                                ticket: request.ticket,
                                progress,
                            });
                        }
                        context.request_repaint();
                    });
                    if outgoing
                        .send(Reply {
                            ticket: request.ticket,
                            result,
                        })
                        .is_err()
                    {
                        break;
                    }
                    context.request_repaint();
                }
            })?;
        // Closing the channels and cancelling stops the worker without blocking a UI frame.
        drop(thread);
        Ok(Self {
            requests,
            replies,
            pending: None,
            cancellation: None,
            exclusive: None,
            progress,
        })
    }

    pub fn submit(&mut self, request: Request) -> Result<(), ArchiveError> {
        if self.exclusive.is_some() {
            return Err(ArchiveError::Unsupported(
                "wait for the current file operation before starting another operation".into(),
            ));
        }
        self.cancel();
        if matches!(
            &request.task,
            Task::Extract { .. } | Task::Save { .. } | Task::OpenFile { .. }
        ) {
            self.exclusive = Some(request.ticket);
        }
        self.cancellation = Some(request.cancel.clone());
        match self.requests.try_send(request) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(request)) => {
                self.pending = Some(request);
                Ok(())
            }
            Err(TrySendError::Disconnected(_)) => {
                self.exclusive = None;
                Err(ArchiveError::Io("archive worker stopped".into()))
            }
        }
    }

    pub fn poll(&mut self) -> Result<Vec<Reply>, ArchiveError> {
        let mut replies = Vec::new();
        loop {
            match self.replies.try_recv() {
                Ok(reply) => {
                    if self.exclusive == Some(reply.ticket) {
                        self.exclusive = None;
                    }
                    replies.push(reply);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    return Err(ArchiveError::Io("archive worker stopped".into()));
                }
            }
        }
        if let Some(request) = self.pending.take() {
            match self.requests.try_send(request) {
                Ok(()) => {}
                Err(TrySendError::Full(request)) => self.pending = Some(request),
                Err(TrySendError::Disconnected(_)) => {
                    return Err(ArchiveError::Io("archive worker stopped".into()));
                }
            }
        }
        Ok(replies)
    }

    pub fn cancel(&mut self) {
        if let Some(cancel) = self.cancellation.take() {
            cancel.cancel();
        }
        if let Some(pending) = &self.pending {
            pending.cancel.cancel();
        }
        // A cancelled file operation still needs its terminal result.
        if self.exclusive.is_none() {
            self.pending = None;
        }
    }

    pub fn take_progress(&self) -> Option<ProgressReply> {
        self.progress.try_lock().ok()?.take()
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel();
    }
}

fn execute(
    request: &Request,
    progress: &mut dyn FnMut(ExtractionProgress),
) -> Result<Completed, ArchiveError> {
    check_cancel(&request.cancel)?;
    let limits = ArchiveLimits::default();
    let result = match &request.task {
        Task::Open { path, policy } => {
            let snapshot = open_archive(path, policy, &limits, &request.cancel)?;
            prepare(
                Arc::new(snapshot),
                RemovalHistory::default(),
                "",
                ViewMode::Outline,
                &request.cancel,
            )
        }
        Task::Reinterpret {
            document,
            policy,
            query,
            mode,
        } => {
            let snapshot = reinterpret(&document.snapshot, policy, &request.cancel)?;
            prepare(
                Arc::new(snapshot),
                document.edits.clone(),
                query,
                *mode,
                &request.cancel,
            )
        }
        Task::Filter {
            document,
            query,
            folder,
            mode,
            expanded,
        } => {
            let rows =
                document.view_rows(query, folder.as_deref(), *mode, expanded, &request.cancel)?;
            Ok(Completed::Filter {
                rows,
                query: query.clone(),
                folder: folder.clone(),
                mode: *mode,
                expanded: expanded.clone(),
            })
        }
        Task::Preview { document, entry } => {
            let preview = preview_entry(&document.snapshot, *entry, &limits, &request.cancel)?;
            PreparedPreview::new(preview, &request.cancel).map(Completed::Preview)
        }
        Task::OpenFile { document, entry } => {
            if document.entry(*entry).is_none() {
                return Err(ArchiveError::EntryNotFound(*entry));
            }
            let copy =
                super::open_file::OpenCopy::prepare(&document.snapshot, *entry, &request.cancel)?;
            check_cancel(&request.cancel)?;
            super::open_file::handoff(&copy.path)?;
            // The OS accepted the handoff. A late cancellation must keep its copy.
            return Ok(Completed::OpenedFile(copy));
        }
        Task::Edit {
            document,
            action,
            query,
            folder,
            mode,
            expanded,
        } => {
            let mut edits = document.edits.clone();
            match action {
                EditAction::Delete { ids, folders } => edits.delete(document.expand_selection(
                    ids.clone(),
                    folders.clone(),
                    &request.cancel,
                )?),
                EditAction::Undo => edits.undo(),
                EditAction::Redo => edits.redo(),
            }
            let updated = Arc::new(PreparedDocument::with_edits(
                document.snapshot.clone(),
                edits,
                &request.cancel,
            )?);
            let mut folder = folder.clone();
            while let Some(current) = &folder {
                if updated.folders.binary_search(current).is_ok() {
                    break;
                }
                folder = current
                    .trim_end_matches('/')
                    .rfind('/')
                    .map(|end| current[..=end].into());
            }
            let rows =
                updated.view_rows(query, folder.as_deref(), *mode, expanded, &request.cancel)?;
            Ok(Completed::Edited {
                document: updated,
                rows,
                folder,
            })
        }
        Task::Save {
            document,
            destination,
        } => {
            let saved = save_zip_without_entries(
                &document.snapshot,
                &document.edits.removed,
                destination,
                &StreamLimits::default(),
                &request.cancel,
            )?;
            // Publication won the race against any late cancel. Always report
            // the committed copy, even if preparing its display should fail.
            let view = (|| {
                let cancel = CancellationToken::new();
                let updated = Arc::new(PreparedDocument::new(saved.snapshot, &cancel)?);
                let rows =
                    updated.view_rows("", None, ViewMode::Outline, &BTreeSet::new(), &cancel)?;
                Ok((updated, rows))
            })();
            return Ok(Completed::Saved {
                report: saved.report,
                view,
            });
        }
        Task::Extract {
            document,
            selection,
            folders,
            destination,
        } => {
            let selection = match selection {
                ExtractionSelection::Entries(ids) => {
                    let expanded = document.expand_selection(
                        ids.iter().copied().collect(),
                        folders.clone(),
                        &request.cancel,
                    )?;
                    ExtractionSelection::ExactEntries(expanded.into_iter().collect())
                }
                ExtractionSelection::All if document.edits.is_dirty() => {
                    let mut retained = Vec::new();
                    for entry in &document.snapshot.entries {
                        check_cancel(&request.cancel)?;
                        if !document.edits.removed.contains(&entry.id) {
                            retained.push(entry.id);
                        }
                    }
                    ExtractionSelection::ExactEntries(retained)
                }
                _ => selection.clone(),
            };
            // Preserve the actual filesystem outcome if cancellation races completion.
            return extract_archive(
                &document.snapshot,
                &selection,
                destination,
                &ExtractionLimits::default(),
                &request.cancel,
                progress,
            )
            .map(Completed::Extraction);
        }
    }?;
    check_cancel(&request.cancel)?;
    Ok(result)
}

fn prepare(
    snapshot: Arc<ArchiveSnapshot>,
    edits: RemovalHistory,
    query: &str,
    mode: ViewMode,
    cancel: &CancellationToken,
) -> Result<Completed, ArchiveError> {
    let document = Arc::new(PreparedDocument::with_edits(snapshot, edits, cancel)?);
    let rows = document.view_rows(query, None, mode, &BTreeSet::new(), cancel)?;
    Ok(Completed::Document {
        document,
        rows,
        query: query.into(),
        mode,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(number: u64) -> Request {
        Request {
            ticket: Ticket {
                generation: 1,
                request: number,
            },
            cancel: CancellationToken::new(),
            task: Task::Open {
                path: "not-opened.zip".into(),
                policy: FilenamePolicy::default(),
            },
        }
    }

    #[test]
    fn queue_keeps_only_one_queued_and_one_latest_pending_request() {
        let (requests, incoming) = mpsc::sync_channel(1);
        let (_outgoing, replies) = mpsc::sync_channel(1);
        let mut worker = Worker {
            requests,
            replies,
            pending: None,
            cancellation: None,
            exclusive: None,
            progress: Arc::default(),
        };
        let first = request(1);
        let first_cancel = first.cancel.clone();
        worker.submit(first).expect("first request");
        let second = request(2);
        let second_cancel = second.cancel.clone();
        worker.submit(second).expect("pending second request");
        worker.submit(request(3)).expect("replace pending request");
        assert!(first_cancel.is_cancelled());
        assert!(second_cancel.is_cancelled());
        assert_eq!(
            incoming
                .try_recv()
                .expect("one queued request")
                .ticket
                .request,
            1
        );
        assert!(incoming.try_recv().is_err());
        worker.poll().expect("flush latest pending");
        let newest = incoming.try_recv().expect("latest request");
        assert_eq!(newest.ticket.request, 3);
        assert!(!newest.cancel.is_cancelled());
        worker.cancel();
        assert!(newest.cancel.is_cancelled());
        assert!(worker.pending.is_none());
    }

    #[test]
    fn text_view_is_bounded_while_hex_keeps_the_complete_checked_bytes() {
        let bytes = vec![b'\n'; MAX_TEXT_ROWS + 5];
        let preview = EntryPreview {
            entry_id: 0,
            text: Some(String::from_utf8(bytes.clone()).expect("UTF-8 fixture")),
            data_hex: "0a".repeat(bytes.len()),
            bytes,
            crc32_verified: true,
        };
        let cancel = CancellationToken::new();
        let prepared = PreparedPreview::new(preview.clone(), &cancel).expect("prepared preview");
        assert!(prepared.text_limited);
        assert_eq!(
            prepared.text_rows.as_ref().expect("text rows").len(),
            MAX_TEXT_ROWS
        );
        assert_eq!(prepared.hex_rows.len(), preview.bytes.len().div_ceil(16));
        assert_eq!(prepared.preview.bytes, preview.bytes);
        cancel.cancel();
        assert!(matches!(
            PreparedPreview::new(preview, &cancel),
            Err(ArchiveError::Cancelled)
        ));
    }
}
