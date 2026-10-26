use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension};

use super::{model::CollageProject, render::ExportOptions};

pub(super) fn export_project(
    connection: &Connection,
    project: &CollageProject,
    photo_id: Option<i64>,
    path: &Path,
    options: &ExportOptions,
    confirmed_replace: bool,
) -> Result<i64> {
    let target_photo_id = validate_destination(connection, photo_id, path, confirmed_replace)?;
    // Stage beside the destination, keeping publication on the same filesystem.
    // A failed render/database write must preserve the last successful export.
    let directory = path
        .parent()
        .context("Export path has no parent")?
        .join(format!(
            ".pic-collage-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
    std::fs::create_dir(&directory)?;
    let staged = directory.join("rendered");
    let backup = directory.join("previous");
    let result = (|| -> Result<i64> {
        super::render::export(project, &staged, options)?;
        let file = std::fs::metadata(&staged)?;
        // The staging filename has no extension; inspect its contents.
        let dimensions = image::ImageReader::open(&staged)?
            .with_guessed_format()?
            .into_dimensions()?;
        let metadata = crate::db::PhotoMetadata {
            width: Some(dimensions.0 as i64),
            height: Some(dimensions.1 as i64),
            size_bytes: Some(file.len() as i64),
            mtime: file
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|time| time.as_secs() as i64),
            ..Default::default()
        };
        anyhow::ensure!(
            validate_destination(connection, photo_id, path, confirmed_replace)? == target_photo_id,
            "Destination library item changed while exporting"
        );
        let had_previous = path.exists();
        if had_previous {
            std::fs::rename(path, &backup)?;
        }
        // Reserve a new path without overwriting a file that appeared meanwhile.
        // Then rename the staged file over our reservation (also works on FAT).
        let publish = (|| -> std::io::Result<()> {
            let reservation = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)?;
            drop(reservation);
            if let Err(error) = std::fs::rename(&staged, path) {
                let _ = std::fs::remove_file(path);
                return Err(error);
            }
            Ok(())
        })();
        if let Err(error) = publish {
            if had_previous {
                std::fs::rename(&backup, path)?;
            }
            return Err(error.into());
        }
        let draft = super::model::draft_to_json(project);
        let saved = if target_photo_id != photo_id {
            crate::db::replace_photo_with_collage(
                connection,
                target_photo_id.expect("a changed target must have a library photo"),
                path,
                &metadata,
                &draft,
            )
        } else {
            crate::db::save_collage_project(connection, photo_id, path, &metadata, &draft)
        };
        if saved.is_err() {
            std::fs::remove_file(path)?;
            if had_previous {
                std::fs::rename(&backup, path)?;
            }
        }
        saved
    })();
    if result.is_err() && backup.exists() {
        return result.with_context(|| format!("Previous export retained at {}", backup.display()));
    }
    if let Err(error) = std::fs::remove_dir_all(&directory) {
        eprintln!(
            "Could not remove collage staging directory {}: {error}",
            directory.display()
        );
    }
    result
}

fn validate_destination(
    connection: &Connection,
    photo_id: Option<i64>,
    path: &Path,
    confirmed_replace: bool,
) -> Result<Option<i64>> {
    let owner: Option<i64> = connection
        .query_row(
            "SELECT id FROM photos WHERE path=?1",
            [path.to_string_lossy().as_ref()],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(id) = photo_id {
        anyhow::ensure!(
            crate::db::collage_project(connection, id)?.is_some(),
            "Saved collage no longer exists"
        );
    }
    anyhow::ensure!(
        !path.exists() || confirmed_replace,
        "This file already exists. Confirm replacing it in the Save dialog."
    );
    if owner.is_some() && owner != photo_id {
        anyhow::ensure!(
            !path.exists() || confirmed_replace,
            "Confirm replacing this library item in the Save dialog."
        );
        return Ok(owner);
    }
    Ok(photo_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_collage_export_preserves_unrelated_original_and_failed_save() {
        let directory = std::env::temp_dir().join(format!(
            "pic-collage-history-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("original.jpg");
        std::fs::write(&path, b"precious original").unwrap();
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(crate::db::SCHEMA).unwrap();
        connection
            .execute(
                "INSERT INTO photos(path) VALUES (?1)",
                [path.to_string_lossy().as_ref()],
            )
            .unwrap();
        let project = CollageProject::new(Vec::new());
        let options = ExportOptions {
            max_edge: 32,
            format: super::super::render::ExportFormat::Png,
            jpeg_quality: 90,
        };
        assert!(export_project(&connection, &project, None, &path, &options, false).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"precious original");

        let output = directory.join("collage.png");
        let id = export_project(&connection, &project, None, &output, &options, false).unwrap();
        let previous = std::fs::read(&output).unwrap();
        let mut modified = project.clone();
        modified.background = super::super::model::Background::Black;
        connection.execute_batch("CREATE TRIGGER fail_save BEFORE UPDATE ON collage_projects BEGIN SELECT RAISE(ABORT,'test failure'); END;").unwrap();
        assert!(export_project(&connection, &modified, Some(id), &output, &options, true).is_err());
        assert_eq!(std::fs::read(&output).unwrap(), previous);
        assert_eq!(crate::db::history_photos(&connection).unwrap().len(), 1);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn confirmed_save_replaces_library_item_and_keeps_its_identity() {
        let directory = std::env::temp_dir().join(format!(
            "pic-collage-replace-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("existing.jpg");
        std::fs::write(&path, b"existing library photo").unwrap();
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(crate::db::SCHEMA).unwrap();
        connection
            .execute(
                "INSERT INTO photos(path,camera) VALUES (?1,'Existing camera')",
                [path.to_string_lossy().as_ref()],
            )
            .unwrap();
        let existing_id = connection.last_insert_rowid();
        let project = CollageProject::new(Vec::new());
        let options = ExportOptions {
            max_edge: 32,
            format: super::super::render::ExportFormat::Png,
            jpeg_quality: 90,
        };

        let saved_id = export_project(&connection, &project, None, &path, &options, true).unwrap();

        assert_eq!(saved_id, existing_id);
        assert_ne!(std::fs::read(&path).unwrap(), b"existing library photo");
        assert!(crate::db::collage_project(&connection, existing_id)
            .unwrap()
            .is_some());
        let saved_photo = crate::db::photo(&connection, existing_id).unwrap().unwrap();
        assert!(saved_photo.camera.is_none());
        assert_eq!(saved_photo.width, Some(32));
        std::fs::remove_dir_all(directory).unwrap();
    }
}
