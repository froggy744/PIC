//! Cancellable metadata-only backfill. Reads and commits happen off the GTK thread.
use crate::{db, scanner};
use anyhow::Result;
use gio::prelude::*;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::Instant,
};

#[derive(Debug, Clone, Default)]
pub struct Progress {
    pub examined: usize,
    pub updated: usize,
    pub skipped: usize,
    pub total: usize,
    pub finished: bool,
    pub cancelled: bool,
    pub error: Option<String>,
}

pub fn start(path: PathBuf) -> (Arc<AtomicBool>, mpsc::Receiver<Progress>) {
    let cancel = Arc::new(AtomicBool::new(false));
    let stop = cancel.clone();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut progress = Progress::default();
        if let Err(error) = run(&path, &stop, &mut progress, |p| {
            let _ = sender.send(p.clone());
        }) {
            progress.error = Some(error.to_string());
        }
        progress.finished = true;
        progress.cancelled = stop.load(Ordering::Relaxed);
        let _ = sender.send(progress);
    });
    (cancel, receiver)
}

fn attributes(path: &str) -> Result<gio::FileInfo> {
    #[cfg(target_os = "linux")]
    if crate::network_shares::private(path) {
        let stat = crate::network_shares::stat(path)?;
        let info = gio::FileInfo::new();
        info.set_size(i64::try_from(stat.size)?);
        if let Some(mtime) = stat.mtime.filter(|m| *m >= 0) {
            info.set_attribute_uint64("time::modified", mtime as u64);
        }
        return Ok(info);
    }
    Ok(crate::source::file(path).query_info(
        "time::modified,standard::size",
        gio::FileQueryInfoFlags::NONE,
        gio::Cancellable::NONE,
    )?)
}

fn run(
    path: &std::path::Path,
    stop: &AtomicBool,
    progress: &mut Progress,
    mut notify: impl FnMut(&Progress),
) -> Result<()> {
    let c = db::open_existing(path)?;
    let upper: i64 = c.query_row("SELECT COALESCE(MAX(id),0) FROM photos", [], |r| r.get(0))?;
    progress.total=c.query_row("SELECT COUNT(*) FROM photos p LEFT JOIN photo_metadata m ON m.photo_id=p.id WHERE p.trashed=0 AND COALESCE(m.version,0)<1 AND p.id<=?1",[upper],|r|r.get(0))?;
    notify(progress);
    let mut cursor = 0;
    let mut last = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        let batch=c.prepare("SELECT p.id,p.path,p.mtime,p.size_bytes FROM photos p LEFT JOIN photo_metadata m ON m.photo_id=p.id
            WHERE p.trashed=0 AND COALESCE(m.version,0)<1 AND p.id>?1 AND p.id<=?2 ORDER BY p.id LIMIT 32")?
            .query_map(rusqlite::params![cursor,upper],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<i64>>(2)?,r.get::<_,Option<i64>>(3)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if batch.is_empty() {
            break;
        }
        for (id, source, mtime, size) in batch {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            cursor = id;
            let result = (|| -> Result<bool> {
                let info = attributes(&source)?;
                let current_mtime = info.modification_date_time().map(|v| v.to_unix());
                anyhow::ensure!(
                    mtime.is_none_or(|v| Some(v) == current_mtime)
                        && size.is_none_or(|v| v == info.size()),
                    "Source changed; refresh the folder first"
                );
                let metadata = scanner::read_metadata(&source, &info)?;
                if stop.load(Ordering::Relaxed) {
                    return Ok(false);
                }
                let after = attributes(&source)?;
                anyhow::ensure!(
                    after.size() == info.size()
                        && after.modification_date_time() == info.modification_date_time(),
                    "Source changed while indexing"
                );
                db::metadata::apply_backfill(&c, id, &source, mtime, size, &metadata)
            })();
            match result {
                Ok(true) => progress.updated += 1,
                _ => progress.skipped += 1,
            }
            progress.examined += 1;
            if last.elapsed().as_millis() >= 150 {
                notify(progress);
                last = Instant::now();
            }
        }
    }
    if std::env::var_os("PIC_METADATA_TRACE").is_some() {
        eprintln!(
            "PIC_METADATA stage=index_complete examined={} updated={} skipped={} cancelled={}",
            progress.examined,
            progress.updated,
            progress.skipped,
            stop.load(Ordering::Relaxed)
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_index_existing_png_preserves_edits_and_fingerprint() {
        let root =
            std::env::temp_dir().join(format!("pic-metadata-index-png-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("existing.png");
        image::RgbImage::new(8, 6).save(&source).unwrap();
        let info = attributes(source.to_str().unwrap()).unwrap();
        let path = root.join("library.db");
        let c = db::open(&path).unwrap();
        let id = db::upsert_photo(
            &c,
            &source,
            None,
            &db::PhotoMetadata {
                mtime: info.modification_date_time().map(|v| v.to_unix()),
                size_bytes: Some(info.size()),
                ..Default::default()
            },
        )
        .unwrap();
        c.execute(
            "UPDATE photos SET edit_recipe='preserve',rotation=90,rating=4,favorite=1 WHERE id=?1",
            [id],
        )
        .unwrap();
        let before = db::photos(&c, None, false, None).unwrap().remove(0);
        let mut progress = Progress::default();
        run(&path, &AtomicBool::new(false), &mut progress, |_| {}).unwrap();
        assert_eq!((progress.updated, progress.skipped), (1, 0));
        let after = db::photos(&c, None, false, None).unwrap().remove(0);
        assert_eq!(
            (
                after.edit_recipe,
                after.rotation,
                after.rating,
                after.favorite,
                after.mtime,
                after.size_bytes
            ),
            (
                before.edit_recipe,
                before.rotation,
                before.rating,
                before.favorite,
                before.mtime,
                before.size_bytes
            )
        );
        let indexed: (i64, i64, String) = c
            .query_row(
                "SELECT version,has_exif,file_type FROM photo_metadata WHERE photo_id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(indexed, (1, 0, "PNG".into()));
        drop(c);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn metadata_index_cancellation_and_missing_sources_preserve_catalog() {
        let root = std::env::temp_dir().join(format!("pic-metadata-index-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("library.db");
        let c = db::open(&path).unwrap();
        c.execute("INSERT OR IGNORE INTO photos(path,edit_recipe) VALUES ('/missing/pic-filter.jpg','original-edit')",[]).unwrap();
        let stop = AtomicBool::new(true);
        let mut p = Progress::default();
        run(&path, &stop, &mut p, |_| {}).unwrap();
        assert_eq!(p.examined, 0);
        stop.store(false, Ordering::Relaxed);
        run(&path, &stop, &mut p, |_| {}).unwrap();
        assert_eq!(p.skipped, 1);
        assert_eq!(
            db::photos(&c, None, false, None).unwrap()[0].edit_recipe,
            "original-edit"
        );
        drop(c);
        std::fs::remove_dir_all(root).unwrap();
    }
}
