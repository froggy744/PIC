//! Application storage locations and migration from the former folder name.
use std::{
    fs, io,
    path::{Path, PathBuf},
    collections::HashSet,
};

pub const APP_DIRECTORY: &str = "pic-rs";
const LEGACY_DIRECTORY: &str = "picasa-rs";

const CURRENT_FLATPAK_ID: &str = "io.github.froggy744.PIC";
const LEGACY_FLATPAK_IDS: [&str; 2] = ["io.github.you.PicRs", "io.github.you.PicasaRs"];

/// Former Flatpak sandbox roots, when this root belongs to the renamed app ID.
pub fn legacy_sandbox_roots(root: &Path) -> Vec<PathBuf> {
    let Some(app) = root.parent() else {
        return Vec::new();
    };
    if app.file_name().and_then(|name| name.to_str()) != Some(CURRENT_FLATPAK_ID) {
        return Vec::new();
    }
    let Some(apps_root) = app.parent() else {
        return Vec::new();
    };
    let Some(kind) = root.file_name() else {
        return Vec::new();
    };
    LEGACY_FLATPAK_IDS
        .iter()
        .map(|id| apps_root.join(id).join(kind))
        .collect()
}

/// Packaged launchers use host data/cache roots but sandboxed config. Discover
/// former sandboxes through either location so that mixed layout also works.
pub fn legacy_storage_roots(root: &Path, config: &Path, kind: &str) -> Vec<PathBuf> {
    let direct = legacy_sandbox_roots(root);
    if !direct.is_empty() {
        return direct;
    }
    legacy_sandbox_roots(config)
        .into_iter()
        .filter_map(|legacy_config| legacy_config.parent().map(|app| app.join(kind)))
        .collect()
}

pub fn storage_path(path: &Path, root: &Path, config: &Path, kind: &str) -> PathBuf {
    let destination = root.join(APP_DIRECTORY);
    let mut result = relocated_path(path, &root.join(LEGACY_DIRECTORY), &destination);
    for legacy in legacy_storage_roots(root, config, kind) {
        for directory in [LEGACY_DIRECTORY, APP_DIRECTORY] {
            result = relocated_path(&result, &legacy.join(directory), &destination);
        }
    }
    result
}

static MIGRATION: std::sync::OnceLock<Result<(), String>> = std::sync::OnceLock::new();

/// Database recovery must not bypass a failed storage migration and rewrite
/// paths to files that have not moved. Standalone database tests/tools have no
/// initialized migration and are unaffected.
pub fn migration_status() -> io::Result<()> {
    match MIGRATION.get() {
        Some(Err(message)) => Err(io::Error::other(message.clone())),
        _ => Ok(()),
    }
}

pub fn initialize(data: &Path, config: &Path, cache: &Path) -> io::Result<()> {
    match MIGRATION
        .get_or_init(|| migrate_roots(data, config, cache).map_err(|error| error.to_string()))
    {
        Ok(()) => Ok(()),
        Err(message) => Err(io::Error::other(message.clone())),
    }
}

