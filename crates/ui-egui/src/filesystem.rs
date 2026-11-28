//! Bounded, nonrecursive filesystem navigation independent of archive jobs.
//!
//! Hosts opt in by constructing `FilesystemBrowser`. Tests that only render the
//! workspace never need to discover or scan the real user's directories.

use std::collections::{BTreeMap, VecDeque};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::time::{Duration, Instant};

const MAX_CACHE: usize = 32;
const MAX_PENDING: usize = 16;
const MAX_PATH_BYTES: usize = 128 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Place {
    pub path: PathBuf,
    pub label: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilesystemEntry {
    pub path: PathBuf,
    pub label: String,
    pub is_directory: bool,
    /// A regular file with a .zip extension, not a claim that its ZIP is valid.
    pub is_supported_archive: bool,
}

#[derive(Clone, Debug)]
pub struct DirectoryListing {
    pub entries: Vec<FilesystemEntry>,
    pub truncated: bool,
    pub error: Option<String>,
    pub generation: u64,
}

impl DirectoryListing {
    /// An observed count for a complete immediate directory scan only.
    pub fn supported_archive_count(&self) -> Option<usize> {
        (!self.truncated && self.error.is_none()).then(|| {
            self.entries
                .iter()
                .filter(|entry| entry.is_supported_archive)
                .count()
        })
    }
}

#[derive(Clone, Copy)]
struct ScanLimits {
    entries: usize,
    metadata_bytes: usize,
    elapsed: Duration,
}

impl Default for ScanLimits {
    fn default() -> Self {
        Self {
            entries: 512,
            metadata_bytes: 1024 * 1024,
            elapsed: Duration::from_millis(500),
        }
    }
}

#[derive(Default)]
struct Roots {
    places: Vec<Place>,
    error: Option<String>,
}

struct Request {
    path: PathBuf,
    generation: u64,
    cancelled: Arc<AtomicBool>,
}

enum Reply {
    Roots(Roots),
    Listing {
        path: PathBuf,
        listing: DirectoryListing,
    },
}

struct Cached {
    generation: u64,
    listing: Option<DirectoryListing>,
    pending: bool,
    cancelled: Arc<AtomicBool>,
}

struct Worker {
    requests: SyncSender<Request>,
    replies: Receiver<Reply>,
}

pub struct FilesystemBrowser {
    roots: Roots,
    roots_pending: bool,
    worker: Option<Worker>,
    shutdown: Arc<AtomicBool>,
    cache: BTreeMap<PathBuf, Cached>,
    recent: VecDeque<PathBuf>,
    pending: VecDeque<Request>,
    generation: u64,
}

impl FilesystemBrowser {
    /// Starts one independent filesystem worker. No filesystem I/O runs here.
    pub fn new(context: egui::Context) -> std::io::Result<Self> {
        let (requests, incoming) = mpsc::sync_channel::<Request>(1);
        let (outgoing, replies) = mpsc::sync_channel(1);
        let shutdown = Arc::new(AtomicBool::new(false));
        let worker_shutdown = shutdown.clone();
        let thread = std::thread::Builder::new()
            .name("arcforge-filesystem".into())
            .spawn(move || {
                let roots = discover_roots(&worker_shutdown);
                if worker_shutdown.load(Ordering::Relaxed)
                    || outgoing.send(Reply::Roots(roots)).is_err()
                {
                    return;
                }
                context.request_repaint();
                run_requests(incoming, outgoing, &worker_shutdown, || {
                    context.request_repaint();
                });
            })?;
        // Drop/closed channels stop the worker without blocking a native UI frame.
        drop(thread);
        Ok(Self {
            roots: Roots::default(),
            roots_pending: true,
            worker: Some(Worker { requests, replies }),
            shutdown,
            cache: BTreeMap::new(),
            recent: VecDeque::new(),
            pending: VecDeque::new(),
            generation: 0,
        })
    }

    pub fn roots(&self) -> &[Place] {
        &self.roots.places
    }

    pub fn roots_pending(&self) -> bool {
        self.roots_pending
    }

    pub fn roots_error(&self) -> Option<&str> {
        self.roots.error.as_deref()
    }

    pub fn listing(&self, path: &Path) -> Option<&DirectoryListing> {
        self.cache.get(path)?.listing.as_ref()
    }

    pub fn pending(&self, path: &Path) -> bool {
        self.cache.get(path).is_some_and(|cached| cached.pending)
    }

    /// Load an uncached directory. Repeated requests retain the cached result;
    /// use `refresh` for an explicit retry or a new observation of the directory.
    pub fn request(&mut self, path: PathBuf) -> Result<(), String> {
        if self.cache.contains_key(&path) {
            self.touch(&path);
            return Ok(());
        }
        self.refresh(path)
    }

    /// Supersedes only this directory's previous request, never archive work or
    /// another directory's scan. Stale replies cannot replace the new generation.
    pub fn refresh(&mut self, path: PathBuf) -> Result<(), String> {
        if self.worker.is_none() {
            return Err("Filesystem worker is unavailable.".into());
        }
        if path.as_os_str().is_empty() || path.as_os_str().as_encoded_bytes().len() > MAX_PATH_BYTES
        {
            return Err("Filesystem path is empty or exceeds the navigation limit.".into());
        }
        let generation = self
            .generation
            .checked_add(1)
            .ok_or("Filesystem request generation limit reached.")?;
        let queued_for_path = self
            .pending
            .iter()
            .filter(|request| request.path == path)
            .count();
        if self.pending.len() - queued_for_path >= MAX_PENDING {
            return Err(
                "Too many filesystem folders are pending; wait for a scan to finish.".into(),
            );
        }
        if !self.cache.contains_key(&path) && self.cache.len() == MAX_CACHE {
            let victim = self
                .recent
                .iter()
                .find(|path| self.cache.get(*path).is_some_and(|cached| !cached.pending))
                .cloned()
                .ok_or("Filesystem cache is busy; wait for a scan to finish.")?;
            if let Some(previous) = self.cache.remove(&victim) {
                previous.cancelled.store(true, Ordering::Relaxed);
            }
            self.recent.retain(|path| path != &victim);
        }
        if let Some(previous) = self.cache.get(&path) {
            previous.cancelled.store(true, Ordering::Relaxed);
        }
        self.pending.retain(|request| request.path != path);
        let cancelled = Arc::new(AtomicBool::new(false));
        self.generation = generation;
        self.cache.insert(
            path.clone(),
            Cached {
                generation,
                listing: None,
                pending: true,
                cancelled: cancelled.clone(),
            },
        );
        self.touch(&path);
        self.pending.push_back(Request {
            path,
            generation,
            cancelled,
        });
        self.flush();
        if self.worker.is_some() {
            Ok(())
        } else {
            Err("Filesystem worker is unavailable.".into())
        }
    }

    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        // At most the bounded request backlog plus discovery, even if a worker
        // replies very quickly while the UI is collecting completed scans.
        for _ in 0..MAX_PENDING + 3 {
            let Some(result) = self.worker.as_ref().map(|worker| worker.replies.try_recv()) else {
                break;
            };
            match result {
                Ok(reply) => changed |= self.apply(reply),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.stopped();
                    changed = true;
                    break;
                }
            }
        }
        changed |= self.flush();
        changed
    }

    fn touch(&mut self, path: &Path) {
        self.recent.retain(|previous| previous != path);
        self.recent.push_back(path.to_path_buf());
    }

    fn apply(&mut self, reply: Reply) -> bool {
        match reply {
            Reply::Roots(roots) => {
                self.roots = roots;
                self.roots_pending = false;
                true
            }
            Reply::Listing { path, listing } => {
                if let Some(cached) = self.cache.get_mut(&path)
                    && cached.generation == listing.generation
                    && !cached.cancelled.load(Ordering::Relaxed)
                {
                    cached.listing = Some(listing);
                    cached.pending = false;
                    true
                } else {
                    false
                }
            }
        }
    }

    fn flush(&mut self) -> bool {
        while let Some(request) = self.pending.pop_front() {
            let Some(worker) = &self.worker else {
                break;
            };
            match worker.requests.try_send(request) {
                Ok(()) => {}
                Err(TrySendError::Full(request)) => {
                    self.pending.push_front(request);
                    break;
                }
                Err(TrySendError::Disconnected(_)) => {
                    self.stopped();
                    return true;
                }
            }
        }
        false
    }

    fn stopped(&mut self) {
        self.worker = None;
        self.roots_pending = false;
        self.roots.error = Some("Filesystem worker stopped; reopen the desktop to retry.".into());
        self.pending.clear();
        for cached in self.cache.values_mut().filter(|cached| cached.pending) {
            cached.cancelled.store(true, Ordering::Relaxed);
            cached.pending = false;
            cached.listing = Some(DirectoryListing {
                entries: Vec::new(),
                truncated: false,
                error: Some("Filesystem worker stopped before this scan completed.".into()),
                generation: cached.generation,
            });
        }
    }
}

