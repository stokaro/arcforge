use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use arcforge_core::archive::{ArchiveLimits, EntryPreview, open_archive, reinterpret};

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
    workspace.open_extraction(ExtractionScope::Selected);
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
    workspace.open_extraction(ExtractionScope::Selected);
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
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1100.0, 680.0),
            )),
            ..Default::default()
        },
        |ui| workspace.show(ui),
    );
    let mut text = String::new();
    for shape in &output.shapes {
        collect(&shape.shape, &mut text);
    }
    // This harness inspects shapes without a GPU painter, so discard its texture uploads.
    output.textures_delta.clear();
    text
}

#[test]
fn empty_populated_and_error_workspaces_render_without_a_native_window() {
    let mut workspace = ArchiveWorkspace::default();
    assert!(frame(&mut workspace).contains("No archive open"));
    let fixture = Fixture::new(b"folder/entry.txt");
    workspace.document = Some(fixture.document());
    workspace.rows = vec![ViewRow {
        entry: Some(0),
        folder: None,
        depth: 0,
    }];
    workspace.selected = Some(0);
    let text = frame(&mut workspace);
    assert!(text.contains("folder/entry.txt"));
    assert!(text.contains("Info"));
    assert!(text.contains("CRC32"));
    assert!(text.contains("Load preview"));
    workspace.error = Some("Fixture archive is truncated".into());
    assert!(frame(&mut workspace).contains("Fixture archive is truncated"));
    assert!(
        workspace.worker.is_none(),
        "rendering existing state does not perform archive I/O"
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
    workspace.open_extraction(ExtractionScope::Selected);
    let text = frame(&mut workspace);
    assert!(text.contains("Selected (1)"));
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