/// Run before opening any database or rendering overlays. Errors stop startup
/// rather than opening a new empty catalog or replacing existing user data.
pub fn migrate_roots(data: &Path, config: &Path, cache: &Path) -> io::Result<()> {
    let mut moves = Vec::new();
    for (root, kind) in [(data, "data"), (config, "config"), (cache, "cache")] {
        let movement = (root.join(LEGACY_DIRECTORY), root.join(APP_DIRECTORY));
        if !moves.contains(&movement) {
            moves.push(movement);
        }
        for legacy in legacy_storage_roots(root, config, kind) {
            for directory in [LEGACY_DIRECTORY, APP_DIRECTORY] {
                let movement = (legacy.join(directory), root.join(APP_DIRECTORY));
                if !moves.contains(&movement) {
                    moves.push(movement);
                }
            }
        }
    }
    for (source, destination) in &moves {
        check_conflicts(source, destination)?;
    }
    // Check all overlay locations before moving any user data.
    let mut overlay_sources = vec![
        data.join(LEGACY_DIRECTORY).join("overlays"),
        data.join(APP_DIRECTORY).join("overlays"),
    ];
    for legacy in legacy_storage_roots(data, config, "data") {
        for directory in [LEGACY_DIRECTORY, APP_DIRECTORY] {
            overlay_sources.push(legacy.join(directory).join("overlays"));
        }
    }
    let mut overlay_destinations = vec![
        cache.join(APP_DIRECTORY).join("thumbs/overlay"),
        cache.join(LEGACY_DIRECTORY).join("thumbs/overlay"),
    ];
    for legacy in legacy_storage_roots(cache, config, "cache") {
        for directory in [LEGACY_DIRECTORY, APP_DIRECTORY] {
            overlay_destinations.push(legacy.join(directory).join("thumbs/overlay"));
        }
    }
    for source in &overlay_sources {
        for destination in &overlay_destinations {
            check_conflicts(source, destination)?;
        }
    }
    // Two legacy roots must not silently replace one another either.
    for (index, (source, destination)) in moves.iter().enumerate() {
        for (other_source, other_destination) in &moves[index + 1..] {
            if destination == other_destination {
                check_conflicts(source, other_source)?;
            }
        }
    }
    for (source, destination) in &moves {
        move_directory(source, destination)?;
    }
    move_directory(
        &data.join(APP_DIRECTORY).join("overlays"),
        &cache.join(APP_DIRECTORY).join("thumbs/overlay"),
    )
}

fn is_sqlite_shm(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with("-shm"))
}

fn is_orphan_sqlite_wal(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let Some(database_name) = name.strip_suffix("-wal") else {
        return false;
    };
    !path.with_file_name(database_name).exists()
}

fn snapshot_discardable_sqlite_sidecars(source: &Path) -> io::Result<HashSet<PathBuf>> {
    let mut discardable = HashSet::new();
    collect_discardable_sqlite_sidecars(source, &mut discardable)?;
    Ok(discardable)
}

fn collect_discardable_sqlite_sidecars(
    path: &Path,
    discardable: &mut HashSet<PathBuf>,
) -> io::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            collect_discardable_sqlite_sidecars(&entry?.path(), discardable)?;
        }
    } else if is_sqlite_shm(path) || is_orphan_sqlite_wal(path) {
        discardable.insert(path.to_path_buf());
    }
    Ok(())
}

fn is_discardable_sqlite_sidecar(path: &Path, discardable: &HashSet<PathBuf>) -> bool {
    discardable.contains(path)
}

fn check_conflicts(source: &Path, destination: &Path) -> io::Result<()> {
    let discardable = snapshot_discardable_sqlite_sidecars(source)?;
    check_conflicts_with_snapshot(source, destination, &discardable)
}

fn check_conflicts_with_snapshot(
    source: &Path,
    destination: &Path,
    discardable: &HashSet<PathBuf>,
) -> io::Result<()> {
    if !source.exists() || !destination.exists() {
        return Ok(());
    }
    // SQLite shared-memory files are transient coordination state. They are
    // recreated from the database/WAL and must never block a storage migration.
    if is_discardable_sqlite_sidecar(source, discardable) {
        return Ok(());
    }
    if source.is_dir() && destination.is_dir() {
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            check_conflicts_with_snapshot(&entry.path(), &destination.join(entry.file_name()), discardable)?;
        }
        return Ok(());
    }
    // Never choose between two catalogs or settings files automatically.
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!(
            "cannot migrate {}: {} already exists; both copies have been preserved",
            source.display(),
            destination.display()
        ),
    ))
}

fn move_directory(source: &Path, destination: &Path) -> io::Result<()> {
    if !source.exists() {
        return Ok(());
    }
    let discardable = snapshot_discardable_sqlite_sidecars(source)?;
    check_conflicts_with_snapshot(source, destination, &discardable)?;
    move_directory_with_snapshot(source, destination, &discardable)
}

