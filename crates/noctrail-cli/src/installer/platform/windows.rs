use std::fs;

use crate::installer::{
    ENV_MSI, InstallerArtifacts, require_path, run_command, run_packaged_smoke, temp_smoke_dir,
};

pub(super) fn run(artifacts: &InstallerArtifacts) -> Result<(), String> {
    let msi = require_path(&artifacts.msi, ".msi package", ENV_MSI)?;
    let temp = temp_smoke_dir("noctrail-installer-windows")?;
    let install_root = temp.join("Noctrail");
    fs::create_dir_all(&install_root)
        .map_err(|error| format!("create {}: {error}", install_root.display()))?;

    run_command(
        "msiexec",
        &[
            std::ffi::OsStr::new("/i"),
            msi.as_os_str(),
            std::ffi::OsStr::new("/qn"),
            std::ffi::OsStr::new("INSTALLDIR"),
            install_root.as_os_str(),
        ],
    )?;
    run_packaged_smoke(&install_root.join("noctrail-app.exe"))?;
    run_command(
        "msiexec",
        &[
            std::ffi::OsStr::new("/i"),
            msi.as_os_str(),
            std::ffi::OsStr::new("/qn"),
            std::ffi::OsStr::new("REINSTALL=ALL"),
            std::ffi::OsStr::new("REINSTALLMODE=vomus"),
            std::ffi::OsStr::new("INSTALLDIR"),
            install_root.as_os_str(),
        ],
    )?;
    run_packaged_smoke(&install_root.join("noctrail-app.exe"))?;
    run_command(
        "msiexec",
        &[
            std::ffi::OsStr::new("/x"),
            msi.as_os_str(),
            std::ffi::OsStr::new("/qn"),
        ],
    )?;

    println!("platform=windows msi={} uninstall=ok", msi.display());
    println!("installer smoke ok");
    Ok(())
}
