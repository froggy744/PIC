use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::Result;
use rusqlite::Connection;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecoderKind {
    TurboJpeg,
    Image,
    Heif,
    Raw,
    Svg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageFormat {
    pub id: &'static str,
    pub name: &'static str,
    pub extensions: &'static [&'static str],
    pub decoder: DecoderKind,
}

pub const FORMATS: &[ImageFormat] = &[
    ImageFormat {
        id: "jpeg",
        name: "JPEG",
        extensions: &["jpg", "jpeg"],
        decoder: DecoderKind::TurboJpeg,
    },
    ImageFormat {
        id: "png",
        name: "PNG",
        extensions: &["png"],
        decoder: DecoderKind::Image,
    },
    ImageFormat {
        id: "webp",
        name: "WebP",
        extensions: &["webp"],
        decoder: DecoderKind::Image,
    },
    ImageFormat {
        id: "gif",
        name: "GIF",
        extensions: &["gif"],
        decoder: DecoderKind::Image,
    },
    ImageFormat {
        id: "bmp",
        name: "BMP",
        extensions: &["bmp"],
        decoder: DecoderKind::Image,
    },
    ImageFormat {
        id: "tiff",
        name: "TIFF",
        extensions: &["tif", "tiff"],
        decoder: DecoderKind::Image,
    },
    ImageFormat {
        id: "heif",
        name: "HEIC / HEIF",
        extensions: &["heic", "heif"],
        decoder: DecoderKind::Heif,
    },
    ImageFormat {
        id: "avif",
        name: "AVIF",
        extensions: &["avif"],
        decoder: DecoderKind::Image,
    },
    ImageFormat {
        id: "nikon_raw",
        name: "Nikon RAW",
        extensions: &["nef", "nrw"],
        decoder: DecoderKind::Raw,
    },
    ImageFormat {
        id: "canon_raw",
        name: "Canon RAW",
        extensions: &["cr2", "cr3"],
        decoder: DecoderKind::Raw,
    },
    ImageFormat {
        id: "sony_raw",
        name: "Sony RAW",
        extensions: &["arw"],
        decoder: DecoderKind::Raw,
    },
    ImageFormat {
        id: "dng",
        name: "Digital Negative",
        extensions: &["dng"],
        decoder: DecoderKind::Raw,
    },
    ImageFormat {
        id: "fujifilm_raw",
        name: "Fujifilm RAW",
        extensions: &["raf"],
        decoder: DecoderKind::Raw,
    },
    ImageFormat {
        id: "olympus_raw",
        name: "Olympus RAW",
        extensions: &["orf"],
        decoder: DecoderKind::Raw,
    },
    ImageFormat {
        id: "panasonic_raw",
        name: "Panasonic RAW",
        extensions: &["rw2"],
        decoder: DecoderKind::Raw,
    },
    ImageFormat {
        id: "pentax_raw",
        name: "Pentax RAW",
        extensions: &["pef"],
        decoder: DecoderKind::Raw,
    },
    ImageFormat {
        id: "samsung_raw",
        name: "Samsung RAW",
        extensions: &["srw"],
        decoder: DecoderKind::Raw,
    },
    ImageFormat {
        id: "svg",
        name: "SVG",
        extensions: &["svg"],
        decoder: DecoderKind::Svg,
    },
    ImageFormat {
        id: "generic_raw",
        name: "Generic RAW",
        extensions: &["raw"],
        decoder: DecoderKind::Raw,
    },
];

pub fn all() -> &'static [ImageFormat] {
    FORMATS
}

pub fn for_path(path: impl AsRef<Path>) -> Option<&'static ImageFormat> {
    let extension = path.as_ref().extension()?.to_str()?;
    FORMATS.iter().find(|format| {
        format
            .extensions
            .iter()
            .any(|candidate| extension.eq_ignore_ascii_case(candidate))
    })
}

