use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use arcforge_core::archive::{ArchiveLimits, EntryPreview, open_archive, reinterpret};

use super::jobs::FolderTotals;
use super::*;

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

struct Fixture {
    directory: PathBuf,
    path: PathBuf,
}

impl Fixture {
    fn new(name: &[u8]) -> Self {
        let directory = std::env::temp_dir().join(format!(
            "arcforge-ui-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).expect("create isolated fixture directory");
        let path = directory.join("fixture.zip");
        fs::write(&path, stored_zip(name, b"Fixture preview\n")).expect("write fixture");
        Self { directory, path }
    }

    fn document(&self) -> Arc<PreparedDocument> {
        let cancel = CancellationToken::new();
        let snapshot = open_archive(
            &self.path,
            &FilenamePolicy::default(),
            &ArchiveLimits::default(),
            &cancel,
        )
        .expect("open fixture");
        Arc::new(PreparedDocument::new(snapshot, &cancel).expect("prepare fixture"))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn stored_zip(name: &[u8], contents: &[u8]) -> Vec<u8> {
    stored_zip_entries(&[(name, contents)])
}

fn stored_zip_entries(entries: &[(&[u8], &[u8])]) -> Vec<u8> {
    fn u16le(output: &mut Vec<u8>, value: u16) {
        output.extend(value.to_le_bytes());
    }
    fn u32le(output: &mut Vec<u8>, value: u32) {
        output.extend(value.to_le_bytes());
    }
    let mut bytes = Vec::new();
    let mut directory = Vec::new();
    for (name, contents) in entries {
        let size = u32::try_from(contents.len()).expect("small fixture");
        let name_size = u16::try_from(name.len()).expect("small fixture name");
        let mut crc = !0u32;
        for byte in *contents {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xedb8_8320 & (0u32.wrapping_sub(crc & 1)));
            }
        }
        crc = !crc;
        let offset = u32::try_from(bytes.len()).expect("small entry offset");
        u32le(&mut bytes, 0x0403_4b50);
        for value in [20, 0, 0, 0, 0] {
            u16le(&mut bytes, value);
        }
        for value in [crc, size, size] {
            u32le(&mut bytes, value);
        }
        u16le(&mut bytes, name_size);
        u16le(&mut bytes, 0);
        bytes.extend_from_slice(name);
        bytes.extend_from_slice(contents);
        u32le(&mut directory, 0x0201_4b50);
        for value in [20, 20, 0, 0, 0, 0] {
            u16le(&mut directory, value);
        }
        for value in [crc, size, size] {
            u32le(&mut directory, value);
        }
        for value in [name_size, 0, 0, 0, 0] {
            u16le(&mut directory, value);
        }
        u32le(&mut directory, 0);
        u32le(&mut directory, offset);
        directory.extend_from_slice(name);
    }
    let central_offset = u32::try_from(bytes.len()).expect("small central offset");
    let central_size = u32::try_from(directory.len()).expect("small central size");
    bytes.extend(directory);
    let count = u16::try_from(entries.len()).expect("small fixture entry count");
    u32le(&mut bytes, 0x0605_4b50);
    for value in [0, 0, count, count] {
        u16le(&mut bytes, value);
    }
    u32le(&mut bytes, central_size);
    u32le(&mut bytes, central_offset);
    u16le(&mut bytes, 0);
    bytes
}

fn document_reply(ticket: Ticket, document: Arc<PreparedDocument>) -> Reply {
    Reply {
        ticket,
        result: Ok(Completed::Document {
            rows: document
                .view_rows(
                    "",
                    None,
                    ViewMode::Outline,
                    &BTreeSet::new(),
                    &CancellationToken::new(),
                )
                .expect("outline rows"),
            document,
            query: String::new(),
            mode: ViewMode::Outline,
        }),
    }
}

fn preview(entry: EntryId) -> PreparedPreview {
    PreparedPreview::new(
        EntryPreview {
            entry_id: entry,
            bytes: b"hi".to_vec(),
            data_hex: "6869".into(),
            text: Some("hi".into()),
            crc32_verified: true,
        },
        &CancellationToken::new(),
    )
    .expect("prepare preview fixture")
}

fn controlled_picker(
    generation: u64,
    context: &egui::Context,
) -> (
    PendingNativePicker,
    std::sync::mpsc::SyncSender<Option<PathBuf>>,
) {
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    let future = std::future::poll_fn(move |_| match receive.try_recv() {
        Ok(path) => Poll::Ready(path),
        Err(std::sync::mpsc::TryRecvError::Empty) => Poll::Pending,
        Err(std::sync::mpsc::TryRecvError::Disconnected) => Poll::Ready(None),
    });
    (
        PendingNativePicker {
            generation,
            purpose: NativePickerPurpose::OpenArchive,
            future: Box::pin(future),
            waker: Waker::from(Arc::new(PickerRepaint(context.clone()))),
        },
        send,
    )
}

fn choose_extraction_parent(workspace: &mut ArchiveWorkspace, scope: ExtractionScope) {
    let context = egui::Context::default();
    let parent = workspace
        .document
        .as_ref()
        .expect("archive for extraction")
        .snapshot
        .path()
        .parent()
        .expect("archive parent")
        .to_path_buf();
    workspace.open_extraction(scope);
    let purpose = workspace
        .native_picker_request
        .take()
        .expect("native parent chooser requested");
    assert!(matches!(
        purpose,
        NativePickerPurpose::ExtractionParent { .. }
    ));
    let (mut picker, result) = controlled_picker(workspace.generation, &context);
    picker.purpose = purpose;
    workspace.native_picker = Some(picker);
    result
        .send(Some(parent))
        .expect("choose an existing parent");
    workspace.poll_native_picker(&context);
    assert!(workspace.extraction.open);
}

#[test]
fn native_picker_cancel_preserves_document_selection_preview_and_pending_job() {
    let fixture = Fixture::new(b"selected.txt");
    let document = fixture.document();
    let context = egui::Context::default();
    let current = Ticket {
        generation: 3,
        request: 4,
    };
    let mut workspace = ArchiveWorkspace {
        document: Some(document.clone()),
        selected: Some(0),
        selected_entries: BTreeSet::from([0]),
        preview: Some(preview(0)),
        generation: 3,
        request: 4,
        current: Some(current),
        status: "Current archive remains open".into(),
        ..Default::default()
    };
    workspace.request_open_archive();
    assert_eq!(
        workspace.native_picker_request,
        Some(NativePickerPurpose::OpenArchive)
    );
    assert!(
        !workspace.show_open_path,
        "normal Open never shows the path form"
    );
    let (picker, result) = controlled_picker(3, &context);
    workspace.native_picker_request = None;
    workspace.native_picker = Some(picker);
    workspace.poll_native_picker(&context);
    assert!(workspace.native_picker.is_some());
    workspace.request_open_archive();
    assert!(
        workspace.native_picker_request.is_none(),
        "a second Open must not queue another picker"
    );
    result.send(None).expect("cancel native picker");
    workspace.poll_native_picker(&context);
    assert!(workspace.native_picker.is_none());
    assert!(Arc::ptr_eq(
        workspace.document.as_ref().expect("same document"),
        &document
    ));
    assert_eq!(workspace.selected, Some(0));
    assert_eq!(workspace.selected_entries, BTreeSet::from([0]));
    assert_eq!(
        workspace
            .preview
            .as_ref()
            .expect("retained preview")
            .preview
            .bytes,
        b"hi"
    );
    assert_eq!(workspace.current, Some(current));
    assert_eq!(workspace.status, "Current archive remains open");
    assert!(workspace.error.is_none());
    assert!(workspace.worker.is_none(), "cancel performs no archive I/O");
}

#[test]
fn stale_native_picker_cannot_replace_a_newer_archive_or_reopen_a_closed_one() {
    let first = Fixture::new(b"first.txt");
    let second = Fixture::new(b"second.txt");
    let first_document = first.document();
    let second_document = second.document();
    let context = egui::Context::default();
    for open_newer in [true, false] {
        let (worker, incoming, _outgoing) = Worker::test_channels();
        let (picker, result) = controlled_picker(3, &context);
        let mut workspace = ArchiveWorkspace {
            worker: Some(worker),
            document: Some(first_document.clone()),
            generation: 3,
            native_picker: Some(picker),
            ..Default::default()
        };
        if open_newer {
            workspace.open(second.path.clone(), &context);
            let request = incoming.try_recv().expect("newer open request");
            workspace.apply(document_reply(request.ticket, second_document.clone()));
        } else {
            workspace.close();
        }
        let generation = workspace.generation;
        result
            .send(Some(first.path.clone()))
            .expect("complete old chooser");
        workspace.poll_native_picker(&context);
        assert!(workspace.native_picker.is_none());
        assert_eq!(workspace.generation, generation);
        assert!(
            incoming.try_recv().is_err(),
            "a stale selection must not enqueue an open"
        );
        if open_newer {
            assert!(Arc::ptr_eq(
                workspace.document.as_ref().expect("newer document remains"),
                &second_document,
            ));
        } else {
            assert!(workspace.document.is_none());
        }
    }
}

#[test]
fn native_picker_passes_the_exact_native_path_to_the_archive_worker() {
    #[cfg(unix)]
    let path = {
        use std::os::unix::ffi::OsStringExt;
        PathBuf::from(std::ffi::OsString::from_vec(b"/archive-\xff.zip".to_vec()))
    };
    #[cfg(windows)]
    let path = {
        use std::os::windows::ffi::OsStringExt;
        PathBuf::from(std::ffi::OsString::from_wide(&[
            67, 58, 92, 97, 114, 99, 104, 105, 118, 101, 45, 0xd800, 46, 122, 105, 112,
        ]))
    };
    #[cfg(not(any(unix, windows)))]
    let path = PathBuf::from("archive-Привет.zip");
    let context = egui::Context::default();
    let (worker, incoming, _outgoing) = Worker::test_channels();
    let (picker, result) = controlled_picker(2, &context);
    let mut workspace = ArchiveWorkspace {
        worker: Some(worker),
        generation: 2,
        native_picker: Some(picker),
        ..Default::default()
    };
    result
        .send(Some(PathBuf::from(rfd::FileHandle::from(path.clone()))))
        .expect("select native path");
    workspace.poll_native_picker(&context);
    let request = incoming.try_recv().expect("archive open request");
    match request.task {
        Task::Open { path: actual, .. } => assert_eq!(actual, path),
        _ => panic!("the picker must submit the selected archive path"),
    }
    assert_eq!(workspace.generation, 3);
    assert!(workspace.native_picker.is_none());
}

#[test]
fn unavailable_native_parent_reports_an_error_without_replacing_the_archive() {
    struct MissingNativeParent;
    impl HasWindowHandle for MissingNativeParent {
        fn window_handle(
            &self,
        ) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
            Err(raw_window_handle::HandleError::NotSupported)
        }
    }
    impl HasDisplayHandle for MissingNativeParent {
        fn display_handle(
            &self,
        ) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
            Err(raw_window_handle::HandleError::NotSupported)
        }
    }
    let fixture = Fixture::new(b"entry.txt");
    let document = fixture.document();
    let mut workspace = ArchiveWorkspace {
        document: Some(document.clone()),
        ..Default::default()
    };
    workspace.request_open_archive();
    workspace.service_native_picker(&MissingNativeParent, &egui::Context::default());
    assert!(
        workspace
            .error
            .as_deref()
            .is_some_and(|message| message.contains("file chooser"))
    );
    assert!(Arc::ptr_eq(
        workspace.document.as_ref().expect("retained archive"),
        &document
    ));
    assert!(!workspace.native_picker_pending());
    assert!(
        !workspace.show_open_path,
        "failure must not silently replace the native picker with a form"
    );
    assert!(workspace.worker.is_none());
}

