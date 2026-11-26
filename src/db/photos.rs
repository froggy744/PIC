use std::collections::{HashMap, HashSet};

pub(crate) fn folder_name(path: &str) -> String {
    if is_remote_path(path) {
        let segment = path
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .filter(|name| !name.is_empty())
            .unwrap_or(path);
        return glib::uri_unescape_string(segment, None::<&str>)
            .map(|name| name.to_string())
            .unwrap_or_else(|| segment.to_string());
    }

    crate::source::filename(path)
}

fn ensure_parent_folder(connection: &Connection, path: &str) -> Result<Option<i64>> {
    let Some(parent) = Path::new(path).parent().and_then(|parent| parent.to_str()) else {
        return Ok(None);
    };
    // Do not expose generic filesystem containers such as /mnt or /run in
    // the sidebar. A mounted share (for example /mnt/4TBP) is a useful
    // persisted grouping parent for imported folders below it.
    if matches!(parent, "/mnt" | "/run" | "/run/media") {
        return Ok(None);
    }
    if let Some(id) = connection
        .query_row("SELECT id FROM folders WHERE path = ?1", [parent], |row| row.get(0))
        .optional()?
    {
        return Ok(Some(id));
    }
    let parent_id = ensure_parent_folder(connection, parent)?;
    let name = folder_name(parent);
    connection.execute(
        "INSERT INTO folders(path, name, parent_id, imported_root) VALUES (?1, ?2, ?3, 0)",
        params![parent, name, parent_id],
    )?;
    Ok(Some(connection.query_row("SELECT id FROM folders WHERE path = ?1", [parent], |row| row.get(0))?))
}

fn repair_existing_folder_parents(connection: &Connection) -> Result<()> {
    let rows = connection
        .prepare("SELECT id, path, parent_id FROM folders")?
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<i64>>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    for (id, path, current_parent_id) in rows {
        let Some(parent_path) = Path::new(&path).parent().and_then(|parent| parent.to_str())
        else {
            continue;
        };
        let parent_id: Option<i64> = connection
            .query_row(
                "SELECT id FROM folders WHERE path = ?1 AND id != ?2",
                params![parent_path, id],
                |row| row.get(0),
            )
            .optional()?;
        if parent_id != current_parent_id {
            connection.execute(
                "UPDATE folders SET parent_id = ?1 WHERE id = ?2",
                params![parent_id, id],
            )?;
        }
    }
    Ok(())
}