fn move_directory_with_snapshot(
    source: &Path,
    destination: &Path,
    discardable: &HashSet<PathBuf>,
) -> io::Result<()> {
    if !source.exists() {
        return Ok(());
    }

    // The snapshot was taken before any migration mutation. A valid WAL
    // therefore stays valid even if its database file is moved first.
    if source.is_file() && is_discardable_sqlite_sidecar(source, discardable) {
        return fs::remove_file(source);
    }

    if source.is_dir() {
        fs::create_dir_all(destination)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            move_directory_with_snapshot(
                &entry.path(),
                &destination.join(entry.file_name()),
                discardable,
            )?;
        }
        return fs::remove_dir(source);
    }

    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    if !destination.exists() && fs::rename(source, destination).is_ok() {
        return Ok(());
    }

    // Data and cache can be on different filesystems. Publish a complete
    // copy and retain the source if copying fails.
    let temporary = destination.with_file_name(format!(
        ".{}.migrating-{}",
        destination.file_name().unwrap().to_string_lossy(),
        std::process::id()
    ));
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let result = (|| {
        let mut input = fs::File::open(source)?;
        io::copy(&mut input, &mut output)?;
        output.sync_all()?;
        drop(output);
        fs::rename(&temporary, destination)
    })();
    if let Err(error) = result {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    fs::remove_file(source)
}

