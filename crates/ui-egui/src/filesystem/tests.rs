use super::*;
use std::sync::atomic::AtomicU64;
use std::time::{SystemTime, UNIX_EPOCH};

struct TemporaryDirectory(PathBuf);

impl TemporaryDirectory {
    fn new() -> Self {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "arcforge-filesystem-{}-{stamp}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("isolated fixture directory");
        Self(path)
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn scan(path: &Path, limits: ScanLimits) -> DirectoryListing {
    scan_directory(
        path,
        7,
        limits,
        &AtomicBool::new(false),
        &AtomicBool::new(false),
    )
}

fn browser_channels() -> (FilesystemBrowser, Receiver<Request>, SyncSender<Reply>) {
    let (requests, incoming) = mpsc::sync_channel(1);
    let (outgoing, replies) = mpsc::sync_channel(1);
    (
        FilesystemBrowser {
            roots: Roots::default(),
            roots_pending: true,
            worker: Some(Worker { requests, replies }),
            shutdown: Arc::new(AtomicBool::new(false)),
            cache: BTreeMap::new(),
            recent: VecDeque::new(),
            pending: VecDeque::new(),
            generation: 0,
        },
        incoming,
        outgoing,
    )
}

fn reply(request: &Request) -> Reply {
    Reply::Listing {
        path: request.path.clone(),
        listing: DirectoryListing {
            entries: Vec::new(),
            truncated: false,
            error: None,
            generation: request.generation,
        },
    }
}

#[test]
fn reads_only_immediate_entries_and_counts_observed_zip_candidates() {
    let directory = TemporaryDirectory::new();
    fs::create_dir(directory.0.join("folder")).expect("subdirectory");
    fs::create_dir(directory.0.join("directory.zip")).expect("ZIP-named directory");
    fs::write(
        directory.0.join("folder/deep.zip"),
        b"not scanned recursively",
    )
    .expect("nested fixture");
    for name in ["one.zip", "Привет 𐐷.ZIP", "notes.txt"] {
        fs::write(
            directory.0.join(name),
            b"extension candidate; contents not inspected",
        )
        .expect("fixture");
    }
    let listing = scan(&directory.0, ScanLimits::default());
    assert!(!listing.truncated);
    assert!(listing.error.is_none());
    assert_eq!(listing.generation, 7);
    assert_eq!(listing.entries.len(), 5);
    assert_eq!(listing.supported_archive_count(), Some(2));
    assert!(listing.entries[..2].iter().all(|entry| entry.is_directory));
    assert!(
        !listing
            .entries
            .iter()
            .any(|entry| entry.label == "deep.zip")
    );
    let zip_directory = listing
        .entries
        .iter()
        .find(|entry| entry.label == "directory.zip")
        .expect("directory");
    assert!(!zip_directory.is_supported_archive);
    assert_eq!(
        fs::read(directory.0.join("one.zip")).expect("source remains"),
        b"extension candidate; contents not inspected"
    );
}

#[test]
fn count_is_unavailable_for_truncated_timed_out_or_failed_observations() {
    let directory = TemporaryDirectory::new();
    fs::write(directory.0.join("one.zip"), b"one").expect("fixture");
    fs::write(directory.0.join("two.zip"), b"two").expect("fixture");
    let exact = scan(
        &directory.0,
        ScanLimits {
            entries: 2,
            ..ScanLimits::default()
        },
    );
    assert_eq!(exact.supported_archive_count(), Some(2));
    for limits in [
        ScanLimits {
            entries: 1,
            ..ScanLimits::default()
        },
        ScanLimits {
            metadata_bytes: 1,
            ..ScanLimits::default()
        },
        ScanLimits {
            elapsed: Duration::ZERO,
            ..ScanLimits::default()
        },
    ] {
        let partial = scan(&directory.0, limits);
        assert!(partial.truncated);
        assert_eq!(partial.supported_archive_count(), None);
    }
    let failed = scan(&directory.0.join("absent"), ScanLimits::default());
    assert!(failed.error.is_some());
    assert_eq!(failed.supported_archive_count(), None);
    let cancelled = scan_directory(
        &directory.0.join("also-absent"),
        1,
        ScanLimits::default(),
        &AtomicBool::new(true),
        &AtomicBool::new(false),
    );
    assert!(cancelled.truncated);
    assert!(
        cancelled.error.is_none(),
        "cancellation was checked before read_dir"
    );
    assert!(cancelled.entries.is_empty());
}

#[test]
fn roots_are_actual_existing_directories_without_creating_missing_places() {
    let directory = TemporaryDirectory::new();
    let desktop = directory.0.join("Actual Desktop");
    fs::create_dir(&desktop).expect("existing configured directory");
    let missing = directory.0.join("Missing Documents");
    let regular = directory.0.join("Downloads-file");
    fs::write(&regular, b"not a directory").expect("regular fixture");
    let roots = existing_roots(
        [
            ("Desktop", Some(desktop.clone())),
            ("Documents", Some(missing.clone())),
            ("Downloads", Some(regular)),
        ],
        &AtomicBool::new(false),
    );
    assert_eq!(
        roots.places,
        vec![Place {
            path: desktop,
            label: "Desktop".into()
        }]
    );
    assert!(
        roots
            .error
            .as_deref()
            .expect("invalid configured root")
            .contains("Downloads")
    );
    assert!(!missing.exists());
}

#[test]
fn retries_reject_stale_generations_without_cancelling_another_folder() {
    let (mut browser, incoming, _outgoing) = browser_channels();
    browser.request("first".into()).expect("first request");
    let first = incoming.try_recv().expect("first queued");
    browser
        .request("second".into())
        .expect("independent request");
    let second = incoming.try_recv().expect("second queued");
    browser.refresh("first".into()).expect("retry first");
    let replacement = incoming.try_recv().expect("replacement queued");
    assert!(first.cancelled.load(Ordering::Relaxed));
    assert!(!second.cancelled.load(Ordering::Relaxed));
    assert!(
        !browser.apply(reply(&first)),
        "superseded reply must not become visible"
    );
    assert!(browser.pending(Path::new("first")));
    assert!(browser.listing(Path::new("first")).is_none());
    assert!(browser.apply(reply(&second)));
    assert!(browser.apply(reply(&replacement)));
    assert_eq!(
        browser
            .listing(Path::new("first"))
            .expect("latest listing")
            .generation,
        replacement.generation
    );
    assert!(!browser.pending(Path::new("first")));
}

#[test]
fn cancelled_queue_item_wakes_its_pending_replacement_without_user_input() {
    let directory = TemporaryDirectory::new();
    fs::write(directory.0.join("observed.zip"), b"extension candidate").expect("fixture");
    let (mut browser, incoming, outgoing) = browser_channels();
    browser
        .request(directory.0.clone())
        .expect("initial request");
    let superseded = incoming.try_recv().expect("first request consumed");
    browser
        .refresh(directory.0.clone())
        .expect("queued replacement");
    browser
        .refresh(directory.0.clone())
        .expect("replacement waits for channel capacity");
    assert!(superseded.cancelled.load(Ordering::Relaxed));
    assert_eq!(browser.pending.len(), 1);
    let generation = browser.generation;

    // Exercise the production worker loop with a deterministic event-loop wake
    // boundary. The cancelled queued item performs no filesystem I/O; only its
    // latest replacement scans the isolated fixture after the UI is notified.
    let (wake_sender, wake_receiver) = mpsc::channel();
    let shutdown = browser.shutdown.clone();
    let worker = std::thread::spawn(move || {
        run_requests(incoming, outgoing, &shutdown, || {
            let _ = wake_sender.send(());
        });
    });
    let woke_after_discard = wake_receiver.recv_timeout(Duration::from_secs(2)).is_ok();
    let completed = if woke_after_discard {
        // Only the actual worker wake drives this poll; no unrelated UI event
        // or periodic spinner is allowed to rescue the replacement request.
        browser.poll();
        let completion_wake = wake_receiver.recv_timeout(Duration::from_secs(2)).is_ok();
        if completion_wake {
            browser.poll();
        }
        completion_wake
            && browser.listing(&directory.0).is_some_and(|listing| {
                listing.generation == generation
                    && listing.supported_archive_count() == Some(1)
                    && !browser.pending(&directory.0)
            })
    } else {
        false
    };
    // Close channels before asserting so the broken baseline also terminates
    // its worker instead of leaving a waiting test thread behind.
    drop(browser);
    worker.join().expect("filesystem worker stopped");
    assert!(
        woke_after_discard,
        "discarding cancelled work must wake the pending UI queue"
    );
    assert!(
        completed,
        "the newest scan must complete without unrelated user input"
    );
}

#[test]
fn queue_is_bounded_and_cached_requests_do_not_rescan() {
    let (mut browser, incoming, _outgoing) = browser_channels();
    browser.request("cached".into()).expect("initial scan");
    let initial = incoming.try_recv().expect("request");
    assert!(browser.apply(reply(&initial)));
    browser.request("cached".into()).expect("cached access");
    assert!(incoming.try_recv().is_err());
    assert_eq!(browser.generation, initial.generation);
    for index in 0..=MAX_PENDING {
        browser
            .request(format!("folder-{index}").into())
            .expect("bounded backlog");
    }
    assert_eq!(browser.pending.len(), MAX_PENDING);
    assert!(browser.request("beyond-queue-limit".into()).is_err());
    assert!(!browser.cache.contains_key(Path::new("beyond-queue-limit")));
    browser
        .refresh("folder-1".into())
        .expect("replace queued request without growing queue");
    assert_eq!(browser.pending.len(), MAX_PENDING);
}

#[test]
fn cache_evicts_completed_observations_and_disconnection_clears_pending() {
    let (mut browser, incoming, outgoing) = browser_channels();
    for index in 0..=MAX_CACHE {
        browser
            .request(format!("folder-{index}").into())
            .expect("request");
        let request = incoming.try_recv().expect("queued request");
        assert!(browser.apply(reply(&request)));
    }
    assert_eq!(browser.cache.len(), MAX_CACHE);
    assert!(browser.listing(Path::new("folder-0")).is_none());
    browser.request("pending".into()).expect("last request");
    drop(outgoing);
    assert!(browser.poll());
    assert!(!browser.pending(Path::new("pending")));
    assert!(
        browser
            .listing(Path::new("pending"))
            .expect("failed pending observation")
            .error
            .is_some()
    );
    assert!(browser.roots_error().is_some());
    assert!(
        !browser.poll(),
        "unchanged state does not rebuild the UI cache"
    );

    let (mut browser, incoming, _outgoing) = browser_channels();
    drop(incoming);
    assert!(browser.request("disconnected-submit".into()).is_err());
    assert!(!browser.pending(Path::new("disconnected-submit")));
    assert!(
        browser
            .listing(Path::new("disconnected-submit"))
            .expect("failed submission")
            .error
            .is_some()
    );
}

#[test]
fn poll_reports_only_visible_changes() {
    let (mut browser, _incoming, outgoing) = browser_channels();
    outgoing
        .send(Reply::Roots(Roots::default()))
        .expect("discovery result");
    assert!(browser.poll());
    assert!(!browser.roots_pending());
    assert!(browser.roots().is_empty());
    assert!(!browser.poll());
    outgoing
        .send(Reply::Listing {
            path: "not-requested".into(),
            listing: DirectoryListing {
                entries: Vec::new(),
                truncated: false,
                error: None,
                generation: 9,
            },
        })
        .expect("stale reply");
    assert!(!browser.poll());
}

#[cfg(target_os = "linux")]
#[test]
fn native_non_utf8_paths_survive_listing_without_replacement_names() {
    use std::os::unix::ffi::OsStringExt;
    let directory = TemporaryDirectory::new();
    let name = std::ffi::OsString::from_vec(b"native-\xff.ZIP".to_vec());
    let path = directory.0.join(name);
    fs::write(&path, b"native path fixture").expect("Linux native filename");
    let listing = scan(&directory.0, ScanLimits::default());
    assert_eq!(listing.entries.len(), 1);
    assert_eq!(listing.entries[0].path, path);
    assert!(listing.entries[0].is_supported_archive);
    assert!(!listing.entries[0].label.contains('\u{fffd}'));
}

#[cfg(unix)]
#[test]
fn symlink_kinds_are_observed_without_recursive_traversal() {
    use std::os::unix::fs::symlink;
    let directory = TemporaryDirectory::new();
    fs::create_dir(directory.0.join("target-directory")).expect("directory");
    symlink(
        directory.0.join("target-directory"),
        directory.0.join("linked-directory"),
    )
    .expect("symlink");
    symlink(directory.0.join("missing"), directory.0.join("broken.zip")).expect("broken link");
    let listing = scan(&directory.0, ScanLimits::default());
    assert!(
        listing
            .entries
            .iter()
            .find(|entry| entry.label == "linked-directory")
            .expect("link")
            .is_directory
    );
    assert!(
        !listing
            .entries
            .iter()
            .find(|entry| entry.label == "broken.zip")
            .expect("broken link")
            .is_supported_archive
    );
    assert!(listing.error.is_some());
    assert_eq!(listing.supported_archive_count(), None);
}