pub fn insert_folder(connection: &Connection, path: &str) -> Result<i64> {
    let name = folder_name(path);
    let mut imported_parent: Option<i64> = connection
        .query_row(
            "SELECT id FROM folders
             WHERE imported_root = 1 AND path != ?1 AND ?1 LIKE path || '/%'
             ORDER BY length(path) DESC LIMIT 1",
            [path],
            |row| row.get(0),
        )
        .optional()?;
    let existing: Option<(i64, Option<i64>, bool)> = connection
        .query_row(
            "SELECT id, parent_id, imported_root FROM folders WHERE path = ?1",
            [path],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let direct_parent: Option<i64> = Path::new(path)
        .parent()
        .and_then(|parent| parent.to_str())
        .and_then(|parent| {
            connection
                .query_row("SELECT id FROM folders WHERE path = ?1", [parent], |row| {
                    row.get(0)
                })
                .optional()
                .ok()
                .flatten()
        });
    let direct_parent = match direct_parent {
        Some(parent_id) => Some(parent_id),
        None => ensure_parent_folder(connection, path)?,
    };
    if imported_parent.is_none() && existing.as_ref().is_none_or(|(_, _, root)| *root) {
        let selected_parent = Path::new(path).parent().and_then(|parent| parent.to_str());
        let sibling_parent = if let Some(selected_parent) = selected_parent {
            let paths = connection
                .prepare("SELECT path FROM folders WHERE imported_root = 1 AND path != ?1")?
                .query_map([path], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            paths.into_iter().find_map(|candidate| {
                (Path::new(&candidate).parent().and_then(|parent| parent.to_str())
                    == Some(selected_parent))
                .then_some(selected_parent.to_string())
            })
        } else {
            None
        };
        if let Some(sibling_parent) = sibling_parent {
            let parent_id = insert_folder(connection, &sibling_parent)?;
            imported_parent = Some(parent_id);
            
        }
    }
    let (parent_id, imported_root) = match existing {
        // A refresh or re-import of a discovered folder must keep its real
        // containing folder. Only a previously imported root may be demoted
        // under an already-imported ancestor.
        Some((_, parent_id, true)) => {
            let parent_id = direct_parent.or(parent_id);
            (parent_id, true)
        }
        Some((_, parent_id, false)) => (direct_parent.or(parent_id), false),
        _ => {
            let parent_id = direct_parent.or(imported_parent);
            (parent_id, parent_id.is_none())
        }
    };
    
    connection.execute(
        "INSERT INTO folders(path, name, parent_id, imported_root) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(path) DO UPDATE SET name = excluded.name,
           parent_id = excluded.parent_id, imported_root = excluded.imported_root",
        params![path, name, parent_id, imported_root],
    )?;
    let id = connection.query_row("SELECT id FROM folders WHERE path = ?1", [path], |row| {
        row.get(0)
    })?;
    let _reparented = if imported_root {
        connection.execute(
            "UPDATE folders
             SET parent_id = ?1, imported_root = 0
             WHERE id != ?1 AND imported_root = 1 AND path LIKE ?2 || '/%'",
            params![id, path],
        )?
    } else {
        0
    };
    
    repair_existing_folder_parents(connection)?;
    Ok(id)
}

/// Mark a user-selected folder as an independent library root. Ancestor roots
/// are no longer refreshed because the user has narrowed the imported scope to
/// this folder (and any other explicitly imported descendants).
pub fn mark_import_root(connection: &Connection, path: &str) -> Result<i64> {
    let id = insert_folder(connection, path)?;
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "UPDATE folders SET imported_root = 0
         WHERE imported_root = 1 AND path != ?1 AND ?1 LIKE path || '/%'",
        [path],
    )?;
    transaction.execute("UPDATE folders SET imported_root = 1 WHERE id = ?1", [id])?;
    transaction.commit()?;
    Ok(id)
}

/// True when a folder path points at a directly transported network share
/// (`smb://`, `nfs://`) instead of a local path. Such roots are presented in
/// the sidebar's Network Shares section, never under Folders.
#[cfg(target_os = "linux")]
pub fn is_remote_path(path: &str) -> bool {
    crate::network_shares::private(path)
}

#[cfg(not(target_os = "linux"))]
pub fn is_remote_path(_path: &str) -> bool {
    false
}

pub fn insert_discovered_folder(connection: &Connection, path: &str, parent_id: i64) -> Result<i64> {
    let name = folder_name(path);
    let _existing: Option<(i64, Option<i64>, bool)> = connection
        .query_row("SELECT id, parent_id, imported_root FROM folders WHERE path = ?1", [path], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .optional()?;
    connection.execute(
        "INSERT INTO folders(path, name, parent_id, imported_root) VALUES (?1, ?2, ?3, 0)
         ON CONFLICT(path) DO UPDATE SET name = excluded.name,
           parent_id = CASE WHEN folders.imported_root = 1 THEN folders.parent_id ELSE excluded.parent_id END",
        params![path, name, parent_id],
    )?;
    let id = connection.query_row("SELECT id FROM folders WHERE path = ?1", [path], |row| row.get(0))?;
    
    Ok(id)
}

pub fn search_folders(
    connection: &Connection,
    query: &str,
    limit: usize,
) -> Result<Vec<FolderSearchResult>> {
    let query = query.trim();
    if query.chars().count() < 2 || limit == 0 {
        return Ok(Vec::new());
    }

    let mut statement = connection.prepare(
        "SELECT f.id, f.path, COALESCE(f.name, f.path)
         FROM folders f
         WHERE (instr(lower(COALESCE(f.name, f.path)), lower(?1)) > 0
            OR instr(lower(f.path), lower(?1)) > 0)
           AND EXISTS (
             WITH RECURSIVE descendants(id) AS (
               SELECT f.id
               UNION ALL
               SELECT child.id
               FROM folders child
               JOIN descendants ON child.parent_id = descendants.id
             )
             SELECT 1
             FROM photos p
             WHERE p.trashed = 0
               AND p.folder_id IN (SELECT id FROM descendants)
           )
         ORDER BY
           CASE
             WHEN lower(COALESCE(f.name, f.path)) = lower(?1) THEN 0
             WHEN instr(lower(COALESCE(f.name, f.path)), lower(?1)) = 1 THEN 1
             ELSE 2
           END,
           COALESCE(f.name, f.path) COLLATE NOCASE,
           f.path COLLATE NOCASE
         LIMIT ?2",
    )?;
    let rows = statement.query_map(params![query, limit as i64], |row| {
        Ok(FolderSearchResult {
            id: row.get(0)?,
            path: row.get(1)?,
            name: row.get(2)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

#[cfg(test)]
pub fn folder_path_by_id(connection: &Connection, folder_id: i64) -> Result<Option<String>> {
    Ok(connection
        .query_row(
            "SELECT path FROM folders WHERE id = ?1",
            [folder_id],
            |row| row.get(0),
        )
        .optional()?)
}

pub fn set_raw_jpeg_pair_counts(
    connection: &Connection,
    counts_by_path: &HashMap<String, i64>,
) -> Result<()> {
    if counts_by_path.is_empty() {
        return Ok(());
    }
    let transaction = connection.unchecked_transaction()?;
    for (path, count) in counts_by_path {
        transaction.execute(
            "UPDATE folders SET raw_jpeg_pair_count = ?1 WHERE path = ?2",
            params![count, path],
        )?;
    }
    transaction.commit()?;
    Ok(())
}

pub fn raw_jpeg_pair_folder_ids(connection: &Connection) -> Result<HashSet<i64>> {
    let mut statement =
        connection.prepare("SELECT id FROM folders WHERE raw_jpeg_pair_count > 0")?;
    let rows = statement.query_map([], |row| row.get::<_, i64>(0))?;
    Ok(rows.collect::<rusqlite::Result<HashSet<_>>>()?)
}

pub fn folders(connection: &Connection) -> Result<Vec<Folder>> {
    // Read the folder rows and direct counts in one pass. The old query ran a
    // recursive descendant CTE once per folder, which became very expensive on
    // large trees and could block the GTK thread for seconds when Settings or
    // the sidebar requested a refresh.
    let mut statement = connection.prepare(
        "SELECT
             f.id,
             f.path,
             COALESCE(f.name, f.path),
             f.parent_id,
             f.imported_root,
             f.watched,
             COALESCE(photo_counts.photo_count, 0),
             COALESCE(child_counts.subfolder_count, 0)
         FROM folders f
         LEFT JOIN (
             SELECT folder_id, COUNT(*) AS photo_count
             FROM photos
             WHERE trashed = 0
             GROUP BY folder_id
         ) photo_counts ON photo_counts.folder_id = f.id
         LEFT JOIN (
             SELECT parent_id, COUNT(*) AS subfolder_count
             FROM folders
             WHERE parent_id IS NOT NULL
             GROUP BY parent_id
         ) child_counts ON child_counts.parent_id = f.id
         ORDER BY f.path COLLATE NOCASE",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(Folder {
            id: row.get(0)?,
            path: row.get(1)?,
            name: row.get(2)?,
            parent_id: row.get(3)?,
            imported_root: row.get(4)?,
            watched: row.get(5)?,
            // Direct photo count for now; rolled up to descendants below.
            photo_count: row.get(6)?,
            subfolder_count: row.get(7)?,
            available: true,
        })
    })?;
    let mut folders = rows.collect::<rusqlite::Result<Vec<_>>>()?;

    // Build the hierarchy once and roll direct photo counts upward in memory.
    // This preserves the historical meaning of Folder::photo_count (the
    // folder plus all descendants) without repeated recursive SQL.
    let by_id = folders
        .iter()
        .enumerate()
        .map(|(index, folder)| (folder.id, index))
        .collect::<HashMap<_, _>>();
    let mut children = HashMap::<i64, Vec<i64>>::new();
    for folder in &folders {
        if let Some(parent_id) = folder.parent_id.filter(|id| by_id.contains_key(id)) {
            children.entry(parent_id).or_default().push(folder.id);
        }
    }
    let direct_counts = folders
        .iter()
        .map(|folder| (folder.id, folder.photo_count))
        .collect::<HashMap<_, _>>();

    fn subtree_photo_count(
        folder_id: i64,
        direct_counts: &HashMap<i64, i64>,
        children: &HashMap<i64, Vec<i64>>,
        memo: &mut HashMap<i64, i64>,
        visiting: &mut HashSet<i64>,
    ) -> i64 {
        if let Some(total) = memo.get(&folder_id) {
            return *total;
        }
        if !visiting.insert(folder_id) {
            return direct_counts.get(&folder_id).copied().unwrap_or_default();
        }
        let mut total = direct_counts.get(&folder_id).copied().unwrap_or_default();
        if let Some(child_ids) = children.get(&folder_id) {
            for child_id in child_ids {
                total += subtree_photo_count(*child_id, direct_counts, children, memo, visiting);
            }
        }
        visiting.remove(&folder_id);
        memo.insert(folder_id, total);
        total
    }

    let mut memo = HashMap::with_capacity(folders.len());
    let mut visiting = HashSet::new();
    for folder in &mut folders {
        folder.photo_count = subtree_photo_count(
            folder.id,
            &direct_counts,
            &children,
            &mut memo,
            &mut visiting,
        );
    }

    for folder in &mut folders {
        if is_remote_path(&folder.path) {
            folder.name = folder_name(&folder.path);
        }
    }
    let trace_startup = std::env::var_os("PIC_STARTUP_TRACE").is_some();
    let availability_started = std::time::Instant::now();
    let mut root_checks = 0usize;
    let mut unavailable_roots = 0usize;
    let mut slowest_root_check_us = 0u128;
    let availability = folder_availability_by_id(&folders, |path| {
        root_checks += 1;
        let check_started = trace_startup.then(std::time::Instant::now);
        let available = crate::source::cached_source_available(path);
        if let Some(check_started) = check_started {
            slowest_root_check_us = slowest_root_check_us.max(check_started.elapsed().as_micros());
            unavailable_roots += usize::from(!available);
        }
        available
    });
    if trace_startup {
        eprintln!(
            "PIC_STARTUP stage=folder_availability roots={root_checks} unavailable={unavailable_roots} elapsed_us={} max_root_check_us={slowest_root_check_us}",
            availability_started.elapsed().as_micros(),
        );
    }
    for folder in &mut folders {
        folder.available = availability.get(&folder.id).copied().unwrap_or(true);
    }
    crate::source::replace_folder_availability(availability);
    Ok(folders)
}

/// Return only user-selected scan roots. Discovered descendants are never
/// refresh roots, even when they contain photos or are marked watched.
pub fn imported_root_paths(connection: &Connection) -> Result<Vec<String>> {
    let mut statement = connection.prepare(
        "SELECT path FROM folders
         WHERE imported_root = 1
           AND NOT EXISTS (
             SELECT 1
             FROM folders ancestor
             WHERE ancestor.imported_root = 1
               AND ancestor.id != folders.id
               AND folders.path LIKE ancestor.path || '/%'
           )
         ORDER BY path COLLATE NOCASE",
    )?;
    let rows = statement.query_map([], |row| row.get(0))?;
    Ok(rows.collect::<rusqlite::Result<Vec<String>>>()?)
}

/// Persist whether a registered library folder should be watched.
/// Any indexed folder can be watched independently; the watcher runtime remains
/// responsible for coalescing filesystem activity safely.
pub fn set_folder_watched(connection: &Connection, folder_id: i64, watched: bool) -> Result<bool> {
    let changed = connection.execute(
        "UPDATE folders SET watched = ?1 WHERE id = ?2",
        params![watched, folder_id],
    )?;
    Ok(changed > 0)
}

/// Resolve every indexed folder to the availability of its imported source
/// root. Only imported roots touch the filesystem; discovered descendants
/// inherit their root's state.
pub fn folder_availability_by_id(
    folders: &[Folder],
    mut source_available: impl FnMut(&str) -> bool,
) -> HashMap<i64, bool> {
    let by_id = folders
        .iter()
        .map(|folder| (folder.id, folder))
        .collect::<HashMap<_, _>>();
    let mut root_availability = HashMap::new();
    for folder in folders.iter().filter(|folder| folder.imported_root) {
        root_availability.insert(folder.id, source_available(&folder.path));
    }

    fn resolve(
        folder_id: i64,
        by_id: &HashMap<i64, &Folder>,
        root_availability: &HashMap<i64, bool>,
        resolved: &mut HashMap<i64, bool>,
        visiting: &mut HashSet<i64>,
    ) -> bool {
        if let Some(available) = resolved.get(&folder_id) {
            return *available;
        }
        if !visiting.insert(folder_id) {
            return true;
        }
        let available = by_id.get(&folder_id).map_or(true, |folder| {
            if folder.imported_root {
                root_availability.get(&folder.id).copied().unwrap_or(true)
            } else {
                folder
                    .parent_id
                    .map(|parent_id| resolve(parent_id, by_id, root_availability, resolved, visiting))
                    .unwrap_or(true)
            }
        });
        visiting.remove(&folder_id);
        resolved.insert(folder_id, available);
        available
    }

    let mut resolved = HashMap::with_capacity(folders.len());
    let mut visiting = HashSet::new();
    for folder in folders {
        resolve(
            folder.id,
            &by_id,
            &root_availability,
            &mut resolved,
            &mut visiting,
        );
    }
    resolved
}

/// Remove a folder and its indexed descendants from the application database.
/// This only deletes database rows; it never touches the filesystem.
pub fn remove_folder(connection: &Connection, folder_id: i64) -> Result<()> {
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "DELETE FROM settings WHERE key IN (
           WITH RECURSIVE descendants(id) AS (
             SELECT id FROM folders WHERE id = ?1
             UNION ALL
             SELECT child.id FROM folders child
             JOIN descendants ON child.parent_id = descendants.id
           )
           SELECT 'scan-devices:' || id FROM descendants
         )",
        [folder_id],
    )?;
    transaction.execute(
        "DELETE FROM photos
         WHERE folder_id IN (
           WITH RECURSIVE descendants(id) AS (
             SELECT id FROM folders WHERE id = ?1
             UNION ALL
             SELECT child.id FROM folders child
             JOIN descendants ON child.parent_id = descendants.id
           )
           SELECT id FROM descendants
         )",
        [folder_id],
    )?;
    transaction.execute(
        "DELETE FROM folders
         WHERE id IN (
           WITH RECURSIVE descendants(id) AS (
             SELECT id FROM folders WHERE id = ?1
             UNION ALL
             SELECT child.id FROM folders child
             JOIN descendants ON child.parent_id = descendants.id
           )
           SELECT id FROM descendants
         )",
        [folder_id],
    )?;
    transaction.commit()?;
    Ok(())
}

/// Remove indexed photos below a folder when a completed scan confirms they
/// are no longer present. The caller must verify that the storage location is
/// available before calling this function; an empty offline mount must never
/// be treated as an intentional deletion.
pub fn remove_missing_photos(
    connection: &Connection,
    folder_id: i64,
    present_paths: &HashSet<String>,
) -> Result<Vec<i64>> {
    let stale_ids = {
        let mut statement = connection.prepare(
            "SELECT id, path FROM photos
             WHERE folder_id IN (
               WITH RECURSIVE descendants(id) AS (
                 SELECT id FROM folders WHERE id = ?1
                 UNION ALL
                 SELECT child.id FROM folders child
                 JOIN descendants ON child.parent_id = descendants.id
               )
               SELECT id FROM descendants
             )",
        )?;
        let rows = statement
            .query_map([folder_id], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
            .into_iter()
            .filter(|(_, path)| !present_paths.contains(path))
            .map(|(id, _)| id)
            .collect::<Vec<_>>()
    };
    if stale_ids.is_empty() {
        return Ok(Vec::new());
    }
    // Keep refresh deletion commits bounded so interactive settings/state
    // writes are never queued behind one transaction containing thousands of
    // stale rows.
    for chunk in stale_ids.chunks(128) {
        let transaction = connection.unchecked_transaction()?;
        for id in chunk {
            transaction.execute("DELETE FROM photos WHERE id = ?1", [id])?;
        }
        transaction.commit()?;
    }
    Ok(stale_ids)
}

/// Reconcile folder records only after a complete, successful tree scan.
/// Existing empty directories and the scanned root remain in the catalogue.
pub fn remove_missing_folders(
    connection: &Connection,
    folder_id: i64,
    present_paths: &HashSet<String>,
) -> Result<usize> {
    let stale_ids = {
        let mut statement = connection.prepare(
            "WITH RECURSIVE descendants(id, path, depth) AS (
               SELECT id, path, 0 FROM folders WHERE id = ?1
               UNION ALL
               SELECT child.id, child.path, parent.depth + 1
               FROM folders child JOIN descendants parent ON child.parent_id = parent.id
             )
             SELECT id, path FROM descendants WHERE depth > 0 ORDER BY depth DESC",
        )?;
        let rows = statement.query_map([folder_id], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?.collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter().filter(|(_, path)| !present_paths.contains(path))
            .map(|(id, _)| id).collect::<Vec<_>>()
    };
    // Children precede parents so foreign keys remain valid between batches.
    for chunk in stale_ids.chunks(128) {
        let transaction = connection.unchecked_transaction()?;
        for id in chunk {
            transaction.execute("DELETE FROM settings WHERE key = ?1", [format!("scan-devices:{id}")])?;
            transaction.execute("DELETE FROM folders WHERE id = ?1", [id])?;
        }
        transaction.commit()?;
    }
    Ok(stale_ids.len())
}

pub fn folder_exists(connection: &Connection, folder_id: i64) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM folders WHERE id = ?1)",
        [folder_id],
        |row| row.get(0),
    )?)
}

