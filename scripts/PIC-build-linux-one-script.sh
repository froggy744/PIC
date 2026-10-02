#!/usr/bin/env bash
# PIC - Picasa iPhoto Clone Linux packager
# Builds local files or GitHub source after checking dependencies and asking
# before installing packages or downloading missing build dependencies.
set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
REPO_ROOT="$SCRIPT_DIR"
if [[ -f "$SCRIPT_DIR/../Cargo.toml" ]]; then
    REPO_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd -P)"
fi
ORIGINAL_ARGS=("$@")
CACHE_ROOT="${PIC_BUILD_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/pic-linux-build}"
TOOLS_DIR="$CACHE_ROOT/tools"
GITHUB_CACHE="$CACHE_ROOT/github-source"
WORK_ROOT="$CACHE_ROOT/work"
FLATPAK_STATE_DIR="${PIC_FLATPAK_STATE_DIR:-$REPO_ROOT/.flatpak-builder}"
FLATPAK_SOURCE_CACHE="$CACHE_ROOT/flatpak-sources"
DIST_DIR="${PIC_DIST_DIR:-$REPO_ROOT/dist}"
REPO_URL="${PIC_REPO_URL:-https://github.com/froggy744/PIC.git}"
DEFAULT_BRANCH="${PIC_BRANCH:-main}"
APP_ID="${PIC_APP_ID:-io.github.you.PicRs}"
BIN_NAME_OVERRIDE="${PIC_BIN_NAME:-}"
BIN_NAME=""
GNOME_RUNTIME="${PIC_GNOME_RUNTIME:-50}"
FDO_RUST_RUNTIME="${PIC_FDO_RUST_RUNTIME:-25.08}"
MODE=""
UNINSTALL_APPIMAGE=""
PROJECT_DIR=""
BRANCH="$DEFAULT_BRANCH"
ONLINE=0
SKIP_TESTS="${PIC_SKIP_TESTS:-0}"
CHECK_DEPENDENCIES_ONLY=0
BUILD_TARGET="${PIC_BUILD_TARGET:-}"
LOG_DIR="${PIC_BUILD_LOG_DIR:-$REPO_ROOT/build-logs}"
LOG_FILE=""
BUILD_STARTED_AT=""
BUILD_STARTED_EPOCH=0
CANCEL_SIGNAL=""

log()  { printf '\n\033[1;34m==>\033[0m %s\n' "$*" >&2; }
ok()   { printf '\033[1;32mOK:\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33mWARN:\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31mERROR:\033[0m %s\n' "$*" >&2; exit 1; }

uninstall_appimage() {
    local requested="$1" appimage launcher icon_name icon_dir data_home
    [[ "$requested" == *.AppImage ]] || die "Uninstall expects a .AppImage file path."
    appimage="$(realpath -m -- "$requested")" || die "Could not resolve AppImage path: $requested"
    data_home="${XDG_DATA_HOME:-$HOME/.local/share}"
    launcher="$data_home/applications/$APP_ID.AppImage.desktop"
    icon_name="$APP_ID.AppImage"
    icon_dir="$data_home/icons/hicolor"

    rm -f -- "$appimage"
    if [[ -f "$launcher" ]] && grep -Fqx -- "Exec=\"$appimage\" %F" "$launcher"; then
        if grep -Fqx -- "Icon=$icon_name" "$launcher"; then
            rm -f -- "$icon_dir/256x256/apps/$icon_name.png" \
                "$icon_dir/scalable/apps/$icon_name.svg"
        fi
        rm -f -- "$launcher"
        printf 'Removed AppImage, GNOME launcher, and AppImage icon: %s\n' "$appimage"
    else
        if [[ ! -e "$launcher" ]]; then
            rm -f -- "$icon_dir/256x256/apps/$icon_name.png" \
                "$icon_dir/scalable/apps/$icon_name.svg"
            printf 'Removed AppImage and orphaned AppImage icon: %s\n' "$appimage"
        else
            printf 'Removed AppImage: %s\n' "$appimage"
            printf 'Kept GNOME launcher because it does not point to this file: %s\n' "$launcher"
        fi
    fi
}

usage() {
    cat <<HELP
PIC Linux build script

Build targets:
  • Both AppImage + Flatpak (default)
  • AppImage only
  • Flatpak only

Uninstall:
  "$SCRIPT_DIR/PIC-build-linux-one-script.sh" --uninstall-appimage /path/to/PIC.AppImage

Source modes:
  local    Build the files already on this PC; missing dependencies need approval.
  github   Clone/update the latest GitHub branch, cache build requirements, then build.

Interactive:
  "$SCRIPT_DIR/PIC-build-linux-one-script.sh"

Direct commands:
  "$SCRIPT_DIR/PIC-build-linux-one-script.sh" local
  "$SCRIPT_DIR/PIC-build-linux-one-script.sh" local --project /home/peet/PIC
  "$SCRIPT_DIR/PIC-build-linux-one-script.sh" github
  "$SCRIPT_DIR/PIC-build-linux-one-script.sh" github --branch main
  "$SCRIPT_DIR/PIC-build-linux-one-script.sh" github --branch editing.phase1
  "$SCRIPT_DIR/PIC-build-linux-one-script.sh" local --appimage-only
  "$SCRIPT_DIR/PIC-build-linux-one-script.sh" local --flatpak-only
  "$SCRIPT_DIR/PIC-build-linux-one-script.sh" local --target appimage

Options:
  --source MODE       local or github
  --project PATH      local project folder (default: checkout root, or standalone script folder)
  --branch NAME       GitHub branch (default: main)
  --dist PATH         output folder (default: dist/ at checkout root or standalone script folder)
  --log-dir PATH      build log folder (default: build-logs/ at checkout root or standalone script folder)
  --target TARGET     both, appimage, or flatpak (default: both)
  --appimage-only     build only the AppImage
  --flatpak-only      build only the Flatpak bundle
  --uninstall-appimage PATH  remove an AppImage and its matching GNOME registration
  --strict-tests      accepted for compatibility; release tests are always fatal
  --skip-tests        do not run cargo test
  --check-dependencies  check/setup dependencies with approval, then exit
  -h, --help          show this help

Useful environment overrides:
  PIC_SKIP_TESTS=1             skip cargo test
  PIC_GNOME_RUNTIME=50         Flatpak GNOME runtime branch
  PIC_FDO_RUST_RUNTIME=25.08   Flatpak Rust SDK-extension branch
  PIC_APP_ID=...               application/Flatpak ID
  PIC_BUILD_CACHE=...          build cache location
  PIC_BUILD_LOG_DIR=...        build log folder
  PIC_BUILD_TARGET=...         both, appimage, or flatpak
  PIC_GNOME_INTEGRATE=0         skip host GNOME icon setup after AppImage build

Dependency setup:
  Every build checks its selected target's requirements before compiling.
  Missing packages, Rust crates, SDKs and tools are listed for approval [y/N].
  Local mode keeps your checkout and only downloads dependencies after approval.
  Compilation and packaging then use the offline caches. With no input or a
  declined prompt, missing dependencies stop the build before compilation.
HELP
}

