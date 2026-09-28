const LIBRARY_REGISTRY_VERSION: u32 = 1;
const AUTO_BACKUP_DAYS: i64 = 5;
const BACKUP_HISTORY: usize = 5;

#[derive(Clone, Copy)]
enum BackupKind {
    Automatic,
    Manual,
}

impl BackupKind {
    fn prefix(self) -> &'static str {
        match self {
            Self::Automatic => "auto",
            Self::Manual => "manual",
        }
    }
}

static ACTIVE_DATABASE: std::sync::OnceLock<std::sync::RwLock<PathBuf>> =
    std::sync::OnceLock::new();
static REGISTRY_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
static BACKUP_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct LibraryEntry {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub path: PathBuf,
    #[serde(default)]
    pub last_automatic_backup: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    pub last_backup_signature: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct LibraryRegistry {
    version: u32,
    current: String,
    libraries: Vec<LibraryEntry>,
}

/// Resolve the last-used catalog, preserving the historical `library.db` as
/// the first catalog for existing installations.
pub fn initialize_library_manager() -> Result<LibraryEntry> {
    let _guard = registry_lock();
    let registry_already_existed = registry_path()?.is_file();
    let mut registry = load_or_create_registry()?;
    let selected = registry
        .libraries
        .iter()
        .find(|library| library.id == registry.current)
        .cloned()
        .context("the last-used photo library is no longer registered")?;
    if registry_already_existed && !selected.path.is_file() {
        anyhow::bail!(
            "the last-used library is unavailable: {}",
            selected.path.display()
        );
    }
    registry.current = selected.id.clone();
    save_registry(&registry)?;
    set_active_database_path(selected.path.clone());
    Ok(selected)
}

pub fn active_database_path() -> Result<PathBuf> {
    if let Some(path) = ACTIVE_DATABASE.get() {
        return path
            .read()
            .map(|path| path.clone())
            .map_err(|_| anyhow::anyhow!("active database lock poisoned"));
    }
    Ok(database_path()?)
}

pub fn active_library() -> Result<LibraryEntry> {
    let path = active_database_path()?;
    known_libraries()?
        .into_iter()
        .find(|library| library.path == path)
        .with_context(|| format!("active library {} is not registered", path.display()))
}

pub fn known_libraries() -> Result<Vec<LibraryEntry>> {
    let _guard = registry_lock();
    Ok(load_or_create_registry()?.libraries)
}

pub fn create_library(path: &Path, name: &str, description: &str) -> Result<LibraryEntry> {
    let path = absolute_library_path(path)?;
    if path.exists() {
        anyhow::bail!("a file already exists at {}", path.display());
    }
    let name = validate_library_name(name)?;
    // Opening initializes and validates the complete PIC schema before the
    // catalog is advertised in the registry.
    drop(open(&path)?);
    register_library(path, name, description.trim().to_owned())
}

pub fn add_existing_library(path: &Path, name: &str, description: &str) -> Result<LibraryEntry> {
    let path = absolute_library_path(path)?;
    if !path.is_file() {
        anyhow::bail!("library does not exist: {}", path.display());
    }
    drop(open(&path)?);
    register_library(
        path,
        validate_library_name(name)?,
        description.trim().to_owned(),
    )
}

fn register_library(path: PathBuf, name: String, description: String) -> Result<LibraryEntry> {
    let _guard = registry_lock();
    let mut registry = load_or_create_registry()?;
    if registry
        .libraries
        .iter()
        .any(|library| library.path == path)
    {
        anyhow::bail!("library is already registered: {}", path.display());
    }
    let entry = LibraryEntry {
        id: library_id(&path),
        name,
        description,
        path,
        last_automatic_backup: None,
        last_backup_signature: None,
    };
    registry.libraries.push(entry.clone());
    save_registry(&registry)?;
    Ok(entry)
}

pub fn update_library_details(id: &str, name: &str, description: &str) -> Result<LibraryEntry> {
    let name = validate_library_name(name)?;
    let _guard = registry_lock();
    let mut registry = load_or_create_registry()?;
    let library = registry
        .libraries
        .iter_mut()
        .find(|library| library.id == id)
        .context("library is no longer registered")?;
    library.name = name;
    library.description = description.trim().to_owned();
    let updated = library.clone();
    save_registry(&registry)?;
    Ok(updated)
}

/// Validate a library before publishing it as the process-wide active path.
/// Workers launched after this call capture the new path; existing workers
/// retain their explicit old path and cannot cross catalog boundaries.
pub fn select_library(id: &str) -> Result<(LibraryEntry, Connection)> {
    let _guard = registry_lock();
    let mut registry = load_or_create_registry()?;
    let selected = registry
        .libraries
        .iter()
        .find(|library| library.id == id)
        .cloned()
        .context("library is no longer registered")?;
    if !selected.path.is_file() {
        anyhow::bail!("library is unavailable: {}", selected.path.display());
    }
    let connection = open(&selected.path)?;
    registry.current = selected.id.clone();
    save_registry(&registry)?;
    set_active_database_path(selected.path.clone());
    Ok((selected, connection))
}

pub fn backup_directory() -> Result<PathBuf> {
    Ok(dirs::data_dir()
        .context("could not determine the user's data directory")?
        .join("picasa-rs")
        .join("backups"))
}

pub fn library_directory() -> Result<PathBuf> {
    Ok(dirs::data_dir()
        .context("could not determine the user's data directory")?
        .join("picasa-rs")
        .join("libraries"))
}

pub fn suggested_library_path(name: &str) -> Result<PathBuf> {
    let directory = library_directory()?;
    std::fs::create_dir_all(&directory)?;
    let stem = safe_file_stem(validate_library_name(name)?.as_str());
    let mut candidate = directory.join(format!("{stem}.db"));
    let mut suffix = 2;
    while candidate.exists() {
        candidate = directory.join(format!("{stem}-{suffix}.db"));
        suffix += 1;
    }
    Ok(candidate)
}

/// Create a consistent SQLite snapshot. This deliberately uses SQLite's
/// online backup API rather than copying a WAL-backed database file.
pub fn backup_library(library: &LibraryEntry) -> Result<PathBuf> {
    backup_library_with_kind(library, BackupKind::Manual)
}

fn backup_library_with_kind(library: &LibraryEntry, kind: BackupKind) -> Result<PathBuf> {
    let _guard = BACKUP_LOCK
        .get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .expect("backup lock poisoned");
    let directory = backup_directory()?.join(&library.id);
    backup_library_to(library, &directory, kind)
}

fn backup_library_to(
    library: &LibraryEntry,
    directory: &Path,
    kind: BackupKind,
) -> Result<PathBuf> {
    std::fs::create_dir_all(&directory)?;
    let timestamp = chrono::Local::now().format("%Y-%m-%d-%H%M%S");
    let prefix = kind.prefix();
    let mut destination = directory.join(format!("{prefix}-{timestamp}.db"));
    let mut suffix = 2;
    while destination.exists() {
        destination = directory.join(format!("{prefix}-{timestamp}-{suffix}.db"));
        suffix += 1;
    }
    let temporary = destination.with_extension("db.partial");
    let source = open_existing(&library.path)?;
    let mut target = Connection::open(&temporary)?;
    target.busy_timeout(std::time::Duration::from_secs(2))?;
    {
        let backup = rusqlite::backup::Backup::new(&source, &mut target)?;
        backup.run_to_completion(128, std::time::Duration::from_millis(10), None)?;
    }
    let integrity: String = target.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    if integrity != "ok" {
        let _ = std::fs::remove_file(&temporary);
        anyhow::bail!("backup integrity check failed: {integrity}");
    }
    drop(target);
    std::fs::rename(&temporary, &destination)?;
    if matches!(kind, BackupKind::Automatic) {
        prune_automatic_backups(&directory)?;
    }
    Ok(destination)
}

/// Run the five-day policy on a worker thread. A source signature avoids a
/// duplicate snapshot when the catalog and its WAL have not changed.
pub fn automatic_backup_if_due(library: &LibraryEntry) -> Result<Option<PathBuf>> {
    let now = chrono::Utc::now();
    if library
        .last_automatic_backup
        .is_some_and(|last| now.signed_duration_since(last).num_days() < AUTO_BACKUP_DAYS)
    {
        return Ok(None);
    }
    let signature = database_signature(&library.path)?;
    if library.last_backup_signature.as_deref() == Some(signature.as_str()) {
        record_automatic_backup(&library.id, now, Some(signature))?;
        return Ok(None);
    }
    let destination = backup_library_with_kind(library, BackupKind::Automatic)?;
    record_automatic_backup(&library.id, now, Some(signature))?;
    Ok(Some(destination))
}

/// Restore a backup into a new PIC library. Restoring to a new file avoids
/// replacing an open WAL database; the caller can safely switch to it after
/// validation.
pub fn restore_as_library(
    backup_path: &Path,
    destination: &Path,
    name: &str,
    description: &str,
) -> Result<LibraryEntry> {
    let destination = absolute_library_path(destination)?;
    restore_snapshot(backup_path, &destination)?;
    add_existing_library(&destination, name, description)
}

fn restore_snapshot(backup_path: &Path, destination: &Path) -> Result<()> {
    if destination.exists() {
        anyhow::bail!("a file already exists at {}", destination.display());
    }
    let source =
        Connection::open_with_flags(backup_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let integrity: String = source.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    if integrity != "ok" {
        anyhow::bail!("backup integrity check failed: {integrity}");
    }
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = destination.with_extension("db.restore-partial");
    let mut target = Connection::open(&temporary)?;
    {
        let backup = rusqlite::backup::Backup::new(&source, &mut target)?;
        backup.run_to_completion(128, std::time::Duration::from_millis(5), None)?;
    }
    drop(target);
    std::fs::rename(&temporary, &destination)?;
    Ok(())
}

fn record_automatic_backup(
    id: &str,
    timestamp: chrono::DateTime<chrono::Utc>,
    signature: Option<String>,
) -> Result<()> {
    let _guard = registry_lock();
    let mut registry = load_or_create_registry()?;
    let library = registry
        .libraries
        .iter_mut()
        .find(|library| library.id == id)
        .context("library is no longer registered")?;
    library.last_automatic_backup = Some(timestamp);
    library.last_backup_signature = signature;
    save_registry(&registry)
}

fn database_signature(path: &Path) -> Result<String> {
    let mut hasher = blake3::Hasher::new();
    for candidate in [
        path.to_path_buf(),
        PathBuf::from(format!("{}-wal", path.display())),
    ] {
        if let Ok(metadata) = std::fs::metadata(&candidate) {
            hasher.update(candidate.as_os_str().as_encoded_bytes());
            hasher.update(&metadata.len().to_le_bytes());
            if let Ok(modified) = metadata.modified() {
                if let Ok(duration) = modified.duration_since(std::time::UNIX_EPOCH) {
                    hasher.update(&duration.as_nanos().to_le_bytes());
                }
            }
        }
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn prune_automatic_backups(directory: &Path) -> Result<()> {
    let mut backups = std::fs::read_dir(directory)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().is_some_and(|extension| extension == "db")
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("auto-"))
        })
        .collect::<Vec<_>>();
    backups.sort();
    let remove_count = backups.len().saturating_sub(BACKUP_HISTORY);
    for path in backups.into_iter().take(remove_count) {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

fn load_or_create_registry() -> Result<LibraryRegistry> {
    let path = registry_path()?;
    if path.is_file() {
        let bytes = std::fs::read(&path)?;
        let registry: LibraryRegistry = serde_json::from_slice(&bytes)
            .with_context(|| format!("could not read library registry {}", path.display()))?;
        if registry.version != LIBRARY_REGISTRY_VERSION {
            anyhow::bail!("unsupported library registry version {}", registry.version);
        }
        if !registry.libraries.is_empty() {
            return Ok(registry);
        }
    }
    let legacy = database_path()?;
    let entry = LibraryEntry {
        id: library_id(&legacy),
        name: "Default Library".to_owned(),
        description: "Original PIC photo library".to_owned(),
        path: legacy,
        last_automatic_backup: None,
        last_backup_signature: None,
    };
    Ok(LibraryRegistry {
        version: LIBRARY_REGISTRY_VERSION,
        current: entry.id.clone(),
        libraries: vec![entry],
    })
}

fn save_registry(registry: &LibraryRegistry) -> Result<()> {
    let path = registry_path()?;
    let parent = path
        .parent()
        .context("library registry has no parent directory")?;
    std::fs::create_dir_all(parent)?;
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, serde_json::to_vec_pretty(registry)?)?;
    std::fs::rename(&temporary, &path)?;
    Ok(())
}

fn registry_path() -> Result<PathBuf> {
    Ok(dirs::config_dir()
        .context("could not determine the user's configuration directory")?
        .join("picasa-rs")
        .join("libraries.json"))
}

fn registry_lock() -> std::sync::MutexGuard<'static, ()> {
    REGISTRY_LOCK
        .get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .expect("library registry lock poisoned")
}

fn set_active_database_path(path: PathBuf) {
    if let Some(active) = ACTIVE_DATABASE.get() {
        *active.write().expect("active database lock poisoned") = path;
    } else {
        let _ = ACTIVE_DATABASE.set(std::sync::RwLock::new(path));
    }
}

fn absolute_library_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    Ok(std::env::current_dir()?.join(path))
}

