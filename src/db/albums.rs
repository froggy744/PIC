/// Columns `album_from_row` reads, for a query over `albums a` whose fourth
/// column is the manual membership count.
const ALBUM_COLUMNS: &str = "a.id,a.name,a.created_at,COUNT(p.id),a.cover_frame,a.cover_photo_id,a.rules";

fn album_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Album> {
    Ok(Album {
        id: row.get(0)?,
        name: row.get(1)?,
        created_at: row.get(2)?,
        photo_count: row.get(3)?,
        cover_frame: row.get(4)?,
        cover_photo_id: row.get(5)?,
        rules: row.get(6)?,
    })
}

/// SQL condition (over `p`) for the photos an album holds: its manual
/// members, or the photos its Smart Album rules match. Parameters are
/// numbered from `first`. A missing album or unreadable rules match nothing,
/// never the whole library.
pub fn album_membership(
    connection: &Connection,
    album_id: i64,
    first: usize,
) -> Result<(String, Vec<rusqlite::types::Value>)> {
    let rules = connection
        .query_row("SELECT rules FROM albums WHERE id = ?1", [album_id], |row| {
            row.get::<_, Option<String>>(0)
        })
        .optional()?;
    Ok(match rules {
        None => ("0".into(), Vec::new()),
        Some(None) => (
            format!("p.id IN (SELECT photo_id FROM album_photos WHERE album_id = ?{first})"),
            vec![album_id.into()],
        ),
        Some(Some(json)) => smart_membership(album_id, &json, first),
    })
}

fn smart_membership(
    album_id: i64,
    json: &str,
    first: usize,
) -> (String, Vec<rusqlite::types::Value>) {
    match filters::AlbumRules::parse(json) {
        Ok(rules) => rules.predicate(first),
        Err(error) => {
            eprintln!("Smart Album {album_id} has unusable rules and shows no photos: {error}");
            ("0".into(), Vec::new())
        }
    }
}

/// Smart Albums hold no `album_photos` rows; count what their rules match,
/// narrowed by `browse` (`None`: whole-album counts).
fn fill_smart_album_counts(
    connection: &Connection,
    albums: &mut [Album],
    browse: Option<&BrowseScope>,
) -> Result<()> {
    let mut browse_args = Vec::new();
    let browse_sql = browse.map_or_else(|| "1".to_owned(), |scope| scope.narrowing(&mut browse_args));
    for album in albums.iter_mut() {
        let Some(json) = album.rules.as_deref() else {
            continue;
        };
        let (sql, args) = smart_membership(album.id, json, browse_args.len() + 1);
        let mut values = browse_args.clone();
        values.extend(args);
        album.photo_count = connection.query_row(
            &format!(
                "SELECT COUNT(*) FROM photos p WHERE p.trashed = 0 AND ({browse_sql}) AND ({sql})"
            ),
            rusqlite::params_from_iter(values),
            |row| row.get(0),
        )?;
    }
    Ok(())
}

pub fn albums(connection: &Connection) -> Result<Vec<Album>> {
    let mut statement = connection.prepare(&format!(
        "SELECT {ALBUM_COLUMNS}
         FROM albums a
         LEFT JOIN album_photos ap ON ap.album_id = a.id
         LEFT JOIN photos p ON p.id = ap.photo_id AND p.trashed = 0
         GROUP BY a.id
         ORDER BY a.name COLLATE NOCASE, a.id"
    ))?;
    let mut albums = statement
        .query_map([], album_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    fill_smart_album_counts(connection, &mut albums, None)?;
    Ok(albums)
}

/// The albums, manual and Smart, that hold `photo_id`, ordered like `albums`.
/// Membership is `album_membership`, so this agrees with every album view.
pub fn albums_containing_photo(connection: &Connection, photo_id: i64) -> Result<Vec<Album>> {
    let mut containing = Vec::new();
    for album in albums(connection)? {
        let (membership, args) = album_membership(connection, album.id, 2)?;
        let mut values: Vec<rusqlite::types::Value> = vec![photo_id.into()];
        values.extend(args);
        let holds: bool = connection.query_row(
            &format!(
                "SELECT EXISTS(SELECT 1 FROM photos p
                 WHERE p.id = ?1 AND p.trashed = 0 AND ({membership}))"
            ),
            rusqlite::params_from_iter(values),
            |row| row.get(0),
        )?;
        if holds {
            containing.push(album);
        }
    }
    Ok(containing)
}