#[test]
fn native_extraction_parent_cancellation_preserves_the_previous_result_and_requires_review() {
    let fixture = Fixture::new(b"entry.txt");
    let context = egui::Context::default();
    let (worker, incoming, _outgoing) = Worker::test_channels();
    let mut workspace = ArchiveWorkspace {
        worker: Some(worker),
        document: Some(fixture.document()),
        selected: Some(0),
        selected_entries: BTreeSet::from([0]),
        extraction: ExtractionDialog {
            state: ExtractionState::Finished(Ok(extraction_report(ExtractionStatus::Completed))),
            ..Default::default()
        },
        ..Default::default()
    };
    workspace.open_extraction(ExtractionScope::Selected);
    assert!(
        !workspace.extraction.open,
        "the native chooser precedes the new-folder review"
    );
    let purpose = workspace
        .native_picker_request
        .take()
        .expect("native parent request");
    let (mut picker, result) = controlled_picker(workspace.generation, &context);
    picker.purpose = purpose;
    workspace.native_picker = Some(picker);
    result.send(None).expect("cancel parent selection");
    workspace.poll_native_picker(&context);
    assert!(matches!(
        workspace.extraction.state,
        ExtractionState::Finished(Ok(_))
    ));
    assert_eq!(workspace.selected_entries, BTreeSet::from([0]));
    assert!(incoming.try_recv().is_err());

    choose_extraction_parent(&mut workspace, ExtractionScope::Selected);
    assert!(matches!(
        workspace.extraction.state,
        ExtractionState::Editing
    ));
    assert_eq!(
        workspace.extraction.destination,
        fixture.directory.join("fixture-extracted")
    );
    assert!(
        incoming.try_recv().is_err(),
        "choosing the parent must not write any files"
    );
    for name in [
        "",
        ".",
        "..",
        "../output",
        "nested/output",
        "nested\\output",
    ] {
        workspace.extraction.name_input = name.into();
        workspace.start_extraction(&context);
        assert!(
            incoming.try_recv().is_err(),
            "invalid child name must not start extraction: {name:?}"
        );
        assert!(matches!(
            workspace.extraction.state,
            ExtractionState::Editing
        ));
    }
    workspace.extraction.name_input = "chosen-output".into();
    workspace.extraction.destination = fixture.directory.join("chosen-output");
    workspace.start_extraction(&context);
    match incoming
        .try_recv()
        .expect("explicitly confirmed extraction")
        .task
    {
        Task::Extract {
            destination,
            selection,
            ..
        } => {
            assert_eq!(destination, fixture.directory.join("chosen-output"));
            assert_eq!(selection, ExtractionSelection::Entries(vec![0]));
        }
        _ => panic!("confirmation must reach the extraction worker"),
    }
}

#[test]
fn native_toolbar_commands_expose_only_available_operations() {
    let context = egui::Context::default();
    let mut workspace = ArchiveWorkspace::default();
    let empty = workspace.toolbar_state();
    assert!(empty.can_open);
    assert!(!empty.can_close && !empty.can_extract_all && !empty.can_test);
    workspace.dispatch_command(WorkspaceCommand::OpenArchive, &context);
    assert_eq!(
        workspace.native_picker_request,
        Some(NativePickerPurpose::OpenArchive)
    );
    assert!(!workspace.show_open_path);
    assert!(!workspace.toolbar_state().can_open);
    workspace.native_picker_request = None;
    workspace.dispatch_command(WorkspaceCommand::OpenByPath, &context);
    assert!(
        workspace.show_open_path,
        "path input requires the deliberate advanced command"
    );
    workspace.dispatch_command(WorkspaceCommand::OpenArchive, &context);
    assert!(!workspace.show_open_path);
}