pub fn upsert_photo(
    connection: &Connection,
    path: &Path,
    folder_id: Option<i64>,
    metadata: &PhotoMetadata,
) -> Result<i64> {
    let path = path.to_string_lossy();
    connection.execute(
        "INSERT INTO photos(path, folder_id, taken_at, camera, aperture, lens, shutter_speed, iso, focal_length, exposure_bias, width, height, size_bytes, mtime, added_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, CAST(strftime('%s', 'now') AS INTEGER))
         ON CONFLICT(path) DO UPDATE SET folder_id=excluded.folder_id,
           taken_at=excluded.taken_at, camera=excluded.camera, aperture=excluded.aperture,
           lens=excluded.lens, shutter_speed=excluded.shutter_speed, iso=excluded.iso,
           focal_length=excluded.focal_length, exposure_bias=excluded.exposure_bias, width=excluded.width,
           height=excluded.height, size_bytes=excluded.size_bytes, mtime=excluded.mtime",
        params![
            path.as_ref(),
            folder_id,
            metadata.taken_at,
            metadata.camera,
            // Zero records that the file was examined but carried no aperture.
            // NULL is reserved for rows created before aperture indexing so a
            // refresh can repair them exactly once.
            metadata.aperture.or(Some(0.0)),
            metadata.lens.as_deref().unwrap_or(""),
            metadata.shutter_speed.unwrap_or_default(),
            metadata.iso.unwrap_or_default(),
            metadata.focal_length.unwrap_or_default(),
            // Zero exposure bias is valid, so the ISO column is the indexing marker.
            metadata.exposure_bias,
            metadata.width,
            metadata.height,
            metadata.size_bytes,
            metadata.mtime
        ],
    )?;
    let id = connection.query_row("SELECT id FROM photos WHERE path = ?1", [&path], |row| row.get(0))?;
    metadata::store(connection, id, &path, metadata)?;
    Ok(id)
}