while (($#)); do
    case "$1" in
        local|github)
            [[ -z "$MODE" ]] || die "Source mode specified more than once."
            MODE="$1"; shift ;;
        --source)
            [[ $# -ge 2 ]] || die "--source needs local or github"
            MODE="$2"; shift 2 ;;
        --uninstall-appimage)
            [[ $# -ge 2 ]] || die "--uninstall-appimage needs an AppImage path"
            [[ -n "$2" ]] || die "--uninstall-appimage needs a non-empty file path"
            [[ -z "$UNINSTALL_APPIMAGE" ]] || die "AppImage uninstall specified more than once."
            UNINSTALL_APPIMAGE="$2"; shift 2 ;;
        --project)
            [[ $# -ge 2 ]] || die "--project needs a path"
            PROJECT_DIR="$2"; shift 2 ;;
        --branch)
            [[ $# -ge 2 ]] || die "--branch needs a branch name"
            BRANCH="$2"; shift 2 ;;
        --dist)
            [[ $# -ge 2 ]] || die "--dist needs a path"
            DIST_DIR="$2"; shift 2 ;;
        --log-dir)
            [[ $# -ge 2 ]] || die "--log-dir needs a path"
            LOG_DIR="$2"; shift 2 ;;
        --target)
            [[ $# -ge 2 ]] || die "--target needs both, appimage, or flatpak"
            [[ -z "$BUILD_TARGET" ]] || die "Build target specified more than once."
            BUILD_TARGET="$2"; shift 2 ;;
        --appimage-only)
            [[ -z "$BUILD_TARGET" ]] || die "Build target specified more than once."
            BUILD_TARGET=appimage; shift ;;
        --flatpak-only)
            [[ -z "$BUILD_TARGET" ]] || die "Build target specified more than once."
            BUILD_TARGET=flatpak; shift ;;
        --strict-tests)
            shift ;;
        --skip-tests)
            SKIP_TESTS=1; shift ;;
        --check-dependencies)
            CHECK_DEPENDENCIES_ONLY=1; shift ;;
        -h|--help)
            usage; exit 0 ;;
        *)
            die "Unknown argument: $1 (use --help)" ;;
    esac
done

if [[ -n "$UNINSTALL_APPIMAGE" ]]; then
    [[ "${#ORIGINAL_ARGS[@]}" -eq 2 && "${ORIGINAL_ARGS[0]}" == --uninstall-appimage ]] || \
        die "--uninstall-appimage runs by itself with one file path."
    uninstall_appimage "$UNINSTALL_APPIMAGE"
    exit 0
fi

start_logging() {
    mkdir -p "$LOG_DIR"
    LOG_DIR="$(cd -- "$LOG_DIR" && pwd -P)"
    local stamp
    stamp="$(date '+%Y-%m-%d-%H%M%S')"
    LOG_FILE="$LOG_DIR/build-${stamp}-$$.log"
    : > "$LOG_FILE"

    # Stream all subsequent stdout/stderr to both the terminal and the log.
    # The file is written continuously, so it remains useful if the build is
    # interrupted before AppImage/Flatpak packaging finishes.
    # Keep the logger alive through Ctrl+C/TERM so the cancellation footer can
    # still be written. It exits naturally when this script closes the pipe.
    exec > >(trap '' INT TERM HUP; exec tee -a "$LOG_FILE") 2>&1

    BUILD_STARTED_EPOCH="$(date '+%s')"
    BUILD_STARTED_AT="$(date '+%Y-%m-%dT%H:%M:%S%z')"
    printf '%s\n' '============================================================'
    printf '%s\n' 'PIC Linux Packager build log'
    printf 'Started: %s\n' "$BUILD_STARTED_AT"
    printf 'PID:     %s\n' "$$"
    printf 'Script:  %s\n' "$0"
    printf 'Command:'
    printf ' %q' "$0" "${ORIGINAL_ARGS[@]}"
    printf '\n'
    printf '%s\n' '============================================================'
}

handle_signal() {
    local signal="$1" code="$2"
    CANCEL_SIGNAL="$signal"
    printf '\n%s\n' '============================================================'
    printf '%s\n' 'BUILD CANCELLED'
    printf 'Signal: %s\n' "$signal"
    printf 'Time:   %s\n' "$(date '+%Y-%m-%dT%H:%M:%S%z')"
    printf '%s\n' '============================================================'
    exit "$code"
}

finish_logging() {
    local status=$?
    trap - EXIT
    local finished finished_epoch elapsed duration outcome
    finished_epoch="$(date '+%s')"
    finished="$(date '+%Y-%m-%dT%H:%M:%S%z')"
    elapsed=$((finished_epoch-BUILD_STARTED_EPOCH))
    printf -v duration '%02d:%02d:%02d' \
        "$((elapsed/3600))" "$(((elapsed%3600)/60))" "$((elapsed%60))"

    if [[ -n "$CANCEL_SIGNAL" ]]; then
        outcome="CANCELLED"
    elif ((status == 130)); then
        CANCEL_SIGNAL="INT"
        outcome="CANCELLED"
    elif ((status == 143)); then
        CANCEL_SIGNAL="TERM"
        outcome="CANCELLED"
    elif ((status == 0)); then
        outcome="SUCCESS"
    else
        outcome="FAILED"
    fi

    printf '\n%s\n' '============================================================'
    printf 'Build session: %s\n' "$outcome"
    printf 'Started:       %s\n' "$BUILD_STARTED_AT"
    printf 'Ended:         %s\n' "$finished"
    printf 'Duration:      %s (HH:MM:SS)\n' "$duration"
    printf 'Exit code:     %s\n' "$status"
    [[ -z "$CANCEL_SIGNAL" ]] || printf 'Signal:        %s\n' "$CANCEL_SIGNAL"
    printf 'Build log: %s\n' "$LOG_FILE"
    printf '%s\n' '============================================================'
    return "$status"
}

start_logging
trap 'handle_signal INT 130' INT
trap 'handle_signal TERM 143' TERM
trap finish_logging EXIT

interactive_menu() {
    printf '\nPIC Linux Packager\n'
    printf '  1) Local files  (dependency downloads require approval)\n'
    printf '  2) GitHub latest\n'
    printf '  3) Exit\n\n'
    read -r -p 'Choose [1-3]: ' choice
    case "$choice" in
        1)
            MODE=local
            read -r -p "Project folder [$REPO_ROOT]: " PROJECT_DIR
            PROJECT_DIR="${PROJECT_DIR:-$REPO_ROOT}"
            ;;
        2)
            MODE=github
            read -r -p "Git branch [$DEFAULT_BRANCH]: " BRANCH
            BRANCH="${BRANCH:-$DEFAULT_BRANCH}"
            ;;
        3) exit 0 ;;
        *) die "Invalid choice." ;;
    esac

    if [[ -z "$BUILD_TARGET" ]]; then
        printf '\nWhat do you want to build?\n'
        printf '  1) Both AppImage + Flatpak\n'
        printf '  2) AppImage only\n'
        printf '  3) Flatpak only\n\n'
        read -r -p 'Choose [1-3]: ' target_choice
        case "$target_choice" in
            1|'') BUILD_TARGET=both ;;
            2) BUILD_TARGET=appimage ;;
            3) BUILD_TARGET=flatpak ;;
            *) die "Invalid build target." ;;
        esac
    fi
}

[[ -n "$MODE" ]] || interactive_menu
[[ "$MODE" == local || "$MODE" == github ]] || die "Source mode must be local or github."
BUILD_TARGET="${BUILD_TARGET:-both}"
[[ "$BUILD_TARGET" == both || "$BUILD_TARGET" == appimage || "$BUILD_TARGET" == flatpak ]] || \
    die "Build target must be both, appimage, or flatpak."

mkdir -p "$CACHE_ROOT" "$TOOLS_DIR" "$WORK_ROOT" "$DIST_DIR"

have() { command -v "$1" >/dev/null 2>&1; }

find_gdk_pixbuf_svg_loader() {
    find /usr/lib /usr/lib64 /lib /lib64 -type f \
        -path '*/gdk-pixbuf-2.0/*/loaders/libpixbufloader-svg.so' \
        -print -quit 2>/dev/null || true
}

find_gdk_pixbuf_query_loaders() {
    local candidate
    for candidate in gdk-pixbuf-query-loaders gdk-pixbuf-query-loaders-64; do
        if have "$candidate"; then
            command -v "$candidate"
            return 0
        fi
    done
    find /usr/lib /usr/lib64 /lib /lib64 -type f \
        -name 'gdk-pixbuf-query-loaders*' -perm -u+x \
        -print -quit 2>/dev/null || true
}

# Host package names are selected for Fedora or Debian/Ubuntu. Other systems
# still get a complete missing-dependency report and manual setup instructions.
add_host_dependency() {
    local description="$1" fedora_package="$2" debian_package="$3" package existing
    MISSING_DEPENDENCIES+=("$description")
    if [[ "$PACKAGE_MANAGER" == dnf ]]; then
        package="$fedora_package"
    else
        package="$debian_package"
    fi
    for existing in "${HOST_PACKAGES[@]}"; do
        [[ "$existing" != "$package" ]] || return 0
    done
    HOST_PACKAGES+=("$package")
}

require_host_command() {
    have "$1" || add_host_dependency "Host command: $1" "$2" "$3"
}

collect_host_dependencies() {
    require_host_command cargo cargo cargo
    require_host_command tar tar tar
    require_host_command awk gawk gawk
    require_host_command sed sed sed
    require_host_command sha256sum coreutils coreutils
    [[ "$MODE" != github ]] || require_host_command git git git
    if [[ "$BUILD_TARGET" != flatpak ]]; then
        require_host_command rustc rust rustc
        require_host_command pkg-config pkgconf-pkg-config pkg-config
        require_host_command cmake cmake cmake
        require_host_command cc gcc build-essential
        require_host_command c++ gcc-c++ build-essential
        require_host_command make make make
        require_host_command file file file
        require_host_command patchelf patchelf patchelf
        require_host_command nasm nasm nasm
        # This repository's native Cargo configuration uses clang and mold.
        # Custom source folders only need them when their config requests them.
        local config="${SOURCE_DIR:-}/.cargo/config.toml"
        if [[ -f "$config" ]]; then
            if awk '/^[[:space:]]*linker[[:space:]]*=.*"clang"/ { found=1 } END { exit !found }' "$config"; then
                require_host_command clang clang clang
            fi
            if awk '/^[[:space:]]*rustflags[[:space:]]*=.*fuse-ld=mold/ { found=1 } END { exit !found }' "$config"; then
                require_host_command mold mold mold
            fi
        fi
        if ! have pkg-config || ! pkg-config --exists 'gtk4 >= 4.12'; then
            add_host_dependency "GTK4 development files >= 4.12" gtk4-devel libgtk-4-dev
        fi
        if ! have pkg-config || ! pkg-config --exists 'libadwaita-1 >= 1.5'; then
            add_host_dependency "libadwaita development files >= 1.5" libadwaita-devel libadwaita-1-dev
        fi
        if ! have pkg-config || ! pkg-config --exists smbclient; then
            add_host_dependency "SMB development files" libsmbclient-devel libsmbclient-dev
        fi
        if ! have pkg-config || ! pkg-config --exists libnfs; then
            add_host_dependency "NFS development files" libnfs-devel libnfs-dev
        fi
        # SVG icons are decoded through a dynamically loaded GdkPixbuf plugin.
        # linuxdeploy cannot discover that plugin from ELF dependencies alone.
        if [[ -z "$(find_gdk_pixbuf_svg_loader)" ]]; then
            add_host_dependency "GdkPixbuf SVG loader" librsvg2 librsvg2-common
        fi
        if [[ -z "$(find_gdk_pixbuf_query_loaders)" ]]; then
            add_host_dependency "GdkPixbuf loader cache tool" gdk-pixbuf2 libgdk-pixbuf2.0-bin
        fi
    fi
    if [[ "$BUILD_TARGET" != appimage ]]; then
        require_host_command flatpak flatpak flatpak
        require_host_command flatpak-builder flatpak-builder flatpak-builder
    fi
    if [[ -n "${SOURCE_DIR:-}" ]] && ! have magick && ! have convert; then
        local icon
        icon="$(project_icon_candidate)"
        if [[ "${icon,,}" == *.png ]]; then
            add_host_dependency "ImageMagick (PNG application icon resizing)" ImageMagick imagemagick
        fi
    fi
}

