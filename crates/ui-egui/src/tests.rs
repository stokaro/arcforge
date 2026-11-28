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
    fn u16le(output: &mut Vec<u8>, value: u16) {
        output.extend(value.to_le_bytes());
    }
    fn u32le(output: &mut Vec<u8>, value: u32) {
        output.extend(value.to_le_bytes());
    }
    let size = u32::try_from(contents.len()).expect("small fixture");
    let name_size = u16::try_from(name.len()).expect("small fixture name");
    let mut crc = !0u32;
    for byte in contents {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & (0u32.wrapping_sub(crc & 1)));
        }
    }
    crc = !crc;
    let mut bytes = Vec::new();
    u32le(&mut bytes, 0x0403_4b50);
    for value in [20, 0, 0, 0, 0] {
        u16le(&mut bytes, value);
    }
    for value in [crc, size, size] {
        u32le(&mut bytes, value);
    }
    u16le(&mut bytes, name_size);
    u16le(&mut bytes, 0);
    bytes.extend(name);
    bytes.extend(contents);
    let central_offset = u32::try_from(bytes.len()).expect("small central offset");
    u32le(&mut bytes, 0x0201_4b50);
    for value in [20, 20, 0, 0, 0, 0] {
        u16le(&mut bytes, value);
    }
    for value in [crc, size, size] {
        u32le(&mut bytes, value);
    }
    for value in [name_size, 0, 0, 0, 0] {
        u16le(&mut bytes, value);
    }
    u32le(&mut bytes, 0);
    u32le(&mut bytes, 0);
    bytes.extend(name);
    let central_size = u32::try_from(bytes.len()).expect("small central size") - central_offset;
    u32le(&mut bytes, 0x0605_4b50);
    for value in [0, 0, 1, 1] {
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
            rows: (0..document.snapshot.entries.len()).collect(),
            document,
            query: String::new(),
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
    workspace.rows = vec![0];
    workspace.selected = Some(0);
    let text = frame(&mut workspace);
    assert!(text.contains("folder/entry.txt"));
    assert!(text.contains("Entry ID: 0"));
    assert!(text.contains("Load preview"));
    workspace.error = Some("Fixture archive is truncated".into());
    assert!(frame(&mut workspace).contains("Fixture archive is truncated"));
    assert!(
        workspace.worker.is_none(),
        "rendering existing state does not perform archive I/O"
    );
}