fn validate_library_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        anyhow::bail!("library name cannot be empty");
    }
    Ok(name.to_owned())
}

fn library_id(path: &Path) -> String {
    blake3::hash(path.as_os_str().as_encoded_bytes()).to_hex()[..16].to_owned()
}

fn safe_file_stem(name: &str) -> String {
    let stem = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    let trimmed = stem.trim_matches('-');
    if trimmed.is_empty() {
        "PIC-Library".to_owned()
    } else {
        trimmed.to_owned()
    }
}

#[cfg(test)]
mod library_tests {
    use super::*;

    fn temporary_directory(label: &str) -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("pic-{label}-{nonce}"));
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    #[test]
    fn sqlite_backup_includes_wal_rows_and_is_integrity_checked() {
        let directory = temporary_directory("online-backup");
        let source_path = directory.join("source.db");
        let source = open(&source_path).unwrap();
        source
            .execute(
                "INSERT INTO settings(key, value) VALUES ('backup-test', 'preserved')",
                [],
            )
            .unwrap();
        let library = LibraryEntry {
            id: "test".to_owned(),
            name: "Work Photos".to_owned(),
            description: String::new(),
            path: source_path,
            last_automatic_backup: None,
            last_backup_signature: None,
        };

        let backup = backup_library_to(
            &library,
            &directory.join("backups"),
            BackupKind::Manual,
        )
        .unwrap();
        assert!(backup
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("manual-")));
        let snapshot = Connection::open(&backup).unwrap();
        let value: String = snapshot
            .query_row(
                "SELECT value FROM settings WHERE key = 'backup-test'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(value, "preserved");
        assert_eq!(
            snapshot
                .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
                .unwrap(),
            "ok"
        );
        let restored_path = directory.join("restored.db");
        restore_snapshot(&backup, &restored_path).unwrap();
        let restored = open_existing(&restored_path).unwrap();
        assert_eq!(
            setting(&restored, "backup-test").unwrap().as_deref(),
            Some("preserved")
        );
        drop(restored);
        drop(snapshot);
        drop(source);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn backup_names_are_readable_and_catalog_names_are_sanitized() {
        assert_eq!(safe_file_stem("Work / 2026"), "Work---2026");
        assert_eq!(safe_file_stem("***"), "PIC-Library");
    }

    #[test]
    fn automatic_backup_retention_keeps_only_configured_history() {
        let directory = temporary_directory("automatic-backup-retention");
        for day in 1..=BACKUP_HISTORY + 2 {
            std::fs::write(
                directory.join(format!("auto-2026-09-{day:02}-120000.db")),
                [],
            )
            .unwrap();
        }

        prune_automatic_backups(&directory).unwrap();

        let mut automatic = std::fs::read_dir(&directory)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with("auto-"))
            .collect::<Vec<_>>();
        automatic.sort();
        assert_eq!(automatic.len(), BACKUP_HISTORY);
        assert_eq!(automatic.first().unwrap(), "auto-2026-09-03-120000.db");
        assert_eq!(automatic.last().unwrap(), "auto-2026-09-07-120000.db");
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn automatic_backup_retention_never_removes_manual_backups() {
        let directory = temporary_directory("manual-backup-retention");
        for day in 1..=BACKUP_HISTORY + 2 {
            std::fs::write(
                directory.join(format!("auto-2026-09-{day:02}-120000.db")),
                [],
            )
            .unwrap();
        }
        let manual = [
            "manual-2026-08-01-090000.db",
            "manual-2026-08-02-090000.db",
            "Work-legacy-manual-backup.db",
        ];
        for name in manual {
            std::fs::write(directory.join(name), []).unwrap();
        }

        prune_automatic_backups(&directory).unwrap();

        for name in manual {
            assert!(directory.join(name).is_file(), "manual backup {name} was removed");
        }
        let automatic_count = std::fs::read_dir(&directory)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with("auto-"))
            .count();
        assert_eq!(automatic_count, BACKUP_HISTORY);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn application_connections_wait_for_a_short_writer_instead_of_failing_busy() {
        let directory = temporary_directory("busy-timeout");
        let path = directory.join("library.db");
        let writer = open(&path).unwrap();
        let interactive = open_existing(&path).unwrap();
        let transaction = writer.unchecked_transaction().unwrap();
        transaction
            .execute(
                "INSERT INTO settings(key, value) VALUES ('scanner', 'writing')",
                [],
            )
            .unwrap();
        let handle = std::thread::spawn(move || set_setting(&interactive, "interactive", "saved"));
        std::thread::sleep(std::time::Duration::from_millis(50));
        transaction.commit().unwrap();
        handle.join().unwrap().unwrap();
        assert_eq!(
            setting(&writer, "interactive").unwrap().as_deref(),
            Some("saved")
        );
        drop(writer);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