pub fn set_photo_folder(connection: &Connection, path: &str, folder_id: i64) -> Result<()> {
    connection.execute(
        "UPDATE photos SET folder_id = ?2 WHERE path = ?1",
        params![path, folder_id],
    )?;
    Ok(())
}

pub fn photo_fingerprints(
    connection: &Connection,
) -> Result<std::collections::HashMap<String, (Option<i64>, Option<i64>, Option<i64>, Option<i64>, Option<f64>, Option<i64>)>>
{
    let mut statement =
        connection.prepare("SELECT path, mtime, size_bytes, width, height, CAST(aperture AS REAL), iso FROM photos")?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            (row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?),
        ))
    })?;
    Ok(rows.collect::<rusqlite::Result<std::collections::HashMap<_, _>>>()?)
}

pub fn photo(connection: &Connection, id: i64) -> Result<Option<Photo>> {
    Ok(connection
        .query_row(
            "SELECT p.id,p.path,p.folder_id,p.taken_at,p.camera,CAST(p.aperture AS REAL),p.width,p.height,p.size_bytes,p.mtime,p.added_at,p.rotation,p.edit_recipe,p.favorite,p.rating,p.trashed,f.path,p.lens,p.shutter_speed,p.iso,p.focal_length,p.exposure_bias
             FROM photos p LEFT JOIN folders f ON f.id = p.folder_id WHERE p.id = ?1",
            [id],
            photo_from_row,
        )
        .optional()?)
}

