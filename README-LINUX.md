# PIC on Linux

Flatpak is PIC's primary Linux package. It builds against the selected GNOME
runtime, so the resulting application does not depend on the host distribution's
GLIBC, GTK4 or libadwaita versions. The AppImage target remains available for
compatible systems.

## Build prerequisites

Use `PIC-build-linux-one-script.sh` for new builds. It checks the selected
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
./PIC-build-linux-one-script.sh local --check-dependencies
```

Build both packages from the current checkout:

```sh
./PIC-build-linux-one-script.sh local
```

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
./PIC-build-linux-one-script.sh local --flatpak-only
```

To build a GitHub checkout instead, select the branch explicitly. Dependency
setup still asks for approval:

```sh
./PIC-build-linux-one-script.sh github --branch main --flatpak-only
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
flatpak run io.github.you.PicasaRs
```

These commands are the same on KDE Neon, Ubuntu and Fedora. Desktop menus may
need a logout/login after Flatpak is installed for the first time.

PIC retains access to its existing native library at
`~/.local/share/picasa-rs/library.db` and thumbnail cache under
`~/.cache/picasa-rs`. Network shares use PIC's bundled direct SMB/libnfs
transports; GVfs and manually mounted shares are not required. PIC also uses
the host Avahi service to resolve discovered `.local` SMB/NFS server names
when the Flatpak runtime lacks host `nss-mdns`.

## Optional AppImage

```sh
./PIC-build-linux-one-script.sh local --appimage-only
```

The AppImage is not the primary cross-distribution package because a binary
built on a newer distribution can require a newer GLIBC than the target system.
