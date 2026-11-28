//! Readable regular/semibold faces, with system fonts loaded before the event loop.
//!
//! Apple and Microsoft fonts are read from the current machine only. The bundled
//! Open Sans fallback is unmodified and licensed separately; see
//! `docs/reference/typography.md` and `licenses/fonts/OpenSans-NOTICE.txt`.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use egui::{FontData, FontDefinitions, FontFamily, FontId};
use skrifa::attribute::{Stretch, Style};
use skrifa::{FontRef, MetadataProvider, Tag};

const SEMIBOLD_FAMILY: &str = "ArcForge Semibold";
const REGULAR_FONT: &str = "Open Sans Regular";
const SEMIBOLD_FONT: &str = "Open Sans Semibold";
const SYSTEM_REGULAR: &str = "ArcForge System Regular";
const SYSTEM_SEMIBOLD: &str = "ArcForge System Semibold";
const MAX_FONT_BYTES: u64 = 32 * 1024 * 1024;
const MAX_FACES: usize = 64;
const MAX_NAMED_INSTANCES: usize = 256;
const REGULAR_BYTES: &[u8] = include_bytes!("../assets/fonts/OpenSans-Regular.ttf");
const SEMIBOLD_BYTES: &[u8] = include_bytes!("../assets/fonts/OpenSans-Semibold.ttf");

static PREPARED: OnceLock<PreparedFonts> = OnceLock::new();

/// The actual primary faces used by this process, for native-capture provenance.
#[derive(Clone, Debug)]
pub struct FontSelection {
    pub regular: FaceSelection,
    pub semibold: FaceSelection,
}

#[derive(Clone, Debug)]
pub struct FaceSelection {
    /// Absolute system path, or repository-relative path of an embedded asset.
    pub source_path: PathBuf,
    pub embedded: bool,
    /// The font's own full-name record; variable instances may share this name.
    pub font_name: Option<String>,
    pub face_index: u32,
    /// Selected OpenType weight class, including a variable/named instance.
    pub weight: f32,
    pub variation_coordinates: Vec<(String, f32)>,
}

struct PreparedFonts {
    definitions: FontDefinitions,
    selection: FontSelection,
}

/// Read the bounded platform candidates once, before starting the UI event loop.
/// Missing or unusable system fonts leave the licensed embedded pair in place.
pub fn prepare() {
    PREPARED.get_or_init(|| {
        let pair = system_pair();
        PreparedFonts {
            selection: describe_selection(pair.as_ref()),
            definitions: definitions(pair),
        }
    });
}

/// Apply the prepared fonts without doing filesystem I/O inside a UI frame.
/// A library-only caller that has not called [`prepare`] gets the embedded pair.
pub fn install(context: &egui::Context) {
    context.set_fonts(
        PREPARED
            .get()
            .map(|prepared| prepared.definitions.clone())
            .unwrap_or_else(|| definitions(None)),
    );
}

/// Describe the same prepared pair that [`install`] uses; this does no I/O.
pub fn selection() -> FontSelection {
    PREPARED
        .get()
        .map(|prepared| prepared.selection.clone())
        .unwrap_or_else(|| describe_selection(None))
}

pub fn regular(size: f32) -> FontId {
    FontId::proportional(size)
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD_FAMILY.into()))
}

struct FontPair {
    regular: FontData,
    semibold: FontData,
    regular_path: PathBuf,
    semibold_path: PathBuf,
}

fn describe_selection(system: Option<&FontPair>) -> FontSelection {
    let describe = |font: &FontData, path: PathBuf, embedded, weight| FaceSelection {
        source_path: path,
        embedded,
        font_name: FontRef::from_index(font.bytes(), font.index)
            .ok()
            .and_then(|face| {
                face.localized_strings(skrifa::string::StringId::FULL_NAME)
                    .english_or_first()
                    .map(|name| name.chars().collect())
            }),
        face_index: font.index,
        weight,
        variation_coordinates: font
            .tweak
            .coords
            .as_ref()
            .iter()
            .map(|(tag, value)| (tag.to_string(), *value))
            .collect(),
    };
    if let Some(pair) = system {
        FontSelection {
            regular: describe(&pair.regular, pair.regular_path.clone(), false, 400.0),
            semibold: describe(&pair.semibold, pair.semibold_path.clone(), false, 600.0),
        }
    } else {
        FontSelection {
            regular: describe(
                &FontData::from_static(REGULAR_BYTES),
                "crates/ui-egui/assets/fonts/OpenSans-Regular.ttf".into(),
                true,
                400.0,
            ),
            semibold: describe(
                &FontData::from_static(SEMIBOLD_BYTES),
                "crates/ui-egui/assets/fonts/OpenSans-Semibold.ttf".into(),
                true,
                600.0,
            ),
        }
    }
}