pub fn relocated_path(path: &Path, old: &Path, new: &Path) -> PathBuf {
    match path.strip_prefix(old) {
        Ok(suffix) => new.join(suffix),
        Err(_) => path.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "pic-rs-migration-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&root).unwrap();
            Self(root)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn shared_data_and_config_root_migrates_once() {
        let f = Fixture::new();
        let shared = f.0.join("shared");
        fs::create_dir_all(shared.join("picasa-rs")).unwrap();
        fs::write(shared.join("picasa-rs/library.db"), b"database").unwrap();
        migrate_roots(&shared, &shared, &f.0.join("cache")).unwrap();
        assert!(shared.join("pic-rs/library.db").is_file());
    }

    #[test]
    fn flatpak_host_roots_also_migrate_old_sandbox_assets() {
        let f = Fixture::new();
        let data = f.0.join("share");
        let cache = f.0.join("cache");
        let app = f.0.join(".var/app/io.github.froggy744.PIC");
        let old_app = f.0.join(".var/app/io.github.you.PicasaRs");
        fs::create_dir_all(old_app.join("data/picasa-rs/overlays")).unwrap();
        fs::write(old_app.join("data/picasa-rs/library.db"), b"database").unwrap();
        fs::write(old_app.join("data/picasa-rs/overlays/hash.png"), b"overlay").unwrap();
        migrate_roots(&data, &app.join("config"), &cache).unwrap();
        assert!(data.join("pic-rs/library.db").is_file());
        assert!(cache.join("pic-rs/thumbs/overlay/hash.png").is_file());
    }

    #[test]
    fn renamed_flatpak_migrates_the_former_sandbox_and_registry_paths() {
        let f = Fixture::new();
        let app = f.0.join(".var/app/io.github.froggy744.PIC");
        let old_app = f.0.join(".var/app/io.github.you.PicRs");
        let old_data = old_app.join("data/pic-rs");
        fs::create_dir_all(old_data.join("overlays")).unwrap();
        fs::create_dir_all(old_app.join("config/picasa-rs")).unwrap();
        fs::write(old_data.join("library.db"), b"database").unwrap();
        fs::write(old_data.join("overlays/hash.png"), b"image").unwrap();
        fs::write(old_app.join("config/picasa-rs/libraries.json"), b"registry").unwrap();
        migrate_roots(&app.join("data"), &app.join("config"), &app.join("cache")).unwrap();
        assert!(app.join("data/pic-rs/library.db").is_file());
        assert!(app.join("cache/pic-rs/thumbs/overlay/hash.png").is_file());
        assert!(app.join("config/pic-rs/libraries.json").is_file());
        assert_eq!(
            storage_path(
                &old_data.join("library.db"),
                &app.join("data"),
                &app.join("config"),
                "data"
            ),
            app.join("data/pic-rs/library.db")
        );
    }

    #[test]
    fn overlay_conflict_stops_before_any_storage_moves() {
        let f = Fixture::new();
        let data = f.0.join("data");
        let config = f.0.join("config");
        let cache = f.0.join("cache");
        fs::create_dir_all(data.join("picasa-rs/overlays")).unwrap();
        fs::create_dir_all(cache.join("pic-rs/thumbs/overlay")).unwrap();
        fs::write(data.join("picasa-rs/overlays/hash.png"), b"old").unwrap();
        fs::write(cache.join("pic-rs/thumbs/overlay/hash.png"), b"new").unwrap();
        assert!(migrate_roots(&data, &config, &cache).is_err());
        assert!(data.join("picasa-rs/overlays/hash.png").is_file());
        assert!(!data.join("pic-rs").exists());
    }

    #[test]
    fn overlays_move_under_thumbnails_and_migration_is_repeatable() {
        let f = Fixture::new();
        let data = f.0.join("data");
        let config = f.0.join("config");
        let cache = f.0.join("cache");
        fs::create_dir_all(data.join("picasa-rs/overlays")).unwrap();
        fs::create_dir_all(config.join("picasa-rs")).unwrap();
        fs::write(data.join("picasa-rs/overlays/hash.jpg"), b"overlay").unwrap();
        fs::write(config.join("picasa-rs/libraries.json"), b"registry").unwrap();
        migrate_roots(&data, &config, &cache).unwrap();
        assert_eq!(
            fs::read(cache.join("pic-rs/thumbs/overlay/hash.jpg")).unwrap(),
            b"overlay"
        );
        assert!(config.join("pic-rs/libraries.json").is_file());
        assert!(!data.join("pic-rs/overlays").exists());
        migrate_roots(&data, &config, &cache).unwrap();
    }

    #[test]
    fn migration_preserves_database_wal_and_nested_assets() {
        let f = Fixture::new();
        let old = f.0.join("picasa-rs");
        let new = f.0.join("pic-rs");
        fs::create_dir_all(old.join("overlays")).unwrap();
        fs::write(old.join("library.db"), b"database").unwrap();
        fs::write(old.join("library.db-wal"), b"wal").unwrap();
        fs::write(old.join("overlays/hash.png"), b"image").unwrap();
        move_directory(&old, &new).unwrap();
        assert_eq!(fs::read(new.join("library.db-wal")).unwrap(), b"wal");
        assert_eq!(fs::read(new.join("overlays/hash.png")).unwrap(), b"image");
        assert!(!old.exists());
        move_directory(&old, &new).unwrap();
    }

    #[test]
    fn database_wal_pair_survives_when_database_moves_before_wal() {
        let f = Fixture::new();
        let old = f.0.join("old");
        let new = f.0.join("new");
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("library.db"), b"database").unwrap();
        fs::write(old.join("library.db-wal"), b"important-wal").unwrap();

        // Force the exact mutation that exposed the bug: snapshot the source,
        // move the database first, then process its WAL.
        let discardable = snapshot_discardable_sqlite_sidecars(&old).unwrap();
        assert!(!discardable.contains(&old.join("library.db-wal")));
        fs::create_dir_all(&new).unwrap();
        move_directory_with_snapshot(
            &old.join("library.db"),
            &new.join("library.db"),
            &discardable,
        )
        .unwrap();
        assert!(!old.join("library.db").exists());
        assert!(is_orphan_sqlite_wal(&old.join("library.db-wal")));

        move_directory_with_snapshot(
            &old.join("library.db-wal"),
            &new.join("library.db-wal"),
            &discardable,
        )
        .unwrap();

        assert_eq!(fs::read(new.join("library.db-wal")).unwrap(), b"important-wal");
        assert!(!old.join("library.db-wal").exists());
    }

    #[test]
    fn sqlite_shm_conflicts_do_not_block_migration() {
        let f = Fixture::new();
        let old = f.0.join("old");
        let new = f.0.join("new");
        fs::create_dir_all(&old).unwrap();
        fs::create_dir_all(&new).unwrap();
        fs::write(old.join("library.db-shm"), b"stale-old-shm").unwrap();
        fs::write(new.join("library.db-shm"), b"stale-new-shm").unwrap();

        move_directory(&old, &new).unwrap();

        assert!(!old.exists());
        assert_eq!(
            fs::read(new.join("library.db-shm")).unwrap(),
            b"stale-new-shm"
        );
    }

    #[test]
    fn sqlite_shm_is_discarded_when_no_destination_exists() {
        let f = Fixture::new();
        let old = f.0.join("old");
        let new = f.0.join("new");
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("library.db-shm"), b"stale-shm").unwrap();

        move_directory(&old, &new).unwrap();

        assert!(!old.exists());
        assert!(!new.join("library.db-shm").exists());
    }

    #[test]
    fn orphan_sqlite_wal_conflict_does_not_block_migration() {
        let f = Fixture::new();
        let old = f.0.join("old");
        let new = f.0.join("new");
        fs::create_dir_all(&old).unwrap();
        fs::create_dir_all(&new).unwrap();
        fs::write(old.join("library.db-wal"), b"orphan-old-wal").unwrap();
        fs::write(new.join("library.db-wal"), b"current-wal").unwrap();

        move_directory(&old, &new).unwrap();

        assert!(!old.exists());
        assert_eq!(
            fs::read(new.join("library.db-wal")).unwrap(),
            b"current-wal"
        );
    }

    #[test]
    fn sqlite_wal_is_preserved_when_its_database_still_exists() {
        let f = Fixture::new();
        let old = f.0.join("old");
        let new = f.0.join("new");
        fs::create_dir_all(&old).unwrap();
        fs::create_dir_all(&new).unwrap();
        fs::write(old.join("library.db"), b"old-database").unwrap();
        fs::write(old.join("library.db-wal"), b"old-wal").unwrap();
        fs::write(new.join("library.db-wal"), b"current-wal").unwrap();

        assert!(move_directory(&old, &new).is_err());
        assert_eq!(fs::read(old.join("library.db-wal")).unwrap(), b"old-wal");
        assert_eq!(fs::read(new.join("library.db-wal")).unwrap(), b"current-wal");
    }

    #[test]
    fn migration_refuses_to_overwrite_existing_data() {
        let f = Fixture::new();
        let old = f.0.join("old");
        let new = f.0.join("new");
        fs::create_dir_all(&old).unwrap();
        fs::create_dir_all(&new).unwrap();
        fs::write(old.join("library.db"), b"old").unwrap();
        fs::write(new.join("library.db"), b"new").unwrap();
        assert!(move_directory(&old, &new).is_err());
        assert_eq!(fs::read(old.join("library.db")).unwrap(), b"old");
        assert_eq!(fs::read(new.join("library.db")).unwrap(), b"new");
    }

    #[test]
    fn migration_merges_disjoint_directories() {
        let f = Fixture::new();
        let old = f.0.join("old");
        let new = f.0.join("new");
        fs::create_dir_all(old.join("libraries")).unwrap();
        fs::create_dir_all(new.join("libraries")).unwrap();
        fs::write(old.join("libraries/one.db"), b"one").unwrap();
        fs::write(new.join("libraries/two.db"), b"two").unwrap();
        move_directory(&old, &new).unwrap();
        assert_eq!(fs::read(new.join("libraries/one.db")).unwrap(), b"one");
        assert_eq!(fs::read(new.join("libraries/two.db")).unwrap(), b"two");
    }

    #[test]
    fn only_paths_inside_moved_directory_are_rewritten() {
        let old = Path::new("/home/example/.local/share/picasa-rs");
        let new = Path::new("/home/example/.local/share/pic-rs");
        assert_eq!(
            relocated_path(&old.join("libraries/main.db"), old, new),
            new.join("libraries/main.db")
        );
        for path in [
            "/photos/picasa/image.jpg",
            "/home/example/.local/share/picasa-rs-other/image.jpg",
        ] {
            assert_eq!(
                relocated_path(Path::new(path), old, new),
                PathBuf::from(path)
            );
        }
    }
}