#[test]
fn native_settings_opens_existing_controls_and_waits_for_picker_or_extraction() {
    let context = egui::Context::default();
    let mut workspace = ArchiveWorkspace::default();
    workspace.set_native_toolbar_attached(true);
    workspace.dispatch_command(WorkspaceCommand::OpenSettings, &context);
    let text = frame(&mut workspace);
    assert!(text.contains("Settings"));
    assert!(text.contains("Theme: Light"));
    assert!(text.contains("Archive filename fallback"));
    assert!(workspace.document.is_none());
    assert!(workspace.worker.is_none());

    workspace.show_settings = false;
    workspace.request_open_archive();
    assert!(!workspace.toolbar_state().can_open);
    workspace.dispatch_command(WorkspaceCommand::OpenSettings, &context);
    assert!(!workspace.show_settings, "wait for a queued native chooser");

    workspace.native_picker_request = None;
    let (picker, result) = controlled_picker(workspace.generation, &context);
    workspace.native_picker = Some(picker);
    workspace.dispatch_command(WorkspaceCommand::OpenSettings, &context);
    assert!(
        !workspace.show_settings,
        "wait for an active native chooser"
    );
    result.send(None).expect("cancel native chooser");
    workspace.poll_native_picker(&context);

    workspace.extraction.state = ExtractionState::Running {
        progress: None,
        cancel_requested: false,
    };
    assert!(!workspace.toolbar_state().can_open);
    workspace.dispatch_command(WorkspaceCommand::OpenSettings, &context);
    assert!(!workspace.show_settings, "wait for extraction to finish");
    workspace.extraction.state = ExtractionState::Running {
        progress: None,
        cancel_requested: true,
    };
    workspace.dispatch_command(WorkspaceCommand::OpenSettings, &context);
    assert!(
        !workspace.show_settings,
        "cancellation is not a terminal result"
    );

    workspace.extraction.state = ExtractionState::Finished(Err(ArchiveError::Cancelled));
    assert!(workspace.toolbar_state().can_open);
    workspace.dispatch_command(WorkspaceCommand::OpenSettings, &context);
    assert!(frame(&mut workspace).contains("Archive filename fallback"));
    assert!(workspace.worker.is_none());
}

#[test]
fn stale_open_cannot_replace_a_newer_document_or_reopen_a_closed_one() {
    let first = Fixture::new(b"first.txt");
    let second = Fixture::new(b"second.txt");
    let first_document = first.document();
    let second_document = second.document();
    let old = Ticket {
        generation: 1,
        request: 1,
    };
    let current = Ticket {
        generation: 2,
        request: 1,
    };
    let mut workspace = ArchiveWorkspace {
        generation: 2,
        request: 1,
        current: Some(current),
        blocks_selection: true,
        ..Default::default()
    };
    workspace.apply(document_reply(old, first_document));
    assert!(workspace.document.is_none());
    assert_eq!(workspace.current, Some(current));
    workspace.apply(document_reply(current, second_document.clone()));
    assert_eq!(
        workspace
            .document
            .as_ref()
            .expect("current document")
            .snapshot
            .id,
        second_document.snapshot.id
    );
    workspace.close();
    workspace.apply(document_reply(current, second_document));
    assert!(workspace.document.is_none());
    assert!(workspace.current.is_none());
    assert!(workspace.rows.is_empty());
}

#[test]
fn stale_preview_and_cancelled_error_cannot_overwrite_current_selection() {
    let fixture = Fixture::new(b"entry.txt");
    let current = Ticket {
        generation: 3,
        request: 2,
    };
    let mut workspace = ArchiveWorkspace {
        document: Some(fixture.document()),
        selected: Some(0),
        generation: 3,
        request: 2,
        current: Some(current),
        ..Default::default()
    };
    workspace.apply(Reply {
        ticket: Ticket {
            generation: 3,
            request: 1,
        },
        result: Ok(Completed::Preview(preview(0))),
    });
    assert!(workspace.preview.is_none());
    assert_eq!(workspace.current, Some(current));
    workspace.apply(Reply {
        ticket: current,
        result: Ok(Completed::Preview(preview(1))),
    });
    assert!(
        workspace.preview.is_none(),
        "a matching ticket also needs the selected entry"
    );
    workspace.current = Some(Ticket {
        generation: 3,
        request: 3,
    });
    let cancelled = workspace.current.expect("pending request");
    workspace.cancel();
    workspace.apply(Reply {
        ticket: cancelled,
        result: Err(ArchiveError::SourceChanged),
    });
    assert!(!workspace.source_changed);
    assert!(workspace.error.is_none());
    assert_eq!(workspace.selected, Some(0));
}

#[test]
fn encoding_change_keeps_entry_identity_raw_names_and_source_bytes() {
    let fixture = Fixture::new("Привет.txt".as_bytes());
    let document = fixture.document();
    let before = fs::read(&fixture.path).expect("source bytes before reinterpretation");
    let original_name = document.snapshot.entries[0].raw_name_hex.clone();
    let original_id = document.snapshot.id.clone();
    let mut policy = FilenamePolicy::default();
    policy.archive.encoding = Some(FilenameEncoding::Utf8);
    let cancel = CancellationToken::new();
    let interpreted = reinterpret(&document.snapshot, &policy, &cancel).expect("reinterpret name");
    let interpreted = Arc::new(PreparedDocument::new(interpreted, &cancel).expect("prepare names"));
    let current = Ticket {
        generation: 2,
        request: 1,
    };
    let mut workspace = ArchiveWorkspace {
        document: Some(document),
        selected: Some(0),
        current: Some(current),
        generation: 2,
        request: 1,
        policy,
        ..Default::default()
    };
    workspace.apply(document_reply(current, interpreted));
    let snapshot = &workspace
        .document
        .as_ref()
        .expect("interpreted document")
        .snapshot;
    assert_eq!(snapshot.id, original_id);
    assert_eq!(snapshot.entries[0].raw_name_hex, original_name);
    assert_eq!(snapshot.entries[0].name.display, "Привет.txt");
    assert_eq!(snapshot.entries[0].id, 0);
    assert_eq!(workspace.selected, Some(0));
    assert_eq!(
        workspace.policy.archive.encoding,
        Some(FilenameEncoding::Utf8)
    );
    assert_eq!(
        fs::read(&fixture.path).expect("source bytes after reinterpretation"),
        before
    );
}

#[test]
fn search_preserves_pending_name_policy_and_cancel_restores_the_applied_view() {
    let fixture = Fixture::new(b"folder/entry.txt");
    let document = fixture.document();
    let (worker, incoming, _outgoing) = Worker::test_channels();
    let mut workspace = ArchiveWorkspace {
        worker: Some(worker),
        document: Some(document),
        generation: 2,
        query: "new query".into(),
        applied_query: "old query".into(),
        applied_folder: Some("folder/".into()),
        ..Default::default()
    };
    workspace.policy.archive.encoding = Some(FilenameEncoding::Cp866);
    workspace.filter(&egui::Context::default());
    let request = incoming.try_recv().expect("queued request");
    match request.task {
        Task::Reinterpret { policy, query, .. } => {
            assert_eq!(policy.archive.encoding, Some(FilenameEncoding::Cp866));
            assert_eq!(query, "new query");
        }
        _ => panic!("a search must retain the pending name reinterpretation"),
    }
    workspace.cancel();
    assert!(request.cancel.is_cancelled());
    assert_eq!(workspace.policy, FilenamePolicy::default());
    assert_eq!(workspace.query, "old query");
    assert_eq!(workspace.folder.as_deref(), Some("folder/"));
}

