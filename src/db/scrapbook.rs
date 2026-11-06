// Optional user-authored photo tags and notes for the Scrapbook inspector.
// Photo IDs reference the existing library catalog. No example labels or
// notes are preloaded; edits remain in the local SQLite library only.
pub fn scrapbook_details(connection: &Connection, photo_id: i64) -> Result<(Vec<String>, String)> {
    let row: Option<(String, String)> = connection
        .query_row(
            "SELECT tags,note FROM scrapbook_details WHERE photo_id=?1",
            [photo_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((tags, note)) = row else {
        return Ok((Vec::new(), String::new()));
    };
    Ok((serde_json::from_str::<Vec<String>>(&tags).unwrap_or_default(), note))
}

pub fn scrapbook_add_tag(connection: &Connection, photo_id: i64, new_tag: &str) -> Result<()> {
    let tag = new_tag.trim();
    if tag.is_empty() || tag.chars().count() > 40 {
        anyhow::bail!("Tag must contain 1 to 40 characters");
    }
    let (mut tags, _) = scrapbook_details(connection, photo_id)?;
    if !tags.iter().any(|existing| existing.eq_ignore_ascii_case(tag)) {
        tags.push(tag.to_string());
    }
    let encoded = serde_json::to_string(&tags)?;
    connection.execute(
        "INSERT INTO scrapbook_details(photo_id,tags) VALUES (?1,?2)
         ON CONFLICT(photo_id) DO UPDATE SET tags=excluded.tags",
        params![photo_id, encoded],
    )?;
    Ok(())
}

pub fn scrapbook_remove_tag(connection: &Connection, photo_id: i64, tag: &str) -> Result<()> {
    let (mut tags, _) = scrapbook_details(connection, photo_id)?;
    tags.retain(|old| old != tag);
    connection.execute(
        "INSERT INTO scrapbook_details(photo_id,tags) VALUES (?1,?2)
         ON CONFLICT(photo_id) DO UPDATE SET tags=excluded.tags",
        params![photo_id, serde_json::to_string(&tags)?],
    )?;
    Ok(())
}

pub fn scrapbook_set_note(connection: &Connection, photo_id: i64, note: &str) -> Result<()> {
    if note.chars().count() > 2000 {
        anyhow::bail!("A scrapbook note cannot exceed 2,000 characters");
    }
    connection.execute(
        "INSERT INTO scrapbook_details(photo_id,note) VALUES (?1,?2)
         ON CONFLICT(photo_id) DO UPDATE SET note=excluded.note",
        params![photo_id, note],
    )?;
    Ok(())
}

#[cfg(test)]
mod scrapbook_detail_tests {
    use super::*;

    #[test]
    fn tags_and_notes_are_real_erasable_database_records() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(SCHEMA).unwrap();
        connection.execute("INSERT INTO photos(id,path) VALUES (1,'/test.jpg')", []).unwrap();
        assert_eq!(scrapbook_details(&connection, 1).unwrap(), (Vec::new(), String::new()));
        scrapbook_add_tag(&connection, 1, "Family").unwrap();
        scrapbook_add_tag(&connection, 1, "family").unwrap();
        scrapbook_set_note(&connection, 1, "Beach day").unwrap();
        assert_eq!(scrapbook_details(&connection, 1).unwrap(),
            (vec!["Family".to_string()], "Beach day".to_string()));
        scrapbook_remove_tag(&connection, 1, "Family").unwrap();
        assert!(scrapbook_details(&connection, 1).unwrap().0.is_empty());
        connection.execute("DELETE FROM photos WHERE id=1", []).unwrap();
        assert!(connection.query_row(
            "SELECT 1 FROM scrapbook_details WHERE photo_id=1", [], |_| Ok(())
        ).optional().unwrap().is_none());
    }
}
