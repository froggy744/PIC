/// Repair application-owned paths after relocating storage. Photo directories
/// outside the moved application roots retain their original spelling.
fn migrate_stored_paths(connection: &Connection) -> Result<()> {
    let marker = "pic_rs_storage_paths_v1";
    if connection
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            [marker],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .as_deref()
        == Some("done")
    {
        return Ok(());
    }
    let transaction = connection.unchecked_transaction()?;
    let config = dirs::config_dir().context("could not determine configuration directory")?;
    if let (Some(data), Some(cache)) = (dirs::data_dir(), dirs::cache_dir()) {
        if let Some(legacy) = crate::app_paths::legacy_storage_root(&data, &config, "data") {
            rewrite_stored_paths(
                &transaction,
                &legacy.join("picasa-rs/overlays"),
                &cache.join("pic-rs/thumbs/overlay"),
            )?;
        }
        for name in ["picasa-rs", "pic-rs"] {
            rewrite_stored_paths(
                &transaction,
                &data.join(name).join("overlays"),
                &cache.join("pic-rs/thumbs/overlay"),
            )?;
        }
    }
    for (root, kind) in [
        (dirs::data_dir(), "data"),
        (Some(config.clone()), "config"),
        (dirs::cache_dir(), "cache"),
    ] {
        let Some(root) = root else {
            continue;
        };
        rewrite_stored_paths(
            &transaction,
            &root.join("picasa-rs"),
            &root.join(crate::app_paths::APP_DIRECTORY),
        )?;
        if let Some(legacy) = crate::app_paths::legacy_storage_root(&root, &config, kind) {
            rewrite_stored_paths(
                &transaction,
                &legacy.join("picasa-rs"),
                &root.join(crate::app_paths::APP_DIRECTORY),
            )?;
        }
    }
    transaction.execute(
        "INSERT OR REPLACE INTO settings(key,value) VALUES (?1,'done')",
        [marker],
    )?;
    transaction.commit()?;
    Ok(())
}

fn rewrite_stored_paths(connection: &Connection, old: &Path, new: &Path) -> Result<()> {
    for (table, column) in [
        ("photos", "path"),
        ("photos", "edit_recipe"),
        ("folders", "path"),
        ("settings", "value"),
        ("collage_projects", "draft"),
    ] {
        let query = format!("SELECT rowid, {column} FROM {table} WHERE instr({column}, ?1) > 0 OR instr({column}, ?2) > 0");
        let prefix = old.to_string_lossy();
        let escaped = serde_json::to_string(prefix.as_ref())?;
        let escaped = &escaped[1..escaped.len() - 1];
        let rows = connection
            .prepare(&query)?
            .query_map(params![prefix.as_ref(), escaped], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (id, text) in rows {
            let updated = rewrite_path_value(&text, old, new);
            if updated != text {
                connection.execute(
                    &format!("UPDATE {table} SET {column} = ?1 WHERE rowid = ?2"),
                    params![updated, id],
                )?;
            }
        }
    }
    Ok(())
}

fn rewrite_path_value(text: &str, old: &Path, new: &Path) -> String {
    if let Ok(mut value) = serde_json::from_str::<serde_json::Value>(text) {
        fn rewrite(value: &mut serde_json::Value, old: &Path, new: &Path) {
            match value {
                serde_json::Value::String(text) => {
                    *text = crate::app_paths::relocated_path(Path::new(text), old, new)
                        .to_string_lossy()
                        .into_owned()
                }
                serde_json::Value::Array(items) => {
                    for item in items {
                        rewrite(item, old, new);
                    }
                }
                serde_json::Value::Object(items) => {
                    for item in items.values_mut() {
                        rewrite(item, old, new);
                    }
                }
                _ => {}
            }
        }
        let original = value.clone();
        rewrite(&mut value, old, new);
        if value != original {
            return serde_json::to_string(&value).expect("JSON value is serializable");
        }
        return text.to_owned();
    }
    crate::app_paths::relocated_path(Path::new(text), old, new)
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod path_migration_tests {
    use super::*;
    #[test]
    fn serialized_json_with_escaped_path_separators_is_repaired() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(SCHEMA).unwrap();
        // Backslashes in a path are escaped in JSON, including on Unix. This
        // exercises the SQL candidate filter used for Windows paths too.
        let old = Path::new(r"/home/test/a\b/picasa-rs");
        let new = Path::new(r"/home/test/a\b/pic-rs");
        let original = serde_json::json!({"directory": old.join("exports")});
        connection
            .execute(
                "INSERT INTO settings(key,value) VALUES ('paths',?1)",
                [original.to_string()],
            )
            .unwrap();
        rewrite_stored_paths(&connection, old, new).unwrap();
        let text: String = connection
            .query_row("SELECT value FROM settings", [], |row| row.get(0))
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            value["directory"],
            new.join("exports").to_string_lossy().as_ref()
        );
    }

    #[test]
    fn repairs_managed_paths_in_database_and_json_without_renaming_user_photos() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(SCHEMA).unwrap();
        let old = Path::new("/home/test/.local/share/picasa-rs");
        let new = Path::new("/home/test/.local/share/pic-rs");
        connection.execute("INSERT INTO photos(path, edit_recipe) VALUES (?1, ?2)", params!["/photos/picasa-rs/family.jpg", r#"{"source":"/home/test/.local/share/picasa-rs/image.png","label":"picasa-rs"}"#]).unwrap();
        connection
            .execute(
                "INSERT INTO folders(path) VALUES (?1)",
                ["/home/test/.local/share/picasa-rs/exports"],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO settings(key,value) VALUES ('export_directory',?1)",
                ["/home/test/.local/share/picasa-rs/exports"],
            )
            .unwrap();
        connection.execute("INSERT INTO collage_projects(photo_id,draft) VALUES (1,?1)", [r#"{"paths":["/home/test/.local/share/picasa-rs/image.jpg","/photos/picasa/image.jpg"]}"#]).unwrap();
        rewrite_stored_paths(&connection, old, new).unwrap();
        let folder: String = connection
            .query_row("SELECT path FROM folders", [], |row| row.get(0))
            .unwrap();
        assert_eq!(folder, "/home/test/.local/share/pic-rs/exports");
        let photo: String = connection
            .query_row("SELECT path FROM photos", [], |row| row.get(0))
            .unwrap();
        assert_eq!(photo, "/photos/picasa-rs/family.jpg");
        let recipe: String = connection
            .query_row("SELECT edit_recipe FROM photos", [], |row| row.get(0))
            .unwrap();
        let recipe: serde_json::Value = serde_json::from_str(&recipe).unwrap();
        assert_eq!(recipe["source"], "/home/test/.local/share/pic-rs/image.png");
        assert_eq!(recipe["label"], "picasa-rs");
        let draft: String = connection
            .query_row("SELECT draft FROM collage_projects", [], |row| row.get(0))
            .unwrap();
        assert!(draft.contains("share/pic-rs/image.jpg"));
        assert!(draft.contains("/photos/picasa/image.jpg"));
        let setting: String = connection
            .query_row("SELECT value FROM settings", [], |row| row.get(0))
            .unwrap();
        assert_eq!(setting, "/home/test/.local/share/pic-rs/exports");
        rewrite_stored_paths(&connection, old, new).unwrap();
    }
}