/// The catalogued, untrashed photo whose original is `path`, if any.
pub fn photo_by_path(connection: &Connection, path: &str) -> Result<Option<Photo>> {
    Ok(connection
        .query_row(
            "SELECT p.id,p.path,p.folder_id,p.taken_at,p.camera,CAST(p.aperture AS REAL),p.width,p.height,p.size_bytes,p.mtime,p.added_at,p.rotation,p.edit_recipe,p.favorite,p.rating,p.trashed,f.path,p.lens,p.shutter_speed,p.iso,p.focal_length,p.exposure_bias
             FROM photos p LEFT JOIN folders f ON f.id = p.folder_id WHERE p.path = ?1 AND p.trashed = 0",
            [path],
            photo_from_row,
        )
        .optional()?)
}

pub fn photos(
    connection: &Connection,
    folder_id: Option<i64>,
    favorites_only: bool,
    search: Option<&str>,
) -> Result<Vec<Photo>> {
    photos_limited(connection, folder_id, favorites_only, search, -1)
}

pub fn photos_limited(
    connection: &Connection,
    folder_id: Option<i64>,
    favorites_only: bool,
    search: Option<&str>,
    limit: i64,
) -> Result<Vec<Photo>> {
    photos_limited_filtered(connection, folder_id, favorites_only, search, limit)
}

