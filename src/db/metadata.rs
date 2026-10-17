//! Searchable EXIF data. Original strings are retained alongside normalized keys.
use super::*;

#[derive(Debug, Clone, Default)]
pub struct SearchMetadata {
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub lens_make: Option<String>,
    pub lens_model: Option<String>,
    pub captured_at: Option<String>,
    pub flash: Option<bool>,
    pub white_balance: Option<String>,
    pub gps: Option<bool>,
    pub has_exif: Option<bool>,
}

pub fn normalize(value: &str) -> String {
    value
        .trim_matches('\0')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

pub fn manufacturer(value: &str) -> String {
    let value = normalize(value);
    match value.as_str() {
        "nikon corporation" | "nikon" => "nikon".into(),
        "canon inc." | "canon inc" | "canon" => "canon".into(),
        "sony corporation" | "sony" => "sony".into(),
        "olympus imaging corp." | "olympus corporation" | "olympus" => "olympus".into(),
        _ => value,
    }
}

pub fn model_key(
    make: Option<&str>,
    model: Option<&str>,
    fallback: Option<&str>,
) -> Option<String> {
    let make = make.map(manufacturer).unwrap_or_default();
    let mut model = normalize(model.or(fallback)?);
    // Older imports concatenate Make and Model, including Nikon's repeated brand.
    model = model.replace("nikon corporation", "nikon");
    while model.starts_with("nikon nikon ") {
        model = model[6..].to_owned();
    }
    if !make.is_empty() && model != make && !model.starts_with(&format!("{make} ")) {
        model = format!("{make} {model}");
    }
    (!model.is_empty()).then_some(model)
}

pub fn file_type(path: &str) -> String {
    crate::image_format::for_path(path)
        .map(|format| {
            if format.decoder == crate::image_format::DecoderKind::Raw {
                "RAW".into()
            } else {
                format.name.to_owned()
            }
        })
        .unwrap_or_else(|| {
            Path::new(path)
                .extension()
                .and_then(|x| x.to_str())
                .unwrap_or("Unknown")
                .to_uppercase()
        })
}

pub fn store(connection: &Connection, id: i64, path: &str, metadata: &PhotoMetadata) -> Result<()> {
    let m = &metadata.searchable;
    let camera = model_key(
        m.camera_make.as_deref(),
        m.camera_model.as_deref(),
        metadata.camera.as_deref(),
    );
    let lens = model_key(None, m.lens_model.as_deref(), metadata.lens.as_deref());
    connection.execute(
        "INSERT INTO photo_metadata(photo_id,camera_make,camera_model,lens_make,lens_model,
         camera_key,lens_key,camera_make_key,lens_make_key,file_type,captured_at,flash,white_balance,gps,has_exif,version)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)
         ON CONFLICT(photo_id) DO UPDATE SET camera_make=excluded.camera_make,camera_model=excluded.camera_model,
         lens_make=excluded.lens_make,lens_model=excluded.lens_model,camera_key=excluded.camera_key,lens_key=excluded.lens_key,
         camera_make_key=excluded.camera_make_key,lens_make_key=excluded.lens_make_key,file_type=excluded.file_type,
         captured_at=excluded.captured_at,flash=excluded.flash,white_balance=excluded.white_balance,gps=excluded.gps,
         has_exif=excluded.has_exif,version=excluded.version",
        params![id,m.camera_make,m.camera_model,m.lens_make,m.lens_model,camera,lens,
            m.camera_make.as_deref().map(manufacturer),m.lens_make.as_deref().map(manufacturer),file_type(path),
            m.captured_at,m.flash,m.white_balance,m.gps,m.has_exif,i64::from(m.has_exif.is_some())],
    )?;
    Ok(())
}

/// Only metadata changes: never update edit recipes, history or thumbnail fingerprints.
/// A stale worker cannot resurrect deleted rows or overwrite a newer import.
pub fn apply_backfill(
    connection: &Connection,
    id: i64,
    path: &str,
    mtime: Option<i64>,
    size: Option<i64>,
    metadata: &PhotoMetadata,
) -> Result<bool> {
    let tx = connection.unchecked_transaction()?;
    let changed = tx.execute(
        "UPDATE photos SET camera=COALESCE(?5,camera),lens=COALESCE(?6,lens),
         aperture=COALESCE(?7,aperture,0),shutter_speed=COALESCE(?8,shutter_speed,0),
         iso=COALESCE(?9,iso,0),focal_length=COALESCE(?10,focal_length,0),exposure_bias=COALESCE(?11,exposure_bias)
         WHERE id=?1 AND path=?2 AND mtime IS ?3 AND size_bytes IS ?4 AND trashed=0",
        params![id,path,mtime,size,metadata.camera,metadata.lens,metadata.aperture,metadata.shutter_speed,
            metadata.iso,metadata.focal_length,metadata.exposure_bias],
    )?;
    if changed != 0 {
        store(&tx, id, path, metadata)?;
    }
    tx.commit()?;
    Ok(changed != 0)
}

/// Seed search keys from metadata already in the catalog. No original-file I/O.
/// Version stays zero until the expanded EXIF fields have actually been read.
pub fn seed_existing(connection: &Connection) -> Result<()> {
    let tx = connection.unchecked_transaction()?;
    let rows = tx
        .prepare(
            "SELECT p.id,p.path,p.camera,p.lens FROM photos p
        WHERE NOT EXISTS (SELECT 1 FROM photo_metadata m WHERE m.photo_id=p.id)",
        )?
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (id, path, camera, lens) in rows {
        store(
            &tx,
            id,
            &path,
            &PhotoMetadata {
                camera,
                lens,
                ..Default::default()
            },
        )?;
    }
    tx.commit()?;
    Ok(())
}
