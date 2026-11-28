//! Lossless filename evidence and explicit ZIP decoding policy.
//!
//! Filename decoding never changes source bytes or interprets file contents.

use std::collections::BTreeMap;

use serde::Serialize;

use super::EntryId;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum FilenameEncoding {
    #[serde(rename = "utf-8")]
    Utf8,
    #[serde(rename = "cp437")]
    Cp437,
    #[serde(rename = "cp866")]
    Cp866,
    #[serde(rename = "windows-1251")]
    Windows1251,
    #[serde(rename = "windows-1252")]
    Windows1252,
    #[serde(rename = "koi8-r")]
    Koi8R,
    #[serde(rename = "koi8-u")]
    Koi8U,
    #[serde(rename = "shift-jis")]
    ShiftJis,
    #[serde(rename = "gbk")]
    Gbk,
    #[serde(rename = "big5")]
    Big5,
    #[serde(rename = "euc-kr")]
    EucKr,
}

impl FilenameEncoding {
    pub const ALL: &[Self] = &[
        Self::Utf8,
        Self::Cp437,
        Self::Cp866,
        Self::Windows1251,
        Self::Windows1252,
        Self::Koi8R,
        Self::Koi8U,
        Self::ShiftJis,
        Self::Gbk,
        Self::Big5,
        Self::EucKr,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Utf8 => "utf-8",
            Self::Cp437 => "cp437",
            Self::Cp866 => "cp866",
            Self::Windows1251 => "windows-1251",
            Self::Windows1252 => "windows-1252",
            Self::Koi8R => "koi8-r",
            Self::Koi8U => "koi8-u",
            Self::ShiftJis => "shift-jis",
            Self::Gbk => "gbk",
            Self::Big5 => "big5",
            Self::EucKr => "euc-kr",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Utf8 => "UTF-8",
            Self::Cp437 => "CP437 (DOS)",
            Self::Cp866 => "CP866 (DOS Cyrillic)",
            Self::Windows1251 => "Windows-1251",
            Self::Windows1252 => "Windows-1252",
            Self::Koi8R => "KOI8-R",
            Self::Koi8U => "KOI8-U",
            Self::ShiftJis => "Shift JIS (WHATWG)",
            Self::Gbk => "GBK (WHATWG)",
            Self::Big5 => "Big5 (WHATWG)",
            Self::EucKr => "EUC-KR (WHATWG)",
        }
    }

    pub fn parse(label: &str) -> Option<Self> {
        match label.to_ascii_lowercase().as_str() {
            "utf-8" | "utf8" => Some(Self::Utf8),
            "cp437" | "ibm437" => Some(Self::Cp437),
            "cp866" | "ibm866" => Some(Self::Cp866),
            "windows-1251" | "cp1251" => Some(Self::Windows1251),
            "windows-1252" | "cp1252" => Some(Self::Windows1252),
            "koi8-r" => Some(Self::Koi8R),
            "koi8-u" => Some(Self::Koi8U),
            "shift-jis" | "shift_jis" => Some(Self::ShiftJis),
            "gbk" => Some(Self::Gbk),
            "big5" => Some(Self::Big5),
            "euc-kr" => Some(Self::EucKr),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct FilenameChoice {
    /// None means automatic metadata resolution with provisional CP437 fallback.
    pub encoding: Option<FilenameEncoding>,
    /// Explicit recovery ignores filename declarations and decodes original bytes.
    pub recovery: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct FilenamePolicy {
    pub archive: FilenameChoice,
    pub entries: BTreeMap<EntryId, FilenameChoice>,
}

impl FilenamePolicy {
    pub fn choice_for(&self, entry_id: EntryId) -> FilenameChoice {
        self.entries.get(&entry_id).copied().unwrap_or(self.archive)
    }

    pub fn validate(&self) -> Result<(), String> {
        if std::iter::once(&self.archive)
            .chain(self.entries.values())
            .any(|choice| choice.recovery && choice.encoding.is_none())
        {
            return Err("filename recovery requires an explicit encoding".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NameProvenance {
    DeclaredUtf8,
    UnicodePath,
    UserFallback,
    Ascii,
    ProvisionalCp437,
    Recovery,
    EscapedBytes,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NameState {
    Resolved,
    Provisional,
    Conflict,
    Invalid,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct NameResolution {
    /// Exact interpreted Unicode, or an explicitly escaped byte view if invalid.
    pub display: String,
    pub encoding: Option<FilenameEncoding>,
    pub provenance: NameProvenance,
    pub state: NameState,
    pub diagnostics: Vec<String>,
}

pub fn resolve_name(
    raw: &[u8],
    flags: u16,
    extra: &[u8],
    choice: FilenameChoice,
) -> NameResolution {
    let mut diagnostics = Vec::new();
    let unicode_names = unicode_paths(raw, extra, &mut diagnostics);
    let declared = flags & (1 << 11) != 0;
    if choice.recovery {
        diagnostics.push(
            "Explicit recovery overrides filename declarations using original header bytes.".into(),
        );
        return decode_choice(raw, choice.encoding, NameProvenance::Recovery, diagnostics);
    }
    if declared {
        return match std::str::from_utf8(raw) {
            Ok(name) => {
                let conflict = unicode_names.iter().any(|other| other != name);
                if conflict {
                    diagnostics.push("Declared UTF-8 and validated Unicode Path metadata disagree; explicit recovery is required to resolve the conflict.".into());
                }
                NameResolution {
                    display: name.into(),
                    encoding: Some(FilenameEncoding::Utf8),
                    provenance: NameProvenance::DeclaredUtf8,
                    state: if conflict {
                        NameState::Conflict
                    } else {
                        NameState::Resolved
                    },
                    diagnostics,
                }
            }
            Err(_) => {
                diagnostics.push("EFS declares UTF-8, but the original filename bytes are invalid UTF-8; choose an explicit recovery encoding.".into());
                invalid(raw, None, diagnostics)
            }
        };
    }
    if let Some(first) = unicode_names.first() {
        let conflict = unicode_names.iter().any(|other| other != first);
        if conflict {
            diagnostics
                .push("Validated Unicode Path fields disagree; choose explicit recovery.".into());
        }
        return NameResolution {
            display: first.clone(),
            encoding: Some(FilenameEncoding::Utf8),
            provenance: NameProvenance::UnicodePath,
            state: if conflict {
                NameState::Conflict
            } else {
                NameState::Resolved
            },
            diagnostics,
        };
    }
    if choice.encoding.is_some() {
        return decode_choice(
            raw,
            choice.encoding,
            NameProvenance::UserFallback,
            diagnostics,
        );
    }
    if raw.is_ascii() {
        return NameResolution {
            display: raw.iter().map(|byte| char::from(*byte)).collect(),
            encoding: None,
            provenance: NameProvenance::Ascii,
            state: NameState::Resolved,
            diagnostics,
        };
    }
    diagnostics.push("Unmarked non-ASCII filename: CP437 is provisional, not detected. Select a filename encoding if needed.".into());
    NameResolution {
        display: decode_cp437(raw),
        encoding: Some(FilenameEncoding::Cp437),
        provenance: NameProvenance::ProvisionalCp437,
        state: NameState::Provisional,
        diagnostics,
    }
}

fn decode_choice(
    raw: &[u8],
    encoding: Option<FilenameEncoding>,
    provenance: NameProvenance,
    mut diagnostics: Vec<String>,
) -> NameResolution {
    let Some(encoding) = encoding else {
        diagnostics.push("Explicit recovery requires an encoding.".into());
        return invalid(raw, None, diagnostics);
    };
    if let Some(display) = decode(raw, encoding) {
        NameResolution {
            display,
            encoding: Some(encoding),
            provenance,
            state: NameState::Resolved,
            diagnostics,
        }
    } else {
        diagnostics.push(format!(
            "Original filename bytes cannot be decoded strictly as {}.",
            encoding.as_str()
        ));
        invalid(raw, Some(encoding), diagnostics)
    }
}

fn invalid(
    raw: &[u8],
    encoding: Option<FilenameEncoding>,
    diagnostics: Vec<String>,
) -> NameResolution {
    NameResolution {
        display: raw.iter().map(|byte| format!("\\x{byte:02x}")).collect(),
        encoding,
        provenance: NameProvenance::EscapedBytes,
        state: NameState::Invalid,
        diagnostics,
    }
}

fn decode(raw: &[u8], encoding: FilenameEncoding) -> Option<String> {
    let decoder = match encoding {
        FilenameEncoding::Utf8 => return std::str::from_utf8(raw).ok().map(str::to_owned),
        FilenameEncoding::Cp437 => return Some(decode_cp437(raw)),
        FilenameEncoding::Cp866 => encoding_rs::IBM866,
        FilenameEncoding::Windows1251 => encoding_rs::WINDOWS_1251,
        FilenameEncoding::Windows1252 => encoding_rs::WINDOWS_1252,
        FilenameEncoding::Koi8R => encoding_rs::KOI8_R,
        FilenameEncoding::Koi8U => encoding_rs::KOI8_U,
        FilenameEncoding::ShiftJis => encoding_rs::SHIFT_JIS,
        FilenameEncoding::Gbk => encoding_rs::GBK,
        FilenameEncoding::Big5 => encoding_rs::BIG5,
        FilenameEncoding::EucKr => encoding_rs::EUC_KR,
    };
    decoder
        .decode_without_bom_handling_and_without_replacement(raw)
        .map(|value| value.into_owned())
}

fn unicode_paths(raw: &[u8], mut extra: &[u8], diagnostics: &mut Vec<String>) -> Vec<String> {
    let mut names = Vec::new();
    let original_crc = crc32fast::hash(raw);
    let mut omitted_diagnostics = false;
    // Retain the first diagnostics without allocating one String per hostile
    // repeated field. The final public diagnostics are the same bounded view.
    let mut diagnostic = |message: &str| {
        if diagnostics.len() < 32 {
            diagnostics.push(message.into());
        } else {
            omitted_diagnostics = true;
        }
    };
    while !extra.is_empty() {
        if extra.len() < 4 {
            diagnostic("Truncated ZIP extra-field header.");
            break;
        }
        let tag = u16::from_le_bytes([extra[0], extra[1]]);
        let length = usize::from(u16::from_le_bytes([extra[2], extra[3]]));
        extra = &extra[4..];
        if length > extra.len() {
            diagnostic("ZIP extra-field payload exceeds its containing header.");
            break;
        }
        let (payload, rest) = extra.split_at(length);
        extra = rest;
        if tag != 0x7075 {
            continue;
        }
        if payload.len() < 5 {
            diagnostic("Unicode Path field is shorter than its version/CRC header.");
            continue;
        }
        if payload[0] != 1 {
            diagnostic(&format!(
                "Unsupported Unicode Path version {}; field ignored.",
                payload[0]
            ));
            continue;
        }
        let expected_crc = u32::from_le_bytes([payload[1], payload[2], payload[3], payload[4]]);
        if original_crc != expected_crc {
            diagnostic("Unicode Path CRC does not match original filename bytes; field ignored.");
            continue;
        }
        match std::str::from_utf8(&payload[5..]) {
            Ok(name) if names.len() < 2 && !names.iter().any(|existing| existing == name) => {
                names.push(name.to_owned())
            }
            Ok(_) => {}
            Err(_) => diagnostic("Unicode Path contains invalid UTF-8; field ignored."),
        }
    }
    if omitted_diagnostics {
        diagnostics.push("Additional metadata diagnostics omitted; the complete raw extra fields remain available.".into());
    }
    names
}

/// Render names/control bytes safely without changing the interpreted path.
pub fn escape_display(value: &str) -> String {
    let mut result = String::new();
    for character in value.chars() {
        match character {
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            '\\' => result.push_str("\\\\"),
            c if c.is_control()
                || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') =>
            {
                use std::fmt::Write;
                let _ = write!(result, "\\u{{{:x}}}", u32::from(c));
            }
            c => result.push(c),
        }
    }
    result
}

// CP437 mapping adapted from zip-rs/zip2 37a9a8156893c7bce8ac3bf5c3c7cf264f7242ba,
// src/cp437.rs (MIT). Copyright (c) 2014 Mathijs van de Nes.
// Full upstream license and source mapping: licenses/zip-rs-MIT.txt, docs/zip-browser.md.
fn decode_cp437(raw: &[u8]) -> String {
    raw.iter()
        .map(|byte| {
            if *byte < 128 {
                char::from(*byte)
            } else {
                CP437_HIGH[usize::from(*byte - 128)]
            }
        })
        .collect()
}

const CP437_HIGH: [char; 128] = [
    'Ç', 'ü', 'é', 'â', 'ä', 'à', 'å', 'ç', 'ê', 'ë', 'è', 'ï', 'î', 'ì', 'Ä', 'Å', 'É', 'æ', 'Æ',
    'ô', 'ö', 'ò', 'û', 'ù', 'ÿ', 'Ö', 'Ü', '¢', '£', '¥', '₧', 'ƒ', 'á', 'í', 'ó', 'ú', 'ñ', 'Ñ',
    'ª', 'º', '¿', '⌐', '¬', '½', '¼', '¡', '«', '»', '░', '▒', '▓', '│', '┤', '╡', '╢', '╖', '╕',
    '╣', '║', '╗', '╝', '╜', '╛', '┐', '└', '┴', '┬', '├', '─', '┼', '╞', '╟', '╚', '╔', '╩', '╦',
    '╠', '═', '╬', '╧', '╨', '╤', '╥', '╙', '╘', '╒', '╓', '╫', '╪', '┘', '┌', '█', '▄', '▌', '▐',
    '▀', 'α', 'ß', 'Γ', 'π', 'Σ', 'σ', 'µ', 'τ', 'Φ', 'Θ', 'Ω', 'δ', '∞', 'φ', 'ε', '∩', '≡', '±',
    '≥', '≤', '⌠', '⌡', '÷', '≈', '°', '∙', '·', '√', 'ⁿ', '²', '■', '\u{00a0}',
];

#[cfg(test)]
#[path = "names_tests.rs"]
mod tests;