#[test]
fn extraction_waits_for_its_terminal_result_and_cannot_be_superseded() {
    let fixture = Fixture::new(b"entry.txt");
    let document = fixture.document();
    let (worker, incoming, _outgoing) = Worker::test_channels();
    let mut workspace = ArchiveWorkspace {
        worker: Some(worker),
        document: Some(document.clone()),
        rows: vec![ViewRow {
            entry: Some(0),
            folder: None,
            depth: 0,
        }],
        selected: Some(0),
        selected_entries: BTreeSet::from([0]),
        generation: 1,
        ..Default::default()
    };
    let context = egui::Context::default();
    choose_extraction_parent(&mut workspace, ExtractionScope::Selected);
    workspace.start_extraction(&context);
    let request = incoming.try_recv().expect("extraction request");
    assert!(matches!(request.task, Task::Extract { .. }));
    let ticket = workspace.current.expect("pending extraction");
    workspace.close();
    workspace.open("another.zip".into(), &context);
    workspace.filter(&context);
    workspace.change_policy(FilenamePolicy::default(), &context);
    workspace.preview_selected(&context);
    assert_eq!(workspace.current, Some(ticket));
    assert_eq!(
        workspace
            .document
            .as_ref()
            .expect("archive retained")
            .snapshot
            .id,
        document.snapshot.id
    );
    assert!(
        !request.cancel.is_cancelled(),
        "read actions must not cancel filesystem work"
    );
    assert!(incoming.try_recv().is_err());
    workspace.cancel();
    assert!(request.cancel.is_cancelled());
    assert_eq!(
        workspace.current,
        Some(ticket),
        "cancellation still awaits a filesystem report"
    );
    assert!(matches!(
        workspace.extraction.state,
        ExtractionState::Running {
            cancel_requested: true,
            ..
        }
    ));
    workspace.apply(Reply {
        ticket,
        result: Ok(Completed::Extraction(extraction_report(
            ExtractionStatus::Completed,
        ))),
    });
    assert!(workspace.current.is_none());
    assert!(matches!(
        workspace.extraction.state,
        ExtractionState::Finished(Ok(ExtractionReport {
            status: ExtractionStatus::Completed,
            ..
        }))
    ));
    assert!(
        workspace.status.starts_with("Extracted"),
        "completion racing cancellation must remain truthful"
    );
    assert_eq!(workspace.selected_entries, BTreeSet::from([0]));
}

fn extraction_report(status: ExtractionStatus) -> ExtractionReport {
    ExtractionReport {
        destination: "new-output".into(),
        destination_created: true,
        status,
        entries_planned: 1,
        completed_entries: vec![0],
        files_written: 1,
        directories_created: 0,
        bytes_written: 16,
        incomplete: None,
        error: None,
    }
}

#[test]
fn extraction_progress_is_ticket_bound_and_preflight_failure_retains_selection() {
    let fixture = Fixture::new(b"entry.txt");
    let ticket = Ticket {
        generation: 3,
        request: 1,
    };
    let mut workspace = ArchiveWorkspace {
        document: Some(fixture.document()),
        selected: Some(0),
        selected_entries: BTreeSet::from([0]),
        current: Some(ticket),
        extraction: ExtractionDialog {
            open: true,
            state: ExtractionState::Running {
                progress: None,
                cancel_requested: false,
            },
            ..Default::default()
        },
        ..Default::default()
    };
    let progress = ExtractionProgress {
        phase: ExtractionPhase::Extracting,
        completed_entries: 0,
        total_entries: 1,
        bytes_written: 8,
        total_bytes: 16,
        current_entry: Some(0),
    };
    workspace.apply_progress(ProgressReply {
        ticket: Ticket {
            generation: 2,
            request: 1,
        },
        progress: progress.clone(),
    });
    assert!(matches!(
        workspace.extraction.state,
        ExtractionState::Running { progress: None, .. }
    ));
    workspace.apply_progress(ProgressReply { ticket, progress });
    assert!(matches!(
        workspace.extraction.state,
        ExtractionState::Running {
            progress: Some(ExtractionProgress {
                bytes_written: 8,
                ..
            }),
            ..
        }
    ));
    workspace.apply(Reply {
        ticket,
        result: Err(ArchiveError::Unsupported(
            "ambiguous filename; choose an encoding".into(),
        )),
    });
    assert!(matches!(
        workspace.extraction.state,
        ExtractionState::Finished(Err(_))
    ));
    assert_eq!(workspace.selected_entries, BTreeSet::from([0]));
    assert!(workspace.document.is_some());
    assert!(workspace.status.contains("did not create"));
}

#[test]
fn worker_keeps_a_cancelled_pending_extraction_until_its_result() {
    let fixture = Fixture::new(b"entry.txt");
    let (mut worker, incoming, outgoing) = Worker::test_channels();
    let readonly = Ticket {
        generation: 1,
        request: 1,
    };
    let extraction = Ticket {
        generation: 1,
        request: 2,
    };
    let next = Ticket {
        generation: 1,
        request: 3,
    };
    let open_request = |ticket| Request {
        ticket,
        cancel: CancellationToken::new(),
        task: Task::Open {
            path: "unused.zip".into(),
            policy: FilenamePolicy::default(),
        },
    };
    worker
        .submit(open_request(readonly))
        .expect("occupy request slot");
    let cancel = CancellationToken::new();
    worker
        .submit(Request {
            ticket: extraction,
            cancel: cancel.clone(),
            task: Task::Extract {
                document: fixture.document(),
                selection: ExtractionSelection::All,
                folders: BTreeSet::new(),
                destination: "new-output".into(),
            },
        })
        .expect("pending extraction");
    assert!(worker.submit(open_request(next)).is_err());
    assert!(
        !cancel.is_cancelled(),
        "rejected request must not cancel extraction"
    );
    worker.cancel();
    assert!(cancel.is_cancelled());
    assert_eq!(
        incoming.try_recv().expect("old queued request").ticket,
        readonly
    );
    worker.poll().expect("flush cancelled extraction");
    let request = incoming
        .try_recv()
        .expect("cancelled extraction still reaches worker");
    assert_eq!(request.ticket, extraction);
    assert!(request.cancel.is_cancelled());
    outgoing
        .send(Reply {
            ticket: extraction,
            result: Err(ArchiveError::Cancelled),
        })
        .expect("terminal result");
    assert_eq!(worker.poll().expect("collect terminal result").len(), 1);
    worker
        .submit(open_request(next))
        .expect("next operation allowed after result");
}

#[cfg(unix)]
#[test]
fn default_extraction_path_preserves_native_non_utf8_bytes() {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    let source = PathBuf::from(std::ffi::OsString::from_vec(
        b"/tmp/archive-\xff.zip".to_vec(),
    ));
    let destination = default_extraction_destination(&source);
    assert_eq!(
        destination.as_os_str().as_bytes(),
        b"/tmp/archive-\xff-extracted"
    );
}

