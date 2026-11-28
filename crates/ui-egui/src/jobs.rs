//! One bounded worker for archive I/O and prepared views.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};

use arcforge_core::archive::{
    ArchiveEntry, ArchiveError, ArchiveLimits, ArchiveSnapshot, CancellationToken, EntryId,
    EntryPreview, FilenamePolicy, open_archive, preview_entry, reinterpret,
};

const MAX_VIEW_BYTES: usize = 64 * 1024 * 1024;
const MAX_FOLDER_BYTES: usize = 8 * 1024 * 1024;
const MAX_FOLDERS: usize = 10_000;
const MAX_TEXT_ROWS: usize = 16_384;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Ticket {
    pub generation: u64,
    pub request: u64,
}

pub(super) struct PreparedDocument {
    pub snapshot: Arc<ArchiveSnapshot>,
    pub folders: Vec<String>,
    pub folders_limited: bool,
    search_keys: Vec<String>,
}

impl PreparedDocument {
    pub fn new(
        snapshot: ArchiveSnapshot,
        cancel: &CancellationToken,
    ) -> Result<Self, ArchiveError> {
        let mut folders = BTreeSet::new();
        let mut folder_bytes = 0;
        let mut key_bytes = 0usize;
        let mut folders_limited = false;
        let mut search_keys = Vec::with_capacity(snapshot.entries.len());
        for entry in &snapshot.entries {
            check_cancel(cancel)?;
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
        Ok(Self {
            snapshot: Arc::new(snapshot),
            folders: folders.into_iter().collect(),
            folders_limited,
            search_keys,
        })
    }

    pub fn entry(&self, id: EntryId) -> Option<&ArchiveEntry> {
        // Provider identities are central-directory ordinals; still verify the match.
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
            if folder.is_none_or(|prefix| entry.name.display.starts_with(prefix))
                && self.search_keys[index].contains(&query)
            {
                rows.push(index);
            }
        }
        Ok(rows)
    }
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
}

impl PreparedPreview {
    pub(super) fn new(
        preview: EntryPreview,
        cancel: &CancellationToken,
    ) -> Result<Self, ArchiveError> {
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
    },
    Filter {
        document: Arc<PreparedDocument>,
        query: String,
        folder: Option<String>,
    },
    Preview {
        document: Arc<PreparedDocument>,
        entry: EntryId,
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
        rows: Vec<usize>,
        query: String,
    },
    Filter {
        rows: Vec<usize>,
        query: String,
        folder: Option<String>,
    },
    Preview(PreparedPreview),
}

pub(super) struct Reply {
    pub ticket: Ticket,
    pub result: Result<Completed, ArchiveError>,
}

pub(super) struct Worker {
    requests: SyncSender<Request>,
    replies: Receiver<Reply>,
    pending: Option<Request>,
    cancellation: Option<CancellationToken>,
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
            },
            incoming,
            outgoing,
        )
    }

    pub fn new(context: egui::Context) -> std::io::Result<Self> {
        let (requests, incoming) = mpsc::sync_channel::<Request>(1);
        let (outgoing, replies) = mpsc::sync_channel(1);
        let thread = std::thread::Builder::new()
            .name("arcforge-archive".into())
            .spawn(move || {
                while let Ok(request) = incoming.recv() {
                    let result = execute(&request);
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
        })
    }

    pub fn submit(&mut self, request: Request) -> Result<(), ArchiveError> {
        self.cancel();
        self.cancellation = Some(request.cancel.clone());
        match self.requests.try_send(request) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(request)) => {
                self.pending = Some(request);
                Ok(())
            }
            Err(TrySendError::Disconnected(_)) => {
                Err(ArchiveError::Io("archive worker stopped".into()))
            }
        }
    }

    pub fn poll(&mut self) -> Result<Vec<Reply>, ArchiveError> {
        let mut replies = Vec::new();
        loop {
            match self.replies.try_recv() {
                Ok(reply) => replies.push(reply),
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
        if let Some(pending) = self.pending.take() {
            pending.cancel.cancel();
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel();
    }
}

fn execute(request: &Request) -> Result<Completed, ArchiveError> {
    check_cancel(&request.cancel)?;
    let limits = ArchiveLimits::default();
    let result = match &request.task {
        Task::Open { path, policy } => {
            let snapshot = open_archive(path, policy, &limits, &request.cancel)?;
            prepare(snapshot, "", &request.cancel)
        }
        Task::Reinterpret {
            document,
            policy,
            query,
        } => {
            let snapshot = reinterpret(&document.snapshot, policy, &request.cancel)?;
            prepare(snapshot, query, &request.cancel)
        }
        Task::Filter {
            document,
            query,
            folder,
        } => {
            let rows = document.filter(query, folder.as_deref(), &request.cancel)?;
            Ok(Completed::Filter {
                rows,
                query: query.clone(),
                folder: folder.clone(),
            })
        }
        Task::Preview { document, entry } => {
            let preview = preview_entry(&document.snapshot, *entry, &limits, &request.cancel)?;
            PreparedPreview::new(preview, &request.cancel).map(Completed::Preview)
        }
    }?;
    check_cancel(&request.cancel)?;
    Ok(result)
}

fn prepare(
    snapshot: ArchiveSnapshot,
    query: &str,
    cancel: &CancellationToken,
) -> Result<Completed, ArchiveError> {
    let document = Arc::new(PreparedDocument::new(snapshot, cancel)?);
    let rows = document.filter(query, None, cancel)?;
    Ok(Completed::Document {
        document,
        rows,
        query: query.into(),
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
