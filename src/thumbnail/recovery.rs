type RecoveryItem = (String, Option<i64>, Option<i64>);

#[cfg(test)]
pub fn recovery_items(items: Vec<RecoveryItem>) -> (Vec<RecoveryItem>, usize) {
    recovery_items_cancellable(items, || false)
}

pub fn recovery_items_cancellable(
    items: Vec<RecoveryItem>, cancelled: impl FnMut() -> bool,
) -> (Vec<RecoveryItem>, usize) {
    recovery_items_with_probe(items, cancelled, crate::source::file_available)
}

fn recovery_items_with_probe(
    items: Vec<RecoveryItem>, mut cancelled: impl FnMut() -> bool,
    mut available: impl FnMut(&str) -> bool,
) -> (Vec<RecoveryItem>, usize) {
    let mut ready = Vec::new();
    let mut offline = 0;
    for item in items {
        if cancelled() { break; }
        #[cfg(target_os = "linux")]
        if crate::network_shares::private(&item.0) {
            // Visible preview requests and durable imports handle these files.
            // Startup must not stat the entire NAS before displaying photos.
            continue;
        }
        let Ok(destination) = cache_path(&item.0, item.1, item.2) else { continue; };
        if existing_cache_path(&item.0, item.1, item.2).ok().flatten().is_some()
            || known_decode_failure(&item.0, &destination) { continue; }
        if available(&item.0) { ready.push(item); } else { offline += 1; }
    }
    (ready, offline)
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
    use gio::prelude::*;

    struct Fixture {
        directory: PathBuf,
        item: RecoveryItem,
        destination: PathBuf,
    }

    impl Fixture {
        fn new(uri: bool) -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let directory = std::env::temp_dir().join(format!(
                "pic-offline-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&directory).unwrap();
            let path = directory.join("photo.png");
            let reference = if uri {
                gio::File::for_path(&path).uri().to_string()
            } else {
                path.to_string_lossy().into_owned()
            };
            let destination = cache_path(&reference, Some(123), Some(456)).unwrap();
            Self {
                directory,
                item: (reference, Some(123), Some(456)),
                destination,
            }
        }

        fn reconnect(&self) {
            image::RgbImage::from_pixel(16, 12, image::Rgb([20, 90, 160]))
                .save(self.directory.join("photo.png"))
                .unwrap();
        }

        fn create(&self) -> Result<PathBuf> {
            create(&self.item.0, self.item.1, self.item.2)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
            let _ = fs::remove_file(&self.destination);
            let _ = fs::remove_file(self.destination.with_extension("failed"));
        }
    }

    #[test]
    fn offline_startup_recovers_after_reconnect_even_with_stale_uri_availability() {
        for uri in [false, true] {
            let fixture = Fixture::new(uri);
            assert!(!crate::source::cached_file_available(&fixture.item.0));
            assert_eq!(recovery_items(vec![fixture.item.clone()]), (vec![], 1));
            assert!(!fixture.destination.with_extension("failed").exists());
            fixture.reconnect();
            let (ready, offline) = recovery_items(vec![fixture.item.clone()]);
            assert_eq!(offline, 0);
            assert_eq!(ready.len(), 1);
            let results = create_many_cancellable(&ready, || false, |_| {});
            assert!(results[0].as_ref().unwrap().is_ok());
            assert!(fixture.destination.is_file());
            assert_eq!(recovery_items(vec![fixture.item.clone()]), (vec![], 0));
        }
    }

    #[test]
    fn disconnect_during_work_does_not_poison_the_cache() {
        let fixture = Fixture::new(false);
        assert!(fixture.create().is_err());
        assert!(!fixture.destination.with_extension("failed").exists());
        fixture.reconnect();
        assert!(fixture.create().unwrap().is_file());
    }

    #[test]
    fn legacy_offline_failure_marker_is_retried() {
        let fixture = Fixture::new(false);
        fs::create_dir_all(fixture.destination.parent().unwrap()).unwrap();
        fs::write(
            fixture.destination.with_extension("failed"),
            b"thumbnail generation failed\n",
        )
        .unwrap();
        fixture.reconnect();
        assert_eq!(recovery_items(vec![fixture.item.clone()]).0.len(), 1);
        assert!(fixture.create().unwrap().is_file());
        assert!(!fixture.destination.with_extension("failed").exists());
    }

    #[test]
    fn corrupt_online_source_is_still_suppressed() {
        let fixture = Fixture::new(false);
        fs::write(fixture.directory.join("photo.png"), b"not an image").unwrap();
        assert!(fixture.create().is_err());
        assert!(known_decode_failure(&fixture.item.0, &fixture.destination));
        assert_eq!(recovery_items(vec![fixture.item.clone()]), (vec![], 0));
    }
}

#[cfg(test)]
mod startup_recovery_tests {
    use super::*;
    #[test]
    #[cfg(target_os = "linux")]
    fn automatic_recovery_does_not_probe_each_network_photo() {
        let mut probes=0;
        let items=vec![
            ("nfs://offline.invalid/photos/a.jpg".into(),Some(1),Some(2)),
            ("smb://offline.invalid/photos/b.jpg".into(),Some(1),Some(2)),
        ];
        let (ready,offline)=recovery_items_with_probe(items,||false,|_| { probes+=1; false });
        assert_eq!(probes,0,"network previews belong to visible requests and durable imports");
        assert!(ready.is_empty());
        assert_eq!(offline,0,"deferring a network preview must not report an offline file");
    }
    #[test]
    fn stopping_recovery_avoids_further_source_probes() {
        let item=("/missing-startup-cancel-test.jpg".into(),Some(1),Some(2));
        let mut probes=0;
        let (ready,offline)=recovery_items_with_probe(vec![item],||true,|_| {probes+=1;true});
        assert!(ready.is_empty());
        assert_eq!((probes,offline),(0,0));
    }
}
