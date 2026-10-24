pub const RECENTLY_ADDED_LIMIT_SETTING_KEY: &str = "recently-added-limit";
pub const DEFAULT_RECENTLY_ADDED_LIMIT: usize = 100;
const MAX_RECENTLY_ADDED_LIMIT: usize = 1_000_000;
pub const CORNER_STYLE_SETTING_KEY: &str = "corner-style";
pub const THUMBNAIL_SQUARE_CORNERS_SETTING_KEY: &str = "thumbnail-square-corners";
pub const THUMBNAIL_FIT_WHOLE_PHOTO_SETTING_KEY: &str = "thumbnail-fit-whole-photo";
pub const THUMBNAIL_FILE_NAMES_SETTING_KEY: &str = "thumbnail-file-names";
pub const THUMBNAIL_HIGH_QUALITY_SETTING_KEY: &str = "thumbnail-high-quality";
pub const ZOOM_ANIMATIONS_ENABLED_SETTING_KEY: &str = "zoom-animations-enabled";
pub const LIBRARY_AVAILABLE_SETTING_KEY: &str = "library-stats-available";
pub const LIBRARY_UNAVAILABLE_SETTING_KEY: &str = "library-stats-unavailable";
pub const LIBRARY_STATS_UPDATED_SETTING_KEY: &str = "library-stats-updated";
pub const FOLDER_WATCHING_ENABLED_SETTING_KEY: &str = "folder-watching-enabled";

pub fn high_quality_thumbnails_enabled(connection: &Connection) -> bool {
    setting(connection, THUMBNAIL_HIGH_QUALITY_SETTING_KEY)
        .ok()
        .flatten()
        .as_deref()
        == Some("true")
}

pub fn zoom_animations_enabled(connection: &Connection) -> bool {
    setting(connection, ZOOM_ANIMATIONS_ENABLED_SETTING_KEY)
        .ok()
        .flatten()
        .as_deref()
        != Some("false")
}

pub fn setting(connection: &Connection, key: &str) -> Result<Option<String>> {
    Ok(connection
        .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
            row.get(0)
        })
        .optional()?)
}

pub fn folder_watching_enabled(connection: &Connection) -> bool {
    setting(connection, FOLDER_WATCHING_ENABLED_SETTING_KEY)
        .ok()
        .flatten()
        .as_deref()
        != Some("false")
}

pub fn set_folder_watching_enabled(connection: &Connection, enabled: bool) -> Result<()> {
    set_setting(
        connection,
        FOLDER_WATCHING_ENABLED_SETTING_KEY,
        if enabled { "true" } else { "false" },
    )
}

pub fn recently_added_limit(connection: &Connection) -> usize {
    setting(connection, RECENTLY_ADDED_LIMIT_SETTING_KEY)
        .ok()
        .flatten()
        .and_then(|value| value.parse::<usize>().ok())
        .map(|value| value.clamp(1, MAX_RECENTLY_ADDED_LIMIT))
        .unwrap_or(DEFAULT_RECENTLY_ADDED_LIMIT)
}

pub fn set_setting(connection: &Connection, key: &str, value: &str) -> Result<()> {
    connection.execute(
        "INSERT INTO settings(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

/// Remove a stored preference so it falls back to its default.
pub fn delete_setting(connection: &Connection, key: &str) -> Result<()> {
    connection.execute("DELETE FROM settings WHERE key = ?1", [key])?;
    Ok(())
}

pub fn library_counts(connection: &Connection) -> Result<LibraryCounts> {
    Ok(LibraryCounts {
        photos: connection.query_row(
            "SELECT COUNT(*) FROM photos WHERE trashed = 0",
            [],
            |row| row.get(0),
        )?,
        albums: connection.query_row("SELECT COUNT(*) FROM albums", [], |row| row.get(0))?,
        folders: connection.query_row("SELECT COUNT(*) FROM folders", [], |row| row.get(0))?,
    })
}

/// Counts shown in the Library sidebar, including exact-rating totals.
/// `RecentlyAdded` currently uses the same photo set as All Photos, so keep
/// its count aligned with what that destination actually displays.
pub fn sidebar_counts(connection: &Connection) -> Result<SidebarCounts> {
    let photos: i64 = connection.query_row(
        "SELECT COUNT(*) FROM photos WHERE trashed = 0",
        [],
        |row| row.get(0),
    )?;
    let favorites: i64 = connection.query_row(
        "SELECT COUNT(*) FROM photos WHERE trashed = 0 AND favorite = 1",
        [],
        |row| row.get(0),
    )?;
    let mut ratings = [0_i64; 5];
    let mut statement = connection.prepare(
        "SELECT rating, COUNT(*) FROM photos WHERE trashed = 0 AND rating BETWEEN 1 AND 5 GROUP BY rating",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, usize>(0)?, row.get::<_, i64>(1)?))
    })?;
    for row in rows {
        let (rating, count) = row?;
        ratings[rating - 1] = count;
    }
    Ok(SidebarCounts {
        photos,
        favorites,
        recently_added: photos.min(recently_added_limit(connection) as i64),
        ratings,
    })
}