/// The photos `scope` browses: its destination, search, effective rating and
/// metadata filters, newest first (History: most recently edited first).
/// Files choices (Online, Offline, Corrupt) are not SQL; callers check them
/// on the result (`BrowseScope::files_check`).
pub fn browse_photos(connection: &Connection, scope: &BrowseScope) -> Result<Vec<Photo>> {
    browse_photos_limited(connection, scope, -1)
}
pub fn browse_photos_limited(
    connection: &Connection,
    scope: &BrowseScope,
    limit: i64,
) -> Result<Vec<Photo>> {
    if scope.destination == Destination::History {
        return history_photos_in_scope(connection, limit, scope);
    }
    let mut values: Vec<rusqlite::types::Value> = vec![limit.into()];
    let membership = scope.predicate(connection, &mut values)?;
    let mut statement = connection.prepare(&format!(
        "SELECT p.id,p.path,p.folder_id,p.taken_at,p.camera,CAST(p.aperture AS REAL),p.width,p.height,p.size_bytes,p.mtime,p.added_at,p.rotation,p.edit_recipe,p.favorite,p.rating,p.trashed,f.path,p.lens,p.shutter_speed,p.iso,p.focal_length,p.exposure_bias
         FROM photos p LEFT JOIN folders f ON f.id = p.folder_id
         WHERE p.trashed = 0 AND {membership}
         ORDER BY p.taken_at IS NULL, p.taken_at DESC, p.path COLLATE NOCASE LIMIT ?1",
    ))?;
    let rows = statement.query_map(rusqlite::params_from_iter(values), photo_from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}
/// `LIKE` pattern for a free-text photo search. Backslash is the escape
/// character, so the user's own backslash, `%` and `_` match literally.
fn search_like_pattern(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{escaped}%")
}

/// SQL condition for the free-text search bound at `?{param}` (NULL = no
/// search). A photo matches on its path, its capture date (the date part
/// only, so `2023-05` finds May 2023 without hitting clock times), or the
/// name of any album it belongs to. `?{date_param}` carries the query read
/// as a written date (`search_date_pattern`), or NULL when it is not one.
fn search_text_predicate(param: usize, date_param: usize) -> String {
    format!(
        "(?{param} IS NULL
          OR p.path LIKE ?{param} ESCAPE '\\'
          OR substr(p.taken_at, 1, 10) LIKE ?{param} ESCAPE '\\'
          OR substr(p.taken_at, 1, 10) LIKE ?{date_param}
          OR EXISTS (
            SELECT 1 FROM album_photos search_ap
            JOIN albums search_album ON search_album.id = search_ap.album_id
            WHERE search_ap.photo_id = p.id
              AND search_album.name LIKE ?{param} ESCAPE '\\'
          ))"
    )
}
/// Reads a search query written as a calendar date and returns a `LIKE`
/// pattern for the stored `YYYY-MM-DD` date part, or `None` when the query
/// is not a date. Accepted shapes (case-insensitive, English month names or
/// three-letter abbreviations, `/ - . ,` or spaces as separators, `st/nd/rd/th`
/// day suffixes):
/// - `May 2023`, `2023 May`, `05/2023`, `2023/05` -> that month
/// - `14 May 2023` (the InfoBar format), `May 14, 2023`, `2023 May 14`,
///   `14/05/2023` (day first), `2023/05/14` -> that day
/// A month name alone is not a date: it would hijack every partial word
/// (`mar` while typing `marianne`).
fn search_date_pattern(query: &str) -> Option<String> {
    enum Part {
        Number(u32, usize),
        Month(u32),
    }
    fn month(word: &str) -> Option<u32> {
        const MONTHS: [&str; 12] = [
            "january", "february", "march", "april", "may", "june", "july", "august",
            "september", "october", "november", "december",
        ];
        let word = word.to_ascii_lowercase();
        if word == "sept" {
            return Some(9);
        }
        MONTHS
            .iter()
            .position(|name| word == *name || (word.len() == 3 && name.starts_with(&word)))
            .map(|index| index as u32 + 1)
    }
    let mut parts = Vec::new();
    for token in query
        .split(|c: char| c.is_whitespace() || matches!(c, '/' | '-' | '.' | ','))
        .filter(|token| !token.is_empty())
    {
        let digits = token.trim_end_matches(|c: char| c.is_ascii_alphabetic());
        let suffix = &token[digits.len()..];
        if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
            let day_suffix = matches!(suffix.to_ascii_lowercase().as_str(), "" | "st" | "nd" | "rd" | "th");
            if !day_suffix || digits.len() > 4 {
                return None;
            }
            parts.push(Part::Number(digits.parse().ok()?, digits.len()));
        } else {
            parts.push(Part::Month(month(token)?));
        }
    }
    let year = |value: u32, len: usize| (len == 4).then_some(value as i32);
    let small = |value: u32, len: usize| (len <= 2).then_some(value);
    let (year, month, day) = match parts.as_slice() {
        [Part::Month(m), Part::Number(y, yl)] | [Part::Number(y, yl), Part::Month(m)] => {
            (year(*y, *yl)?, *m, None)
        }
        [Part::Number(m, ml), Part::Number(y, yl)] if *yl == 4 => (*y as i32, small(*m, *ml)?, None),
        [Part::Number(y, yl), Part::Number(m, ml)] if *yl == 4 => (*y as i32, small(*m, *ml)?, None),
        // `14 May 2023` or `2023 May 14`: the four-digit side is the year.
        [Part::Number(a, al), Part::Month(m), Part::Number(b, bl)] => {
            if *bl == 4 {
                (*b as i32, *m, Some(small(*a, *al)?))
            } else {
                (year(*a, *al)?, *m, Some(small(*b, *bl)?))
            }
        }
        [Part::Month(m), Part::Number(d, dl), Part::Number(y, yl)] => {
            (year(*y, *yl)?, *m, Some(small(*d, *dl)?))
        }
        [Part::Number(y, yl), Part::Number(m, ml), Part::Number(d, dl)] if *yl == 4 => {
            (*y as i32, small(*m, *ml)?, Some(small(*d, *dl)?))
        }
        [Part::Number(d, dl), Part::Number(m, ml), Part::Number(y, yl)] if *yl == 4 => {
            (*y as i32, small(*m, *ml)?, Some(small(*d, *dl)?))
        }
        _ => return None,
    };
    match day {
        Some(day) => chrono::NaiveDate::from_ymd_opt(year, month, day)
            .map(|date| date.format("%Y-%m-%d").to_string()),
        None => chrono::NaiveDate::from_ymd_opt(year, month, 1)
            .map(|date| date.format("%Y-%m-%%").to_string()),
    }
}