impl Drop for FilesystemBrowser {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        for cached in self.cache.values() {
            cached.cancelled.store(true, Ordering::Relaxed);
        }
    }
}

fn run_requests(
    incoming: Receiver<Request>,
    outgoing: SyncSender<Reply>,
    shutdown: &AtomicBool,
    wake: impl Fn(),
) {
    while let Ok(request) = incoming.recv() {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }
        let listing = scan_directory(
            &request.path,
            request.generation,
            ScanLimits::default(),
            &request.cancelled,
            shutdown,
        );
        if shutdown.load(Ordering::Relaxed) {
            break;
        }
        if request.cancelled.load(Ordering::Relaxed) {
            // Consuming a cancelled item frees the bounded request channel.
            // Its replacement may still be in the UI-side queue: without a
            // wake, both sides can wait forever for unrelated user input.
            wake();
            continue;
        }
        if outgoing
            .send(Reply::Listing {
                path: request.path,
                listing,
            })
            .is_err()
        {
            break;
        }
        wake();
    }
}

fn discover_roots(shutdown: &AtomicBool) -> Roots {
    // dirs may read XDG configuration or call the OS; this function runs only
    // on the dedicated worker, never while drawing Favorites.
    let candidates = [
        ("Desktop", dirs::desktop_dir()),
        ("Documents", dirs::document_dir()),
        ("Downloads", dirs::download_dir()),
    ];
    let mut roots = existing_roots(candidates, shutdown);
    if roots.places.is_empty() && !shutdown.load(Ordering::Relaxed) {
        let fallback = existing_roots([("Home", dirs::home_dir())], shutdown);
        roots.places = fallback.places;
        if let Some(error) = fallback.error {
            roots.error = Some(
                roots
                    .error
                    .map_or(error.clone(), |previous| format!("{previous}\n{error}")),
            );
        }
    }
    roots
}