pub fn supported(path: impl AsRef<Path>) -> bool {
    for_path(path).is_some()
}

pub fn uses(path: impl AsRef<Path>, decoder: DecoderKind) -> bool {
    for_path(path).is_some_and(|format| format.decoder == decoder)
}

pub fn setting_key(format: &ImageFormat) -> String {
    format!("format-enabled-{}", format.id)
}

pub fn is_enabled(connection: &Connection, format: &ImageFormat) -> Result<bool> {
    Ok(crate::db::setting(connection, &setting_key(format))?.as_deref() != Some("false"))
}

pub fn set_enabled(connection: &Connection, format: &ImageFormat, enabled: bool) -> Result<()> {
    crate::db::set_setting(
        connection,
        &setting_key(format),
        if enabled { "true" } else { "false" },
    )
}

pub fn path_is_enabled(connection: &Connection, path: &str) -> bool {
    for_path(path).is_some_and(|format| is_enabled(connection, format).unwrap_or(true))
}

pub fn enabled_ids(connection: &Connection) -> Result<HashSet<&'static str>> {
    all()
        .iter()
        .filter_map(|format| match is_enabled(connection, format) {
            Ok(true) => Some(Ok(format.id)),
            Ok(false) => None,
            Err(error) => Some(Err(error)),
        })
        .collect()
}

pub fn path_is_enabled_in(enabled: &HashSet<&str>, path: &str) -> bool {
    for_path(path).is_some_and(|format| enabled.contains(format.id))
}

pub const RAW_JPEG_PAIR_MODE_SETTING_KEY: &str = "raw-jpeg-pair-mode";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawJpegPairMode {
    Both,
    PreferJpeg,
    PreferRaw,
}

impl RawJpegPairMode {
    pub fn key(self) -> &'static str {
        match self {
            Self::Both => "both",
            Self::PreferJpeg => "jpeg",
            Self::PreferRaw => "raw",
        }
    }

    pub fn from_key(value: &str) -> Self {
        match value {
            "jpeg" => Self::PreferJpeg,
            "raw" => Self::PreferRaw,
            _ => Self::Both,
        }
    }
}

pub fn raw_jpeg_pair_mode(connection: &Connection) -> RawJpegPairMode {
    crate::db::setting(connection, RAW_JPEG_PAIR_MODE_SETTING_KEY)
        .ok()
        .flatten()
        .as_deref()
        .map(RawJpegPairMode::from_key)
        .unwrap_or(RawJpegPairMode::Both)
}

pub fn set_raw_jpeg_pair_mode(connection: &Connection, mode: RawJpegPairMode) -> Result<()> {
    crate::db::set_setting(connection, RAW_JPEG_PAIR_MODE_SETTING_KEY, mode.key())
}