/// Unfiltered photos for administrative and bulk work (screensaver, scans,
/// Scrapbook pickers): no browse scope applies.
fn photos_limited_filtered(
    connection: &Connection,
    folder_id: Option<i64>,
    favorites_only: bool,
    search: Option<&str>,
    limit: i64,
) -> Result<Vec<Photo>> {
    let date = search.and_then(search_date_pattern);
    let search = search.map(search_like_pattern);
    let text = search_text_predicate(3, 5);
    let mut statement = connection.prepare(&format!(
        "SELECT p.id,p.path,p.folder_id,p.taken_at,p.camera,CAST(p.aperture AS REAL),p.width,p.height,p.size_bytes,p.mtime,p.added_at,p.rotation,p.edit_recipe,p.favorite,p.rating,p.trashed,f.path,p.lens,p.shutter_speed,p.iso,p.focal_length,p.exposure_bias
         FROM photos p LEFT JOIN folders f ON f.id = p.folder_id
         WHERE p.trashed = 0 AND (?1 IS NULL OR p.folder_id IN
             (WITH RECURSIVE descendants(id) AS (
                SELECT id FROM folders WHERE id = ?1
                UNION ALL
                SELECT child.id FROM folders child JOIN descendants ON child.parent_id = descendants.id
              ) SELECT id FROM descendants))
           AND (?2 = 0 OR p.favorite = 1) AND {text}
         ORDER BY p.taken_at IS NULL, p.taken_at DESC, p.path COLLATE NOCASE LIMIT ?4",
    ))?;
    let values: Vec<rusqlite::types::Value> = vec![
        folder_id.into(),
        (favorites_only as i64).into(),
        search.into(),
        limit.into(),
        date.into(),
    ];
    let rows = statement.query_map(rusqlite::params_from_iter(values), photo_from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn photo_availability_page(
    connection: &Connection,
    limit: usize,
    offset: usize,
) -> Result<Vec<(String, Option<String>)>> {
    let mut statement = connection.prepare(
        "SELECT p.path, f.path
         FROM photos p LEFT JOIN folders f ON f.id = p.folder_id
         WHERE p.trashed = 0
         ORDER BY p.id
         LIMIT ?1 OFFSET ?2",
    )?;
    let rows = statement.query_map(params![limit as i64, offset as i64], |row| {
        Ok((row.get(0)?, row.get(1)?))
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn photos_in_album(
    connection: &Connection,
    album_id: i64,
    search: Option<&str>,
) -> Result<Vec<Photo>> {
    browse_photos(
        connection,
        &BrowseScope {
            destination: Destination::Album(album_id),
            search: search.unwrap_or_default().to_owned(),
            ..BrowseScope::default()
        },
    )
}

#[cfg(test)]
pub fn set_favorite(connection: &Connection, id: i64, favorite: bool) -> Result<()> {
    connection.execute(
        "UPDATE photos SET favorite = ?1 WHERE id = ?2",
        params![favorite, id],
    )?;
    Ok(())
}

#[cfg(test)]
pub fn set_rating(connection: &Connection, id: i64, rating: i32) -> Result<()> {
    anyhow::ensure!((0..=5).contains(&rating), "invalid rating: {rating}");
    connection.execute(
        "UPDATE photos SET rating = ?1 WHERE id = ?2",
        params![rating, id],
    )?;
    Ok(())
}

pub fn set_rating_for_photos(connection: &Connection, ids: &[i64], rating: i32) -> Result<()> {
    anyhow::ensure!((0..=5).contains(&rating), "invalid rating: {rating}");
    if ids.is_empty() {
        return Ok(());
    }

    let transaction = connection.unchecked_transaction()?;
    {
        let mut statement = transaction.prepare(
            "UPDATE photos SET rating = ?1 WHERE id = ?2 AND trashed = 0",
        )?;
        for id in ids {
            statement.execute(params![rating, id])?;
        }
    }
    transaction.commit()?;
    Ok(())
}

/// Mark or unmark several photos in one transaction: on error none change.
/// Trashed photos are skipped, like `set_rating_for_photos`.
pub fn set_favorite_for_photos(connection: &Connection, ids: &[i64], favorite: bool) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }

    let transaction = connection.unchecked_transaction()?;
    {
        let mut statement = transaction.prepare(
            "UPDATE photos SET favorite = ?1 WHERE id = ?2 AND trashed = 0",
        )?;
        for id in ids {
            statement.execute(params![favorite, id])?;
        }
    }
    transaction.commit()?;
    Ok(())
}

pub fn set_favorite_for_folder(
    connection: &Connection,
    folder_id: i64,
    favorite: bool,
) -> Result<usize> {
    let changed = connection.execute(
        "UPDATE photos
         SET favorite = ?1
         WHERE trashed = 0 AND folder_id IN (
             WITH RECURSIVE descendants(id) AS (
                 SELECT id FROM folders WHERE id = ?2
                 UNION ALL
                 SELECT child.id FROM folders child
                 JOIN descendants ON child.parent_id = descendants.id
             )
             SELECT id FROM descendants
         )",
        params![favorite, folder_id],
    )?;
    Ok(changed)
}


pub fn set_edit_recipe(connection: &Connection, id: i64, recipe: &str) -> Result<()> {
    commit_edit_recipes(
        connection,
        &[(id, recipe.to_owned())],
        if recipe.is_empty() { "reset" } else { "edit" },
        None,
    )?;
    Ok(())
}

pub fn any_edited(connection: &Connection, ids: &[i64]) -> Result<bool> {
    if ids.is_empty() {
        return Ok(false);
    }
    for chunk in ids.chunks(500) {
        let placeholders = (0..chunk.len())
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT 1 FROM photos WHERE id IN ({placeholders}) AND edit_recipe != '' LIMIT 1"
        );
        let mut statement = connection.prepare(&sql)?;
        let found = statement
            .query_map(rusqlite::params_from_iter(chunk.iter()), |row| {
                row.get::<_, i64>(0)
            })?
            .next()
            .transpose()?
            .is_some();
        if found {
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn set_rotation(connection: &Connection, id: i64, rotation: i32) -> Result<()> {
    let normalized = rotation.rem_euclid(360);
    if ![0, 90, 180, 270].contains(&normalized) {
        anyhow::bail!("invalid rotation: {rotation}");
    }
    let transaction = connection.unchecked_transaction()?;
    let changed = transaction.execute(
        "UPDATE photos SET rotation = ?1 WHERE id = ?2 AND rotation != ?1 AND trashed=0",
        params![normalized, id],
    )?;
    if changed > 0 {
        record_edit(&transaction, id, "edit", None)?;
    }
    transaction.commit()?;
    Ok(())
}