/// Whether any Smart Album exists, so photo changes can skip recounting
/// album rows in libraries that have none.
pub fn has_smart_albums(connection: &Connection) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM albums WHERE rules IS NOT NULL)",
        [],
        |row| row.get(0),
    )?)
}

/// Albums photos can be added to by hand: Smart Albums are read-only photo
/// sources and never destinations.
pub fn manual_albums(connection: &Connection) -> Result<Vec<Album>> {
    let mut statement = connection.prepare(&format!(
        "SELECT {ALBUM_COLUMNS}
         FROM albums a
         LEFT JOIN album_photos ap ON ap.album_id = a.id
         LEFT JOIN photos p ON p.id = ap.photo_id AND p.trashed = 0
         WHERE a.rules IS NULL
         GROUP BY a.id
         ORDER BY a.name COLLATE NOCASE, a.id"
    ))?;
    let albums = statement
        .query_map([], album_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(albums)
}

/// Album counts narrowed by `scope`'s metadata filters and rating
/// (`BrowseScope::narrowing`); administrative album APIs stay unfiltered.
pub fn browse_albums(connection: &Connection, scope: &BrowseScope) -> Result<Vec<Album>> {
    let mut args = Vec::new();
    let predicate = scope.narrowing(&mut args);
    let mut statement = connection.prepare(&format!(
        "SELECT {ALBUM_COLUMNS}
         FROM albums a LEFT JOIN album_photos ap ON ap.album_id=a.id
         LEFT JOIN photos p ON p.id=ap.photo_id AND p.trashed=0 AND ({predicate})
         GROUP BY a.id ORDER BY a.name COLLATE NOCASE,a.id"
    ))?;
    let mut albums = statement
        .query_map(rusqlite::params_from_iter(args), album_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    fill_smart_album_counts(connection, &mut albums, Some(scope))?;
    Ok(albums)
}

/// Albums whose name contains `query` (case-insensitive), for the search
/// suggestions. Counts are narrowed by `scope` like `browse_albums`.
/// Exact names rank first, then prefix matches, then the rest by name.
pub fn search_albums(
    connection: &Connection,
    scope: &BrowseScope,
    query: &str,
    limit: usize,
) -> Result<Vec<Album>> {
    let query = query.trim();
    if query.chars().count() < 2 || limit == 0 {
        return Ok(Vec::new());
    }
    let mut values: Vec<rusqlite::types::Value> = vec![query.to_owned().into(), (limit as i64).into()];
    let predicate = scope.narrowing(&mut values);
    let mut statement = connection.prepare(&format!(
        "SELECT {ALBUM_COLUMNS}
         FROM albums a LEFT JOIN album_photos ap ON ap.album_id=a.id
         LEFT JOIN photos p ON p.id=ap.photo_id AND p.trashed=0 AND ({predicate})
         WHERE instr(lower(a.name), lower(?1)) > 0
         GROUP BY a.id
         ORDER BY
           CASE
             WHEN lower(a.name) = lower(?1) THEN 0
             WHEN instr(lower(a.name), lower(?1)) = 1 THEN 1
             ELSE 2
           END,
           a.name COLLATE NOCASE, a.id
         LIMIT ?2"
    ))?;
    let mut albums = statement
        .query_map(rusqlite::params_from_iter(values), album_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    fill_smart_album_counts(connection, &mut albums, Some(scope))?;
    Ok(albums)
}

pub fn create_album(connection: &Connection, name: &str) -> Result<Album> {
    insert_album(connection, name, None)
}

/// Create a Smart Album whose photos are chosen by `rules`. Zero matching
/// photos is fine; an album without a valid rule is not.
pub fn create_smart_album(
    connection: &Connection,
    name: &str,
    rules: &filters::AlbumRules,
) -> Result<Album> {
    rules.validate()?;
    let mut album = insert_album(connection, name, Some(rules.to_json()?))?;
    fill_smart_album_counts(
        connection,
        std::slice::from_mut(&mut album),
        None,
    )?;
    Ok(album)
}

/// The checks a new album name must pass, so a two-step creation can report
/// a bad name before its second step.
pub fn check_new_album_name(connection: &Connection, name: &str) -> Result<()> {
    let name = name.trim();
    if name.is_empty() {
        anyhow::bail!("album name cannot be blank");
    }
    let duplicate = connection
        .query_row(
            "SELECT id FROM albums WHERE name = ?1 COLLATE NOCASE",
            [name],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    if duplicate.is_some() {
        anyhow::bail!("an album named ‘{name}’ already exists");
    }
    Ok(())
}

fn insert_album(connection: &Connection, name: &str, rules: Option<String>) -> Result<Album> {
    check_new_album_name(connection, name)?;
    let name = name.trim();
    connection.execute(
        "INSERT INTO albums(name, created_at, rules)
         VALUES (?1, CAST(strftime('%s', 'now') AS INTEGER), ?2)",
        params![name, rules],
    )?;
    let id = connection.last_insert_rowid();
    Ok(Album {
        id,
        name: name.to_string(),
        created_at: connection.query_row(
            "SELECT created_at FROM albums WHERE id = ?1",
            [id],
            |row| row.get(0),
        )?,
        photo_count: 0,
        cover_frame: None,
        cover_photo_id: None,
        rules,
    })
}

/// A Smart Album's saved rules; `None` for a manual album.
pub fn album_rules(connection: &Connection, album_id: i64) -> Result<Option<filters::AlbumRules>> {
    let json = connection
        .query_row("SELECT rules FROM albums WHERE id = ?1", [album_id], |row| {
            row.get::<_, Option<String>>(0)
        })
        .optional()?
        .ok_or_else(|| anyhow::anyhow!("album no longer exists"))?;
    json.map(|json| filters::AlbumRules::parse(&json)).transpose()
}

/// Replace a Smart Album's rules. The album keeps its id, name, cover and
/// presentation settings; a manual album cannot gain rules.
pub fn set_album_rules(
    connection: &Connection,
    album_id: i64,
    rules: &filters::AlbumRules,
) -> Result<()> {
    rules.validate()?;
    let updated = connection.execute(
        "UPDATE albums SET rules = ?1 WHERE id = ?2 AND rules IS NOT NULL",
        params![rules.to_json()?, album_id],
    )?;
    if updated == 0 {
        anyhow::bail!("Smart Album no longer exists");
    }
    Ok(())
}

fn ensure_manual_album(connection: &Connection, album_id: i64) -> Result<()> {
    let smart = connection
        .query_row(
            "SELECT rules IS NOT NULL FROM albums WHERE id = ?1",
            [album_id],
            |row| row.get::<_, bool>(0),
        )
        .optional()?;
    match smart {
        None => anyhow::bail!("album no longer exists"),
        Some(true) => anyhow::bail!("a Smart Album chooses its photos from its rules"),
        Some(false) => Ok(()),
    }
}

/// Photos in an album, newest first, at most `limit` (`-1` for all). Uses the
/// album's membership, so Smart Albums work for every album consumer.
pub fn album_photos_page(connection: &Connection, album_id: i64, limit: i64) -> Result<Vec<Photo>> {
    let (membership, args) = album_membership(connection, album_id, 2)?;
    let mut statement = connection.prepare(&format!(
        "SELECT p.id,p.path,p.folder_id,p.taken_at,p.camera,CAST(p.aperture AS REAL),p.width,p.height,p.size_bytes,p.mtime,p.added_at,p.rotation,p.edit_recipe,p.favorite,p.rating,p.trashed,f.path,p.lens,p.shutter_speed,p.iso,p.focal_length,p.exposure_bias
         FROM photos p LEFT JOIN folders f ON f.id = p.folder_id
         WHERE p.trashed = 0 AND ({membership})
         ORDER BY p.taken_at IS NULL, p.taken_at DESC, p.path COLLATE NOCASE
         LIMIT ?1"
    ))?;
    let mut values: Vec<rusqlite::types::Value> = vec![limit.into()];
    values.extend(args);
    let rows = statement.query_map(rusqlite::params_from_iter(values), photo_from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Rename an album while retaining the same validation as album creation.
pub fn rename_album(connection: &Connection, album_id: i64, name: &str) -> Result<()> {
    let name = name.trim();
    if name.is_empty() {
        anyhow::bail!("album name cannot be blank");
    }
    let updated = connection.execute(
        "UPDATE albums SET name = ?1 WHERE id = ?2",
        params![name, album_id],
    )?;
    if updated == 0 {
        anyhow::bail!("album no longer exists");
    }
    Ok(())
}

pub fn set_album_cover_frame(
    connection: &Connection,
    album_id: i64,
    cover_frame: &str,
) -> Result<()> {
    connection.execute(
        "UPDATE albums SET cover_frame = ?1 WHERE id = ?2",
        params![cover_frame, album_id],
    )?;
    Ok(())
}

/// Drop an album's own frame so it follows the page's automatic assignment
/// again.
pub fn clear_album_cover_frame(connection: &Connection, album_id: i64) -> Result<()> {
    connection.execute(
        "UPDATE albums SET cover_frame = NULL WHERE id = ?1",
        [album_id],
    )?;
    Ok(())
}

/// Drop every album's own frame, returning the number that had one.
pub fn clear_all_album_cover_frames(connection: &Connection) -> Result<usize> {
    Ok(connection.execute(
        "UPDATE albums SET cover_frame = NULL WHERE cover_frame IS NOT NULL",
        [],
    )?)
}

/// Choose the photo the album shows on its card.
pub fn set_album_cover_photo(connection: &Connection, album_id: i64, photo_id: i64) -> Result<()> {
    connection.execute(
        "UPDATE albums SET cover_photo_id = ?1 WHERE id = ?2",
        params![photo_id, album_id],
    )?;
    Ok(())
}

/// Drop one album's chosen cover photo so it goes back to the automatic pick.
pub fn clear_album_cover_photo(connection: &Connection, album_id: i64) -> Result<()> {
    connection.execute(
        "UPDATE albums SET cover_photo_id = NULL WHERE id = ?1",
        [album_id],
    )?;
    Ok(())
}

/// Drop every album's chosen cover photo, returning the number that had one.
pub fn clear_all_album_cover_photos(connection: &Connection) -> Result<usize> {
    Ok(connection.execute(
        "UPDATE albums SET cover_photo_id = NULL WHERE cover_photo_id IS NOT NULL",
        [],
    )?)
}

pub fn delete_album(connection: &Connection, album_id: i64) -> Result<()> {
    connection.execute("DELETE FROM albums WHERE id = ?1", [album_id])?;
    Ok(())
}

pub fn add_photos_to_album(
    connection: &Connection,
    album_id: i64,
    photo_ids: &[i64],
) -> Result<usize> {
    ensure_manual_album(connection, album_id)?;
    let transaction = connection.unchecked_transaction()?;
    let mut added = 0;
    for photo_id in photo_ids {
        added += transaction.execute(
            "INSERT OR IGNORE INTO album_photos(album_id, photo_id) VALUES (?1, ?2)",
            params![album_id, photo_id],
        )?;
    }
    transaction.commit()?;
    Ok(added)
}

pub fn remove_photos_from_album(
    connection: &Connection,
    album_id: i64,
    photo_ids: &[i64],
) -> Result<usize> {
    ensure_manual_album(connection, album_id)?;
    let transaction = connection.unchecked_transaction()?;
    let mut removed = 0;
    for photo_id in photo_ids {
        removed += transaction.execute(
            "DELETE FROM album_photos WHERE album_id = ?1 AND photo_id = ?2",
            params![album_id, photo_id],
        )?;
    }
    transaction.commit()?;
    Ok(removed)
}
