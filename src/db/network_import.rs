/// Durable state belongs to the catalog, so quitting PIC cannot lose queued
/// imports or metadata work. No function in this file accesses a source file.
#[derive(Debug, Clone, Copy)]
pub struct NetworkJob {
    pub generation: i64,
    pub recovered: bool,
}

pub struct NetworkCatalogPhoto {
    pub photo: Photo,
    pub newly_discovered: bool,
    pub metadata_delta: i64,
    pub preview_delta: i64,
}

#[derive(Debug, Clone)]
pub struct NetworkWork {
    pub path: String,
    pub root: String,
    pub generation: i64,
    pub mtime: Option<i64>,
    pub size: Option<i64>,
    pub metadata_pending: bool,
    pub thumbnail_pending: bool,
    pub metadata_state: String,
    pub thumbnail_state: String,
}

pub fn register_network_import(connection: &Connection, root: &str) -> Result<()> {
    anyhow::ensure!(
        root.starts_with("nfs://") || root.starts_with("smb://"),
        "not a direct network import root"
    );
    let transaction = connection.unchecked_transaction()?;
    // Re-adding a completed root is a new discovery, while re-adding an
    // interrupted root resumes its acknowledged directory checkpoints.
    for table in [
        "network_import_dirs",
        "network_import_seen",
        "network_import_pairs",
    ] {
        transaction.execute(
            &format!(
                "DELETE FROM {table} WHERE root=?1 AND EXISTS(
            SELECT 1 FROM network_import_jobs WHERE root=?1 AND status='complete')"
            ),
            [root],
        )?;
    }
    transaction.execute(
        "INSERT INTO network_import_jobs(root, status) VALUES (?1, 'queued')
         ON CONFLICT(root) DO UPDATE SET status='queued'",
        [root],
    )?;
    transaction.commit()?;
    Ok(())
}

pub fn resumable_network_imports(connection: &Connection) -> Result<Vec<String>> {
    Ok(connection.prepare(
        "SELECT root FROM network_import_jobs WHERE status IN ('queued','running','waiting') ORDER BY rowid",
    )?.query_map([], |row| row.get(0))?.collect::<rusqlite::Result<_>>()?)
}

pub fn pause_network_import(connection: &Connection, root: &str) -> Result<()> {
    connection.execute(
        "UPDATE network_import_jobs SET status='paused' WHERE root=?1 AND status != 'complete'",
        [root],
    )?;
    Ok(())
}