validate_project() {
    local dir="$1"
    [[ -d "$dir" ]] || die "Project directory does not exist: $dir"
    [[ -f "$dir/Cargo.toml" ]] || die "Cargo.toml not found in: $dir"
    [[ -f "$dir/Cargo.lock" ]] || die "Cargo.lock not found. Commit/generate Cargo.lock first for repeatable offline builds."
}

prepare_local_source() {
    ONLINE=0
    PROJECT_DIR="${PROJECT_DIR:-$REPO_ROOT}"
    PROJECT_DIR="$(cd -- "$PROJECT_DIR" && pwd -P)"
    validate_project "$PROJECT_DIR"
    SOURCE_DIR="$PROJECT_DIR"
    log "LOCAL source selected"
    printf 'Source: %s\n' "$SOURCE_DIR"
    printf 'Dependency downloads: only with approval; builds run offline\n'
    if [[ -d "$SOURCE_DIR/.git" ]] && have git; then
        local branch dirty
        branch="$(git -C "$SOURCE_DIR" branch --show-current 2>/dev/null || true)"
        dirty="$(git -C "$SOURCE_DIR" status --porcelain 2>/dev/null || true)"
        printf 'Git branch: %s\n' "${branch:-detached/unknown}"
        [[ -z "$dirty" ]] || warn "Local tree has uncommitted changes. They WILL be included in this build."
    fi
}

prepare_github_source() {
    ONLINE=1
    log "GitHub latest source selected"
    printf 'Repository: %s\nBranch: %s\n' "$REPO_URL" "$BRANCH"

    if [[ -d "$GITHUB_CACHE/.git" ]]; then
        log "Updating clean cached GitHub checkout"
        git -C "$GITHUB_CACHE" remote set-url origin "$REPO_URL"
        git -C "$GITHUB_CACHE" fetch --prune --depth 1 origin "$BRANCH"
        git -C "$GITHUB_CACHE" reset --hard FETCH_HEAD
        # Keep ignored Cargo target/cache directories; remove only untracked non-ignored files.
        git -C "$GITHUB_CACHE" clean -fd
    else
        rm -rf "$GITHUB_CACHE"
        git clone --depth 1 --branch "$BRANCH" "$REPO_URL" "$GITHUB_CACHE"
    fi

    SOURCE_DIR="$GITHUB_CACHE"
    validate_project "$SOURCE_DIR"

    # Dependency preflight below checks this checkout's Rust cache and requests
    # approval before fetching missing crates. Builds always use --offline.
}

# Module archives required by the generated Flatpak manifest (name|sha256|url).
# Cached durably so 'local' builds work offline with --disable-download.
FLATPAK_MODULE_SOURCES=(
    "libnfs-6.0.2.tar.gz|4e5459cc3e0242447879004e9ad28286d4d27daa42cbdcde423248fad911e747|https://github.com/sahlberg/libnfs/archive/libnfs-6.0.2.tar.gz"
    "samba-4.24.7.tar.gz|45b7747a47452eff2b2159a44cc63eb43690d339fd1069088e023a015fed06c7|https://download.samba.org/pub/samba/stable/samba-4.24.7.tar.gz"
    "Parse-Yapp-1.21.tar.gz|3810e998308fba2e0f4f26043035032b027ce51ce5c8a52a8b8e340ca65f13e5|https://cpan.metacpan.org/authors/id/W/WB/WBRASWELL/Parse-Yapp-1.21.tar.gz"
)

verify_sha256() {
    local file="$1" expected="$2" actual
    actual="$(sha256sum "$file" | awk '{print $1}')" || return 1
    [[ "$actual" == "$expected" ]]
}

ensure_flatpak_module_sources() {
    log "Caching Flatpak module source archives"
    local entry name sha url durable dest missing=()
    mkdir -p "$FLATPAK_SOURCE_CACHE" "$FLATPAK_STATE_DIR/downloads"

    for entry in "${FLATPAK_MODULE_SOURCES[@]}"; do
        IFS='|' read -r name sha url <<<"$entry"
        durable="$FLATPAK_SOURCE_CACHE/$sha/$name"
        dest="$FLATPAK_STATE_DIR/downloads/$sha/$name"

        if [[ -f "$dest" ]] && verify_sha256 "$dest" "$sha"; then
            continue
        fi
        if [[ -f "$durable" ]] && verify_sha256 "$durable" "$sha"; then
            mkdir -p "$(dirname "$dest")"
            cp -f "$durable" "$dest"
            continue
        fi
        if ((ONLINE)); then
            printf '  Downloading %s\n' "$name"
            if ! download_file "$url" "$durable" "$sha"; then
                rm -f "$durable.tmp"
                die "Could not download or verify Flatpak module source: $name ($url)"
            fi
            mkdir -p "$(dirname "$dest")"
            cp -f "$durable" "$dest"
        else
            missing+=("$name")
        fi
    done

    if ((${#missing[@]})); then
        printf '\nOffline Flatpak build is missing module source archives:\n' >&2
        printf '  %s\n' "${missing[@]}" >&2
        printf '\nCache location: %s\n' "$FLATPAK_SOURCE_CACHE" >&2
        printf "Run '%s local --check-dependencies' to approve downloading them.\n" "$0" >&2
        return 1
    fi
}

download_file() {
    local url="$1" dest="$2" expected_sha="${3:-}"
    local parent tmp
    parent="$(dirname "$dest")"
    mkdir -p "$parent"
    tmp="$dest.tmp"
    rm -f "$tmp"
    if have curl; then
        if ! curl -fL --retry 3 --connect-timeout 20 -o "$tmp" "$url"; then
            rm -f "$tmp"
            return 1
        fi
    elif have wget; then
        if ! wget -O "$tmp" "$url"; then
            rm -f "$tmp"
            return 1
        fi
    else
        die "Need curl or wget to download $url"
    fi
    if [[ -n "$expected_sha" ]]; then
        if ! verify_sha256 "$tmp" "$expected_sha"; then
            rm -f "$tmp"
            return 1
        fi
    fi
    mv -f "$tmp" "$dest"
}

linuxdeploy_path() {
    local machine tool arch_url
    machine="$(uname -m)"
    case "$machine" in
        x86_64|amd64) arch_url=x86_64 ;;
        i386|i486|i586|i686) arch_url=i386 ;;
        *) arch_url="$machine" ;;
    esac

    tool="$TOOLS_DIR/linuxdeploy-${arch_url}.AppImage"
    if [[ ! -x "$tool" ]]; then
        if have linuxdeploy; then
            command -v linuxdeploy
            return 0
        fi
        case "$arch_url" in
            x86_64|i386) ;;
            *)
                warn "AppImage skipped: automatic linuxdeploy download supports x86_64/i386 here. Install linuxdeploy manually for $machine."
                return 1 ;;
        esac
        if ((ONLINE)); then
            log "Caching linuxdeploy (one-time online setup)"
            if ! download_file \
                "https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-${arch_url}.AppImage" \
                "$tool"; then
                warn "AppImage skipped: linuxdeploy could not be downloaded."
                return 1
            fi
            chmod +x "$tool"
        else
            warn "AppImage skipped: linuxdeploy is not cached. Run '$0 local --check-dependencies' to approve downloading it."
            return 1
        fi
    fi
    printf '%s\n' "$tool"
}