#[test]
fn extraction_review_preserves_unicode_filesystem_paths() {
    assert_extraction_review_preserves_native_paths(
        OsStr::new("архив-東京.zip"),
        OsStr::new("архив-東京-extracted"),
        OsStr::new("назначение-資料"),
    );
}

// macOS filesystems can reject invalid UTF-8 before the application sees the
// path. Unix OsString preservation is tested above without filesystem access.
#[cfg(target_os = "linux")]
#[test]
fn extraction_review_preserves_non_utf8_filesystem_paths() {
    use std::os::unix::ffi::OsStrExt;

    assert_extraction_review_preserves_native_paths(
        OsStr::from_bytes(b"archive-\xff.zip"),
        OsStr::from_bytes(b"archive-\xff-extracted"),
        OsStr::from_bytes(b"parent-\xfe"),
    );
}

fn assert_extraction_review_preserves_native_paths(
    archive_name: &OsStr,
    expected_name: &OsStr,
    parent_name: &OsStr,
) {
    let mut fixture = Fixture::new(b"entry.txt");
    let native_archive = fixture.directory.join(archive_name);
    fs::rename(&fixture.path, &native_archive).expect("create native filename fixture");
    fixture.path = native_archive;
    let mut workspace = ArchiveWorkspace {
        document: Some(fixture.document()),
        selected: Some(0),
        selected_entries: BTreeSet::from([0]),
        ..Default::default()
    };
    choose_extraction_parent(&mut workspace, ExtractionScope::Selected);
    let expected = fixture.directory.join(expected_name);
    assert_eq!(workspace.extraction.destination, expected);
    let _ = frame(&mut workspace);
    assert_eq!(
        workspace.extraction.destination, expected,
        "displaying the review never reconstructs its native name from lossy text"
    );

    let context = egui::Context::default();
    workspace.request_extraction_parent(ExtractionScope::Selected, true);
    let purpose = workspace
        .native_picker_request
        .take()
        .expect("change parent request");
    let (mut picker, result) = controlled_picker(workspace.generation, &context);
    picker.purpose = purpose;
    workspace.native_picker = Some(picker);
    let native_parent = fixture.directory.join(parent_name);
    fs::create_dir(&native_parent).expect("create native parent fixture");
    result
        .send(Some(native_parent.clone()))
        .expect("choose another native parent");
    workspace.poll_native_picker(&context);
    assert_eq!(
        workspace.extraction.destination,
        native_parent.join(expected.file_name().expect("original native name"))
    );
}

#[test]
fn prepared_views_search_full_names_and_include_implicit_directories() {
    let fixture = Fixture::new(b"outer/inner/Report.txt");
    let document = fixture.document();
    assert_eq!(document.folders, ["outer/", "outer/inner/"]);
    let cancel = CancellationToken::new();
    assert_eq!(
        document
            .filter("REPORT", Some("outer/"), &cancel)
            .expect("filter"),
        [0]
    );
    assert!(
        document
            .filter("report", Some("other/"), &cancel)
            .expect("other folder")
            .is_empty()
    );
    cancel.cancel();
    assert!(matches!(
        document.filter("", None, &cancel),
        Err(ArchiveError::Cancelled)
    ));
}

#[test]
fn outline_disclosures_search_and_flat_view_preserve_archive_identity() {
    let fixture = Fixture::new(b"outer/inner/Report.txt");
    let document = fixture.document();
    let cancel = CancellationToken::new();
    let rows = |query: &str, folder: Option<&str>, mode: ViewMode, expanded: &[&str]| {
        document
            .view_rows(
                query,
                folder,
                mode,
                &expanded.iter().map(|path| (*path).to_owned()).collect(),
                &cancel,
            )
            .expect("prepared view")
    };
    let collapsed = rows("", None, ViewMode::Outline, &[]);
    assert_eq!(collapsed.len(), 1);
    assert_eq!(row_full_path(&document, collapsed[0]), "outer/");
    let first_level = rows("", None, ViewMode::Outline, &["outer/"]);
    assert_eq!(first_level.len(), 2);
    assert_eq!(first_level[1].depth, 1);
    let expanded = rows("", None, ViewMode::Outline, &["outer/", "outer/inner/"]);
    assert_eq!(expanded.len(), 3);
    assert_eq!(row_key(&document, expanded[2]), RowKey::Entry(0));
    assert_eq!(expanded[2].depth, 2);
    assert_eq!(
        rows("REPORT", None, ViewMode::Outline, &[]),
        expanded,
        "search reveals the matching hierarchy without losing its entry id"
    );
    let flat = rows("", None, ViewMode::Flat, &[]);
    assert_eq!(flat.len(), 1);
    assert_eq!(row_key(&document, flat[0]), RowKey::Entry(0));
    assert_eq!(flat[0].depth, 0);
    assert_eq!(
        row_display_name(&document, flat[0], ViewMode::Flat),
        "outer/inner/Report.txt"
    );
    let within = rows("", Some("outer/"), ViewMode::Outline, &[]);
    assert_eq!(within.len(), 1);
    assert_eq!(row_full_path(&document, within[0]), "outer/inner/");
    assert_eq!(within[0].depth, 0);
}

#[test]
fn unrelated_outline_disclosure_preserves_primary_selection_and_prepared_preview() {
    let fixture = Fixture::new(b"placeholder.txt");
    fs::write(
        &fixture.path,
        stored_zip_entries(&[
            (b"Documents/Guide.txt", b"Guide"),
            (b"Photos/Color-study.png", b"Image fixture"),
            (b"Source/lib.rs", b"fn main() {}"),
        ]),
    )
    .expect("write separate archive branches");
    let document = fixture.document();
    let context = egui::Context::default();

    for selected in [0, 1] {
        let expanded = BTreeSet::from(["Documents/".into(), "Photos/".into()]);
        let rows = document
            .view_rows(
                "",
                None,
                ViewMode::Outline,
                &expanded,
                &CancellationToken::new(),
            )
            .expect("initial outline rows");
        let mut prepared = preview(selected);
        if selected == 1 {
            prepared.image = Some(egui::ColorImage::new(
                [1, 1],
                vec![egui::Color32::from_rgb(28, 100, 180)],
            ));
        }
        let (worker, incoming, _outgoing) = Worker::test_channels();
        let mut workspace = ArchiveWorkspace {
            worker: Some(worker),
            document: Some(document.clone()),
            rows,
            selected: Some(selected),
            selected_entries: BTreeSet::from([selected]),
            preview: Some(prepared),
            preview_show_image: selected == 1,
            expanded_folders: expanded.clone(),
            applied_expanded_folders: expanded,
            ..Default::default()
        };
        for expanded in [true, false] {
            workspace.toggle_folder("Source/".into(), &context);
            assert_eq!(
                workspace.selected,
                Some(selected),
                "changing another branch must retain the inspector's primary selection"
            );
            assert!(
                workspace.preview.is_some(),
                "a completed preview remains available while the layout job is pending"
            );
            let request = incoming.try_recv().expect("queued outline update");
            let result = match &request.task {
                Task::Filter {
                    document,
                    query,
                    folder,
                    mode,
                    expanded,
                } => Completed::Filter {
                    rows: document
                        .view_rows(query, folder.as_deref(), *mode, expanded, &request.cancel)
                        .expect("updated outline rows"),
                    query: query.clone(),
                    folder: folder.clone(),
                    mode: *mode,
                    expanded: expanded.clone(),
                },
                _ => panic!("an outline disclosure must only update the prepared view"),
            };
            workspace.apply(Reply {
                ticket: request.ticket,
                result: Ok(result),
            });
            assert_eq!(workspace.expanded_folders.contains("Source/"), expanded);
            assert_eq!(workspace.selected, Some(selected));
            assert_eq!(workspace.selected_entries, BTreeSet::from([selected]));
            assert!(workspace.rows.iter().any(|row| {
                row.entry.map(|index| document.snapshot.entries[index].id) == Some(selected)
            }));
            let retained = workspace
                .preview
                .as_ref()
                .expect("retained completed preview");
            assert_eq!(retained.preview.entry_id, selected);
            assert_eq!(retained.preview.bytes, b"hi");
            assert!(retained.preview.crc32_verified);
            assert_eq!(retained.image.is_some(), selected == 1);
            assert_eq!(workspace.preview_show_image, selected == 1);
        }
    }
}

