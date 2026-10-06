use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS photos (
  id INTEGER PRIMARY KEY,
  path TEXT UNIQUE NOT NULL,
  folder_id INTEGER,
  taken_at TEXT,
  camera TEXT,
  aperture REAL,
  lens TEXT,
  shutter_speed REAL,
  iso INTEGER,
  focal_length REAL,
  exposure_bias REAL,
  width INTEGER,
  height INTEGER,
  size_bytes INTEGER,
  mtime INTEGER,
  added_at INTEGER NOT NULL DEFAULT 0,
  rotation INTEGER DEFAULT 0,
  edit_recipe TEXT NOT NULL DEFAULT '',
  favorite BOOLEAN DEFAULT 0,
  rating INTEGER NOT NULL DEFAULT 0,
  trashed BOOLEAN DEFAULT 0
);
CREATE TABLE IF NOT EXISTS folders (
  id INTEGER PRIMARY KEY,
  path TEXT UNIQUE NOT NULL,
  name TEXT,
  parent_id INTEGER REFERENCES folders(id),
  imported_root BOOLEAN NOT NULL DEFAULT 0,
  watched BOOLEAN NOT NULL DEFAULT 0,
  raw_jpeg_pair_count INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS albums (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL COLLATE NOCASE UNIQUE,
  created_at INTEGER NOT NULL DEFAULT 0,
  cover_frame TEXT,
  cover_photo_id INTEGER REFERENCES photos(id) ON DELETE SET NULL
);
CREATE TABLE IF NOT EXISTS album_photos (
  album_id INTEGER NOT NULL REFERENCES albums(id) ON DELETE CASCADE,
  photo_id INTEGER NOT NULL REFERENCES photos(id) ON DELETE CASCADE,
  PRIMARY KEY(album_id, photo_id)
);
CREATE TABLE IF NOT EXISTS settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS overlay_assets (
  hash TEXT PRIMARY KEY,
  width INTEGER NOT NULL,
  height INTEGER NOT NULL,
  format TEXT NOT NULL,
  original_name TEXT NOT NULL,
  added_at INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS editing_events (
  id INTEGER PRIMARY KEY,
  action TEXT NOT NULL,
  edited_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS editing_event_items (
  event_id INTEGER NOT NULL REFERENCES editing_events(id) ON DELETE CASCADE,
  photo_id INTEGER NOT NULL REFERENCES photos(id) ON DELETE CASCADE,
  PRIMARY KEY(event_id, photo_id)
);
CREATE TABLE IF NOT EXISTS recently_edited (
  photo_id INTEGER PRIMARY KEY REFERENCES photos(id) ON DELETE CASCADE,
  event_id INTEGER NOT NULL REFERENCES editing_events(id),
  edited_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS collage_projects (
  photo_id INTEGER PRIMARY KEY REFERENCES photos(id) ON DELETE CASCADE,
  draft TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_recently_edited_time ON recently_edited(edited_at DESC, event_id DESC);
CREATE INDEX IF NOT EXISTS idx_editing_event_items_photo ON editing_event_items(photo_id);
CREATE INDEX IF NOT EXISTS idx_photos_taken_at ON photos(taken_at DESC);
CREATE INDEX IF NOT EXISTS idx_photos_folder ON photos(folder_id);
CREATE INDEX IF NOT EXISTS idx_album_photos_photo ON album_photos(photo_id);
CREATE TABLE IF NOT EXISTS network_import_jobs (
  root TEXT PRIMARY KEY REFERENCES folders(path) ON DELETE CASCADE,
  generation INTEGER NOT NULL DEFAULT 0,
  status TEXT NOT NULL DEFAULT 'queued',
  recovered INTEGER NOT NULL DEFAULT 0,
  added INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS network_import_dirs (
  root TEXT NOT NULL REFERENCES network_import_jobs(root) ON DELETE CASCADE,
  path TEXT NOT NULL,
  parent TEXT,
  state TEXT NOT NULL DEFAULT 'pending',
  PRIMARY KEY(root, path)
);
CREATE INDEX IF NOT EXISTS idx_network_dirs_pending ON network_import_dirs(root, state);
CREATE TABLE IF NOT EXISTS network_import_seen (
  root TEXT NOT NULL REFERENCES network_import_jobs(root) ON DELETE CASCADE,
  path TEXT NOT NULL,
  kind TEXT NOT NULL,
  PRIMARY KEY(root, path)
);
CREATE TABLE IF NOT EXISTS network_photo_work (
  path TEXT PRIMARY KEY REFERENCES photos(path) ON DELETE CASCADE ON UPDATE CASCADE,
  root TEXT NOT NULL REFERENCES network_import_jobs(root) ON DELETE CASCADE,
  generation INTEGER NOT NULL,
  mtime INTEGER,
  size INTEGER,
  metadata_state TEXT NOT NULL DEFAULT 'pending',
  thumbnail_state TEXT NOT NULL DEFAULT 'pending'
);
CREATE INDEX IF NOT EXISTS idx_network_work_pending ON network_photo_work(root, generation, metadata_state, thumbnail_state);
CREATE INDEX IF NOT EXISTS idx_network_work_queue ON network_photo_work(root, generation)
  WHERE metadata_state='pending' OR thumbnail_state='pending';
CREATE TABLE IF NOT EXISTS network_import_pairs (
  root TEXT NOT NULL REFERENCES network_import_jobs(root) ON DELETE CASCADE,
  folder TEXT NOT NULL,
  stem TEXT NOT NULL,
  raw INTEGER NOT NULL DEFAULT 0,
  jpeg INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY(root, folder, stem)
);
"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderSearchResult {
    pub id: i64,
    pub path: String,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct Folder {
    pub id: i64,
    pub path: String,
    pub name: String,
    pub parent_id: Option<i64>,
    pub imported_root: bool,
    pub watched: bool,
    pub photo_count: i64,
    pub subfolder_count: i64,
    pub available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Album {
    pub id: i64,
    pub name: String,
    pub created_at: i64,
    pub photo_count: i64,
    pub cover_frame: Option<String>,
    /// Photo the album shows on its card, chosen from the thumbnail menu.
    pub cover_photo_id: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct Photo {
    pub id: i64,
    pub path: String,
    pub folder_id: Option<i64>,
    pub folder_path: Option<String>,
    pub taken_at: Option<String>,
    pub camera: Option<String>,
    pub aperture: Option<f64>,
    pub lens: Option<String>,
    pub shutter_speed: Option<f64>,
    pub iso: Option<i64>,
    pub focal_length: Option<f64>,
    pub exposure_bias: Option<f64>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub size_bytes: Option<i64>,
    pub mtime: Option<i64>,
    pub added_at: i64,
    pub rotation: i32,
    pub edit_recipe: String,
    pub favorite: bool,
    pub rating: i32,
    pub trashed: bool,
    /// Presentation metadata populated only by the History query.
    pub history_caption: Option<String>,
    /// Last edit time (epoch millis) from `recently_edited`. 0 for ordinary
    /// library rows that have never been edited.
    pub edited_at: i64,
}

#[derive(Debug, Clone, Default)]
pub struct PhotoMetadata {
    pub taken_at: Option<String>,
    pub camera: Option<String>,
    pub aperture: Option<f64>,
    pub lens: Option<String>,
    pub shutter_speed: Option<f64>,
    pub iso: Option<i64>,
    pub focal_length: Option<f64>,
    pub exposure_bias: Option<f64>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub size_bytes: Option<i64>,
    pub mtime: Option<i64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LibraryCounts {
    pub photos: i64,
    pub albums: i64,
    pub folders: i64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SidebarCounts {
    pub photos: i64,
    pub favorites: i64,
    pub recently_added: i64,
}

include!("db/core.rs");
include!("db/libraries.rs");
include!("db/albums.rs");
include!("db/photos.rs");
include!("db/history.rs");
include!("db/library_home.rs");
include!("db/settings.rs");
include!("db/overlay_assets.rs");
include!("db/tests.rs");

include!("db/path_migration.rs");
include!("db/network_import.rs");
