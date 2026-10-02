# PIC on Linux

Flatpak is PIC's primary Linux package. It builds against the selected GNOME
runtime, so the resulting application does not depend on the host distribution's
GLIBC, GTK4 or libadwaita versions. The AppImage target remains available for
compatible systems.

Run the build commands below from the repository root.

## Build prerequisites

Use `scripts/PIC-build-linux-one-script.sh` for new builds. It checks the selected
targets' host tools and development libraries, Rust crate cache, AppImage tool,
Flatpak SDK compatibility, and source archive checksums before compiling.
Missing dependencies are listed with an **Install/download? [y/N]** prompt.
Only an explicit `y` or `yes` permits setup; declining or providing no input
stops before compilation. Fedora (`dnf`) and Debian/Ubuntu (`apt-get`) host
packages can be installed through `sudo`; other distributions receive a list
to install manually. An older distribution may not provide the native GTK
versions required by AppImage; use the Flatpak target in that case.

Check and optionally set up dependencies without building:

```sh
./scripts/PIC-build-linux-one-script.sh local --check-dependencies
```

Build both packages from the current checkout:

```sh
./scripts/PIC-build-linux-one-script.sh local
```

Flatpak's temporary source/build/export tree lives under
`.flatpak-builder/pic-build-work` at the checkout root, beside builder state
on the same filesystem. `PIC_FLATPAK_STATE_DIR` relocates both state and this
temporary tree; dependency archives remain in `PIC_BUILD_CACHE`.

Local mode keeps local changes. Approved downloads populate dependency caches;
compilation and packaging run offline. Existing valid cache files are reused.
Add `--appimage-only` or `--flatpak-only` to check/build only that target.

If you prefer to install the basic host tools manually:

KDE Neon and Ubuntu:

```sh
sudo apt update
sudo apt install flatpak flatpak-builder cargo git tar
flatpak remote-add --user --if-not-exists flathub \
  https://dl.flathub.org/repo/flathub.flatpakrepo
```

Fedora:

```sh
sudo dnf install flatpak flatpak-builder cargo git tar
flatpak remote-add --user --if-not-exists flathub \
  https://dl.flathub.org/repo/flathub.flatpakrepo
```

Build the current checkout:

```sh
./scripts/PIC-build-linux-one-script.sh local --flatpak-only
```

To build a GitHub checkout instead, select the branch explicitly. Dependency
setup still asks for approval:

```sh
./scripts/PIC-build-linux-one-script.sh github --branch main --flatpak-only
```

Release tests run inside the Flatpak SDK before bundle export. Their temporary
development app ID lets Glycin decode SVG fixtures in an uninstalled build;
the exported app keeps its normal ID and decoder sandbox. Any test failure
stops packaging. `--skip-tests` is intended only for diagnostic
builds, not published releases.

## Install and launch

Install or replace the generated bundle (substitute its actual filename):

```sh
flatpak install --user --reinstall ./dist/PIC-1.0.0-REVISION-x86_64.flatpak
flatpak run io.github.you.PicRs
```

These commands are the same on KDE Neon, Ubuntu and Fedora. Desktop menus may
need a logout/login after Flatpak is installed for the first time.

PIC retains access to its existing native library at
`~/.local/share/pic-rs/library.db` and thumbnail cache under
`~/.cache/pic-rs`. Network shares use PIC's bundled direct SMB/libnfs
transports; GVfs and manually mounted shares are not required. PIC also uses
the host Avahi service to resolve discovered `.local` SMB/NFS server names
when the Flatpak runtime lacks host `nss-mdns`.

## Optional AppImage

Published AppImages are built by `.github/workflows/appimage-release.yml` on Ubuntu 24.04. That is the oldest Ubuntu LTS whose packaged GTK4 and libadwaita satisfy PIC's current GTK 4.12 and libadwaita 1.5 requirements. The release workflow verifies that the finished AppImage does not reference GLIBC symbols newer than 2.39 before uploading it.