/// Whether `photo` is one half of a RAW+JPEG pair already in the catalogue:
/// a photo of the other kind (RAW or JPEG) in the same folder whose file stem
/// matches ignoring ASCII letter case, not in the Trash, its format shown.
/// The pairing rule of [`retain_raw_jpeg_pair_mode`], asked for one photo
/// while a scan runs.
///
/// Reads only the file's own siblings through the `path` UNIQUE index: one
/// range `dir/stem.` to `dir/stem/` per letter-case spelling of the stem.
/// The index compares case, so every spelling is tried for a stem of up to
/// [`TWIN_CASE_LETTERS`] letters (camera names such as `DSC_1240`); a longer
/// stem tries it as written, in lower case and in upper case, so a twin
/// spelled in some other mixed case is not found here (the full list still
/// pairs it).
pub fn has_raw_jpeg_twin(connection: &Connection, photo: &crate::db::Photo) -> bool {
    let Some(format) = for_path(&photo.path) else {
        return false;
    };
    let raw = format.decoder == DecoderKind::Raw;
    if !raw && format.id != "jpeg" {
        return false;
    }
    let Some(dot) = photo.path.rfind('.') else {
        return false;
    };
    let name_start = photo.path.rfind('/').map_or(0, |slash| slash + 1);
    if name_start > dot {
        return false;
    }
    let directory = &photo.path[..name_start];
    let stem = &photo.path[name_start..dot];
    let Ok(mut statement) = connection.prepare_cached(
        "SELECT path FROM photos
         WHERE path >= ?1 AND path < ?2 AND id != ?3 AND +folder_id IS ?4 AND trashed = 0",
    ) else {
        return false;
    };
    for spelling in stem_case_spellings(stem) {
        let prefix = format!("{directory}{spelling}.");
        let upper = format!("{directory}{spelling}/");
        let Ok(rows) = statement.query_map(
            rusqlite::params![prefix, upper, photo.id, photo.folder_id],
            |row| row.get::<_, String>(0),
        ) else {
            continue;
        };
        let siblings: Vec<String> = rows.flatten().collect();
        let found = siblings.into_iter().any(|path| {
            // Same stem only: `dir/a.jpg` must not pair with `dir/a.b.nef`.
            let extension = &path[prefix.len()..];
            if extension.contains('.') || extension.contains('/') {
                return false;
            }
            for_path(&path).is_some_and(|twin| {
                let twin_raw = twin.decoder == DecoderKind::Raw;
                (if raw { twin.id == "jpeg" } else { twin_raw })
                    && path_is_enabled(connection, &path)
            })
        });
        if found {
            return true;
        }
    }
    false
}

/// Stems with at most this many ASCII letters are looked up in every
/// letter-case spelling (2^n index ranges).
pub const TWIN_CASE_LETTERS: u32 = 6;

/// The spellings of `stem` a twin lookup tries, without repeats: every
/// letter-case combination up to [`TWIN_CASE_LETTERS`] letters, otherwise as
/// written, lower case and upper case.
fn stem_case_spellings(stem: &str) -> Vec<String> {
    let letters: Vec<usize> = stem
        .char_indices()
        .filter(|(_, c)| c.is_ascii_alphabetic())
        .map(|(index, _)| index)
        .collect();
    let mut spellings: Vec<String> = Vec::new();
    let mut push = |spelling: String| {
        if !spellings.contains(&spelling) {
            spellings.push(spelling);
        }
    };
    push(stem.to_owned());
    if letters.len() as u32 <= TWIN_CASE_LETTERS {
        for mask in 0u32..(1 << letters.len()) {
            let mut bytes = stem.as_bytes().to_vec();
            for (bit, &index) in letters.iter().enumerate() {
                bytes[index] = if mask & (1 << bit) != 0 {
                    bytes[index].to_ascii_uppercase()
                } else {
                    bytes[index].to_ascii_lowercase()
                };
            }
            push(String::from_utf8(bytes).unwrap_or_default());
        }
    } else {
        push(stem.to_ascii_lowercase());
        push(stem.to_ascii_uppercase());
    }
    spellings
}