#[test]
fn folder_totals_include_each_nested_payload_once_for_explicit_and_implicit_folders() {
    let fixture = Fixture::new(b"placeholder.txt");
    fs::write(
        &fixture.path,
        stored_zip_entries(&[
            (b"Root/", b""),
            (b"Root/first.txt", b"first"),
            (b"Root/Nested/data.bin", b"1234567"),
            (b"Root/Nested/second.txt", b"abc"),
            (b"Root/Sub/", b""),
            (b"Root/Sub/item.txt", b"four"),
            (b"Root/Sub/Deeper/item.bin", b"xy"),
            (b"Empty/", b""),
            (b"outside.txt", b"outside"),
        ]),
    )
    .expect("nested stored fixture");
    let document = fixture.document();
    assert_eq!(
        document.snapshot.entries[0].size, 0,
        "directory payload is not its contents size"
    );
    for (path, size, entries) in [
        ("Root/", 21, 5),
        ("Root/Nested/", 10, 2),
        ("Root/Sub/", 6, 2),
        ("Root/Sub/Deeper/", 2, 1),
        ("Empty/", 0, 0),
    ] {
        let folder = document
            .folders
            .binary_search(&path.to_owned())
            .expect("prepared folder");
        assert_eq!(
            document.folder_totals[folder],
            FolderTotals {
                size,
                compressed_size: size,
                entries
            },
            "totals for {path}"
        );
    }
    let filtered = document
        .view_rows(
            "first",
            None,
            ViewMode::Outline,
            &BTreeSet::new(),
            &CancellationToken::new(),
        )
        .expect("search rows");
    assert_eq!(filtered.len(), 2);
    let root = filtered[0].folder.expect("ancestor folder row");
    assert_eq!(
        document.folder_totals[root].size, 21,
        "search does not relabel a full folder total as just the matching file"
    );
}

#[test]
fn status_distinguishes_visible_outline_rows_from_archive_records() {
    let fixture = Fixture::new(b"outer/inner/Report.txt");
    let document = fixture.document();
    let expanded = BTreeSet::from(["outer/".into(), "outer/inner/".into()]);
    let rows = document
        .view_rows(
            "",
            None,
            ViewMode::Outline,
            &expanded,
            &CancellationToken::new(),
        )
        .expect("outline rows");
    let ticket = Ticket {
        generation: 1,
        request: 1,
    };
    let mut workspace = ArchiveWorkspace {
        document: Some(document),
        current: Some(ticket),
        ..Default::default()
    };
    workspace.apply(Reply {
        ticket,
        result: Ok(Completed::Filter {
            rows,
            query: String::new(),
            folder: None,
            mode: ViewMode::Outline,
            expanded,
        }),
    });
    assert!(workspace.status.contains("3 visible rows"));
    assert!(workspace.status.contains("1 archive entry"));
    assert!(!workspace.status.contains("matching"));
    assert!(!workspace.status.contains("Search active"));
}

#[test]
fn ratio_column_reports_packed_size_instead_of_space_saved() {
    let fixture = Fixture::new(b"entry.txt");
    let mut entry = fixture.document().snapshot.entries[0].clone();
    assert_eq!(
        compression_ratio(&entry),
        "100%",
        "stored entries preserve their full size"
    );
    entry.size = 106_398;
    entry.compressed_size = 106_056;
    assert_eq!(compression_ratio(&entry), "99%");
    entry.size = 1_000;
    entry.compressed_size = 250;
    assert_eq!(compression_ratio(&entry), "25%");
    entry.size = 0;
    assert_eq!(compression_ratio(&entry), "—");
}

#[test]
fn modifier_and_range_selection_include_synthetic_folders_without_checkboxes() {
    let fixture = Fixture::new(b"outer/inner/Report.txt");
    let document = fixture.document();
    let expanded = BTreeSet::from(["outer/".into(), "outer/inner/".into()]);
    let rows = document
        .view_rows(
            "",
            None,
            ViewMode::Outline,
            &expanded,
            &CancellationToken::new(),
        )
        .expect("expanded rows");
    let mut workspace = ArchiveWorkspace {
        document: Some(document),
        rows,
        ..Default::default()
    };
    workspace.select_row(0, egui::Modifiers::NONE);
    assert_eq!(
        workspace.selected_folders,
        BTreeSet::from(["outer/".into()])
    );
    workspace.select_row(1, egui::Modifiers::COMMAND);
    assert_eq!(workspace.selection_count(), 2);
    workspace.select_row(1, egui::Modifiers::COMMAND);
    assert_eq!(
        workspace.selected_folders,
        BTreeSet::from(["outer/".into()])
    );
    workspace.select_row(2, egui::Modifiers::SHIFT);
    assert_eq!(
        workspace.selected_folders,
        BTreeSet::from(["outer/inner/".into()])
    );
    assert_eq!(workspace.selected_entries, BTreeSet::from([0]));
    assert_eq!(workspace.selected, Some(0));
    workspace.select_all_visible();
    assert_eq!(workspace.selection_count(), 3);
    let (worker, incoming, _outgoing) = Worker::test_channels();
    workspace.worker = Some(worker);
    choose_extraction_parent(&mut workspace, ExtractionScope::Selected);
    workspace.start_extraction(&egui::Context::default());
    match incoming.try_recv().expect("selected extraction").task {
        Task::Extract {
            selection, folders, ..
        } => {
            assert_eq!(selection, ExtractionSelection::Entries(vec![0]));
            assert_eq!(
                folders,
                BTreeSet::from(["outer/".into(), "outer/inner/".into()])
            );
        }
        _ => panic!("selected extraction must reach the shared extraction worker"),
    }
}

fn frame(workspace: &mut ArchiveWorkspace) -> String {
    fn collect(shape: &egui::epaint::Shape, text: &mut String) {
        match shape {
            egui::epaint::Shape::Text(value) => {
                text.push_str(&value.galley.job.text);
                text.push('\n');
            }
            egui::epaint::Shape::Vec(shapes) => {
                for shape in shapes {
                    collect(shape, text);
                }
            }
            _ => {}
        }
    }
    let context = egui::Context::default();
    configure_style(&context);
    let mut text = String::new();
    // A new egui Window uses an invisible sizing pass before it is painted.
    // Retain the Context across deterministic layout/animation frames, then
    // inspect only the settled frame rather than relying on an incidental Grid
    // multipass elsewhere in the workspace. No native display or sleep is needed.
    for frame_index in 0..3 {
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100.0, 680.0),
                )),
                time: Some(f64::from(frame_index) * 0.1),
                ..Default::default()
            },
            |ui| workspace.show(ui),
        );
        text.clear();
        for shape in &output.shapes {
            collect(&shape.shape, &mut text);
        }
        // This harness inspects shapes without a GPU painter, so discard uploads.
        output.textures_delta.clear();
    }
    text
}

