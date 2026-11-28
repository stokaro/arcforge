use super::*;

fn choice(encoding: FilenameEncoding) -> FilenameChoice {
    FilenameChoice {
        encoding: Some(encoding),
        recovery: false,
    }
}

fn unicode(raw: &[u8], name: &[u8], version: u8, crc: u32) -> Vec<u8> {
    let _ = raw;
    let mut field = vec![0x75, 0x70];
    field.extend(u16::try_from(name.len() + 5).unwrap().to_le_bytes());
    field.push(version);
    field.extend(crc.to_le_bytes());
    field.extend(name);
    field
}

#[test]
fn ambiguous_valid_utf8_is_provisional_until_an_explicit_choice() {
    let raw = b"\xc3\xa9.txt";
    let automatic = resolve_name(raw, 0, &[], FilenameChoice::default());
    assert_eq!(automatic.display, "├⌐.txt");
    assert_eq!(automatic.state, NameState::Provisional);
    assert_eq!(automatic.provenance, NameProvenance::ProvisionalCp437);
    assert_eq!(
        resolve_name(raw, 0, &[], choice(FilenameEncoding::Utf8)).display,
        "é.txt"
    );
    assert_eq!(
        resolve_name(raw, 0, &[], choice(FilenameEncoding::Windows1252)).display,
        "Ã©.txt"
    );
    assert_eq!(raw, b"\xc3\xa9.txt");
}

#[test]
fn declarations_survive_fallback_but_explicit_recovery_uses_original_bytes() {
    let raw = "Привет.txt".as_bytes();
    let normal = resolve_name(raw, 0x800, &[], choice(FilenameEncoding::Cp866));
    assert_eq!(normal.display, "Привет.txt");
    assert_eq!(normal.provenance, NameProvenance::DeclaredUtf8);
    let recovery = resolve_name(
        raw,
        0x800,
        &[],
        FilenameChoice {
            recovery: true,
            ..choice(FilenameEncoding::Cp866)
        },
    );
    assert_eq!(recovery.provenance, NameProvenance::Recovery);
    assert_ne!(recovery.display, normal.display);
    let broken = resolve_name(b"\xff.txt", 0x800, &[], choice(FilenameEncoding::Cp866));
    assert_eq!(broken.state, NameState::Invalid);
    assert_eq!(broken.display, "\\xff\\x2e\\x74\\x78\\x74");
    let repaired = resolve_name(
        b"\xff.txt",
        0x800,
        &[],
        FilenameChoice {
            recovery: true,
            ..choice(FilenameEncoding::Cp866)
        },
    );
    assert_eq!(repaired.state, NameState::Resolved);
    assert_eq!(repaired.provenance, NameProvenance::Recovery);
}

#[test]
fn unicode_fields_require_supported_version_crc_bounds_and_strict_utf8() {
    let raw = b"legacy.txt";
    let crc = crc32fast::hash(raw);
    let valid = unicode(raw, "日本語.txt".as_bytes(), 1, crc);
    let decoded = resolve_name(raw, 0, &valid, choice(FilenameEncoding::Cp866));
    assert_eq!(decoded.display, "日本語.txt");
    assert_eq!(decoded.provenance, NameProvenance::UnicodePath);
    for malformed in [
        unicode(raw, b"other.txt", 2, crc),
        unicode(raw, b"other.txt", 1, crc ^ 1),
        unicode(raw, b"\xff.txt", 1, crc),
        vec![0x75, 0x70, 0xff, 0xff, 1],
        vec![0x75],
    ] {
        let result = resolve_name(raw, 0, &malformed, FilenameChoice::default());
        assert_eq!(result.display, "legacy.txt");
        assert_eq!(result.provenance, NameProvenance::Ascii);
        assert!(!result.diagnostics.is_empty());
    }
}