fn definitions(system: Option<FontPair>) -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        REGULAR_FONT.into(),
        Arc::new(FontData::from_static(REGULAR_BYTES)),
    );
    fonts.font_data.insert(
        SEMIBOLD_FONT.into(),
        Arc::new(FontData::from_static(SEMIBOLD_BYTES)),
    );

    let mut regular = vec![REGULAR_FONT.to_owned()];
    let mut semibold = vec![SEMIBOLD_FONT.to_owned(), REGULAR_FONT.to_owned()];
    if let Some(pair) = system {
        fonts
            .font_data
            .insert(SYSTEM_REGULAR.into(), Arc::new(pair.regular));
        fonts
            .font_data
            .insert(SYSTEM_SEMIBOLD.into(), Arc::new(pair.semibold));
        regular.insert(0, SYSTEM_REGULAR.into());
        semibold.insert(0, SYSTEM_SEMIBOLD.into());
    }
    // Keep egui's symbol/glyph fallbacks and its genuine regular monospace face.
    // Ubuntu Light is no longer the primary face of either proportional family.
    if let Some(fallbacks) = fonts.families.get(&FontFamily::Proportional) {
        regular.extend(fallbacks.iter().cloned());
        semibold.extend(fallbacks.iter().cloned());
    }
    fonts.families.insert(FontFamily::Proportional, regular);
    fonts
        .families
        .insert(FontFamily::Name(SEMIBOLD_FAMILY.into()), semibold);
    fonts
}

fn system_pair() -> Option<FontPair> {
    // Use a pair from one family. A missing semibold face must not silently turn
    // headings into regular/light text or mix unrelated primary typefaces.
    for candidates in system_candidates() {
        let mut regular = None;
        let mut semibold = None;
        for path in candidates {
            let Some(bytes) = read_font(&path) else {
                continue;
            };
            if regular.is_none() {
                regular = select_face(Arc::clone(&bytes), 400.0).map(|font| (font, path.clone()));
            }
            if semibold.is_none() {
                semibold = select_face(bytes, 600.0).map(|font| (font, path));
            }
            if regular.is_some() && semibold.is_some() {
                break;
            }
        }
        if let (Some((regular, regular_path)), Some((semibold, semibold_path))) =
            (regular, semibold)
        {
            return Some(FontPair {
                regular,
                semibold,
                regular_path,
                semibold_path,
            });
        }
    }
    None
}

fn read_font(path: &Path) -> Option<Arc<Vec<u8>>> {
    let file = File::open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_FONT_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_FONT_BYTES + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > MAX_FONT_BYTES {
        return None;
    }
    Some(Arc::new(bytes))
}

fn select_face(bytes: Arc<Vec<u8>>, weight: f32) -> Option<FontData> {
    for (index, font) in FontRef::fonts(bytes.as_slice()).take(MAX_FACES).enumerate() {
        let Ok(font) = font else { continue };
        let attributes = font.attributes();
        if attributes.style != Style::Normal || attributes.stretch != Stretch::NORMAL {
            continue;
        }
        if ['A', 'a', '0', ' ']
            .into_iter()
            .any(|character| font.charmap().map(character).is_none())
        {
            continue;
        }

        let mut data = FontData::from_blob(bytes.clone(), index as u32);
        let axes = font.axes();
        if let Some(axis) = axes.iter().find(|axis| axis.tag() == Tag::new(b"wght")) {
            let Some(value) = variable_weight(&font, &axis, weight) else {
                continue;
            };
            data.tweak.coords.push(b"wght", value);
        } else if (attributes.weight.value() - weight).abs() > 1.0 {
            continue;
        }
        // egui does not derive the optical-size axis from FontId's point size.
        // Choose the text-sized design for this compact desktop UI, rather than
        // a system variable font's potentially large display-size default.
        if let Some(axis) = axes.iter().find(|axis| axis.tag() == Tag::new(b"opsz")) {
            if axis.min_value() > axis.max_value() {
                continue;
            }
            data.tweak
                .coords
                .push(b"opsz", 14.0_f32.clamp(axis.min_value(), axis.max_value()));
        }
        return Some(data);
    }
    None
}

