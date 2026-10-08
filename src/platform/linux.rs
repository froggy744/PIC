use std::rc::Rc;

use gio::prelude::*;

pub(super) fn reveal_file(file: &gio::File) -> bool {
    let Some(path) = file.path() else {
        return false;
    };
    std::process::Command::new("nautilus")
        .arg("--select")
        .arg(path)
        .spawn()
        .is_ok()
}

pub(super) fn install_native_mount_monitor(
    window: &libadwaita::ApplicationWindow,
    refresh: Rc<dyn Fn()>,
) {
    let monitor = gio::UnixMountMonitor::get();
    let refresh_mountpoints = refresh.clone();
    monitor.connect_mountpoints_changed(move |_| refresh_mountpoints());
    monitor.connect_mounts_changed(move |_| refresh());
    unsafe {
        window.set_data("picasa-unix-mount-monitor", monitor);
    }
}