#[test]
fn contradictory_valid_names_remain_conflicts_until_recovery() {
    let raw = b"original.txt";
    let mut fields = unicode(raw, b"one.txt", 1, crc32fast::hash(raw));
    fields.extend(unicode(raw, b"two.txt", 1, crc32fast::hash(raw)));
    let result = resolve_name(raw, 0, &fields, FilenameChoice::default());
    assert_eq!(result.state, NameState::Conflict);
    let declared = resolve_name(raw, 0x800, &fields, choice(FilenameEncoding::Cp437));
    assert_eq!(declared.state, NameState::Conflict);
    assert_eq!(declared.display, "original.txt");
    let recovered = resolve_name(
        raw,
        0,
        &fields,
        FilenameChoice {
            recovery: true,
            ..choice(FilenameEncoding::Cp437)
        },
    );
    assert_eq!(recovered.display, "original.txt");
    assert_eq!(recovered.state, NameState::Resolved);
}

#[test]
fn mixed_archive_policies_are_scoped_by_ordinal_and_preserve_declarations() {
    let mut policy = FilenamePolicy {
        archive: choice(FilenameEncoding::Cp866),
        ..Default::default()
    };
    policy
        .entries
        .insert(7, choice(FilenameEncoding::Windows1251));
    assert_eq!(
        resolve_name(
            b"\x8f\xe0\xa8\xa2\xa5\xe2.txt",
            0,
            &[],
            policy.choice_for(0)
        )
        .display,
        "Привет.txt"
    );
    assert_eq!(
        resolve_name(
            b"\xcf\xf0\xe8\xe2\xe5\xf2.txt",
            0,
            &[],
            policy.choice_for(7)
        )
        .display,
        "Привет.txt"
    );
    assert_eq!(
        resolve_name("世界.txt".as_bytes(), 0x800, &[], policy.choice_for(7)).display,
        "世界.txt"
    );
    policy.archive = FilenameChoice {
        encoding: None,
        recovery: true,
    };
    assert!(policy.validate().is_err());
}

#[test]
fn strict_decoding_does_not_split_multibyte_names_switch_bom_or_normalize() {
    let japanese = resolve_name(b"\x95\x5c.txt", 0, &[], choice(FilenameEncoding::ShiftJis));
    assert_eq!(japanese.display, "表.txt");
    assert_eq!(
        resolve_name(b"\x95", 0, &[], choice(FilenameEncoding::ShiftJis)).state,
        NameState::Invalid
    );
    assert_eq!(
        resolve_name(
            b"\xef\xbb\xbf.txt",
            0,
            &[],
            choice(FilenameEncoding::Windows1252)
        )
        .display,
        "ï»¿.txt"
    );
    assert_eq!(
        resolve_name(b"\xef\xbb\xbf.txt", 0, &[], choice(FilenameEncoding::Utf8)).display,
        "\u{feff}.txt"
    );
    for name in ["é.txt", "e\u{301}.txt"] {
        assert_eq!(
            resolve_name(name.as_bytes(), 0x800, &[], FilenameChoice::default()).display,
            name
        );
    }
    assert!(FilenameEncoding::parse("iso-8859-1").is_none());
    assert!(FilenameEncoding::parse("cp932").is_none());
}

#[test]
fn raw_byte_inspection_and_terminal_labels_do_not_emit_controls() {
    assert_eq!(
        escape_display("a\n\x1b[31m\t世界\u{202e}\\b"),
        "a\\n\\u{1b}[31m\\t世界\\u{202e}\\\\b"
    );
    let malformed = resolve_name(b"\xff\0\n", 0x800, &[], FilenameChoice::default());
    assert_eq!(malformed.display, "\\xff\\x00\\x0a");
}

#[test]
fn repeated_metadata_keeps_diagnostics_and_conflict_memory_bounded() {
    let malformed = [0x75, 0x70, 0, 0].repeat(16_000);
    let result = resolve_name(b"name.txt", 0, &malformed, FilenameChoice::default());
    assert!(result.diagnostics.len() <= 33);
    let field = unicode(b"name.txt", b"same.txt", 1, crc32fast::hash(b"name.txt"));
    let repeated = field.repeat(1000);
    let result = resolve_name(b"name.txt", 0, &repeated, FilenameChoice::default());
    assert_eq!(result.display, "same.txt");
    assert_eq!(result.state, NameState::Resolved);
}