fn native_workspace_frame(
    context: &egui::Context,
    workspace: &mut ArchiveWorkspace,
    size: egui::Vec2,
    frame_index: u32,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            time: Some(f64::from(frame_index) * 0.1),
            events,
            ..Default::default()
        },
        |ui| workspace.show(ui),
    );
    output.textures_delta.clear();
    output
}

fn painted_text_rect(
    output: &egui::FullOutput,
    expected: &str,
) -> Option<(egui::Rect, egui::Rect)> {
    fn find(shape: &egui::epaint::Shape, expected: &str) -> Option<egui::Rect> {
        match shape {
            egui::epaint::Shape::Text(text) if text.galley.job.text == expected => {
                Some(text.galley.rect.translate(text.pos.to_vec2()))
            }
            egui::epaint::Shape::Vec(shapes) => {
                shapes.iter().find_map(|shape| find(shape, expected))
            }
            _ => None,
        }
    }
    output
        .shapes
        .iter()
        .find_map(|shape| find(&shape.shape, expected).map(|rect| (rect, shape.clip_rect)))
}

#[test]
fn narrow_favorites_keeps_count_badges_inside_the_visible_right_gutter() {
    for width in [900.0, 1024.0] {
        let context = egui::Context::default();
        configure_style(&context);
        let mut workspace = ArchiveWorkspace {
            favorites_rows: vec![
                FavoritesRow {
                    path: PathBuf::from("Downloads"),
                    label: "Downloads".into(),
                    depth: 0,
                    kind: FavoritesRowKind::Directory {
                        expanded: true,
                        count: Some(512),
                    },
                },
                FavoritesRow {
                    path: PathBuf::from("Downloads/nested"),
                    label: "Nested folder".into(),
                    depth: 8,
                    kind: FavoritesRowKind::Directory {
                        expanded: false,
                        count: None,
                    },
                },
            ],
            favorites_content_width: 250.0,
            ..Default::default()
        };
        workspace.set_native_toolbar_attached(true);
        let size = egui::vec2(width, 622.0);
        for frame_index in 0..2 {
            native_workspace_frame(&context, &mut workspace, size, frame_index, vec![]);
        }
        let output = native_workspace_frame(&context, &mut workspace, size, 2, vec![]);
        let (badge, clip) = painted_text_rect(&output, "512").expect("actual count badge");
        assert!(
            clip.contains_rect(badge),
            "badge must remain fully visible at {width}: {badge:?}, {clip:?}"
        );
        assert!(
            badge.right() + 6.0 <= clip.right(),
            "badge needs a visible right gutter"
        );
        assert!(
            workspace.filesystem.is_none(),
            "the layout test performs no folder discovery"
        );
    }
}

