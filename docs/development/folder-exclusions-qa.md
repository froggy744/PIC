# Folder exclusion / removal QA

Feature branch: `feature/compact-folder-context-menu`

## Contract
- **Remove only** deletes catalogue membership without deleting originals; subsequent refresh may rediscover it.
- **Remove & Exclude** saves a persistent exclusion in the current PIC library database and removes indexed photos and folders. Descendants are excluded, not similarly prefixed sibling paths.
- Normal parent refresh, direct refresh, automatic watch scans, and progressive NFS/SMB discovery must not import excluded paths.
- Explicit re-import of an excluded local/network folder asks whether to restore it.
- Settings → Folders → Manage Excluded Folders… lists exclusions and allows restoring them individually.
- Exclusions are per library/database, survive restart and library switching, and never delete originals.

## Automated regression (must run on Linux developer workstation)
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo check`
- [ ] `cargo test exclusion`
- [ ] `cargo test excluded_subfolder_does_not_reappear_on_parent_refresh`
- [ ] Existing scanner/progressive network tests and DB tests remain green.

## Manual QA
- [ ] Remove only, refresh containing parent: folder may be rediscovered.
- [ ] Remove & Exclude, refresh containing parent: folder and descendants stay absent.
- [ ] Test /Pictures/Private excluded while /Pictures/Private2 remains importable.
- [ ] Test an imported root and a discovered nested folder.
- [ ] Test restart and switch libraries: exclusions persist only in chosen library.
- [ ] Test local folder picker selecting excluded folder → confirmation; Cancel keeps exclusion; Restore & Import removes exclusion and reindexes.
- [ ] Test NFS/SMB picker recovery similarly and a resumed progressive import.
- [ ] Test Settings → Folders → Manage Excluded Folders; restore a path, then refresh parent to reimport.
- [ ] Check no original photos are deleted and no folder returns from an in-flight scan.
- [ ] Check local paths with spaces, trailing slashes, and paths containing Unicode.
- [ ] Check dark/light themes and disabled Refresh menu item for non-import roots.

**Release gate:** Keep the PR in draft until compilation, scanner tests and manual refresh/recovery checks pass.