fn existing_roots<const N: usize>(
    candidates: [(&str, Option<PathBuf>); N],
    shutdown: &AtomicBool,
) -> Roots {
    let mut roots = Roots::default();
    let mut errors = Vec::new();
    for (label, path) in candidates {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }
        let Some(path) = path else {
            continue;
        };
        if path.as_os_str().as_encoded_bytes().len() > MAX_PATH_BYTES {
            errors.push(format!("{label}: path exceeds the navigation limit."));
            continue;
        }
        match fs::metadata(&path) {
            Ok(metadata) if metadata.is_dir() => roots.places.push(Place {
                path,
                label: label.into(),
            }),
            Ok(_) => errors.push(format!("{label}: configured location is not a directory.")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => errors.push(format!("{label}: {error}")),
        }
    }
    if !errors.is_empty() {
        roots.error = Some(errors.join("\n"));
    }
    roots
}

fn scan_directory(
    path: &Path,
    generation: u64,
    limits: ScanLimits,
    cancelled: &AtomicBool,
    shutdown: &AtomicBool,
) -> DirectoryListing {
    let started = Instant::now();
    let mut result = DirectoryListing {
        entries: Vec::new(),
        truncated: false,
        error: None,
        generation,
    };
    let stopped = || cancelled.load(Ordering::Relaxed) || shutdown.load(Ordering::Relaxed);
    if stopped() {
        result.truncated = true;
        return result;
    }
    let mut directory = match fs::read_dir(path) {
        Ok(directory) => directory,
        Err(error) => {
            result.error = Some(error.to_string());
            return result;
        }
    };
    let mut metadata_bytes = 0usize;
    loop {
        // A blocking filesystem syscall cannot be preempted. Limits and
        // cancellation are checked between calls; the native UI never waits.
        if stopped() || started.elapsed() >= limits.elapsed {
            result.truncated = true;
            break;
        }
        let entry = match directory.next() {
            Some(Ok(entry)) => entry,
            Some(Err(error)) => {
                result.error = Some(error.to_string());
                break;
            }
            None => break,
        };
        if result.entries.len() == limits.entries {
            result.truncated = true;
            break;
        }
        let path = entry.path();
        let label = display_label(&entry.file_name());
        let bytes = path
            .as_os_str()
            .as_encoded_bytes()
            .len()
            .saturating_add(label.len());
        if bytes > limits.metadata_bytes.saturating_sub(metadata_bytes) {
            result.truncated = true;
            break;
        }
        metadata_bytes += bytes;
        // Resolve a symlink only for this immediate entry's kind. There is no
        // recursive traversal; inaccessible/broken links keep their native path
        // with an explicit incomplete-inspection error instead of guessed flags.
        let kind = entry.file_type().and_then(|kind| {
            if kind.is_symlink() {
                fs::metadata(&path).map(|metadata| metadata.file_type())
            } else {
                Ok(kind)
            }
        });
        let (is_directory, is_regular) = match kind {
            Ok(kind) => (kind.is_dir(), kind.is_file()),
            Err(error) => {
                result
                    .error
                    .get_or_insert_with(|| format!("Some entries could not be inspected: {error}"));
                (false, false)
            }
        };
        let is_supported_archive = is_regular && zip_extension(&path);
        result.entries.push(FilesystemEntry {
            path,
            label,
            is_directory,
            is_supported_archive,
        });
    }
    result.entries.sort_by(|left, right| {
        right
            .is_directory
            .cmp(&left.is_directory)
            .then_with(|| left.label.cmp(&right.label))
            .then_with(|| left.path.cmp(&right.path))
    });
    result
}

fn zip_extension(path: &Path) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
}

fn display_label(name: &OsStr) -> String {
    if let Some(name) = name.to_str() {
        return arcforge_core::archive::escape_display(name);
    }
    // Debug formatting escapes non-Unicode native bytes/code units. It is used
    // only for presentation; opening/navigation always retains the PathBuf.
    let escaped = format!("{name:?}");
    escaped
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(&escaped)
        .to_owned()
}

#[cfg(test)]
mod tests;