pub fn database_size(connection: &Connection) -> Result<u64> {
    let path: String = connection.query_row(
        "SELECT file FROM pragma_database_list WHERE name = 'main'",
        [],
        |row| row.get(0),
    )?;
    if path.is_empty() {
        return Ok(0);
    }
    Ok(std::fs::metadata(path)?.len())
}

pub fn set_photo_path(connection: &Connection, id: i64, path: &str) -> Result<()> {
    connection.execute(
        "UPDATE photos SET path = ?1 WHERE id = ?2",
        params![path, id],
    )?;
    Ok(())
}

pub fn set_trashed(connection: &Connection, id: i64, trashed: bool) -> Result<()> {
    connection.execute(
        "UPDATE photos SET trashed = ?1 WHERE id = ?2",
        params![trashed, id],
    )?;
    Ok(())
}

pub fn clear_photos(connection: &Connection) -> Result<()> {
    let transaction = connection.unchecked_transaction()?;
    transaction.execute("DELETE FROM album_photos", [])?;
    // Reset persisted scan progress so a future refresh can repopulate every
    // registered folder after the indexed photos are cleared.
    transaction.execute("DELETE FROM network_import_jobs", [])?;
    transaction.execute("DELETE FROM photos", [])?;
    // Photo rows cascade to event items, while the event rows themselves have
    // no photo foreign key. Remove those now-unreferenced history records too.
    transaction.execute("DELETE FROM editing_events", [])?;
    transaction.commit()?;
    Ok(())
}

pub fn clear_all(connection: &Connection) -> Result<()> {
    let transaction = connection.unchecked_transaction()?;
    transaction.execute("DELETE FROM album_photos", [])?;
    transaction.execute("DELETE FROM albums", [])?;
    transaction.execute("DELETE FROM photos", [])?;
    transaction.execute("DELETE FROM editing_events", [])?;
    transaction.execute("DELETE FROM folders", [])?;
    transaction.commit()?;
    Ok(())
}

#[cfg(test)]
mod zoom_animation_tests {
    use super::*;

    #[test]
    fn high_quality_thumbnails_default_off_and_can_be_enabled() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)")
            .unwrap();

        assert!(!high_quality_thumbnails_enabled(&connection));
        set_setting(&connection, THUMBNAIL_HIGH_QUALITY_SETTING_KEY, "true").unwrap();
        assert!(high_quality_thumbnails_enabled(&connection));
        set_setting(&connection, THUMBNAIL_HIGH_QUALITY_SETTING_KEY, "false").unwrap();
        assert!(!high_quality_thumbnails_enabled(&connection));
    }

    #[test]
    fn zoom_animations_default_on_and_can_be_disabled() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)")
            .unwrap();

        assert!(zoom_animations_enabled(&connection));
        set_setting(&connection, ZOOM_ANIMATIONS_ENABLED_SETTING_KEY, "false").unwrap();
        assert!(!zoom_animations_enabled(&connection));
        set_setting(&connection, ZOOM_ANIMATIONS_ENABLED_SETTING_KEY, "true").unwrap();
        assert!(zoom_animations_enabled(&connection));
    }

    #[test]
    fn sidebar_rating_counts_exclude_trashed_photos() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(crate::db::SCHEMA).unwrap();
        for (id, rating, favorite, trashed) in [
            (1, 1, 1, 0),
            (2, 1, 0, 0),
            (3, 2, 0, 0),
            (4, 3, 0, 0),
            (5, 4, 0, 0),
            (6, 5, 0, 0),
            (7, 5, 0, 1),
        ] {
            connection
                .execute(
                    "INSERT INTO photos(id, path, rating, favorite, trashed) VALUES (?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![id, format!("/{id}.jpg"), rating, favorite, trashed],
                )
                .unwrap();
        }

        let counts = sidebar_counts(&connection).unwrap();

        assert_eq!(counts.ratings, [2, 1, 1, 1, 1]);
        assert_eq!(counts.photos, 6);
        assert_eq!(counts.favorites, 1);
    }
}
