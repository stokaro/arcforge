# ZIP browsing, preview, and extraction

This is an initial M1 slice of [issue #1](https://github.com/stokaro/arcforge/issues/1) and part of the filename-policy work in [issue #7](https://github.com/stokaro/arcforge/issues/7). It supports opening ZIP/ZIP64, browsing/searching metadata, changing filename interpretation, previewing a complete small entry, and extracting selected/all Stored/Deflate entries into a new directory. It does not complete the M1 editing round trip or all encoding requirements.

## Desktop use

Run `cargo run --locked -p arcforge -- archive.zip`, enter a path through **Open** (`Ctrl+O` / `Cmd+O`), or drop one archive into the workspace. The path field is the current opening interface; a native filesystem picker is future work. Startup and dropped paths retain native OS path bytes.

The workspace is being developed against the [BetterZip 6 design contract](reference/betterzip-6.md), with macOS the primary visual target: session Favorites on the left, a central expandable **Outline** or **Flat** list, and Info/Preview on the right. Remaining visual and native-integration differences are recorded in that contract; the broad arrangement is not a completed fidelity claim. Duplicate central-directory records stay separate. Search is case-insensitive substring matching over decoded names; outline ancestors remain visible. Prepared rows are virtualized. Selection supports modifiers, ranges, and Select All. The inspector exposes sizes, method, CRC, filename evidence, diagnostics, and original bytes. Read-only cancellation invalidates the request; extraction cancellation keeps its ticket until the terminal report is received. Close invalidates the document only when no extraction is still publishing output.

The archive encoding selector applies a fallback only where metadata does not already declare a name. The inspector offers an entry override and explicit recovery. Reinterpretation uses retained original bytes, keeps entry ordinals, and rebuilds folders/search state without writing the archive. Overrides last for the open document; persistent per-archive preferences are not implemented.

Open, indexing, search, reinterpretation, and preview use one worker with bounded request/result channels and at most one latest pending request. Each result carries a document generation and request identity; the UI applies it only if both are current. Superseded/cancelled work cannot replace a newer view or preview. Preview and filename/search preparation run off the UI thread. Extract All/Selected uses the same worker, progress, and core validation as the CLI. While extraction is active, it cannot be superseded by Open, Close, reinterpretation, or another read. Archive-editing actions remain unavailable.

## CLI use

```sh
arcforge-cli list archive.zip --json
arcforge-cli info archive.zip --json
arcforge-cli preview archive.zip --entry 0 --json
arcforge-cli extract archive.zip --output new-output --json
arcforge-cli extract archive.zip --output selected-output --entry 0 --entry 3 --json
arcforge-cli list legacy.zip --filename-encoding cp866
arcforge-cli list broken.zip --filename-encoding windows-1251 --filename-encoding-mode force
arcforge-cli preview archive.zip --entry 0 --max-bytes 1048576
arcforge-cli list -- -option-like-filename.zip
```

Entry IDs are zero-based central-directory ordinals. They are scoped to an opened snapshot, not derived from decoded paths. `list` includes both an ephemeral `id` for the open snapshot and a reproducible `metadata_revision`; `info` and `preview` expose `snapshot_id` and `metadata_revision`. A revision is SHA-256 over source length, directory location, original central records, and footer/comment. It binds entry ordering and original metadata independently of decoding policy. It is not a hash of every payload byte.

For a per-entry override in another invocation, pass the revision from `list` or `info`:

```sh
arcforge-cli list mixed.zip --json
# Substitute the metadata_revision returned above:
arcforge-cli list mixed.zip --metadata-revision REVISION --entry-encoding 3=windows-1251 --json
arcforge-cli preview mixed.zip --entry 3 --metadata-revision REVISION --recover-entry 3=cp866 --json
```

`--entry-encoding` sets a fallback for that ordinal; `--recover-entry` overrides declarations explicitly. Either requires `--metadata-revision`. A changed revision fails without success output. Repeated conflicting options, unknown codecs, invalid IDs, and recovery without a concrete codec are errors. `--metadata-revision` may also guard ordinary list/info/preview/extract requests. IDs without a revision select the archive as opened by the current invocation.

JSON uses `{ "kind": "list|info|preview|extract", "data": ... }`. Extraction success data contains `snapshot_id`, `metadata_revision`, and `report`. A failed/cancelled extraction after destination creation includes the complete report in `data.extraction` on stderr, with exit code 4; preflight errors have no extraction report. Archive failures produce `{ "kind": "error", "data": { "exit_code": 4, "error": ... } }` on stderr with empty stdout. Usage errors are text on stderr. Exit codes: `0` success, `1` output failure, `2` invalid usage, `3` planned operation, `4` archive failure. Commands never prompt or read stdin. The CLI imports no desktop dependencies.

## Filename evidence and interpretation

The provider retains every original central-directory filename, flag word, extra field, and comment. Hex values are exact bytes. Display names and provenance are separate fields; interpretation never changes original bytes or marks archive contents modified.

| Evidence | Automatic/fallback behavior |
| --- | --- |
| UTF-8 flag (EFS / bit 11) and valid UTF-8 | Use declared UTF-8 |
| Invalid bytes with EFS | Keep escaped bytes and an invalid-name diagnostic |
| Unicode Path `0x7075`, version 1, matching filename CRC, valid UTF-8 | Use the declared Unicode name when EFS is absent |
| Conflicting valid declarations | Keep the conflict visible; ordinary fallback cannot erase it |
| No declaration, ASCII bytes | Resolve as ASCII |
| No declaration, non-ASCII bytes | Show provisional CP437 with provenance; UTF-8-shaped bytes alone do not establish UTF-8 |
| Explicit ordinary codec | Decode undeclared original bytes with that fallback |
| Explicit recovery codec | Decode original bytes while ignoring declarations; record recovery provenance |

Unknown/malformed Unicode Path versions, CRC mismatches, and invalid UTF-8 are diagnosed. Structurally invalid extra-field lengths may cause the ZIP dependency to reject the complete archive; no partial listing is reported as success. Duplicate decoded names and raw duplicate records remain separate entries with collision warnings. No Unicode normalization, heuristic UTF-8 guessing, replacement-character decoding, or BOM switching is used.

Available choices are UTF-8, CP437, CP866, Windows-1251, Windows-1252, KOI8-R, KOI8-U, Shift JIS, GBK, Big5, and EUC-KR. CP437 uses the retained lookup table; other legacy choices use strict `encoding_rs` mappings. East Asian labels identify WHATWG mappings, not arbitrary operating-system code-page aliases. Unsupported labels such as `iso-8859-1` and `cp932` are rejected. CP850 and additional codecs remain future work.

Only decoded `/` characters form folder boundaries. Original byte `0x5c` is never split before a multibyte decoder runs. Control characters and bidi formatting controls are escaped in visible labels/terminal output. Text preview is decoded separately as strict UTF-8; a filename codec never reinterprets payload contents.

This slice reports central-directory name evidence. It does not yet collect local-only Unicode Path metadata for every listed entry. Preview compares the selected raw local name, flags, method, timestamps, lengths, and data descriptor with its central record. Structural or raw-name disagreement prevents preview. Conflicting Unicode declarations require explicit recovery even when original local/central name bytes match. Full local/central name reconciliation, persistent preferences, configurable extraction collision policies, and metadata-preserving save remain in issue #7. Current extraction rejects ambiguous names and portable path collisions before writing.

## Preview and source consistency

Preview supports unencrypted Stored (0) and Deflate (8) entries. It reads a complete small entry, verifies declared/actual output length and CRC at EOF, and returns text only if all bytes are valid UTF-8; hex is always available. CRC is an integrity check, not cryptographic authentication. Large entries, directories, encrypted content, and unsupported methods return explicit errors. Partial prefixes are not presented as verified content.

The desktop additionally recognizes PNG/JPEG signatures in these verified bytes and decodes small images on the worker. The existing **256 KiB complete-entry cap** remains in force; ordinary multi-megabyte photos are not yet previewable. Width and height are limited to **2048 pixels**, with explicit pixel/output checks before decoding and a **64 MiB best-effort decoder allocation budget**. The strict dimensions and input bound are distinct from the library's non-strict allocation budget. EXIF orientation is applied. Decoding is bounded but not preemptible; cancellation discards its result before presentation. Corrupt or oversized images retain an explicit image error and access to the verified raw hex. Other image formats, PDF previews, image color management, and animated playback remain unsupported. Image codecs belong to the UI layer and do not change the shared core or CLI payload contract.

An opened snapshot retains a file-identity handle, native path, length/modified observations, directory/footer fingerprint, and immutable provider records. Preview opens an independent reader, checks path/file identity and observations before/after reading, and rechecks directory/footer and selected local metadata. Replacing, truncating, or changing observed metadata invalidates the snapshot. This does not lock the source or provide an atomic filesystem snapshot: concurrent payload changes are additionally detected by the selected entry's CRC, and unselected payloads are not hashed.

## Bounded format support

| Bound | Default |
| --- | --- |
| Entry count | 100,000 |
| Original central directory | 32 MiB |
| Original directory plus archive comment | 32 MiB |
| Reads per operation | 256 MiB |
| Complete preview output | 256 KiB; CLI may select 1 byte–1 MiB |
| Compressed input for one preview | 16 MiB |
| Operation time | 15 seconds, cooperative checks |
| ZIP64 extensible sector | 64 KiB |
| Prepared desktop search keys | 64 MiB |
| Desktop folder view | 10,000 paths / 8 MiB; remaining entries stay in list/search |

Lengths and offsets remain checked `u64` until bounded allocation. Directory/comment limits cover original input bytes; immutable discovery data, parsed records, hexadecimal representations, strings, and views add bounded overhead. There is no whole-file 512 MiB limit in the application provider; sparse ZIP64 beyond 4 GiB is covered by a low-read-budget regression. The development-bundle validator has separate, stricter layout and size limits.

Cancellation/deadline checks run between bounded reads and metadata records; they cannot interrupt an OS-blocked read. The provider preflights EOCD/ZIP64 ranges and allocations, then validates a bounded immutable copy of directory/footer/discovery data before dependency parsing. The dependency cannot reread changed allocation headers from the live file. Comment-embedded end signatures cannot select a different unchecked footer. Final source checks return to the live file.

The initial provider accepts single-disk canonical ZIP layout with a contiguous central directory/footer. SFX/prefixed archives, trailing data, multi-disk layouts, encrypted headers, and inconsistent structural metadata are unsupported. Unsupported compression/encryption may remain inspectable in a structurally valid directory; this does not enable preview. Extraction is a separate bounded operation described below; no archive is edited or saved.

## Extraction

`extract_archive` is shared by the GUI and CLI. It supports unencrypted Stored and Deflate files and empty directory records in the currently supported single-disk ZIP/ZIP64 layout. The source remains read-only. All entries are selected by default; repeated CLI `--entry ID` values select explicit ordinals, and a selected directory includes its interpreted descendants. The core deduplicates overlapping selections by ID; duplicate explicit CLI `--entry` options are rejected as usage errors. Archive-relative paths are preserved. The desktop expands synthetic outline folders on the worker, including their real descendant IDs.

### Preflight and destination

Select a **new directory under an existing parent**. The output itself must not exist: an existing file, directory, symlink, or other object causes failure rather than merge, rename, overwrite, or reuse. The CLI retains native OS bytes for both archive and `--output` paths. Display text is never reopened as a filesystem path.

Before creating output, validate the complete selected set. Require resolved filenames; an undeclared non-ASCII CP437 fallback remains provisional until the user chooses an encoding. The existing fallback/recovery rules and optional metadata revision apply. Reject unsupported/encrypted entries, links, special files, inconsistent file types, and directory records with data.

Accept only portable relative archive paths. Reject absolute/drive/UNC syntax, backslashes, empty or dot/dot-dot components, controls, alternate-stream syntax, reserved Windows characters/device names, and trailing dots/spaces. Limit depth and lengths. Build collision keys using canonical decomposition and Unicode default case folding, then reject duplicates, case/canonical-normalization aliases, differing parent spellings, and file/directory-prefix conflicts. This deliberately conservative policy is the same on all OSes, including case-sensitive filesystems. It never silently renames or discards entries and does not normalize the spelling written to disk.

### Streaming and placement

Open the explicitly chosen parent as a capability. Create the output root exclusively, then traverse/create individual directories through `cap-std`/`cap-fs-ext` directory handles with no-follow opens. Create regular files exclusively; never truncate an existing leaf or follow a pre-existing hardlink to replace its data. Newly created directories use mode 0700 and files 0600 on Unix, subject to the process umask. Do not restore archive ownership, permissions, executable bits, timestamps, ACLs, or extended attributes in this slice; Windows uses its ordinary inherited creation permissions.

Use a fresh source cursor for the operation. Validate source identity and the full retained directory/footer at start and finish; check identity/stamps and local metadata for each entry. Verify output length and CRC for each stream. Write with a 16 KiB buffer and count bytes actually accepted by the sink, including short writes before failure. Any copy error poisons the session so later calls cannot report success. Flush each successfully written file before counting it complete.

Directory-relative handles and no-follow opens constrain placement, with parent/root identity checks before entries and final success. They do not lock the namespace or provide exclusive ownership against another process with the same filesystem permissions. In particular, creating then opening a directory is not an atomic proof that no same-user process replaced it; an already opened directory or file can be renamed while work continues. Report detected replacement as failure, retain handles during work, and never claim immunity to arbitrary concurrent modification. Source checks likewise are not an atomic whole-file snapshot, and CRC is not cryptographic authentication.

### Outcomes and cancellation

An error returned **before destination creation** means this invocation created no destination. Once creation succeeds, return an `ExtractionReport` for every normal outcome: `completed`, `failed`, or `cancelled`. It records the destination, planned/completed entry IDs, written files/directories/bytes, the error, and any incomplete file's entry/path/actual accepted bytes. `incomplete` describes only a new file actually created by this job; a racing existing leaf is not claimed as our partial file.

Completed and partial output remains in the new directory after cancellation, CRC/I/O error, or a detected source/destination change. No automatic recursive cleanup, overwrite, or whole-tree rollback is performed. A retry needs another new directory. The report is an observation of the job's work, not a guarantee that another process cannot subsequently alter the files. Abrupt process termination or power failure may prevent a report and requires inspection of the destination; whole-tree crash durability is not promised.

The desktop offers cooperative Cancel, shows actual progress, and retains the exclusive extraction ticket until the final result. A late cancel does not relabel completed writes as cancellation. The core checks cancellation during reads/writes and between entries. The current CLI has no graceful Ctrl-C signal handler; process interruption can leave output without a terminal JSON report. OS-blocked filesystem calls cannot be forcibly interrupted by the cooperative token.

### Default extraction limits

| Bound | Default |
| --- | --- |
| Selected entries | 100,000 |
| Output per entry | 1 GiB |
| Total actual output | 4 GiB |
| Physical source reads, including validation | 8 GiB |
| Operation time | 300 seconds, cooperative checks |
| Path depth | 64 components |
| Resolved path / component length | 4096 / 255 UTF-8 bytes |
| Explicit and implicit output paths | 100,000 |
| Stored plan paths and collision keys | 64 MiB, plus bounded record overhead |

The core accepts explicit limit objects; this CLI slice uses the defaults. ZIP64 metadata support does not imply that the default extractor accepts a single file larger than 1 GiB. Existing-directory conflict policies, link extraction, archive writing, encrypted payloads, other compression methods, restoration of metadata, and independent queued jobs remain planned.

## Verification

Core fixtures independently encode raw/Unicode duplicate names, malformed metadata, ZIP64 bounds, data descriptors, unsupported methods, CRC/truncation, source replacement, cancellation, and resource budgets. A separate archive from Python's `zipfile` writer exercises CLI listing and Stored/Deflate preview and extraction. Its [generator](../scripts/generate_zip_fixture.py) is optional maintenance tooling; normal Rust checks consume [committed hexadecimal bytes](../tests/fixtures/python-zipfile.hex).

UI tests cover headless rendering and worker/state behavior, including stale results, cancellation, close, and changing search during reinterpretation. These tests do not establish native window interaction, drag/drop integration, accessibility, or visual parity. Each native CI target also unpacks its delivered bundle and runs list/Deflate-preview/extract against the independent fixture; see [release evidence](release.md).

## Provider and source provenance

The dependency remains pinned to `zip = 9.8.2`. The adapter isolates `zip::unstable::read::ZipIterable` behind core. The usual `ZipArchive` name index merges duplicate effective names, while parsed Unicode fields can replace original name bytes. Sequential iteration plus bounded reads of original central records preserves separate ordinals and original evidence. ArcForge owns policy/range validation; zip-rs owns metadata decoding and the Deflate stream. The unstable entry point must be re-audited on dependency upgrades.

The CP437 table in `crates/core/src/archive/names.rs` is adapted from [`src/cp437.rs`](https://github.com/zip-rs/zip2/blob/37a9a8156893c7bce8ac3bf5c3c7cf264f7242ba/src/cp437.rs), zip-rs/zip2 commit `37a9a8156893c7bce8ac3bf5c3c7cf264f7242ba`, recorded in the published 9.8.2 crate. High-byte match arms become a 128-character lookup table; ASCII bytes retain direct values. Copyright (c) 2014 Mathijs van de Nes, MIT. The complete unchanged notice is in [licenses/zip-rs-MIT.txt](../licenses/zip-rs-MIT.txt). No BetterZip/OpenZip application source or artwork is imported.