Local AppImage builds remain useful for development, but an AppImage built on a newer host such as Fedora can inherit that host's newer GLIBC requirements and should not be uploaded as the cross-distribution release artifact. Use the GitHub Actions artifact, or the AppImage attached automatically when a GitHub release is published, for public releases and AppImage catalog testing.

```sh
./scripts/PIC-build-linux-one-script.sh local --appimage-only
```

To remove the AppImage and its matching GNOME launcher and icon, pass its path
to the build script:

```sh
./scripts/PIC-build-linux-one-script.sh --uninstall-appimage "$PWD/dist/PIC-1.0.0-REVISION-x86_64.AppImage"
```

The command also removes the launcher and icon if the AppImage file was already
deleted, provided the launcher's `Exec` entry still points to that path. It
leaves the Flatpak launcher, Flatpak icon and PIC photo catalogue in place.

The AppImage is not the primary cross-distribution package because a binary
built on a newer distribution can require a newer GLIBC than the target system.

### Storage migration

PIC stores libraries and backups under `~/.local/share/pic-rs`, the library
registry under `~/.config/pic-rs`, and thumbnails under `~/.cache/pic-rs/thumbs`.
Imported overlay images live in `~/.cache/pic-rs/thumbs/overlay`; retain this
folder when clearing caches manually, since photo edits reference these assets.
The application's thumbnail cleanup preserves overlays.

On first startup, PIC migrates existing `picasa-rs` storage and repairs managed
paths in the library registry and databases. The renamed Flatpak also migrates
application storage from its former `io.github.you.PicasaRs` sandbox. Close the
older version before starting the upgraded application. If matching files exist
in both locations, migration stops and preserves both copies for resolution.
Original photo folders and filenames outside application storage are unchanged.

## Other package tools

From the repository root, run `./scripts/build-deb.sh` or
`./scripts/build-rpm.sh` for host packages. These tools build with Cargo
and require the corresponding package builder.

For Windows, open PowerShell in the folder containing the executable and
run the script by path:

```powershell
powershell.exe -ExecutionPolicy Bypass -File C:\path\to\PIC\scripts\build.windows.ps1
```

The caller's current directory supplies the executable and receives the runtime.

## Software-center app information

The Flatpak package installs AppStream metadata from
`resources/io.github.you.PicRs.metainfo.xml`. It supplies the app description,
developer, release version, project links and screenshot URLs for software
centers. Packaging updates the ID, desktop entry reference and version to
match the selected build, including `PIC_APP_ID` overrides.

To replace the screenshots, overwrite these files while retaining their names:

- `screenshots/all photos.jpg` — default library screenshot
- `screenshots/effects screen.jpg` — editing tools
- `screenshots/photo wall screen.jpg` — Photo Wall

Commit and push the replacements to GitHub `main`. The metadata uses direct
image URLs on that branch so the filenames can stay stable while the images
change. Software centers may cache old images. For a published Flathub release,
pin screenshot URLs to a release tag or commit as recommended by Flathub.

Rebuild the Flatpak to include the new app information:

```sh
./scripts/PIC-build-linux-one-script.sh local --flatpak-only
```

Validate metadata without network access:

```sh
appstreamcli validate --no-net --explain resources/io.github.you.PicRs.metainfo.xml
```

Preview it in GNOME Software if installed:

```sh
gnome-software --show-metainfo resources/io.github.you.PicRs.metainfo.xml
```

Display and caching of screenshots depend on the software center. This adds
package metadata; repository publication remains a separate step.

## AppImage and Flatpak launchers

AppImage GNOME integration creates `io.github.you.PicRs.AppImage.desktop`
and a separate AppImage icon in the user's data directory. Flatpak uses
`io.github.you.PicRs.desktop` and its own icon, so uninstalling the AppImage
does not affect the Flatpak. Existing user-edited AppImage launchers are
preserved during builds; uninstall removes one only when its `Exec` entry
matches the AppImage path supplied.

Older AppImage builds may have left a local `io.github.you.PicRs.desktop`
entry pointing to a removed file. If that happens, back up that specific
AppImage entry outside `~/.local/share/applications/`; keep Flatpak's exported
desktop entry. New builds use the separate AppImage desktop filename.
