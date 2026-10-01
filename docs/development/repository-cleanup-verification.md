# Repository cleanup verification — 2026-10-01

The cleanup is prepared on `cleanup/repository-2026-10-01` in an isolated
worktree. The user authorized committing, pushing this branch and opening a PR against
`main`. Merge remains pending review and is not authorized.

## Network gate

The user confirmed `10.0.0.1` for read-only checks. No credentials are
recorded here. Existing diagnostics compiled with:

```sh
gcc -O2 -Wall -Wextra -o /tmp/pic-smb-probe scripts/pic-smb-probe.c $(pkg-config --cflags --libs smbclient)
gcc -O2 -Wall -Wextra -o /tmp/pic-nfs-probe pic-nfs-probe.c $(pkg-config --cflags --libs libnfs)
```

SMB compilation reports the existing `smbc_init` deprecation warning.
The diagnostic entry point remains executable (100755); both C sources
remain 100644. The original source locations are retained.

Read-only commands that passed from the cleanup worktree:

```sh
bash scripts/test-shares.sh --exports 10.0.0.1
bash scripts/test-shares.sh --nfs 3 10.0.0.1 /mnt/4TBP /
bash scripts/test-shares.sh --nfs 4 10.0.0.1 /mnt/4TBP /
bash scripts/test-shares.sh --probe 3 10.0.0.1 /mnt/4TBP '/Other/Tat Sing/20190917_184453.jpg'
bash scripts/test-shares.sh --smb smb://10.0.0.1
bash scripts/test-shares.sh --smb smb://10.0.0.1/4TBP
```

Four NFS exports were returned, NFS v3/v4 each listed 11 entries, and the
known JPEG passed stat/open/read/signature checks. SMB share listing and
recursive subfolder enumeration completed successfully.

Opening a photo through PIC's application transports and discovery scans
are not verified. Consequently, neither diagnostic source is relocated.
The root has six tracked entry files rather than the proposed five: the
additional file is the protected `pic-nfs-probe.c`.

Mocked diagnostic checks cover unrelated working directories, checkout
paths containing spaces, dispatch and `PIC_DIAGNOSTICS_BIN_DIR`,
`PIC_SMB_PROBE_BIN` and `PIC_NFS_PROBE_BIN` overrides. They do not establish
live networking behavior. Native transport code, launcher and bundled
network-library configuration are unchanged. Source-copy checks preserve
both native transports and both diagnostic sources.

## Cleanup policy

Useful network reference code and development notes were retained under
`docs/development/`. No additional files were established as unnecessary;
there are no candidates awaiting deletion approval. The six previously
authorized deletions remain absent.

`.superpowers/` contains local execution state and is ignored and excluded
from package source copies. It remains on disk. Source copies retain user
ZIP archives and offline dependency inputs while excluding worktrees,
logs, caches and `to-be-deleted/`. No runtime assets were regenerated.

## Validation scope

Python script checks validate documentation links/screenshots, repository
and standalone packager paths, pinned downloader inputs, deb/rpm icon
staging, temporary icon-bundle regeneration, diagnostic paths/overrides,
and source-copy contents. Bash syntax and real native link-order checks
passed. Full package builds and Windows PowerShell execution are not
verified; `pwsh` is unavailable on this host.

Final local results: 30 Python checks pass; nine Bash scripts pass syntax
checks; `cargo check --locked` passes; `cargo test --locked` reports 478
passed and 52 ignored (display/optional-fixture requirements). The thumbnail
orientation fixture and embedded SVG resource tests pass. Native link-order
verification passes. Cargo emits existing application/native warnings.
The fresh Astra review found no Critical, Important or Minor issues.