/// Collapse only true RAW+JPEG capture pairs. JPEG-only and RAW-only photos
/// remain visible in every mode.
pub fn retain_raw_jpeg_pair_mode(photos: &mut Vec<crate::db::Photo>, mode: RawJpegPairMode) {
    if mode == RawJpegPairMode::Both {
        return;
    }

    #[derive(Default, Clone, Copy)]
    struct PairState {
        raw: bool,
        jpeg: bool,
    }

    let mut states = HashMap::<(String, String), PairState>::new();
    for photo in photos.iter() {
        let Some(format) = for_path(&photo.path) else {
            continue;
        };
        if format.decoder != DecoderKind::Raw && format.id != "jpeg" {
            continue;
        }
        let filename = crate::source::filename(&photo.path);
        let stem = Path::new(&filename)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or(&filename)
            .to_ascii_lowercase();
        let folder = photo
            .folder_id
            .map(|id| format!("id:{id}"))
            .or_else(|| photo.folder_path.clone())
            .unwrap_or_default();
        let state = states.entry((folder, stem)).or_default();
        if format.decoder == DecoderKind::Raw {
            state.raw = true;
        } else if format.id == "jpeg" {
            state.jpeg = true;
        }
    }

    photos.retain(|photo| {
        let Some(format) = for_path(&photo.path) else {
            return true;
        };
        if format.decoder != DecoderKind::Raw && format.id != "jpeg" {
            return true;
        }
        let filename = crate::source::filename(&photo.path);
        let stem = Path::new(&filename)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or(&filename)
            .to_ascii_lowercase();
        let folder = photo
            .folder_id
            .map(|id| format!("id:{id}"))
            .or_else(|| photo.folder_path.clone())
            .unwrap_or_default();
        let paired = states
            .get(&(folder, stem))
            .is_some_and(|state| state.raw && state.jpeg);
        if !paired {
            return true;
        }
        match mode {
            RawJpegPairMode::Both => true,
            RawJpegPairMode::PreferJpeg => format.id == "jpeg",
            RawJpegPairMode::PreferRaw => format.decoder == DecoderKind::Raw,
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_recognizes_all_declared_extensions_case_insensitively() {
        for format in all() {
            for extension in format.extensions {
                assert_eq!(for_path(format!("photo.{extension}")), Some(format));
                assert_eq!(
                    for_path(format!("photo.{}", extension.to_ascii_uppercase())),
                    Some(format)
                );
            }
        }
        assert_eq!(for_path("photo.txt"), None);
    }

    #[test]
    fn twin_lookups_try_each_letter_case_spelling_of_short_stems() {
        let short = stem_case_spellings("Dsc_1");
        assert_eq!(short.len(), 8);
        assert!(short.contains(&"dSc_1".to_owned()) && short.contains(&"DSC_1".to_owned()));
        assert_eq!(stem_case_spellings("1240"), ["1240"]);
        assert_eq!(
            stem_case_spellings("Holiday_Beach"),
            ["Holiday_Beach", "holiday_beach", "HOLIDAY_BEACH"]
        );
        assert_eq!(stem_case_spellings("abcdef").len(), 64);
    }

    #[test]
    fn a_raw_jpeg_twin_is_the_other_kind_with_the_same_stem_in_the_same_folder() {
        let directory = std::env::temp_dir().join(format!(
            "pic-raw-jpeg-twin-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let connection = crate::db::open(&directory.join("library.db")).unwrap();
        for (id, path, folder, trashed) in [
            (1, "/lib/a.NEF", 1, 0),
            (2, "/lib/a.jpg", 1, 0),
            (3, "/lib/solo.nef", 1, 0),
            (4, "/lib/solo2.JPG", 1, 0),
            (5, "/lib/sub/a.jpg", 2, 0),
            (6, "/lib/b.nef", 1, 0),
            (7, "/lib/b.jpg", 1, 1),
            (8, "/lib/c.b.nef", 1, 0),
            (9, "/lib/c.jpg", 1, 0),
            (10, "/lib/d.png", 1, 0),
            (11, "/lib/d.jpg", 1, 0),
            // Stems differing only in letter case pair, as in the full list.
            (12, "/lib/IMG_1.NEF", 1, 0),
            (13, "/lib/img_1.jpg", 1, 0),
            (14, "/lib/Dsc_9.jpg", 1, 0),
            (15, "/lib/dSC_9.nef", 1, 0),
            (16, "/lib/HOLIDAY_BEACH.NEF", 1, 0),
            (17, "/lib/holiday_beach.JPG", 1, 0),
        ] {
            connection
                .execute(
                    "INSERT INTO photos(id, path, folder_id, trashed) VALUES (?1, ?2, ?3, ?4)",
                    rusqlite::params![id, path, folder, trashed],
                )
                .unwrap();
        }
        let photo = |id: i64, path: &str, folder: i64| crate::db::Photo {
            id,
            path: path.into(),
            folder_id: Some(folder),
            ..crate::db::Photo::default()
        };
        // RAW-first and JPEG-first: either half finds the other.
        assert!(has_raw_jpeg_twin(&connection, &photo(1, "/lib/a.NEF", 1)));
        assert!(has_raw_jpeg_twin(&connection, &photo(2, "/lib/a.jpg", 1)));
        // Standalone RAW and JPEG.
        assert!(!has_raw_jpeg_twin(
            &connection,
            &photo(3, "/lib/solo.nef", 1)
        ));
        assert!(!has_raw_jpeg_twin(
            &connection,
            &photo(4, "/lib/solo2.JPG", 1)
        ));
        // The same name in another folder is not a twin.
        assert!(!has_raw_jpeg_twin(
            &connection,
            &photo(5, "/lib/sub/a.jpg", 2)
        ));
        // A twin in the Trash does not pair.
        assert!(!has_raw_jpeg_twin(&connection, &photo(6, "/lib/b.nef", 1)));
        // A longer stem sharing the prefix is not a twin; PNG never pairs.
        assert!(!has_raw_jpeg_twin(&connection, &photo(9, "/lib/c.jpg", 1)));
        assert!(!has_raw_jpeg_twin(&connection, &photo(11, "/lib/d.jpg", 1)));
        assert!(!has_raw_jpeg_twin(&connection, &photo(10, "/lib/d.png", 1)));
        // Letter case: both arrival orders, mixed case, a long stem.
        assert!(has_raw_jpeg_twin(
            &connection,
            &photo(12, "/lib/IMG_1.NEF", 1)
        ));
        assert!(has_raw_jpeg_twin(
            &connection,
            &photo(13, "/lib/img_1.jpg", 1)
        ));
        assert!(has_raw_jpeg_twin(
            &connection,
            &photo(14, "/lib/Dsc_9.jpg", 1)
        ));
        assert!(has_raw_jpeg_twin(
            &connection,
            &photo(15, "/lib/dSC_9.nef", 1)
        ));
        assert!(has_raw_jpeg_twin(
            &connection,
            &photo(16, "/lib/HOLIDAY_BEACH.NEF", 1)
        ));
        assert!(has_raw_jpeg_twin(
            &connection,
            &photo(17, "/lib/holiday_beach.JPG", 1)
        ));
        // A hidden RAW format does not pair.
        set_enabled(&connection, for_path("a.nef").unwrap(), false).unwrap();
        assert!(!has_raw_jpeg_twin(&connection, &photo(2, "/lib/a.jpg", 1)));

        // The lookup reads the path index, not the whole table or folder.
        let plan: Vec<String> = connection
            .prepare(
                "EXPLAIN QUERY PLAN SELECT path FROM photos
                 WHERE path >= ?1 AND path < ?2 AND id != ?3 AND +folder_id IS ?4 AND trashed = 0",
            )
            .unwrap()
            .query_map(rusqlite::params!["/lib/a.", "/lib/a/", 2, 1], |row| {
                row.get::<_, String>(3)
            })
            .unwrap()
            .flatten()
            .collect();
        assert!(
            plan.iter()
                .any(|step| step.contains("sqlite_autoindex_photos") && step.contains("path>?")),
            "{plan:?}"
        );
        drop(connection);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn format_visibility_defaults_on_and_persists_off() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE settings (
                   key TEXT PRIMARY KEY,
                   value TEXT NOT NULL
                 );",
            )
            .unwrap();
        let heif = for_path("photo.heic").unwrap();
        assert!(is_enabled(&connection, heif).unwrap());
        set_enabled(&connection, heif, false).unwrap();
        assert!(!path_is_enabled(&connection, "photo.HEIF"));
        assert!(!crate::db::setting(&connection, &setting_key(heif))
            .unwrap()
            .is_none());
    }
}
