use std::{env, ffi::OsStr, fs, path::Path};

use crate::installer::{
    ENV_APPIMAGE, ENV_DEB, ENV_RPM, InstallerArtifacts, require_path, run_command,
    run_packaged_smoke, run_shell_command, temp_smoke_dir,
};

pub(super) fn run(artifacts: &InstallerArtifacts) -> Result<(), String> {
    let deb = require_path(&artifacts.deb, ".deb package", ENV_DEB)?;
    let appimage = require_path(&artifacts.appimage, ".AppImage bundle", ENV_APPIMAGE)?;
    let rpm = require_path(&artifacts.rpm, ".rpm package", ENV_RPM)?;
    let temp = temp_smoke_dir("noctrail-installer-linux")?;

    let appimage_copy = temp.join("Noctrail.AppImage");
    fs::copy(&appimage, &appimage_copy)
        .map_err(|error| format!("copy {}: {error}", appimage.display()))?;
    make_executable(&appimage_copy)?;
    run_packaged_smoke(&appimage_copy)?;

    let deb_root = temp.join("deb-root");
    fs::create_dir_all(&deb_root)
        .map_err(|error| format!("create {}: {error}", deb_root.display()))?;
    run_command(
        "dpkg-deb",
        &[OsStr::new("-x"), deb.as_os_str(), deb_root.as_os_str()],
    )?;
    run_packaged_smoke(&deb_root.join("usr/bin/noctrail-app"))?;

    let rpm_root = temp.join("rpm-root");
    fs::create_dir_all(&rpm_root)
        .map_err(|error| format!("create {}: {error}", rpm_root.display()))?;
    if command_exists("bsdtar") {
        run_command(
            "bsdtar",
            &[
                OsStr::new("-xf"),
                rpm.as_os_str(),
                OsStr::new("-C"),
                rpm_root.as_os_str(),
            ],
        )?;
    } else if command_exists("rpm2cpio") && command_exists("cpio") {
        let pipeline = format!(
            "rpm2cpio '{}' | (cd '{}' && cpio -idm --quiet)",
            rpm.display(),
            rpm_root.display()
        );
        run_shell_command(&pipeline)?;
    } else {
        return Err("need bsdtar or rpm2cpio+cpio to inspect RPM payloads".to_string());
    }
    run_packaged_smoke(&rpm_root.join("usr/bin/noctrail-app"))?;

    fs::remove_file(&appimage_copy)
        .map_err(|error| format!("remove {}: {error}", appimage_copy.display()))?;
    fs::remove_dir_all(&deb_root)
        .map_err(|error| format!("remove {}: {error}", deb_root.display()))?;
    fs::remove_dir_all(&rpm_root)
        .map_err(|error| format!("remove {}: {error}", rpm_root.display()))?;

    println!(
        "platform=linux appimage={} deb={} rpm={} uninstall=ok",
        appimage.display(),
        deb.display(),
        rpm.display(),
    );
    println!("installer smoke ok");
    Ok(())
}

fn command_exists(name: &str) -> bool {
    env::var_os("PATH")
        .map(|paths| env::split_paths(&paths).any(|path| path.join(name).exists()))
        .unwrap_or(false)
}

fn make_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)
        .map_err(|error| format!("stat {}: {error}", path.display()))?
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions)
        .map_err(|error| format!("chmod {}: {error}", path.display()))
}