#[test]
fn narrow_inspector_shows_metadata_disclosure_and_scrolls_expanded_details() {
    let fixture = Fixture::new(b"Photos/Color-study.png");
    let document = fixture.document();
    for size in [
        egui::vec2(900.0, 622.0),
        egui::vec2(1024.0, 622.0),
        egui::vec2(900.0, 440.0),
    ] {
        let context = egui::Context::default();
        configure_style(&context);
        let mut workspace = ArchiveWorkspace {
            document: Some(document.clone()),
            selected: Some(0),
            selected_entries: BTreeSet::from([0]),
            preview: Some(preview(0)),
            ..Default::default()
        };
        workspace.set_native_toolbar_attached(true);
        for frame_index in 0..2 {
            native_workspace_frame(&context, &mut workspace, size, frame_index, vec![]);
        }
        let mut output = native_workspace_frame(&context, &mut workspace, size, 2, vec![]);
        let (_, clip) = painted_text_rect(&output, "PNG image").expect("ordinary Info property");
        if size.y >= 622.0 {
            let (header, clip) = painted_text_rect(&output, "Filename and archive metadata")
                .expect("metadata disclosure is painted");
            assert!(
                clip.contains_rect(header),
                "ordinary Info must show the whole disclosure at {size:?}: {header:?}, {clip:?}"
            );
        }
        let hover = clip.center();
        native_workspace_frame(
            &context,
            &mut workspace,
            size,
            3,
            vec![
                egui::Event::PointerMoved(hover),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, -800.0),
                    phase: egui::TouchPhase::Move,
                    source: egui::MouseWheelSource::Wheel,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        for frame_index in 4..7 {
            output = native_workspace_frame(&context, &mut workspace, size, frame_index, vec![]);
        }
        let (header, clip) = painted_text_rect(&output, "Filename and archive metadata")
            .expect("metadata disclosure after scrolling");
        assert!(
            clip.contains_rect(header),
            "short Info remains scrollable to the complete disclosure"
        );
        let click = header.center();
        for (frame_index, pressed) in [(7, true), (8, false)] {
            native_workspace_frame(
                &context,
                &mut workspace,
                size,
                frame_index,
                vec![
                    egui::Event::PointerMoved(click),
                    egui::Event::PointerButton {
                        pos: click,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        for frame_index in 9..12 {
            native_workspace_frame(&context, &mut workspace, size, frame_index, vec![]);
        }
        native_workspace_frame(
            &context,
            &mut workspace,
            size,
            12,
            vec![
                egui::Event::PointerMoved(hover),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, -2000.0),
                    phase: egui::TouchPhase::Move,
                    source: egui::MouseWheelSource::Wheel,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        for frame_index in 13..17 {
            output = native_workspace_frame(&context, &mut workspace, size, frame_index, vec![]);
        }
        let (last, clip) = painted_text_rect(&output, "Original entry comment (hex)")
            .expect("expanded metadata is reachable by scrolling");
        assert!(
            clip.contains_rect(last),
            "last metadata disclosure is fully reachable at {size:?}"
        );
        let (checksum, _) = painted_text_rect(&output, "CRC32").expect("checksum remains visible");
        assert!(
            last.bottom() < checksum.top(),
            "metadata must stay above the checksum panel"
        );
        assert_eq!(workspace.selected, Some(0));
        assert!(workspace.preview.is_some());
        assert!(workspace.worker.is_none());
    }
}

#[test]
fn empty_populated_and_error_workspaces_render_without_a_native_window() {
    let mut workspace = ArchiveWorkspace::default();
    let empty = frame(&mut workspace);
    assert!(empty.contains("Drop an archive here to open"));
    assert!(empty.contains("No files selected"));
    assert!(!empty.contains("compress or extract"));
    let fixture = Fixture::new(b"folder/entry.txt");
    workspace.document = Some(fixture.document());
    workspace.rows = vec![ViewRow {
        entry: Some(0),
        folder: None,
        depth: 0,
    }];
    workspace.selected = Some(0);
    let text = frame(&mut workspace);
    assert!(text.contains("entry.txt"));
    assert!(text.contains("Path"));
    assert!(text.contains("folder"));
    assert!(text.contains("Attributes"));
    assert!(text.contains("CRC32"));
    assert!(text.contains("Load preview"));
    workspace.error = Some("Fixture archive is truncated".into());
    assert!(frame(&mut workspace).contains("Fixture archive is truncated"));
    assert!(
        workspace.worker.is_none(),
        "rendering existing state does not perform archive I/O"
    );
    assert!(
        workspace.filesystem.is_none(),
        "headless rendering does not discover or scan local folders"
    );
}

#[test]
fn extraction_form_and_retained_output_report_render_without_filesystem_work() {
    let fixture = Fixture::new(b"entry.txt");
    let mut workspace = ArchiveWorkspace {
        document: Some(fixture.document()),
        selected: Some(0),
        selected_entries: BTreeSet::from([0]),
        ..Default::default()
    };
    choose_extraction_parent(&mut workspace, ExtractionScope::Selected);
    let text = frame(&mut workspace);
    assert!(
        text.contains("Selected (1)"),
        "the extraction form must render its selected scope; rendered text:\n{text}"
    );
    assert!(text.contains("Extract 1 selected item"));
    assert!(!text.contains("1 selected items"));
    assert!(text.contains("destination must not exist"));
    assert!(text.contains("Extract Here"));
    workspace.extraction.state = ExtractionState::Running {
        progress: None,
        cancel_requested: true,
    };
    let text = frame(&mut workspace);
    assert!(text.contains("Cancelling extraction"));
    assert!(text.contains("Waiting for the worker"));
    assert!(
        !text.contains("0 of 0"),
        "checking with unknown totals must stay indeterminate"
    );
    let mut report = extraction_report(ExtractionStatus::Failed);
    report.directories_created = 1;
    report.error = Some(ArchiveError::InvalidArchive("CRC32 mismatch".into()));
    report.incomplete = Some(arcforge_core::archive::IncompleteExtractionEntry {
        entry_id: 0,
        path: "entry.txt".into(),
        bytes_written: 8,
    });
    workspace.extraction.state = ExtractionState::Finished(Ok(report));
    let text = frame(&mut workspace);
    assert!(text.contains("Extraction failed"));
    assert!(text.contains("1 file · 1 directory"));
    assert!(text.contains("new directory is retained"));
    assert!(text.contains("Incomplete file: entry.txt"));
    assert!(text.contains("CRC32 mismatch"));
    workspace.extraction.open = false;
    assert!(frame(&mut workspace).contains("Details"));
    assert!(
        matches!(workspace.extraction.state, ExtractionState::Finished(Ok(_))),
        "dismissing the result keeps its report available"
    );
    workspace.extraction.open = true;
    workspace.extraction.state = ExtractionState::Interrupted("worker disconnected".into());
    let text = frame(&mut workspace);
    assert!(text.contains("final output state is unknown"));
    assert!(!text.contains("No destination directory was created"));
    assert!(workspace.worker.is_none());
}

#[test]
fn extraction_dialog_height_tracks_its_content_instead_of_the_viewport() {
    fn settled_rect(
        context: &egui::Context,
        workspace: &mut ArchiveWorkspace,
        viewport_height: f32,
    ) -> egui::Rect {
        // Auto-sized windows use the preceding frame's width for their title.
        // Three layout passes settle that feedback without a display or sleeps.
        for _ in 0..3 {
            let mut output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1100.0, viewport_height),
                    )),
                    ..Default::default()
                },
                |_ui| workspace.extraction_window(context),
            );
            output.textures_delta.clear();
        }
        context
            .memory(|memory| memory.area_rect(egui::Id::unique("extraction_sheet")))
            .expect("the real extraction window has a measured rectangle")
    }

    let fixture = Fixture::new(b"entry.txt");
    let context = egui::Context::default();
    configure_style(&context);
    let mut workspace = ArchiveWorkspace {
        document: Some(fixture.document()),
        selected: Some(0),
        selected_entries: BTreeSet::from([0]),
        ..Default::default()
    };
    choose_extraction_parent(&mut workspace, ExtractionScope::Selected);
    let editing = settled_rect(&context, &mut workspace, 822.0);
    assert!(
        editing.height() < 450.0,
        "the form must not consume the remaining viewport: {editing:?}"
    );
    let editing_tall = settled_rect(&context, &mut workspace, 1200.0);
    assert!(
        (editing_tall.height() - editing.height()).abs() < 2.0,
        "extra viewport height must not expand the footer: before={editing:?}, after={editing_tall:?}"
    );

    workspace.extraction.state =
        ExtractionState::Finished(Ok(extraction_report(ExtractionStatus::Completed)));
    let finished_tall = settled_rect(&context, &mut workspace, 1200.0);
    assert!(
        finished_tall.height() < 280.0,
        "a short result should stay compact: {finished_tall:?}"
    );
    assert!(
        finished_tall.height() + 40.0 < editing_tall.height(),
        "the same window must shrink when the form becomes a shorter result: form={editing_tall:?}, result={finished_tall:?}"
    );
    let finished = settled_rect(&context, &mut workspace, 822.0);
    assert!(
        (finished_tall.height() - finished.height()).abs() < 2.0,
        "result height follows its content at either viewport size"
    );
    assert!(
        workspace.worker.is_none(),
        "measuring the form performs no filesystem work"
    );
}

#[test]
fn narrow_preview_keeps_prepared_rows_on_one_line_and_scrolls_horizontally() {
    fn render(
        context: &egui::Context,
        workspace: &mut ArchiveWorkspace,
        time: f64,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(240.0, 320.0),
                )),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| {
                workspace.preview_view(ui);
            },
        );
        output.textures_delta.clear();
        output
    }

    fn row_layout(shape: &egui::epaint::Shape, expected: &str) -> Option<(usize, f32, egui::Pos2)> {
        match shape {
            egui::epaint::Shape::Text(value) if value.galley.job.text == expected => Some((
                value.galley.rows.len(),
                value.galley.rect.width(),
                value.pos,
            )),
            egui::epaint::Shape::Vec(shapes) => {
                shapes.iter().find_map(|shape| row_layout(shape, expected))
            }
            _ => None,
        }
    }

    for hex in [false, true] {
        let context = egui::Context::default();
        let prepared = PreparedPreview::new(
            EntryPreview {
                entry_id: 0,
                bytes: vec![b'x'; 200],
                data_hex: "78".repeat(200),
                text: Some("x".repeat(200)),
                crc32_verified: true,
            },
            &CancellationToken::new(),
        )
        .expect("long preview fixture");
        let expected = if hex {
            prepared.hex_rows[0].clone()
        } else {
            prepared.text_rows.as_ref().expect("UTF-8 preview")[0].clone()
        };
        let mut workspace = ArchiveWorkspace {
            preview: Some(prepared),
            preview_hex: hex,
            ..Default::default()
        };
        let first = render(&context, &mut workspace, 0.0, vec![]);
        let (rows, width, origin) = first
            .shapes
            .iter()
            .find_map(|shape| row_layout(&shape.shape, &expected))
            .expect("preview row is painted");
        assert_eq!(
            rows, 1,
            "prepared rows must keep the virtualized row height"
        );
        assert!(
            width > 240.0,
            "long content extends past the narrow viewport"
        );
        let _wheel_frame = render(
            &context,
            &mut workspace,
            1.0 / 60.0,
            vec![
                egui::Event::PointerMoved(origin + egui::vec2(10.0, 5.0)),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::Vec2::ZERO,
                    phase: egui::TouchPhase::Start,
                    source: egui::MouseWheelSource::Trackpad,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(-90.0, 0.0),
                    phase: egui::TouchPhase::Move,
                    source: egui::MouseWheelSource::Trackpad,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        // ScrollArea consumes wheel input after laying out its content; inspect the next frame.
        let scrolled = render(&context, &mut workspace, 2.0 / 60.0, vec![]);
        let (scrolled_rows, _, scrolled_origin) = scrolled
            .shapes
            .iter()
            .find_map(|shape| row_layout(&shape.shape, &expected))
            .expect("preview row remains painted after scrolling");
        assert_eq!(scrolled_rows, 1);
        assert!(
            scrolled_origin.x < origin.x,
            "wide preview can scroll horizontally: hex={hex}, width={width}, before={origin:?}, after={scrolled_origin:?}"
        );
    }
}
