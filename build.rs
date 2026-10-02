//! Stage runtime resource folders next to the built binary.
//!
//! PIC resolves `themes` and `images` relative to the working directory
//! (see `resolve_runtime_dir` in `src/css/mod.rs`). Packaging scripts copy
//! those folders next to the executable, but a plain `cargo build` did not,
//! so `target/release/pic-rs` started without appearance themes or album
//! artwork. After each build this script mirrors the packaging layout by
//! copying the current `themes/` and `images/` folders into the profile
//! directory (for example `target/release`), keeping them in sync whenever
//! their contents change.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    emit_build_metadata();
    // Fedora/Linux proof of concept; do not link private transports for other OSes.
    #[cfg(target_os = "linux")]
    {
        let samba = pkg_config::Config::new()
            .cargo_metadata(false)
            .probe("smbclient")
            .expect("Install libsmbclient-devel for direct SMB support");
        let nfs = pkg_config::Config::new()
            .cargo_metadata(false)
            .probe("libnfs")
            .expect("Install libnfs-devel for direct NFS support");
        let mut cc = cc::Build::new();
        cc.file("native/private_smb.c").file("native/private_nfs.c");
        for path in samba.include_paths.iter().chain(nfs.include_paths.iter()) {
            cc.include(path);
        }

        // libnfs 5.x uses nfs_read(nfs, fh, count, buf), while newer
        // releases use nfs_read(nfs, fh, buf, count). Ubuntu 24.04 ships
        // libnfs 5.x; Fedora currently ships the newer API.
        let libnfs_major = nfs
            .version
            .split('.')
            .next()
            .and_then(|part| part.parse::<u64>().ok())
            .unwrap_or(0);
        if libnfs_major > 0 && libnfs_major < 6 {
            cc.define("PIC_LIBNFS_LEGACY_READ_ORDER", None);
        }

        cc.compile("pic_private_transports");

        // Emit the native archive before the system libraries it depends on.
        pkg_config::Config::new()
            .probe("smbclient")
            .expect("Install libsmbclient-devel for direct SMB support");
        pkg_config::Config::new()
            .probe("libnfs")
            .expect("Install libnfs-devel for direct NFS support");

        println!("cargo:rerun-if-changed=native/private_smb.c");
        println!("cargo:rerun-if-changed=native/private_nfs.c");
    }

    println!("cargo:rerun-if-changed=themes");
    println!("cargo:rerun-if-changed=images");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    // OUT_DIR looks like <target>/<profile>/build/<pkg>-<hash>/out, so three
    // levels up is the profile directory that holds the binaries.
    let Some(profile_dir) = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"))
        .ancestors()
        .nth(3)
        .map(Path::to_path_buf)
    else {
        return;
    };

    for folder in ["themes", "images"] {
        let source = manifest_dir.join(folder);
        if source.is_dir() {
            stage_dir(&source, &profile_dir.join(folder));
        }
    }
}

fn emit_build_metadata() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into());
    let revision = env::var("PIC_BUILD_REVISION")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            std::process::Command::new("git")
                .args(["-C", &manifest_dir, "rev-parse", "--short=10", "HEAD"])
                .output()
                .ok()
                .filter(|output| output.status.success())
                .and_then(|output| String::from_utf8(output.stdout).ok())
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_else(|| "unknown".into());

    let build_date = std::process::Command::new("date")
        .args(["-u", "+%Y-%m-%d"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown".into());

    println!("cargo:rustc-env=PIC_BUILD_REVISION={revision}");
    println!("cargo:rerun-if-env-changed=PIC_BUILD_REVISION");
    println!("cargo:rustc-env=PIC_BUILD_DATE={build_date}");
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");
}

/// Replace `destination` with a fresh copy of `source`.
fn stage_dir(source: &Path, destination: &Path) {
    match fs::remove_dir_all(destination) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            eprintln!(
                "build.rs: could not clear {}: {error}",
                destination.display()
            );
            return;
        }
    }
    if let Err(error) = copy_tree(source, destination) {
        eprintln!(
            "build.rs: could not stage {} -> {}: {error}",
            source.display(),
            destination.display()
        );
    }
}

fn copy_tree(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}