ensure_flatpak_runtime() {
    local refs=(
        "org.gnome.Platform//$GNOME_RUNTIME"
        "org.gnome.Sdk//$GNOME_RUNTIME"
        "org.freedesktop.Sdk.Extension.rust-stable//$FDO_RUST_RUNTIME"
    )
    local missing=() ref
    for ref in "${refs[@]}"; do
        flatpak info "$ref" >/dev/null 2>&1 || missing+=("$ref")
    done

    if ((${#missing[@]})) && ((ONLINE)); then
        log "Installing missing Flatpak build runtimes for the current user"
        flatpak remote-add --user --if-not-exists flathub \
            https://dl.flathub.org/repo/flathub.flatpakrepo
        flatpak install --user -y flathub "${missing[@]}"
    elif ((${#missing[@]})); then
        printf '\nMissing Flatpak runtime/SDK required for OFFLINE mode:\n' >&2
        printf '  %s\n' "${missing[@]}" >&2
        printf "\nRun '%s local --check-dependencies' to approve installing them.\n" "$0" >&2
        return 1
    fi

    verify_flatpak_sdk_compatibility
}

verify_flatpak_sdk_compatibility() {
    local gnome_metadata rust_metadata supported rust_base
    gnome_metadata="$(flatpak info --show-metadata "org.gnome.Sdk//$GNOME_RUNTIME")" || return 1
    rust_metadata="$(flatpak info --show-metadata \
        "org.freedesktop.Sdk.Extension.rust-stable//$FDO_RUST_RUNTIME")" || return 1
    supported="$(awk '
        /^\[Extension org[.]freedesktop[.]Platform[.]GL\]$/ { found=1; next }
        found && /^versions[[:space:]]*=/ { sub(/^[^=]*=[[:space:]]*/, ""); print; exit }
    ' <<<"$gnome_metadata")"
    rust_base="$(awk -F/ '
        /^runtime=org[.]freedesktop[.]Sdk\// { print $NF; exit }
    ' <<<"$rust_metadata")"
    [[ ";$supported;" == *";$FDO_RUST_RUNTIME;"* ]] || die \
        "GNOME SDK $GNOME_RUNTIME is based on a different Freedesktop SDK (supported: ${supported:-unknown}); Rust extension $FDO_RUST_RUNTIME is incompatible."
    [[ "$rust_base" == "$FDO_RUST_RUNTIME" ]] || die \
        "Rust SDK extension metadata targets ${rust_base:-unknown}, expected $FDO_RUST_RUNTIME."
    ok "Compatible Flatpak SDKs: GNOME $GNOME_RUNTIME / Freedesktop Rust $FDO_RUST_RUNTIME"
}

# Read-only inventory: never invoke a helper that downloads during this pass.
collect_dependencies() {
    MISSING_DEPENDENCIES=()
    HOST_PACKAGES=()
    NEED_LINUXDEPLOY=0
    NEED_FLATPAK_SOURCES=0
    NEED_FLATPAK_RUNTIME=0
    NEED_RUST_CRATES=0
    PACKAGE_MANAGER=""
    if have dnf; then PACKAGE_MANAGER=dnf;
    elif have apt-get; then PACKAGE_MANAGER=apt-get; fi
    collect_host_dependencies

    local arch entry name sha url ref diagnostic
    if [[ "$BUILD_TARGET" != flatpak ]]; then
        arch="$(uname -m)"
        [[ "$arch" != amd64 ]] || arch=x86_64
        case "$arch" in i386|i486|i586|i686) arch=i386 ;; esac
        if [[ ! -x "$TOOLS_DIR/linuxdeploy-$arch.AppImage" ]] && ! have linuxdeploy; then
            MISSING_DEPENDENCIES+=("AppImage tool: linuxdeploy ($arch)")
            NEED_LINUXDEPLOY=1
        fi
    fi
    if [[ "$BUILD_TARGET" != appimage ]]; then
        for ref in "org.gnome.Platform//$GNOME_RUNTIME" "org.gnome.Sdk//$GNOME_RUNTIME" \
                   "org.freedesktop.Sdk.Extension.rust-stable//$FDO_RUST_RUNTIME"; do
            if ! have flatpak || ! flatpak info "$ref" >/dev/null 2>&1; then
                MISSING_DEPENDENCIES+=("Flatpak runtime/SDK: $ref")
                NEED_FLATPAK_RUNTIME=1
            fi
        done
        for entry in "${FLATPAK_MODULE_SOURCES[@]}"; do
            IFS='|' read -r name sha url <<<"$entry"
            if have sha256sum && \
               { { [[ -f "$FLATPAK_SOURCE_CACHE/$sha/$name" ]] && \
                   verify_sha256 "$FLATPAK_SOURCE_CACHE/$sha/$name" "$sha"; } || \
                 { [[ -f "$FLATPAK_STATE_DIR/downloads/$sha/$name" ]] && \
                   verify_sha256 "$FLATPAK_STATE_DIR/downloads/$sha/$name" "$sha"; }; }; then
                continue
            fi
            MISSING_DEPENDENCIES+=("Flatpak source archive: $name (missing or invalid checksum)")
            NEED_FLATPAK_SOURCES=1
        done
    fi
    if ((NEED_LINUXDEPLOY || NEED_FLATPAK_SOURCES)) && ! have curl && ! have wget; then
        add_host_dependency "Downloader: curl or wget" curl curl
    fi
    if [[ -n "${SOURCE_DIR:-}" ]]; then
        diagnostic="$(mktemp "$WORK_ROOT/cargo-dependencies.XXXXXX")"
        if ! have cargo || ! (cd "$SOURCE_DIR" && cargo metadata --locked --offline \
            --format-version 1 > /dev/null 2> "$diagnostic"); then
            MISSING_DEPENDENCIES+=("Rust crates: resolve/fetch the locked dependency cache for $SOURCE_DIR")
            NEED_RUST_CRATES=1
            [[ ! -s "$diagnostic" ]] || cat "$diagnostic" >&2
        fi
        rm -f "$diagnostic"
    fi
}

dependency_preflight() {
    log "Checking build dependencies before compilation ($BUILD_TARGET)"
    collect_dependencies
    if ((${#MISSING_DEPENDENCIES[@]})); then
        printf '\nMissing build dependencies:\n'
        printf '  - %s\n' "${MISSING_DEPENDENCIES[@]}"
        local install_command=() answer tool saved_online="$ONLINE"
        if ((${#HOST_PACKAGES[@]})); then
            [[ -n "$PACKAGE_MANAGER" ]] || \
                die "Automatic host installation supports dnf or apt-get. Install the listed host dependencies manually."
            if [[ "$(id -u)" != 0 ]]; then
                have sudo || die "sudo is required to install host packages; install them manually and rerun."
                install_command+=(sudo)
            fi
            install_command+=("$PACKAGE_MANAGER" install -y "${HOST_PACKAGES[@]}")
            printf '\nHost installation command:'
            printf ' %q' "${install_command[@]}"
            printf '\n'
        fi
        printf '\nSetup may use the internet and install the listed packages/SDKs or populate build caches.\n'
        printf 'The selected source remains: %s\n' "${SOURCE_DIR:-GitHub branch $BRANCH}"
        printf 'Install/download these missing dependencies now? [y/N]: '
        if ! read -r answer; then
            die "Dependency installation requires explicit approval; no input was received."
        fi
        case "$answer" in
            y|Y|yes|YES|Yes) ;;
            *) die "Dependency installation declined; stopping before compilation." ;;
        esac

        if ((${#install_command[@]})); then
            "${install_command[@]}" || die "Host dependency installation failed."
        fi
        # This network permission applies only to approved dependency setup.
        # Local source selection and offline build flags remain unchanged.
        ONLINE=1
        if ((NEED_LINUXDEPLOY)); then
            tool="$(linuxdeploy_path)" || die "Could not install linuxdeploy."
            [[ -x "$tool" ]] || die "Downloaded linuxdeploy is not executable."
        fi
        if ((NEED_FLATPAK_RUNTIME)); then
            ensure_flatpak_runtime || die "Could not install compatible Flatpak runtimes/SDKs."
        fi
        if ((NEED_FLATPAK_SOURCES)); then
            ensure_flatpak_module_sources || die "Could not cache Flatpak source archives."
        fi
        if ((NEED_RUST_CRATES)); then
            (cd "$SOURCE_DIR" && cargo fetch --locked) || die "Could not fetch locked Rust dependencies."
        fi
        ONLINE="$saved_online"
        collect_dependencies
        if ((${#MISSING_DEPENDENCIES[@]})); then
            printf '\nDependencies still unavailable after setup:\n'
            printf '  - %s\n' "${MISSING_DEPENDENCIES[@]}"
            die "Dependency setup is incomplete; stopping before compilation."
        fi
    fi
    if [[ "$BUILD_TARGET" != appimage ]]; then
        verify_flatpak_sdk_compatibility
    fi
    ok "All build dependencies are ready."
}

project_binary_name() {
    # Prefer the first explicit [[bin]] name. If Cargo.toml has no [[bin]],
    # Cargo uses the [package] name for src/main.rs.
    local explicit package
    explicit="$(awk '
        /^\[\[bin\]\]/ { in_bin=1; next }
        /^\[/ { if (in_bin) exit }
        in_bin && /^[[:space:]]*name[[:space:]]*=/ {
            line=$0; sub(/^[^"]*"/, "", line); sub(/".*$/, "", line); print line; exit
        }
    ' "$SOURCE_DIR/Cargo.toml")"
    if [[ -n "$explicit" ]]; then
        printf '%s\n' "$explicit"
        return
    fi
    package="$(awk '
        /^\[package\]/ { in_package=1; next }
        /^\[/ { if (in_package) exit }
        in_package && /^[[:space:]]*name[[:space:]]*=/ {
            line=$0; sub(/^[^"]*"/, "", line); sub(/".*$/, "", line); print line; exit
        }
    ' "$SOURCE_DIR/Cargo.toml")"
    [[ -n "$package" ]] || die "Could not determine the Cargo binary name. Set PIC_BIN_NAME manually."
    printf '%s\n' "$package"
}

project_version() {
    awk -F'"' '/^[[:space:]]*version[[:space:]]*=/ {print $2; exit}' "$SOURCE_DIR/Cargo.toml"
}

project_revision() {
    if [[ -d "$SOURCE_DIR/.git" ]] && have git; then
        git -C "$SOURCE_DIR" rev-parse --short=10 HEAD 2>/dev/null || true
    fi
}

normalize_png_icon() {
    local icon="$1" tmp
    tmp="${icon}.resize-tmp.png"

    if have magick; then
        magick "$icon" -resize '256x256' -background none -gravity center -extent '256x256' "$tmp"
    elif have convert; then
        convert "$icon" -resize '256x256' -background none -gravity center -extent '256x256' "$tmp"
    else
        warn "PNG application icon needs ImageMagick so it can be staged at 256x256. Install it with: sudo dnf install ImageMagick"
        return 1
    fi

    mv -f "$tmp" "$icon"
}

project_icon_candidate() {
    local candidate
    local candidates=(
        "$SOURCE_DIR/icon/pic-icon.png"
        "$SOURCE_DIR/icon/pic-icon.svg"
        # Prefer the highest-resolution shipped PIC icon. The old generic
        # find fallback selected pic-16.png first and stretched it to 256px.
        "$SOURCE_DIR/icon/pic-1024.png"
        "$SOURCE_DIR/icon/pic-512.png"
        "$SOURCE_DIR/icon/pic-256.png"
        "$SOURCE_DIR/icon/pic-128.png"
        "$SOURCE_DIR/icon/icon.png"
        "$SOURCE_DIR/icon/icon.svg"
    )
    for candidate in "${candidates[@]}"; do
        if [[ -f "$candidate" ]]; then
            printf '%s\n' "$candidate"
            return 0
        fi
    done
    find "$SOURCE_DIR" -maxdepth 3 -type f \( -iname '*.png' -o -iname '*.svg' \) \
        ! -path '*/target/*' ! -path '*/screenshots/*' | head -n 1 || true
}

find_or_make_icon() {
    local out_dir="$1" candidate
    candidate="$(project_icon_candidate)"
    if [[ -n "$candidate" ]]; then
        ICON_EXT="${candidate##*.}"
        ICON_EXT="${ICON_EXT,,}"
        ICON_FILE="$out_dir/$APP_ID.$ICON_EXT"
        cp -f "$candidate" "$ICON_FILE"
        if [[ "$ICON_EXT" == png ]]; then
            normalize_png_icon "$ICON_FILE" || return 1
        fi
        [[ "$candidate" == "$SOURCE_DIR/icon/"* ]] || \
            warn "Expected application icon was not found; using $candidate"
        return 0
    fi

    ICON_EXT=svg
    ICON_FILE="$out_dir/$APP_ID.svg"
    cat > "$ICON_FILE" <<'SVG'
<svg xmlns="http://www.w3.org/2000/svg" width="256" height="256" viewBox="0 0 256 256">
  <rect width="256" height="256" rx="48" fill="#3584e4"/>
  <rect x="42" y="72" width="172" height="124" rx="22" fill="#fff"/>
  <circle cx="128" cy="134" r="42" fill="#3584e4"/>
  <circle cx="128" cy="134" r="24" fill="#fff"/>
  <path d="M82 72l18-24h56l18 24z" fill="#fff"/>
</svg>
SVG
    warn "No project icon found; generated a temporary PIC camera icon."
}

write_runtime_launcher() {
    local path="$1"
    cat > "$path" <<EOF_LAUNCHER
#!/bin/sh
set -eu

# PIC predates its Flatpak package and must continue to use the existing native
# library and thumbnail cache. The host filesystem grant makes these available;
# keep dirs(3) pointed at their established host XDG locations.
if [ -n "\${FLATPAK_ID:-}" ]; then
    export XDG_DATA_HOME="\${PIC_XDG_DATA_HOME:-\$HOME/.local/share}"
    export XDG_CACHE_HOME="\${PIC_XDG_CACHE_HOME:-\$HOME/.cache}"
fi

# AppImage launches this script through the top-level AppRun symlink. In that
# case \$0 points at AppRun, not usr/bin/$BIN_NAME, so derive the prefix from
# APPDIR (set by the AppImage runtime). Flatpak launches /app/bin/$BIN_NAME
# directly and does not set APPDIR, so keep the normal bin-directory fallback.
if [ -n "\${APPDIR:-}" ] && [ -d "\$APPDIR/usr/share/$BIN_NAME" ]; then
    PREFIX="\$APPDIR/usr"
    # Make GTK/GIO discover data bundled in the AppImage before host data.
    # This is required for named Adwaita symbolic icons to render consistently
    # on hosts whose installed icon theme differs from the build environment.
    export XDG_DATA_DIRS="\$PREFIX/share\${XDG_DATA_DIRS:+:\$XDG_DATA_DIRS}"

    # SVG support is a dynamically loaded GdkPixbuf plugin. Its cache must
    # contain the real AppImage mount path, which only exists at launch time.
    PIXBUF_CACHE_TEMPLATE="\$PREFIX/lib/gdk-pixbuf-2.0/2.10.0/pic-svg-loaders.cache.in"
    if [ -f "\$PIXBUF_CACHE_TEMPLATE" ]; then
        PIC_CACHE_ROOT="\${XDG_CACHE_HOME:-\${HOME:-/tmp}/.cache}/pic-rs"
        mkdir -p "\$PIC_CACHE_ROOT"
        GDK_PIXBUF_MODULE_FILE="\$PIC_CACHE_ROOT/gdk-pixbuf-svg-loaders.cache"
        PIC_PIXBUF_PREFIX_ESCAPED="\$(printf '%s' "\$PREFIX" | sed 's/[\\\\&|]/\\\\&/g')"
        sed "s|@PIC_PREFIX@|\$PIC_PIXBUF_PREFIX_ESCAPED|g" \
            "\$PIXBUF_CACHE_TEMPLATE" > "\$GDK_PIXBUF_MODULE_FILE.tmp"
        mv -f "\$GDK_PIXBUF_MODULE_FILE.tmp" "\$GDK_PIXBUF_MODULE_FILE"
        export GDK_PIXBUF_MODULE_FILE
    fi
else
    BIN_DIR="\$(CDPATH= cd -- "\$(dirname -- "\$0")" && pwd)"
    PREFIX="\$(dirname -- "\$BIN_DIR")"
fi

cd "\$PREFIX/share/$BIN_NAME"
exec "\$PREFIX/libexec/$BIN_NAME" "\$@"
EOF_LAUNCHER
    chmod +x "$path"
}

copy_runtime_resources() {
    local resource_root="$1"
    local folder
    mkdir -p "$resource_root"
    for folder in images themes resources; do
        [[ -d "$SOURCE_DIR/$folder" ]] || die "Required runtime resource folder missing: $SOURCE_DIR/$folder"
        [[ -n "$(find "$SOURCE_DIR/$folder" -type f -print -quit)" ]] || die \
            "Required runtime resource folder is empty: $SOURCE_DIR/$folder"
        rm -rf "$resource_root/$folder"
        cp -a "$SOURCE_DIR/$folder" "$resource_root/$folder"
        ok "Bundled runtime resources: $resource_root/$folder"
    done
}

validate_packaging_resources() {
    local required
    for required in images themes resources resources/icons.gresource; do
        [[ -e "$SOURCE_DIR/$required" ]] || die "Required application resource missing: $SOURCE_DIR/$required"
    done
    find "$SOURCE_DIR/icon" -maxdepth 1 -type f \( -iname '*.png' -o -iname '*.svg' \) \
        -print -quit 2>/dev/null | grep -q . || die "Required application icon missing from $SOURCE_DIR/icon"
}

write_desktop_file() {
    local path="$1"
    cat > "$path" <<EOF_DESKTOP
[Desktop Entry]
Type=Application
Name=PIC — Personal Image Catalogue
Comment=Fast local photo manager inspired by Picasa and iPhoto
Exec=$BIN_NAME %F
Icon=$APP_ID
Terminal=false
StartupNotify=true
Categories=Graphics;Photography;
MimeType=image/jpeg;image/png;image/webp;image/gif;image/tiff;image/bmp;image/avif;image/heif;
EOF_DESKTOP
}

write_metainfo_file() {
    local path="$1"
    python3 - "$SOURCE_DIR/resources/io.github.you.PicRs.metainfo.xml" "$path" "$APP_ID" "$VERSION" <<'EOF_METAINFO'
from pathlib import Path
import sys
import xml.etree.ElementTree as ET

source, destination, app_id, version = sys.argv[1:]
tree = ET.parse(source)
component = tree.getroot()
component.find('id').text = app_id
component.find('launchable').text = app_id + '.desktop'
component.find('releases/release').set('version', version)
Path(destination).parent.mkdir(parents=True, exist_ok=True)
tree.write(destination, encoding='utf-8', xml_declaration=True)
EOF_METAINFO
}

copy_source_tree() {
    local dest="$1"
    rm -rf "$dest"
    mkdir -p "$dest"
    (cd "$SOURCE_DIR" && tar \
        --exclude='./.git' \
        --exclude='./target' \
        --exclude='./dist' \
        --exclude='./build-logs' \
        --exclude='./.flatpak-builder' \
        --exclude='./*.log' \
        --exclude='.worktrees' \
        --exclude='worktrees' \
        --exclude='to-be-deleted' \
        --exclude='.superpowers' \
        --exclude='__pycache__' \
        --exclude='./logs' \
        --exclude='build.windows.log' \
        -cf - .) | (cd "$dest" && tar -xf -)
}

build_native() {
    log "Testing/building Rust release binary OFFLINE"
    if [[ "$SKIP_TESTS" != 1 ]]; then
        (cd "$SOURCE_DIR" && cargo test --release --locked --offline) || \
            die "cargo test failed; refusing to create a release package."
    else
        warn "Tests skipped (--skip-tests / PIC_SKIP_TESTS=1)."
    fi
    (cd "$SOURCE_DIR" && cargo build --release --locked --offline)
    NATIVE_BIN="$SOURCE_DIR/target/release/$BIN_NAME"
    [[ -x "$NATIVE_BIN" ]] || die "Release executable not found: $NATIVE_BIN"
    ok "Native release binary: $NATIVE_BIN"
}

# AppImage icon metadata is independent of the host file-manager icon.
# Validate the staged AppDir instead of assuming linuxdeploy packaged the icon.
validate_appimage_icon_layout() {
    local appdir="$1" desktop_path="$2" icon_path="$3" icon_in_theme="$4"
    [[ -s "$icon_path" ]] || die "AppImage icon missing or empty: $icon_path"
    [[ -s "$icon_in_theme" ]] || die "AppImage themed icon missing or empty: $icon_in_theme"
    [[ -f "$desktop_path" ]] || die "AppImage desktop entry missing: $desktop_path"
    grep -Fxq "Icon=$APP_ID" "$desktop_path" || \
        die "AppImage desktop entry icon does not match $APP_ID"
    [[ -e "$appdir/$APP_ID.desktop" ]] || \
        die "AppImage root desktop entry missing: $appdir/$APP_ID.desktop"
    [[ -e "$appdir/$APP_ID.$ICON_EXT" ]] || \
        die "AppImage root icon missing: $appdir/$APP_ID.$ICON_EXT"

    # appimagetool may regenerate this symlink during squashfs creation.
    # Its target must be the application's real (not generic) staged icon.
    ln -sfn "$APP_ID.$ICON_EXT" "$appdir/.DirIcon"
    [[ -s "$appdir/.DirIcon" ]] || die "AppImage .DirIcon is invalid"
    ok "AppImage desktop/icon metadata: $APP_ID ($ICON_EXT), .DirIcon valid"
}

build_appimage() {
    local linuxdeploy app_work appdir desktop staging_icon output_name deployed_bin real_bin resource_root app_icon_dir
    local svg_loader pixbuf_query deployed_svg_loader pixbuf_cache_dir pixbuf_cache_template cache_loader line
    if ! linuxdeploy="$(linuxdeploy_path)"; then
        return 1
    fi
    [[ -n "$linuxdeploy" && -x "$linuxdeploy" ]] || {
        warn "AppImage skipped: linuxdeploy is unavailable or not executable."
        return 1
    }

    svg_loader="$(find_gdk_pixbuf_svg_loader)"
    [[ -n "$svg_loader" && -f "$svg_loader" ]] || \
        die "GdkPixbuf SVG loader is unavailable; install librsvg2/librsvg2-common."
    pixbuf_query="$(find_gdk_pixbuf_query_loaders)"
    [[ -n "$pixbuf_query" && -x "$pixbuf_query" ]] || \
        die "gdk-pixbuf-query-loaders is unavailable."

    app_work="$WORK_ROOT/appimage"
    appdir="$app_work/AppDir"
    rm -rf "$app_work"
    mkdir -p "$app_work" "$appdir"

    desktop="$app_work/$APP_ID.desktop"
    write_desktop_file "$desktop"
    find_or_make_icon "$app_work" || return 1
    staging_icon="$ICON_FILE"
    output_name="PIC-${BUILD_LABEL}-${ARCH_NAME}.AppImage"
    rm -f "$DIST_DIR/$output_name"

    log "Creating AppDir with linuxdeploy"
    APPIMAGE_EXTRACT_AND_RUN=1 "$linuxdeploy" \
        --appdir "$appdir" \
        --executable "$NATIVE_BIN" \
        --library "$svg_loader" \
        --desktop-file "$desktop" \
        --icon-file "$staging_icon"

    # GdkPixbuf image loaders are plugins, not normal link dependencies. The
    # explicit --library above makes linuxdeploy collect librsvg and its ELF
    # dependencies. Build a one-loader cache template whose path is resolved
    # by the AppImage launcher after the runtime mount point is known.
    deployed_svg_loader="$appdir/usr/lib/$(basename "$svg_loader")"
    [[ -s "$deployed_svg_loader" ]] || \
        die "linuxdeploy did not stage the GdkPixbuf SVG loader: $deployed_svg_loader"
    pixbuf_cache_dir="$appdir/usr/lib/gdk-pixbuf-2.0/2.10.0"
    pixbuf_cache_template="$pixbuf_cache_dir/pic-svg-loaders.cache.in"
    mkdir -p "$pixbuf_cache_dir"
    cache_loader="@PIC_PREFIX@/lib/$(basename "$svg_loader")"
    : > "$pixbuf_cache_template"
    while IFS= read -r line; do
        printf '%s\n' "${line//$svg_loader/$cache_loader}"
    done < <("$pixbuf_query" "$svg_loader") > "$pixbuf_cache_template"
    grep -Fq "$cache_loader" "$pixbuf_cache_template" || \
        die "Could not generate relocatable GdkPixbuf SVG loader cache template."
    ok "Bundled GdkPixbuf SVG loader: $deployed_svg_loader"

    # linuxdeploy versions differ in whether they leave a real application icon
    # at the AppDir root. Install PIC's selected custom icon deterministically:
    # the root copy/.DirIcon is the AppImage file icon, while hicolor is used by
    # desktop launchers. This is separate from the Adwaita symbolic UI icons.
    cp -f "$staging_icon" "$appdir/$APP_ID.$ICON_EXT"
    if [[ "$ICON_EXT" == svg ]]; then
        app_icon_dir="$appdir/usr/share/icons/hicolor/scalable/apps"
    else
        app_icon_dir="$appdir/usr/share/icons/hicolor/256x256/apps"
    fi
    mkdir -p "$app_icon_dir"
    cp -f "$staging_icon" "$app_icon_dir/$APP_ID.$ICON_EXT"
    ln -sfn "$APP_ID.$ICON_EXT" "$appdir/.DirIcon"

    # Keep the real executable separate and put a launcher at usr/bin/pic-rs.
    # The launcher changes into usr/share/pic-rs before starting PIC so existing
    # relative paths such as images/theme/... continue to work in the AppImage.
    deployed_bin="$appdir/usr/bin/$BIN_NAME"
    if [[ ! -f "$deployed_bin" ]]; then
        warn "AppImage staging did not contain the expected executable: $deployed_bin"
        return 1
    fi
    mkdir -p "$appdir/usr/libexec"
    real_bin="$appdir/usr/libexec/$BIN_NAME"
    mv -f "$deployed_bin" "$real_bin"
    chmod +x "$real_bin"
    write_runtime_launcher "$deployed_bin"

    if find "$appdir" -type f \( -name 'pic-nfs-helper*' -o -name 'pic-nfs-probe*' -o -name 'pic-smb-probe*' \) \
        -print -quit | grep -q .; then
        die "AppImage staging unexpectedly contains a diagnostic helper/probe binary"
    fi
    if ldd "$real_bin" | grep -q 'libnfs'; then
        find "$appdir" -type f -name 'libnfs.so*' -print -quit | grep -q . || \
            die "AppImage is missing the libnfs runtime required by direct PIC NFS"
    fi
    if ldd "$real_bin" | grep -q 'libsmbclient'; then
        find "$appdir" -type f -name 'libsmbclient.so*' -print -quit | grep -q . || \
            die "AppImage is missing the libsmbclient runtime required by direct PIC SMB"
    fi

    resource_root="$appdir/usr/share/$BIN_NAME"
    copy_runtime_resources "$resource_root"

    # GTK4/libadwaita applications rely on GLib schemas and Adwaita symbolic icons.
    # linuxdeploy follows shared libraries; these data files are added explicitly.
    if [[ -d /usr/share/glib-2.0/schemas ]]; then
        mkdir -p "$appdir/usr/share/glib-2.0/schemas"
        cp -a /usr/share/glib-2.0/schemas/. "$appdir/usr/share/glib-2.0/schemas/"
        if have glib-compile-schemas; then
            glib-compile-schemas "$appdir/usr/share/glib-2.0/schemas" || true
        fi
    fi
    # Bundle the icon-theme data used by GTK/libadwaita widgets. Adwaita
    # inherits from hicolor, and Ubuntu may also provide AdwaitaLegacy assets
    # referenced by applications/themes. Copy the complete available themes,
    # not only index.theme, so symbolic toolbar/sidebar icons do not fall back
    # to missing-image placeholders when the host theme differs.
    mkdir -p "$appdir/usr/share/icons"
    for icon_theme in Adwaita AdwaitaLegacy hicolor; do
        if [[ -d "/usr/share/icons/$icon_theme" ]]; then
            # Merge rather than replace: linuxdeploy has already installed
            # PIC's own application icon under hicolor, and replacing that
            # directory would leave the AppDir root icon symlink dangling.
            mkdir -p "$appdir/usr/share/icons/$icon_theme"
            cp -a "/usr/share/icons/$icon_theme/." "$appdir/usr/share/icons/$icon_theme/"
        fi
    done

    # The embedded icon is used by AppImage-aware launchers/integrators. GNOME
    # Files does not automatically render arbitrary executable files using it.
    validate_appimage_icon_layout \
        "$appdir" \
        "$appdir/usr/share/applications/$APP_ID.desktop" \
        "$appdir/$APP_ID.$ICON_EXT" \
        "$appdir/usr/share/icons/hicolor/$([[ "$ICON_EXT" == svg ]] && printf scalable || printf 256x256)/apps/$APP_ID.$ICON_EXT"

    log "Writing AppImage: $DIST_DIR/$output_name"
    (
        cd "$DIST_DIR"
        ARCH="$APPIMAGE_ARCH" \
        LDAI_OUTPUT="$output_name" \
        LDAI_NO_APPSTREAM=1 \
        APPIMAGE_EXTRACT_AND_RUN=1 \
            "$linuxdeploy" --appdir "$appdir" --output appimage
    )
    [[ -s "$DIST_DIR/$output_name" ]] || return 1
    chmod +x "$DIST_DIR/$output_name"
    APPIMAGE_OUTPUT="$DIST_DIR/$output_name"
    ok "AppImage created: $APPIMAGE_OUTPUT"

    # GNOME integration is local user metadata, not part of AppImage packaging.
    # Keep it in this one build command; never change an installed Flatpak.
    if [[ "${PIC_GNOME_INTEGRATE:-1}" == 1 ]]; then
        integrate_appimage_gnome "$APPIMAGE_OUTPUT" "$staging_icon" || \
            warn "GNOME icon integration was incomplete; the AppImage itself built successfully."
    fi
}

# Automatically give the newly built AppImage its own icon in GNOME Files and
# make the icon discoverable for the running GTK application. Do not replace an
# existing Flatpak/user launcher sharing APP_ID. Use a distinct AppImage
# desktop ID so a missing AppImage cannot hide the installed Flatpak.
integrate_appimage_gnome() {
    local appimage="$1" source_icon="$2" data_home icon_size icon_dir icon_dest
    local app_dir launcher icon_uri appimage_path icon_name

    [[ -s "$appimage" && -s "$source_icon" ]] || return 1
    data_home="${XDG_DATA_HOME:-$HOME/.local/share}"
    app_dir="$data_home/applications"
    launcher="$app_dir/$APP_ID.AppImage.desktop"
    appimage_path="$(realpath -- "$appimage")" || return 1
    if [[ -f "$launcher" ]]; then
        log "Preserving existing PIC desktop launcher: $launcher"
        return 0
    fi
    # Do not generate an invalid .desktop Exec entry for unusual filenames.
    if [[ "$appimage_path" == *$'\n'* || "$appimage_path" == *'"'* || \
          "$appimage_path" == *'`'* || "$appimage_path" == *'\'* || \
          "$appimage_path" == *'$'* ]]; then
        warn "AppImage path cannot safely be represented in a desktop launcher."
        return 0
    fi
    if [[ "$ICON_EXT" == svg ]]; then icon_size=scalable; else icon_size=256x256; fi
    icon_name="$APP_ID.AppImage"
    icon_dir="$data_home/icons/hicolor/$icon_size/apps"
    icon_dest="$icon_dir/$icon_name.$ICON_EXT"
    mkdir -p "$app_dir" "$icon_dir" || return 1
    cp -f "$source_icon" "$icon_dest" || return 1
    ok "GNOME AppImage icon: $icon_dest"

    # Nautilus/GNOME Files does not automatically read .DirIcon inside AppImages.
    # GVfs metadata lets this user see the proper icon for this exact file.
    if have gio && have python3; then
        icon_uri="$(python3 -c 'from pathlib import Path; import sys; print(Path(sys.argv[1]).resolve().as_uri())' "$icon_dest")" || return 1
        if gio set -t string "$appimage_path" metadata::custom-icon "$icon_uri"; then
            ok "GNOME Files icon: $appimage_path"
        else
            warn "GNOME Files custom icon could not be set; GVfs metadata may be unavailable."
        fi
    else
        warn "gio/python3 unavailable; GNOME Files custom file icon not set."
    fi

    cat > "$launcher" <<EOF_PIC_GNOME
[Desktop Entry]
Type=Application
Name=PIC — Personal Image Catalogue (AppImage)
Comment=PIC photo manager
Exec="$appimage_path" %F
Icon=$icon_name
Terminal=false
StartupNotify=true
Categories=Graphics;Photography;
EOF_PIC_GNOME
    ok "GNOME PIC AppImage launcher: $launcher"
}

run_flatpak_tests() {
    local fp_build="$1" source_dir="$2" test_metadata status=0
    if [[ "$SKIP_TESTS" == 1 ]]; then
        warn "Flatpak tests skipped (--skip-tests / PIC_SKIP_TESTS=1)."
        return 0
    fi
    source_dir="$(realpath "$source_dir")" || return 1
    [[ -d "$source_dir" ]] || die "Flatpak test source directory is unavailable."
    test_metadata="$(mktemp "$fp_build/.pic-tests.XXXXXX.metadata")" || return 1
    # Glycin recognizes an uninstalled Flatpak development build only when
    # /.flatpak-info has a Devel app ID. Override metadata for this test process
    # only; the normal package metadata and runtime sandbox stay unchanged.
    if ! awk -v id="$APP_ID.Devel" '
        /^\[/ { application = ($0 == "[Application]") }
        application && /^name=/ { $0 = "name=" id }
        { print }
    ' "$fp_build/metadata" > "$test_metadata"; then
        rm -f "$test_metadata"
        return 1
    fi
    log "Testing Rust release binary in the Flatpak SDK development sandbox"
    flatpak build \
        --metadata="$(basename "$test_metadata")" \
        --bind-mount="/run/build/pic-rs=$source_dir" \
        --build-dir=/run/build/pic-rs \
        --env=PATH=/usr/lib/sdk/rust-stable/bin:/app/bin:/usr/bin \
        --env=CARGO_NET_OFFLINE=true \
        "$fp_build" sh -c '
            set -eu
            for test_binary in /app/libexec/pic-build-tests/*; do
                [ -x "$test_binary" ] || { echo "Compiled Flatpak tests are unavailable" >&2; exit 1; }
                "$test_binary"
            done
        ' || status=$?
    rm -f "$test_metadata"
    return "$status"
}

build_flatpak() {
    local fp_work fp_src fp_build fp_repo manifest desktop_rel icon_rel bundle_name vendor_dir launcher_rel metainfo_rel
    ensure_flatpak_runtime || return 1

    # flatpak-builder hardlinks files between its state and build directories.
    # Keep this temporary tree on the selected state's filesystem, even when
    # the checkout, dependency cache or explicit state override lives elsewhere.
    fp_work="$FLATPAK_STATE_DIR/pic-build-work"
    fp_src="$fp_work/flatpak-src"
    fp_build="$fp_work/build-dir"
    fp_repo="$fp_work/repo"
    manifest="$fp_work/$APP_ID.json"
    rm -rf "$fp_work"
    mkdir -p "$fp_work"
    ensure_flatpak_module_sources || return 1
    copy_source_tree "$fp_src"

    mkdir -p "$fp_src/packaging-generated" "$fp_src/.cargo"
    write_desktop_file "$fp_src/packaging-generated/$APP_ID.desktop"
    write_metainfo_file "$fp_src/packaging-generated/$APP_ID.metainfo.xml" || return 1
    write_runtime_launcher "$fp_src/packaging-generated/$BIN_NAME-launcher"
    find_or_make_icon "$fp_src/packaging-generated" || return 1
    icon_rel="packaging-generated/$APP_ID.$ICON_EXT"
    desktop_rel="packaging-generated/$APP_ID.desktop"
    metainfo_rel="packaging-generated/$APP_ID.metainfo.xml"
    launcher_rel="packaging-generated/$BIN_NAME-launcher"
    if [[ "$ICON_EXT" == svg ]]; then
        FLATPAK_ICON_DEST="/app/share/icons/hicolor/scalable/apps/$APP_ID.svg"
    else
        FLATPAK_ICON_DEST="/app/share/icons/hicolor/256x256/apps/$APP_ID.$ICON_EXT"
    fi

    log "Vendoring Rust crates for a network-free Flatpak build"
    vendor_dir="$fp_src/vendor"
    rm -rf "$vendor_dir"
    (cd "$fp_src" && cargo vendor --locked --offline vendor >/dev/null)
    cat > "$fp_src/.cargo/config.toml" <<'EOF_CARGO'
[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "vendor"

[net]
offline = true
EOF_CARGO

    # Cache test executables in the same module as the application, so a
    # builder cache hit always restores tests for the selected source revision.
    # Finish-phase cleanup removes them before exporting the runtime package.
    cat > "$fp_src/packaging-generated/install-tests.py" <<'EOF_TEST_INSTALLER'
import json
from pathlib import Path
import shutil
import sys

destination = Path(sys.argv[2])
destination.mkdir(parents=True, exist_ok=True)
installed = 0
for line in Path(sys.argv[1]).read_text().splitlines():
    artifact = json.loads(line)
    if artifact.get("reason") != "compiler-artifact" or not artifact.get("profile", {}).get("test"):
        continue
    executable = artifact.get("executable")
    if executable:
        source = Path(executable)
        output = destination / source.name
        shutil.copyfile(source, output)
        output.chmod(0o755)
        installed += 1
if not installed:
    sys.exit("Cargo produced no test executables")
EOF_TEST_INSTALLER
    local flatpak_test_build_command="cargo test --release --locked --offline --no-run --message-format=json > packaging-generated/test-artifacts.json && python3 packaging-generated/install-tests.py packaging-generated/test-artifacts.json /app/libexec/pic-build-tests"
    [[ "$SKIP_TESTS" != 1 ]] || flatpak_test_build_command="true"

    cat > "$manifest" <<EOF_MANIFEST
{
  "app-id": "$APP_ID",
  "runtime": "org.gnome.Platform",
  "runtime-version": "$GNOME_RUNTIME",
  "sdk": "org.gnome.Sdk",
  "sdk-extensions": ["org.freedesktop.Sdk.Extension.rust-stable"],
  "command": "$BIN_NAME",
  "finish-args": [
    "--share=ipc",
    "--socket=wayland",
    "--socket=fallback-x11",
    "--device=dri",
"--share=network",
"--filesystem=host",
"--filesystem=xdg-data/pic-rs:create",
"--filesystem=xdg-data/picasa-rs",
"--filesystem=xdg-cache/pic-rs:create",
"--filesystem=xdg-cache/picasa-rs",
"--filesystem=~/.var/app/io.github.you.PicasaRs",

"--talk-name=org.gtk.vfs.*",
"--filesystem=xdg-run/gvfs",
"--filesystem=xdg-run/gvfsd",

"--system-talk-name=org.freedesktop.Avahi"
  ],
  "build-options": {
    "append-path": "/usr/lib/sdk/rust-stable/bin",
    "env": { "CARGO_NET_OFFLINE": "true" }
  },
  "cleanup": ["/libexec/pic-build-tests"],
  "modules": [
    {
      "name": "libnfs",
      "buildsystem": "cmake-ninja",
      "config-opts": ["-DCMAKE_BUILD_TYPE=Release"],
      "cleanup": ["/include", "/bin", "/lib/pkgconfig", "/lib/*.a", "/lib/*.so"],
      "sources": [{
        "type": "archive",
        "url": "https://github.com/sahlberg/libnfs/archive/libnfs-6.0.2.tar.gz",
        "sha256": "4e5459cc3e0242447879004e9ad28286d4d27daa42cbdcde423248fad911e747"
      }]
    },
    {
      "name": "samba",
      "buildsystem": "autotools",
      "config-opts": [
        "--prefix=/app", "--libdir=/app/lib", "--disable-rpath",
        "--disable-python", "--without-ads", "--without-ldap", "--without-pam",
        "--without-acl-support", "--without-systemd", "--without-ad-dc",
        "--without-json", "--disable-cups", "--disable-iprint", "--without-ldb-lmdb"
      ],
      "build-options": { "env": { "PERL5LIB": "/app/lib/perl5" } },
      "cleanup": ["/bin", "/sbin", "/libexec", "/share", "/include", "/lib/pkgconfig", "/lib/*.so", "/lib/perl5"],
      "sources": [{
        "type": "archive",
        "url": "https://download.samba.org/pub/samba/stable/samba-4.24.7.tar.gz",
        "sha256": "45b7747a47452eff2b2159a44cc63eb43690d339fd1069088e023a015fed06c7"
      }],
      "modules": [{
        "name": "parse-yapp",
        "buildsystem": "simple",
        "build-commands": ["perl Makefile.PL PREFIX=/app LIB=/app/lib/perl5", "make", "make install"],
        "sources": [{
          "type": "archive",
          "url": "https://cpan.metacpan.org/authors/id/W/WB/WBRASWELL/Parse-Yapp-1.21.tar.gz",
          "sha256": "3810e998308fba2e0f4f26043035032b027ce51ce5c8a52a8b8e340ca65f13e5"
        }]
      }]
    },
    {
      "name": "pic-rs",
      "buildsystem": "simple",
      "build-commands": [
        "$flatpak_test_build_command",
        "cargo build --release --locked --offline",
        "install -Dm755 target/release/$BIN_NAME /app/libexec/$BIN_NAME",
        "install -Dm755 $launcher_rel /app/bin/$BIN_NAME",
        "install -d /app/share/$BIN_NAME",
        "cp -a images /app/share/$BIN_NAME/",
        "cp -a themes /app/share/$BIN_NAME/",
        "cp -a resources /app/share/$BIN_NAME/",
        "install -Dm644 $desktop_rel /app/share/applications/$APP_ID.desktop",
        "install -Dm644 $metainfo_rel /app/share/metainfo/$APP_ID.metainfo.xml",
        "install -Dm644 $icon_rel $FLATPAK_ICON_DEST"
      ],
      "sources": [
        { "type": "dir", "path": "flatpak-src" }
      ]
    }
  ]
}
EOF_MANIFEST

    local download_args=()
    if ((ONLINE)); then
        log "Building Flatpak inside GNOME SDK (dependency downloads allowed)"
    else
        log "Building Flatpak inside GNOME SDK (downloads disabled)"
        download_args+=(--disable-download)
    fi
    flatpak-builder \
        --force-clean \
        --build-only \
        --state-dir="$FLATPAK_STATE_DIR" \
        "${download_args[@]}" \
        "$fp_build" "$manifest" || return 1

    # Test before clean/finish/export. A test failure never produces a bundle.
    run_flatpak_tests "$fp_build" "$fp_src" || return 1
    flatpak-builder \
        --finish-only \
        --state-dir="$FLATPAK_STATE_DIR" \
        "${download_args[@]}" \
        --repo="$fp_repo" \
        "$fp_build" "$manifest" || return 1

    if find "$fp_build/files" -type f \( -name 'pic-nfs-helper*' -o -name 'pic-nfs-probe*' -o -name 'pic-smb-probe*' \) \
        -print -quit | grep -q .; then
        die "Flatpak staging unexpectedly contains a diagnostic helper/probe binary"
    fi
    if ! find "$fp_build/files" -type f -name 'libnfs.so*' -print -quit | grep -q .; then
        die "Flatpak staging is missing the libnfs runtime required by direct PIC NFS"
    fi
    if ! find "$fp_build/files" -type f -name 'libsmbclient.so*' -print -quit | grep -q .; then
        die "Flatpak staging is missing the libsmbclient runtime required by direct PIC SMB"
    fi

    bundle_name="PIC-${BUILD_LABEL}-${ARCH_NAME}.flatpak"
    rm -f "$DIST_DIR/$bundle_name"
    flatpak build-bundle "$fp_repo" "$DIST_DIR/$bundle_name" "$APP_ID" || return 1
    [[ -s "$DIST_DIR/$bundle_name" ]] || return 1
    FLATPAK_OUTPUT="$DIST_DIR/$bundle_name"
    ok "Flatpak bundle created: $FLATPAK_OUTPUT"
}

# -------------------- main --------------------
if [[ "$MODE" == local ]]; then
    prepare_local_source
    dependency_preflight
else
    # Install host tools before using Git; check Rust crates again once the
    # selected checkout and its lockfile are available.
    dependency_preflight
    prepare_github_source
    dependency_preflight
fi
# A disappearing cache entry must fail packaging rather than trigger any
# additional installation/download outside the approval preflight.
ONLINE=0
if ((CHECK_DEPENDENCIES_ONLY)); then
    ok "Dependency check/setup finished; no compilation requested."
    exit 0
fi

BIN_NAME="${BIN_NAME_OVERRIDE:-$(project_binary_name)}"
validate_packaging_resources
VERSION="$(project_version)"
VERSION="${VERSION:-0.0.0}"
REVISION="$(project_revision)"
BUILD_LABEL="$VERSION${REVISION:+-$REVISION}"

case "$(uname -m)" in
    x86_64|amd64) ARCH_NAME=x86_64; APPIMAGE_ARCH=x86_64 ;;
    aarch64|arm64) ARCH_NAME=aarch64; APPIMAGE_ARCH=aarch64 ;;
    i386|i486|i586|i686) ARCH_NAME=i686; APPIMAGE_ARCH=i686 ;;
    *) ARCH_NAME="$(uname -m)"; APPIMAGE_ARCH="$ARCH_NAME" ;;
esac

log "Build summary"
printf 'Mode:       %s\n' "$MODE"
printf 'Source:     %s\n' "$SOURCE_DIR"
printf 'Version:    %s\n' "$VERSION"
printf 'Revision:   %s\n' "${REVISION:-local-uncommitted}"
printf 'Binary:     %s\n' "$BIN_NAME"
printf 'App ID:     %s\n' "$APP_ID"
printf 'Output:     %s\n' "$DIST_DIR"
printf 'Build log:  %s\n' "$LOG_FILE"
printf 'Target:     %s\n' "$BUILD_TARGET"
if [[ "$BUILD_TARGET" == both || "$BUILD_TARGET" == flatpak ]]; then
    printf 'Flatpak:    GNOME %s + Rust extension %s\n' "$GNOME_RUNTIME" "$FDO_RUST_RUNTIME"
fi

if [[ "$BUILD_TARGET" == both || "$BUILD_TARGET" == appimage ]]; then
    build_native
fi

appimage_ok=0
flatpak_ok=0
appimage_selected=0
flatpak_selected=0

if [[ "$BUILD_TARGET" == both || "$BUILD_TARGET" == appimage ]]; then
    appimage_selected=1
    if build_appimage; then
        appimage_ok=1
    else
        if [[ "$BUILD_TARGET" == both ]]; then
            warn "AppImage build failed; continuing so the Flatpak build still gets a chance."
        else
            warn "AppImage build failed."
        fi
    fi
fi

if [[ "$BUILD_TARGET" == both || "$BUILD_TARGET" == flatpak ]]; then
    flatpak_selected=1
    if build_flatpak; then
        flatpak_ok=1
    else
        warn "Flatpak build failed."
    fi
fi

printf '\n============================================================\n'
printf 'PIC Linux packaging finished\n'
printf '============================================================\n'
if ((appimage_selected)); then
    ((appimage_ok)) && printf 'AppImage: %s\n' "$APPIMAGE_OUTPUT" || printf 'AppImage: FAILED\n'
else
    printf 'AppImage: SKIPPED\n'
fi
if ((flatpak_selected)); then
    ((flatpak_ok)) && printf 'Flatpak:  %s\n' "$FLATPAK_OUTPUT" || printf 'Flatpak:  FAILED\n'
else
    printf 'Flatpak:  SKIPPED\n'
fi
printf 'Source:   %s (%s)\n' "$SOURCE_DIR" "$MODE"
printf 'Log:      %s\n' "$LOG_FILE"
printf '============================================================\n'

if ((appimage_selected && ! appimage_ok)); then
    exit 1
fi
if ((flatpak_selected && ! flatpak_ok)); then
    exit 1
fi