pub fn start_network_job(connection: &Connection, root: &str, resume: bool) -> Result<NetworkJob> {
    let transaction = connection.unchecked_transaction()?;
    let previous: Option<(i64, String)> = transaction
        .query_row(
            "SELECT generation, status FROM network_import_jobs WHERE root=?1",
            [root],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let checkpointed: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM network_import_dirs WHERE root=?1)",
        [root],
        |row| row.get(0),
    )?;
    let recovered = resume
        && checkpointed
        && previous
            .as_ref()
            .is_some_and(|(_, status)| status != "complete");
    let generation = previous.map_or(1, |(value, _)| if recovered { value } else { value + 1 });
    transaction.execute(
        "INSERT INTO network_import_jobs(root,generation,status,recovered,added) VALUES (?1,?2,'running',?3,0)
         ON CONFLICT(root) DO UPDATE SET generation=?2,status='running',recovered=?3,added=CASE WHEN ?3 THEN added ELSE 0 END",
        params![root, generation, recovered],
    )?;
    if recovered {
        transaction.execute(
            "UPDATE network_import_dirs SET state='pending' WHERE root=?1 AND state='failed'",
            [root],
        )?;
    } else {
        for table in [
            "network_import_dirs",
            "network_import_seen",
            "network_import_pairs",
        ] {
            transaction.execute(&format!("DELETE FROM {table} WHERE root=?1"), [root])?;
        }
        transaction.execute(
            "INSERT INTO network_import_dirs(root,path) VALUES (?1,?1)",
            [root],
        )?;
        // Pending work from an interrupted traversal remains durable even if
        // an explicit Refresh starts a fresh traversal.
        transaction.execute(
            "UPDATE network_photo_work SET generation=?2 WHERE root=?1",
            params![root, generation],
        )?;
        transaction.execute("UPDATE network_photo_work SET metadata_state=CASE WHEN metadata_state='failed' THEN 'pending' ELSE metadata_state END,
            thumbnail_state=CASE WHEN thumbnail_state='failed' THEN 'pending' ELSE thumbnail_state END WHERE root=?1",[root])?;
    }
    if recovered {
        transaction.execute("UPDATE network_photo_work SET metadata_state=CASE WHEN metadata_state='failed' THEN 'pending' ELSE metadata_state END,
            thumbnail_state=CASE WHEN thumbnail_state='failed' THEN 'pending' ELSE thumbnail_state END WHERE root=?1",[root])?;
    }
    transaction.commit()?;
    Ok(NetworkJob {
        generation,
        recovered,
    })
}

pub fn next_network_directory(
    connection: &Connection,
    root: &str,
) -> Result<Option<(String, Option<String>)>> {
    Ok(connection.query_row(
        "SELECT path,parent FROM network_import_dirs WHERE root=?1 AND state='pending' ORDER BY rowid LIMIT 1",
        [root], |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional()?)
}

pub fn catalog_network_directory(
    connection: &Connection,
    root: &str,
    path: &str,
    parent: Option<&str>,
) -> Result<i64> {
    let id = if let Some(parent) = parent {
        let parent_id =
            connection.query_row("SELECT id FROM folders WHERE path=?1", [parent], |row| {
                row.get(0)
            })?;
        insert_discovered_folder(connection, path, parent_id)?
    } else {
        connection.query_row("SELECT id FROM folders WHERE path=?1", [root], |row| {
            row.get(0)
        })?
    };
    connection.execute(
        "INSERT OR IGNORE INTO network_import_dirs(root,path,parent) VALUES (?1,?2,?3)",
        params![root, path, parent],
    )?;
    connection.execute(
        "INSERT OR IGNORE INTO network_import_seen(root,path,kind) VALUES (?1,?2,'folder')",
        params![root, path],
    )?;
    Ok(id)
}

pub fn catalog_network_photo(
    connection: &Connection,
    root: &str,
    generation: i64,
    path: &str,
    folder_id: i64,
    mtime: Option<i64>,
    size: Option<i64>,
) -> Result<NetworkCatalogPhoto> {
    let previous_ready: Option<(i64, i64)> = connection
        .query_row(
            "SELECT metadata_state='ready',thumbnail_state='ready' FROM network_photo_work w
         WHERE path=?1 AND root=?2 AND generation=?3 AND EXISTS(
         SELECT 1 FROM network_import_seen s WHERE s.root=w.root AND s.path=w.path)",
            params![path, root, generation],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let existing: Option<(i64, Option<i64>, Option<i64>, bool)> = connection.query_row(
        "SELECT id,mtime,size_bytes,(width>0 AND height>0 AND aperture IS NOT NULL AND iso IS NOT NULL) FROM photos WHERE path=?1",
        [path], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get::<_,Option<bool>>(3)?.unwrap_or(false))),
    ).optional()?;
    let newly_discovered = existing.is_none();
    let unchanged = existing
        .as_ref()
        .is_some_and(|(_, old_mtime, old_size, _)| {
            mtime.is_some() && size.is_some() && (*old_mtime, *old_size) == (mtime, size)
        });
    let legacy_ready = unchanged && existing.as_ref().is_some_and(|(_, _, _, ready)| *ready);
    connection.execute(
        "INSERT INTO photos(path,folder_id,mtime,size_bytes,taken_at,added_at)
         VALUES (?1,?2,?3,?4,CASE WHEN ?3 IS NULL THEN NULL ELSE strftime('%Y-%m-%dT%H:%M:%SZ',?3,'unixepoch') END,CAST(strftime('%s','now') AS INTEGER))
         ON CONFLICT(path) DO UPDATE SET folder_id=?2,mtime=?3,size_bytes=?4",
        params![path,folder_id,mtime,size],
    )?;
    connection.execute(
        "INSERT INTO network_photo_work(path,root,generation,mtime,size,metadata_state,thumbnail_state)
         VALUES (?1,?2,?3,?4,?5,?6,?7)
         ON CONFLICT(path) DO UPDATE SET root=?2,generation=?3,mtime=?4,size=?5,
           metadata_state=CASE WHEN network_photo_work.mtime IS ?4 AND network_photo_work.size IS ?5 THEN network_photo_work.metadata_state ELSE 'pending' END,
           thumbnail_state=CASE WHEN network_photo_work.mtime IS ?4 AND network_photo_work.size IS ?5 THEN network_photo_work.thumbnail_state ELSE 'pending' END",
        params![path,root,generation,mtime,size,if legacy_ready {"ready"} else {"pending"},"pending"],
    )?;
    connection.execute(
        "INSERT OR IGNORE INTO network_import_seen(root,path,kind) VALUES (?1,?2,'photo')",
        params![root, path],
    )?;
    if newly_discovered {
        connection.execute(
            "UPDATE network_import_jobs SET added=added+1 WHERE root=?1",
            [root],
        )?;
    }
    let photo = photo(
        connection,
        connection.query_row("SELECT id FROM photos WHERE path=?1", [path], |row| {
            row.get(0)
        })?,
    )?
    .context("network catalog row disappeared")?;
    let (metadata_ready,preview_ready) = connection.query_row(
        "SELECT metadata_state='ready',thumbnail_state='ready'
         FROM network_photo_work WHERE path=?1", [path], |row| Ok((row.get::<_,i64>(0)?,row.get::<_,i64>(1)?)),
    )?;
    let (previous_metadata, previous_preview) = previous_ready.unwrap_or_default();
    Ok(NetworkCatalogPhoto {
        photo,
        newly_discovered,
        metadata_delta: metadata_ready - previous_metadata,
        preview_delta: preview_ready - previous_preview,
    })
}

pub fn next_network_work(
    connection: &Connection,
    root: &str,
    generation: i64,
) -> Result<Option<NetworkWork>> {
    Ok(connection.query_row(
        "SELECT path,mtime,size,metadata_state,thumbnail_state FROM network_photo_work w
         WHERE root=?1 AND generation=?2 AND (metadata_state='pending' OR thumbnail_state='pending')
         AND EXISTS(SELECT 1 FROM network_import_seen s WHERE s.root=w.root AND s.path=w.path AND s.kind='photo') ORDER BY rowid LIMIT 1",
        params![root,generation], |row| {
            let metadata_state:String=row.get(3)?;
            let thumbnail_state:String=row.get(4)?;
            Ok(NetworkWork {
            path: row.get(0)?, root: root.into(), generation, mtime: row.get(1)?,size: row.get(2)?,
            metadata_pending: metadata_state=="pending",thumbnail_pending: thumbnail_state=="pending",
            metadata_state,thumbnail_state,
        })},
    ).optional()?)
}

/// Apply only results for the current source version and current job. Updating
/// the existing row (rather than upserting) cannot resurrect a deleted photo.
pub fn finish_network_work(
    connection: &Connection,
    work: &NetworkWork,
    metadata: Option<&PhotoMetadata>,
    metadata_state: &str,
    thumbnail_state: &str,
) -> Result<Option<Photo>> {
    let valid: Option<i64> = connection.query_row(
        "SELECT p.id FROM photos p JOIN network_photo_work w ON w.path=p.path
         JOIN network_import_jobs j ON j.root=w.root
         WHERE w.path=?1 AND w.root=?2 AND w.generation=?3 AND j.generation=?3
         AND j.status='running' AND w.mtime IS ?4 AND w.size IS ?5 AND p.mtime IS ?4 AND p.size_bytes IS ?5",
        params![work.path,work.root,work.generation,work.mtime,work.size], |row| row.get(0),
    ).optional()?;
    let Some(id) = valid else {
        return Ok(None);
    };
    if let Some(metadata) = metadata {
        connection.execute(
            "UPDATE photos SET taken_at=?2,camera=?3,aperture=?4,lens=?5,shutter_speed=?6,iso=?7,
             focal_length=?8,exposure_bias=?9,width=?10,height=?11 WHERE id=?1",
            params![
                id,
                metadata.taken_at,
                metadata.camera,
                metadata.aperture.or(Some(0.0)),
                metadata.lens.as_deref().unwrap_or(""),
                metadata.shutter_speed.unwrap_or_default(),
                metadata.iso.unwrap_or_default(),
                metadata.focal_length.unwrap_or_default(),
                metadata.exposure_bias,
                metadata.width,
                metadata.height
            ],
        )?;
    }
    connection.execute(
        "UPDATE network_photo_work SET metadata_state=?2,thumbnail_state=?3 WHERE path=?1",
        params![work.path, metadata_state, thumbnail_state],
    )?;
    photo(connection, id)
}

#[cfg(test)]
mod network_import_tests {
    use super::*;

    fn catalog() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.pragma_update(None, "foreign_keys", "ON").unwrap();
        db.execute_batch(SCHEMA).unwrap();
        db
    }

    #[test]
    fn network_discovery_preserves_existing_metadata_and_edits() {
        let db = catalog();
        let root = "nfs://nas/photos";
        let folder = mark_import_root(&db, root).unwrap();
        let path = "nfs://nas/photos/a.jpg";
        let id = upsert_photo(
            &db,
            Path::new(path),
            Some(folder),
            &PhotoMetadata {
                camera: Some("Canon".into()),
                width: Some(6000),
                height: Some(4000),
                mtime: Some(10),
                size_bytes: Some(100),
                ..Default::default()
            },
        )
        .unwrap();
        db.execute(
            "UPDATE photos SET rating=5, edit_recipe='saved' WHERE id=?1",
            [id],
        )
        .unwrap();
        let job = start_network_job(&db, root, false).unwrap();
        let record =
            catalog_network_photo(&db, root, job.generation, path, folder, Some(10), Some(100))
                .unwrap();
        assert!(!record.newly_discovered);
        assert_eq!(
            next_network_work(&db, root, job.generation)
                .unwrap()
                .unwrap()
                .metadata_state,
            "ready"
        );
        let photo = photo(&db, id).unwrap().unwrap();
        assert_eq!(photo.camera.as_deref(), Some("Canon"));
        assert_eq!(photo.width, Some(6000));
        assert_eq!(photo.rating, 5);
        assert_eq!(photo.edit_recipe, "saved");
    }

    #[test]
    fn pending_network_rows_are_visible_and_old_enrichment_cannot_overwrite_changes() {
        let db = catalog();
        let root = "nfs://nas/photos";
        let folder = mark_import_root(&db, root).unwrap();
        let job = start_network_job(&db, root, false).unwrap();
        let path = "nfs://nas/photos/a.jpg";
        let row =
            catalog_network_photo(&db, root, job.generation, path, folder, Some(10), Some(100))
                .unwrap();
        assert!(row.newly_discovered);
        assert_eq!(photos(&db, None, false, None).unwrap().len(), 1);
        let old = next_network_work(&db, root, job.generation)
            .unwrap()
            .unwrap();
        catalog_network_photo(&db, root, job.generation, path, folder, Some(20), Some(200))
            .unwrap();
        let metadata = PhotoMetadata {
            camera: Some("stale".into()),
            mtime: Some(10),
            size_bytes: Some(100),
            ..Default::default()
        };
        assert!(
            finish_network_work(&db, &old, Some(&metadata), "ready", "ready")
                .unwrap()
                .is_none()
        );
        let photo = photo(&db, row.photo.id).unwrap().unwrap();
        assert_eq!(photo.mtime, Some(20));
        assert_eq!(photo.camera, None);
        assert!(next_network_work(&db, root, job.generation)
            .unwrap()
            .is_some());
    }

    #[test]
    fn unfinished_jobs_resume_but_explicit_stop_and_deleted_roots_do_not() {
        let db = catalog();
        let root = "smb://nas/photos";
        let folder = mark_import_root(&db, root).unwrap();
        register_network_import(&db, root).unwrap();
        assert_eq!(resumable_network_imports(&db).unwrap(), vec![root]);
        let initial = start_network_job(&db, root, false).unwrap();
        let resumed = start_network_job(&db, root, true).unwrap();
        assert_eq!(initial.generation, resumed.generation);
        assert!(resumed.recovered);
        pause_network_import(&db, root).unwrap();
        assert!(resumable_network_imports(&db).unwrap().is_empty());
        start_network_job(&db, root, true).unwrap();
        remove_folder(&db, folder).unwrap();
        assert!(resumable_network_imports(&db).unwrap().is_empty());
    }

    #[test]
    fn adding_a_completed_root_starts_fresh_discovery() {
        let db = catalog();
        let root = "nfs://nas/photos";
        mark_import_root(&db, root).unwrap();
        let initial = start_network_job(&db, root, false).unwrap();
        db.execute("UPDATE network_import_dirs SET state='done'", [])
            .unwrap();
        db.execute("UPDATE network_import_jobs SET status='complete'", [])
            .unwrap();
        register_network_import(&db, root).unwrap();
        let next = start_network_job(&db, root, true).unwrap();
        assert!(
            !next.recovered,
            "a completed import must not hide newly added source files"
        );
        assert!(next.generation > initial.generation);
        assert_eq!(next_network_directory(&db, root).unwrap().unwrap().0, root);
    }

    #[test]
    fn fresh_refresh_only_enriches_files_seen_in_this_traversal() {
        let db = catalog();
        let root = "nfs://nas/photos";
        let folder = mark_import_root(&db, root).unwrap();
        let job = start_network_job(&db, root, false).unwrap();
        catalog_network_photo(
            &db,
            root,
            job.generation,
            "nfs://nas/photos/removed.jpg",
            folder,
            Some(10),
            Some(100),
        )
        .unwrap();
        let next = start_network_job(&db, root, false).unwrap();
        assert!(next_network_work(&db, root, next.generation)
            .unwrap()
            .is_none());
    }
    #[test]
    fn recovery_retries_previously_failed_enrichment() {
        let db = catalog();
        let root = "nfs://nas/photos";
        let folder = mark_import_root(&db, root).unwrap();
        let job = start_network_job(&db, root, false).unwrap();
        catalog_network_photo(
            &db,
            root,
            job.generation,
            "nfs://nas/photos/a.jpg",
            folder,
            Some(10),
            Some(100),
        )
        .unwrap();
        db.execute(
            "UPDATE network_photo_work SET metadata_state='failed',thumbnail_state='failed'",
            [],
        )
        .unwrap();
        db.execute("UPDATE network_import_jobs SET status='waiting'", [])
            .unwrap();
        let recovered = start_network_job(&db, root, true).unwrap();
        assert!(recovered.recovered);
        let work = next_network_work(&db, root, recovered.generation)
            .unwrap()
            .unwrap();
        assert!(work.metadata_pending && work.thumbnail_pending);
    }
}