fn variable_weight(font: &FontRef<'_>, axis: &skrifa::Axis, weight: f32) -> Option<f32> {
    let range = axis.min_value()..=axis.max_value();
    if range.contains(&weight) {
        return Some(weight);
    }
    // Some older system fonts use a nonstandard weight coordinate range. Prefer
    // their named instances over guessing that 400/600 or TTC face zero applies.
    for instance in font.named_instances().iter().take(MAX_NAMED_INSTANCES) {
        let Some(name) = font
            .localized_strings(instance.subfamily_name_id())
            .english_or_first()
        else {
            continue;
        };
        let name: String = name.chars().collect();
        let matches = if weight == 400.0 {
            name.eq_ignore_ascii_case("regular")
        } else {
            name.eq_ignore_ascii_case("semibold") || name.eq_ignore_ascii_case("semi bold")
        };
        if matches
            && let Some(value) = instance.user_coords().nth(axis.index())
            && range.contains(&value)
        {
            return Some(value);
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn system_candidates() -> Vec<Vec<PathBuf>> {
    vec![
        [
            "/System/Library/Fonts/SFNS.ttf",
            "/System/Library/Fonts/SFNS.ttc",
            "/System/Library/Fonts/SFNSText.ttf",
            "/System/Library/Fonts/SFNSDisplay.ttf",
        ]
        .map(PathBuf::from)
        .to_vec(),
    ]
}

#[cfg(target_os = "windows")]
fn system_candidates() -> Vec<Vec<PathBuf>> {
    let windows = std::env::var_os("WINDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let fonts = windows.join("Fonts");
    vec![vec![fonts.join("segoeui.ttf"), fonts.join("seguisb.ttf")]]
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn system_candidates() -> Vec<Vec<PathBuf>> {
    [
        &[
            "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/truetype/noto/NotoSans-SemiBold.ttf",
        ][..],
        &[
            "/usr/share/fonts/truetype/open-sans/OpenSans-Regular.ttf",
            "/usr/share/fonts/truetype/open-sans/OpenSans-Semibold.ttf",
        ],
        &["/usr/share/fonts/cantarell/Cantarell-VF.otf"],
    ]
    .into_iter()
    .map(|family| family.iter().map(PathBuf::from).collect())
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_faces_have_distinct_genuine_weights_and_cyrillic_glyphs() {
        let fonts = definitions(None);
        for (family, expected_name, weight) in [
            (FontFamily::Proportional, REGULAR_FONT, 400.0),
            (
                FontFamily::Name(SEMIBOLD_FAMILY.into()),
                SEMIBOLD_FONT,
                600.0,
            ),
        ] {
            let name = &fonts.families[&family][0];
            assert_eq!(name, expected_name);
            let data = &fonts.font_data[name];
            let font = FontRef::from_index(data.bytes(), data.index).unwrap();
            assert_eq!(font.attributes().weight.value(), weight);
            for character in "Archive Привет 0123".chars() {
                assert!(font.charmap().map(character).is_some(), "{character}");
            }
        }
        assert_ne!(regular(14.0).family, semibold(14.0).family);
    }

    #[test]
    fn malformed_and_wrong_weight_fonts_are_not_installed_as_system_faces() {
        assert!(select_face(Arc::new(b"not a font".to_vec()), 400.0).is_none());
        assert!(select_face(Arc::new(REGULAR_BYTES.to_vec()), 600.0).is_none());
        assert!(select_face(Arc::new(SEMIBOLD_BYTES.to_vec()), 400.0).is_none());
    }

    #[test]
    fn collection_face_indices_select_semibold_after_regular() {
        // Construct a TTC from our two licensed, unmodified SFNTs in memory.
        // Table offsets in TTC are absolute from the collection's beginning.
        let mut collection = b"ttcf\0\x01\0\0\0\0\0\x02".to_vec();
        collection.resize(20, 0);
        for (index, source) in [REGULAR_BYTES, SEMIBOLD_BYTES].into_iter().enumerate() {
            while !collection.len().is_multiple_of(4) {
                collection.push(0);
            }
            let start = collection.len();
            collection[12 + index * 4..16 + index * 4]
                .copy_from_slice(&(start as u32).to_be_bytes());
            collection.extend_from_slice(source);
            let tables = u16::from_be_bytes(source[4..6].try_into().unwrap());
            for table in 0..usize::from(tables) {
                let offset = start + 12 + table * 16 + 8;
                let old = u32::from_be_bytes(collection[offset..offset + 4].try_into().unwrap());
                collection[offset..offset + 4].copy_from_slice(&(old + start as u32).to_be_bytes());
            }
        }
        let bytes = Arc::new(collection);
        let regular = select_face(Arc::clone(&bytes), 400.0).unwrap();
        let semibold = select_face(bytes, 600.0).unwrap();
        assert_eq!(regular.index, 0);
        assert_eq!(semibold.index, 1);
        assert!(Arc::ptr_eq(&regular.font, &semibold.font));
        let semibold_font = FontRef::from_index(semibold.bytes(), semibold.index).unwrap();
        assert_eq!(semibold_font.attributes().weight.value(), 600.0);
    }
}
