pub const HOME_PREVIEW_LIMIT: i64 = 20;
pub const SCRAPBOOK_ALBUM_LIMIT: i64 = 200;
pub const SCRAPBOOK_STRIP_LIMIT: i64 = 100;

pub struct LibraryHomeData {
    pub photo_count: i64,
    pub favorite_count: i64,
    pub folder_count: i64,
    pub added: Vec<Photo>,
    pub edited: Vec<Photo>,
    pub favorites: Vec<Photo>,
    pub albums: Vec<(Album, Option<Photo>)>,
    /// Independent album shelf, not limited to classic dashboard preview.
    pub scrapbook_albums: Vec<(Album, Option<Photo>)>,
    pub scrapbook_album_id: Option<i64>,
    /// Bounded feature gallery for the paper/scrapbook Library dashboard.
    /// Empty libraries fall back to the same recent-photo preview data.
    pub scrapbook_photos: Vec<Photo>,
    pub scrapbook_title: String,
    pub scrapbook_total: i64,
}

/// Bounded, database-only queries. Retained as a test helper.
#[cfg(test)]
pub fn library_home_data(connection: &Connection) -> Result<LibraryHomeData> {
    library_home_data_for_album(connection, None)
}

/// The scrapbook row reloads independently of other navigation destinations.
pub fn library_home_data_for_album(
    connection: &Connection,
    selected_album_id: Option<i64>,
) -> Result<LibraryHomeData> {
    let added = {
        let mut query = connection.prepare(
            "SELECT p.id,p.path,p.folder_id,p.taken_at,p.camera,CAST(p.aperture AS REAL),p.width,p.height,p.size_bytes,p.mtime,p.added_at,p.rotation,p.edit_recipe,p.favorite,p.rating,p.trashed,f.path,p.lens,p.shutter_speed,p.iso,p.focal_length,p.exposure_bias
             FROM photos p LEFT JOIN folders f ON f.id=p.folder_id
             WHERE p.trashed=0 ORDER BY p.added_at DESC,p.id DESC LIMIT ?1",
        )?;
        let rows = query.query_map([HOME_PREVIEW_LIMIT], photo_from_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    let mut query = connection.prepare(
        "WITH preview AS (SELECT * FROM albums ORDER BY name COLLATE NOCASE,id LIMIT ?1)
         SELECT a.id,a.name,a.created_at,COUNT(p.id),a.cover_frame,a.cover_photo_id
         FROM preview a LEFT JOIN album_photos ap ON ap.album_id=a.id
         LEFT JOIN photos p ON p.id=ap.photo_id AND p.trashed=0
         GROUP BY a.id ORDER BY a.name COLLATE NOCASE,a.id",
    )?;
    let albums = query
        .query_map([HOME_PREVIEW_LIMIT], |row| {
            Ok(Album {
                id: row.get(0)?,
                name: row.get(1)?,
                created_at: row.get(2)?,
                photo_count: row.get(3)?,
                cover_frame: row.get(4)?,
                cover_photo_id: row.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let albums =
        albums
            .into_iter()
            .map(|album| -> Result<_> {
                let chosen = album
                    .cover_photo_id
                    .map(|id| photo(connection, id))
                    .transpose()?
                    .flatten()
                    .filter(|photo| !photo.trashed);
                let cover =
                    match chosen {
                        Some(photo) => Some(photo),
                        None => {
                            let id = connection.query_row(
                    "SELECT p.id FROM album_photos ap JOIN photos p ON p.id=ap.photo_id
                     WHERE ap.album_id=?1 AND p.trashed=0
                     ORDER BY p.taken_at IS NULL,p.taken_at DESC,p.path COLLATE NOCASE LIMIT 1",
                    [album.id], |row| row.get::<_,i64>(0),
                ).optional()?;
                            id.map(|id| photo(connection, id)).transpose()?.flatten()
                        }
                    };
                Ok((album, cover))
            })
            .collect::<Result<Vec<_>>>()?;
    // Keep classic Library previews bounded while offering the Scrapbook
    // shelf its own horizontally scrollable collection of album covers.
    let mut scrapbook_albums = Vec::new();
    for album in crate::db::albums(connection)?
        .into_iter()
        .take(SCRAPBOOK_ALBUM_LIMIT as usize)
    {
        let cover = album.cover_photo_id
            .map(|id| photo(connection, id))
            .transpose()?
            .flatten()
            .filter(|photo| !photo.trashed);
        let cover = if cover.is_some() {
            cover
        } else {
            let id = connection.query_row(
                "SELECT p.id FROM album_photos ap JOIN photos p ON p.id=ap.photo_id
                 WHERE ap.album_id=?1 AND p.trashed=0
                 ORDER BY p.taken_at IS NULL,p.taken_at DESC,p.path COLLATE NOCASE
                 LIMIT 1",
                [album.id],
                |row| row.get::<_, i64>(0),
            ).optional()?;
            id.map(|id| photo(connection, id)).transpose()?.flatten()
        };
        scrapbook_albums.push((album, cover));
    }
    let featured = selected_album_id
        .and_then(|id| scrapbook_albums.iter().find(|(album, _)| album.id == id))
        .or_else(|| scrapbook_albums.iter()
            .filter(|(album, _)| album.photo_count > 0)
            .max_by_key(|(album, _)| album.photo_count))
        .map(|(album, _)| (album.id, album.name.clone(), album.photo_count));
    let scrapbook_album_id = featured.as_ref().map(|(id, _, _)| *id);
    let (scrapbook_photos, scrapbook_title, scrapbook_total) =
        if let Some((album_id, title, count)) = featured {
            let mut query = connection.prepare(
                "SELECT p.id,p.path,p.folder_id,p.taken_at,p.camera,CAST(p.aperture AS REAL),p.width,p.height,p.size_bytes,p.mtime,p.added_at,p.rotation,p.edit_recipe,p.favorite,p.rating,p.trashed,f.path,p.lens,p.shutter_speed,p.iso,p.focal_length,p.exposure_bias
                 FROM album_photos ap JOIN photos p ON p.id=ap.photo_id
                 LEFT JOIN folders f ON f.id=p.folder_id
                 WHERE ap.album_id=?1 AND p.trashed=0
                 ORDER BY p.taken_at IS NULL,p.taken_at DESC,p.path COLLATE NOCASE
                 LIMIT ?2",
            )?;
            let rows = query.query_map(params![album_id, SCRAPBOOK_STRIP_LIMIT], photo_from_row)?;
            (rows.collect::<rusqlite::Result<Vec<_>>>()?, title, count)
        } else {
            (added.clone(), "Recently Added".to_string(), added.len() as i64)
        };
    Ok(LibraryHomeData {
        scrapbook_albums,
        scrapbook_album_id,
        photo_count: connection.query_row("SELECT COUNT(*) FROM photos WHERE trashed=0", [], |r| r.get(0))?,
        favorite_count: connection.query_row("SELECT COUNT(*) FROM photos WHERE trashed=0 AND favorite=1", [], |r| r.get(0))?,
        folder_count: connection.query_row("SELECT COUNT(*) FROM folders WHERE imported_root=1", [], |r| r.get(0))?,
        added,
        edited: history_photos_limited(connection, HOME_PREVIEW_LIMIT)?,
        favorites: photos_limited(connection, None, true, None, HOME_PREVIEW_LIMIT)?,
        albums,
        scrapbook_photos,
        scrapbook_title,
        scrapbook_total,
    })
}

#[cfg(test)]
mod home_tests {
    use super::*;

    #[test]
    fn home_previews_are_bounded_ordered_and_follow_favorite_changes() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(SCHEMA).unwrap();
        for id in 1..=100 {
            c.execute(
                "INSERT INTO photos(id,path,added_at,favorite,taken_at) VALUES (?1,?2,?1,1,?3)",
                params![
                    id,
                    format!("smb://offline/{id:03}.jpg"),
                    format!("2026-01-{:02}", id % 28 + 1)
                ],
            )
            .unwrap();
            set_edit_recipe(&c, id, "edited").unwrap();
        }
        c.execute("UPDATE photos SET trashed=1 WHERE id=100", [])
            .unwrap();
        let data = library_home_data(&c).unwrap();
        assert_eq!(data.photo_count, 99);
        assert_eq!(data.favorite_count, 99);
        assert_eq!(data.folder_count, 0);
        assert_eq!(
            data.added.iter().map(|p| p.id).collect::<Vec<_>>(),
            (80..=99).rev().collect::<Vec<_>>()
        );
        assert_eq!(
            data.edited.iter().map(|p| p.id).collect::<Vec<_>>(),
            (80..=99).rev().collect::<Vec<_>>()
        );
        let expected = photos(&c, None, true, None)
            .unwrap()
            .into_iter()
            .take(HOME_PREVIEW_LIMIT as usize)
            .map(|p| p.id)
            .collect::<Vec<_>>();
        assert_eq!(
            data.favorites.iter().map(|p| p.id).collect::<Vec<_>>(),
            expected
        );
        let removed = data.favorites[0].id;
        set_favorite(&c, removed, false).unwrap();
        let refreshed = library_home_data(&c).unwrap();
        assert_eq!(refreshed.favorites.len(), HOME_PREVIEW_LIMIT as usize);
        assert!(!refreshed.favorites.iter().any(|p| p.id == removed));
        assert_eq!(
            c.query_row("SELECT count(*) FROM editing_events", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            100
        );
    }


    #[test]
    fn scrapbook_selected_album_changes_photo_strip_without_changing_album_list() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(SCHEMA).unwrap();
        for id in 1..=6_i64 {
            c.execute("INSERT INTO photos(id,path,added_at) VALUES (?1,?2,?1)",
                params![id, format!("/tmp/scrapbook-{id}.jpg")]).unwrap();
        }
        let first = create_album(&c, "First").unwrap();
        let second = create_album(&c, "Second").unwrap();
        add_photos_to_album(&c, first.id, &[1,2]).unwrap();
        add_photos_to_album(&c, second.id, &[3,4,5]).unwrap();
        let before = library_home_data_for_album(&c, Some(first.id)).unwrap();
        let after = library_home_data_for_album(&c, Some(second.id)).unwrap();
        assert_eq!(before.scrapbook_album_id, Some(first.id));
        assert_eq!(before.scrapbook_title, "First");
        assert_eq!(before.scrapbook_total, 2);
        assert_eq!(before.scrapbook_photos.iter().map(|p| p.id).collect::<Vec<_>>(), vec![1,2]);
        assert_eq!(after.scrapbook_album_id, Some(second.id));
        assert_eq!(after.scrapbook_title, "Second");
        assert_eq!(after.scrapbook_total, 3);
        assert_eq!(after.scrapbook_photos.iter().map(|p| p.id).collect::<Vec<_>>(), vec![3,4,5]);
        assert_eq!(before.scrapbook_albums.len(), 2);
        assert_eq!(after.scrapbook_albums.len(), 2);
        // The classic Library album preview remains unchanged by the request.
        assert_eq!(before.albums.len(), after.albums.len());
    }

    #[test]
    fn home_album_previews_keep_covers_counts_and_empty_albums() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(SCHEMA).unwrap();
        c.execute("INSERT INTO photos(id,path,trashed) VALUES (1,'/one.jpg',0),(2,'/two.jpg',0),(3,'/trash.jpg',1)",[]).unwrap();
        for index in 0..10 {
            create_album(&c, &format!("Album {index}")).unwrap();
        }
        add_photos_to_album(&c, 1, &[1, 2, 3]).unwrap();
        set_album_cover_photo(&c, 1, 2).unwrap();
        let data = library_home_data(&c).unwrap();
        assert_eq!(data.albums.len(), 10);
        assert_eq!(data.albums[0].0.photo_count, 2);
        assert_eq!(data.albums[0].1.as_ref().unwrap().id, 2);
        assert_eq!(data.albums[1].0.photo_count, 0);
        assert!(data.albums[1].1.is_none());
    }
}
